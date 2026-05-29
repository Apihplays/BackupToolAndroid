# andpull — Task Tracker

> This file lives in the project so we can always look back on what we've done and what's next.

## Completed ✅

- [x] Initial MVP v0.1.0 — TUI with SD card support (2026-04-18)
- [x] Fix ADB file/folder name parsing for names with spaces (2026-04-26)
- [x] Implement Delta Sync, Concurrent Transfer, and Thumbnail Previews (2026-04-26)
- [x] Fix: Stop transfer timer when completed or cancelled (2026-04-26)
- [x] Refactor: Clean up all 19 clippy warnings, unused imports, deprecated APIs (2026-05-27)
- [x] **Feature #4: Smart Deduplication & Integrity Checks** (2026-05-27)
  - [x] Add `xxhash-rust` crate dependency (xxh3-128, ~30 GB/s)
  - [x] Create `src/transfer/hash.rs` — streaming file hash computation
  - [x] Extend `SyncRecord` and `CompletedFile` with optional hash field
  - [x] Extend `StateManager` sync_map to store (size, hash) tuples
  - [x] Add `get_sync_hash()` method for hash-based dedup lookups
  - [x] Add `ChecksumMismatch` error variant to `AppError`
  - [x] Compute xxh3 hash after every successful file pull (single + concurrent paths)
  - [x] Track `integrity_verified` / `integrity_failed` in `TransferProgress`
  - [x] Display verified count in progress TUI and integrity stats in summary TUI
  - [x] Write 5 unit tests for hash module (all passing)
  - [x] Git commit the feature

- [x] **Remove Image Preview Feature**
  - [x] `Cargo.toml`: Remove `image` dependency
  - [x] `src/tui/thumbnail.rs`: Delete file
  - [x] `src/tui/mod.rs`: Remove `thumbnail` module declaration
  - [x] `src/app.rs`: Remove `ThumbnailCache`, `current_preview`, and fetching logic
  - [x] `src/tui/browser.rs`: Remove split layout and thumbnail rendering, expand list to 100% width
  - [x] Run `cargo clippy` and `cargo test` to verify clean removal
  - [x] Git commit the removal

- [x] **Fix Local Tree "Loading" Bug**
  - [x] `src/main.rs`: Auto-create destination directory
  - [x] `src/tui/browser.rs`: Fix misleading "Loading..." text for empty folders
  - [x] Commit fix

- [x] **Fix Search/Filter Bugs**
  - [x] `src/tui/browser.rs`: Fix Android panel empty-state ("No results" vs "Loading" vs "Empty")
  - [x] `src/app.rs`: Add `apply_search_filter()` method (replace double-toggle hack)
  - [x] `src/tui/mod.rs`: Replace double `toggle_media_filter()` with `apply_search_filter()`
  - [x] Run `cargo clippy` + `cargo test`
  - [x] Git commit

- [x] **Fix Critical Select-All Data Loss Bug** (2026-05-27)
  - [x] `src/app.rs`: Refactor `browser_select_all()` to select only visible filtered files
  - [x] Run `cargo clippy` + `cargo test`
  - [x] Commit critical fix

- [x] **Fix Context-Aware Tree Reloading (Push/Pull)** (2026-05-29)
  - [x] `src/app.rs`: Add `self.load_local_tree()` to `go_to_browser()` (Initial step)
  - [x] `src/app.rs`: Add `last_transfer_direction` field to `App` struct
  - [x] `src/app.rs`: Save transfer direction in `start_transfer()`
  - [x] `src/app.rs`: Refactor `go_to_browser()` to conditionally load remote or local tree based on direction
  - [x] Run `cargo clippy` and `cargo test`

- [x] **Fix Push Path Preservation Bug** (2026-05-29)
  - [x] `src/transfer/engine.rs`: Update worker signatures to accept `base_path`
  - [x] `src/app.rs`: Pass `tree.path` as `base_path` in `start_transfer()`
  - [x] `src/transfer/engine.rs`: Fix `TransferDirection::Push` logic to strip `base_path` to preserve original Android path
  - [x] Run `cargo clippy` and `cargo test`
  - [x] Git commit all sync-related fixes

## In Progress 🔧

## Backlog 📋

- [ ] Feature #1: Fuzzy Search & Advanced Filtering (includes Phase 2: Deep recursive search in collapsed directories)
- [ ] Feature #2: "Headless" Auto-Backup Mode (CLI)
- [ ] Feature #3: Bidirectional Sync (Push Capabilities) — partially exists
- [ ] Feature #5: Configurable Pull Profiles
- [ ] Write comprehensive unit tests for Transfer Engine & ADB client
