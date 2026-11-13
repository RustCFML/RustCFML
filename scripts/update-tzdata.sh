#!/usr/bin/env bash
# Refresh the IANA tz database bundled into the vendored chrono-tz
# (vendor/chrono-tz/tz). chrono-tz compiles these files into the binary at
# build time, and its crates.io releases lag IANA by months — 0.10.4 shipped
# 2025b, which predates BC (America/Vancouver) dropping DST in Nov 2026.
#
# Usage: scripts/update-tzdata.sh [version]   (default: latest, e.g. 2026f)
set -euo pipefail

cd "$(dirname "$0")/.."
version="${1:-$(curl -fsS https://data.iana.org/time-zones/tzdb/version)}"
dest="vendor/chrono-tz/tz"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

echo "Fetching tzdata ${version}"
curl -fsS -o "$tmp/tzdata.tar.gz" \
  "https://data.iana.org/time-zones/releases/tzdata${version}.tar.gz"
tar -xzf "$tmp/tzdata.tar.gz" -C "$tmp"

# The files chrono-tz-build reads (its FILES list), plus NEWS (the version is
# read from it) and LICENSE.
files="africa antarctica asia australasia backward etcetera europe northamerica southamerica NEWS LICENSE"
for f in $files; do
  cp "$tmp/$f" "$dest/$f"
done

echo "vendor/chrono-tz/tz now holds tzdata ${version}"
