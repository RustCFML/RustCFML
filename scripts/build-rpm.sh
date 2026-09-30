#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

echo "==> Compiling RHEL target binary..."
cargo build --release --target x86_64-unknown-linux-gnu -p rustcfml-cli

echo "==> Stripping release binary..."
strip -s target/x86_64-unknown-linux-gnu/release/rustcfml

echo "==> Generating RPM artifact..."
cargo generate-rpm --metadata-overwrite scripts/rpm/rustcfml.toml

echo "==> RPM build complete: target/generate-rpm/"
