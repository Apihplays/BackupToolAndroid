# Optimization Execution Plan — andpull v0.2.1

> **For Freebuff/Codebuff:** Execute each phase sequentially. Run `cargo test` and `cargo clippy --all-targets -- -D warnings` after EACH phase. Do NOT skip tests.

---

## Phase 0: Delta bypass (Critical — fixes re-run hang)

**Goal:** When state file says all files unchanged, skip tar streaming entirely.

**Files to modify:** `src/transfer/engine.rs`

**Changes:**
1. In `execute()` method, before calling `TarPuller::pull_dir`, check if ALL files under this directory are already synced in state (matching size + mtime). If yes, skip tar call entirely and return `0 new, 0 changed, N skipped`.
2. Specifically: after `let mut new_files = 0; let mut changed_files = 0;` loop over `dir_node.children`, call `state_manager.is_unchanged_strict(key, size, mtime)` for each. If ALL return true, log skip and continue to next directory.

**Gate:** `cargo test` + `cargo clippy`

---

## Phase 1: Atomic counters (Throughput)

**Goal:** Replace `Arc<Mutex<TransferProgress>>` lock contention with lock-free atomics.

**Files to modify:** `src/transfer/engine.rs`, `src/transfer/pool.rs`, `src/transfer/tar.rs`

**Changes:**
1. Add `use std::sync::atomic::{AtomicU64, AtomicBool, Ordering};` to `engine.rs`.
2. Create new struct `AtomicProgress` with fields: `completed_files: AtomicU64`, `new_files: AtomicU64`, `changed_files: AtomicU64`, `skipped_files: AtomicU64`, `delta_skipped: AtomicU64`, `transferred_bytes: AtomicU64`, `failed_files: AtomicU64`.
3. Keep `current_file: Arc<Mutex<String>>` for the TUI display string (can't atomic a String).
4. Replace `progress.lock().unwrap().completed_files += 1` with `progress.completed_files.fetch_add(1, Ordering::Relaxed)`.
5. In `pool.rs`, worker threads use `AtomicProgress` references directly.
6. TUI reads atomics: `progress.completed_files.load(Ordering::Relaxed)`.

**Gate:** `cargo test` + `cargo clippy`

---

## Phase 2: Tar buffer size (Throughput)

**Goal:** Increase I/O buffer sizes for tar streaming.

**Files to modify:** `src/transfer/tar.rs`

**Changes:**
1. In `stream_entry_data()`, replace `BufReader::new(self.reader)` with `BufReader::with_capacity(64 * 1024, self.reader)`.
2. Replace `BufWriter::new(File::create(&full_path)?)` with `BufWriter::with_capacity(64 * 1024, File::create(&full_path)?)`.
3. These are created per-entry currently — ideally re-use a single 64KB buffer across entries by passing it into `stream_entry_data` as `&mut BufReader` and `&mut BufWriter`.

**Gate:** `cargo test` + `cargo clippy`

---

## Phase 3: Single find-scan (Throughput)

**Goal:** Replace recursive `ls -la` directory listing with single `find` command.

**Files to modify:** `src/adb/client.rs`, `src/scanner/tree.rs`

**Changes:**
1. Add `pub fn find_all_files(&self, root: &str) -> AppResult<Vec<RemoteEntry>>` to `AdbClient` in `client.rs`. Command: `adb exec-out "find '{root}' -type f -printf '%s|%T@|%p\n'"`. Parse output into `Vec<RemoteEntry>`.
2. Add fallback: if `-printf` not supported (toybox/busybox variant), fall back to `find '{root}' -type f -exec stat -c '%s|%Y|%n' {} +`.
3. In `scanner/tree.rs`, when `use_find_scan` flag is set (new field on `Scanner`), call `find_all_files` instead of recursive `list_dir`.
4. Gate on `root: &str` — only use for USB connections (check `device.transport != "wifi"`).

**Gate:** `cargo test` + `cargo clippy`

---

## Phase 4: Retry + zombie reap (Reliability)

**Goal:** Add ADB retry on transient errors. Reap zombie child processes.

**Files to modify:** `src/adb/client.rs`, `src/transfer/pool.rs`

**Changes:**
1. In `client.rs`, add `fn with_retry<F, T>(&self, f: F) -> AppResult<T>` where `F: Fn() -> AppResult<T>`. Retry up to 3 times on `AppError::Io` with exponential backoff (1s, 2s, 4s).
2. Wrap `shell_command` and `pull_file` calls in `with_retry`.
3. In `pool.rs`, implement `Drop` for `WorkerPool` that kills any still-running child processes.

**Gate:** `cargo test` + `cargo clippy`

---

## Phase 5: Quick wins (Polish)

**Goal:** Minor optimizations and cleanup.

**Files to modify:** multiple

**Changes:**
1. Cache `su_available()` in `AdbClient` struct (add `su_cached: Option<bool>` field, check once).
2. Move `tempfile` from `[dependencies]` to `[dev-dependencies]` in `Cargo.toml`.
3. Replace `fs2::available_space` with `libc::statvfs` in `src/error.rs` (remove `fs2` dep from Cargo.toml).
4. In `tar.rs`, replace `format!("{}{}", remote_dir, entry.name)` with pre-allocated buffer using `write!()`.

**Gate:** `cargo test` + `cargo clippy` + `cargo bloat --release -n 20`

---

## Final Verification

1. `cargo test` — all tests pass
2. `cargo clippy --all-targets -- -D warnings` — clean
3. `cargo build --release` — builds
4. Commit with descriptive message covering all phases
