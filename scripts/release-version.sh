#!/bin/sh
# Decides whether this push publishes a release, for the release workflow's
# jobs to share. Prints two lines for $GITHUB_OUTPUT:
#   version=<the version in src-tauri/tauri.conf.json>
#   release=true    when no release v<version> exists yet, else release=false
# Fails when tauri.conf.json, package.json and Cargo.toml disagree.
# Needs node and an authenticated gh (GH_TOKEN).
# Usage: sh scripts/release-version.sh >> "$GITHUB_OUTPUT"
set -eu
cd "$(dirname "$0")/.."
version="$(node -p "require('./src-tauri/tauri.conf.json').version")"
pkg="$(node -p "require('./package.json').version")"
cargo_version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)"
mobile_version="$(node -p "require('./mobile/tauri.conf.json').version")"
if [ "$version" != "$pkg" ] || [ "$version" != "$cargo_version" ] || [ "$version" != "$mobile_version" ]; then
  echo "version mismatch: tauri.conf.json=$version package.json=$pkg Cargo.toml=$cargo_version mobile/tauri.conf.json=$mobile_version" >&2
  echo "run: sh scripts/set-version.sh $version" >&2
  exit 1
fi
echo "version=$version"
if gh release view "v$version" >/dev/null 2>&1; then
  echo "v$version is already released; bump the version to publish another" >&2
  echo "release=false"
else
  echo "release=true"
fi
