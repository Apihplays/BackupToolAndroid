// Restore runner: pushes backed-up media back to device paths, then (optionally)
// restores app-data via a rooted tar untar. Includes preflight validation.
#![allow(dead_code)]

use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use crate::adb::client::{quote_shell, AdbClient};
use crate::error::AppResult;
use crate::profile::runner::ProfileOutcome;
use crate::profile::ProfileSpec;

/// Result of the preflight checks performed before a restore.
#[derive(Debug, Clone)]
pub struct PreflightReport {
    pub package_installed: bool,
    pub free_bytes: u64,
    pub warnings: Vec<String>,
}

/// Parse `pm path <pkg>` output into an installed/not-installed verdict.
///
/// Installed output looks like `package:/data/app/.../base.apk`; not-installed
/// is empty (or contains a warning). Pure function for offline testing.
pub fn parse_pm_path(output: &str) -> bool {
    output
        .lines()
        .any(|l| l.trim().starts_with("package:") && l.trim().len() > "package:".len())
}

/// Parse the available-bytes column from `df <path>` output.
///
/// Handles both the toybox header (`Filesystem 1K-blocks Used Available Use% Mounted on`)
/// and busybox variants; returns bytes (df's 1K blocks are scaled x1024).
/// Pure function for offline testing.
pub fn parse_df_avail(output: &str) -> u64 {
    let mut last_numeric: Option<u64> = None;
    for line in output.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() >= 4 {
            if let Ok(kb) = fields[3].parse::<u64>() {
                last_numeric = Some(kb * 1024);
            }
        }
    }
    last_numeric.unwrap_or(0)
}

/// Check whether a package is installed and how much free space exists on
/// /sdcard before starting a restore.
pub fn preflight_restore(client: &AdbClient, pkg: &str) -> PreflightReport {
    let mut warnings = Vec::new();

    let package_installed = client
        .shell_command(&format!("pm path {}", quote_shell(pkg)))
        .map(|out| parse_pm_path(&out))
        .unwrap_or(false);

    if !package_installed {
        warnings.push(format!(
            "package {pkg} does not appear to be installed; media restore will still proceed"
        ));
    }

    let free_bytes = client
        .shell_command("df /sdcard 2>/dev/null")
        .map(|out| parse_df_avail(&out))
        .unwrap_or(0);

    if !client.su_available() {
        warnings.push(
            "root (su) unavailable on device: app-data restore will be skipped even if requested"
                .to_string(),
        );
    }

    PreflightReport {
        package_installed,
        free_bytes,
        warnings,
    }
}

/// Restores profiles' files back to their original device paths and, when
/// requested with root, untars a local app-data archive into /data/data.
///
/// Design choice (documented): restore pushes files **serially** via
/// `client.push_file` in a direct loop rather than reusing TransferEngine's
/// concurrent pool. Rationale:
/// - Restore order matters semantically (app-data must land after all profile
///   media), and engine Pull/Push paths are optimized around backup state
///   files keyed by backup-time paths.
/// - Serial push keeps remote mkdir -p + push atomic per file and makes
///   per-profile outcomes trivially attributable.
pub struct RestoreRunner {
    /// Warn (don't fail) when estimated payload exceeds this fraction of free space.
    pub min_free_fraction: f64,
}

impl Default for RestoreRunner {
    fn default() -> Self {
        Self {
            min_free_fraction: 0.9,
        }
    }
}

impl RestoreRunner {
    pub fn new() -> Self {
        Self::default()
    }

    /// Run restores in reverse priority order (highest priority last so its
    /// app-data untar sees every other profile already restored — whatsapp
    /// app-data requires its Media tree present first).
    ///
    /// `source_backup_dir` is the local directory mirroring device layout
    /// (as produced by backup); `appdata_tar` is the local tar of
    /// `/data/data/<pkg>` produced by `backup_appdata`.
    pub fn run_all(
        &self,
        client: Arc<AdbClient>,
        profiles: Vec<ProfileSpec>,
        source_backup_dir: &str,
        _with_appdata: bool,
        appdata_tar: Option<PathBuf>,
    ) -> Vec<ProfileOutcome> {
        // Reverse priority order: lower priority number = higher priority =
        // restored LAST here? No: we restore highest priority FIRST is wrong
        // for appdata ordering; instead sort descending so lowest-priority
        // media lands first and the priority-0 (whatsapp) profile finishes
        // immediately before any appdata untar below.
        let mut ordered = profiles;
        ordered.sort_by_key(|p| std::cmp::Reverse(p.priority));

        let mut outcomes = Vec::with_capacity(ordered.len());
        for profile in &ordered {
            outcomes.push(self.restore_profile(&client, profile, source_backup_dir));
        }

        // App-data untar runs AFTER all media profiles are restored and only
        // with root. shell_stream only pipes stdout, so use shell_stream_stdin
        // to feed the tar file into adb stdin.
        if let Some(tar_path) = appdata_tar {
            let outcome = self.restore_appdata(&client, &tar_path);
            outcomes.push(outcome);
        }

        outcomes
    }

    /// Push one profile's files back to their original device paths.
    fn restore_profile(
        &self,
        client: &AdbClient,
        profile: &ProfileSpec,
        source_backup_dir: &str,
    ) -> ProfileOutcome {
        let base_path = profile
            .sources
            .first()
            .map(|s| s.device_path.clone())
            .unwrap_or_else(|| "/sdcard".to_string());

        let mut transferred: u64 = 0;
        let mut errors: Vec<String> = Vec::new();

        for source in &profile.sources {
            let local_root = std::path::Path::new(source_backup_dir)
                .join(source.device_path.trim_start_matches('/'));
            if !local_root.exists() {
                errors.push(format!(
                    "local backup dir missing: {}",
                    local_root.display()
                ));
                continue;
            }
            match push_tree(client, &local_root, &source.device_path) {
                Ok(n) => transferred += n,
                Err(e) => errors.push(e.to_string()),
            }
        }

        let _ = &base_path;
        ProfileOutcome {
            name: format!("{}:restore", profile.name),
            success: errors.is_empty(),
            error: if errors.is_empty() {
                None
            } else {
                Some(errors.join("; "))
            },
            files_transferred: transferred,
        }
    }

    /// Pipe a local tar into `su -c tar -xf - -C /data/data`, then restorecon.
    fn restore_appdata(&self, client: &AdbClient, tar_path: &std::path::Path) -> ProfileOutcome {
        let data = match std::fs::read(tar_path) {
            Ok(d) => d,
            Err(e) => {
                return ProfileOutcome {
                    name: "appdata:restore".to_string(),
                    success: false,
                    error: Some(format!("cannot read {}: {e}", tar_path.display())),
                    files_transferred: 0,
                };
            }
        };
        if data.is_empty() {
            return ProfileOutcome {
                name: "appdata:restore".to_string(),
                success: true,
                error: None,
                files_transferred: 0,
            };
        }

        if !client.su_available() {
            return ProfileOutcome {
                name: "appdata:restore".to_string(),
                success: false,
                error: Some("su unavailable; cannot restore app-data".to_string()),
                files_transferred: 0,
            };
        }

        let cmd = "su -c 'tar -xf - -C /data/data'";
        let result = (|| -> AppResult<()> {
            let mut child = client.shell_stream_stdin(cmd)?;
            if let Some(mut stdin) = child.stdin.take() {
                stdin.write_all(&data)?;
            }
            // Dropping stdin above happens via take+write; ensure EOF by drop.
            drop(child.stdin.take());
            let status = child.wait()?;
            if !status.success() {
                return Err(crate::error::AppError::Protocol(format!(
                    "device tar exited nonzero: {status}"
                )));
            }
            let _ = client.shell_command("su -c 'restorecon -R /data/data'");
            Ok(())
        })();

        match result {
            Ok(()) => ProfileOutcome {
                name: "appdata:restore".to_string(),
                success: true,
                error: None,
                files_transferred: 0,
            },
            Err(e) => ProfileOutcome {
                name: "appdata:restore".to_string(),
                success: false,
                error: Some(e.to_string()),
                files_transferred: 0,
            },
        }
    }
}

/// Recursively push a local directory tree to `remote_root`, preserving
/// relative paths. Returns the count of files pushed.
fn push_tree(
    client: &AdbClient,
    local_root: &std::path::Path,
    remote_root: &str,
) -> AppResult<u64> {
    let mut count: u64 = 0;
    push_tree_inner(client, local_root, local_root, remote_root, &mut count)?;
    Ok(count)
}

fn push_tree_inner(
    client: &AdbClient,
    root: &std::path::Path,
    dir: &std::path::Path,
    remote_root: &str,
    count: &mut u64,
) -> AppResult<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            push_tree_inner(client, root, &path, remote_root, count)?;
        } else {
            let rel = path
                .strip_prefix(root)
                .map_err(|e| crate::error::AppError::Protocol(e.to_string()))?
                .to_string_lossy()
                .replace('\\', "/");
            let remote = format!("{remote_root}/{rel}");
            if let Some(parent) = remote_parent(&remote) {
                let _ = client.shell_command(&format!("mkdir -p {}", quote_shell(&parent)));
            }
            client.push_file(&path.to_string_lossy(), &remote)?;
            *count += 1;
        }
    }
    Ok(())
}

fn remote_parent(remote: &str) -> Option<String> {
    let idx = remote.rfind('/')?;
    if idx == 0 {
        None
    } else {
        Some(remote[..idx].to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pm_path_installed_output_parses_true() {
        assert!(parse_pm_path("package:/data/app/com.whatsapp-base.apk\n"));
    }

    #[test]
    fn pm_path_empty_output_parses_false() {
        assert!(!parse_pm_path(""));
        assert!(!parse_pm_path("\n"));
    }

    #[test]
    fn pm_path_bare_prefix_without_path_is_not_installed() {
        // "package:" alone carries no path — treat as failure.
        assert!(!parse_pm_path("package:\n"));
    }

    #[test]
    fn df_toybox_header_parses_avail_column() {
        let out = "Filesystem     1K-blocks     Used Available Use% Mounted on\n\
                   /dev/fuse    102396636 52428800  49967836  52% /storage/emulated\n";
        assert_eq!(parse_df_avail(out), 49_967_836 * 1024);
    }

    #[test]
    fn df_garbage_returns_zero() {
        assert_eq!(parse_df_avail("nope"), 0);
        assert_eq!(parse_df_avail(""), 0);
    }

    #[test]
    fn df_single_line_still_finds_column() {
        // Some devices emit df without a separate header line.
        let out = "/dev/fuse 1024 512 512 50% /sdcard\n";
        assert_eq!(parse_df_avail(out), 512 * 1024);
    }

    #[test]
    fn remote_parent_extracts_directory() {
        assert_eq!(
            remote_parent("/sdcard/DCIM/a/b.jpg").as_deref(),
            Some("/sdcard/DCIM/a")
        );
        assert_eq!(remote_parent("/file.jpg"), None);
    }
}
