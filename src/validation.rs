use std::time::Duration;

use crate::Error;

pub(crate) const MAX_KEY_BYTES: usize = 1_024;
const MIN_MULTIPART_PART_SIZE: u64 = 5 * 1024 * 1024;
const MAX_MULTIPART_PART_SIZE: u64 = 5 * 1024 * 1024 * 1024;
const MAX_PRESIGN_SECONDS: u64 = 7 * 24 * 60 * 60;

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
        return Err(Error::InvalidInput {
            field: "expires_in",
            reason: "must be between 1 second and 7 days",
        });
    }
    Ok(())
}

pub(crate) fn validate_part_size(part_size: u64) -> Result<(), Error> {
    if !(MIN_MULTIPART_PART_SIZE..=MAX_MULTIPART_PART_SIZE).contains(&part_size) {
        return Err(Error::InvalidInput {
            field: "part_size",
            reason: "must be between 5 MiB and 5 GiB",
        });
    }
    Ok(())
}
