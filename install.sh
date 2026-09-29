#!/bin/sh
# Installs the latest Maya release. macOS only for now.
#
#   curl -fsSL https://raw.githubusercontent.com/dosaki/maya/main/install.sh | sh
#
# Environment:
#   MAYA_INSTALL_DIR  where Maya.app goes (default /Applications, or ~/Applications when that is not writable)
#   MAYA_REPO         GitHub repo to install from (default dosaki/maya)
set -eu

REPO="${MAYA_REPO:-dosaki/maya}"

fail() { printf 'maya install: %s\n' "$1" >&2; exit 1; }

os="$(uname -s)"
case "$os" in
  Darwin) ;;
  Linux) fail "Linux is not supported yet; only macOS builds are published today." ;;
  *) fail "unsupported platform: $os" ;;
esac

case "$(uname -m)" in
  arm64|aarch64) arch=aarch64 ;;
  x86_64) arch=x86_64 ;;
  *) fail "unsupported architecture: $(uname -m)" ;;
esac

# The download URL of the .app.tar.gz for this architecture in the latest release.
asset_url() {
  # $1: releases/latest JSON, $2: architecture
  printf '%s' "$1" | tr -d '\n' | grep -o '"browser_download_url": *"[^"]*' | sed 's/.*"//' | grep "_$2\.app\.tar\.gz$" | head -n 1
}

if [ "${1:-}" = "--print-url" ]; then
  json="$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest")"
  asset_url "$json" "$arch"
  exit 0
fi

json="$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest")" || fail "could not reach GitHub releases for $REPO"
url="$(asset_url "$json" "$arch")"
[ -n "$url" ] || fail "no macOS $arch build found in the latest release of $REPO"

dest="${MAYA_INSTALL_DIR:-}"
if [ -z "$dest" ]; then
  if [ -w /Applications ]; then dest=/Applications; else dest="$HOME/Applications"; fi
fi
mkdir -p "$dest"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
printf 'Downloading %s\n' "$url"
curl -fsSL "$url" -o "$tmp/maya.tar.gz"
tar -xzf "$tmp/maya.tar.gz" -C "$tmp"
[ -d "$tmp/Maya.app" ] || fail "the download did not contain Maya.app"

if [ -d "$dest/Maya.app" ]; then
  if pgrep -x Maya >/dev/null 2>&1; then
    printf 'Maya is running; quitting it before replacing the app.\n'
    osascript -e 'tell application "Maya" to quit' >/dev/null 2>&1 || true
    sleep 1
  fi
  rm -rf "$dest/Maya.app"
fi
mv "$tmp/Maya.app" "$dest/Maya.app"
# The build is not signed or notarised: clear the quarantine flag so it opens.
xattr -dr com.apple.quarantine "$dest/Maya.app" 2>/dev/null || true

printf 'Installed Maya to %s/Maya.app\n' "$dest"
printf 'Start it with: open -a Maya\n'
