use std::{
    collections::{BTreeMap, HashMap},
    fmt,
    time::{Duration, SystemTime},
};

use aws_sdk_s3::{
    presigning::PresigningConfig,
    primitives::{ByteStream, DateTime},
    types::{Delete, ObjectIdentifier},
};
use aws_smithy_types::date_time::Format as DateTimeFormat;
use futures_util::{Stream, stream};
use headers::Header;
use mime::Mime;
use oxilangtag::LanguageTag;

use crate::{Bucket, Error, PresignedRequest, ValidationError, validation};

const MAX_SINGLE_PUT_SIZE: u64 = 5 * 1024 * 1024 * 1024;
const MAX_LIST_KEYS: u16 = 1_000;
const MAX_DELETE_KEYS: usize = 1_000;
const MAX_OBJECT_METADATA_BYTES: usize = 8_192;

/// Typed system metadata applied when an object is created.
///
/// These values are stored by R2 on the completed object. For a presigned
/// single PUT, every returned required header must be replayed exactly by the
/// uploader. Multipart metadata is applied once when the session is created,
/// not on individual part requests.
#[derive(Clone, Debug, Default)]
pub struct ObjectUploadOptions {
    content_type: Option<Mime>,
    cache_control: Option<headers::CacheControl>,
    content_disposition: Option<String>,
    content_encoding: Option<String>,
    content_language: Option<String>,
    expires: Option<SystemTime>,
    custom: BTreeMap<String, String>,
}

impl ObjectUploadOptions {
    /// Starts building typed upload metadata.
    #[must_use]
    pub const fn builder() -> ObjectUploadOptionsBuilder {
        ObjectUploadOptionsBuilder {
            content_type: None,
            cache_control: None,
            content_disposition: None,
            content_encoding: None,
            content_language: None,
            expires: None,
            custom: BTreeMap::new(),
        }
    }

    /// Returns the configured media type.
    #[must_use]
    pub const fn content_type(&self) -> Option<&Mime> {
        self.content_type.as_ref()
    }

    /// Returns the configured cache policy.
    #[must_use]
    pub const fn cache_control(&self) -> Option<&headers::CacheControl> {
        self.cache_control.as_ref()
    }

    /// Returns the configured content disposition.
    #[must_use]
    pub fn content_disposition(&self) -> Option<&str> {
        self.content_disposition.as_deref()
    }

    /// Returns the configured content encoding.
    #[must_use]
    pub fn content_encoding(&self) -> Option<&str> {
        self.content_encoding.as_deref()
    }

    /// Returns the configured content language.
    #[must_use]
    pub fn content_language(&self) -> Option<&str> {
        self.content_language.as_deref()
    }

    /// Returns the configured HTTP expiration time.
    #[must_use]
    pub const fn expires(&self) -> Option<SystemTime> {
        self.expires
    }

    /// Returns user-defined metadata without the `x-amz-meta-` prefix.
    #[must_use]
    pub const fn custom_metadata(&self) -> &BTreeMap<String, String> {
        &self.custom
    }

    /// Returns whether no system metadata was configured.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.content_type.is_none()
            && self.cache_control.is_none()
            && self.content_disposition.is_none()
            && self.content_encoding.is_none()
            && self.content_language.is_none()
            && self.expires.is_none()
            && self.custom.is_empty()
    }

    /// Returns a copy configured with this MIME media type.
    #[must_use]
    pub fn with_content_type(mut self, value: Mime) -> Self {
        self.content_type = Some(value);
        self
    }

    /// Returns a copy configured with this typed HTTP cache policy.
    #[must_use]
    pub fn with_cache_control(mut self, value: headers::CacheControl) -> Self {
        self.cache_control = Some(value);
        self
    }

    /// Returns a copy configured with this content disposition.
    #[must_use]
    pub fn with_content_disposition(mut self, value: impl Into<String>) -> Self {
        self.content_disposition = Some(value.into());
        self
    }

    /// Returns a copy configured with this content encoding.
    #[must_use]
    pub fn with_content_encoding(mut self, value: impl Into<String>) -> Self {
        self.content_encoding = Some(value.into());
        self
    }

    /// Returns a copy configured with this content language.
    #[must_use]
    pub fn with_content_language(mut self, value: impl Into<String>) -> Self {
        self.content_language = Some(value.into());
        self
    }

    /// Returns a copy configured with this HTTP expiration time.
    #[must_use]
    pub fn with_expires(mut self, value: SystemTime) -> Self {
        self.expires = Some(value);
        self
    }

    /// Returns a copy containing this user-defined metadata entry.
    ///
    /// The key is supplied without the `x-amz-meta-` prefix. Keys are
    /// canonicalized to lowercase when the options are validated.
    #[must_use]
    pub fn with_custom_metadata(
        mut self,
        key: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        self.custom.insert(key.into(), value.into());
        self
    }

    pub(crate) fn content_type_value(&self) -> Option<String> {
        self.content_type.as_ref().map(ToString::to_string)
    }

    pub(crate) fn cache_control_value(&self) -> Option<String> {
        self.cache_control.as_ref().map(encode_header)
    }

    pub(crate) fn content_disposition_value(&self) -> Option<String> {
        self.content_disposition.clone()
    }

    pub(crate) fn content_encoding_value(&self) -> Option<String> {
        self.content_encoding.clone()
    }

    pub(crate) fn content_language_value(&self) -> Option<String> {
        self.content_language.clone()
    }

    pub(crate) fn expires_value(&self) -> Option<DateTime> {
        self.expires.map(DateTime::from)
    }

    pub(crate) fn custom_metadata_values(&self) -> Option<HashMap<String, String>> {
        (!self.custom.is_empty()).then(|| {
            self.custom
                .iter()
                .map(|(key, value)| (key.to_ascii_lowercase(), value.clone()))
                .collect()
        })
    }

    pub(crate) fn validate(&self) -> Result<(), Error> {
        for (field, value) in [
            ("content_disposition", self.content_disposition.as_deref()),
            ("content_encoding", self.content_encoding.as_deref()),
        ] {
            if let Some(value) = value {
                validate_metadata_header(field, value)?;
            }
        }
        if let Some(value) = self.content_language.as_deref() {
            validate_content_language(value)?;
        }

        let mut total_bytes = self
            .content_type_value()
            .map_or(0, |value| "content-type".len() + value.len())
            + self
                .cache_control_value()
                .map_or(0, |value| "cache-control".len() + value.len())
            + self
                .content_disposition
                .as_ref()
                .map_or(0, |value| "content-disposition".len() + value.len())
            + self
                .content_encoding
                .as_ref()
                .map_or(0, |value| "content-encoding".len() + value.len())
            + self
                .content_language
                .as_ref()
                .map_or(0, |value| "content-language".len() + value.len());
        let mut normalized = BTreeMap::new();
        for (key, value) in &self.custom {
            validate_metadata_key(key)?;
            validate_metadata_header("custom_metadata", value)?;
            let key = key.to_ascii_lowercase();
            if normalized.insert(key.clone(), ()).is_some() {
                return Err(Error::InvalidInput {
                    field: "custom_metadata",
                    reason: "contains duplicate keys after ASCII case normalization",
                });
            }
            total_bytes = total_bytes
                .saturating_add("x-amz-meta-".len())
                .saturating_add(key.len())
                .saturating_add(value.len());
        }
        if total_bytes > MAX_OBJECT_METADATA_BYTES {
            return Err(Error::InvalidInput {
                field: "upload_options",
                reason: "metadata exceeds R2's 8,192-byte object metadata limit",
            });
        }
        Ok(())
    }
}

/// Builds typed metadata for a newly uploaded object.
#[derive(Clone, Debug, Default)]
pub struct ObjectUploadOptionsBuilder {
    content_type: Option<Mime>,
    cache_control: Option<headers::CacheControl>,
    content_disposition: Option<String>,
    content_encoding: Option<String>,
    content_language: Option<String>,
    expires: Option<SystemTime>,
    custom: BTreeMap<String, String>,
}

impl ObjectUploadOptionsBuilder {
    /// Sets the object's MIME media type.
    #[must_use]
    pub fn content_type(mut self, value: Mime) -> Self {
        self.content_type = Some(value);
        self
    }

    /// Sets the object's typed HTTP cache policy.
    #[must_use]
    pub fn cache_control(mut self, value: headers::CacheControl) -> Self {
        self.cache_control = Some(value);
        self
    }

    /// Sets the object's content disposition.
    #[must_use]
    pub fn content_disposition(mut self, value: impl Into<String>) -> Self {
        self.content_disposition = Some(value.into());
        self
    }

    /// Sets the object's content encoding.
    #[must_use]
    pub fn content_encoding(mut self, value: impl Into<String>) -> Self {
        self.content_encoding = Some(value.into());
        self
    }

    /// Sets the object's content language.
    #[must_use]
    pub fn content_language(mut self, value: impl Into<String>) -> Self {
        self.content_language = Some(value.into());
        self
    }

    /// Sets the object's HTTP expiration time.
    #[must_use]
    pub fn expires(mut self, value: SystemTime) -> Self {
        self.expires = Some(value);
        self
    }

    /// Adds user-defined metadata without the `x-amz-meta-` prefix.
    #[must_use]
    pub fn custom_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.custom.insert(key.into(), value.into());
        self
    }

    /// Finishes the immutable upload metadata value.
    #[must_use]
    pub fn build(self) -> ObjectUploadOptions {
        ObjectUploadOptions {
            content_type: self.content_type,
            cache_control: self.cache_control,
            content_disposition: self.content_disposition,
            content_encoding: self.content_encoding,
            content_language: self.content_language,
            expires: self.expires,
            custom: self.custom,
        }
    }
}

fn validate_metadata_header(field: &'static str, value: &str) -> Result<(), Error> {
    if value.is_empty() {
        return Err(Error::InvalidInput {
            field,
            reason: "must not be empty",
        });
    }
    if !value
        .bytes()
        .all(|byte| matches!(byte, b'\t' | 0x20..=0x7e))
    {
        return Err(Error::InvalidInput {
            field,
            reason: "must contain only visible ASCII or horizontal tabs",
        });
    }
    Ok(())
}

fn validate_content_language(value: &str) -> Result<(), Error> {
    validate_metadata_header("content_language", value)?;
    if value.split(',').any(|tag| {
        let tag = tag.trim();
        tag.is_empty() || LanguageTag::parse(tag).is_err()
    }) {
        return Err(Error::InvalidInput {
            field: "content_language",
            reason: "must contain one or more well-formed BCP 47 language tags",
        });
    }
    Ok(())
}

fn validate_metadata_key(key: &str) -> Result<(), Error> {
    if key.is_empty() {
        return Err(Error::InvalidInput {
            field: "custom_metadata",
            reason: "keys must not be empty",
        });
    }
    if key.to_ascii_lowercase().starts_with("x-amz-meta-") {
        return Err(Error::InvalidInput {
            field: "custom_metadata",
            reason: "keys must omit the x-amz-meta- prefix",
        });
    }
    if !key
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(Error::InvalidInput {
            field: "custom_metadata",
            reason: "keys must use ASCII letters, digits, hyphens, underscores, or periods",
        });
    }
    Ok(())
}

fn encode_header(value: &impl Header) -> String {
    let mut encoded = Vec::with_capacity(1);
    value.encode(&mut encoded);
    encoded
        .into_iter()
        .next()
        .expect("typed header must encode one value")
        .to_str()
        .expect("typed cache policy must be visible ASCII")
        .to_owned()
}

/// Metadata common to downloaded and inspected objects.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObjectMetadata {
    size: u64,
    etag: Option<String>,
    content_type: Option<String>,
    cache_control: Option<String>,
    content_disposition: Option<String>,
    content_encoding: Option<String>,
    content_language: Option<String>,
    expires: Option<SystemTime>,
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

    /// Returns the response content disposition when present.
    #[must_use]
    pub fn content_disposition(&self) -> Option<&str> {
        self.content_disposition.as_deref()
    }

    /// Returns the response content encoding when present.
    #[must_use]
    pub fn content_encoding(&self) -> Option<&str> {
        self.content_encoding.as_deref()
    }

    /// Returns the response content language when present.
    #[must_use]
    pub fn content_language(&self) -> Option<&str> {
        self.content_language.as_deref()
    }

    /// Returns the HTTP expiration time when present.
    #[must_use]
    pub const fn expires(&self) -> Option<SystemTime> {
        self.expires
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

/// One per-key failure returned by an R2 multi-object delete request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeleteObjectFailure {
    key: String,
    code: Option<String>,
    message: Option<String>,
}

impl DeleteObjectFailure {
    /// Returns the key R2 did not delete.
    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }

    /// Returns R2's machine-readable error code when present.
    #[must_use]
    pub fn code(&self) -> Option<&str> {
        self.code.as_deref()
    }

    /// Returns R2's error message when present.
    #[must_use]
    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }
}

/// Aggregate result of one or more `DeleteObjects` requests.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DeleteObjectsResult {
    deleted_keys: Vec<String>,
    failures: Vec<DeleteObjectFailure>,
    request_count: usize,
}

impl DeleteObjectsResult {
    /// Returns keys R2 reported as deleted.
    #[must_use]
    pub fn deleted_keys(&self) -> &[String] {
        &self.deleted_keys
    }

    /// Returns per-key failures reported inside successful HTTP responses.
    #[must_use]
    pub fn failures(&self) -> &[DeleteObjectFailure] {
        &self.failures
    }

    /// Returns the number of completed `DeleteObjects` requests.
    #[must_use]
    pub const fn request_count(&self) -> usize {
        self.request_count
    }

    /// Returns whether every requested key was reported as deleted.
    #[must_use]
    pub const fn is_complete(&self) -> bool {
        self.failures.is_empty()
    }
}

/// A request-level batch delete error with results from earlier batches.
#[derive(Debug)]
pub struct BatchDeleteError {
    error: Error,
    partial: DeleteObjectsResult,
}

impl BatchDeleteError {
    /// Returns the sanitized request failure.
    #[must_use]
    pub const fn error(&self) -> &Error {
        &self.error
    }

    /// Returns results accumulated before the failed request.
    #[must_use]
    pub const fn partial_result(&self) -> &DeleteObjectsResult {
        &self.partial
    }
}

impl fmt::Display for BatchDeleteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "batch delete failed: {}", self.error)
    }
}

impl std::error::Error for BatchDeleteError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
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

        let next_continuation_token = if output.is_truncated() == Some(true) {
            match output.next_continuation_token {
                Some(token) if !token.is_empty() => Some(token),
                _ => {
                    return Err(Error::Service {
                        operation: "ListObjectsV2",
                    });
                }
            }
        } else {
            None
        };

        Ok(ObjectPage {
            objects,
            common_prefixes,
            next_continuation_token,
        })
    }

    /// Streams every listing page until R2 reports that the listing is complete.
    ///
    /// The configured page limit applies to each request. Page boundaries and
    /// common prefixes are preserved.
    pub fn into_pages(self) -> impl Stream<Item = Result<ObjectPage, Error>> + Send {
        stream::try_unfold(Some(self), |state| async move {
            let Some(builder) = state else {
                return Ok(None);
            };
            let previous_token = builder.continuation_token.clone();
            let next_builder = builder.clone();
            let page = builder.send().await?;
            let next_token = page.next_continuation_token.clone();
            if next_token.is_some() && next_token == previous_token {
                return Err(Error::Service {
                    operation: "ListObjectsV2",
                });
            }
            let state = next_token.map(|token| next_builder.continuation_token(token));
            Ok(Some((page, state)))
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
        self.presign_put_with_options(
            key,
            content_length,
            expires_in,
            ObjectUploadOptions::default(),
        )
        .await
    }

    /// Creates a temporary signed PUT with typed object metadata.
    ///
    /// Metadata headers included in the returned request are part of its
    /// signature and must be replayed exactly by the uploader.
    pub async fn presign_put_with_options(
        &self,
        key: impl Into<String>,
        content_length: u64,
        expires_in: Duration,
        options: ObjectUploadOptions,
    ) -> Result<PresignedPutObject, Error> {
        let key = key.into();
        validation::validate_key(&key)?;
        validation::validate_expiry(expires_in)?;
        options.validate()?;
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
            .set_content_type(options.content_type_value())
            .set_cache_control(options.cache_control_value())
            .set_content_disposition(options.content_disposition_value())
            .set_content_encoding(options.content_encoding_value())
            .set_content_language(options.content_language_value())
            .set_expires(options.expires_value())
            .set_metadata(options.custom_metadata_values())
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
        self.put_bytes_with_options(key, bytes, ObjectUploadOptions::default())
            .await
    }

    /// Uploads in-memory bytes with typed object metadata.
    pub async fn put_bytes_with_options(
        &self,
        key: impl Into<String>,
        bytes: impl Into<Vec<u8>>,
        options: ObjectUploadOptions,
    ) -> Result<PutObjectResult, Error> {
        let bytes = bytes.into();
        let content_length = bytes.len() as u64;
        self.put_stream_with_options(key, ByteStream::from(bytes), content_length, options)
            .await
    }

    /// Uploads a streaming body with a declared byte length using one request.
    pub async fn put_stream(
        &self,
        key: impl Into<String>,
        body: ByteStream,
        content_length: u64,
    ) -> Result<PutObjectResult, Error> {
        self.put_stream_with_options(key, body, content_length, ObjectUploadOptions::default())
            .await
    }

    /// Uploads a streaming body with typed object metadata.
    pub async fn put_stream_with_options(
        &self,
        key: impl Into<String>,
        body: ByteStream,
        content_length: u64,
        options: ObjectUploadOptions,
    ) -> Result<PutObjectResult, Error> {
        let key = key.into();
        validation::validate_key(&key)?;
        options.validate()?;
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
            .set_content_type(options.content_type_value())
            .set_cache_control(options.cache_control_value())
            .set_content_disposition(options.content_disposition_value())
            .set_content_encoding(options.content_encoding_value())
            .set_content_language(options.content_language_value())
            .set_expires(options.expires_value())
            .set_metadata(options.custom_metadata_values())
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
            content_disposition: output.content_disposition,
            content_encoding: output.content_encoding,
            content_language: output.content_language,
            expires: optional_http_date(output.expires_string.as_deref(), "GetObject")?,
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
            content_disposition: output.content_disposition,
            content_encoding: output.content_encoding,
            content_language: output.content_language,
            expires: optional_http_date(output.expires_string.as_deref(), "HeadObject")?,
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

    /// Deletes arbitrary many keys in sequential batches of at most 1,000.
    ///
    /// Every key is validated before the first remote mutation. An HTTP-successful
    /// response can still contain per-key failures, which are returned in the
    /// result. If a later request fails, [`BatchDeleteError::partial_result`]
    /// retains outcomes from all earlier batches.
    pub async fn delete_objects<I, K>(
        &self,
        keys: I,
    ) -> Result<DeleteObjectsResult, BatchDeleteError>
    where
        I: IntoIterator<Item = K>,
        K: Into<String>,
    {
        let keys = keys.into_iter().map(Into::into).collect::<Vec<_>>();
        for key in &keys {
            validation::validate_key(key).map_err(|error| BatchDeleteError {
                error,
                partial: DeleteObjectsResult::default(),
            })?;
        }

        let mut result = DeleteObjectsResult::default();
        for keys in delete_batches(&keys) {
            let objects = keys
                .iter()
                .map(|key| {
                    ObjectIdentifier::builder()
                        .key(key)
                        .build()
                        .expect("validated object identifiers contain keys")
                })
                .collect();
            let delete = Delete::builder()
                .set_objects(Some(objects))
                .quiet(false)
                .build()
                .expect("each delete batch contains at least one key");
            let output = self
                .client
                .as_sdk()
                .delete_objects()
                .bucket(&self.name)
                .delete(delete)
                .send()
                .await
                .map_err(|error| BatchDeleteError {
                    error: Error::remote("DeleteObjects", &error),
                    partial: result.clone(),
                })?;
            result.request_count += 1;
            result.deleted_keys.extend(
                output
                    .deleted()
                    .iter()
                    .filter_map(|deleted| deleted.key().map(ToOwned::to_owned)),
            );
            for failure in output.errors() {
                let key = failure.key().ok_or_else(|| BatchDeleteError {
                    error: Error::Service {
                        operation: "DeleteObjects",
                    },
                    partial: result.clone(),
                })?;
                result.failures.push(DeleteObjectFailure {
                    key: key.to_owned(),
                    code: failure.code().map(ToOwned::to_owned),
                    message: failure.message().map(ToOwned::to_owned),
                });
            }
        }
        Ok(result)
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

fn delete_batches(keys: &[String]) -> impl Iterator<Item = &[String]> {
    keys.chunks(MAX_DELETE_KEYS)
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

fn optional_http_date(
    value: Option<&str>,
    operation: &'static str,
) -> Result<Option<SystemTime>, Error> {
    value
        .map(|value| DateTime::from_str(value, DateTimeFormat::HttpDate))
        .transpose()
        .map_err(|_| Error::Service { operation })?
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

    #[test]
    fn delete_batches_never_exceed_r2s_request_limit() {
        let keys = (0..2_001)
            .map(|index| index.to_string())
            .collect::<Vec<_>>();
        let lengths = delete_batches(&keys).map(<[_]>::len).collect::<Vec<_>>();

        assert_eq!(lengths, [1_000, 1_000, 1]);
    }

    #[test]
    fn upload_metadata_validation_is_case_insensitive_and_bounded() {
        let duplicate = ObjectUploadOptions::builder()
            .custom_metadata("Tenant", "one")
            .custom_metadata("tenant", "two")
            .build();
        assert!(duplicate.validate().is_err());

        let too_large = ObjectUploadOptions::builder()
            .custom_metadata("large", "x".repeat(MAX_OBJECT_METADATA_BYTES))
            .build();
        assert!(too_large.validate().is_err());
    }

    #[test]
    fn content_language_accepts_bcp47_lists_and_rejects_malformed_tags() {
        for valid in ["en", "vi", "en-US", "zh-Hant", "x-private", "en, vi"] {
            assert!(validate_content_language(valid).is_ok(), "{valid}");
        }
        for invalid in ["", "en_US", "en--US", "en,", ",vi", "abc ???"] {
            assert!(validate_content_language(invalid).is_err(), "{invalid}");
        }
    }
}
