#!/bin/sh
# Fails when the Android project's launcher icon is not Maya's: every
# file `tauri icon` wrote to mobile/icons/android must be in the
# generated project's res/ as it is. Usage: sh scripts/check-android-icons.sh
set -eu
cd "$(dirname "$0")/.."
src=mobile/icons/android
res=mobile/gen/android/app/src/main/res
bad=0
for f in $(cd "$src" && find . -type f | sort); do
  if ! cmp -s "$src/$f" "$res/$f"; then
    echo "launcher icon out of step: $res/${f#./} does not match $src/${f#./}" >&2
    bad=1
  fi
done
if [ "$bad" -ne 0 ]; then
  echo "copy it in: cp -R $src/. $res/" >&2
fi
exit "$bad"
