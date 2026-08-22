# Changelog

All notable changes to this project will be documented in this file. The format
is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this
project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Machine-readable numeric validation errors with supplied and accepted bounds.
- Sanitized R2/AWS failure categories without exposing raw SDK errors.
- Explicit read-only bucket-access preflight helpers.
- Optional secret-safe `tracing` events, disabled by default.
- `part_size_mib` conveniences for readable multipart configuration without
  repeated byte-unit arithmetic.

### Fixed

- Enforced R2's documented 63-character bucket-name maximum.

## [0.1.0] - 2026-08-23

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

[Unreleased]: https://github.com/zer0horizon/r2kit/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/zer0horizon/r2kit/releases/tag/v0.1.0
