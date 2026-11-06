//! `github`: show GitHub repository information as plain text lines.
//!
//! The core sends a JSON command request on stdin; this guest reads the first
//! positional argument as `owner/repo` or a GitHub URL, fetches repository and
//! language metadata through the sandboxed HTTP host call, and prints one
//! `key: value` line per field.

use serde::Deserialize;
use serde_json::Value;
use std::io::Read;
use xfetch_guest_api::{HostCallError, http_request, log};

const REQUEST_TIMEOUT_MS: u64 = 5_000;

#[derive(Debug, Deserialize)]
struct CommandRequest {
    #[serde(default = "default_version")]
    version: u32,
    #[serde(default = "default_kind")]
    kind: String,
    #[serde(default)]
    args: Vec<String>,
}

fn default_version() -> u32 {
    1
}

fn default_kind() -> String {
    "command".to_string()
}

fn main() {
    let request = match read_request() {
        Ok(request) => request,
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(1);
        }
    };

    if request.version != 1 || request.kind != "command" {
        eprintln!(
            "unsupported command request: version {} kind '{}'",
            request.version, request.kind
        );
        std::process::exit(1);
    }

    let Some(source) = request.args.first() else {
        eprintln!("usage: xfetch run github -- <owner/repo | github-url>");
        std::process::exit(1);
    };

    let (owner, repo) = match parse_repo(source) {
        Ok(repo) => repo,
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(1);
        }
    };

    match fetch_repo(&owner, &repo) {
        Ok(lines) => {
            for line in lines {
                println!("{line}");
            }
        }
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(1);
        }
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

/// Accepts `owner/repo`, `https://github.com/owner/repo` and
/// `git@github.com:owner/repo.git`.
fn parse_repo(value: &str) -> Result<(String, String), String> {
    let value = value.trim();
    let value = value
        .strip_prefix("https://github.com/")
        .or_else(|| value.strip_prefix("http://github.com/"))
        .or_else(|| value.strip_prefix("git@github.com:"))
        .unwrap_or(value);
    let mut parts = value.split('/').filter(|part| !part.is_empty());
    let owner = parts
        .next()
        .ok_or_else(|| format!("invalid repository '{}': expected owner/repo", value))?;
    let repo = parts
        .next()
        .ok_or_else(|| format!("invalid repository '{}': expected owner/repo", value))?;
    let repo = repo.strip_suffix(".git").unwrap_or(repo);
    if owner.is_empty() || repo.is_empty() {
        return Err(format!(
            "invalid repository '{}': expected owner/repo",
            value
        ));
    }
    Ok((owner.to_string(), repo.to_string()))
}

fn fetch_repo(owner: &str, repo: &str) -> Result<Vec<String>, String> {
    let base = format!("https://api.github.com/repos/{owner}/{repo}");
    let headers = vec![
        (
            "User-Agent".to_string(),
            "xfetch-command-github".to_string(),
        ),
        (
            "Accept".to_string(),
            "application/vnd.github+json".to_string(),
        ),
        ("X-GitHub-Api-Version".to_string(), "2022-11-28".to_string()),
    ];

    log("info", &format!("github: fetching {}", base));
    let repo_value = fetch_json(&base, &headers)?;
    let languages = fetch_json(&format!("{base}/languages"), &headers).ok();

    Ok(format_repo(&repo_value, languages.as_ref()))
}

fn fetch_json(url: &str, headers: &[(String, String)]) -> Result<Value, String> {
    let response = http_request("GET", url, headers, None, Some(REQUEST_TIMEOUT_MS))
        .map_err(|err| describe_host_error(url, err))?;
    if response.status != 200 {
        return Err(format!(
            "GitHub API returned HTTP {} for {}",
            response.status, url
        ));
    }
    serde_json::from_slice(&response.body)
        .map_err(|err| format!("invalid GitHub response for {}: {}", url, err))
}

fn describe_host_error(url: &str, err: HostCallError) -> String {
    format!("request to {} failed: {}", url, err)
}

fn format_repo(repo: &Value, languages: Option<&Value>) -> Vec<String> {
    let full_name = string_field(repo, "full_name").unwrap_or_else(|| "unknown".to_string());
    let author = repo
        .get("owner")
        .and_then(|owner| owner.get("login"))
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let language_list = languages
        .map(format_languages)
        .unwrap_or_else(|| "unknown".to_string());
    let stars = number_field(repo, "stargazers_count");
    let forks = number_field(repo, "forks_count");
    let watchers = number_field(repo, "subscribers_count");
    let issues = number_field(repo, "open_issues_count");
    let license = repo
        .get("license")
        .and_then(|license| {
            license
                .get("spdx_id")
                .and_then(Value::as_str)
                .or_else(|| license.get("name").and_then(Value::as_str))
        })
        .unwrap_or("none");
    let updated = string_field(repo, "updated_at").unwrap_or_else(|| "unknown".to_string());
    let url = string_field(repo, "html_url").unwrap_or_else(|| "unknown".to_string());
    let description = string_field(repo, "description").unwrap_or_else(|| "none".to_string());

    vec![
        format!("repo: {full_name}"),
        format!("author: {author}"),
        format!("languages: {language_list}"),
        format!("stars: {stars}"),
        format!("forks: {forks}"),
        format!("watchers: {watchers}"),
        format!("open issues: {issues}"),
        format!("license: {license}"),
        format!("updated: {updated}"),
        format!("url: {url}"),
        format!("description: {description}"),
    ]
}

fn format_languages(value: &Value) -> String {
    let Some(languages) = value.as_object() else {
        return "unknown".to_string();
    };
    let mut entries: Vec<(&str, u64)> = languages
        .iter()
        .filter_map(|(name, bytes)| Some((name.as_str(), bytes.as_u64()?)))
        .collect();
    entries.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(right.0)));
    if entries.is_empty() {
        "none".to_string()
    } else {
        entries
            .into_iter()
            .map(|(name, _)| name)
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn string_field(value: &Value, field: &str) -> Option<String> {
    value.get(field).and_then(Value::as_str).map(str::to_string)
}

fn number_field(value: &Value, field: &str) -> String {
    value
        .get(field)
        .and_then(Value::as_u64)
        .map(|number| number.to_string())
        .unwrap_or_else(|| "0".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_owner_repo_forms() {
        assert_eq!(
            parse_repo("rust-lang/rust").unwrap(),
            ("rust-lang".to_string(), "rust".to_string())
        );
        assert_eq!(
            parse_repo("https://github.com/rust-lang/rust.git").unwrap(),
            ("rust-lang".to_string(), "rust".to_string())
        );
        assert_eq!(
            parse_repo("git@github.com:rust-lang/rust.git").unwrap(),
            ("rust-lang".to_string(), "rust".to_string())
        );
        assert!(parse_repo("rust-lang").is_err());
    }

    #[test]
    fn formats_repository_lines() {
        let repo = json!({
            "full_name": "rust-lang/rust",
            "owner": { "login": "rust-lang" },
            "stargazers_count": 100,
            "forks_count": 20,
            "subscribers_count": 5,
            "open_issues_count": 7,
            "license": { "spdx_id": "MIT" },
            "updated_at": "2026-11-06T00:00:00Z",
            "html_url": "https://github.com/rust-lang/rust",
            "description": "Empowering everyone"
        });
        let languages = json!({ "Rust": 1000, "Shell": 10 });
        let lines = format_repo(&repo, Some(&languages));
        assert!(lines.contains(&"repo: rust-lang/rust".to_string()));
        assert!(lines.contains(&"author: rust-lang".to_string()));
        assert!(lines.contains(&"languages: Rust, Shell".to_string()));
        assert!(lines.contains(&"stars: 100".to_string()));
        assert!(lines.contains(&"license: MIT".to_string()));
    }
}
