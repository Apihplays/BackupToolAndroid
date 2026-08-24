# Clean-Flash Backup Profiles Implementation Plan

> **For Hermes:** Use subagent-driven-development skill to implement this plan task-by-task.

**Goal:** Add named backup profiles (priority-ordered: WhatsApp first, DCIM second) with parallel profile execution and a working restore path, so Apih can clean-flash any ROM on his veux and get chats + photos back in minutes.

**Architecture:** Layer a `profile` module on top of the existing TransferEngine. A profile = ordered list of source specs (device path + filters) + destination layout + optional root requirement. A ProfileRunner executes independent profiles concurrently by spawning one TransferEngine per profile (each already runs its own thread pool against distinct file sets — no shared mutable state beyond progress handles). Restore reuses the same engine in Push direction with pre-flight validation.

**Tech Stack:** Existing Rust stack only (ratatui/crossterm TUI, serde_json state, xxh3, glob, anyhow). No new dependencies except maybe `libc` for permission checks if root-path probing needs it (avoid if possible).

---

## Current Context (verified from repo)

- Repo: `/home/hayyan/Desktop/Code/BackupToolAndroid`, branch `main`, clean.
- `src/transfer/engine.rs` (520 lines): TransferEngine with Pull/Push enum, WorkerPool, progress Arc<Mutex>. Push exists as an enum variant but the push code path is thinner than pull — verify before relying on it.
- `src/state/resume.rs`: StateManager with `.andpull-state.json` per destination — resume + delta sync already work.
- `src/scanner/tree.rs`: FileNode tree built from `adb ls` via AdbClient; MEDIA_EXTENSIONS filter exists.
- `src/app.rs`: TUI state machine (DeviceSelect → FileBrowser → Transferring → Summary).
- `src/adb/client.rs` (378 lines): AdbClient wrapping adb shell/exec-out.
- 0 tests. 19 clippy warnings. `cargo check` passes.
- Device target: Xiaomi veux (PixelOS/custom ROMs), rooted via KernelSU — `su` available in adb shell.

## Key domain facts (do not get these wrong)

1. Modern WhatsApp stores everything under `/sdcard/Android/media/com.whatsapp/WhatsApp/`:
   - `Media/` (photos/videos/documents)
   - `Backups/` (chat export zips)
   - Legacy installs may still use `/sdcard/WhatsApp/` — check BOTH paths.
   - `Databases/msgstore.db.crypt15` lives under the app-media dir on recent versions; it is only useful together with the account's encryption key (stored in the app's private data / Google Drive end-to-end key). Plain file restore of crypt15 without the key does NOT restore chats into a fresh install unless using local-backup restore flow where key derives from the account. Plan assumes: back up the whole `com.whatsapp/` media tree + `/data/data/com.whatsapp` ONLY as best-effort extra via root.
2. `/sdcard/Android/data/*` and `/sdcard/Android/media/*` are NOT listable via plain `adb ls` on Android 11+ shell user — **root required**: `su -c 'ls ...'`. This is why KernelSU matters. All profile sources must go through a root-capable listing/pull path (`exec-out su -c 'cat file'` per file, or tar over su).
3. DCIM is at `/sdcard/DCIM/` — world-readable, no root needed, biggest byte volume → run it as a background profile while WhatsApp finishes first (priority ordering).

## Proposed approach

### New module: `src/profile/mod.rs`
```rust
pub struct ProfileSpec {
    pub name: String,            // "whatsapp", "dcim"
    pub priority: u8,            // lower = starts first
    pub requires_root: bool,
    pub sources: Vec<SourceSpec>,
}

pub struct SourceSpec {
    pub device_path: String,     // "/sdcard/Android/media/com.whatsapp"
    pub alt_paths: Vec<String>,  // legacy fallbacks
    pub recursive: bool,
    pub extensions: Option<Vec<String>>, // None = all
}
```
Built-in defaults:
- `whatsapp` (priority 0, root=true): `Android/media/com.whatsapp` (+ fallback `/sdcard/WhatsApp`) and best-effort `su -c tar -c /data/data/com.whatsapp` streamed to a single `whatsapp_appdata.tar`.
- `dcim` (priority 1, root=false): `/sdcard/DCIM`.

### Parallel execution
- `ProfileRunner::run_all(specs, client, dest)` spawns one OS thread per profile; each owns its own TransferEngine instance and StateManager (state files namespaced: `<dest>/.andpull-state.<profile>.json`).
- Priority honored by staggering thread start (whatsapp immediately, dcim after 2s delay OR when whatsapp's metadata scan completes) so USB bandwidth goes to critical data first; they then share the pipe naturally since each profile's internal WorkerPool is sized by existing auto-detect logic. Cap total concurrency: global semaphore of N workers across pools (add `Arc<Semaphore>` — implement with `Mutex<usize>` + condvar to avoid new deps).
- Progress: extend `TransferProgress` display to aggregate per-profile rows in the TUI Transferring view (one gauge row per profile).

### Restore
- `RestoreRunner` iterates profiles in reverse priority: push `DCIM` back to `/sdcard/DCIM/`, push WhatsApp media tree back to `Android/media/com.whatsapp/`, then (root, opt-in flag) untar appdata via `su -c 'tar -x -C /data/data' && restorecon -R /data/data/com.whatsapp`. Pre-flight checks: package installed (`pm path com.whatsapp`), enough space (`df /sdcard`), device not in Doze.
- SELinux/ownership warning printed for the appdata step; it is best-effort and skipped unless `--with-appdata`.

## Step-by-step plan

### Task 1: Baseline hygiene (pre-req, keeps later diffs clean)
**Files:** various (lint only).
1. Run `cargo clippy --fix --allow-dirty` and manually clear remaining warnings (see implementation_plan.md list in repo).
2. Run `cargo build --release` — expect success.
3. Commit: `chore: clear clippy warnings`.

### Task 2: Test harness setup
**Files:** Create `tests/common/mod.rs`; add `[dev-dependencies] tempfile = "3"` to `Cargo.toml`.
1. Add dev-dependency.
2. Write a trivial smoke test that constructs `FileNode::root("/sdcard")` and asserts `compute_totals` on a hand-built tree.
3. Run `cargo test` — expect PASS. Commit: `test: add harness and tree totals test`.

### Task 3: Root-capable remote listing
**Files:** Modify `src/adb/client.rs`, `src/adb/shell.rs`.
1. Failing test: unit test for new `list_dir_rooted(path)` output parser feeding it canned `ls -A --authorless`-style / `find` output strings.
2. Implement `AdbClient::su_available() -> bool` (run `su -c id`, check uid=0).
3. Implement `AdbClient::list_dir(path)` wrapper: try plain `ls`; on permission error and su available, retry via `su -c 'ls -1ap <path>'` parsing trailing-`/` dirs.
4. Tests pass. Commit: `feat(adb): root-aware directory listing with fallback`.

### Task 4: Root-capable pull (per-file cat streaming)
**Files:** Modify `src/transfer/recv.rs`, `src/adb/client.rs`.
1. Failing test: parser test for `su -c wc -c <file>` size probe.
2. Implement `RecvPuller::pull_rooted(remote, local)`: stream `exec-out su -c 'cat "<remote>"'` to file, verify size matches probe (xxh3 verify stays as-is afterwards).
3. Escape/quoting audit: all paths single-quote-wrapped with `'` doubled (paths with spaces/apostrophes in WhatsApp media are common).
4. Tests pass. Commit: `feat(transfer): rooted pull via su cat streaming`.

### Task 5: Profile module + builtin specs
**Files:** Create `src/profile/mod.rs` (specs + `builtin_profiles()`); modify `src/main.rs` (`mod profile;`).
1. Failing test: `builtin_profiles()` returns 2 profiles, whatsapp priority < dcim priority, whatsapp requires_root == true; SourceSpec paths match the domain facts above.
2. Implement structs + builders.
3. Tests pass. Commit: `feat(profile): ProfileSpec model with whatsapp/dcim builtins`.

### Task 6: ProfileRunner — sequential first, then parallel
**Files:** Create `src/profile/runner.rs`; modify `src/transfer/engine.rs` (expose per-engine construction), `src/state/resume.rs` (namespaced state filename param).
1. Refactor: `StateManager::new` gains explicit state-file-name parameter (default preserves old name — no behavior change).
2. Failing test: runner with two fake profiles (temp-dir "device" simulated by pointing engine at local scanner? — NO, keep it simple: test the scheduling logic extracted into `plan_execution(profiles) -> Vec<(profile, start_delay)>` pure function; integration with real adb is manual).
3. Implement sequential run honoring priority; verify with real device manually.
4. Add parallel spawn: one thread per profile, shared global worker budget via `Mutex<usize>`+condvar semaphore.
5. Commit: `feat(profile): ProfileRunner with priority-ordered parallel execution`.

### Task 7: Appdata tar best-effort backup
**Files:** Create `src/profile/appdata.rs`.
1. Implement `backup_appdata(client, pkg, out_tar)`: `exec-out su -c "tar -cf - -C /data/data com.whatsapp"` streamed to file; tolerate nonzero exit (record warning, never fail whole run).
2. Unit test: tar stream writer truncation handling with canned bytes.
3. Commit: `feat(profile): best-effort app-data tar via root`.

### Task 8: Restore path
**Files:** Create `src/profile/restore.rs`; modify `src/transfer/engine.rs` (ensure Push path complete — audit `execute()` Push branch; fix gaps found).
1. Audit Push: write failing test for push file selection mapping (local→remote path reconstruction).
2. Complete push implementation if thin.
3. Implement `RestoreRunner::run_all(profiles, dest)`: reverse-priority order, pre-flight (pkg installed, free space via `df`), media pushes, opt-in appdata untar + `restorecon -R` via su.
4. Commit: `feat(profile): restore runner with preflight validation`.

### Task 9: TUI integration
**Files:** Modify `src/app.rs`, `src/tui/progress.rs`, `src/tui/mod.rs`, `src/tui/widgets.rs`.
1. New AppView::ProfileSelect listing builtin profiles with checkboxes (space toggles, `r` toggles restore mode, enter starts).
2. Transferring view renders one progress line per active profile (name, %, speed, ETA) plus aggregate.
3. Summary view groups results per profile.
4. Manual verification on device. Commit: `feat(tui): profile select and per-profile progress views`.

### Task 10: CLI non-interactive mode (rom-flash-day ergonomics)
**Files:** Modify `src/main.rs`.
1. `andpull backup [--profiles whatsapp,dcim] [--with-appdata] <dest>` and `andpull restore [...]` skip TUI entirely — flash day you want one command.
2. Commit: `feat(cli): non-interactive backup/restore subcommands`.

## Files likely to change
- Modify: `Cargo.toml`, `src/main.rs`, `src/app.rs`, `src/adb/client.rs`, `src/adb/shell.rs`, `src/transfer/{engine.rs,recv.rs}`, `src/state/resume.rs`, `src/tui/{mod.rs,progress.rs,widgets.rs}`
- Create: `src/profile/{mod.rs,runner.rs,appdata.rs,restore.rs}`, `tests/`

## Tests / validation
- `cargo test` after every task (parser/scheduler tests are offline — no device needed).
- `cargo clippy` — zero warnings gate.
- On-device (veux, KernelSU): 
  1. Backup run → confirm `.andpull-state.whatsapp.json` + `Android_media_com.whatsapp/` + `DCIM/` populated; spot-check msgstore db present.
  2. Kill mid-run, rerun → delta skips prior files.
  3. Wipe-profile restore into fresh ROM install → WhatsApp sees local backup on first launch ("local backup found"), DCIM intact in gallery.

## Risks & tradeoffs
- **crypt15 chat restore is account-key-dependent.** File backup alone may not restore chats without the WhatsApp account's 64-digit encryption key / Google Drive E2E key. Mitigation: also instruct user to enable WhatsApp's own local backup (which the profile backs up) AND record the key offline. Be explicit in README; do not oversell.
- **Parallel over one USB link:** total bandwidth is fixed; parallelism buys latency-hiding and priority overlap, not throughput. Expect combined speed ≈ single-stream speed ±10%.
- **su tar of /data/data can be large & fragile** (sockets, lib dirs) — hence best-effort flag only, never blocks the critical path.
- **Push path may be incomplete** in current engine (Task 8 audit may surface real work).
- Open questions: (a) Should profiles live in a TOML config for custom additions, or are two builtins enough? (b) Wireless-adb support for restore (slower, worker cap already exists)?

## Honest note on Rust
No decision needed — the repo is already Rust and the right call. Rewriting in anything else would be pure waste; the existing engine (worker pool, tar streaming, xxh3 dedup) is exactly what this feature needs underneath.
