#!/bin/sh
# Checks scripts/release-notes.sh against fake notes: which first lines pass
# the version check and which fail it, and the release body it prints.
# Usage: sh scripts/test-release-notes.sh
set -eu
cd "$(dirname "$0")/.."
dir="$(mktemp -d)"
trap 'rm -rf "$dir"' EXIT

passes() {
  sh scripts/release-notes.sh check 0.42.0 "$1" 2> "$dir/err" || { echo "FAIL: $2 did not pass the check" >&2; cat "$dir/err" >&2; exit 1; }
}
# fails <file> <what> <text the error must say>...
fails() {
  f="$1"
  what="$2"
  shift 2
  if sh scripts/release-notes.sh check 0.42.0 "$f" 2> "$dir/err"; then
    echo "FAIL: $what passed the check" >&2
    exit 1
  fi
  for want in "$@"; do
    grep -q -- "$want" "$dir/err" || { echo "FAIL: the error for $what does not say '$want'" >&2; cat "$dir/err" >&2; exit 1; }
  done
}

printf '# 0.42.0\n\n## New\n\n- A thing.\n' > "$dir/ok.md"
passes "$dir/ok.md" "a matching first line"
printf '# 0.42.0\r\n\r\n- A thing.\r\n' > "$dir/crlf.md"
passes "$dir/crlf.md" "Windows line endings"
printf '# 0.42.0  \n\n- A thing.\n' > "$dir/spaces.md"
passes "$dir/spaces.md" "trailing spaces"

printf '# 0.41.0\n\n- A thing.\n' > "$dir/old.md"
fails "$dir/old.md" "another version" "first line is .# 0.41.0., not .# 0.42.0."
printf '# v0.42.0\n\n- A thing.\n' > "$dir/v.md"
fails "$dir/v.md" "a v prefix" v0.42.0
printf '## New\n\n- A thing.\n' > "$dir/nohead.md"
fails "$dir/nohead.md" "no version heading" "first line is .## New., not .# 0.42.0."
fails "$dir/none.md" "a missing file" missing

out="$(sh scripts/release-notes.sh body abc123 "$dir/ok.md")"
[ "$(printf '%s\n' "$out" | head -n 1)" = "## Release Notes" ] || { echo "FAIL: the body does not start with ## Release Notes" >&2; printf '%s\n' "$out" >&2; exit 1; }
line() { printf '%s\n' "$out" | grep -n -- "$1" | head -n 1 | cut -d: -f1; }
new="$(line '^## New$')"
thing="$(line '^- A thing\.$')"
built="$(line '^Built from abc123 on main\. macOS builds are not notarised')"
[ -n "$new" ] && [ -n "$thing" ] && [ -n "$built" ] && [ "$new" -lt "$thing" ] && [ "$thing" -lt "$built" ] || { echo "FAIL: the body is not notes then install text" >&2; printf '%s\n' "$out" >&2; exit 1; }
if printf '%s\n' "$out" | grep -q '^# 0.42.0'; then
  echo "FAIL: the body keeps the version line" >&2
  exit 1
fi
# Without a final newline the install text must still be its own paragraph,
# not run on into the last bullet.
printf '# 0.42.0\n\n- A thing.' > "$dir/nonl.md"
out="$(sh scripts/release-notes.sh body abc123 "$dir/nonl.md")"
printf '%s\n' "$out" | grep -A 2 -- '^- A thing\.$' | sed -n 2p | grep -q '^$' || { echo "FAIL: no blank line between the notes and the install text" >&2; printf '%s\n' "$out" >&2; exit 1; }
# The file keeps every version of the major, newest first: the release gets
# only the top one.
printf '# 0.42.0\n\n## New\n\n- The new thing.\n\n# 0.41.0\n\n## Fixed\n\n- An old thing.\n' > "$dir/many.md"
passes "$dir/many.md" "several versions"
out="$(sh scripts/release-notes.sh body abc123 "$dir/many.md")"
printf '%s\n' "$out" | grep -q -- '^- The new thing\.$' || { echo "FAIL: the body lacks the top version's notes" >&2; printf '%s\n' "$out" >&2; exit 1; }
if printf '%s\n' "$out" | grep -q -- 'An old thing\|^# 0.41.0'; then
  echo "FAIL: the body carries an older version's notes" >&2
  printf '%s\n' "$out" >&2
  exit 1
fi
printf '%s\n' "$out" | grep -B 1 '^Built from' | head -n 1 | grep -q '^$' || { echo "FAIL: no blank line before the install text" >&2; printf '%s\n' "$out" >&2; exit 1; }
echo "release-notes: ok"
