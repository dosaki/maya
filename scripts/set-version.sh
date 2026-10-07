#!/bin/sh
# Sets the app version everywhere it is carried (package.json, both
# tauri.conf.json files, Cargo.toml and Cargo.lock), so a push to main
# publishes release v<version>. Usage: sh scripts/set-version.sh 0.2.0
set -eu
cd "$(dirname "$0")/.."
v="${1:-}"
case "$v" in
  [0-9]*.[0-9]*.[0-9]*) ;;
  *) echo "usage: sh scripts/set-version.sh <major.minor.patch>" >&2; exit 1 ;;
esac
# `-i.bak` works with both BSD (macOS) and GNU (Linux, Git Bash) sed.
sed -i.bak -E "s/^(  \"version\": \")[^\"]+(\",)$/\1$v\2/" package.json src-tauri/tauri.conf.json mobile/tauri.conf.json
# The workspace root carries the version; every crate inherits it.
sed -i.bak -E "s/^(version = \")[^\"]+(\")$/\1$v\2/" Cargo.toml
rm -f package.json.bak src-tauri/tauri.conf.json.bak mobile/tauri.conf.json.bak Cargo.toml.bak
# Cargo.lock records the crates' own versions too: every workspace member.
if [ -f Cargo.lock ]; then
  cargo update --workspace --offline >/dev/null 2>&1 || cargo generate-lockfile --offline >/dev/null 2>&1 || true
fi
grep -H '"version"' package.json src-tauri/tauri.conf.json mobile/tauri.conf.json
grep -H '^version' Cargo.toml
