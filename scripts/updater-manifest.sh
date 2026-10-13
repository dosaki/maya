#!/bin/sh
# Prints the release's updater feed, latest.json, in Tauri's static
# manifest format: for each desktop platform the asset's download URL and
# the contents of its .sig. Fails, naming it, when any asset or .sig is
# missing, so no release goes out that would break updates.
# Needs jq. REPO (default dosaki/maya) and PUB_DATE (default now) override.
# Usage: sh scripts/updater-manifest.sh <version> <dir> > <dir>/latest.json
set -eu
v="$1"
dir="$2"
repo="${REPO:-dosaki/maya}"
pub_date="${PUB_DATE:-$(date -u +%Y-%m-%dT%H:%M:%SZ)}"
base="https://github.com/$repo/releases/download/v$v"
platforms='{}'
for pair in \
  "darwin-aarch64 Maya_${v}_aarch64.app.tar.gz" \
  "darwin-x86_64 Maya_${v}_x86_64.app.tar.gz" \
  "windows-x86_64 Maya_${v}_x64-setup.exe" \
  "linux-x86_64 Maya_${v}_amd64.AppImage" \
  "linux-aarch64 Maya_${v}_arm64.AppImage"; do
  platform="${pair%% *}"
  asset="${pair#* }"
  for f in "$asset" "$asset.sig"; do
    [ -f "$dir/$f" ] || { echo "updater-manifest: $f is missing from $dir" >&2; exit 1; }
  done
  platforms="$(printf '%s' "$platforms" | jq --arg p "$platform" --arg url "$base/$asset" --rawfile sig "$dir/$asset.sig" \
    '.[$p] = { signature: ($sig | rtrimstr("\n")), url: $url }')"
done
jq -n --arg v "$v" --arg notes "https://github.com/$repo/releases/tag/v$v" --arg date "$pub_date" --argjson platforms "$platforms" \
  '{ version: $v, notes: $notes, pub_date: $date, platforms: $platforms }'
