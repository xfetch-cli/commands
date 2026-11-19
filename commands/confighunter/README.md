# confighunter command

Install complete xfetch configuration presets from
[`xfetch-cli/configs`](https://github.com/xfetch-cli/configs).

The command downloads `configshunter/index.json`, stages every preset file,
verifies its SHA-256 digest, decodes base64 image assets to their real
extensions, backs up the files it replaces and prints the preset README with a
requirement checklist.

## Usage

```bash
xfetch command install confighunter
xfetch run confighunter list
xfetch run confighunter info cyberpunk
xfetch run confighunter install cyberpunk
```

Options:

```bash
xfetch run confighunter install cyberpunk --branch <branch>
xfetch run confighunter install cyberpunk --no-backup
```

`--branch` is useful for testing a configuration pull request before it is
merged. The default branch is `main`.

## What it installs

A preset mirrors `~/.config/xfetch`:

```text
config.jsonc
themes/
logos/
fetchs/<preset>/
```

Files are installed relative to `~/.config/xfetch`. Base64 sources named
`*.b64` are decoded to the target extension declared in the preset index; the
cyberpunk preset stores `01-neon-tokyo.b64` and installs
`01-neon-tokyo.jpg`.

## Safety

- Every file is staged under `~/.config/xfetch/.confighunter-staging-*` and
  verified before anything is replaced.
- Existing files are copied to
  `~/.config/xfetch/.confighunter/backups/<timestamp>/` before the staged
  files are moved into place. Use `--no-backup` to skip this.
- The catalog rejects absolute paths, `..`, reserved `.confighunter` targets
  and duplicate targets.
- The wasm manifest only allows reads from the `xfetch-cli/configs` raw
  endpoint and read-write access to `~/.config/xfetch`.

## License

GPL-3.0-only. See [LICENSE](LICENSE).
