//! `confighunter`: install complete xfetch config presets from
//! `xfetch-cli/configs`.
//!
//! The command downloads a preset index, stages every file, decodes base64
//! assets to their real extensions, verifies SHA-256 digests, backs up the
//! files it replaces and prints the preset README with its requirement
//! checklist.

use base64::Engine as _;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use xfetch_guest_api::{HostCallError, host_call, http_request, log};

const CONFIG_ROOT: &str = "/config";
const DEFAULT_BRANCH: &str = "main";
const REPO_RAW: &str = "https://raw.githubusercontent.com/xfetch-cli/configs";
const CATALOG_PATH: &str = "configshunter/index.json";
const INTERNAL_DIR: &str = ".confighunter";
const HTTP_TIMEOUT_MS: u64 = 60_000;
const USER_AGENT: &str = "xfetch-command-confighunter/0.1.0";

#[derive(Debug, Deserialize)]
struct CommandRequest {
    #[serde(default = "default_version")]
    version: u32,
    #[serde(default = "default_kind")]
    kind: String,
    #[serde(default)]
    args: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct Catalog {
    schema_version: u32,
    presets: Vec<Preset>,
}

#[derive(Debug, Deserialize, Clone)]
struct Preset {
    name: String,
    version: String,
    description: String,
    path: String,
    #[serde(default)]
    readme: Option<String>,
    #[serde(default)]
    xfetch_min: Option<String>,
    #[serde(default)]
    requires: Requirements,
    files: Vec<FileSpec>,
}

#[derive(Debug, Deserialize, Clone, Default)]
struct Requirements {
    #[serde(default)]
    plugins: Vec<String>,
    #[serde(default)]
    extensions: Vec<String>,
    #[serde(default)]
    fonts: Vec<String>,
    #[serde(default)]
    tools: Vec<String>,
}

#[derive(Debug, Deserialize, Clone)]
struct FileSpec {
    source: String,
    target: String,
    #[serde(default)]
    encoding: Option<String>,
    #[serde(default)]
    sha256: Option<String>,
    #[serde(default)]
    size: Option<u64>,
}

#[derive(Debug, PartialEq, Eq)]
enum Action {
    Help,
    List {
        branch: String,
    },
    Info {
        name: String,
        branch: String,
    },
    Install {
        name: String,
        branch: String,
        backup: bool,
    },
}

struct InstallResult {
    files: usize,
    bytes: u64,
    backup: Option<String>,
}

fn main() {
    if let Err(err) = run() {
        eprintln!("confighunter: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let request = read_request()?;
    if request.version != 1 || request.kind != "command" {
        return Err(format!(
            "unsupported command request: version {} kind '{}'",
            request.version, request.kind
        ));
    }

    match parse_action(&request.args)? {
        Action::Help => {
            print_help();
            Ok(())
        }
        Action::List { branch } => list_presets(&branch),
        Action::Info { name, branch } => show_preset(&name, &branch),
        Action::Install {
            name,
            branch,
            backup,
        } => install_preset_command(&name, &branch, backup),
    }
}

fn read_request() -> Result<CommandRequest, String> {
    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .map_err(|err| format!("failed to read command request: {err}"))?;
    if input.trim().is_empty() {
        return Err("missing command request on stdin".to_string());
    }
    serde_json::from_str(&input).map_err(|err| format!("invalid command request: {err}"))
}

fn default_version() -> u32 {
    1
}

fn default_kind() -> String {
    "command".to_string()
}

fn parse_action(args: &[String]) -> Result<Action, String> {
    let Some(command) = args.first() else {
        return Ok(Action::Help);
    };

    let mut branch = DEFAULT_BRANCH.to_string();
    let mut backup = true;
    let mut positional = Vec::new();
    let mut index = 1;

    while index < args.len() {
        let arg = &args[index];
        match arg.as_str() {
            "-h" | "--help" => return Ok(Action::Help),
            "--no-backup" => backup = false,
            "--branch" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| "--branch requires a value".to_string())?;
                branch = value.clone();
            }
            value if value.starts_with("--branch=") => {
                branch = value["--branch=".len()..].to_string();
            }
            value if value.starts_with('-') => {
                return Err(format!("unknown option '{value}'"));
            }
            value => positional.push(value.to_string()),
        }
        index += 1;
    }

    validate_branch(&branch)?;

    match command.as_str() {
        "help" => Ok(Action::Help),
        "list" => {
            if !positional.is_empty() {
                return Err("usage: xfetch run confighunter list [--branch <branch>]".to_string());
            }
            Ok(Action::List { branch })
        }
        "info" => {
            let name = positional
                .first()
                .ok_or_else(|| "usage: xfetch run confighunter info <preset>".to_string())?
                .clone();
            if positional.len() > 1 {
                return Err("usage: xfetch run confighunter info <preset>".to_string());
            }
            Ok(Action::Info { name, branch })
        }
        "install" => {
            let name = positional
                .first()
                .ok_or_else(|| "usage: xfetch run confighunter install <preset>".to_string())?
                .clone();
            if positional.len() > 1 {
                return Err("usage: xfetch run confighunter install <preset>".to_string());
            }
            Ok(Action::Install {
                name,
                branch,
                backup,
            })
        }
        other => Err(format!("unknown command '{other}'")),
    }
}

fn print_help() {
    println!("confighunter: install complete xfetch config presets");
    println!();
    println!("usage:");
    println!("  xfetch run confighunter list [--branch <branch>]");
    println!("  xfetch run confighunter info <preset> [--branch <branch>]");
    println!("  xfetch run confighunter install <preset> [--branch <branch>] [--no-backup]");
    println!();
    println!("presets are read from xfetch-cli/configs/configshunter/index.json");
    println!("default branch: {DEFAULT_BRANCH}");
}

fn list_presets(branch: &str) -> Result<(), String> {
    let catalog = load_catalog(branch)?;
    println!("presets (branch {branch}):");
    for preset in &catalog.presets {
        println!(
            "  {:<12} {:<8} {}",
            preset.name, preset.version, preset.description
        );
    }
    Ok(())
}

fn show_preset(name: &str, branch: &str) -> Result<(), String> {
    let catalog = load_catalog(branch)?;
    let preset = find_preset(&catalog, name)?;

    println!("name: {}", preset.name);
    println!("version: {}", preset.version);
    println!("description: {}", preset.description);
    println!("branch: {branch}");
    println!("files: {}", preset.files.len());
    println!("source: {}", preset_base(branch, preset)?);
    if let Some(min) = &preset.xfetch_min {
        println!("xfetch_min: {min}");
    }
    println!();
    print_requirements(preset);
    println!();
    match readme_text(preset, branch) {
        Ok(text) => println!("README:\n{}", text.trim_end()),
        Err(err) => eprintln!("readme unavailable: {err}"),
    }
    Ok(())
}

fn install_preset_command(name: &str, branch: &str, backup: bool) -> Result<(), String> {
    let catalog = load_catalog(branch)?;
    let preset = find_preset(&catalog, name)?;
    let base = preset_base(branch, preset)?;
    let stamp = timestamp_ms();
    let staging = Path::new(CONFIG_ROOT).join(format!(".confighunter-staging-{stamp}"));

    let result = stage_and_install(preset, &base, backup, &staging);
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    let result = result?;

    println!("installed: {} {}", preset.name, preset.version);
    println!("source: {base}");
    println!("files: {} ({})", result.files, human_bytes(result.bytes));
    println!("config: ~/.config/xfetch");
    match &result.backup {
        Some(path) => println!("backup: ~/.config/xfetch/{path}"),
        None => println!("backup: none"),
    }
    println!();
    print_requirements(preset);
    println!();

    match readme_text(preset, branch) {
        Ok(text) => println!("README:\n{}", text.trim_end()),
        Err(err) => eprintln!("readme unavailable: {err}"),
    }

    Ok(())
}

fn stage_and_install(
    preset: &Preset,
    base: &str,
    backup: bool,
    staging: &Path,
) -> Result<InstallResult, String> {
    if staging.exists() {
        fs::remove_dir_all(staging)
            .map_err(|err| format!("cannot clear staging '{}': {err}", staging.display()))?;
    }
    fs::create_dir_all(staging)
        .map_err(|err| format!("cannot create staging '{}': {err}", staging.display()))?;

    let mut targets = Vec::new();
    let mut bytes_total = 0u64;

    for (position, file) in preset.files.iter().enumerate() {
        let source = safe_rel_path(&file.source)?;
        let target = safe_rel_path(&file.target)?;
        let url = format!("{base}/{}", source.display());
        let raw = fetch_bytes(&url)?;
        let bytes = if file.encoding.as_deref() == Some("base64") {
            decode_base64_text(&raw)?
        } else {
            raw
        };

        if let Some(size) = file.size
            && bytes.len() as u64 != size
        {
            return Err(format!(
                "size mismatch for '{}': expected {size} bytes, got {}",
                file.source,
                bytes.len()
            ));
        }
        if let Some(expected) = &file.sha256 {
            verify_sha256(&bytes, expected)
                .map_err(|err| format!("{} for '{}'", err, file.source))?;
        }
        if !image_magic_ok(target, &bytes) {
            return Err(format!(
                "decoded '{}' is not a valid image for target '{}'",
                file.source, file.target
            ));
        }

        let destination = staging.join(target);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).map_err(|err| {
                format!(
                    "cannot create staging directory '{}': {err}",
                    parent.display()
                )
            })?;
        }
        fs::write(&destination, &bytes).map_err(|err| {
            format!(
                "cannot write staging file '{}': {err}",
                destination.display()
            )
        })?;

        targets.push(target.to_path_buf());
        bytes_total += bytes.len() as u64;

        if position % 20 == 0 || position + 1 == preset.files.len() {
            log(
                "info",
                &format!(
                    "confighunter: downloaded {}/{}",
                    position + 1,
                    preset.files.len()
                ),
            );
        }
    }

    let existing: Vec<PathBuf> = targets
        .iter()
        .filter(|target| Path::new(CONFIG_ROOT).join(target).is_file())
        .cloned()
        .collect();
    let backup_path = if backup && !existing.is_empty() {
        Some(backup_targets(&existing, timestamp_ms())?)
    } else {
        None
    };

    for target in &targets {
        let source = staging.join(target);
        let destination = Path::new(CONFIG_ROOT).join(target);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)
                .map_err(|err| format!("cannot create directory '{}': {err}", parent.display()))?;
        }
        if destination.exists() {
            if destination.is_dir() {
                return Err(format!(
                    "cannot replace '{}': target is a directory",
                    destination.display()
                ));
            }
            fs::remove_file(&destination)
                .map_err(|err| format!("cannot replace '{}': {err}", destination.display()))?;
        }
        fs::rename(&source, &destination).map_err(|err| {
            format!(
                "cannot move '{}' to '{}': {err}",
                source.display(),
                destination.display()
            )
        })?;
    }

    fs::remove_dir_all(staging)
        .map_err(|err| format!("cannot remove staging '{}': {err}", staging.display()))?;

    Ok(InstallResult {
        files: targets.len(),
        bytes: bytes_total,
        backup: backup_path,
    })
}

fn backup_targets(targets: &[PathBuf], stamp: u128) -> Result<String, String> {
    let backups = Path::new(CONFIG_ROOT).join(INTERNAL_DIR).join("backups");
    fs::create_dir_all(&backups).map_err(|err| {
        format!(
            "cannot create backup directory '{}': {err}",
            backups.display()
        )
    })?;

    let mut name = stamp.to_string();
    let mut attempt = 0u32;
    let mut root = backups.join(&name);
    while root.exists() {
        attempt += 1;
        name = format!("{stamp}-{attempt}");
        root = backups.join(&name);
    }
    fs::create_dir_all(&root)
        .map_err(|err| format!("cannot create backup '{}': {err}", root.display()))?;

    for target in targets {
        let source = Path::new(CONFIG_ROOT).join(target);
        let destination = root.join(target);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).map_err(|err| {
                format!(
                    "cannot create backup directory '{}': {err}",
                    parent.display()
                )
            })?;
        }
        fs::copy(&source, &destination).map_err(|err| {
            format!(
                "cannot back up '{}' to '{}': {err}",
                source.display(),
                destination.display()
            )
        })?;
    }

    Ok(Path::new(INTERNAL_DIR)
        .join("backups")
        .join(name)
        .to_string_lossy()
        .into_owned())
}

fn load_catalog(branch: &str) -> Result<Catalog, String> {
    let url = format!("{REPO_RAW}/{branch}/{CATALOG_PATH}");
    let catalog: Catalog = fetch_json(&url)?;
    validate_catalog(&catalog)?;
    Ok(catalog)
}

fn find_preset<'a>(catalog: &'a Catalog, name: &str) -> Result<&'a Preset, String> {
    catalog
        .presets
        .iter()
        .find(|preset| preset.name == name)
        .ok_or_else(|| format!("preset '{name}' not found"))
}

fn validate_catalog(catalog: &Catalog) -> Result<(), String> {
    if catalog.schema_version != 1 {
        return Err(format!(
            "unsupported catalog schema version {}",
            catalog.schema_version
        ));
    }
    if catalog.presets.is_empty() {
        return Err("catalog has no presets".to_string());
    }

    let mut names = BTreeSet::new();
    for preset in &catalog.presets {
        if preset.name.is_empty() || preset.name.contains('/') {
            return Err(format!("invalid preset name '{}'", preset.name));
        }
        if !names.insert(preset.name.clone()) {
            return Err(format!("duplicate preset '{}'", preset.name));
        }
        safe_rel_path(&preset.path)?;
        if let Some(readme) = &preset.readme {
            safe_rel_path(readme)?;
        }
        if let Some(min) = &preset.xfetch_min
            && parse_version(min).is_none()
        {
            return Err(format!(
                "preset '{}' has invalid xfetch_min '{min}'",
                preset.name
            ));
        }
        if preset.files.is_empty() {
            return Err(format!("preset '{}' has no files", preset.name));
        }

        let mut targets = BTreeSet::new();
        for file in &preset.files {
            safe_rel_path(&file.source)?;
            let target = safe_rel_path(&file.target)?;
            let target_text = target.to_string_lossy();
            if target_text == INTERNAL_DIR || target_text.starts_with(&format!("{INTERNAL_DIR}/")) {
                return Err(format!(
                    "preset '{}' targets reserved path '{}'",
                    preset.name, file.target
                ));
            }
            if !targets.insert(target_text.into_owned()) {
                return Err(format!(
                    "preset '{}' has duplicate target '{}'",
                    preset.name, file.target
                ));
            }
            match file.encoding.as_deref() {
                None | Some("base64") => {}
                Some(other) => {
                    return Err(format!(
                        "preset '{}' has unsupported encoding '{other}'",
                        preset.name
                    ));
                }
            }
            let Some(sha256) = &file.sha256 else {
                return Err(format!(
                    "preset '{}' file '{}' has no sha256",
                    preset.name, file.source
                ));
            };
            if !is_hex64(sha256) {
                return Err(format!(
                    "preset '{}' file '{}' has an invalid sha256",
                    preset.name, file.source
                ));
            }
        }
    }

    Ok(())
}

fn preset_base(branch: &str, preset: &Preset) -> Result<String, String> {
    let path = safe_rel_path(&preset.path)?;
    Ok(format!(
        "{REPO_RAW}/{branch}/configshunter/{}",
        path.display()
    ))
}

fn readme_text(preset: &Preset, branch: &str) -> Result<String, String> {
    let readme = preset.readme.as_deref().unwrap_or("README.md");
    let readme = safe_rel_path(readme)?;
    let url = format!("{}/{}", preset_base(branch, preset)?, readme.display());
    fetch_text(&url)
}

fn print_requirements(preset: &Preset) {
    let requirements = &preset.requires;
    println!("requirements:");
    if !requirements.plugins.is_empty() {
        let installed = installed_addons("plugins", "xfetch-plugin-");
        println!(
            "  plugins: {}",
            status_list(&requirements.plugins, &installed)
        );
    }
    if !requirements.extensions.is_empty() {
        let installed = installed_addons("extensions", "xfetch-extension-");
        println!(
            "  extensions: {}",
            status_list(&requirements.extensions, &installed)
        );
    }
    if !requirements.fonts.is_empty() {
        println!("  fonts: {}", requirements.fonts.join(", "));
    }
    if !requirements.tools.is_empty() {
        println!("  tools: {}", requirements.tools.join(", "));
    }
    if let Some(min) = &preset.xfetch_min
        && let Some(current) = xfetch_version()
        && !version_at_least(&current, min)
    {
        println!("  warning: xfetch {current} is older than required {min}");
    }
}

fn status_list(required: &[String], installed: &BTreeSet<String>) -> String {
    required
        .iter()
        .map(|name| {
            if installed.contains(name) {
                format!("{name} [ok]")
            } else {
                format!("{name} [missing]")
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn installed_addons(directory: &str, prefix: &str) -> BTreeSet<String> {
    let mut installed = BTreeSet::new();
    let Ok(entries) = fs::read_dir(Path::new(CONFIG_ROOT).join(directory)) else {
        return installed;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(rest) = name.strip_prefix(prefix) else {
            continue;
        };
        let rest = rest.strip_suffix(".install.json").unwrap_or(rest);
        let rest = rest.strip_suffix(".json").unwrap_or(rest);
        let rest = rest.strip_suffix(".wasm").unwrap_or(rest);
        let rest = rest.strip_suffix(".exe").unwrap_or(rest);
        if !rest.is_empty() {
            installed.insert(rest.to_string());
        }
    }
    installed
}

fn xfetch_version() -> Option<String> {
    let value = host_call("version", &serde_json::Value::Null).ok()?;
    value
        .get("xfetch")
        .and_then(|version| version.as_str())
        .map(str::to_string)
}

fn fetch_bytes(url: &str) -> Result<Vec<u8>, String> {
    let response = http_request(
        "GET",
        url,
        &[("User-Agent".to_string(), USER_AGENT.to_string())],
        None,
        Some(HTTP_TIMEOUT_MS),
    )
    .map_err(|err| describe_host_error(url, err))?;

    if response.status != 200 {
        return Err(format!("GET {url} returned HTTP {}", response.status));
    }
    Ok(response.body)
}

fn fetch_text(url: &str) -> Result<String, String> {
    let bytes = fetch_bytes(url)?;
    String::from_utf8(bytes).map_err(|err| format!("GET {url} returned non-UTF-8 text: {err}"))
}

fn fetch_json<T: DeserializeOwned>(url: &str) -> Result<T, String> {
    let bytes = fetch_bytes(url)?;
    serde_json::from_slice(&bytes).map_err(|err| format!("invalid JSON from {url}: {err}"))
}

fn describe_host_error(url: &str, err: HostCallError) -> String {
    format!("request to {url} failed: {err}")
}

fn decode_base64_text(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let text =
        std::str::from_utf8(bytes).map_err(|err| format!("base64 asset is not UTF-8: {err}"))?;
    let compact: String = text
        .chars()
        .filter(|character| !character.is_ascii_whitespace())
        .collect();
    base64::engine::general_purpose::STANDARD
        .decode(compact.as_bytes())
        .map_err(|err| format!("invalid base64 asset: {err}"))
}

fn verify_sha256(bytes: &[u8], expected: &str) -> Result<(), String> {
    let actual = sha256_hex(bytes);
    if actual.eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        Err(format!(
            "sha256 mismatch: expected {expected}, got {actual}"
        ))
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(64);
    for byte in digest {
        let _ = write!(&mut output, "{byte:02x}");
    }
    output
}

fn image_magic_ok(path: &Path, bytes: &[u8]) -> bool {
    let path = path.to_string_lossy().to_ascii_lowercase();
    if path.ends_with(".jpg") || path.ends_with(".jpeg") {
        bytes.starts_with(&[0xff, 0xd8, 0xff])
    } else if path.ends_with(".png") {
        bytes.starts_with(b"\x89PNG\r\n\x1a\n")
    } else if path.ends_with(".gif") {
        bytes.starts_with(b"GIF8")
    } else if path.ends_with(".webp") {
        bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP"
    } else {
        true
    }
}

fn safe_rel_path(value: &str) -> Result<&Path, String> {
    if value.is_empty() {
        return Err("empty path in catalog".to_string());
    }
    let path = Path::new(value);
    if path.is_absolute() {
        return Err(format!("absolute path not allowed: '{value}'"));
    }
    if !path
        .components()
        .all(|component| matches!(component, Component::Normal(_)))
    {
        return Err(format!("path traversal not allowed: '{value}'"));
    }
    Ok(path)
}

fn validate_branch(branch: &str) -> Result<(), String> {
    if branch.is_empty() || branch.starts_with('/') || branch.ends_with('/') {
        return Err(format!("invalid branch '{branch}'"));
    }
    if branch.split('/').any(|segment| {
        segment.is_empty()
            || segment == "."
            || segment == ".."
            || !segment.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
            })
    }) {
        return Err(format!("invalid branch '{branch}'"));
    }
    Ok(())
}

fn parse_version(value: &str) -> Option<(u64, u64, u64)> {
    let value = value.trim().trim_start_matches('v');
    let mut parts = value.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().unwrap_or("0").parse().ok()?;
    let patch = parts
        .next()
        .unwrap_or("0")
        .split(|character: char| !character.is_ascii_digit())
        .next()?
        .parse()
        .ok()?;
    Some((major, minor, patch))
}

fn version_at_least(current: &str, minimum: &str) -> bool {
    match (parse_version(current), parse_version(minimum)) {
        (Some(current), Some(minimum)) => current >= minimum,
        _ => true,
    }
}

fn is_hex64(value: &str) -> bool {
    value.len() == 64 && value.chars().all(|character| character.is_ascii_hexdigit())
}

fn timestamp_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
}

fn human_bytes(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.2} MiB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.1} KiB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_install_arguments() {
        assert_eq!(parse_action(&[]).unwrap(), Action::Help);
        assert_eq!(
            parse_action(&["install".to_string(), "cyberpunk".to_string()]).unwrap(),
            Action::Install {
                name: "cyberpunk".to_string(),
                branch: DEFAULT_BRANCH.to_string(),
                backup: true,
            }
        );
        assert_eq!(
            parse_action(&[
                "install".to_string(),
                "cyberpunk".to_string(),
                "--branch".to_string(),
                "feat/configshunter".to_string(),
                "--no-backup".to_string(),
            ])
            .unwrap(),
            Action::Install {
                name: "cyberpunk".to_string(),
                branch: "feat/configshunter".to_string(),
                backup: false,
            }
        );
    }

    #[test]
    fn rejects_unsafe_paths() {
        assert!(safe_rel_path("fetchs/cyberpunk/config.jsonc").is_ok());
        assert!(safe_rel_path("../config.jsonc").is_err());
        assert!(safe_rel_path("/etc/passwd").is_err());
        assert!(safe_rel_path("fetchs/../../config").is_err());
        assert!(safe_rel_path("").is_err());
    }

    #[test]
    fn rejects_invalid_branches() {
        assert!(validate_branch("main").is_ok());
        assert!(validate_branch("feat/configshunter").is_ok());
        assert!(validate_branch("../main").is_err());
        assert!(validate_branch("bad branch").is_err());
        assert!(validate_branch("").is_err());
    }

    #[test]
    fn decodes_base64_with_whitespace() {
        let encoded = b"aGVsbG8g\nY29uZmlnaHVudGVy\n";
        assert_eq!(decode_base64_text(encoded).unwrap(), b"hello confighunter");
    }

    #[test]
    fn hashes_and_verifies() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert!(
            verify_sha256(
                b"abc",
                "BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD"
            )
            .is_ok()
        );
        assert!(verify_sha256(b"abc", "00").is_err());
    }

    #[test]
    fn checks_image_magic() {
        assert!(image_magic_ok(
            Path::new("a.jpg"),
            &[0xff, 0xd8, 0xff, 0xe0]
        ));
        assert!(!image_magic_ok(Path::new("a.jpg"), b"not an image"));
        assert!(image_magic_ok(Path::new("a.png"), b"\x89PNG\r\n\x1a\nrest"));
        assert!(image_magic_ok(Path::new("a.txt"), b"plain text"));
    }

    #[test]
    fn compares_versions() {
        assert!(version_at_least("1.0.4", "1.0.4"));
        assert!(version_at_least("1.1.0", "1.0.4"));
        assert!(!version_at_least("1.0.3", "1.0.4"));
        assert!(version_at_least("v2.0.0", "1.0.4"));
    }

    #[test]
    fn formats_requirement_status() {
        let installed = BTreeSet::from(["banner".to_string()]);
        let required = vec!["banner".to_string(), "gauges".to_string()];
        assert_eq!(
            status_list(&required, &installed),
            "banner [ok], gauges [missing]"
        );
    }

    #[test]
    fn parses_catalog_preset() {
        let catalog: Catalog = serde_json::from_str(
            r#"{
                "schema_version": 1,
                "presets": [{
                    "name": "cyberpunk",
                    "version": "1.0.0",
                    "description": "test",
                    "path": "cyberpunk",
                    "readme": "README.md",
                    "xfetch_min": "1.0.4",
                    "requires": {"plugins": ["banner"]},
                    "files": [{
                        "source": "fetchs/cyberpunk/images/a.b64",
                        "target": "fetchs/cyberpunk/images/a.jpg",
                        "encoding": "base64",
                        "sha256": "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
                        "size": 3
                    }]
                }]
            }"#,
        )
        .unwrap();
        validate_catalog(&catalog).unwrap();
        assert_eq!(
            catalog.presets[0].files[0].target,
            "fetchs/cyberpunk/images/a.jpg"
        );
    }
}
