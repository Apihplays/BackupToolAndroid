# Incremental Delta + Workers + Tar Fast-Path Fix — Revised Plan

> **For Hermes:** Use subagent-driven-development skill to implement this plan task-by-task.

**Goal:** Fix the broken tar fast-path (silent truncated transfers, no filenames, zero delta-sync state), then add mtime-aware incremental detection, single-round-trip scanning, and configurable worker count.

**Architecture:** Three phases, strictly ordered. Phase 1 repairs the transfer core — the tar fast-path currently truncates mid-stream and buffers to a temp file before extracting, which both hides filenames and bypasses per-file delta state. Phase 2 makes re-runs incremental (mtime+size compare) and kills the 8k-file scan cost with one `find` round-trip. Phase 3 makes workers configurable. Each phase ends with a live-device verification gate before the next starts.

**Tech Stack:** Existing Rust stack, no new deps.

---

## Current Context (verified by live reproduction on veux, 2026-08-25)

**Bug reproduced:** `andpull backup --profiles dcim /tmp/dcim_test` produced a **2.79GB truncated `.andpull_temp.tar`** (actual DCIM = ~5GB; raw `adb exec-out tar` carries all 4.99GB in 2m37s at ~32MB/s), **zero extracted photos**, **0-byte state file**, and only `(tar stream)` in the progress display. Root causes:

1. **Tar fast-path truncation:** `pull_dir_tar_stream` (`client.rs`) swallows stderr (`2>/dev/null`), and `TarPuller::pull_dir` never checks `child.wait()` exit status — a device-side tar error or adb hiccup silently ends the stream early and the code treats EOF as success.
2. **Extract-after-buffer:** entire tar is buffered to `.andpull_temp.tar` first, extraction only after stream ends → truncated stream = nothing extracted, plus double disk usage.
3. **No per-file state from tar path:** engine records only `mark_dir_completed(dir)` (`engine.rs:182`) — delta sync has nothing to compare on re-runs.
4. **No filename display:** progress shows `(tar stream) <dir>` for the whole run.

**Commit ce9c3bc (Codebuff) already landed** — rooted fallbacks wired into scanner/recv/tar paths:
- `Scanner::load_children` falls back to `list_dir_rooted` when plain listing fails/empty.
- `RecvPuller::pull_file` falls back to `su -c cat` on permission-denied errors.
- `TarPuller::pull_dir` retries via rooted tar when spawn fails.
- `list_dir_rooted` now returns `Ok(vec![])` instead of Err when root unavailable.

⚠️ ce9c3bc does NOT fix the truncation/extraction/state bugs above — it fixes *access*, not *integrity*. It also introduces one new hazard: `load_children` falling back to `list_dir_rooted` on ANY error (including transient adb failures) can mask real errors as "empty directory". Phase 0 tightens this.

**Other verified facts:**
- DCIM: 796 files across `/sdcard/DCIM/{Camera,Facebook,GPS Map Camera,Screenshots,WhatsApp}`, nesting depth 5 path components (root=depth 0, files at depth ≤ 2 — well under MAX_SCAN_DEPTH=4).
- Camera dir contains many `.trashed-<ts>-IMG_*.jpg` long-named files — stress-tests GNU longname handling in the hand-rolled TarReader.
- Delta compare today: size-only (`is_unchanged`, resume.rs:153). mtime recorded but unused.
- Workers hard-coded 4 USB / 2 WiFi (`pool.rs:36-39`); each worker spawns own adb client (safe to raise).
- ProfileRunner adds GlobalBudget default max 6 on top of pool workers.

## Proposed approach

### Phase 0 — Tighten ce9c3bc's fallback semantics (small, do first)
The new blanket fallback converts real errors into empty listings. Change:
- `list_dir_rooted`: distinguish "empty because inaccessible" from "genuinely empty". Run su-listing whenever su_available AND (error OR permission-flavored); if su unavailable and plain ls errored → return the original Err (don't mask).
- `Scanner::load_children`: fall back ONLY on permission-ish errors (`is_permission_denied` helper already exists in recv.rs — move it to `src/error.rs` and share).

### Phase 1 — Fix tar fast-path integrity
Choose: **repair** vs **retire**. Decision: repair with streaming extraction, but keep it strictly better than per-file or it's dropped.

Task 1a: Check `child.wait()` status after EOF; nonzero → AppError::Transfer with stderr tail captured (remove `2>/dev/null` from tar command; pipe stderr separately). Truncated tar must NEVER be reported as Ok.
Task 1b: Stream-extract while downloading (tar entries arrive sequentially): feed the byte stream directly into TarReader instead of writing temp file. Delete `.andpull_temp.tar` mechanism entirely (also fixes double disk usage).
Task 1c: Per-file state from tar path: after extracting each entry, call `state_manager.update_sync_record(remote_path_of_entry, size, local_path, hash?)`. Requires mapping entry name → remote path (`<dir_node.path>/<entry.name>`) — pass base dir into pull_dir.
Task 1d: Progress: set `current_file` to the entry name as each header is parsed — filenames now visible during transfer (the user's actual complaint).
Task 1e: Hardening test vector: build tar bytes in-memory with GNU 'L' long-name entry (>100 chars, like `.trashed-1787713854-IMG_...jpg` names) and verify extract handles it; current code reads prefix field for UStar but GNU-L handling looks incomplete (`tar.rs:212` comment says "handle" but check implementation).
Task 1f: LIVE GATE: full DCIM backup on veux → file count extracted == 796, sizes match `du`, rerun shows all skipped. If streaming extraction proves flaky on 800-file tars, fallback decision: disable fast-path entirely (per-file pool path is proven correct and feeds state properly) behind `--tar` opt-in flag.

### Phase 2 — True incremental detection
Task 2a: `SyncRecord.mtime: u64` (`#[serde(default)]`); `is_unchanged_strict(path,size,mtime)`; stored-mtime==0 treated as wildcard (back-compat: old records match on size alone, no mass re-pull).
Task 2b: Fast scan: `AdbClient::find_files(root, rooted)` — ONE round-trip. First verify on veux: `adb shell 'toybox find ... -printf'` support (gate everything on this probe). Fallback: `find <root> -type f` + `stat -c '%s|%Y|%n'` pipeline. Parser unit-tested offline. ProfileRunner switches to flat find-scan (no FileNode tree on backup path).
Task 2c: Surface counters: TransferProgress + ProfileOutcome gain `new_files`/`changed_files`; CLI prints `[ok] whatsapp: 12 new, 3 changed, 7984 skipped`; TUI summary shows same.

### Phase 3 — Worker configuration
Task 3a: `WorkerPool::auto_detect(device, override: Option<usize>)`; USB default 4→8; parser tests for `--workers N` (reject 0/non-numeric; cap sanity warn >16 on WiFi).
Task 3b: Plumb override CLI→main→runner→engine (store at engine construction). Raise GlobalBudget default to `max(16, profiles×workers)`.
Task 3c: TUI: `+`/`-` on ProfileSelect cycles Auto→4→8→12→16→Auto, footer hint updated.
Task 3d: README update + `cargo install --path .` refresh.

## Files likely to change
- Phase 0: `src/error.rs`, `src/adb/client.rs`, `src/scanner/tree.rs`
- Phase 1: `src/transfer/tar.rs`, `src/transfer/engine.rs`, `src/state/resume.rs` (record API)
- Phase 2: `src/adb/client.rs`, `src/profile/runner.rs`, `src/transfer/engine.rs`, `src/tui/{summary,progress}.rs`
- Phase 3: `src/transfer/pool.rs`, `src/cli.rs`, `src/main.rs`, `src/app.rs`, `src/tui/profile.rs`, `README.md`

## Tests / validation
- Offline unit tests per task (parsers, strict-delta matrix, GNU-longname tar fixture, --workers parsing).
- Gates: `cargo test`, `cargo clippy --all-targets -- -D warnings`, release build.
- Live gates on veux (75454e789f14):
  - Phase 1: DCIM backup extracts exactly 796 files (~4.65GB user-visible; note du says 4882953KB ≈ 4.66GiB incl. trashed files), NO `.andpull_temp.tar` left behind, state json populated with per-file records, filenames visible in progress, second run = all skipped.
  - Phase 2: add 1 photo → rerun pulls exactly 1, wall time seconds not minutes.
  - Phase 3: `--workers 12` shows active_workers=12, no adb errors under load.

## Risks / tradeoffs / open questions
- **Streaming extraction couples network errors to partial files** — mitigate with size verification per entry (header size vs written bytes) before marking synced.
- **toybox find -printf may not exist** on veux's ROM — stat-pipeline fallback planned; probe before coding (Phase 2 gate).
- **Retiring the tar path is on the table**: if 1f gate fails twice, drop fast-path (opt-in flag) rather than keep patching — per-file path is slower (~796 sequential-ish pulls) but correct, parallelizable, and delta-native. Correctness > speed for backups.
- **ce9c3bc's empty-vs-error masking** could hide WhatsApp dirs that genuinely moved; Phase 0 closes it.
- mtime granularity on sdcardfs (2s) acceptable: photos immutable post-capture; msgstore db has date-stamped filenames so new-file detection covers it.
