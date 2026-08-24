# Changelog

All notable changes to this project will be documented in this file. The format
is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this
project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2026-08-25

### Added

- R2-native client and bucket configuration.
- Typed default, EU, US, and FedRAMP jurisdiction endpoints.
- Validated connection, read, operation, and per-attempt timeouts plus SDK
  request-attempt configuration.
- Core object PUT, GET, HEAD, LIST, and DELETE operations.
- Presigned single-object and multipart upload flows.
- Resumable managed multipart file uploads with bounded concurrency, exact
  retries, progress reporting, cancellation, and cleanup.
- Secret-safe protocol and persistence types with optional Serde support.
- Machine-readable numeric validation errors with supplied and accepted bounds.
- Sanitized R2/AWS failure categories without exposing raw SDK errors.
- Explicit read-only bucket-access preflight helpers.
- Optional secret-safe `tracing` events, disabled by default.
- `part_size_mib` conveniences for readable multipart configuration without
  repeated byte-unit arithmetic.
- Typed `Mime` and `CacheControl` object metadata for regular, presigned, and
  managed uploads, including multipart creation and browser-safe signed headers.
- Upload support for content disposition, encoding, language, expiration, and
  user-defined metadata across regular, presigned, and managed workflows.
- Local structural BCP 47 validation for single and comma-separated
  `Content-Language` values.
- Opt-in R2 live coverage for extended metadata round trips, automatic page
  traversal, ordinary batch deletion, and a 1,001-object multi-request delete.
- Auto-paginating object page streams that preserve delimiter and common-prefix
  semantics.
- Multi-object deletion with automatic 1,000-key batching, per-key failures,
  and partial results when a later request fails.
- Managed-upload progress fraction and percentage helpers.
- A configurable 256 MiB default budget for in-flight managed-upload part
  buffers, validated before file or network I/O.
- Capped exponential full-jitter retries with bounded server retry-delay support.
- Source-file mutation detection using size, modification time, and file
  identity where supported.

### Fixed

- Enforced R2's documented 63-character bucket-name maximum.
- Enforced R2's effective per-request upload maximum of 5 MiB below 5 GiB.

[Unreleased]: https://github.com/zer0horizon/r2kit/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/zer0horizon/r2kit/releases/tag/v0.1.0
