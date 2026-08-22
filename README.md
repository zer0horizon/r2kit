# r2kit

`r2kit` is an ergonomic Rust foundation for Cloudflare R2.

The first release establishes a small, dependency-free configuration layer. It
normalizes the R2 S3 endpoint and validates the credentials needed by a future
S3-compatible client. Transfers, presigned URLs, and Cloudflare admin APIs will
be added behind focused feature flags rather than reimplementing the AWS SDK.

## Quick start

```rust
use r2kit::R2Config;

let config = R2Config::builder()
    .account_id("your-account-id")
    .access_key_id("your-access-key-id")
    .secret_access_key("your-secret-access-key")
    .build()?;

assert_eq!(config.region(), "auto");
assert_eq!(
    config.endpoint_url(),
    "https://your-account-id.r2.cloudflarestorage.com"
);
# Ok::<(), r2kit::ConfigError>(())
```

Environment variables are also supported:

```text
R2_ACCOUNT_ID=...
R2_ACCESS_KEY_ID=...
R2_SECRET_ACCESS_KEY=...
```

```rust
let config = r2kit::R2Config::from_env()?;
# Ok::<(), r2kit::ConfigError>(())
```

## Status

Early development. The public configuration API is intentionally small while
the S3 transport layer is designed.
