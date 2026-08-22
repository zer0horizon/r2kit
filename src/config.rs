use std::{env, fmt};

use crate::ConfigError;

const R2_REGION: &str = "auto";

/// Configuration needed to connect an S3-compatible client to Cloudflare R2.
#[derive(Clone)]
pub struct R2Config {
    account_id: String,
    access_key_id: String,
    secret_access_key: String,
    session_token: Option<String>,
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
            .optional_session_token(env::var("R2_SESSION_TOKEN").ok())
            .build()
    }

    /// Returns the Cloudflare account ID owning the R2 bucket.
    pub fn account_id(&self) -> &str {
        &self.account_id
    }

    pub(crate) fn access_key_id(&self) -> &str {
        &self.access_key_id
    }

    pub(crate) fn secret_access_key(&self) -> &str {
        &self.secret_access_key
    }

    pub(crate) fn session_token(&self) -> Option<&str> {
        self.session_token.as_deref()
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

impl fmt::Debug for R2Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("R2Config")
            .field("account_id", &self.account_id)
            .field("access_key_id", &"[REDACTED]")
            .field("secret_access_key", &"[REDACTED]")
            .field(
                "session_token",
                &self.session_token.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

/// Builder for [`R2Config`].
#[derive(Default, Clone)]
pub struct R2ConfigBuilder {
    account_id: Option<String>,
    access_key_id: Option<String>,
    secret_access_key: Option<String>,
    session_token: Option<String>,
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

    /// Sets a temporary credential session token.
    #[must_use]
    pub fn session_token(mut self, value: impl Into<String>) -> Self {
        self.session_token = Some(value.into());
        self
    }

    pub(crate) fn optional_session_token(mut self, value: Option<String>) -> Self {
        self.session_token = value;
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

        let access_key_id = required_non_empty(self.access_key_id, "access_key_id")?;
        let secret_access_key = required_non_empty(self.secret_access_key, "secret_access_key")?;
        if self
            .session_token
            .as_ref()
            .is_some_and(|value| value.is_empty())
        {
            return Err(ConfigError::EmptyField("session_token"));
        }

        Ok(R2Config {
            account_id,
            access_key_id,
            secret_access_key,
            session_token: self.session_token,
        })
    }
}

impl fmt::Debug for R2ConfigBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("R2ConfigBuilder")
            .field("account_id", &self.account_id)
            .field(
                "access_key_id",
                &self.access_key_id.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "secret_access_key",
                &self.secret_access_key.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "session_token",
                &self.session_token.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

fn required_env(name: &'static str) -> Result<String, ConfigError> {
    env::var(name).map_err(|_| ConfigError::MissingField(name))
}

fn required_non_empty(value: Option<String>, name: &'static str) -> Result<String, ConfigError> {
    let value = value.ok_or(ConfigError::MissingField(name))?;
    if value.is_empty() {
        return Err(ConfigError::EmptyField(name));
    }
    Ok(value)
}
