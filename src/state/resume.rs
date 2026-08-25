use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

/// A completed file record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletedFile {
    pub path: String,
    pub size: u64,
    pub mtime: u64,
    pub pulled_at: DateTime<Utc>,
    /// xxh3-128 hash of the pulled file (hex string). None for files pulled before hashing was added.
    #[serde(default)]
    pub hash: Option<String>,
}

/// A failed file record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailedFile {
    pub path: String,
    pub error: String,
    pub attempts: u32,
    pub last_attempt: DateTime<Utc>,
}

/// A sync record for delta comparison — tracks file size and mtime at last
/// successful pull.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncRecord {
    pub path: String,
    pub size: u64,
    /// Modification time (epoch seconds) at last sync. 0 means "unknown"
    /// and is treated as a wildcard — only size is compared for backward
    /// compatibility with records created before mtime tracking was added.
    #[serde(default)]
    pub mtime: u64,
    /// xxh3-128 hash at last sync (hex string). None for records created before hashing.
    #[serde(default)]
    pub hash: Option<String>,
    pub local_path: String,
    pub synced_at: DateTime<Utc>,
}

/// Persisted transfer state for resume support.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferState {
    pub source: String,
    pub destination: String,
    pub started_at: DateTime<Utc>,
    pub last_updated: DateTime<Utc>,
    pub completed_files: Vec<CompletedFile>,
    pub completed_dirs: Vec<String>,
    pub failed_files: Vec<FailedFile>,
    /// Delta sync manifest — records what was last synced for change detection.
    #[serde(default)]
    pub synced_files: Vec<SyncRecord>,
}

impl TransferState {
    pub fn new(source: &str, destination: &str) -> Self {
        let now = Utc::now();
        Self {
            source: source.to_string(),
            destination: destination.to_string(),
            started_at: now,
            last_updated: now,
            completed_files: Vec::new(),
            completed_dirs: Vec::new(),
            failed_files: Vec::new(),
            synced_files: Vec::new(),
        }
    }
}

/// Manages transfer state persistence for resume and delta sync support.
#[derive(Clone)]
pub struct StateManager {
    state: TransferState,
    state_file: PathBuf,
    completed_set: HashSet<String>, // fast lookup for resume
    sync_map: HashMap<String, SyncRecord>, // fast lookup for delta: path -> record
    max_retries: u32,
}

impl StateManager {
    /// Create a new state manager for a transfer.
    pub fn new(source: &str, destination: &str) -> Self {
        Self::new_named(source, destination, "")
    }

    /// Create a new state manager whose state file is namespaced by
    /// `state_name`: `.andpull-state.<state_name>.json` inside `destination`.
    /// An empty `state_name` produces the classic `.andpull-state.json`.
    pub fn new_named(source: &str, destination: &str, state_name: &str) -> Self {
        let filename = if state_name.is_empty() {
            ".andpull-state.json".to_string()
        } else {
            format!(".andpull-state.{}.json", state_name)
        };
        let state_file = Path::new(destination).join(filename);

        Self {
            state: TransferState::new(source, destination),
            state_file,
            completed_set: HashSet::new(),
            sync_map: HashMap::new(),
            max_retries: 3,
        }
    }

    /// Try to load existing state for resume.
    pub fn load_existing(destination: &str) -> Option<Self> {
        let state_file = Path::new(destination).join(".andpull-state.json");

        if !state_file.exists() {
            return None;
        }

        let data = std::fs::read_to_string(&state_file).ok()?;
        let state: TransferState = serde_json::from_str(&data).ok()?;

        let completed_set: HashSet<String> = state
            .completed_files
            .iter()
            .map(|f| f.path.clone())
            .collect();

        let sync_map: HashMap<String, SyncRecord> = state
            .synced_files
            .iter()
            .cloned()
            .map(|r| (r.path.clone(), r))
            .collect();

        Some(Self {
            state,
            state_file,
            completed_set,
            sync_map,
            max_retries: 3,
        })
    }

    /// Check if a file path has already been completed.
    pub fn is_completed(&self, path: &str) -> bool {
        self.completed_set.contains(path)
    }

    /// Check if a directory has been fully completed via tar.
    pub fn is_dir_completed(&self, path: &str) -> bool {
        self.state.completed_dirs.iter().any(|d| d == path)
    }

    /// Delta sync: check if a remote file is unchanged since last sync.
    /// Uses size-only comparison. For strict mtime+size comparison, see
    /// [`is_unchanged_strict`].
    pub fn is_unchanged(&self, path: &str, remote_size: u64) -> bool {
        if let Some(record) = self.sync_map.get(path) {
            record.size == remote_size
        } else {
            false
        }
    }

    /// Strict delta sync: check size **and** mtime. A stored mtime of 0
    /// means "unknown" (back-compat with older state files) and acts as a
    /// wildcard — only size is compared.
    pub fn is_unchanged_strict(&self, path: &str, remote_size: u64, remote_mtime: u64) -> bool {
        if let Some(record) = self.sync_map.get(path) {
            if record.size != remote_size {
                return false;
            }
            // mtime == 0 in the record means legacy / unknown → size-only
            if record.mtime != 0 && remote_mtime != 0 {
                return record.mtime == remote_mtime;
            }
            true
        } else {
            false
        }
    }

    /// Returns true if a sync record exists for this path (used to
    /// distinguish "new" from "changed" in delta reporting).
    pub fn has_sync_record(&self, path: &str) -> bool {
        self.sync_map.contains_key(path)
    }

    /// Get the stored hash for a synced file (if available).
    pub fn get_sync_hash(&self, path: &str) -> Option<&str> {
        self.sync_map
            .get(path)
            .and_then(|r| r.hash.as_deref())
    }

    /// Update the sync record after a successful pull, with optional hash
    /// and mtime.
    pub fn update_sync_record(
        &mut self,
        path: &str,
        size: u64,
        local_path: &str,
        hash: Option<String>,
        mtime: u64,
    ) {
        let record = SyncRecord {
            path: path.to_string(),
            size,
            mtime,
            hash: hash.clone(),
            local_path: local_path.to_string(),
            synced_at: Utc::now(),
        };

        // Update or insert into the sync map
        self.sync_map.insert(path.to_string(), record.clone());

        // Update or insert into the persisted list
        if let Some(existing) = self.state.synced_files.iter_mut().find(|r| r.path == path) {
            existing.size = size;
            existing.mtime = mtime;
            existing.hash = hash;
            existing.local_path = local_path.to_string();
            existing.synced_at = Utc::now();
        } else {
            self.state.synced_files.push(record);
        }
    }

    /// Check if a file should be retried.
    pub fn should_retry(&self, path: &str) -> bool {
        self.state
            .failed_files
            .iter()
            .find(|f| f.path == path)
            .map(|f| f.attempts < self.max_retries)
            .unwrap_or(true)
    }

    /// Mark a file as completed, with optional hash for integrity tracking.
    pub fn mark_file_completed(&mut self, path: &str, size: u64, mtime: u64, hash: Option<String>) {
        let record = CompletedFile {
            path: path.to_string(),
            size,
            mtime,
            pulled_at: Utc::now(),
            hash,
        };

        self.completed_set.insert(path.to_string());
        self.state.completed_files.push(record);
        self.state.last_updated = Utc::now();

        // Remove from failed if it was there
        self.state.failed_files.retain(|f| f.path != path);
    }

    /// Mark a directory as fully completed (via tar).
    pub fn mark_dir_completed(&mut self, path: &str) {
        if !self.state.completed_dirs.contains(&path.to_string()) {
            self.state.completed_dirs.push(path.to_string());
        }
        self.state.last_updated = Utc::now();
    }

    /// Mark a file as failed.
    pub fn mark_file_failed(&mut self, path: &str, error: &str) {
        let now = Utc::now();

        if let Some(existing) = self.state.failed_files.iter_mut().find(|f| f.path == path) {
            existing.attempts += 1;
            existing.error = error.to_string();
            existing.last_attempt = now;
        } else {
            self.state.failed_files.push(FailedFile {
                path: path.to_string(),
                error: error.to_string(),
                attempts: 1,
                last_attempt: now,
            });
        }

        self.state.last_updated = Utc::now();
    }

    /// Save state to disk.
    pub fn save(&self) -> AppResult<()> {
        // Ensure directory exists
        if let Some(parent) = self.state_file.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let json = serde_json::to_string_pretty(&self.state)
            .map_err(|e| AppError::StateCorrupted(format!("Serialization error: {}", e)))?;

        std::fs::write(&self.state_file, json)?;

        Ok(())
    }

    /// Get summary stats from state.
    pub fn stats(&self) -> (usize, usize, usize) {
        (
            self.state.completed_files.len(),
            self.state.failed_files.len(),
            self.state.completed_dirs.len(),
        )
    }

    /// Get the state reference.
    pub fn state(&self) -> &TransferState {
        &self.state
    }

    /// Get the source path.
    pub fn source(&self) -> &str {
        &self.state.source
    }

    /// Get the destination path.
    pub fn destination(&self) -> &str {
        &self.state.destination
    }
}
