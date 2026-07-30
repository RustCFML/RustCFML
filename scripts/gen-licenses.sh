#!/usr/bin/env bash
# Regenerate THIRD-PARTY.txt — the third-party attribution notice that ships
# with every release binary (attached to the GitHub release AND embedded in the
# binary, reachable via `rustcfml --licenses`).
#
# THIRD-PARTY.txt is a COMMITTED, GENERATED file: crates/cli/src/lib.rs pulls it
# in with include_str!, so it must exist at compile time. Run this script after
# any dependency change and commit the result. CI verifies it is up to date.
#
# Requires cargo-about 0.9.1 (the version pinned in the release workflow — a
# different version may format the output differently and trip CI's staleness
# diff):
#
#   cargo install cargo-about --locked --features cli --version 0.9.1
set -euo pipefail

cd "$(dirname "$0")/.."

if ! command -v cargo-about >/dev/null 2>&1; then
    echo "error: cargo-about not found." >&2
    echo "       cargo install cargo-about --locked --features cli" >&2
    exit 1
fi

# Default features only — that is what release binaries are built with. See the
# note in about.toml about --all-features and the CDDL-1.0 `inferno` crate.
cargo about generate --config about.toml about.hbs -o THIRD-PARTY.txt

echo "Wrote THIRD-PARTY.txt ($(wc -l < THIRD-PARTY.txt | tr -d ' ') lines)"
