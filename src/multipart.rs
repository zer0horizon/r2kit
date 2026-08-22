use std::{
    collections::BTreeMap,
    fmt,
    num::NonZeroU16,
    time::{Duration, SystemTime},
};

use aws_sdk_s3::{
    presigning::PresigningConfig,
    types::{CompletedMultipartUpload, CompletedPart},
};

use crate::{Bucket, Error};

const MIN_PART_SIZE: u64 = 5 * 1024 * 1024;
const MAX_PART_SIZE: u64 = 5 * 1024 * 1024 * 1024;
const MAX_PARTS: u16 = 10_000;
const MAX_KEY_BYTES: usize = 1_024;
const MAX_PRESIGN_SECONDS: u64 = 7 * 24 * 60 * 60;

/// A validated multipart part number in the range `1..=10_000`.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PartNumber(NonZeroU16);

impl PartNumber {
    /// Returns the numeric part number.
    #[must_use]
    pub const fn get(self) -> u16 {
        self.0.get()
    }
}

impl TryFrom<u16> for PartNumber {
    type Error = Error;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        let value = NonZeroU16::new(value).ok_or(Error::InvalidInput {
            field: "part_number",
            reason: "must be at least 1",
        })?;
        if value.get() > MAX_PARTS {
            return Err(Error::InvalidInput {
                field: "part_number",
                reason: "must not exceed 10,000",
            });
        }
        Ok(Self(value))
    }
}

/// A successfully uploaded multipart part.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UploadedPart {
    part_number: PartNumber,
    etag: String,
}

impl UploadedPart {
    /// Creates a completion entry from the exact ETag returned by R2.
    pub fn new(part_number: PartNumber, etag: impl Into<String>) -> Result<Self, Error> {
        let etag = etag.into();
        if etag.is_empty() {
            return Err(Error::InvalidInput {
                field: "etag",
                reason: "must not be empty",
            });
        }
        Ok(Self { part_number, etag })
    }

    /// Returns the part number.
    #[must_use]
    pub const fn part_number(&self) -> PartNumber {
        self.part_number
    }

    /// Returns the exact R2 ETag, including quotes when R2 supplied them.
    #[must_use]
    pub fn etag(&self) -> &str {
        &self.etag
    }
}

/// A canonical, duplicate-free completion manifest sorted by part number.
#[derive(Clone, Debug)]
pub struct CompletionManifest(Vec<UploadedPart>);

impl CompletionManifest {
    /// Validates and canonicalizes uploaded parts.
    pub fn try_from_parts(parts: impl IntoIterator<Item = UploadedPart>) -> Result<Self, Error> {
        let mut canonical = BTreeMap::new();
        for part in parts {
            let number = part.part_number();
            if canonical.insert(number, part).is_some() {
                return Err(Error::InvalidInput {
                    field: "parts",
                    reason: "contains a duplicate part number",
                });
            }
        }
        if canonical.is_empty() {
            return Err(Error::InvalidInput {
                field: "parts",
                reason: "must contain at least one uploaded part",
            });
        }
        Ok(Self(canonical.into_values().collect()))
    }
}

/// A presigned URL treated as a bearer credential.
#[derive(Clone)]
pub struct SecretUrl(String);

impl SecretUrl {
    /// Deliberately exposes the bearer URL for transmission to an uploader.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Deliberately consumes and exposes the bearer URL.
    #[must_use]
    pub fn into_exposed_string(self) -> String {
        self.0
    }
}

impl fmt::Debug for SecretUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretUrl([REDACTED PRESIGNED URL])")
    }
}

/// An HTTP request signed for temporary direct access to R2.
#[derive(Clone)]
pub struct PresignedRequest {
    method: String,
    url: SecretUrl,
    required_headers: Vec<(String, String)>,
    expires_at: SystemTime,
}

impl PresignedRequest {
    /// Returns the HTTP method the uploader must use.
    #[must_use]
    pub fn method(&self) -> &str {
        &self.method
    }

    /// Returns the redacted-by-default bearer URL wrapper.
    #[must_use]
    pub fn url(&self) -> &SecretUrl {
        &self.url
    }

    /// Returns headers that must be replayed exactly by the uploader.
    pub fn required_headers(&self) -> impl ExactSizeIterator<Item = (&str, &str)> {
        self.required_headers
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
    }

    /// Returns the approximate expiration instant.
    #[must_use]
    pub const fn expires_at(&self) -> SystemTime {
        self.expires_at
    }

    /// Deliberately exposes all request components for an HTTP client.
    #[must_use]
    pub fn into_exposed_parts(self) -> (String, String, Vec<(String, String)>) {
        (
            self.method,
            self.url.into_exposed_string(),
            self.required_headers,
        )
    }
}

impl fmt::Debug for PresignedRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let header_names: Vec<&str> = self
            .required_headers
            .iter()
            .map(|(name, _)| name.as_str())
            .collect();
        f.debug_struct("PresignedRequest")
            .field("method", &self.method)
            .field("url", &"[REDACTED PRESIGNED URL]")
            .field("required_header_names", &header_names)
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

/// A presigned upload request for one known multipart part.
#[derive(Clone, Debug)]
pub struct PresignedUploadPart {
    part_number: PartNumber,
    content_length: u64,
    request: PresignedRequest,
}

impl PresignedUploadPart {
    /// Returns the part number.
    #[must_use]
    pub const fn part_number(&self) -> PartNumber {
        self.part_number
    }

    /// Returns the exact number of bytes expected for this part.
    #[must_use]
    pub const fn content_length(&self) -> u64 {
        self.content_length
    }

    /// Returns the signed request.
    #[must_use]
    pub fn request(&self) -> &PresignedRequest {
        &self.request
    }

    /// Consumes the part wrapper and returns the signed request.
    #[must_use]
    pub fn into_request(self) -> PresignedRequest {
        self.request
    }
}

/// Persistable state needed to resume a presigned multipart upload.
#[derive(Clone)]
pub struct MultipartSessionSnapshot {
    bucket: String,
    key: String,
    upload_id: String,
    file_size: u64,
    part_size: u64,
}

impl MultipartSessionSnapshot {
    /// Restores validated multipart session state previously returned by [`PresignedMultipart::snapshot`].
    pub fn restore(
        bucket: impl Into<String>,
        key: impl Into<String>,
        upload_id: impl Into<String>,
        file_size: u64,
        part_size: u64,
    ) -> Result<Self, Error> {
        let key = key.into();
        validate_key(&key)?;
        MultipartPlan::new(file_size, part_size)?;
        let upload_id = upload_id.into();
        if upload_id.is_empty() {
            return Err(Error::InvalidInput {
                field: "upload_id",
                reason: "must not be empty",
            });
        }
        Ok(Self {
            bucket: bucket.into(),
            key,
            upload_id,
            file_size,
            part_size,
        })
    }

    /// Deliberately exposes the upload ID for persistence.
    #[must_use]
    pub fn expose_upload_id(&self) -> &str {
        &self.upload_id
    }
}

impl fmt::Debug for MultipartSessionSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MultipartSessionSnapshot")
            .field("bucket", &self.bucket)
            .field("key", &self.key)
            .field("upload_id", &"[REDACTED]")
            .field("file_size", &self.file_size)
            .field("part_size", &self.part_size)
            .finish()
    }
}

#[derive(Clone, Debug)]
struct MultipartPlan {
    file_size: u64,
    part_size: u64,
    part_count: u16,
}

impl MultipartPlan {
    fn new(file_size: u64, part_size: u64) -> Result<Self, Error> {
        if file_size == 0 {
            return Err(Error::InvalidInput {
                field: "file_size",
                reason: "must be greater than zero for multipart upload",
            });
        }
        if !(MIN_PART_SIZE..=MAX_PART_SIZE).contains(&part_size) {
            return Err(Error::InvalidInput {
                field: "part_size",
                reason: "must be between 5 MiB and 5 GiB",
            });
        }
        let part_count = file_size.div_ceil(part_size);
        if part_count > u64::from(MAX_PARTS) {
            return Err(Error::InvalidInput {
                field: "file_size",
                reason: "requires more than 10,000 parts",
            });
        }
        Ok(Self {
            file_size,
            part_size,
            part_count: part_count as u16,
        })
    }

    fn part_length(&self, number: PartNumber) -> Result<u64, Error> {
        if number.get() > self.part_count {
            return Err(Error::InvalidInput {
                field: "part_number",
                reason: "exceeds this upload's planned part count",
            });
        }
        let start = u64::from(number.get() - 1) * self.part_size;
        Ok((self.file_size - start).min(self.part_size))
    }
}

/// Builder for a new presigned multipart upload session.
#[derive(Clone, Debug)]
pub struct PresignedMultipartBuilder {
    bucket: Bucket,
    key: String,
    file_size: Option<u64>,
    part_size: Option<u64>,
}

impl PresignedMultipartBuilder {
    /// Sets the complete object size in bytes.
    #[must_use]
    pub const fn file_size(mut self, bytes: u64) -> Self {
        self.file_size = Some(bytes);
        self
    }

    /// Sets the uniform multipart part size in bytes.
    #[must_use]
    pub const fn part_size(mut self, bytes: u64) -> Self {
        self.part_size = Some(bytes);
        self
    }

    /// Creates the remote multipart upload after local validation succeeds.
    pub async fn create(self) -> Result<PresignedMultipart, Error> {
        let plan = MultipartPlan::new(
            self.file_size.ok_or(Error::InvalidInput {
                field: "file_size",
                reason: "is required",
            })?,
            self.part_size.ok_or(Error::InvalidInput {
                field: "part_size",
                reason: "is required",
            })?,
        )?;
        let output = self
            .bucket
            .client
            .as_sdk()
            .create_multipart_upload()
            .bucket(&self.bucket.name)
            .key(&self.key)
            .send()
            .await
            .map_err(|_| Error::Service {
                operation: "CreateMultipartUpload",
            })?;
        let upload_id = output.upload_id().ok_or(Error::Service {
            operation: "CreateMultipartUpload",
        })?;

        Ok(PresignedMultipart {
            bucket: self.bucket,
            key: self.key,
            upload_id: upload_id.to_owned(),
            plan,
        })
    }
}

/// An active R2 multipart upload that can presign parts, complete, or abort.
#[derive(Clone)]
pub struct PresignedMultipart {
    bucket: Bucket,
    key: String,
    upload_id: String,
    plan: MultipartPlan,
}

impl PresignedMultipart {
    /// Returns the number of planned parts.
    #[must_use]
    pub const fn part_count(&self) -> u16 {
        self.plan.part_count
    }

    /// Returns the expected byte length for a part.
    pub fn part_length(&self, number: PartNumber) -> Result<u64, Error> {
        self.plan.part_length(number)
    }

    /// Creates a temporary signed PUT request for one part.
    pub async fn presign_part(
        &self,
        number: PartNumber,
        expires_in: Duration,
    ) -> Result<PresignedUploadPart, Error> {
        let content_length = self.plan.part_length(number)?;
        validate_expiry(expires_in)?;
        let config = PresigningConfig::expires_in(expires_in).map_err(|_| Error::Presign)?;
        let signed = self
            .bucket
            .client
            .as_sdk()
            .upload_part()
            .bucket(&self.bucket.name)
            .key(&self.key)
            .upload_id(&self.upload_id)
            .part_number(i32::from(number.get()))
            .presigned(config)
            .await
            .map_err(|_| Error::Presign)?;

        let required_headers = signed
            .headers()
            .map(|(name, value)| (name.to_owned(), value.to_owned()))
            .collect();

        Ok(PresignedUploadPart {
            part_number: number,
            content_length,
            request: PresignedRequest {
                method: signed.method().to_owned(),
                url: SecretUrl(signed.uri().to_string()),
                required_headers,
                expires_at: SystemTime::now() + expires_in,
            },
        })
    }

    /// Completes the upload with the exact ETags returned by every uploaded part.
    pub async fn complete(&self, manifest: CompletionManifest) -> Result<CompletedObject, Error> {
        if manifest.0.len() != usize::from(self.plan.part_count)
            || manifest
                .0
                .iter()
                .enumerate()
                .any(|(index, part)| usize::from(part.part_number().get()) != index + 1)
        {
            return Err(Error::InvalidInput {
                field: "parts",
                reason: "must include every planned part exactly once",
            });
        }

        let parts = manifest
            .0
            .into_iter()
            .map(|part| {
                CompletedPart::builder()
                    .part_number(i32::from(part.part_number.get()))
                    .e_tag(part.etag)
                    .build()
            })
            .collect();
        let upload = CompletedMultipartUpload::builder()
            .set_parts(Some(parts))
            .build();
        let output = self
            .bucket
            .client
            .as_sdk()
            .complete_multipart_upload()
            .bucket(&self.bucket.name)
            .key(&self.key)
            .upload_id(&self.upload_id)
            .multipart_upload(upload)
            .send()
            .await
            .map_err(|_| Error::Service {
                operation: "CompleteMultipartUpload",
            })?;

        Ok(CompletedObject {
            etag: output.e_tag().map(ToOwned::to_owned),
        })
    }

    /// Aborts the remote multipart upload.
    pub async fn abort(&self) -> Result<(), Error> {
        self.bucket
            .client
            .as_sdk()
            .abort_multipart_upload()
            .bucket(&self.bucket.name)
            .key(&self.key)
            .upload_id(&self.upload_id)
            .send()
            .await
            .map_err(|_| Error::Service {
                operation: "AbortMultipartUpload",
            })?;
        Ok(())
    }

    /// Captures resumable session state. Its debug output redacts the upload ID.
    #[must_use]
    pub fn snapshot(&self) -> MultipartSessionSnapshot {
        MultipartSessionSnapshot {
            bucket: self.bucket.name.clone(),
            key: self.key.clone(),
            upload_id: self.upload_id.clone(),
            file_size: self.plan.file_size,
            part_size: self.plan.part_size,
        }
    }
}

impl fmt::Debug for PresignedMultipart {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PresignedMultipart")
            .field("bucket", &self.bucket.name)
            .field("key", &self.key)
            .field("upload_id", &"[REDACTED]")
            .field("plan", &self.plan)
            .finish()
    }
}

/// Result metadata for a completed multipart object.
#[derive(Clone, Debug)]
pub struct CompletedObject {
    etag: Option<String>,
}

impl CompletedObject {
    /// Returns R2's multipart ETag. It is not the MD5 of the complete object.
    #[must_use]
    pub fn etag(&self) -> Option<&str> {
        self.etag.as_deref()
    }
}

impl Bucket {
    /// Starts configuring a presigned multipart upload for an object key.
    pub fn presigned_multipart(
        &self,
        key: impl Into<String>,
    ) -> Result<PresignedMultipartBuilder, Error> {
        let key = key.into();
        validate_key(&key)?;
        Ok(PresignedMultipartBuilder {
            bucket: self.clone(),
            key,
            file_size: None,
            part_size: None,
        })
    }

    /// Restores a previously captured multipart upload session.
    pub fn resume_presigned_multipart(
        &self,
        snapshot: MultipartSessionSnapshot,
    ) -> Result<PresignedMultipart, Error> {
        if snapshot.bucket != self.name {
            return Err(Error::InvalidInput {
                field: "bucket",
                reason: "snapshot belongs to another bucket",
            });
        }
        let plan = MultipartPlan::new(snapshot.file_size, snapshot.part_size)?;
        Ok(PresignedMultipart {
            bucket: self.clone(),
            key: snapshot.key,
            upload_id: snapshot.upload_id,
            plan,
        })
    }
}

fn validate_key(key: &str) -> Result<(), Error> {
    if key.is_empty() || key.len() > MAX_KEY_BYTES {
        return Err(Error::InvalidInput {
            field: "key",
            reason: "must contain between 1 and 1,024 UTF-8 bytes",
        });
    }
    Ok(())
}

fn validate_expiry(expires_in: Duration) -> Result<(), Error> {
    if expires_in.is_zero() || expires_in.as_secs() > MAX_PRESIGN_SECONDS {
        return Err(Error::InvalidInput {
            field: "expires_in",
            reason: "must be between 1 second and 7 days",
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plans_exact_and_short_final_parts() {
        let plan = MultipartPlan::new(11 * 1024 * 1024, 5 * 1024 * 1024).unwrap();
        assert_eq!(plan.part_count, 3);
        assert_eq!(
            plan.part_length(PartNumber::try_from(1).unwrap()).unwrap(),
            5 * 1024 * 1024
        );
        assert_eq!(
            plan.part_length(PartNumber::try_from(3).unwrap()).unwrap(),
            1024 * 1024
        );
    }

    #[test]
    fn rejects_too_many_parts() {
        let result = MultipartPlan::new(10_001 * MIN_PART_SIZE, MIN_PART_SIZE);
        assert!(result.is_err());
    }

    #[test]
    fn canonicalizes_manifest_and_rejects_duplicates() {
        let one = PartNumber::try_from(1).unwrap();
        let two = PartNumber::try_from(2).unwrap();
        let manifest = CompletionManifest::try_from_parts([
            UploadedPart::new(two, "two").unwrap(),
            UploadedPart::new(one, "one").unwrap(),
        ])
        .unwrap();
        assert_eq!(manifest.0[0].part_number(), one);

        let duplicate = CompletionManifest::try_from_parts([
            UploadedPart::new(one, "one").unwrap(),
            UploadedPart::new(one, "again").unwrap(),
        ]);
        assert!(duplicate.is_err());
    }

    #[test]
    fn redacts_secret_wrappers() {
        let url = SecretUrl("https://example.invalid/?X-Amz-Signature=secret".into());
        assert!(!format!("{url:?}").contains("X-Amz-Signature"));
        let snapshot = MultipartSessionSnapshot::restore(
            "r2kit",
            "key",
            "secret-upload-id",
            MIN_PART_SIZE,
            MIN_PART_SIZE,
        )
        .unwrap();
        assert!(!format!("{snapshot:?}").contains("secret-upload-id"));
    }
}
