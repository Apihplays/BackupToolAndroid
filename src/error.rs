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

/// Minimum headroom to reserve on the destination filesystem (500 MiB).
const DISK_HEADROOM: u64 = 500 * 1024 * 1024;

/// Check that the destination filesystem has enough free space for the
/// estimated transfer. Returns `Err(DiskFull)` if available space is
/// less than `required + DISK_HEADROOM`.
pub fn check_disk_space(destination: &str, required_bytes: u64) -> AppResult<()> {
    use fs2::available_space;
    use std::path::Path;

    // Walk up until we find an existing directory (the dest may not exist yet).
    let dir = {
        let mut p = Path::new(destination);
        while !p.is_dir() {
            match p.parent() {
                Some(parent) => p = parent,
                None => p = Path::new("/"),
            }
        }
        p
    };

    let avail = available_space(dir).map_err(AppError::Io)?;
    let needed = required_bytes.saturating_add(DISK_HEADROOM);

    if avail < needed {
        return Err(AppError::DiskFull {
            needed,
            available: avail,
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn disk_space_check_passes_for_small_transfer() {
        let dir = tempdir().unwrap();
        let dest = dir.path().to_str().unwrap();
        // Asking for 1 byte — any real filesystem has more than 500MiB + 1B free.
        assert!(check_disk_space(dest, 1).is_ok());
    }

    #[test]
    fn disk_space_check_fails_for_impossible_size() {
        let dir = tempdir().unwrap();
        let dest = dir.path().to_str().unwrap();
        // u64::MAX is clearly larger than any filesystem.
        assert!(check_disk_space(dest, u64::MAX).is_err());
    }

    #[test]
    fn disk_space_check_handles_nonexistent_dest() {
        // Walks up to an existing parent.
        assert!(check_disk_space("/tmp/__nonexistent_andpull_test__", 1).is_ok());
    }
}
