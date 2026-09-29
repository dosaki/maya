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
sh scripts/fetch-whisper.sh
slice="$PWD/vendor/whisper.xcframework/macos-arm64_x86_64"
mkdir -p src-tauri/binaries
swiftc -O -target "$arch-apple-macos14.0" \
  -framework Speech -framework AVFoundation -framework CoreAudio \
  -F "$slice" -framework whisper \
  -Xlinker -rpath -Xlinker "@executable_path/../Frameworks" \
  -Xlinker -rpath -Xlinker "$slice" \
  -o "src-tauri/binaries/maya-ear-$target" ear/vad.swift ear/main.swift
echo "built src-tauri/binaries/maya-ear-$target"
