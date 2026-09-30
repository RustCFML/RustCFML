#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

echo "==> Building RHEL 9/10 Target Binary..."
cargo build --release --target x86_64-unknown-linux-gnu -p rustcfml-cli

echo "==> Packaging RPM via cargo-generate-rpm..."
cargo generate-rpm --metadata-overwrite scripts/rpm/rustcfml.toml

echo "==> RPM Package created successfully at target/generate-rpm/"
