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
- Presigned single-object GET and PUT requests with exact PUT lengths.
- Canonical completion manifests using exact R2 ETags.
- Complete, abort, snapshot, and resume multipart sessions.
- Optional per-part `Content-MD5` enforcement by R2.
- Versioned `serde` persistence DTOs and redacted protocol DTOs.
- Server-side `ListParts` reconciliation and verified completion.
- Managed file multipart uploads with bounded concurrency, retry, progress,
  automatic abort, and reuse of existing parts when resuming.
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

## Production multipart protocol

Enable the optional `serde` feature when session records, signed upload
requests, or uploader receipts cross a JSON boundary:

```toml
r2kit = { version = "0.1", features = ["serde"] }
```

The trusted server can persist a versioned session, require R2 to validate the
MD5 of each part, and reconcile untrusted uploader receipts before completion:

```rust,no_run
use std::time::Duration;
use r2kit::{CompletionManifest, MultipartSessionSnapshot, PartMd5, PartNumber};

# async fn example(
#     upload: r2kit::PresignedMultipart,
#     part_md5_base64: &str,
# ) -> Result<(), r2kit::Error> {
let record = upload.snapshot().into_persistence_record();
// Serialize `record` into a secret-capable store. It contains the upload ID.
let restored = MultipartSessionSnapshot::from_persistence_record(record)?;
# let _ = restored;

let number = PartNumber::try_from(1)?;
let md5 = PartMd5::try_from(part_md5_base64)?;
let request = upload
    .presign_part_with_md5(number, md5, Duration::from_secs(900))
    .await?
    .into_protocol_request()?;
// Serializing `request` deliberately exposes its bearer URL to the uploader.
# let _ = request;

let remote = upload.reconcile().await?;
if remote.is_complete() {
    let manifest: CompletionManifest = remote.into_completion_manifest()?;
    upload.complete_verified(manifest).await?;
}
# Ok(())
# }
```

For browser uploads, the bucket CORS policy must allow `Content-MD5` and expose
`ETag`. The uploader must replay every signed header exactly and return the
exact ETag response value. A multipart ETag is not a whole-object content hash.

## Managed file upload

```rust,no_run
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
let client = r2kit::R2Client::from_env()?;
let bucket = client.bucket("media")?;
let result = bucket
    .managed_multipart("videos/demo.mp4")?
    .part_size(16 * 1024 * 1024)
    .concurrency(4)
    .max_attempts(4)
    .on_progress(|progress| {
        eprintln!("{}/{} parts", progress.completed_parts(), progress.total_parts());
    })
    .upload_file("videos/demo.mp4")
    .await?;

eprintln!("completed {} parts", result.part_count());
# Ok(())
# }
```

By default, failures trigger a best-effort abort. Set `abort_on_error(false)`
when the caller wants `ManagedUploadError::snapshot()` for a later resume. The
source file must not change while an upload is running.

Managed uploads own their `UploadPart` retry policy: network failures, HTTP 408,
429, and 5xx responses are retried with bounded backoff. SDK retries are disabled
for that operation so `max_attempts` is the exact request-attempt limit.

For cooperative cancellation, attach a cloneable signal and keep awaiting the
upload future while another task requests cancellation:

```rust,no_run
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
let client = r2kit::R2Client::from_env()?;
let bucket = client.bucket("media")?;
let cancellation = r2kit::ManagedUploadCancellation::new();
let request = cancellation.clone();

let upload = bucket
    .managed_multipart("videos/demo.mp4")?
    .cancellation_token(cancellation)
    .upload_file("videos/demo.mp4");

// A signal handler or another task can call this at any time.
request.cancel();
let error = upload.await.unwrap_err();
assert!(matches!(error.error(), r2kit::Error::Cancelled));
# Ok(())
# }
```

With the default `abort_on_error(true)`, cancellation waits for a best-effort R2
abort. If cleanup fails, `ManagedUploadError::snapshot()` retains the session so
the caller can retry cleanup or resume. Dropping the upload future cannot perform
asynchronous cleanup; signal cancellation and continue awaiting it instead.

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

The live multipart contract tests are doubly gated by a Cargo feature and
ignored tests. They cover successful and rejected checksum uploads, recovery,
completion, and cleanup. Use bucket-scoped Object Read & Write credentials for
the dedicated `r2kit-live-tests` bucket:

```sh
R2KIT_LIVE_TESTS=1 \
R2KIT_LIVE_BUCKET=r2kit-live-tests \
cargo test --features live-tests \
  --test live_presigned_multipart -- --ignored --test-threads=1
```

The test only writes under `_r2kit-tests/<random-id>/`, aborts the active upload,
and deletes the completed object during cleanup. It never creates or deletes a
bucket.

For deeper protocol verification, run the property suite and the bounded fuzz
target:

```sh
cargo test --all-features --test property_multipart
cargo +nightly fuzz run protocol_boundaries -- -max_total_time=60
```

The opt-in stress test transfers a deterministic 64 MiB object with eight
parallel multipart workers, verifies progress ordering and downloads the object
for a byte-for-byte comparison. It uses the same dedicated bucket and prefix as
the other live tests, and deletes the object after a successful assertion:

```sh
R2KIT_LIVE_TESTS=1 \
R2KIT_DEEP_LIVE_TESTS=1 \
R2KIT_LIVE_BUCKET=r2kit-live-tests \
cargo test --features live-tests --test live_stress \
  -- --ignored --test-threads=1
```

## Non-goals for v0.1

Bucket administration, ACLs, tagging, versioning, object lock, folder sync, a
CLI, and a custom SigV4 implementation are deliberately out of scope.

The architectural boundary and acceptance rule for future APIs are documented
in [`docs/design.md`](docs/design.md).

## License

Licensed under either Apache License, Version 2.0 or the MIT license, at your
option.
