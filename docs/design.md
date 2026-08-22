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
- Completion requires every planned part exactly once and preserves exact R2
  ETags.
- Live tests use a dedicated bucket, randomized object prefixes, and best-effort
  cleanup. Pull requests do not receive live credentials.

## Dependency boundary

R2's S3-compatible API and SigV4 behavior are delegated to `aws-sdk-s3`.
`r2kit` does not implement custom request signing. The AWS client is exposed so
users can access supported operations outside this crate's deliberate scope.
