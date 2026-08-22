use std::{
    collections::BTreeMap,
    fmt,
    time::{Duration, SystemTime},
};

use aws_sdk_s3::{
    presigning::PresigningConfig,
    primitives::{ByteStream, DateTime},
};

use crate::{Bucket, Error, PresignedRequest, ValidationError, validation};

const MAX_SINGLE_PUT_SIZE: u64 = 5 * 1024 * 1024 * 1024;
const MAX_LIST_KEYS: u16 = 1_000;

/// Metadata common to downloaded and inspected objects.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObjectMetadata {
    size: u64,
    etag: Option<String>,
    content_type: Option<String>,
    cache_control: Option<String>,
    last_modified: Option<SystemTime>,
    custom: BTreeMap<String, String>,
}

impl ObjectMetadata {
    /// Returns the object size in bytes.
    #[must_use]
    pub const fn size(&self) -> u64 {
        self.size
    }

    /// Returns R2's opaque entity tag when present.
    #[must_use]
    pub fn etag(&self) -> Option<&str> {
        self.etag.as_deref()
    }

    /// Returns the object's media type when present.
    #[must_use]
    pub fn content_type(&self) -> Option<&str> {
        self.content_type.as_deref()
    }

    /// Returns the object's cache policy when present.
    #[must_use]
    pub fn cache_control(&self) -> Option<&str> {
        self.cache_control.as_deref()
    }

    /// Returns the object's last modification time when present.
    #[must_use]
    pub const fn last_modified(&self) -> Option<SystemTime> {
        self.last_modified
    }

    /// Returns user-defined object metadata.
    #[must_use]
    pub const fn custom(&self) -> &BTreeMap<String, String> {
        &self.custom
    }
}

/// A streaming object download and its response metadata.
pub struct DownloadedObject {
    metadata: ObjectMetadata,
    body: ByteStream,
}

impl DownloadedObject {
    /// Returns metadata reported with the download.
    #[must_use]
    pub const fn metadata(&self) -> &ObjectMetadata {
        &self.metadata
    }

    /// Returns a shared reference to the streaming body.
    #[must_use]
    pub const fn body(&self) -> &ByteStream {
        &self.body
    }

    /// Consumes the response and returns its streaming body.
    #[must_use]
    pub fn into_body(self) -> ByteStream {
        self.body
    }
}

impl fmt::Debug for DownloadedObject {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DownloadedObject")
            .field("metadata", &self.metadata)
            .field("body", &"ByteStream(..)")
            .finish()
    }
}

/// Result metadata for a successful single-request upload.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PutObjectResult {
    etag: Option<String>,
}

impl PutObjectResult {
    /// Returns R2's opaque entity tag when present.
    #[must_use]
    pub fn etag(&self) -> Option<&str> {
        self.etag.as_deref()
    }
}

/// A presigned single-request upload with an exact expected body length.
#[derive(Clone, Debug)]
pub struct PresignedPutObject {
    content_length: u64,
    request: PresignedRequest,
}

impl PresignedPutObject {
    /// Returns the exact body length the uploader must send.
    #[must_use]
    pub const fn content_length(&self) -> u64 {
        self.content_length
    }

    /// Returns the signed PUT request.
    #[must_use]
    pub const fn request(&self) -> &PresignedRequest {
        &self.request
    }

    /// Consumes this value and returns the signed PUT request.
    #[must_use]
    pub fn into_request(self) -> PresignedRequest {
        self.request
    }
}

/// One object returned by a bucket listing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObjectSummary {
    key: String,
    size: u64,
    etag: Option<String>,
    last_modified: Option<SystemTime>,
}

impl ObjectSummary {
    /// Returns the object key.
    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }

    /// Returns the object size in bytes.
    #[must_use]
    pub const fn size(&self) -> u64 {
        self.size
    }

    /// Returns R2's opaque entity tag when present.
    #[must_use]
    pub fn etag(&self) -> Option<&str> {
        self.etag.as_deref()
    }

    /// Returns the object's last modification time when present.
    #[must_use]
    pub const fn last_modified(&self) -> Option<SystemTime> {
        self.last_modified
    }
}

/// A single page from an R2 object listing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObjectPage {
    objects: Vec<ObjectSummary>,
    common_prefixes: Vec<String>,
    next_continuation_token: Option<String>,
}

impl ObjectPage {
    /// Returns objects in this page.
    #[must_use]
    pub fn objects(&self) -> &[ObjectSummary] {
        &self.objects
    }

    /// Returns rolled-up prefixes when a delimiter was requested.
    #[must_use]
    pub fn common_prefixes(&self) -> &[String] {
        &self.common_prefixes
    }

    /// Returns the opaque token needed to request the next page.
    #[must_use]
    pub fn next_continuation_token(&self) -> Option<&str> {
        self.next_continuation_token.as_deref()
    }
}

/// Builder for one bounded page of an R2 object listing.
#[derive(Clone, Debug)]
pub struct ListObjectsBuilder {
    bucket: Bucket,
    prefix: Option<String>,
    delimiter: Option<String>,
    limit: u16,
    continuation_token: Option<String>,
}

impl ListObjectsBuilder {
    /// Restricts results to keys beginning with this prefix.
    #[must_use]
    pub fn prefix(mut self, value: impl Into<String>) -> Self {
        self.prefix = Some(value.into());
        self
    }

    /// Groups keys by this delimiter and returns rolled-up common prefixes.
    #[must_use]
    pub fn delimiter(mut self, value: impl Into<String>) -> Self {
        self.delimiter = Some(value.into());
        self
    }

    /// Sets the maximum number of entries returned, from 1 through 1,000.
    #[must_use]
    pub const fn limit(mut self, value: u16) -> Self {
        self.limit = value;
        self
    }

    /// Continues from an opaque token returned by a previous page.
    #[must_use]
    pub fn continuation_token(mut self, value: impl Into<String>) -> Self {
        self.continuation_token = Some(value.into());
        self
    }

    /// Validates the request and fetches one page.
    pub async fn send(self) -> Result<ObjectPage, Error> {
        if self.limit == 0 || self.limit > MAX_LIST_KEYS {
            return Err(ValidationError::ListLimitOutOfRange {
                provided: self.limit,
                min: 1,
                max: MAX_LIST_KEYS,
            }
            .into());
        }
        if let Some(prefix) = self.prefix.as_deref() {
            validation::validate_prefix(prefix)?;
        }
        if self.delimiter.as_ref().is_some_and(String::is_empty) {
            return Err(Error::InvalidInput {
                field: "delimiter",
                reason: "must not be empty",
            });
        }
        if self
            .delimiter
            .as_ref()
            .is_some_and(|delimiter| delimiter.len() > validation::MAX_KEY_BYTES)
        {
            return Err(Error::InvalidInput {
                field: "delimiter",
                reason: "must not exceed 1,024 UTF-8 bytes",
            });
        }
        if self
            .continuation_token
            .as_ref()
            .is_some_and(String::is_empty)
        {
            return Err(Error::InvalidInput {
                field: "continuation_token",
                reason: "must not be empty",
            });
        }

        let output = self
            .bucket
            .client
            .as_sdk()
            .list_objects_v2()
            .bucket(&self.bucket.name)
            .set_prefix(self.prefix)
            .set_delimiter(self.delimiter)
            .max_keys(i32::from(self.limit))
            .set_continuation_token(self.continuation_token)
            .send()
            .await
            .map_err(|error| Error::remote("ListObjectsV2", &error))?;

        let objects = output
            .contents()
            .iter()
            .map(|object| {
                let key = object.key().ok_or(Error::Service {
                    operation: "ListObjectsV2",
                })?;
                let size = non_negative_size(object.size(), "ListObjectsV2")?;
                Ok(ObjectSummary {
                    key: key.to_owned(),
                    size,
                    etag: object.e_tag().map(ToOwned::to_owned),
                    last_modified: optional_system_time(
                        object.last_modified().cloned(),
                        "ListObjectsV2",
                    )?,
                })
            })
            .collect::<Result<Vec<_>, Error>>()?;
        let common_prefixes = output
            .common_prefixes()
            .iter()
            .map(|prefix| {
                prefix
                    .prefix()
                    .map(ToOwned::to_owned)
                    .ok_or(Error::Service {
                        operation: "ListObjectsV2",
                    })
            })
            .collect::<Result<Vec<_>, Error>>()?;

        let is_truncated = output.is_truncated();
        let next_continuation_token = output.next_continuation_token;
        if is_truncated == Some(true) && next_continuation_token.is_none() {
            return Err(Error::Service {
                operation: "ListObjectsV2",
            });
        }

        Ok(ObjectPage {
            objects,
            common_prefixes,
            next_continuation_token,
        })
    }
}

impl Bucket {
    /// Creates a temporary signed GET request for one object.
    pub async fn presign_get(
        &self,
        key: impl Into<String>,
        expires_in: Duration,
    ) -> Result<PresignedRequest, Error> {
        let key = key.into();
        validation::validate_key(&key)?;
        validation::validate_expiry(expires_in)?;
        let config = PresigningConfig::expires_in(expires_in).map_err(|_| Error::Presign)?;
        let signed = self
            .client
            .as_sdk()
            .get_object()
            .bucket(&self.name)
            .key(key)
            .presigned(config)
            .await
            .map_err(|_| Error::Presign)?;
        PresignedRequest::from_sdk(signed, expires_in)
    }

    /// Creates a temporary signed PUT request for one object.
    pub async fn presign_put(
        &self,
        key: impl Into<String>,
        content_length: u64,
        expires_in: Duration,
    ) -> Result<PresignedPutObject, Error> {
        let key = key.into();
        validation::validate_key(&key)?;
        validation::validate_expiry(expires_in)?;
        if content_length > MAX_SINGLE_PUT_SIZE {
            return Err(ValidationError::SingleUploadTooLarge {
                provided: content_length,
                max: MAX_SINGLE_PUT_SIZE,
            }
            .into());
        }
        let config = PresigningConfig::expires_in(expires_in).map_err(|_| Error::Presign)?;
        let signed = self
            .client
            .as_sdk()
            .put_object()
            .bucket(&self.name)
            .key(key)
            .content_length(content_length as i64)
            .presigned(config)
            .await
            .map_err(|_| Error::Presign)?;
        Ok(PresignedPutObject {
            content_length,
            request: PresignedRequest::from_sdk(signed, expires_in)?,
        })
    }

    /// Uploads an in-memory object with a single R2 request.
    pub async fn put_bytes(
        &self,
        key: impl Into<String>,
        bytes: impl Into<Vec<u8>>,
    ) -> Result<PutObjectResult, Error> {
        let bytes = bytes.into();
        let content_length = bytes.len() as u64;
        self.put_stream(key, ByteStream::from(bytes), content_length)
            .await
    }

    /// Uploads a streaming body with a declared byte length using one request.
    pub async fn put_stream(
        &self,
        key: impl Into<String>,
        body: ByteStream,
        content_length: u64,
    ) -> Result<PutObjectResult, Error> {
        let key = key.into();
        validation::validate_key(&key)?;
        if content_length > MAX_SINGLE_PUT_SIZE {
            return Err(ValidationError::SingleUploadTooLarge {
                provided: content_length,
                max: MAX_SINGLE_PUT_SIZE,
            }
            .into());
        }
        let output = self
            .client
            .as_sdk()
            .put_object()
            .bucket(&self.name)
            .key(key)
            .content_length(content_length as i64)
            .body(body)
            .send()
            .await
            .map_err(|error| Error::remote("PutObject", &error))?;
        Ok(PutObjectResult { etag: output.e_tag })
    }

    /// Downloads an object as a stream.
    pub async fn get(&self, key: impl Into<String>) -> Result<DownloadedObject, Error> {
        let key = key.into();
        validation::validate_key(&key)?;
        let output = self
            .client
            .as_sdk()
            .get_object()
            .bucket(&self.name)
            .key(key)
            .send()
            .await
            .map_err(|error| {
                if error
                    .raw_response()
                    .is_some_and(|response| response.status().as_u16() == 404)
                {
                    Error::NotFound
                } else {
                    Error::remote("GetObject", &error)
                }
            })?;
        let metadata = ObjectMetadata {
            size: non_negative_size(output.content_length, "GetObject")?,
            etag: output.e_tag,
            content_type: output.content_type,
            cache_control: output.cache_control,
            last_modified: optional_system_time(output.last_modified, "GetObject")?,
            custom: output.metadata.unwrap_or_default().into_iter().collect(),
        };
        Ok(DownloadedObject {
            metadata,
            body: output.body,
        })
    }

    /// Fetches object metadata without downloading its body.
    pub async fn head(&self, key: impl Into<String>) -> Result<ObjectMetadata, Error> {
        let key = key.into();
        validation::validate_key(&key)?;
        let output = self
            .client
            .as_sdk()
            .head_object()
            .bucket(&self.name)
            .key(key)
            .send()
            .await
            .map_err(|error| {
                if error
                    .raw_response()
                    .is_some_and(|response| response.status().as_u16() == 404)
                {
                    Error::NotFound
                } else {
                    Error::remote("HeadObject", &error)
                }
            })?;
        Ok(ObjectMetadata {
            size: non_negative_size(output.content_length, "HeadObject")?,
            etag: output.e_tag,
            content_type: output.content_type,
            cache_control: output.cache_control,
            last_modified: optional_system_time(output.last_modified, "HeadObject")?,
            custom: output.metadata.unwrap_or_default().into_iter().collect(),
        })
    }

    /// Deletes an object. R2 treats deleting a missing key as success.
    pub async fn delete(&self, key: impl Into<String>) -> Result<(), Error> {
        let key = key.into();
        validation::validate_key(&key)?;
        self.client
            .as_sdk()
            .delete_object()
            .bucket(&self.name)
            .key(key)
            .send()
            .await
            .map_err(|error| Error::remote("DeleteObject", &error))?;
        Ok(())
    }

    /// Starts a bounded, paginated object listing.
    #[must_use]
    pub fn list(&self) -> ListObjectsBuilder {
        ListObjectsBuilder {
            bucket: self.clone(),
            prefix: None,
            delimiter: None,
            limit: MAX_LIST_KEYS,
            continuation_token: None,
        }
    }
}

fn non_negative_size(value: Option<i64>, operation: &'static str) -> Result<u64, Error> {
    value
        .and_then(|size| u64::try_from(size).ok())
        .ok_or(Error::Service { operation })
}

fn optional_system_time(
    value: Option<DateTime>,
    operation: &'static str,
) -> Result<Option<SystemTime>, Error> {
    value
        .map(SystemTime::try_from)
        .transpose()
        .map_err(|_| Error::Service { operation })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_reported_sizes() {
        assert!(non_negative_size(None, "test").is_err());
        assert!(non_negative_size(Some(-1), "test").is_err());
        assert_eq!(non_negative_size(Some(0), "test").unwrap(), 0);
    }
}
