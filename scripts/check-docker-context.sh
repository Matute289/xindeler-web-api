#!/bin/sh
set -eu

for pattern in target .git .env '.env.*' '*.db' '*.db-*' '*.log' .worktrees; do
    grep -Fx "$pattern" .dockerignore >/dev/null || {
        echo "missing .dockerignore rule: $pattern" >&2
        exit 1
    }
done

grep -Eq '^FROM rust:[0-9]+\.[0-9]+-bookworm AS builder$' Dockerfile || {
    echo "builder must use a versioned official Rust image" >&2
    exit 1
}
grep -F 'cargo build --locked --release' Dockerfile >/dev/null || {
    echo "release build must honor Cargo.lock" >&2
    exit 1
}
grep -Eq '^USER [1-9][0-9]*:[1-9][0-9]*$' Dockerfile || {
    echo "runtime image must use a numeric non-root user" >&2
    exit 1
}
grep -F 'read_only: true' docker-compose.yml >/dev/null
grep -F 'cap_drop:' docker-compose.yml >/dev/null
grep -F 'no-new-privileges:true' docker-compose.yml >/dev/null
