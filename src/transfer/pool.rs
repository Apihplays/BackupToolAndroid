use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;

use crate::adb::client::{AdbClient, DeviceInfo};
use crate::error::AppResult;
use crate::state::StateManager;
use crate::transfer::engine::{TransferDirection, TransferProgress};
use crate::transfer::hash::compute_file_hash;
use crate::transfer::recv::RecvPuller;

/// A file job to be pulled by a worker.
#[derive(Debug, Clone)]
pub struct FileJob {
    pub remote_path: String,
    pub local_path: String,
    pub name: String,
    pub size: u64,
    pub mtime: u64,
}

/// Worker pool that distributes file pull jobs across multiple ADB connections.
pub struct WorkerPool {
    pub worker_count: usize,
}

impl WorkerPool {
    pub fn new(worker_count: usize) -> Self {
        Self {
            worker_count: worker_count.max(1),
        }
    }

    /// Auto-detect optimal worker count based on device connection type.
    /// USB connections get 4 workers, WiFi gets 2 (to avoid congestion).
    pub fn auto_detect(device: &DeviceInfo) -> Self {
        let count = if device.transport == "wifi" { 2 } else { 4 };
        Self::new(count)
    }

    /// Execute file pulls concurrently using a thread pool.
    pub fn execute(
        &self,
        device: &DeviceInfo,
        jobs: Vec<FileJob>,
        _destination: &str,
        progress: &Arc<Mutex<TransferProgress>>,
        state_manager: &Arc<Mutex<StateManager>>,
        direction: TransferDirection,
    ) -> AppResult<()> {
        if jobs.is_empty() {
            return Ok(());
        }

        // Update active worker count in progress
        {
            let mut p = progress.lock().unwrap();
            p.active_workers = self.worker_count as u8;
        }

        // Create a channel to distribute jobs
        let (tx, rx) = std::sync::mpsc::channel::<FileJob>();
        let rx = Arc::new(Mutex::new(rx));

        // Spawn worker threads
        let mut handles = Vec::with_capacity(self.worker_count);

        for _worker_id in 0..self.worker_count {
            let rx = Arc::clone(&rx);
            let progress = Arc::clone(progress);
            let state_manager = Arc::clone(state_manager);
            let device = device.clone();

            let handle = thread::spawn(move || {
                // Each worker gets its own ADB client connection
                let mut client = AdbClient::new();
                client.select_device(device);

                loop {
                    // Check cancellation
                    {
                        let p = progress.lock().unwrap();
                        if p.is_cancelled {
                            break;
                        }
                    }

                    // Get next job from channel
                    let job = {
                        let rx = rx.lock().unwrap();
                        rx.recv()
                    };

                    let job = match job {
                        Ok(j) => j,
                        Err(_) => break, // Channel closed, no more jobs
                    };

                    // Update current file display
                    {
                        let mut p = progress.lock().unwrap();
                        p.current_file = job.name.clone();
                    }

                    // Transfer the file
                    let transfer_result = match direction {
                        TransferDirection::Pull => {
                            if let Some(parent) = Path::new(&job.local_path).parent() {
                                let _ = std::fs::create_dir_all(parent);
                            }
                            RecvPuller::pull_file(&client, &job.remote_path, &job.local_path)
                        }
                        TransferDirection::Push => {
                            if let Some(pos) = job.remote_path.rfind('/') {
                                let dir = &job.remote_path[..pos];
                                let _ = client.shell_command(&format!("mkdir -p '{}'", dir));
                            }
                            client
                                .push_file(&job.local_path, &job.remote_path)
                                .map(|_| job.size)
                        }
                    };

                    match transfer_result {
                        Ok(bytes) => {
                            // Compute hash for integrity tracking and dedup
                            let file_hash =
                                compute_file_hash(std::path::Path::new(&job.local_path)).ok();

                            let mut p = progress.lock().unwrap();
                            p.completed_files += 1;
                            p.transferred_bytes += bytes;
                            if file_hash.is_some() {
                                p.integrity_verified += 1;
                            }
                            p.update_speed();
                            drop(p);

                            let mut sm = state_manager.lock().unwrap();
                            sm.mark_file_completed(
                                &job.remote_path,
                                job.size,
                                job.mtime,
                                file_hash.clone(),
                            );
                            sm.update_sync_record(
                                &job.remote_path,
                                job.size,
                                &job.local_path,
                                file_hash,
                            );
                        }
                        Err(e) => {
                            let mut p = progress.lock().unwrap();
                            p.failed_files += 1;
                            p.errors.push((job.remote_path.clone(), e.to_string()));
                            p.update_speed();
                            drop(p);

                            let mut sm = state_manager.lock().unwrap();
                            sm.mark_file_failed(&job.remote_path, &e.to_string());
                        }
                    }

                    // Auto-save state periodically
                    let completed = { progress.lock().unwrap().completed_files };
                    if completed % 50 == 0 {
                        let sm = state_manager.lock().unwrap();
                        let _ = sm.save();
                    }
                }
            });

            handles.push(handle);
        }

        // Feed jobs into the channel from the main thread
        for job in jobs {
            let is_cancelled = progress.lock().unwrap().is_cancelled;
            if is_cancelled {
                break;
            }
            // If send fails, workers have all exited
            if tx.send(job).is_err() {
                break;
            }
        }

        // Drop sender to signal workers that no more jobs are coming
        drop(tx);

        // Wait for all workers to finish
        for handle in handles {
            let _ = handle.join();
        }

        // Reset active workers
        {
            let mut p = progress.lock().unwrap();
            p.active_workers = 0;
        }

        Ok(())
    }
}
