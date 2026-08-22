use std::{fmt, sync::Arc};

use aws_sdk_s3::config::{Credentials, Region};

use crate::{Error, R2Config};

/// A configured Cloudflare R2 client.
#[derive(Clone)]
pub struct R2Client {
    inner: aws_sdk_s3::Client,
}

impl fmt::Debug for R2Client {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("R2Client").finish_non_exhaustive()
    }
}

impl R2Client {
    /// Creates an R2 client from explicit configuration.
    #[must_use]
    pub fn new(config: R2Config) -> Self {
        let credentials = Credentials::new(
            config.access_key_id(),
            config.secret_access_key(),
            config.session_token().map(ToOwned::to_owned),
            None,
            "r2kit",
        );
        let sdk_config = aws_sdk_s3::Config::builder()
            .behavior_version_latest()
            .region(Region::new("auto"))
            .endpoint_url(config.endpoint_url())
            .credentials_provider(credentials)
            .build();

        Self {
            inner: aws_sdk_s3::Client::from_conf(sdk_config),
        }
    }

    /// Creates an R2 client from the standard `R2_*` environment variables.
    pub fn from_env() -> Result<Self, Error> {
        Ok(Self::new(R2Config::from_env()?))
    }

    /// Wraps a preconfigured AWS S3 client.
    #[must_use]
    pub fn from_sdk(client: aws_sdk_s3::Client) -> Self {
        Self { inner: client }
    }

    /// Returns the underlying AWS S3 client for operations not wrapped by `r2kit`.
    #[must_use]
    pub fn as_sdk(&self) -> &aws_sdk_s3::Client {
        &self.inner
    }

    /// Selects and validates an R2 bucket.
    pub fn bucket(&self, name: impl Into<String>) -> Result<Bucket, Error> {
        let name = name.into();
        if name.len() < 3 || name.len() > 64 {
            return Err(Error::InvalidInput {
                field: "bucket",
                reason: "must contain between 3 and 64 bytes",
            });
        }
        if !name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
            || name.starts_with('-')
            || name.ends_with('-')
        {
            return Err(Error::InvalidInput {
                field: "bucket",
                reason: "must use lowercase ASCII letters, digits, or interior hyphens",
            });
        }

        Ok(Bucket {
            client: Arc::new(self.clone()),
            name,
        })
    }
}

/// Operations scoped to one R2 bucket.
#[derive(Clone)]
pub struct Bucket {
    pub(crate) client: Arc<R2Client>,
    pub(crate) name: String,
}

impl fmt::Debug for Bucket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Bucket").field("name", &self.name).finish()
    }
}

impl Bucket {
    /// Returns the bucket name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
}
