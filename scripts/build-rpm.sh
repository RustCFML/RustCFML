#!/usr/bin/env bash

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

cd "$REPO_ROOT"

TARGET="x86_64-unknown-linux-gnu"
PROFILE="release-pgo"
BINARY="target/${TARGET}/${PROFILE}/rustcfml"

echo "==> Building hardened RHEL-compatible RustCFML binary..."

cargo build \
  --locked \
  --profile "$PROFILE" \
  --target "$TARGET" \
  -p rustcfml-cli

echo "==> Stripping production binary..."

strip -s "$BINARY"

echo "==> Inspecting binary..."

file "$BINARY"
readelf -h "$BINARY"
readelf -l "$BINARY"

echo "==> Generating SHA-256..."

sha256sum "$BINARY" > rustcfml-rhel-x86_64.sha256

echo "==> Generating RPM..."

cargo generate-rpm \
  -p crates/cli \
  --profile "$PROFILE" \
  --target "$TARGET" \
  --metadata-overwrite scripts/rpm/rustcfml.toml

echo "==> RPM contents..."

rpm -qpl target/${TARGET}/generate-rpm/*.rpm

echo "==> RPM metadata..."

rpm -qpi target/${TARGET}/generate-rpm/*.rpm

echo "==> RPM build complete."
