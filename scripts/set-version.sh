#!/bin/sh
# Sets the app version in the three places that carry it, so a push to main
# publishes release v<version>. Usage: sh scripts/set-version.sh 0.2.0
set -eu
cd "$(dirname "$0")/.."
v="${1:-}"
case "$v" in
  [0-9]*.[0-9]*.[0-9]*) ;;
  *) echo "usage: sh scripts/set-version.sh <major.minor.patch>" >&2; exit 1 ;;
esac
sed -i '' -E "s/^(  \"version\": \")[^\"]+(\",)$/\1$v\2/" package.json src-tauri/tauri.conf.json
sed -i '' -E "s/^(version = \")[^\"]+(\")$/\1$v\2/" src-tauri/Cargo.toml
# Cargo.lock records the crate's own version too.
if [ -f src-tauri/Cargo.lock ]; then
  (cd src-tauri && cargo update -p maya --offline >/dev/null 2>&1 || cargo generate-lockfile --offline >/dev/null 2>&1 || true)
fi
grep -H '"version"' package.json src-tauri/tauri.conf.json
grep -H '^version' src-tauri/Cargo.toml
