#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod client;
mod config;
mod error;
mod managed;
mod multipart;
mod object;
mod observability;
mod validation;

pub use client::{Bucket, R2Client};
pub use config::{R2Config, R2ConfigBuilder, R2Jurisdiction};
pub use error::{ConfigError, Error, ServiceError, ServiceErrorKind, ValidationError};
pub use headers::CacheControl;
pub use managed::{
    ManagedMultipartBuilder, ManagedUploadCancellation, ManagedUploadError, ManagedUploadProgress,
    ManagedUploadResult,
};
pub use mime::{self, Mime};
pub use multipart::{
    CompletedObject, CompletionManifest, MultipartPartReceipt, MultipartReconciliation,
    MultipartSessionRecord, MultipartSessionSnapshot, MultipartUploadPartRequest, PartMd5,
    PartNumber, PresignedMultipart, PresignedMultipartBuilder, PresignedRequest,
    PresignedUploadPart, SecretUrl, UploadedPart,
};
pub use object::{
    BatchDeleteError, DeleteObjectFailure, DeleteObjectsResult, DownloadedObject,
    ListObjectsBuilder, ObjectMetadata, ObjectPage, ObjectSummary, ObjectUploadOptions,
    ObjectUploadOptionsBuilder, PresignedPutObject, PutObjectResult,
};
