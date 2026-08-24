use std::time::Duration;

use crate::{Error, ValidationError};

pub(crate) const MAX_KEY_BYTES: usize = 1_024;
// R2's platform limit is 5 MiB below 5 GiB for a single request/part.
// https://developers.cloudflare.com/r2/platform/limits/
pub(crate) const MIN_MULTIPART_PART_SIZE: u64 = 5 * 1024 * 1024;
pub(crate) const MAX_UPLOAD_SIZE: u64 = 5 * 1024 * 1024 * 1024 - MIN_MULTIPART_PART_SIZE;
pub(crate) const MAX_MULTIPART_PART_SIZE: u64 = MAX_UPLOAD_SIZE;
const MAX_PRESIGN_SECONDS: u64 = 7 * 24 * 60 * 60;

pub(crate) const fn mebibytes(value: u64) -> u64 {
    match value.checked_mul(1024 * 1024) {
        Some(bytes) => bytes,
        None => u64::MAX,
    }
}

pub(crate) fn validate_key(key: &str) -> Result<(), Error> {
    if key.is_empty() || key.len() > MAX_KEY_BYTES {
        return Err(Error::InvalidInput {
            field: "key",
            reason: "must contain between 1 and 1,024 UTF-8 bytes",
        });
    }
    Ok(())
}

pub(crate) fn validate_prefix(prefix: &str) -> Result<(), Error> {
    if prefix.len() > MAX_KEY_BYTES {
        return Err(Error::InvalidInput {
            field: "prefix",
            reason: "must not exceed 1,024 UTF-8 bytes",
        });
    }
    Ok(())
}

pub(crate) fn validate_expiry(expires_in: Duration) -> Result<(), Error> {
    if expires_in < Duration::from_secs(1) || expires_in > Duration::from_secs(MAX_PRESIGN_SECONDS)
    {
        return Err(ValidationError::PresignExpiryOutOfRange {
            provided: expires_in,
            min: Duration::from_secs(1),
            max: Duration::from_secs(MAX_PRESIGN_SECONDS),
        }
        .into());
    }
    Ok(())
}

pub(crate) fn validate_part_size(part_size: u64) -> Result<(), Error> {
    if !(MIN_MULTIPART_PART_SIZE..=MAX_MULTIPART_PART_SIZE).contains(&part_size) {
        return Err(ValidationError::PartSizeOutOfRange {
            provided: part_size,
            min: MIN_MULTIPART_PART_SIZE,
            max: MAX_MULTIPART_PART_SIZE,
        }
        .into());
    }
    Ok(())
}
