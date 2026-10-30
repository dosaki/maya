#!/bin/sh
# RELEASE_NOTES.md holds the notes for the version in the version files,
# its first line `# <version>`. `check` fails, naming both, when the file is
# missing or is for another version; `body` prints the GitHub release's
# notes: "## Release Notes", the file without its first line, then the
# install text. A file argument stands in for RELEASE_NOTES.md.
# Usage: sh scripts/release-notes.sh check <version> [file]
#        sh scripts/release-notes.sh body <sha> [file] > notes.md
set -eu
cd "$(dirname "$0")/.."
cmd="${1:-}"
arg="${2:-}"
file="${3:-RELEASE_NOTES.md}"
case "$cmd" in
  check)
    [ -f "$file" ] || { echo "$file is missing: write the notes for $arg" >&2; exit 1; }
    # A Windows editor's \r, and trailing spaces, are not part of the version.
    first="$(head -n 1 "$file" | tr -d '\r' | sed 's/[[:space:]]*$//')"
    [ "$first" = "# $arg" ] || { echo "$file is for '$first', not $arg: rewrite it for $arg" >&2; exit 1; }
    ;;
  body)
    printf '## Release Notes\n\n'
    # Without the version line and the blank lines under it; `awk 1` ends
    # the last line, which an editor may have left open, so the install
    # text below stays a paragraph of its own.
    tail -n +2 "$file" | tr -d '\r' | sed '/./,$!d' | awk 1
    printf '\nBuilt from %s on main. macOS builds are not notarised: install with the one-line installer in the README, which clears the quarantine flag, or right-click the app and choose Open the first time. Windows installers are not signed: SmartScreen asks once, choose More info, then Run anyway. The Android APK is release-signed; allow installs from your browser to install it. Every APK after 0.21.0 is signed with the same key and installs over the one before; 0.21.0 and earlier each had a new key, so moving from one of those means uninstalling first, once, which forgets the pairings. From 0.18.0 the desktop app updates itself: a bar on the board offers each new release. An .msi or .deb install updates by hand.\n' "$arg"
    ;;
  *)
    echo "usage: sh scripts/release-notes.sh check <version> [file] | body <sha> [file]" >&2
    exit 1
    ;;
esac
