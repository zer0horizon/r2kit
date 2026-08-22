use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicU16, AtomicU64, Ordering},
    },
    time::Duration,
};

use aws_sdk_s3::primitives::ByteStream;
use futures_util::{StreamExt, TryStreamExt, stream};
use tokio::io::{AsyncReadExt, AsyncSeekExt};

use crate::{
    Bucket, CompletedObject, CompletionManifest, Error, MultipartSessionSnapshot, PartNumber,
    PresignedMultipart, UploadedPart, validation,
};

const DEFAULT_PART_SIZE: u64 = 8 * 1024 * 1024;
const DEFAULT_CONCURRENCY: usize = 4;
const DEFAULT_MAX_ATTEMPTS: u8 = 3;
const MAX_CONCURRENCY: usize = 64;
const MAX_ATTEMPTS: u8 = 10;

type ProgressCallback = Arc<dyn Fn(ManagedUploadProgress) + Send + Sync>;

/// A point-in-time progress update for a managed multipart upload.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ManagedUploadProgress {
    completed_parts: u16,
    total_parts: u16,
    transferred_bytes: u64,
    total_bytes: u64,
}

impl ManagedUploadProgress {
    /// Returns the number of uploaded or reused parts.
    #[must_use]
    pub const fn completed_parts(self) -> u16 {
        self.completed_parts
    }

    /// Returns the complete number of planned parts.
    #[must_use]
    pub const fn total_parts(self) -> u16 {
        self.total_parts
    }

    /// Returns bytes uploaded or verified from an existing session.
    #[must_use]
    pub const fn transferred_bytes(self) -> u64 {
        self.transferred_bytes
    }

    /// Returns the complete file size in bytes.
    #[must_use]
    pub const fn total_bytes(self) -> u64 {
        self.total_bytes
    }
}

/// Successful outcome of a managed multipart file upload.
#[derive(Clone, Debug)]
pub struct ManagedUploadResult {
    object: CompletedObject,
    file_size: u64,
    part_count: u16,
    uploaded_parts: u16,
    reused_parts: u16,
}

impl ManagedUploadResult {
    /// Returns metadata from completing the R2 multipart object.
    #[must_use]
    pub const fn object(&self) -> &CompletedObject {
        &self.object
    }

    /// Returns the uploaded file size in bytes.
    #[must_use]
    pub const fn file_size(&self) -> u64 {
        self.file_size
    }

    /// Returns the complete number of parts.
    #[must_use]
    pub const fn part_count(&self) -> u16 {
        self.part_count
    }

    /// Returns parts transferred during this invocation.
    #[must_use]
    pub const fn uploaded_parts(&self) -> u16 {
        self.uploaded_parts
    }

    /// Returns existing remote parts reused while resuming.
    #[must_use]
    pub const fn reused_parts(&self) -> u16 {
        self.reused_parts
    }
}

/// Failure from a managed upload, including recoverable session state.
pub struct ManagedUploadError {
    error: Error,
    snapshot: Option<Box<MultipartSessionSnapshot>>,
    aborted: bool,
}

impl ManagedUploadError {
    /// Returns the sanitized underlying failure.
    #[must_use]
    pub const fn error(&self) -> &Error {
        &self.error
    }

    /// Returns resumable state when the upload may still exist remotely.
    #[must_use]
    pub fn snapshot(&self) -> Option<&MultipartSessionSnapshot> {
        self.snapshot.as_deref()
    }

    /// Returns whether automatic abort completed successfully.
    #[must_use]
    pub const fn was_aborted(&self) -> bool {
        self.aborted
    }
}

impl fmt::Debug for ManagedUploadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ManagedUploadError")
            .field("error", &self.error)
            .field("snapshot", &self.snapshot)
            .field("aborted", &self.aborted)
            .finish()
    }
}

impl fmt::Display for ManagedUploadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.error.fmt(f)
    }
}

impl std::error::Error for ManagedUploadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}

/// Configures a bounded, retrying multipart upload from a local file.
#[derive(Clone)]
pub struct ManagedMultipartBuilder {
    bucket: Bucket,
    key: String,
    part_size: u64,
    concurrency: usize,
    max_attempts: u8,
    abort_on_error: bool,
    resume: Option<MultipartSessionSnapshot>,
    progress: Option<ProgressCallback>,
}

impl fmt::Debug for ManagedMultipartBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ManagedMultipartBuilder")
            .field("bucket", &self.bucket)
            .field("key", &self.key)
            .field("part_size", &self.part_size)
            .field("concurrency", &self.concurrency)
            .field("max_attempts", &self.max_attempts)
            .field("abort_on_error", &self.abort_on_error)
            .field("resume", &self.resume)
            .field("progress", &self.progress.as_ref().map(|_| "callback"))
            .finish()
    }
}

impl ManagedMultipartBuilder {
    /// Sets the uniform non-final part size. Resumed sessions must keep their original size.
    #[must_use]
    pub const fn part_size(mut self, bytes: u64) -> Self {
        self.part_size = bytes;
        self
    }

    /// Sets the maximum number of in-flight parts, from 1 through 64.
    #[must_use]
    pub const fn concurrency(mut self, value: usize) -> Self {
        self.concurrency = value;
        self
    }

    /// Sets total attempts per part, including the first request, from 1 through 10.
    #[must_use]
    pub const fn max_attempts(mut self, value: u8) -> Self {
        self.max_attempts = value;
        self
    }

    /// Chooses whether an error triggers a best-effort remote abort. The default is `true`.
    #[must_use]
    pub const fn abort_on_error(mut self, value: bool) -> Self {
        self.abort_on_error = value;
        self
    }

    /// Registers a fast, non-blocking callback invoked after progress changes.
    #[must_use]
    pub fn on_progress(
        mut self,
        callback: impl Fn(ManagedUploadProgress) + Send + Sync + 'static,
    ) -> Self {
        self.progress = Some(Arc::new(callback));
        self
    }

    /// Uploads the local file, reusing validated remote parts when resuming.
    pub async fn upload_file(
        self,
        path: impl AsRef<Path>,
    ) -> Result<ManagedUploadResult, ManagedUploadError> {
        self.validate().map_err(ManagedUploadError::before_start)?;
        let path = path.as_ref().to_owned();
        let metadata = tokio::fs::metadata(&path).await.map_err(|_| {
            ManagedUploadError::before_start(Error::Io {
                operation: "metadata",
            })
        })?;
        if !metadata.is_file() {
            return Err(ManagedUploadError::before_start(Error::InvalidInput {
                field: "path",
                reason: "must identify a regular file",
            }));
        }
        let file_size = metadata.len();

        let session = match self.resume.clone() {
            Some(snapshot) => {
                if snapshot.file_size() != file_size || snapshot.part_size() != self.part_size {
                    return Err(ManagedUploadError::before_start(Error::InvalidInput {
                        field: "path",
                        reason: "file size or part size differs from the saved session",
                    }));
                }
                self.bucket
                    .resume_presigned_multipart(snapshot)
                    .map_err(ManagedUploadError::before_start)?
            }
            None => self
                .bucket
                .presigned_multipart(&self.key)
                .map_err(ManagedUploadError::before_start)?
                .file_size(file_size)
                .part_size(self.part_size)
                .create()
                .await
                .map_err(ManagedUploadError::before_start)?,
        };

        match self.run(&path, &session).await {
            Ok(result) => Ok(result),
            Err(error) => Err(self.after_start_error(error, &session).await),
        }
    }

    fn validate(&self) -> Result<(), Error> {
        validation::validate_part_size(self.part_size)?;
        if self.concurrency == 0 || self.concurrency > MAX_CONCURRENCY {
            return Err(Error::InvalidInput {
                field: "concurrency",
                reason: "must be between 1 and 64",
            });
        }
        if self.max_attempts == 0 || self.max_attempts > MAX_ATTEMPTS {
            return Err(Error::InvalidInput {
                field: "max_attempts",
                reason: "must be between 1 and 10",
            });
        }
        Ok(())
    }

    async fn run(
        &self,
        path: &Path,
        session: &PresignedMultipart,
    ) -> Result<ManagedUploadResult, Error> {
        let existing = if self.resume.is_some() {
            list_uploaded_parts(session).await?
        } else {
            Vec::new()
        };
        let mut manifest = BTreeMap::new();
        let mut reused_bytes = 0_u64;
        for part in existing {
            let expected = session.part_length(part.part_number())?;
            reused_bytes = reused_bytes.checked_add(expected).ok_or(Error::Service {
                operation: "ListParts",
            })?;
            manifest.insert(part.part_number(), part);
        }

        let total_parts = session.part_count();
        let completed_parts = Arc::new(AtomicU16::new(manifest.len() as u16));
        let transferred_bytes = Arc::new(AtomicU64::new(reused_bytes));
        self.report_progress(
            completed_parts.load(Ordering::Relaxed),
            total_parts,
            transferred_bytes.load(Ordering::Relaxed),
            session.snapshot().file_size(),
        );

        let existing_numbers: BTreeSet<PartNumber> = manifest.keys().copied().collect();
        let missing: Vec<PartNumber> = (1..=total_parts)
            .map(PartNumber::try_from)
            .collect::<Result<Vec<_>, Error>>()?
            .into_iter()
            .filter(|part| !existing_numbers.contains(part))
            .collect();
        let snapshot = session.snapshot();
        let client = self.bucket.client.as_sdk().clone();
        let concurrency = self.concurrency;
        let max_attempts = self.max_attempts;
        let total_bytes = snapshot.file_size();
        let progress = self.progress.clone();
        let path = path.to_owned();
        let part_size = snapshot.part_size();
        let bucket = snapshot.bucket().to_owned();
        let key = snapshot.key().to_owned();
        let upload_id = snapshot.expose_upload_id().to_owned();

        let uploaded = stream::iter(missing.into_iter().map(|number| {
            let client = client.clone();
            let path = path.clone();
            let bucket = bucket.clone();
            let key = key.clone();
            let upload_id = upload_id.clone();
            let completed_parts = Arc::clone(&completed_parts);
            let transferred_bytes = Arc::clone(&transferred_bytes);
            let progress = progress.clone();
            async move {
                let length = planned_part_length(total_bytes, part_size, number)?;
                let bytes = read_part(&path, part_size, number, length).await?;
                let uploaded = upload_part_with_retry(
                    &client,
                    &bucket,
                    &key,
                    &upload_id,
                    number,
                    bytes,
                    max_attempts,
                )
                .await?;
                let parts = completed_parts.fetch_add(1, Ordering::Relaxed) + 1;
                let bytes = transferred_bytes.fetch_add(length, Ordering::Relaxed) + length;
                if let Some(callback) = progress.as_ref() {
                    callback(ManagedUploadProgress {
                        completed_parts: parts,
                        total_parts,
                        transferred_bytes: bytes,
                        total_bytes,
                    });
                }
                Ok::<UploadedPart, Error>(uploaded)
            }
        }))
        .buffer_unordered(concurrency)
        .try_collect::<Vec<_>>()
        .await?;

        let mut uploaded_count = 0_u16;
        for part in uploaded {
            uploaded_count += 1;
            manifest.insert(part.part_number(), part);
        }

        let final_metadata = tokio::fs::metadata(path).await.map_err(|_| Error::Io {
            operation: "metadata",
        })?;
        if final_metadata.len() != total_bytes {
            return Err(Error::InvalidInput {
                field: "path",
                reason: "file size changed during upload",
            });
        }

        let completion = CompletionManifest::try_from_parts(manifest.into_values())?;
        let object = session.complete(completion).await?;
        Ok(ManagedUploadResult {
            object,
            file_size: total_bytes,
            part_count: total_parts,
            uploaded_parts: uploaded_count,
            reused_parts: existing_numbers.len() as u16,
        })
    }

    fn report_progress(
        &self,
        completed_parts: u16,
        total_parts: u16,
        transferred_bytes: u64,
        total_bytes: u64,
    ) {
        if let Some(callback) = self.progress.as_ref() {
            callback(ManagedUploadProgress {
                completed_parts,
                total_parts,
                transferred_bytes,
                total_bytes,
            });
        }
    }

    async fn after_start_error(
        &self,
        error: Error,
        session: &PresignedMultipart,
    ) -> ManagedUploadError {
        let snapshot = session.snapshot();
        if self.abort_on_error && session.abort().await.is_ok() {
            ManagedUploadError {
                error,
                snapshot: None,
                aborted: true,
            }
        } else {
            ManagedUploadError {
                error,
                snapshot: Some(Box::new(snapshot)),
                aborted: false,
            }
        }
    }
}

impl ManagedUploadError {
    fn before_start(error: Error) -> Self {
        Self {
            error,
            snapshot: None,
            aborted: false,
        }
    }
}

impl Bucket {
    /// Starts a new managed multipart upload for a local file.
    pub fn managed_multipart(
        &self,
        key: impl Into<String>,
    ) -> Result<ManagedMultipartBuilder, Error> {
        let key = key.into();
        validation::validate_key(&key)?;
        Ok(ManagedMultipartBuilder {
            bucket: self.clone(),
            key,
            part_size: DEFAULT_PART_SIZE,
            concurrency: DEFAULT_CONCURRENCY,
            max_attempts: DEFAULT_MAX_ATTEMPTS,
            abort_on_error: true,
            resume: None,
            progress: None,
        })
    }

    /// Resumes a managed upload from a saved multipart session snapshot.
    pub fn resume_managed_multipart(
        &self,
        snapshot: MultipartSessionSnapshot,
    ) -> Result<ManagedMultipartBuilder, Error> {
        self.resume_presigned_multipart(snapshot.clone())?;
        Ok(ManagedMultipartBuilder {
            bucket: self.clone(),
            key: snapshot.key().to_owned(),
            part_size: snapshot.part_size(),
            concurrency: DEFAULT_CONCURRENCY,
            max_attempts: DEFAULT_MAX_ATTEMPTS,
            abort_on_error: true,
            resume: Some(snapshot),
            progress: None,
        })
    }
}

async fn read_part(
    path: &Path,
    part_size: u64,
    number: PartNumber,
    length: u64,
) -> Result<Vec<u8>, Error> {
    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|_| Error::Io { operation: "open" })?;
    let offset = u64::from(number.get() - 1)
        .checked_mul(part_size)
        .ok_or(Error::InvalidInput {
            field: "part_number",
            reason: "part offset overflowed",
        })?;
    file.seek(std::io::SeekFrom::Start(offset))
        .await
        .map_err(|_| Error::Io { operation: "seek" })?;
    let capacity = usize::try_from(length).map_err(|_| Error::InvalidInput {
        field: "part_size",
        reason: "does not fit this platform's address space",
    })?;
    let mut bytes = vec![0; capacity];
    file.read_exact(&mut bytes)
        .await
        .map_err(|_| Error::Io { operation: "read" })?;
    Ok(bytes)
}

async fn upload_part_with_retry(
    client: &aws_sdk_s3::Client,
    bucket: &str,
    key: &str,
    upload_id: &str,
    number: PartNumber,
    bytes: Vec<u8>,
    max_attempts: u8,
) -> Result<UploadedPart, Error> {
    let bytes = bytes::Bytes::from(bytes);
    for attempt in 1..=max_attempts {
        let result = client
            .upload_part()
            .bucket(bucket)
            .key(key)
            .upload_id(upload_id)
            .part_number(i32::from(number.get()))
            .content_length(bytes.len() as i64)
            .body(ByteStream::from(bytes.clone()))
            .send()
            .await;
        match result {
            Ok(output) => {
                let etag = output.e_tag().ok_or(Error::Service {
                    operation: "UploadPart",
                })?;
                return UploadedPart::new(number, etag);
            }
            Err(error) if attempt < max_attempts && is_retryable(&error) => {
                tokio::time::sleep(retry_delay(attempt, number)).await;
            }
            Err(_) => {
                return Err(Error::Service {
                    operation: "UploadPart",
                });
            }
        }
    }
    Err(Error::Service {
        operation: "UploadPart",
    })
}

fn is_retryable<E>(error: &aws_sdk_s3::error::SdkError<E>) -> bool {
    match error.raw_response() {
        Some(response) => {
            let status = response.status().as_u16();
            status == 408 || status == 429 || status >= 500
        }
        None => true,
    }
}

fn retry_delay(attempt: u8, number: PartNumber) -> Duration {
    let exponent = u32::from(attempt.saturating_sub(1).min(4));
    let base = 250_u64 * (1_u64 << exponent);
    let jitter = u64::from(number.get() % 17) * 7;
    Duration::from_millis(base + jitter)
}

async fn list_uploaded_parts(session: &PresignedMultipart) -> Result<Vec<UploadedPart>, Error> {
    Ok(session
        .reconcile()
        .await?
        .uploaded_parts()
        .cloned()
        .collect())
}

fn planned_part_length(file_size: u64, part_size: u64, number: PartNumber) -> Result<u64, Error> {
    let offset = u64::from(number.get() - 1)
        .checked_mul(part_size)
        .ok_or(Error::InvalidInput {
            field: "part_number",
            reason: "part offset overflowed",
        })?;
    Ok((file_size - offset).min(part_size))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_delay_is_bounded_and_part_specific() {
        let first = retry_delay(1, PartNumber::try_from(1).unwrap());
        let second = retry_delay(1, PartNumber::try_from(2).unwrap());
        assert!(first >= Duration::from_millis(250));
        assert!(second > first);
        assert!(retry_delay(10, PartNumber::try_from(1).unwrap()) < Duration::from_secs(5));
    }
}
