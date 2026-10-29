#!/usr/bin/env bash
# Build the librepixel-ink xovi extension (extension/librepixel-ink.c) for the reMarkable 2.
# Needs zig (Windows: winget install zig.zig). Output: extension/librepixel-ink.so
set -euo pipefail
cd "$(dirname "$0")/.."
zig cc -target arm-linux-gnueabihf.2.31 -mcpu=cortex_a7 -shared -fPIC -O2 -s \
    -o extension/librepixel-ink.so extension/librepixel-ink.c extension/xovi.c -ldl -lpthread
ls -l extension/librepixel-ink.so
