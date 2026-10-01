#!/bin/sh
# Installs the latest Maya release on macOS or Linux.
#
#   curl -fsSL https://raw.githubusercontent.com/dosaki/maya/main/install.sh | sh
#
# Environment:
#   MAYA_INSTALL_DIR  where Maya goes: Maya.app on macOS (default /Applications,
#                     or ~/Applications when that is not writable), the
#                     maya-app AppImage on Linux (default ~/.local/bin)
#   MAYA_REPO         GitHub repo to install from (default dosaki/maya)
set -eu

REPO="${MAYA_REPO:-dosaki/maya}"

fail() { printf 'maya install: %s\n' "$1" >&2; exit 1; }

os="$(uname -s)"
case "$os" in
  Darwin) ;;
  Linux) ;;
  MINGW*|MSYS*|CYGWIN*) fail "on Windows, install from PowerShell: irm https://raw.githubusercontent.com/dosaki/maya/main/install.ps1 | iex" ;;
  *) fail "unsupported platform: $os" ;;
esac

# The architecture as the release names it, and the asset to take: the
# .app.tar.gz on macOS, the AppImage on Linux.
if [ "$os" = Linux ]; then
  case "$(uname -m)" in
    x86_64|amd64) arch=amd64 ;;
    aarch64|arm64) arch=arm64 ;;
    *) fail "unsupported architecture: $(uname -m)" ;;
  esac
  suffix="_$arch\.AppImage$"
else
  case "$(uname -m)" in
    arm64|aarch64) arch=aarch64 ;;
    x86_64) arch=x86_64 ;;
    *) fail "unsupported architecture: $(uname -m)" ;;
  esac
  suffix="_$arch\.app\.tar\.gz$"
fi

# The download URL of the asset whose name ends in $2 in the latest release.
asset_url() {
  # $1: releases/latest JSON, $2: the asset name's ending, a grep pattern
  printf '%s' "$1" | tr -d '\n' | grep -o '"browser_download_url": *"[^"]*' | sed 's/.*"//' | grep "$2" | head -n 1
}

if [ "${1:-}" = "--print-url" ]; then
  json="$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest")"
  asset_url "$json" "$suffix"
  exit 0
fi

json="$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest")" || fail "could not reach GitHub releases for $REPO"
url="$(asset_url "$json" "$suffix")"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

if [ "$os" = Linux ]; then
  [ -n "$url" ] || fail "no Linux $arch AppImage found in the latest release of $REPO"
  dest="${MAYA_INSTALL_DIR:-$HOME/.local/bin}"
  mkdir -p "$dest"
  printf 'Downloading %s\n' "$url"
  curl -fsSL "$url" -o "$tmp/maya-app"
  chmod +x "$tmp/maya-app"
  # A new file in place of the old, so a running Maya keeps its own copy.
  mv -f "$tmp/maya-app" "$dest/maya-app"

  # The icon from inside the AppImage, where the desktop entry finds it.
  icons="$HOME/.local/share/icons/hicolor/128x128/apps"
  if (cd "$tmp" && "$dest/maya-app" --appimage-extract 'usr/share/icons/hicolor/128x128/apps/*' >/dev/null 2>&1); then
    icon="$(find "$tmp/squashfs-root/usr/share/icons/hicolor/128x128/apps" -name '*.png' 2>/dev/null | head -n 1)"
    if [ -n "$icon" ]; then
      mkdir -p "$icons"
      cp "$icon" "$icons/maya.png"
    fi
  fi

  apps="$HOME/.local/share/applications"
  mkdir -p "$apps"
  cat > "$apps/maya.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=Maya
Exec=$dest/maya-app
Icon=maya
Categories=Development;
DESKTOP

  printf 'Installed Maya to %s/maya-app\n' "$dest"
  printf 'Requirements: tmux, libnotify-bin, speech-dispatcher (sudo apt install tmux libnotify-bin speech-dispatcher)\n'
  printf 'Start it from the app grid, or run: %s/maya-app\n' "$dest"
  exit 0
fi

[ -n "$url" ] || fail "no macOS $arch build found in the latest release of $REPO"

dest="${MAYA_INSTALL_DIR:-}"
if [ -z "$dest" ]; then
  if [ -w /Applications ]; then dest=/Applications; else dest="$HOME/Applications"; fi
fi
mkdir -p "$dest"

printf 'Downloading %s\n' "$url"
curl -fsSL "$url" -o "$tmp/maya.tar.gz"
tar -xzf "$tmp/maya.tar.gz" -C "$tmp"
[ -d "$tmp/Maya.app" ] || fail "the download did not contain Maya.app"

if [ -d "$dest/Maya.app" ]; then
  if pgrep -ix maya >/dev/null 2>&1; then
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
