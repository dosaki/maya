#!/bin/sh
# Builds Maya's ear for the given target triple (default: this machine).
set -eu
cd "$(dirname "$0")/.."
target="${1:-$(rustc -vV | sed -n 's/^host: //p')}"
case "$target" in
  aarch64-apple-darwin) arch=arm64 ;;
  x86_64-apple-darwin) arch=x86_64 ;;
  *-pc-windows-msvc)
    # Windows: the Rust ear (whisper.cpp built in) and the hook, named the
    # way Tauri looks for sidecars.
    mkdir -p src-tauri/binaries
    cargo build --release --target "$target" -p maya-ear -p maya-hook
    for bin in maya-ear maya-hook; do
      cp "target/$target/release/$bin.exe" "src-tauri/binaries/$bin-$target.exe"
      echo "built src-tauri/binaries/$bin-$target.exe"
    done
    exit 0 ;;
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
