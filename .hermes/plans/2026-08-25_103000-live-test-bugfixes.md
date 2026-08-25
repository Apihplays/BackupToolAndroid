# Live-Testing Bugfix Plan — Prioritized

> **For Hermes:** Implement task-by-task. Each task ends with `cargo test && cargo clippy -- -D warnings` gate.

**Goal:** Fix the four issues found during live DCIM backup on veux (796 files, Android 16, rooted).

---

## Issue Analysis

### Issue 1: Tar path re-pulls all files (no delta check) 🔴 Critical

**Root cause:** `TarPuller::pull_dir` extracts every file and records state, but never CHECKS state before extraction. On re-run, the entire tar stream runs again because there's no `is_completed` / `is_unchanged_strict` gate.

**Impact:** A 796-file DCIM backup that should take seconds on re-run (all skipped) instead re-pulls ~5GB every time.

**Fix strategy (two layers):**

1. **Pre-gate:** Before invoking tar at all, check if every file under the dir already has a completed sync record. If so, skip the entire tar pull. This handles the common "nothing changed" case in O(1).

2. **Per-entry skip:** For the incremental case (some files changed), read each tar header, check `is_unchanged_strict`, and if unchanged, skip the data bytes (read + discard) instead of writing to disk. This requires passing `state_manager` into `StreamingTarReader` or doing the check in the `pull_dir` loop before calling `stream_entry_data`.

**Files:** `src/transfer/tar.rs`, `src/transfer/engine.rs`

### Issue 2: File count double-counting (1609 vs 796) 🔴 Critical

**Root cause:** `selected_dirs()` returns ALL selected dirs recursively. When `set_selected_recursive(true)` is called on DCIM, it selects DCIM, DCIM/Camera, DCIM/Facebook, etc. The engine's `execute()` loop iterates ALL selected dirs and calls `TarPuller::pull_dir` for each:

- Tar pulls DCIM → extracts Camera/file1.jpg, Facebook/file2.jpg, ...
- Tar pulls DCIM/Camera → extracts Camera/file1.jpg **AGAIN**
- Tar pulls DCIM/Facebook → extracts file2.jpg **AGAIN**

Result: 1609 = 2 × 796 + 17 (whatsapp files).

**Fix:** Filter `selected_dirs` to only "top-level" dirs — dirs whose parent is NOT also in `selected_dirs`. This prevents ancestor-descendant overlap.

**Files:** `src/transfer/engine.rs`

### Issue 3: mtime=0 in state records breaks delta sync 🟡 High

**Root cause:** `TarPuller::pull_dir` hardcodes mtime=0:
```rust
state_manager.mark_file_completed(&remote_path, entry.size, 0, None);
state_manager.update_sync_record(&remote_path, entry.size, &local_str, None, 0);
```

The tar header contains mtime at bytes 136–147 (octal) but `stream_next_entry` doesn't parse it. With mtime=0 in the sync record, `is_unchanged_strict` falls back to size-only comparison — photos that are re-encoded at the same size won't be detected as changed.

**Fix:** Parse mtime from the tar header, add it to `TarEntryMeta`, pass it through to state recording.

**Files:** `src/transfer/tar.rs`

### Issue 4: TUI worker override not plumbed to manual transfers 🟢 Low

**Root cause:** `App::start_transfer` (manual file browser flow) creates `TransferEngine { progress, worker_override: None }` — it ignores `self.worker_override`. Worker cycling only affects profile backups.

**Fix:** Pass `self.worker_override` into the engine construction in `start_transfer`.

**Files:** `src/app.rs`

---

## Implementation Order

### Task 1: Fix file count double-counting (Issue 2)
**Why first:** This is the simplest fix with the highest impact — it makes the reported file count correct, which is a prerequisite for trusting any other metrics.

1. In `engine.rs::execute()`, filter `selected_dirs` to only top-level dirs:
   ```rust
   let top_dirs: Vec<&FileNode> = selected_dirs.iter()
       .filter(|d| !selected_dirs.iter().any(|other| other != *d && d.path.starts_with(&other.path)))
       .copied()
       .collect();
   ```
2. Use `top_dirs` in the loop instead of `selected_dirs`.
3. Unit test: build a tree with DCIM → {Camera, Facebook}, verify `selected_dirs` filtering produces only DCIM.
4. Gate: `cargo test && cargo clippy -- -D warnings`

### Task 2: Parse mtime from tar headers (Issue 3)
**Why second:** Unblocks proper delta sync for the tar path; prerequisite for Task 3's per-entry skip logic.

1. Add `mtime: u64` to `TarEntryMeta`.
2. In `stream_next_entry`, parse bytes 136–147 as octal u64 (same as size field).
3. Handle GNU long-link path: parse mtime from the REAL header (after the @LongLink data), not the pseudo-header.
4. Return mtime in `TarEntryMeta`.
5. In `TarPuller::pull_dir`, pass `entry.mtime` to `mark_file_completed` and `update_sync_record` instead of hardcoded 0.
6. Update test helper `build_tar` to accept optional mtime parameter; add test verifying mtime round-trip.
7. Gate: `cargo test && cargo clippy -- -D warnings`

### Task 3: Add delta check to tar path (Issue 1)
**Why third:** Depends on Task 2 (mtime must be in sync records for `is_unchanged_strict` to work).

**Layer A — Pre-gate (skip entire tar if dir unchanged):**
1. In `engine.rs::execute()`, before calling `TarPuller::pull_dir`, check if ALL files under `dir_node` have completed sync records. If so, skip the tar pull and increment `skipped_files` for each.
2. Add helper `fn all_files_completed(state_manager: &StateManager, dir: &FileNode) -> bool` that recursively checks `is_completed` for every non-dir child.
3. Unit test: build tree with 3 files, mark 2 completed → returns false; mark all 3 → returns true.

**Layer B — Per-entry skip (incremental tar):**
1. In `TarPuller::pull_dir`, after reading each entry header (name + size + mtime), check `state_manager.is_unchanged_strict(&remote_path, entry.size, entry.mtime)`.
2. If unchanged: skip the data bytes (call a new `skip_entry_data` method that reads + discards), increment `skipped_files` in progress, continue.
3. If changed or new: extract normally (existing `stream_entry_data`).
4. This handles the case where only a few files changed between runs.
5. Add test: build tar with 3 entries, mark 1 as completed in state, verify only 2 are extracted.
6. Gate: `cargo test && cargo clippy -- -D warnings`

### Task 4: Plumb worker override to manual transfers (Issue 4)
**Why last:** Smallest scope, no dependencies.

1. In `app.rs::start_transfer`, replace `worker_override: None` with `worker_override: self.worker_override`.
2. Gate: `cargo test && cargo clippy -- -D warnings`

---

## Files Likely to Change
- `src/transfer/engine.rs` — top-level dir filtering (Task 1), pre-gate check (Task 3A)
- `src/transfer/tar.rs` — mtime parsing (Task 2), per-entry skip (Task 3B)
- `src/app.rs` — worker override plumbing (Task 4)

## Tests / Validation
- Unit: top-level dir filtering, mtime round-trip, all_files_completed helper, tar skip logic
- Integration: build tar with mixed completed/new entries → verify correct extraction count
- Gates: `cargo test`, `cargo clippy --all-targets -- -D warnings`
- Live: re-run DCIM backup on veux → expect "0 new, 0 changed, 796 skipped" in seconds

## Risks
- **Per-entry tar skip reads + discards unchanged data** — still transfers bytes over USB but avoids local disk writes. Acceptable tradeoff; true skip would require device-side filtering (not possible with standard tar).
- **Pre-gate assumes all files under a dir were previously tar-pulled** — if a previous run used the per-file concurrent path, some files may have completed records without the dir being "tar-completed". The pre-gate should check individual file completion, not dir completion.
