use std::path::Path;
use std::sync::{Arc, Mutex};

use crate::adb::client::{AdbClient, DeviceInfo};
use crate::error::{AppError, AppResult};
use crate::scanner::FileNode;
use crate::state::StateManager;
use crate::transfer::tar::TarPuller;
use crate::transfer::recv::RecvPuller;
use crate::transfer::pool::{WorkerPool, FileJob};

/// Progress information for a running transfer.
#[derive(Debug, Clone)]
pub struct TransferProgress {
    pub total_files: u64,
    pub completed_files: u64,
    pub failed_files: u64,
    pub skipped_files: u64,
    pub delta_skipped: u64,
    pub total_bytes: u64,
    pub transferred_bytes: u64,
    pub current_file: String,
    pub speed_bytes_per_sec: f64,
    pub errors: Vec<(String, String)>, // (path, error message)
    pub is_complete: bool,
    pub is_cancelled: bool,
    pub start_time: std::time::Instant,
    pub active_workers: u8,
}

impl TransferProgress {
    pub fn new(total_files: u64, total_bytes: u64) -> Self {
        Self {
            total_files,
            completed_files: 0,
            failed_files: 0,
            skipped_files: 0,
            delta_skipped: 0,
            total_bytes,
            transferred_bytes: 0,
            current_file: String::new(),
            speed_bytes_per_sec: 0.0,
            errors: Vec::new(),
            is_complete: false,
            is_cancelled: false,
            start_time: std::time::Instant::now(),
            active_workers: 1,
        }
    }

    /// Calculate elapsed time.
    pub fn elapsed(&self) -> std::time::Duration {
        self.start_time.elapsed()
    }

    /// Estimate time remaining.
    pub fn eta(&self) -> Option<std::time::Duration> {
        if self.speed_bytes_per_sec <= 0.0 || self.transferred_bytes == 0 {
            return None;
        }
        let remaining = self.total_bytes.saturating_sub(self.transferred_bytes);
        let secs = remaining as f64 / self.speed_bytes_per_sec;
        Some(std::time::Duration::from_secs_f64(secs))
    }

    /// Progress percentage (0.0 - 100.0).
    pub fn percent(&self) -> f64 {
        if self.total_bytes == 0 {
            return 0.0;
        }
        (self.transferred_bytes as f64 / self.total_bytes as f64) * 100.0
    }

    /// Update speed calculation.
    pub fn update_speed(&mut self) {
        let elapsed = self.elapsed().as_secs_f64();
        if elapsed > 0.0 {
            self.speed_bytes_per_sec = self.transferred_bytes as f64 / elapsed;
        }
    }
}

/// The transfer engine orchestrates file pulling from device to local disk.
pub struct TransferEngine {
    pub progress: Arc<Mutex<TransferProgress>>,
}

impl TransferEngine {
    pub fn new() -> Self {
        Self {
            progress: Arc::new(Mutex::new(TransferProgress::new(0, 0))),
        }
    }

    /// Execute a transfer for the given selected nodes.
    /// Uses concurrent workers when pulling individual files.
    pub fn execute(
        &self,
        client: &AdbClient,
        root: &FileNode,
        destination: &str,
        state_manager: &mut StateManager,
    ) -> AppResult<()> {
        let selected_files = root.selected_files();
        let selected_dirs = root.selected_dirs();

        if selected_files.is_empty() && selected_dirs.is_empty() {
            return Ok(());
        }

        let total_bytes = root.selected_total_size();
        let total_files = root.selected_file_count();

        // Initialize progress
        {
            let mut progress = self.progress.lock().unwrap();
            *progress = TransferProgress::new(total_files, total_bytes);
        }

        // Create destination directory
        std::fs::create_dir_all(destination)
            .map_err(|e| AppError::Io(e))?;

        // Check if we can use tar streaming for entire selected directories
        let has_tar = crate::adb::shell::ShellExecutor::has_tar(client);

        // Auto-detect worker count from device connection type
        let pool = client.selected_device.as_ref()
            .map(|d| WorkerPool::auto_detect(d))
            .unwrap_or_else(|| WorkerPool::new(1));

        // Process selected directories
        for dir_node in &selected_dirs {
            let is_cancelled = {
                self.progress.lock().unwrap().is_cancelled
            };
            if is_cancelled {
                break;
            }

            // Compute local destination path preserving directory structure
            let relative = dir_node.path.trim_start_matches("/sdcard/");
            let local_dir = Path::new(destination).join(relative);

            if has_tar {
                // Use tar streaming for entire directory
                match TarPuller::pull_dir(client, &dir_node.path, local_dir.to_str().unwrap_or(""), &self.progress) {
                    Ok(stats) => {
                        let mut progress = self.progress.lock().unwrap();
                        progress.completed_files += stats.files_pulled;
                        progress.transferred_bytes += stats.bytes_transferred;
                        progress.update_speed();

                        // Record in state
                        state_manager.mark_dir_completed(&dir_node.path);
                    }
                    Err(e) => {
                        // Fallback to concurrent file pull
                        let mut progress = self.progress.lock().unwrap();
                        progress.errors.push((dir_node.path.clone(), format!("Tar failed, falling back: {}", e)));
                        drop(progress);

                        let files = dir_node.selected_files();
                        self.pull_files_concurrent(
                            client, &pool, &files, destination, state_manager,
                        )?;
                    }
                }
            } else {
                // No tar available, pull files concurrently
                let files = dir_node.selected_files();
                self.pull_files_concurrent(
                    client, &pool, &files, destination, state_manager,
                )?;
            }
        }

        // Process individually selected files (not part of a selected directory)
        let standalone_files: Vec<&FileNode> = selected_files
            .iter()
            .filter(|f| {
                !selected_dirs.iter().any(|d| f.path.starts_with(&d.path))
            })
            .cloned()
            .collect();

        if !standalone_files.is_empty() {
            self.pull_files_concurrent(
                client, &pool, &standalone_files, destination, state_manager,
            )?;
        }

        // Mark complete
        {
            let mut progress = self.progress.lock().unwrap();
            progress.is_complete = true;
            progress.update_speed();
        }

        // Save final state
        state_manager.save()?;

        Ok(())
    }

    /// Pull files using the concurrent worker pool.
    /// Pre-filters resume/delta skips before dispatching to workers.
    fn pull_files_concurrent(
        &self,
        client: &AdbClient,
        pool: &WorkerPool,
        files: &[&FileNode],
        destination: &str,
        state_manager: &mut StateManager,
    ) -> AppResult<()> {
        let device = match client.selected_device.as_ref() {
            Some(d) => d.clone(),
            None => return self.pull_files_individually(client, files, destination, state_manager),
        };

        // Pre-filter: handle resume skips and delta skips before dispatching
        let mut jobs = Vec::with_capacity(files.len());

        for file_node in files {
            // Check if already completed (resume)
            if state_manager.is_completed(&file_node.path) {
                let mut progress = self.progress.lock().unwrap();
                progress.skipped_files += 1;
                progress.completed_files += 1;
                progress.transferred_bytes += file_node.size;
                progress.update_speed();
                continue;
            }

            // Delta sync: check if file is unchanged since last sync
            if state_manager.is_unchanged(&file_node.path, file_node.size) {
                let mut progress = self.progress.lock().unwrap();
                progress.delta_skipped += 1;
                progress.completed_files += 1;
                progress.transferred_bytes += file_node.size;
                progress.update_speed();
                continue;
            }

            // Build job for worker pool
            let relative = file_node.path.trim_start_matches("/sdcard/");
            let local_path = Path::new(destination).join(relative);

            // Ensure parent directory exists
            if let Some(parent) = local_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }

            jobs.push(FileJob {
                remote_path: file_node.path.clone(),
                local_path: local_path.to_string_lossy().to_string(),
                name: file_node.name.clone(),
                size: file_node.size,
                mtime: file_node.mtime,
            });
        }

        if jobs.is_empty() {
            return Ok(());
        }

        // Wrap state_manager in Arc<Mutex> for concurrent access
        let shared_state = Arc::new(Mutex::new(std::mem::replace(
            state_manager,
            StateManager::new("", destination),
        )));

        pool.execute(&device, jobs, destination, &self.progress, &shared_state)?;

        // Move state manager back out
        let recovered = Arc::try_unwrap(shared_state)
            .unwrap_or_else(|arc| {
                let cloned = arc.lock().unwrap().clone();
                Mutex::new(cloned)
            });
        *state_manager = recovered.into_inner().unwrap();

        Ok(())
    }

    /// Pull files one by one with progress tracking (fallback for single-threaded).
    fn pull_files_individually(
        &self,
        client: &AdbClient,
        files: &[&FileNode],
        destination: &str,
        state_manager: &mut StateManager,
    ) -> AppResult<()> {
        for file_node in files {
            let is_cancelled = {
                self.progress.lock().unwrap().is_cancelled
            };
            if is_cancelled {
                break;
            }

            // Check if already completed (for resume)
            if state_manager.is_completed(&file_node.path) {
                let mut progress = self.progress.lock().unwrap();
                progress.skipped_files += 1;
                progress.completed_files += 1;
                progress.transferred_bytes += file_node.size;
                progress.update_speed();
                continue;
            }

            // Delta sync: check if file is unchanged since last sync (size comparison)
            if state_manager.is_unchanged(&file_node.path, file_node.size) {
                let mut progress = self.progress.lock().unwrap();
                progress.delta_skipped += 1;
                progress.completed_files += 1;
                progress.transferred_bytes += file_node.size;
                progress.update_speed();
                continue;
            }

            // Update current file
            {
                let mut progress = self.progress.lock().unwrap();
                progress.current_file = file_node.name.clone();
            }

            // Compute local path
            let relative = file_node.path.trim_start_matches("/sdcard/");
            let local_path = Path::new(destination).join(relative);

            // Ensure parent directory exists
            if let Some(parent) = local_path.parent() {
                std::fs::create_dir_all(parent)?;
            }

            // Pull the file
            match RecvPuller::pull_file(client, &file_node.path, local_path.to_str().unwrap_or("")) {
                Ok(bytes) => {
                    let local_path_str = local_path.to_str().unwrap_or("");

                    let mut progress = self.progress.lock().unwrap();
                    progress.completed_files += 1;
                    progress.transferred_bytes += bytes;
                    progress.update_speed();

                    state_manager.mark_file_completed(
                        &file_node.path,
                        file_node.size,
                        file_node.mtime,
                    );
                    // Update delta sync record for future runs
                    state_manager.update_sync_record(
                        &file_node.path,
                        file_node.size,
                        local_path_str,
                    );
                }
                Err(e) => {
                    let mut progress = self.progress.lock().unwrap();
                    progress.failed_files += 1;
                    progress.errors.push((file_node.path.clone(), e.to_string()));
                    progress.update_speed();

                    state_manager.mark_file_failed(&file_node.path, &e.to_string());
                }
            }

            // Auto-save state periodically (every 50 files)
            let completed = self.progress.lock().unwrap().completed_files;
            if completed % 50 == 0 {
                let _ = state_manager.save();
            }
        }

        Ok(())
    }
}

