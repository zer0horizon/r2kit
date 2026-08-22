# r2kit

[![CI](https://github.com/zer0horizon/r2kit/actions/workflows/ci.yml/badge.svg)](https://github.com/zer0horizon/r2kit/actions/workflows/ci.yml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

Ergonomic, safety-first Cloudflare R2 transfers for Rust.

`r2kit` handles the R2-specific details around the official AWS S3 SDK: the
account endpoint, `auto` signing region, secret-safe presigned requests,
resumable multipart sessions, and managed file uploads with bounded concurrency
and exact retries.

> **Status:** pre-release `0.1.0`. The API is still evolving and the crate has
> not been published to crates.io yet. Core object and multipart workflows have
> been verified against live Cloudflare R2.

## Why r2kit?

- **R2-native setup:** build a correctly configured client from three required
  environment variables instead of wiring the S3 endpoint and signing behavior
  yourself. Temporary credentials can add an optional session token.
- **Safe secret boundaries:** credentials, upload IDs, and presigned URLs are
  redacted from `Debug`; exposing bearer values requires explicitly named APIs.
- **Transfer workflows included:** use simple object operations, managed local
  file uploads, or a server-controlled presigned multipart protocol.
- **Recovery by design:** snapshot, resume, reconcile, cancel, and clean up
  multipart uploads without inventing a persistence format.
- **No lock-in:** access the underlying `aws_sdk_s3::Client` whenever an
  operation is intentionally outside r2kit's scope.

## Quick start

Until the first crates.io release, install directly from GitHub:

```sh
cargo add r2kit --git https://github.com/zer0horizon/r2kit
cargo add tokio --features macros,rt-multi-thread
```

Create a bucket-scoped R2 token and set its S3 credentials:

```sh
export R2_ACCOUNT_ID="your-32-character-account-id"
export R2_ACCESS_KEY_ID="your-access-key-id"
export R2_SECRET_ACCESS_KEY="your-secret-access-key"
# export R2_SESSION_TOKEN="..." # only for temporary credentials
```

Upload, inspect, download, list, and delete an object:

```rust,no_run
use r2kit::R2Client;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let bucket = R2Client::from_env()?.bucket("media")?;

    let uploaded = bucket
        .put_bytes("hello.txt", b"hello R2".to_vec())
        .await?;
    let metadata = bucket.head("hello.txt").await?;
    assert_eq!(metadata.etag(), uploaded.etag());

    let downloaded = bucket.get("hello.txt").await?;
    let bytes = downloaded.into_body().collect().await?.into_bytes();
    assert_eq!(bytes.as_ref(), b"hello R2");

    let page = bucket.list().prefix("hello").limit(100).send().await?;
    assert_eq!(page.objects().len(), 1);

    bucket.delete("hello.txt").await?;
    Ok(())
}
```

## Choose the right API

| Use case | API |
|---|---|
| Upload bytes already in memory | `Bucket::put_bytes` |
| Upload a known-length async body | `Bucket::put_stream` |
| Download without buffering the whole object | `Bucket::get` |
| Upload a local file with concurrency and retries | `Bucket::managed_multipart` |
| Let a browser or mobile client upload directly | `Bucket::presigned_multipart` |
| Resume a persisted upload session | `Bucket::resume_managed_multipart` or `resume_presigned_multipart` |
| Use an S3 operation not wrapped by r2kit | `R2Client::as_sdk` |

## Managed file uploads

Managed uploads split a local file into R2-compatible parts, upload them in
parallel, retry transient failures, emit monotonic progress, and complete or
abort the remote session.

```rust,no_run
use r2kit::R2Client;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let bucket = R2Client::from_env()?.bucket("media")?;
    let result = bucket
        .managed_multipart("videos/demo.mp4")?
        .part_size(16 * 1024 * 1024)
        .concurrency(4)
        .max_attempts(4)
        .on_progress(|progress| {
            eprintln!(
                "{}/{} bytes",
                progress.transferred_bytes(),
                progress.total_bytes()
            );
        })
        .upload_file("videos/demo.mp4")
        .await?;

    eprintln!("completed {} parts", result.part_count());
    Ok(())
}
```

The uploader owns the `UploadPart` retry policy. Network failures, HTTP 408,
429, and 5xx responses are retried with bounded backoff. AWS SDK retries are
disabled for that operation, so `max_attempts` is the exact request-attempt
limit.

Failures trigger a best-effort abort by default. Use `abort_on_error(false)` to
retain `ManagedUploadError::snapshot()` for a later resume. The source file must
not change while an upload is running.

### Cancellation

Cancellation is cooperative: signal it from another task and continue awaiting
the upload so r2kit can abort the remote multipart session.

```rust,no_run
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let bucket = r2kit::R2Client::from_env()?.bucket("media")?;
    let cancellation = r2kit::ManagedUploadCancellation::new();
    let signal = cancellation.clone();

    let upload = bucket
        .managed_multipart("videos/demo.mp4")?
        .cancellation_token(cancellation)
        .upload_file("videos/demo.mp4");

    signal.cancel();
    let error = upload.await.unwrap_err();
    assert!(matches!(error.error(), r2kit::Error::Cancelled));
    Ok(())
}
```

Dropping the future cannot perform asynchronous cleanup. Signal cancellation
and keep awaiting it instead.

## Direct browser and mobile uploads

The trusted server creates a multipart session and signs each part. The
untrusted uploader receives short-lived bearer URLs but never receives the R2
access key or secret.

```text
trusted server                browser/mobile                    Cloudflare R2
      | create session               |                                |
      | sign part requests ---------->                                |
      |                              | PUT parts with signed headers ->|
      |<--------- exact ETags -------|<-------------------------------|
      | reconcile + complete ---------------------------------------->|
```

```rust,no_run
use std::time::Duration;

use r2kit::{CompletionManifest, PartMd5, PartNumber, R2Client};

async fn sign_and_complete(part_md5_base64: &str) -> Result<(), r2kit::Error> {
    let bucket = R2Client::from_env()?.bucket("media")?;
    let upload = bucket
        .presigned_multipart("videos/demo.mp4")?
        .file_size(11 * 1024 * 1024)
        .part_size(5 * 1024 * 1024)
        .create()
        .await?;

    let part = upload
        .presign_part_with_md5(
            PartNumber::try_from(1)?,
            PartMd5::try_from(part_md5_base64)?,
            Duration::from_secs(15 * 60),
        )
        .await?;

    // Deliberate exposure boundary for sending the bearer request to the uploader.
    let _request = part.into_protocol_request()?;

    // After collecting and validating every uploader receipt:
    let remote = upload.reconcile().await?;
    if remote.is_complete() {
        let manifest: CompletionManifest = remote.into_completion_manifest()?;
        upload.complete_verified(manifest).await?;
    }
    Ok(())
}
```

For browser uploads, configure bucket CORS to allow `Content-MD5` and expose
`ETag`. The uploader must replay every signed header exactly. Multipart ETags
are opaque completion identifiers, not whole-object content hashes.

## Persistence and feature flags

The default build has no optional features enabled.

| Feature | Purpose |
|---|---|
| `serde` | Serialize versioned multipart session records, signed request DTOs, and uploader receipts |
| `live-tests` | Compile the credential-gated Cloudflare R2 integration tests; not intended for applications |

Enable Serde when a session or protocol DTO crosses a storage or JSON boundary:

```toml
[dependencies]
r2kit = { git = "https://github.com/zer0horizon/r2kit", features = ["serde"] }
```

`MultipartSessionSnapshot::into_persistence_record()` deliberately exposes a
secret-bearing persistence value. Store it as securely as an API credential.

## Security model

- Credentials, upload IDs, and presigned URLs are redacted from `Debug` and
  error messages.
- Presigned URLs remain bearer credentials. Authorize before issuing them, use
  short expirations, and never log them.
- Deterministic input failures are validated before network requests whenever
  possible.
- Managed uploads use bounded concurrency and an exact retry limit.
- Applications still own authorization, rate limiting, CORS policy, and the
  lifecycle policy for abandoned uploads.

Report vulnerabilities through GitHub's private security advisory flow. See
[SECURITY.md](SECURITY.md) for scope and reporting guidance.

## Examples and API documentation

- [Object round trip](examples/object_round_trip.rs)
- [Managed file upload](examples/managed_upload.rs)
- [Architecture and protocol invariants](docs/design.md)
- [API documentation](https://docs.rs/r2kit) — available after the first
  crates.io release

The runnable examples require `R2_BUCKET` and `R2_KEY`. The managed upload
example additionally accepts the local file path as its first argument.

## Compatibility and scope

- Minimum supported Rust version: **1.94.1**.
- Runtime: Tokio.
- Backend: Cloudflare R2 through `aws-sdk-s3`.
- License: MIT or Apache-2.0, at your option.

For `0.1`, bucket administration, ACLs, tagging, versioning, object lock,
folder sync, a CLI, and a custom SigV4 implementation are deliberately out of
scope.

## Development

The repository pins its Rust toolchain and keeps Git hooks in version control.
After cloning, enable the hooks once:

```sh
./scripts/install-git-hooks.sh
```

Run the offline quality suite:

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo test --doc --all-features
RUSTDOCFLAGS="-D warnings" cargo doc --all-features --no-deps
cargo package
```

Property, fuzz, live contract, and 64 MiB stress-test commands are documented in
[CONTRIBUTING.md](CONTRIBUTING.md). Live tests only use the dedicated bucket and
prefix supplied by the test operator, and clean up completed objects and active
multipart sessions.

Contributions are welcome when they simplify an R2 transfer workflow, enforce
an R2 invariant, or improve recovery and observability without hiding the
underlying SDK. Please read [CONTRIBUTING.md](CONTRIBUTING.md) before opening a
pull request.

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT License ([LICENSE-MIT](LICENSE-MIT))

at your option.
