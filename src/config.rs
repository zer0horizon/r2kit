use std::{env, fmt, str::FromStr, time::Duration};

use crate::ConfigError;

const R2_REGION: &str = "auto";

/// Data-residency jurisdiction used by an R2 bucket.
///
/// This is distinct from an R2 location hint. A jurisdiction changes the S3
/// endpoint and restricts the client to buckets in that jurisdiction.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum R2Jurisdiction {
    /// The global R2 endpoint, with no jurisdictional restriction.
    #[default]
    Default,
    /// European Union jurisdiction.
    Eu,
    /// United States jurisdiction.
    Us,
    /// FedRAMP jurisdiction.
    FedRamp,
}

impl R2Jurisdiction {
    /// Returns the jurisdiction identifier used in an R2 endpoint.
    ///
    /// The default jurisdiction has no endpoint identifier.
    #[must_use]
    pub const fn as_str(self) -> Option<&'static str> {
        match self {
            Self::Default => None,
            Self::Eu => Some("eu"),
            Self::Us => Some("us"),
            Self::FedRamp => Some("fedramp"),
        }
    }
}

impl FromStr for R2Jurisdiction {
    type Err = ConfigError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "default" => Ok(Self::Default),
            "eu" => Ok(Self::Eu),
            "us" => Ok(Self::Us),
            "fedramp" => Ok(Self::FedRamp),
            _ => Err(ConfigError::InvalidJurisdiction),
        }
    }
}

/// Configuration needed to connect an S3-compatible client to Cloudflare R2.
#[derive(Clone)]
pub struct R2Config {
    account_id: String,
    access_key_id: String,
    secret_access_key: String,
    session_token: Option<String>,
    jurisdiction: R2Jurisdiction,
    connect_timeout: Option<Duration>,
    read_timeout: Option<Duration>,
    operation_timeout: Option<Duration>,
    operation_attempt_timeout: Option<Duration>,
    sdk_max_attempts: Option<u32>,
}

impl R2Config {
    /// Starts a builder for an R2 configuration.
    #[must_use]
    pub fn builder() -> R2ConfigBuilder {
        R2ConfigBuilder::default()
    }

    /// Loads required settings from `R2_ACCOUNT_ID`, `R2_ACCESS_KEY_ID`, and
    /// `R2_SECRET_ACCESS_KEY`. `R2_SESSION_TOKEN` and `R2_JURISDICTION` are
    /// optional.
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_env_with(env::var)
    }

    fn from_env_with(
        mut read: impl FnMut(&'static str) -> Result<String, env::VarError>,
    ) -> Result<Self, ConfigError> {
        let account_id = required_env_with(&mut read, "R2_ACCOUNT_ID")?;
        let access_key_id = required_env_with(&mut read, "R2_ACCESS_KEY_ID")?;
        let secret_access_key = required_env_with(&mut read, "R2_SECRET_ACCESS_KEY")?;
        let session_token = optional_env_with(&mut read, "R2_SESSION_TOKEN")?;
        let jurisdiction = optional_env_with(&mut read, "R2_JURISDICTION")?
            .map_or(Ok(R2Jurisdiction::Default), |value| value.parse())?;
        Self::builder()
            .account_id(account_id)
            .access_key_id(access_key_id)
            .secret_access_key(secret_access_key)
            .optional_session_token(session_token)
            .jurisdiction(jurisdiction)
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

    /// Returns the configured data-residency jurisdiction.
    pub const fn jurisdiction(&self) -> R2Jurisdiction {
        self.jurisdiction
    }

    /// Returns the account-scoped R2 S3 endpoint.
    pub fn endpoint_url(&self) -> String {
        match self.jurisdiction.as_str() {
            Some(jurisdiction) => format!(
                "https://{}.{jurisdiction}.r2.cloudflarestorage.com",
                self.account_id
            ),
            None => format!("https://{}.r2.cloudflarestorage.com", self.account_id),
        }
    }

    /// Returns the socket connection timeout, if explicitly configured.
    pub const fn connect_timeout(&self) -> Option<Duration> {
        self.connect_timeout
    }

    /// Returns the time allowed to receive the first response byte, if configured.
    pub const fn read_timeout(&self) -> Option<Duration> {
        self.read_timeout
    }

    /// Returns the total operation timeout, including retries, if configured.
    pub const fn operation_timeout(&self) -> Option<Duration> {
        self.operation_timeout
    }

    /// Returns the timeout for one operation attempt, if configured.
    pub const fn operation_attempt_timeout(&self) -> Option<Duration> {
        self.operation_attempt_timeout
    }

    /// Returns the AWS SDK request-attempt limit, if explicitly configured.
    ///
    /// Managed multipart part uploads use their own exact attempt limit and do
    /// not inherit this value.
    pub const fn sdk_max_attempts(&self) -> Option<u32> {
        self.sdk_max_attempts
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
            .field("jurisdiction", &self.jurisdiction)
            .field("connect_timeout", &self.connect_timeout)
            .field("read_timeout", &self.read_timeout)
            .field("operation_timeout", &self.operation_timeout)
            .field("operation_attempt_timeout", &self.operation_attempt_timeout)
            .field("sdk_max_attempts", &self.sdk_max_attempts)
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
    jurisdiction: R2Jurisdiction,
    connect_timeout: Option<Duration>,
    read_timeout: Option<Duration>,
    operation_timeout: Option<Duration>,
    operation_attempt_timeout: Option<Duration>,
    sdk_max_attempts: Option<u32>,
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

    /// Selects the endpoint for an R2 data-residency jurisdiction.
    #[must_use]
    pub fn jurisdiction(mut self, value: R2Jurisdiction) -> Self {
        self.jurisdiction = value;
        self
    }

    /// Sets the socket connection timeout.
    #[must_use]
    pub fn connect_timeout(mut self, value: Duration) -> Self {
        self.connect_timeout = Some(value);
        self
    }

    /// Sets the time allowed to receive the first response byte.
    #[must_use]
    pub fn read_timeout(mut self, value: Duration) -> Self {
        self.read_timeout = Some(value);
        self
    }

    /// Sets the total timeout for an operation, including retries.
    #[must_use]
    pub fn operation_timeout(mut self, value: Duration) -> Self {
        self.operation_timeout = Some(value);
        self
    }

    /// Sets the timeout for one operation attempt.
    #[must_use]
    pub fn operation_attempt_timeout(mut self, value: Duration) -> Self {
        self.operation_attempt_timeout = Some(value);
        self
    }

    /// Sets the maximum number of attempts made by the AWS SDK.
    ///
    /// The value includes the initial request and must be at least one.
    /// Managed multipart part uploads deliberately use the attempt limit on
    /// [`crate::ManagedMultipartBuilder`] instead.
    #[must_use]
    pub fn sdk_max_attempts(mut self, value: u32) -> Self {
        self.sdk_max_attempts = Some(value);
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
        if account_id.len() != 32 || !account_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
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

        validate_timeout(self.connect_timeout, "connect_timeout")?;
        validate_timeout(self.read_timeout, "read_timeout")?;
        validate_timeout(self.operation_timeout, "operation_timeout")?;
        validate_timeout(self.operation_attempt_timeout, "operation_attempt_timeout")?;
        if self.sdk_max_attempts == Some(0) {
            return Err(ConfigError::InvalidAttempts("sdk_max_attempts"));
        }
        if self
            .operation_timeout
            .zip(self.operation_attempt_timeout)
            .is_some_and(|(operation, attempt)| attempt > operation)
        {
            return Err(ConfigError::InconsistentTimeouts);
        }

        Ok(R2Config {
            account_id,
            access_key_id,
            secret_access_key,
            session_token: self.session_token,
            jurisdiction: self.jurisdiction,
            connect_timeout: self.connect_timeout,
            read_timeout: self.read_timeout,
            operation_timeout: self.operation_timeout,
            operation_attempt_timeout: self.operation_attempt_timeout,
            sdk_max_attempts: self.sdk_max_attempts,
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
            .field("jurisdiction", &self.jurisdiction)
            .field("connect_timeout", &self.connect_timeout)
            .field("read_timeout", &self.read_timeout)
            .field("operation_timeout", &self.operation_timeout)
            .field("operation_attempt_timeout", &self.operation_attempt_timeout)
            .field("sdk_max_attempts", &self.sdk_max_attempts)
            .finish()
    }
}

fn required_env_with(
    read: &mut impl FnMut(&'static str) -> Result<String, env::VarError>,
    name: &'static str,
) -> Result<String, ConfigError> {
    match read(name) {
        Ok(value) => Ok(value),
        Err(env::VarError::NotPresent) => Err(ConfigError::MissingField(name)),
        Err(env::VarError::NotUnicode(_)) => Err(ConfigError::InvalidEnvironment(name)),
    }
}

fn optional_env_with(
    read: &mut impl FnMut(&'static str) -> Result<String, env::VarError>,
    name: &'static str,
) -> Result<Option<String>, ConfigError> {
    match read(name) {
        Ok(value) => Ok(Some(value)),
        Err(env::VarError::NotPresent) => Ok(None),
        Err(env::VarError::NotUnicode(_)) => Err(ConfigError::InvalidEnvironment(name)),
    }
}

fn required_non_empty(value: Option<String>, name: &'static str) -> Result<String, ConfigError> {
    let value = value.ok_or(ConfigError::MissingField(name))?;
    if value.is_empty() {
        return Err(ConfigError::EmptyField(name));
    }
    Ok(value)
}

fn validate_timeout(value: Option<Duration>, name: &'static str) -> Result<(), ConfigError> {
    if value == Some(Duration::ZERO) {
        return Err(ConfigError::InvalidTimeout(name));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, ffi::OsString};

    use super::*;

    fn valid_environment() -> HashMap<&'static str, String> {
        HashMap::from([
            (
                "R2_ACCOUNT_ID",
                "0123456789abcdef0123456789abcdef".to_owned(),
            ),
            ("R2_ACCESS_KEY_ID", "access".to_owned()),
            ("R2_SECRET_ACCESS_KEY", "secret".to_owned()),
        ])
    }

    #[test]
    fn environment_defaults_to_the_global_jurisdiction() {
        let values = valid_environment();
        let config = R2Config::from_env_with(|name| {
            values.get(name).cloned().ok_or(env::VarError::NotPresent)
        })
        .unwrap();

        assert_eq!(config.jurisdiction(), R2Jurisdiction::Default);
        assert!(config.session_token().is_none());
    }

    #[test]
    fn environment_reports_the_first_missing_required_value() {
        let error = R2Config::from_env_with(|_| Err(env::VarError::NotPresent)).unwrap_err();

        assert_eq!(error, ConfigError::MissingField("R2_ACCOUNT_ID"));
    }

    #[test]
    fn environment_loads_temporary_credentials_and_jurisdiction() {
        let mut values = valid_environment();
        values.insert("R2_SESSION_TOKEN", "session".to_owned());
        values.insert("R2_JURISDICTION", "us".to_owned());
        let config = R2Config::from_env_with(|name| {
            values.get(name).cloned().ok_or(env::VarError::NotPresent)
        })
        .unwrap();

        assert_eq!(config.jurisdiction(), R2Jurisdiction::Us);
        assert_eq!(config.session_token(), Some("session"));
    }

    #[test]
    fn environment_rejects_non_unicode_optional_values() {
        let values = valid_environment();
        let error = R2Config::from_env_with(|name| {
            if name == "R2_JURISDICTION" {
                Err(env::VarError::NotUnicode(OsString::from("invalid")))
            } else {
                values.get(name).cloned().ok_or(env::VarError::NotPresent)
            }
        })
        .unwrap_err();

        assert_eq!(error, ConfigError::InvalidEnvironment("R2_JURISDICTION"));
    }

    #[test]
    fn environment_rejects_an_empty_session_token() {
        let mut values = valid_environment();
        values.insert("R2_SESSION_TOKEN", String::new());
        let error = R2Config::from_env_with(|name| {
            values.get(name).cloned().ok_or(env::VarError::NotPresent)
        })
        .unwrap_err();

        assert_eq!(error, ConfigError::EmptyField("session_token"));
    }
}
