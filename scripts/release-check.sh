#!/bin/sh
set -eu

if [ -n "$(git status --porcelain)" ]; then
    echo "release check requires a clean worktree" >&2
    exit 1
fi

crate_version=$(sed -n 's/^version = "\([^"]*\)"$/\1/p' Cargo.toml | head -n 1)

if [ -z "$crate_version" ]; then
    echo "could not read the package version from Cargo.toml" >&2
    exit 1
fi

if ! grep -Fq "## [$crate_version] - " CHANGELOG.md; then
    echo "CHANGELOG.md has no dated section for $crate_version" >&2
    exit 1
fi

if git rev-parse -q --verify "refs/tags/v$crate_version" >/dev/null; then
    echo "tag v$crate_version already exists" >&2
    exit 1
fi

./scripts/check.sh full
cargo publish --dry-run --locked
