#!/bin/sh
# Fetches the pinned whisper.cpp xcframework into vendor/ (git-ignored).
set -eu
cd "$(dirname "$0")/.."
version="v1.9.2"
sha="af74fed13ea7f2d5ca2a39d9f58ec177713fafd7cab63aef4e27b79f3ceca80b"
zip="vendor/whisper-$version-xcframework.zip"
dest="vendor/whisper.xcframework"
if [ -d "$dest/macos-arm64_x86_64/whisper.framework" ]; then
  echo "whisper.xcframework $version already in vendor/"
  exit 0
fi
mkdir -p vendor
if [ ! -f "$zip" ]; then
  curl -fsSL -o "$zip" "https://github.com/ggml-org/whisper.cpp/releases/download/$version/whisper-$version-xcframework.zip"
fi
actual="$(shasum -a 256 "$zip" | cut -d' ' -f1)"
if [ "$actual" != "$sha" ]; then
  echo "checksum mismatch for $zip: $actual" >&2
  rm -f "$zip"
  exit 1
fi
rm -rf vendor/whisper-unzip "$dest"
mkdir -p vendor/whisper-unzip
unzip -q "$zip" -d vendor/whisper-unzip
mv vendor/whisper-unzip/build-apple/whisper.xcframework "$dest"
rm -rf vendor/whisper-unzip
echo "fetched whisper.xcframework $version"
