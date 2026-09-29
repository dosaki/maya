#!/bin/sh
# Compiles and runs the sidecar's pure-Swift tests.
set -eu
cd "$(dirname "$0")/.."
out="$(mktemp -d)"
swiftc -O -o "$out/vadtest" ear/vad.swift ear/vadtest.swift
"$out/vadtest"
