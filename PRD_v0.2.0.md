# Product Requirements Document (PRD) — `andpull` v0.2.0

## 1. Audit: What Is Already Implemented vs Missing

| ID | Proposed Feature | Current Code Status | Action Needed |
|:---|:---|:---|:---|
| **F1** | **Host Disk Preflight** | ❌ **Missing.** Device has `available_space` query (`src/adb/shell.rs:37`), but host destination mount point has **zero** pre-check. | **Build.** Add host disk check before pulling. |
| **F2** | **Device-Side File List Tar (`tar -T -`)** | ❌ **Missing.** `TarPuller` calls unconditional `tar cf - .` (`src/adb/client.rs:233`). Re-pulls all files. | **Build.** Pass filtered list of changed files via `tar -T -`. |
| **F3** | **Tar mtime & Timestamp Sync** | ⚠️ **Partial/Broken.** `src/transfer/tar.rs` hardcodes `0` for mtime (`103: 0, // mtime unknown`). Local files get `now()`. | **Fix.** Parse octal mtime bytes from tar headers + set local mtime via `filetime`. |
| **F4** | **SELinux / chown on Restore** | ⚠️ **Partial.** `restorecon -R /data/data` exists (`src/profile/restore.rs:258`), but **no `chown -R`** and `/sdcard/Android/media/` is untouched. | **Expand.** Add `chown` to app UID and fix external media SELinux contexts. |
| **F5** | **APK Backup & Reinstall** | ⚠️ **Stub only.** `pm path` parser exists (`src/profile/restore.rs:21`) to check if app is installed; **no APK pulling or `pm install` logic**. | **Build.** Pull base APKs during backup; auto-install on restore. |
| **F6** | **Declarative `profiles.toml`** | ❌ **Missing.** `whatsapp` and `dcim` are hardcoded in Rust (`src/profile/mod.rs`). | **Build.** Add TOML loader from `~/.config/andpull/profiles.toml`. |
| **F7** | **Dry-Run Mode (`--dry-run`)** | ❌ **Missing.** No dry-run flag or scan-only execution path. | **Build.** Add `--dry-run` CLI option. |
| **F8** | **ADB Connection Retries** | ❌ **Missing.** Commands fail immediately on broken pipe / timeout. | **Build.** Add exponential backoff retry loop (max 3) in `AdbClient`. |
| **F9** | **Stream Compression** | ❌ **Missing.** Plain uncompressed tar stream. | **Build.** Add optional `gzip`/`zstd` pipeline. |
| **F10**| **Device Checksum Verification** | ❌ **Missing.** Local has `xxh3-128`, but no device-side hash verification. | **Build.** Run remote `sha256sum` for critical db files. |

---

## 2. Prioritized Scope & Implementation Matrix

### 🔴 Sprint 1: Data Integrity & Bug Fixes (Immediate Priority)

#### P1. Tar mtime Parsing & Local Timestamp Preservation (F3)
- **Problem:** `tar.rs` hardcodes `mtime = 0` and writes files with system timestamp `now()`. Breaks delta sync and corrupts photo timeline.
- **Requirements:**
  - Parse octal bytes at offset `136..148` in standard 512-byte tar header into `u64` epoch seconds.
  - Set local file modification time using `filetime::set_file_mtime`.
  - Save parsed mtime in `SyncRecord.mtime`.

#### P2. Host Disk Space Preflight (F1)
- **Problem:** Transfer aborted with OS Error 122 (Disk quota exceeded) when `/tmp` filled up.
- **Requirements:**
  - Query destination directory filesystem available space using `fs2::available_space` or `libc::statvfs`.
  - Compare available bytes against estimated transfer size before starting.
  - Abort immediately with user-friendly error if `avail < required + 500MB headroom`.

#### P3. Fix Double-Counting in Progress & Summary
- **Problem:** Backup reported 1609 files for 796 actual files (~202%).
- **Requirements:**
  - Only increment `completed_files` and `new_files` for regular file entries (`typeflag == b'0' || typeflag == 0`).
  - Exclude directory entries (`typeflag == b'5'`) from file counts.

---

### 🟡 Sprint 2: Incremental Optimization & Reliability (Medium Priority)

#### P4. Incremental Tar Streaming (`tar -T -`) (F2)
- **Problem:** Tar fast-path streams the entire 4.7 GB DCIM folder even on repeat runs with 0 changes.
- **Requirements:**
  - When state file exists, generate a newline-separated list of modified/new relative paths.
  - Pipe list to `adb exec-out "cd '<remote>' && tar cf - -T -"` via stdin.
  - If 0 files changed, skip tar invocation entirely and report `0 new, 0 changed, N skipped` in <1s.

#### P5. Dry-Run Mode (`--dry-run`) (F7)
- **Requirements:**
  - Add `--dry-run` flag to `andpull backup` and `andpull restore`.
  - Scan remote sources, perform delta comparison against local state file, print planned transfer counts and byte volume, then exit cleanly without writing files or mutating state.

#### P6. ADB Disconnection & Worker Retries (F8)
- **Requirements:**
  - Wrap transient ADB failures (EOF, broken pipe, device offline) in a 3-attempt exponential backoff loop (`1s`, `2s`, `4s`).
  - Ping `adb get-state` before aborting a transfer thread.

---

### 🟢 Sprint 3: Advanced Features & Ecosystem (Lower Priority)

#### P7. User Profiles via `profiles.toml` (F6)
- **Requirements:**
  - Load `~/.config/andpull/profiles.toml` (fallback to `./profiles.toml`).
  - Define custom backup targets with custom sources, exclude regex patterns, and root flags.

#### P8. APK Backup & Restore (F5)
- **Requirements:**
  - Pull `base.apk` and splits from `pm path <package>` to `DEST/<profile>.apk`.
  - On restore, check if package is installed; if not, execute `pm install` before restoring app data.

#### P9. SELinux & App Ownership Remediation (F4)
- **Requirements:**
  - Query app UID via `pm list packages -U <pkg>`.
  - Execute `chown -R <uid>:<gid> /data/data/<pkg>` and `restorecon -R /data/data/<pkg>`.
  - Apply `restorecon -R /sdcard/Android/media/<pkg>` for external storage directories.

---

## 3. Execution Gate & Acceptance Criteria

1. **`cargo test`:** All existing 73 tests pass + new tests for tar mtime parsing, host disk space checks, and directory count filtering.
2. **`cargo clippy`:** `cargo clippy --all-targets -- -D warnings` exits 0.
3. **Live Device Verification (`veux`):**
   - Run 1 (Cold): Pulls 796 files, reports exactly 796 files (100%), local file mtimes match camera capture timestamps, state file has real mtimes.
   - Run 2 (Warm): Re-run finishes in <2 seconds, reporting `0 new, 0 changed, 796 skipped`.
