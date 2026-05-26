# andpull — Task Tracker

> This file lives in the project so we can always look back on what we've done and what's next.

## Completed ✅

- [x] Initial MVP v0.1.0 — TUI with SD card support (2026-04-18)
- [x] Fix ADB file/folder name parsing for names with spaces (2026-04-26)
- [x] Implement Delta Sync, Concurrent Transfer, and Thumbnail Previews (2026-04-26)
- [x] Fix: Stop transfer timer when completed or cancelled (2026-04-26)
- [x] Refactor: Clean up all 19 clippy warnings, unused imports, deprecated APIs (2026-05-27)

## In Progress 🔧

- [/] **Feature #4: Smart Deduplication & Integrity Checks**
  - [ ] Add `xxhash` crate dependency for fast hashing
  - [ ] Extend `SyncRecord` to store file hash alongside size
  - [ ] Extend `StateManager` with hash-based `is_unchanged()` comparison
  - [ ] Add post-transfer integrity verification (hash local file vs device file)
  - [ ] Add `ChecksumMismatch` error variant to `AppError`
  - [ ] Update `TransferProgress` to track integrity-verified count
  - [ ] Update TUI progress/summary views to show verification stats
  - [ ] Add `--verify` CLI flag for optional post-pull verification
  - [ ] Write unit tests for hash computation and comparison logic
  - [ ] Git commit the feature

## Backlog 📋

- [ ] Feature #1: Fuzzy Search & Advanced Filtering
- [ ] Feature #2: "Headless" Auto-Backup Mode (CLI)
- [ ] Feature #3: Bidirectional Sync (Push Capabilities) — partially exists
- [ ] Feature #5: Configurable Pull Profiles
- [ ] Write comprehensive unit tests for Transfer Engine & ADB client
