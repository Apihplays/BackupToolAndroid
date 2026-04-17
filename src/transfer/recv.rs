use std::path::Path;

use crate::adb::client::AdbClient;
use crate::error::{AppError, AppResult};

/// Pulls individual files using ADB pull command.
/// This is the fallback when tar streaming isn't available or when
/// pulling individually selected files.
pub struct RecvPuller;

impl RecvPuller {
    /// Pull a single file from device to local path.
    /// Returns the number of bytes transferred.
    pub fn pull_file(client: &AdbClient, remote_path: &str, local_path: &str) -> AppResult<u64> {
        // Ensure parent directory exists
        if let Some(parent) = Path::new(local_path).parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Use adb pull
        client.pull_file(remote_path, local_path)?;

        // Get the size of the pulled file
        let metadata = std::fs::metadata(local_path).map_err(|e| AppError::Transfer {
            path: remote_path.to_string(),
            reason: format!("Could not read pulled file metadata: {}", e),
        })?;

        Ok(metadata.len())
    }

    /// Pull a file only if it doesn't already exist locally with matching size.
    pub fn pull_file_if_needed(
        client: &AdbClient,
        remote_path: &str,
        local_path: &str,
        expected_size: u64,
    ) -> AppResult<Option<u64>> {
        // Check if local file already exists with matching size
        if let Ok(metadata) = std::fs::metadata(local_path) {
            if metadata.len() == expected_size {
                return Ok(None); // Already exists, skip
            }
        }

        let bytes = Self::pull_file(client, remote_path, local_path)?;
        Ok(Some(bytes))
    }
}
