use crate::Error;

pub(crate) const MAX_KEY_BYTES: usize = 1_024;

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
