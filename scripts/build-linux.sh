#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"

if [ "$(uname -s)" != Linux ]; then
    echo "Run this script on Linux." >&2
    exit 1
fi

cargo build --locked --release --bin clify
echo "Linux binary: ${CARGO_TARGET_DIR:-$root/target}/release/clify"
