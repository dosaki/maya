#!/bin/sh
# Compiles and runs the sidecar's pure-Swift tests.
set -eu
cd "$(dirname "$0")/.."
# Windows and Linux have the Rust ear, whose tests port the Swift ones.
case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*) exec cargo test -p maya-ear ;;
  Linux) exec cargo test -p maya-ear ;;
esac
out="$(mktemp -d)"
swiftc -O -o "$out/vadtest" ear/vad.swift ear/vadtest.swift
"$out/vadtest"
