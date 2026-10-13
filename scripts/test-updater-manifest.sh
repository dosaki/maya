#!/bin/sh
# Checks scripts/updater-manifest.sh against fake release assets: the JSON
# it prints for a complete set, and that a missing .sig fails it.
# Needs jq. Usage: sh scripts/test-updater-manifest.sh
set -eu
cd "$(dirname "$0")/.."
dir="$(mktemp -d)"
trap 'rm -rf "$dir"' EXIT
v=0.19.0
for a in "Maya_${v}_aarch64.app.tar.gz" "Maya_${v}_x86_64.app.tar.gz" "Maya_${v}_x64-setup.exe" "Maya_${v}_amd64.AppImage" "Maya_${v}_arm64.AppImage"; do
  : > "$dir/$a"
  printf 'sig-of-%s\n' "$a" > "$dir/$a.sig"
done

out="$(PUB_DATE=2026-10-20T10:00:00Z sh scripts/updater-manifest.sh "$v" "$dir")"
check() {
  got="$(printf '%s' "$out" | jq -r "$1")"
  [ "$got" = "$2" ] || { echo "FAIL: $1 is '$got', expected '$2'" >&2; exit 1; }
}
check .version "$v"
check .notes "https://github.com/dosaki/maya/releases/tag/v$v"
check .pub_date 2026-10-20T10:00:00Z
check '.platforms | keys | join(",")' "darwin-aarch64,darwin-x86_64,linux-aarch64,linux-x86_64,windows-x86_64"
check '.platforms["windows-x86_64"].url' "https://github.com/dosaki/maya/releases/download/v$v/Maya_${v}_x64-setup.exe"
check '.platforms["linux-aarch64"].signature' "sig-of-Maya_${v}_arm64.AppImage"
check '.platforms["darwin-x86_64"].url' "https://github.com/dosaki/maya/releases/download/v$v/Maya_${v}_x86_64.app.tar.gz"

rm "$dir/Maya_${v}_amd64.AppImage.sig"
if sh scripts/updater-manifest.sh "$v" "$dir" > /dev/null 2> "$dir/err"; then
  echo "FAIL: a missing .sig did not fail the manifest" >&2
  exit 1
fi
grep -q "Maya_${v}_amd64.AppImage.sig" "$dir/err" || { echo "FAIL: the error does not name the missing .sig" >&2; cat "$dir/err" >&2; exit 1; }
echo "updater-manifest: ok"
