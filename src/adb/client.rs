use std::process::{Command, Stdio};

use crate::error::{AppError, AppResult};

/// Represents a connected Android device.
#[derive(Debug, Clone)]
pub struct DeviceInfo {
    pub serial: String,
    pub state: String,
    pub model: String,
    pub transport: String, // "usb" or "wifi"
}

impl std::fmt::Display for DeviceInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let icon = if self.transport == "wifi" { "📶" } else { "🔌" };
        write!(f, "{} {} [{}] ({})", icon, self.model, self.serial, self.transport)
    }
}

/// ADB client that communicates with the ADB server.
pub struct AdbClient {
    pub selected_device: Option<DeviceInfo>,
}

impl AdbClient {
    pub fn new() -> Self {
        Self {
            selected_device: None,
        }
    }

    /// List all connected devices.
    pub fn list_devices(&self) -> AppResult<Vec<DeviceInfo>> {
        let output = Command::new("adb")
            .args(["devices", "-l"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .map_err(|e| AppError::Connection(format!("Failed to run adb: {}", e)))?;

        if !output.status.success() {
            return Err(AppError::Connection("ADB server not responding".into()));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut devices = Vec::new();

        for line in stdout.lines().skip(1) {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                let serial = parts[0].to_string();
                let state = parts[1].to_string();

                // Keep all devices regardless of state so the user is aware they are connected
                // Extract model from key-value pairs
                let model = parts.iter()
                    .find(|p| p.starts_with("model:"))
                    .map(|p| p.trim_start_matches("model:").to_string())
                    .unwrap_or_else(|| "Unknown".into());

                // Determine transport type
                let transport = if serial.contains(':') {
                    "wifi".to_string()
                } else {
                    "usb".to_string()
                };

                devices.push(DeviceInfo {
                    serial,
                    state,
                    model,
                    transport,
                });
            }
        }

        Ok(devices)
    }

    /// Select a specific device for operations.
    pub fn select_device(&mut self, device: DeviceInfo) {
        self.selected_device = Some(device);
    }

    /// Get the serial of the currently selected device.
    pub fn device_serial(&self) -> AppResult<&str> {
        self.selected_device
            .as_ref()
            .map(|d| d.serial.as_str())
            .ok_or(AppError::NoDevice)
    }

    /// Run an ADB shell command and return stdout as string.
    pub fn shell_command(&self, cmd: &str) -> AppResult<String> {
        let serial = self.device_serial()?;
        let output = Command::new("adb")
            .args(["-s", serial, "shell", cmd])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .map_err(|e| AppError::Connection(format!("Shell command failed: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(AppError::Protocol(format!("Shell error: {}", stderr)));
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    /// Run ADB shell and return a streaming reader for large output.
    pub fn shell_stream(&self, cmd: &str) -> AppResult<std::process::Child> {
        let serial = self.device_serial()?;
        let child = Command::new("adb")
            .args(["-s", serial, "exec-out", cmd])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| AppError::Connection(format!("Failed to spawn adb: {}", e)))?;

        Ok(child)
    }

    /// Pull a single file from device to local path using adb pull.
    pub fn pull_file(&self, remote_path: &str, local_path: &str) -> AppResult<()> {
        let serial = self.device_serial()?;
        let output = Command::new("adb")
            .args(["-s", serial, "pull", remote_path, local_path])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .map_err(|e| AppError::Transfer {
                path: remote_path.to_string(),
                reason: format!("Pull command failed: {}", e),
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("Permission denied") {
                return Err(AppError::Permission {
                    path: remote_path.to_string(),
                });
            }
            if stderr.contains("does not exist") || stderr.contains("No such file") {
                return Err(AppError::NotFound {
                    path: remote_path.to_string(),
                });
            }
            return Err(AppError::Transfer {
                path: remote_path.to_string(),
                reason: stderr.to_string(),
            });
        }

        Ok(())
    }

    /// Push a single file from local path to device using adb push.
    pub fn push_file(&self, local_path: &str, remote_path: &str) -> AppResult<()> {
        let serial = self.device_serial()?;
        let output = Command::new("adb")
            .args(["-s", serial, "push", local_path, remote_path])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .map_err(|e| AppError::Transfer {
                path: local_path.to_string(),
                reason: format!("Push command failed: {}", e),
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("Permission denied") {
                return Err(AppError::Permission {
                    path: remote_path.to_string(),
                });
            }
            return Err(AppError::Transfer {
                path: local_path.to_string(),
                reason: stderr.to_string(),
            });
        }

        Ok(())
    }

    /// Delete a file or directory on the selected device.
    pub fn rm_remote(&self, remote_path: &str) -> AppResult<()> {
        // Use rm -rf for robust deletion
        let cmd = format!("rm -rf '{}'", remote_path);
        let _ = self.shell_command(&cmd)?;
        Ok(())
    }

    /// Pull a directory using tar streaming (much faster for bulk).
    /// Returns a child process whose stdout streams the tar data.
    pub fn pull_dir_tar_stream(&self, remote_dir: &str) -> AppResult<std::process::Child> {
        let cmd = format!("cd '{}' && tar cf - . 2>/dev/null", remote_dir);
        self.shell_stream(&cmd)
    }

    /// List files in a remote directory.
    pub fn list_dir(&self, remote_path: &str) -> AppResult<Vec<RemoteEntry>> {
        // Use ls -la for detailed listing
        // Append a trailing slash so that symlinks like /sdcard list their contents instead of the link itself
        let path_with_slash = if remote_path.ends_with('/') {
            remote_path.to_string()
        } else {
            format!("{}/", remote_path)
        };
        let cmd = format!(
            "ls -la '{}' 2>/dev/null | tail -n +2",
            path_with_slash
        );
        let output = self.shell_command(&cmd)?;
        let mut entries = Vec::new();

        for line in output.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with("total") {
                continue;
            }

            if let Some(entry) = parse_ls_line(line, remote_path) {
                // Skip . and ..
                if entry.name != "." && entry.name != ".." {
                    entries.push(entry);
                }
            }
        }

        Ok(entries)
    }

    /// Get file/directory stat info.
    pub fn stat(&self, remote_path: &str) -> AppResult<RemoteEntry> {
        let cmd = format!("stat -c '%F|%s|%Y|%n' '{}' 2>/dev/null", remote_path);
        let output = self.shell_command(&cmd)?;
        let output = output.trim();

        let parts: Vec<&str> = output.splitn(4, '|').collect();
        if parts.len() < 4 {
            return Err(AppError::NotFound {
                path: remote_path.to_string(),
            });
        }

        let is_dir = parts[0].contains("directory");
        let size = parts[1].parse::<u64>().unwrap_or(0);
        let mtime = parts[2].parse::<u64>().unwrap_or(0);
        let name = std::path::Path::new(parts[3])
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| parts[3].to_string());

        Ok(RemoteEntry {
            name,
            path: remote_path.to_string(),
            is_dir,
            size,
            mtime,
        })
    }
}

/// A file or directory entry on the remote device.
#[derive(Debug, Clone)]
pub struct RemoteEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    pub mtime: u64,
}

/// Parse a line of `ls -la` output into a RemoteEntry.
fn parse_ls_line(line: &str, parent_path: &str) -> Option<RemoteEntry> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 6 {
        return None;
    }

    let perms = parts[0];
    if perms.len() != 10 {
        return None;
    }
    let is_dir = perms.starts_with('d');
    let is_link = perms.starts_with('l');

    // Skip symlinks to avoid complexity with target resolution
    if is_link {
        return None;
    }

    let mut date_idx = 0;
    let mut name_idx = 0;

    for (i, part) in parts.iter().enumerate() {
        if i < 3 { continue; }
        
        // Match YYYY-MM-DD
        if part.len() == 10 && part.chars().filter(|c| *c == '-').count() == 2 {
            date_idx = i;
            name_idx = i + 2;
            break;
        }
        
        // Match Month (Jan, Feb, etc)
        if ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"].contains(part) {
            date_idx = i;
            name_idx = i + 3;
            break;
        }
    }

    if date_idx == 0 || name_idx >= parts.len() {
        if parts[1].parse::<u32>().is_ok() {
            date_idx = 5;
            name_idx = std::cmp::min(7, parts.len() - 1);
        } else {
            date_idx = 4;
            name_idx = std::cmp::min(6, parts.len() - 1);
        }
    }

    let size_str = parts.get(date_idx.saturating_sub(1)).unwrap_or(&"0");
    let size = size_str.parse::<u64>().unwrap_or(0);

    let mut name_start = 0;
    let mut current_pos = 0;
    for (i, part) in parts.iter().enumerate() {
        if let Some(pos) = line[current_pos..].find(part) {
            if i == name_idx {
                name_start = current_pos + pos;
                break;
            }
            current_pos += pos + part.len();
        }
    }
    
    let name = if name_start > 0 {
        line[name_start..].to_string()
    } else {
        parts[name_idx..].join(" ")
    };

    let path = if parent_path.ends_with('/') {
        format!("{}{}", parent_path, name)
    } else {
        format!("{}/{}", parent_path, name)
    };

    Some(RemoteEntry {
        name,
        path,
        is_dir,
        size,
        mtime: 0, // ls -la doesn't give epoch time easily
    })
}
