use std::env;

use crate::ConfigError;

const R2_REGION: &str = "auto";

/// Configuration needed to connect an S3-compatible client to Cloudflare R2.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct R2Config {
    account_id: String,
    access_key_id: String,
    secret_access_key: String,
}

impl R2Config {
    /// Starts a builder for an R2 configuration.
    #[must_use]
    pub fn builder() -> R2ConfigBuilder {
        R2ConfigBuilder::default()
    }

    /// Loads required settings from `R2_ACCOUNT_ID`, `R2_ACCESS_KEY_ID`, and
    /// `R2_SECRET_ACCESS_KEY`.
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::builder()
            .account_id(required_env("R2_ACCOUNT_ID")?)
            .access_key_id(required_env("R2_ACCESS_KEY_ID")?)
            .secret_access_key(required_env("R2_SECRET_ACCESS_KEY")?)
            .build()
    }

    /// Returns the Cloudflare account ID owning the R2 bucket.
    pub fn account_id(&self) -> &str {
        &self.account_id
    }

    /// Returns the access key ID used for S3-compatible authentication.
    pub fn access_key_id(&self) -> &str {
        &self.access_key_id
    }

    /// Returns the secret access key used for S3-compatible authentication.
    ///
    /// Take care not to log this value.
    pub fn secret_access_key(&self) -> &str {
        &self.secret_access_key
    }

    /// Returns R2's required S3-compatible region, always `auto`.
    pub const fn region(&self) -> &'static str {
        R2_REGION
    }

    /// Returns the account-scoped R2 S3 endpoint.
    pub fn endpoint_url(&self) -> String {
        format!("https://{}.r2.cloudflarestorage.com", self.account_id)
    }
}

/// Builder for [`R2Config`].
#[derive(Debug, Default, Clone)]
pub struct R2ConfigBuilder {
    account_id: Option<String>,
    access_key_id: Option<String>,
    secret_access_key: Option<String>,
}

impl R2ConfigBuilder {
    /// Sets the Cloudflare account ID.
    #[must_use]
    pub fn account_id(mut self, value: impl Into<String>) -> Self {
        self.account_id = Some(value.into());
        self
    }

    /// Sets the R2 access key ID.
    #[must_use]
    pub fn access_key_id(mut self, value: impl Into<String>) -> Self {
        self.access_key_id = Some(value.into());
        self
    }

    /// Sets the R2 secret access key.
    #[must_use]
    pub fn secret_access_key(mut self, value: impl Into<String>) -> Self {
        self.secret_access_key = Some(value.into());
        self
    }

    /// Validates and creates the configuration.
    pub fn build(self) -> Result<R2Config, ConfigError> {
        let account_id = self
            .account_id
            .ok_or(ConfigError::MissingField("account_id"))?;
        if account_id.is_empty() || account_id.chars().any(char::is_whitespace) {
            return Err(ConfigError::InvalidAccountId);
        }

        Ok(R2Config {
            account_id,
            access_key_id: self
                .access_key_id
                .ok_or(ConfigError::MissingField("access_key_id"))?,
            secret_access_key: self
                .secret_access_key
                .ok_or(ConfigError::MissingField("secret_access_key"))?,
        })
    }
}

fn required_env(name: &'static str) -> Result<String, ConfigError> {
    env::var(name).map_err(|_| ConfigError::MissingField(name))
}
