use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    future::Future,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicU16, AtomicU64, Ordering},
    },
    time::Duration,
};

use aws_sdk_s3::primitives::ByteStream;
use aws_smithy_types::retry::RetryConfig;
use futures_util::{
    StreamExt, TryStreamExt,
    future::{Either, select},
    stream,
};
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio::sync::watch;

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

/// A cloneable signal for explicitly cancelling a managed multipart upload.
///
/// Cancellation is cooperative: pass a clone to the builder, call [`Self::cancel`],
/// and continue awaiting `upload_file` so it can abort or return resumable state.
#[derive(Clone, Debug)]
pub struct ManagedUploadCancellation {
    signal: watch::Sender<bool>,
}

impl ManagedUploadCancellation {
    /// Creates a cancellation signal in the active state.
    #[must_use]
    pub fn new() -> Self {
        let (signal, _) = watch::channel(false);
        Self { signal }
    }

    /// Requests cancellation. Calling this method more than once is harmless.
    pub fn cancel(&self) {
        self.signal.send_replace(true);
    }

    /// Returns whether cancellation has been requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        *self.signal.borrow()
    }

    async fn cancelled(&self) {
        let mut receiver = self.signal.subscribe();
        if *receiver.borrow_and_update() {
            return;
        }
        while receiver.changed().await.is_ok() {
            if *receiver.borrow_and_update() {
                return;
            }
        }
    }
}

impl Default for ManagedUploadCancellation {
    fn default() -> Self {
        Self::new()
    }
}

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
    cancellation: Option<ManagedUploadCancellation>,
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
            .field("cancellation", &self.cancellation.is_some())
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

    /// Registers an explicit cooperative cancellation signal.
    ///
    /// With the default `abort_on_error(true)`, cancellation waits for a
    /// best-effort remote abort before returning [`Error::Cancelled`]. If abort
    /// fails, the returned error carries a snapshot for later cleanup or resume.
    #[must_use]
    pub fn cancellation_token(mut self, cancellation: ManagedUploadCancellation) -> Self {
        self.cancellation = Some(cancellation);
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
        if self
            .cancellation
            .as_ref()
            .is_some_and(ManagedUploadCancellation::is_cancelled)
        {
            return Err(ManagedUploadError::before_start(Error::Cancelled));
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

        let run = self.run(&path, &session);
        let result = match self.cancellation.as_ref() {
            Some(cancellation) => {
                let run = std::pin::pin!(run);
                let cancelled = std::pin::pin!(cancellation.cancelled());

                match select(run, cancelled).await {
                    Either::Left((result, _)) => result,
                    Either::Right(((), _)) => Err(Error::Cancelled),
                }
            }
            None => run.await,
        };

        match result {
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
            cancellation: None,
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
            cancellation: None,
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
    let output = retry_with_policy(
        max_attempts,
        number,
        || {
            client
                .upload_part()
                .bucket(bucket)
                .key(key)
                .upload_id(upload_id)
                .part_number(i32::from(number.get()))
                .content_length(bytes.len() as i64)
                .body(ByteStream::from(bytes.clone()))
                .customize()
                .config_override(
                    aws_sdk_s3::Config::builder().retry_config(RetryConfig::disabled()),
                )
                .send()
        },
        is_retryable,
        tokio::time::sleep,
    )
    .await
    .map_err(|_| Error::Service {
        operation: "UploadPart",
    })?;
    let etag = output.e_tag().ok_or(Error::Service {
        operation: "UploadPart",
    })?;
    UploadedPart::new(number, etag)
}

async fn retry_with_policy<T, E, Operation, OperationFuture, Classify, Sleep, SleepFuture>(
    max_attempts: u8,
    number: PartNumber,
    mut operation: Operation,
    classify: Classify,
    mut sleep: Sleep,
) -> Result<T, E>
where
    Operation: FnMut() -> OperationFuture,
    OperationFuture: Future<Output = Result<T, E>>,
    Classify: Fn(&E) -> bool,
    Sleep: FnMut(Duration) -> SleepFuture,
    SleepFuture: Future<Output = ()>,
{
    for attempt in 1..=max_attempts {
        match operation().await {
            Ok(value) => return Ok(value),
            Err(error) if attempt < max_attempts && classify(&error) => {
                sleep(retry_delay(attempt, number)).await;
            }
            Err(error) => return Err(error),
        }
    }
    unreachable!("validated max_attempts is at least one")
}

fn is_retryable<E>(error: &aws_sdk_s3::error::SdkError<E>) -> bool {
    use aws_sdk_s3::error::SdkError;

    match error {
        SdkError::ConstructionFailure(_) => false,
        SdkError::TimeoutError(_) | SdkError::DispatchFailure(_) | SdkError::ResponseError(_) => {
            true
        }
        SdkError::ServiceError(_) => error
            .raw_response()
            .is_some_and(|response| is_retryable_status(response.status().as_u16())),
        _ => false,
    }
}

fn is_retryable_status(status: u16) -> bool {
    status == 408 || status == 429 || (500..=599).contains(&status)
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
    use std::{collections::VecDeque, future::ready};

    use super::*;

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum InjectedFailure {
        NoResponse,
        Status(u16),
    }

    fn injected_retryable(error: &InjectedFailure) -> bool {
        match error {
            InjectedFailure::NoResponse => true,
            InjectedFailure::Status(status) => is_retryable_status(*status),
        }
    }

    fn run_async(future: impl Future<Output = ()>) {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(future);
    }

    #[test]
    fn retry_delay_is_bounded_and_part_specific() {
        let first = retry_delay(1, PartNumber::try_from(1).unwrap());
        let second = retry_delay(1, PartNumber::try_from(2).unwrap());
        assert!(first >= Duration::from_millis(250));
        assert!(second > first);
        assert!(retry_delay(10, PartNumber::try_from(1).unwrap()) < Duration::from_secs(5));
    }

    #[test]
    fn retry_status_matrix_is_deliberately_bounded() {
        for retryable in [408, 429, 500, 502, 503, 599] {
            assert!(injected_retryable(&InjectedFailure::Status(retryable)));
        }
        for terminal in [400, 401, 403, 404, 409, 499, 600] {
            assert!(!injected_retryable(&InjectedFailure::Status(terminal)));
        }
        assert!(injected_retryable(&InjectedFailure::NoResponse));
    }

    #[test]
    fn sdk_construction_failures_are_terminal_but_timeouts_retry() {
        let construction: aws_sdk_s3::error::SdkError<std::io::Error> =
            aws_sdk_s3::error::SdkError::construction_failure(std::io::Error::other("injected"));
        let timeout: aws_sdk_s3::error::SdkError<std::io::Error> =
            aws_sdk_s3::error::SdkError::timeout_error(std::io::Error::other("injected"));

        assert!(!is_retryable(&construction));
        assert!(is_retryable(&timeout));
    }

    #[test]
    fn retry_runner_reaches_success_after_transient_faults() {
        run_async(async {
            let mut outcomes = VecDeque::from([
                Err(InjectedFailure::Status(408)),
                Err(InjectedFailure::Status(429)),
                Err(InjectedFailure::Status(503)),
                Ok("etag"),
            ]);
            let mut attempts = 0_u8;
            let mut delays = Vec::new();
            let result = retry_with_policy(
                4,
                PartNumber::try_from(3).unwrap(),
                || {
                    attempts += 1;
                    ready(outcomes.pop_front().unwrap())
                },
                injected_retryable,
                |delay| {
                    delays.push(delay);
                    ready(())
                },
            )
            .await;

            assert_eq!(result, Ok("etag"));
            assert_eq!(attempts, 4);
            assert_eq!(delays.len(), 3);
            assert!(delays.windows(2).all(|pair| pair[0] < pair[1]));
        });
    }

    #[test]
    fn retry_runner_stops_on_terminal_fault() {
        run_async(async {
            let mut attempts = 0_u8;
            let result = retry_with_policy(
                10,
                PartNumber::try_from(1).unwrap(),
                || {
                    attempts += 1;
                    ready(Err::<(), _>(InjectedFailure::Status(403)))
                },
                injected_retryable,
                |_| ready(()),
            )
            .await;

            assert_eq!(result, Err(InjectedFailure::Status(403)));
            assert_eq!(attempts, 1);
        });
    }

    #[test]
    fn retry_runner_honors_the_exact_attempt_limit() {
        run_async(async {
            let mut attempts = 0_u8;
            let mut delays = 0_u8;
            let result = retry_with_policy(
                3,
                PartNumber::try_from(1).unwrap(),
                || {
                    attempts += 1;
                    ready(Err::<(), _>(InjectedFailure::NoResponse))
                },
                injected_retryable,
                |_| {
                    delays += 1;
                    ready(())
                },
            )
            .await;

            assert_eq!(result, Err(InjectedFailure::NoResponse));
            assert_eq!(attempts, 3);
            assert_eq!(delays, 2);
        });
    }

    #[test]
    fn cancellation_wakes_all_waiters_and_is_idempotent() {
        run_async(async {
            let cancellation = ManagedUploadCancellation::new();
            let first = cancellation.clone();
            let second = cancellation.clone();
            let first_waiter = tokio::spawn(async move { first.cancelled().await });
            let second_waiter = tokio::spawn(async move { second.cancelled().await });

            assert!(!cancellation.is_cancelled());
            cancellation.cancel();
            cancellation.cancel();
            first_waiter.await.unwrap();
            second_waiter.await.unwrap();
            assert!(cancellation.is_cancelled());
            cancellation.cancelled().await;
        });
    }
}
