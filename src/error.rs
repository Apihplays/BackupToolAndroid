#![allow(dead_code)]

use thiserror::Error;

/// All error types for andpull, categorized by source.
#[derive(Error, Debug)]
pub enum AppError {
    #[error("ADB connection failed: {0}")]
    Connection(String),

    #[error("No ADB devices found. Is USB debugging enabled?")]
    NoDevice,

    #[error("Multiple devices found. Please select one.")]
    MultipleDevices,

    #[error("Permission denied: {path}")]
    Permission { path: String },

    #[error("File transfer failed: {path} — {reason}")]
    Transfer { path: String, reason: String },

    #[error("File not found on device: {path}")]
    NotFound { path: String },

    #[error("Disk full: needed {needed} bytes, only {available} available")]
    DiskFull { needed: u64, available: u64 },

    #[error("ADB protocol error: {0}")]
    Protocol(String),

    #[error("State file corrupted: {0}")]
    StateCorrupted(String),

    #[error("Checksum mismatch: {path} — expected {expected}, got {actual}")]
    ChecksumMismatch {
        path: String,
        expected: String,
        actual: String,
    },

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Unexpected error: {0}")]
    Other(#[from] anyhow::Error),
}

/// Result type alias using AppError.
pub type AppResult<T> = Result<T, AppError>;

/// Returns `true` when the error indicates a permission denial that might
/// be bypassed with root access on the device.
pub fn is_permission_denied(e: &AppError) -> bool {
    match e {
        AppError::Permission { .. } => true,
        AppError::Transfer { reason, .. } => {
            reason.contains("Permission denied") || reason.contains("Operation not permitted")
        }
        AppError::Io(io) => io.kind() == std::io::ErrorKind::PermissionDenied,
        _ => false,
    }
}
