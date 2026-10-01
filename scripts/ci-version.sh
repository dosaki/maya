#!/bin/sh
# What a release-workflow run does, for its jobs to share. Prints three
# lines for $GITHUB_OUTPUT:
#   version=<the version in src-tauri/tauri.conf.json>
#   release=true|false  publish: a push to main whose version has no release
#   build=true|false    build every binary: a release, or any pull request
# A pull request builds everything and never publishes. Fails, through
# release-version.sh, when the version files disagree.
# Needs EVENT (github.event_name), node and an authenticated gh (GH_TOKEN).
# Usage: EVENT=pull_request sh scripts/ci-version.sh >> "$GITHUB_OUTPUT"
set -eu
cd "$(dirname "$0")/.."
out="$(sh scripts/release-version.sh)"
version="$(printf '%s\n' "$out" | sed -n 's/^version=//p')"
release="$(printf '%s\n' "$out" | sed -n 's/^release=//p')"
case "${EVENT:-}" in
  pull_request) release=false; build=true ;;
  push) build="$release" ;;
  *) release=false; build=false ;;
esac
echo "version=$version"
echo "release=$release"
echo "build=$build"
