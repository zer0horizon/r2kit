# Contributing to r2kit

`r2kit` stays intentionally focused on safe, ergonomic Cloudflare R2 object
transfer workflows. New APIs should remove R2-specific ceremony, enforce an R2
invariant, or provide recovery and observability that the raw S3 SDK does not.

## Local checks

Run these checks before opening a pull request:

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo test --doc --all-features
RUSTDOCFLAGS="-D warnings" cargo doc --all-features --no-deps
cargo package
cargo deny check advisories bans licenses sources
```

Live tests require bucket-scoped Object Read & Write credentials and are never
run automatically for pull requests. See the README for the explicit opt-in
command and dedicated bucket requirements.

Run the live contract suite against the dedicated test bucket:

```sh
R2KIT_LIVE_TESTS=1 \
R2KIT_LIVE_BUCKET=r2kit-live-tests \
cargo test --features live-tests -- --ignored --test-threads=1
```

The deeper stress test transfers and verifies a deterministic 64 MiB object:

```sh
R2KIT_LIVE_TESTS=1 \
R2KIT_DEEP_LIVE_TESTS=1 \
R2KIT_LIVE_BUCKET=r2kit-live-tests \
cargo test --features live-tests --test live_stress \
  -- --ignored --test-threads=1
```

For protocol fuzzing, install `cargo-fuzz` and run:

```sh
cargo +nightly fuzz run protocol_boundaries -- -max_total_time=60
```

## API expectations

- Presigned URLs, credentials, and upload IDs must be redacted from `Debug` and
  error messages.
- Validate deterministic failures before making a network request.
- Preserve an escape hatch to `aws-sdk-s3`; do not wrap unrelated S3 features.
- Add offline contract tests for public behavior and a live R2 test for any
  compatibility claim that cannot be proven locally.
- Public API changes require documentation and must respect the declared MSRV.
- Pull requests are checked for accidental SemVer-breaking API changes against
  their target branch.

By contributing, you agree that your work is licensed under either Apache-2.0
or MIT, at the user's option.
