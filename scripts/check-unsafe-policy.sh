#!/bin/sh
set -eu

for root in common/src/lib.rs server/src/main.rs; do
    first_line=$(sed -n '1p' "$root")
    if [ "$first_line" != '#![forbid(unsafe_code)]' ]; then
        echo "$root must begin with #![forbid(unsafe_code)]" >&2
        exit 1
    fi
done
