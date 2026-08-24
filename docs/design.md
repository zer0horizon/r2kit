# Design boundaries

## Decision: build a focused R2 transfer toolkit

The M0 spike successfully created, signed, uploaded, completed, downloaded,
verified, aborted, and cleaned up multipart uploads against Cloudflare R2. This
is enough evidence to continue the project.

`r2kit` will not mirror the complete S3 API. The underlying `aws-sdk-s3` client
remains available as an explicit escape hatch. A higher-level API belongs here
only when it provides at least one of:

- R2-specific configuration or validation;
- a secret-safe presigned request contract;
- multipart planning, recovery, integrity, or cleanup;
- retry, concurrency, and observability semantics that are difficult to compose
  correctly at each call site.

## Safety boundaries

- Credentials, presigned URLs, and upload IDs are sensitive by default.
- Deterministic invalid input is rejected before network I/O.
- Numeric constraint failures are machine-readable and retain supplied and
  accepted bounds; free-form remote SDK errors never cross the public API.
- Completion requires every planned part exactly once and preserves exact R2
  ETags.
- Live tests use a dedicated bucket, randomized object prefixes, and best-effort
  cleanup. Pull requests do not receive live credentials.

## Client configuration invariants

- The signing region is always `auto` when `R2Client` is built from `R2Config`.
- The endpoint is derived from the validated account ID and the typed
  `R2Jurisdiction`; arbitrary endpoint injection is reserved for `from_sdk`.
- Zero timeouts and retry limits are rejected before an SDK client is created.
- An operation-attempt timeout cannot exceed the total operation timeout.
- Unset transport options preserve AWS SDK defaults for backward compatibility.
- SDK retry limits apply to ordinary operations. Managed multipart part uploads
  disable SDK retries and enforce their own exact attempt count.
- `from_sdk` is an explicit unchecked boundary: the caller owns endpoint,
  region, credentials, transport, and retry correctness.
- Bucket selection is offline. `validate_bucket` and `validate_access` are
  explicit, read-only preflight calls for applications that want fail-fast
  bucket existence and list-permission checks.

## Error and observability boundary

R2 service error codes are classified first, with HTTP status and SDK transport
category as bounded fallbacks. Public remote errors contain only the operation,
stable category, and optional status. The raw SDK error is deliberately dropped
because it can retain signed request details.

The optional `tracing` feature emits events through the application's existing
subscriber. It never installs global state. Its field allowlist excludes bucket
names, keys, local paths, account IDs, credentials, upload IDs, presigned URLs,
and signed headers.

## Production multipart protocol

The trusted signer owns session creation, persistence, reconciliation,
completion, and abort. An untrusted browser or mobile client receives only one
short-lived part request at a time and reports the exact ETag returned by R2.

- `MultipartSessionRecord` is versioned and must be validated back into a
  `MultipartSessionSnapshot` after deserialization. Its upload ID is a secret.
- `MultipartUploadPartRequest` is a bearer credential. Its `Debug` output is
  redacted; serialization deliberately exposes it for transport.
- `MultipartPartReceipt` contains primitive wire values and must be converted
  into a validated `UploadedPart` before completion.
- `PartMd5` signs `Content-MD5` into `UploadPart`. R2 rejects a body whose MD5
  differs before the part is committed.
- Typed object metadata is applied once at `CreateMultipartUpload`. It is not
  repeated on parts and cannot be changed when resuming an existing session.
- Managed local-file uploads validate their worst-case in-flight part buffers
  against an explicit memory budget before file or network I/O.
- Managed part retries use capped exponential full jitter and honor bounded
  numeric server retry delays.
- Local-file uploads compare source size, modification time, and file identity
  where supported before completion; callers still own source immutability.
- `reconcile` treats R2 `ListParts` as authoritative and validates the number,
  uniqueness, and planned byte length of every remote part.
- `complete_verified` compares exact remote ETags with the proposed manifest
  immediately before completion. It reduces stale-client errors but cannot
  remove the distributed-systems ambiguity of a lost completion response.

After an ambiguous completion response, applications should reconcile their
own durable terminal state and use `HEAD` plus application-level object identity
or checksum metadata. Finding an object at the same key is not by itself proof
that a particular upload won a race.

## Dependency boundary

R2's S3-compatible API and SigV4 behavior are delegated to `aws-sdk-s3`.
`r2kit` does not implement custom request signing. The AWS client is exposed so
users can access supported operations outside this crate's deliberate scope.
