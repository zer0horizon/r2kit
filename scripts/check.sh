#!/bin/sh
set -eu

mode="${1:-full}"

run_fast_checks() {
    cargo fmt-check
    cargo lint-default
    cargo lint
}

run_test_checks() {
    cargo test-all
    cargo test-docs
    RUSTDOCFLAGS="-D warnings" cargo doc-check
    cargo package-check
}

case "$mode" in
    fast)
        run_fast_checks
        ;;
    test)
        run_test_checks
        ;;
    full)
        run_fast_checks
        run_test_checks
        if command -v cargo-deny >/dev/null 2>&1; then
            cargo deny check advisories bans licenses sources
        else
            echo "cargo-deny not installed; skipping the optional local supply-chain check" >&2
        fi
        ;;
    *)
        echo "usage: $0 [fast|test|full]" >&2
        exit 2
        ;;
esac
