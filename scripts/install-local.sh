#!/usr/bin/env bash
#
# install-local.sh — build (release) and install the rustcfml binary for the
# current host OS onto the PATH, so `rustcfml` resolves to the just-released
# build instead of a stale one.
#
# Run this as the FINAL step of a release, after commit + tag + push. It is a
# no-network, host-native build: cargo compiles for the host target by default,
# so the produced binary is always the "correct local OS" one (rustcfml on
# Unix, rustcfml.exe on Windows).
#
# Install destination resolution order:
#   1. $RUSTCFML_INSTALL_DIR (if set)
#   2. the directory of an existing `rustcfml` already on PATH (so we overwrite
#      the one the user actually invokes)
#   3. `cargo`'s bin dir: ${CARGO_HOME:-$HOME/.cargo}/bin
#
# Usage:
#   scripts/install-local.sh            # build --release then install
#   scripts/install-local.sh --no-build # install the existing target/release build
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

BIN_NAME="rustcfml"
case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*) BIN_NAME="rustcfml.exe" ;;
esac

if [[ "${1:-}" != "--no-build" ]]; then
  echo "==> Building release binary for host OS ($(uname -s) $(uname -m))..."
  cargo build --release
fi

SRC="$REPO_ROOT/target/release/$BIN_NAME"
if [[ ! -x "$SRC" ]]; then
  echo "ERROR: built binary not found at $SRC" >&2
  exit 1
fi

# Resolve install directory.
if [[ -n "${RUSTCFML_INSTALL_DIR:-}" ]]; then
  DEST_DIR="$RUSTCFML_INSTALL_DIR"
elif command -v "$BIN_NAME" >/dev/null 2>&1; then
  DEST_DIR="$(dirname "$(command -v "$BIN_NAME")")"
else
  DEST_DIR="${CARGO_HOME:-$HOME/.cargo}/bin"
fi
mkdir -p "$DEST_DIR"

DEST="$DEST_DIR/$BIN_NAME"
echo "==> Installing $SRC -> $DEST"
install -m 0755 "$SRC" "$DEST"

# macOS: copying over an existing binary invalidates its code signature, and the
# kernel then SIGKILLs it on exec (exit 137, empty --version output). The bytes
# are identical, so a checksum comparison reports a perfectly successful install
# of a binary that cannot run. Re-sign ad-hoc.
if [[ "$(uname -s)" == "Darwin" ]] && command -v codesign >/dev/null 2>&1; then
  echo "==> Re-signing (ad-hoc) for macOS"
  codesign --force -s - "$DEST"
fi

echo "==> Installed version now on PATH:"
hash -r 2>/dev/null || true
# Verify it actually EXECUTES — see the code-signing note above.
if ! "$DEST" --version; then
  echo "ERROR: installed binary failed to run (exit $?)." >&2
  [[ "$(uname -s)" == "Darwin" ]] && echo "       On macOS this is usually a code-signing failure; try: codesign --force -s - '$DEST'" >&2
  exit 1
fi
