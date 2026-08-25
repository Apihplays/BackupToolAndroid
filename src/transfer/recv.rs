use std::io::Read;
use std::path::Path;

use crate::adb::client::{quote_shell, AdbClient};
use crate::error::{is_permission_denied, AppError, AppResult};

/// Pulls individual files using ADB pull command.
/// This is the fallback when tar streaming isn't available or when
/// pulling individually selected files.
pub struct RecvPuller;

impl RecvPuller {
    /// Pull a single file from device to local path.
    /// Returns the number of bytes transferred.
    ///
    /// Tries a normal `adb pull` first; when that fails with a permission
    /// error and root (su) is available on the device, falls back to a
    /// rooted `su -c cat` stream so that protected Android 16 paths
    /// (e.g. `/sdcard/Android/media/com.whatsapp`) are still accessible.
    pub fn pull_file(client: &AdbClient, remote_path: &str, local_path: &str) -> AppResult<u64> {
        // Ensure parent directory exists
        if let Some(parent) = Path::new(local_path).parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Try normal adb pull first
        match client.pull_file(remote_path, local_path) {
            Ok(()) => {
                let metadata = std::fs::metadata(local_path).map_err(|e| {
                    AppError::Transfer {
                        path: remote_path.to_string(),
                        reason: format!("Could not read pulled file metadata: {}", e),
                    }
                })?;
                Ok(metadata.len())
            }
            Err(e) if is_permission_denied(&e) && client.su_available() => {
                // Fall back to rooted cat streaming on permission failure
                Self::pull_rooted(client, remote_path, Path::new(local_path))?;
                let metadata = std::fs::metadata(local_path).map_err(|e| {
                    AppError::Transfer {
                        path: remote_path.to_string(),
                        reason: format!("Could not read pulled file metadata: {}", e),
                    }
                })?;
                Ok(metadata.len())
            }
            Err(e) => Err(e),
        }
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

    /// Pull a single file via rooted `cat` streaming over exec-out.
    /// Falls back to this when adb pull lacks permission. Verifies the
    /// transferred size against a rooted `wc -c` on the remote file.
    pub fn pull_rooted(client: &AdbClient, remote_path: &str, local_path: &Path) -> AppResult<()> {
        // Ensure parent directory exists
        if let Some(parent) = local_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Use su to bypass Android 16 scoped-storage restrictions
        let cmd = format!("su -c 'cat {}'", quote_shell(remote_path));
        let mut child = client.shell_stream(&cmd)?;

        let mut file = std::fs::File::create(local_path)?;
        let mut stdout = child.stdout.take().ok_or_else(|| AppError::Transfer {
            path: remote_path.to_string(),
            reason: "No stdout from adb exec-out".to_string(),
        })?;

        let mut buf = [0u8; 64 * 1024];
        loop {
            let n = stdout.read(&mut buf)?;
            if n == 0 {
                break;
            }
            std::io::copy(&mut &buf[..n], &mut file)?;
        }
        drop(file);

        let status = child.wait()?;
        if !status.success() {
            let _ = std::fs::remove_file(local_path);
            return Err(AppError::Transfer {
                path: remote_path.to_string(),
                reason: format!("su cat exited with {}", status),
            });
        }

        // Verify size against the remote file
        let expected = client.remote_file_size_rooted(remote_path)?;
        let actual = std::fs::metadata(local_path).map(|m| m.len()).unwrap_or(0);
        if expected != actual {
            let _ = std::fs::remove_file(local_path);
            return Err(AppError::Transfer {
                path: remote_path.to_string(),
                reason: format!("size mismatch: expected {}, got {}", expected, actual),
            });
        }

        Ok(())
    }
}
