# Incremental Delta Detection + Configurable Worker Count Plan

> **For Hermes:** Use subagent-driven-development skill to implement this plan task-by-task.

**Goal:** Make re-runs pull ONLY new/changed files (not re-scan 8k+ files into the job queue), and make worker count user-configurable instead of hard-coded 4 (USB) / 2 (WiFi).

**Architecture:** Two independent workstreams. (1) Delta detection already exists (`StateManager::is_unchanged` size-compare at `src/state/resume.rs:153`, pre-filter in `engine.rs:280-330`) but has two weaknesses: it compares by size only (a same-size edit slips through), and the 8k-file *listing* still happens every run — fix listing cost with a fast `find`-based scan + mtime-based pruning, and strengthen change detection to size+mtime. (2) Worker count comes from `WorkerPool::auto_detect()` hard-coding 4/2 at `src/transfer/pool.rs:36-39` — thread a `--workers N` flag through CLI, profile runner, and TUI, and raise USB default to 8 (adb handles concurrent exec-out streams fine; each worker spawns its own adb client per `pool.rs:76-77`).

**Tech Stack:** Existing Rust stack. No new dependencies.

---

## Current Context (verified)

- **Delta sync exists but size-only:** `is_unchanged(path, remote_size)` compares only stored size vs remote size (`resume.rs:153-160`). mtime is recorded on files (`FileJob.mtime`, `FileNode.mtime`) but NOT used for comparison.
- **Pre-filter exists:** `engine.rs` lines ~280-330 skip unchanged files before dispatching to workers, counting into `delta_skipped`. So "only pull new files" ALREADY works after the first backup — provided the `.andpull-state.<profile>.json` state file survives next to the destination.
- **The 8k-file cost is the SCAN, not the pulls:** every run walks the full tree via `list_dir`/`list_dir_rooted` per directory (`scanner/tree.rs` recursion). With 8k files that's thousands of adb round-trips before any transfer decision. This is the real bottleneck to kill.
- **Workers hard-coded:** `pool.rs:36-39` `auto_detect` → 4 (usb) / 2 (wifi). `active_workers` is u8 in TransferProgress. ProfileRunner adds its own GlobalBudget (default max 6) on top — total concurrency = min(budget, sum of pool workers).
- CLI parser is pure/testable in `src/cli.rs`; profiles flow: `cli.rs`/TUI → `ProfileRunner` → `TransferEngine` → `WorkerPool`.

## Proposed approach

### Workstream A: True incremental detection
1. **Fast scan via single `find`:** add `AdbClient::find_files(root, rooted: bool)` running one shell command:
   - non-root: `exec-out find '<root>' -type f -printf '%s\t%T@\t%p\n' 2>/dev/null` (toybox find supports -printf? VERIFY on device; fallback: `find ... -type f | xargs stat -c '%s|%Y|%n'`)
   - root: same via `su -c`
   Parse into `Vec<RemoteEntry>` (name from path tail, path absolute). One round-trip replaces thousands. Unit-test the parser offline with canned output.
2. **mtime+size delta check:** extend `is_unchanged` → `is_unchanged_strict(path, size, mtime)`. SyncRecord gains `mtime: u64` field (`#[serde(default)]` so old state files keep loading). Change = size differs OR mtime differs (mtime newer than synced record).
3. **Prune-before-scan optimization:** since we have last-sync's flat file list, the tree walk can be skipped entirely for profiles whose source roots are unchanged... too clever; YAGNI. Instead: keep the tree build for TUI browsing, but the PROFILE RUNNER path uses `find_files` directly (flat list) and never builds FileNode trees. That alone removes most of the 8k-entry overhead.
4. **"Only newest" summary line:** after pre-filter, log/print `X new, Y changed, Z unchanged-skipped` per profile (data already in progress counters; just surface in ProfileOutcome — add `new_files`/`changed_files` fields).

### Workstream B: Worker count control
1. `WorkerPool::auto_detect(device, override: Option<usize>)` — override wins; else usb=8, wifi=2 (bump USB default; 8 parallel exec-out streams are safe, each worker owns its own adb process).
2. `TransferProgress.active_workers` is u8 — bump to usize or cap workers at 255 (fine).
3. Plumb through:
   - CLI: `andpull backup --workers 12 ...` (parser unit tests first)
   - `ProfileRunner::run_all/spawn_all(..., workers: Option<usize>)` → passes to engine → pool
   - GlobalBudget default raised to match (max(16, workers*profiles)) so budget doesn't strangle higher worker counts
   - TUI: `+`/`-` keys on ProfileSelect view adjust worker count shown in footer (default auto)
4. Sanity guard: warn (not fail) if workers > 16 over WiFi.

## Step-by-step plan

### Task 1: SyncRecord mtime + strict compare
**Files:** Modify `src/state/resume.rs`.
1. Failing tests: `is_unchanged_strict` returns false when mtime newer; true when size+mtime equal; old records without mtime (`serde(default)=0`) behave as changed-if-size-matches-only (back-compat: treat stored mtime 0 as wildcard-match on size alone to avoid re-pulling everything once).
2. Add `#[serde(default)] pub mtime: u64` to `SyncRecord`; update `update_sync_record` signature to take mtime; update callers in `pool.rs:146-151` (job.mtime available there) and engine.
3. Keep `is_unchanged` as delegate to strict-with-wildcard for compatibility.
4. `cargo test` green. Commit: `feat(state): mtime-aware delta detection with back-compat`.

### Task 2: Fast flat scan via find
**Files:** Modify `src/adb/client.rs`, `src/profile/runner.rs`.
1. Failing unit tests: `parse_find_output(&str, root) -> Vec<RemoteEntry>` handling tab-separated `%s\t%T@\t%p` lines, spaces/apostrophes in names, empty output, garbage lines skipped.
2. Verify on device FIRST (manual, read-only): `adb shell 'toybox find /sdcard/DCIM -type f -printf "%s\t%T@\t%p\n" | head -3'` — if toybox lacks -printf, fall back to `stat -c` pipeline; document which works on veux in code comment.
3. Implement `AdbClient::find_files(root:&str, rooted:bool)` using `quote_shell`; parse via tested fn.
4. Runner: replace recursive tree-building with `find_files` per source (+alt_paths existence probe); extension filter applied on flat list; feed jobs straight to engine via a new thin `TransferEngine::execute_flat(jobs, ...)` OR construct synthetic FileNode leaves under a virtual root — choose whichever touches less code after reading `execute()` fully; document choice.
5. Tests + clippy green. Commit: `feat(scan): single-round-trip find-based file scan for profiles`.

### Task 3: New/changed counters surfaced
**Files:** Modify `src/transfer/engine.rs`, `src/profile/runner.rs`, `src/tui/summary.rs`, `src/tui/progress.rs`.
1. Add `pub new_files: u64, pub changed_files: u64` to TransferProgress (increment in pre-filter where delta_skipped is counted: file not in sync_map = new, mtime/size diff = changed).
2. ProfileOutcome gains same fields; copy out of final progress in runner.
3. Summary view + CLI `[ok] whatsapp: 12 new, 3 changed, 7990 skipped` line format.
4. Commit: `feat(ui): surface new/changed/skipped counts per profile`.

### Task 4: Worker count plumbing
**Files:** Modify `src/transfer/pool.rs`, `src/cli.rs`, `src/main.rs`, `src/profile/{mod.rs,runner.rs}`, `src/app.rs`, `src/tui/profile.rs`.
1. Failing parser tests: `--workers 12`, `--workers abc` → Error, absent → None.
2. `WorkerPool::auto_detect(device, override)`; raise USB default 4→8.
3. Thread override: main→runner→engine→pool. Engine stores `workers_override: Option<usize>` set at construction (avoid touching every execute() call site).
4. GlobalBudget default: `DEFAULT_MAX_WORKERS` becomes `max(16, profiles×workers)`.
5. TUI: `+`/`-` on ProfileSelect cycles override (Auto→4→8→12→16→Auto), shown in footer; store on App.
6. Commit: `feat(pool): configurable worker count, USB default raised to 8`.

### Task 5: Docs + install refresh
**Files:** Modify `README.md` (keybindings table + `--workers` flag + incremental behavior section).
1. Update README honestly: describe size+mtime delta, find-based scan, worker defaults.
2. Reinstall binary: `cargo install --path .`
3. Commit: `docs: document incremental sync and worker configuration`.

## Files likely to change
`src/state/resume.rs`, `src/transfer/engine.rs`, `src/transfer/pool.rs`, `src/adb/client.rs`, `src/profile/{runner.rs,mod.rs}`, `src/cli.rs`, `src/main.rs`, `src/app.rs`, `src/tui/{profile.rs,summary.rs,progress.rs}`, `README.md`

## Tests / validation
- Offline unit tests: find-output parser, strict-delta matrix (same/diff size × same/diff/wildcard mtime), CLI --workers parsing.
- `cargo clippy --all-targets -- -D warnings` gate stays clean; `cargo install --path .` refresh.
- On-device (veux): 
  1. First backup of DCIM → note counts. 
  2. Immediately rerun → expect near-zero transfers, summary shows ~all skipped, wall time dominated by single find (~seconds not minutes).
  3. Add one photo, rerun → exactly 1 new pulled.
  4. `andpull backup --workers 12` → progress shows active_workers 12, no adb errors.

## Risks / tradeoffs / open questions
- **toybox find -printf availability** varies by ROM; fallback stat-pipeline costs one extra process spawn but still one round-trip. Must verify on veux before coding parser format (Task 2 step 2 gates everything).
- **mtime granularity:** FAT/exFAT sdcardfs can have 2s timestamp granularity → same-second edits could be missed. Acceptable for photos/videos (immutable once taken); WhatsApp msgstore db changes constantly BUT its filename embeds a date, so new-file detection catches it. Note in docs.
- **State file loss = full rescan/re-pull** (unchanged behavior; document that dest dir must persist across rom flashes — that's the whole point of backing up to PC).
- **Workers > ~10 over MTP-less exec-out**: untested ceiling; each worker is an independent adb process so risk is low, but watch device-side adbd CPU. Guard: warn above 16.
- Open question: should `--workers` apply per-profile or globally? Plan says globally (per-profile pools already share GlobalBudget); revisit if profiling shows starvation of dcim while whatsapp saturates.
