#!/bin/sh
# Builds Maya's ear for the given target triple (default: this machine).
set -eu
cd "$(dirname "$0")/.."
target="${1:-$(rustc -vV | sed -n 's/^host: //p')}"
case "$target" in
  aarch64-apple-darwin) arch=arm64 ;;
  x86_64-apple-darwin) arch=x86_64 ;;
  *) echo "unsupported target $target" >&2; exit 1 ;;
esac
mkdir -p src-tauri/binaries
swiftc -O -target "$arch-apple-macos14.0" -framework Speech -framework AVFoundation -framework CoreAudio \
  -o "src-tauri/binaries/maya-ear-$target" ear/main.swift
echo "built src-tauri/binaries/maya-ear-$target"
