# Changelog

## 0.2.0

- Added the `confighunter` command addon, which installs complete presets from
  [`xfetch-cli/configs`](https://github.com/xfetch-cli/configs) with SHA-256
  verification, base64 image decoding and backups.

## 0.1.1

- Migrated `xfetch-guest-api` to 0.3.1.
- SDK dependencies now resolve from crates.io; the temporary git branch is no
  longer required.

## 0.1.0

- Initial catalog with the `github` command addon, including a prebuilt
  `dist/github.wasm` artifact.
