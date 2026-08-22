use std::fmt;

/// A configuration value required to create an R2 client was absent or invalid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    /// A required configuration field was not supplied.
    MissingField(&'static str),
    /// The Cloudflare account ID was not a 32-character hexadecimal identifier.
    InvalidAccountId,
    /// A supplied configuration field was empty.
    EmptyField(&'static str),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingField(field) => write!(f, "missing required R2 configuration: {field}"),
            Self::InvalidAccountId => {
                write!(
                    f,
                    "R2 account ID must be a 32-character hexadecimal identifier"
                )
            }
            Self::EmptyField(field) => write!(f, "R2 configuration must not be empty: {field}"),
        }
    }
}

impl std::error::Error for ConfigError {}

/// An error returned by an `r2kit` operation.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// Client configuration was invalid.
    Config(ConfigError),
    /// A caller-provided value was invalid.
    InvalidInput {
        /// Name of the invalid field.
        field: &'static str,
        /// Stable, non-sensitive explanation.
        reason: &'static str,
    },
    /// An R2 S3 operation failed. The generated SDK error is intentionally not
    /// formatted because it can contain signed request details.
    Service {
        /// Name of the operation that failed.
        operation: &'static str,
    },
    /// Presigning failed before a request was sent.
    Presign,
    /// A signed header could not be represented as text.
    InvalidSignedHeader,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(error) => error.fmt(f),
            Self::InvalidInput { field, reason } => write!(f, "invalid {field}: {reason}"),
            Self::Service { operation } => write!(f, "R2 operation failed: {operation}"),
            Self::Presign => write!(f, "failed to create a presigned R2 request"),
            Self::InvalidSignedHeader => write!(f, "presigned request contains a non-text header"),
        }
    }
}

impl std::error::Error for Error {}

impl From<ConfigError> for Error {
    fn from(value: ConfigError) -> Self {
        Self::Config(value)
    }
}
