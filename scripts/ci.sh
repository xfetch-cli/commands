#!/usr/bin/env bash
# Local CI for the command addons.
set -euo pipefail
cd "$(dirname "$0")/.."

cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

if rustup target list --installed 2>/dev/null | grep -q '^wasm32-wasip1$'; then
  cargo build --release --target wasm32-wasip1 -p xfetch-command-github
else
  echo "skip wasm build: wasm32-wasip1 target is not installed"
fi

echo "==> commands CI OK"
