# r2kit

`r2kit` is a safety-first Rust transfer toolkit for Cloudflare R2. It builds on
the official AWS S3 SDK and focuses on R2-native configuration, secret-safe
debugging, and presigned multipart uploads for browser and mobile clients.

> Early development: the multipart compatibility spike has passed against live
> R2, but the API is not yet semver-stable.

## Current capabilities

- R2 endpoint generation and signing region `auto`.
- Long-lived and temporary credentials with an optional session token.
- Redacted `Debug` output for credentials, upload IDs, and presigned URLs.
- Validated multipart plans using R2's part-size and part-count constraints.
- Presigned `UploadPart` requests with required headers.
- Canonical completion manifests using exact R2 ETags.
- Complete, abort, snapshot, and resume multipart sessions.
- Raw `aws_sdk_s3::Client` escape hatch.
- Streaming and in-memory PUT, streaming GET, HEAD, paginated LIST, and
  idempotent DELETE.

## Core object operations

```rust,no_run
# async fn example() -> Result<(), r2kit::Error> {
let client = r2kit::R2Client::from_env()?;
let bucket = client.bucket("media")?;

let uploaded = bucket.put_bytes("hello.txt", b"hello R2".to_vec()).await?;
let metadata = bucket.head("hello.txt").await?;
let download = bucket.get("hello.txt").await?;
let body = download.into_body();

let page = bucket.list().prefix("uploads/").limit(100).send().await?;
let next = page.next_continuation_token();

bucket.delete("hello.txt").await?;
# let _ = (uploaded, metadata, body, next);
# Ok(())
# }
```

## Presigned multipart flow

The trusted server creates a session and signs individual parts. The uploader
receives bearer URLs but never receives R2 API credentials.

```rust,no_run
use std::time::Duration;
use r2kit::{CompletionManifest, PartNumber, R2Client, UploadedPart};

# async fn example() -> Result<(), r2kit::Error> {
let client = R2Client::from_env()?;
let bucket = client.bucket("r2kit")?;
let upload = bucket
    .presigned_multipart("videos/demo.mp4")?
    .file_size(11 * 1024 * 1024)
    .part_size(5 * 1024 * 1024)
    .create()
    .await?;

let part = upload
    .presign_part(PartNumber::try_from(1)?, Duration::from_secs(900))
    .await?;

// Deliberate exposure boundary for sending the bearer request to an uploader.
let (_method, _url, _required_headers) =
    part.into_request().into_exposed_parts();

// After the uploader returns the exact ETag from every uploaded part:
let uploaded_parts: Vec<UploadedPart> = Vec::new();
let manifest = CompletionManifest::try_from_parts(uploaded_parts)?;
// upload.complete(manifest).await?;
# let _ = manifest;
# Ok(())
# }
```

Presigned URLs and upload IDs are intentionally hidden from `Debug`. Exposing
them requires an explicitly named method such as `SecretUrl::expose()` or
`PresignedRequest::into_exposed_parts()`.

## Configuration

```text
R2_ACCOUNT_ID=...
R2_ACCESS_KEY_ID=...
R2_SECRET_ACCESS_KEY=...
R2_SESSION_TOKEN=... # optional temporary credential token
```

```rust,no_run
let client = r2kit::R2Client::from_env()?;
# Ok::<(), r2kit::Error>(())
```

## Validation

Run the offline quality suite:

```sh
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --all-features --no-deps
cargo test --all-targets --all-features
```

The live multipart contract test is doubly gated by a Cargo feature and an
ignored test. Use bucket-scoped Object Read & Write credentials for the
dedicated `r2kit-live-tests` bucket:

```sh
R2KIT_LIVE_TESTS=1 \
R2KIT_LIVE_BUCKET=r2kit-live-tests \
cargo test --features live-tests \
  --test live_presigned_multipart -- --ignored --test-threads=1
```

The test only writes under `_r2kit-tests/<random-id>/`, aborts the active upload,
and deletes the completed object during cleanup. It never creates or deletes a
bucket.

## Non-goals for v0.1

Bucket administration, ACLs, tagging, versioning, object lock, folder sync, a
CLI, and a custom SigV4 implementation are deliberately out of scope.

The architectural boundary and acceptance rule for future APIs are documented
in [`docs/design.md`](docs/design.md).

## License

Licensed under either Apache License, Version 2.0 or the MIT license, at your
option.
