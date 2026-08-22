//! Ergonomic configuration for Cloudflare R2 clients.
//!
//! R2 exposes an S3-compatible API. This crate keeps its R2-specific setup in
//! one place: the account endpoint, the required `auto` region, and credentials.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod config;
mod error;

pub use config::{R2Config, R2ConfigBuilder};
pub use error::ConfigError;
