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
cargo about generate --config about.toml about.hbs -o THIRD-PARTY.txt.raw

# Collapse runs of blank lines to a single blank line.
#
# WHY: crates that ship several licence files (miniz_oxide has LICENSE,
# LICENSE-APACHE.md, LICENSE-MIT.md and LICENSE-ZLIB.md) get their text
# harvested in filesystem-iteration order, and the number of blank lines
# joining the harvested pieces differs between APFS (dev machines) and ext4
# (CI runners). That made the generated file differ by a single blank line
# between macOS and Linux, which is legally meaningless but broke the
# byte-exact staleness gate in .github/workflows/licenses.yml.
#
# Squeezing removes the EXTRA blank lines. The opposite also happens: a
# licence text harvested without its trailing blank line runs straight into
# the next entry's opening rule (v0.751.1: bit_field 0.10.3 on Linux only, a
# file macOS generates WITH the line), and which way it falls flips with the
# dependency set — so hand-editing the file to match CI only moved the
# failure (18737bce broke main that way). The awk pass therefore also puts
# the blank line back before every entry's OPENING rule: a line of 80 `=`
# whose next line is not blank (the closing rule after "Applies to" is
# followed by a blank line and is left alone).
#
# Together these make the artifact platform-independent. They only ever add or
# remove empty lines — no licence wording, copyright line or attribution is
# altered.
cat -s THIRD-PARTY.txt.raw | awk '
    function is_rule(s) { return s ~ /^={80}$/ }
    NR > 1 {
        if (is_rule(prev) && $0 != "" && prev_prev != "" && NR > 2) print ""
        print prev
    }
    { prev_prev = (NR > 1 ? prev : ""); prev = $0 }
    END { if (NR > 0) print prev }
' > THIRD-PARTY.txt
rm -f THIRD-PARTY.txt.raw

echo "Wrote THIRD-PARTY.txt ($(wc -l < THIRD-PARTY.txt | tr -d ' ') lines)"
