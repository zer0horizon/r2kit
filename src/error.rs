use std::fmt;

/// A configuration value required to create an R2 client was absent or invalid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    /// A required configuration field was not supplied.
    MissingField(&'static str),
    /// The Cloudflare account ID contained whitespace or was empty.
    InvalidAccountId,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingField(field) => write!(f, "missing required R2 configuration: {field}"),
            Self::InvalidAccountId => {
                write!(f, "R2 account ID must not be empty or contain whitespace")
            }
        }
    }
}

impl std::error::Error for ConfigError {}
