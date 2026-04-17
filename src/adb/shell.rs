use crate::adb::client::AdbClient;
use crate::error::{AppError, AppResult};

/// Executes shell commands on the device for specialized operations.
pub struct ShellExecutor;

impl ShellExecutor {
    /// Check if tar is available on the device.
    pub fn has_tar(client: &AdbClient) -> bool {
        client
            .shell_command("which tar 2>/dev/null")
            .map(|out| !out.trim().is_empty())
            .unwrap_or(false)
    }

    /// Get total size of a directory on the device.
    pub fn dir_size(client: &AdbClient, path: &str) -> AppResult<u64> {
        let cmd = format!("du -sb '{}' 2>/dev/null | cut -f1", path);
        let output = client.shell_command(&cmd)?;
        output
            .trim()
            .parse::<u64>()
            .map_err(|_| AppError::Protocol(format!("Could not parse dir size for {}", path)))
    }

    /// Count files in a directory recursively.
    pub fn file_count(client: &AdbClient, path: &str) -> AppResult<u64> {
        let cmd = format!("find '{}' -type f 2>/dev/null | wc -l", path);
        let output = client.shell_command(&cmd)?;
        output
            .trim()
            .parse::<u64>()
            .map_err(|_| AppError::Protocol("Could not parse file count".into()))
    }

    /// Get available space on the device storage.
    pub fn available_space(client: &AdbClient, path: &str) -> AppResult<u64> {
        let cmd = format!("df '{}' 2>/dev/null | tail -1 | awk '{{print $4}}'", path);
        let output = client.shell_command(&cmd)?;
        let raw = output.trim();

        // df output could be in KB
        raw.parse::<u64>()
            .map(|kb| kb * 1024)
            .map_err(|_| AppError::Protocol("Could not parse available space".into()))
    }

    /// List common media directories on the device.
    pub fn media_directories(client: &AdbClient) -> Vec<String> {
        let common = vec![
            "/sdcard/DCIM",
            "/sdcard/Pictures",
            "/sdcard/Movies",
            "/sdcard/Download",
            "/sdcard/Music",
            "/sdcard/Recordings",
            "/sdcard/Screenshots",
            "/sdcard/WhatsApp/Media",
            "/sdcard/Telegram",
        ];

        common
            .into_iter()
            .filter(|path| {
                client
                    .shell_command(&format!("[ -d '{}' ] && echo yes", path))
                    .map(|out| out.trim() == "yes")
                    .unwrap_or(false)
            })
            .map(|s| s.to_string())
            .collect()
    }
}
