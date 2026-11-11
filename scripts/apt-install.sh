#!/bin/sh
# Installs Ubuntu packages on a CI runner, and tries again when the
# runner cannot reach the Ubuntu mirrors. Without --error-on=any, an
# update that cannot fetch the indexes only warns, and the install
# then fails on stale package names.
# Usage: sh scripts/apt-install.sh <package>...
set -u

tries=5
try=1
while :; do
  if sudo apt-get update --error-on=any -o Acquire::Retries=3 &&
    sudo apt-get install -y -o Acquire::Retries=3 "$@"; then
    exit 0
  fi
  if [ "$try" -ge "$tries" ]; then
    echo "apt failed $tries times; giving up" >&2
    exit 1
  fi
  wait=$((try * 30))
  echo "apt failed (try $try of $tries); trying again in $wait s" >&2
  sleep "$wait"
  try=$((try + 1))
done
