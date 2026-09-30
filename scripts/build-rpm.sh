#!/usr/bin/env bash

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

TARGET="x86_64-unknown-linux-gnu"
PROFILE="release-pgo"
BINARY="target/${TARGET}/${PROFILE}/rustcfml"

echo "==> Building RustCFML RHEL hardened binary"

cargo build \
  --locked \
  --profile "$PROFILE" \
  --target "$TARGET" \
  -p rustcfml-cli

echo "==> Stripping binary"

strip -s "$BINARY"

echo "==> Inspecting binary"

file "$BINARY"

readelf -h "$BINARY"
readelf -l "$BINARY"

echo "==> Dynamic dependencies"

ldd "$BINARY"

echo "==> Generating SHA-256"

sha256sum "$BINARY" \
  > rustcfml-rhel-x86_64.sha256

echo "==> Generating RPM"

cargo generate-rpm \
  -p crates/cli \
  --profile "$PROFILE" \
  --target "$TARGET" \
  --metadata-overwrite scripts/rpm/rustcfml.toml

echo "==> RPM created"

find target -name '*.rpm' -print
