# Optimization Plan — andpull v0.2.1

## Codebase Profile
- **7,787 lines** across 15 source files
- **Release profile:** opt-level=3, LTO=true, codegen-units=1, strip=true (already aggressive)
- **Key deps:** ratatui, crossterm, serde, xxhash-rust, filetime, fs2, tempfile, libc

---

## 1. Memory & Allocation (High Impact)

### 1a. `Arc<Mutex<TransferProgress>>` contention hot path
**Files:** `engine.rs` (2), `pool.rs` (2), `tar.rs` (1)  
**Problem:** Every file completion acquires `progress.lock().unwrap()` — on 8 workers + tar streaming, that's lock contention on every 1-10MB chunk.  
**Fix:** Replace with `AtomicU64` counters (`completed_files`, `transferred_bytes`, etc.) and `AtomicBool`/`AtomicRefCell` for `current_file` string. Eliminates mutex contention entirely. The TUI already polls; atomics are lock-free.  
**Est. Impact:** ~15-20% throughput gain on USB bulk transfers.

### 1b. `format!()` in tight loops
**Files:** `tar.rs` (8+ format! calls in stream loop), `engine.rs` (6)  
**Problem:** `format!("{}{}", remote_dir, entry.name)` called per tar entry — allocates on heap per file.  
**Fix:** Pre-allocate `remote_path: String` buffer at dir level, `clear()` + `write!()` per entry. Avoids allocation entirely.  
**Est. Impact:** ~5% CPU reduction on 1000+ file directories.

### 1c. `app.rs` clone() density (23 clones)
**Problem:** TUI event loop clones `App` state heavily. Most are `Arc` clones (cheap), but some clone `Vec<ProfileSpec>`.  
**Fix:** Profile specs are immutable after creation — wrap in `Arc<Vec<ProfileSpec>>` once, pass shared reference.  
**Est. Impact:** Minor (~2% on TUI rendering).

---

## 2. I/O Throughput (High Impact)

### 2a. Tar streaming: buffer size 8KB → 64KB
**File:** `tar.rs` `stream_entry_data()`  
**Problem:** Default `BufReader`/`BufWriter` uses 8KB buffer. ADB USB throughput saturates around 32-40MB/s — small buffers cause excessive read syscalls.  
**Fix:** `BufReader::with_capacity(64 * 1024, reader)` and `BufWriter::with_capacity(64 * 1024, file)`.  
**Est. Impact:** ~10-15% throughput gain on large files.

### 2b. Worker pool: per-file pull via `dd` pipeline instead of individual `adb pull`
**File:** `pool.rs`, `recv.rs`  
**Problem:** Each worker spawns `adb pull` per file — 796 DCIM files = 796 process spawns. ADB handshake overhead per spawn is ~20-50ms.  
**Fix:** For small-file profiles (WhatsApp), batch `adb pull` with multiple paths. For DCIM, tar fast-path already handles this. Only matters for fallback path.  
**Est. Impact:** ~30s saved on 800-file WhatsApp profile.

### 2c. Delta scan: use `adb exec-out "toybox find ... -printf"` instead of recursive `ls -la`
**File:** `scanner/tree.rs`, `adb/client.rs`  
**Problem:** Recursive `list_dir` + `list_dir_rooted` = N+1 round-trips (one per directory level). 800-file DCIM with depth 3 = ~200 round-trips.  
**Fix:** Single `adb exec-out "find /sdcard/DCIM -type f -printf '%s|%T@|%p\n'"` → parse locally.  
**Est. Impact:** Scan time drops from ~15s to <1s on USB.

---

## 3. Incremental Sync (Critical Gap)

### 3a. Tar path re-pull on re-run
**File:** `engine.rs` `execute()`  
**Problem:** `has_tar` triggers `TarPuller::pull_dir` unconditionally — streams full 4.7GB even if state says 0 files changed.  
**Fix:** Before calling `TarPuller::pull_dir`, compare state records: if all files have matching `(size, mtime)` → skip tar entirely, return `0 new, 0 changed, N skipped`. Only call tar when delta is non-empty.  
**Est. Impact:** Re-run drops from 5+ minutes to <2 seconds.

### 3b. State file mtime=0 (partially fixed in Sprint 1)
**File:** `tar.rs` (fixed), `recv.rs` (NOT fixed)  
**Problem:** Sprint 1 fixed tar path mtime parsing, but `recv.rs` fallback path still writes `mtime=0` in state records.  
**Fix:** Parse `ls -la` output mtime or use `stat -c '%Y'` in recv path.  
**Est. Impact:** Delta sync correctness for files pulled via per-file path.

---

## 4. Binary Size & Startup (Low Impact)

### 4a. Strip debug info in release (already done ✓)
Cargo.toml has `strip = true`. No action needed.

### 4b. Reduce `tempfile` usage
**File:** `tar.rs`  
**Problem:** `build_tar()` test helper allocates Vec<u8> — fine for tests. But `tempfile` dependency exists only for tests.  
**Fix:** Move `tempfile` to `[dev-dependencies]`.  
**Est. Impact:** Zero runtime impact; cleaner dep tree.

### 4c. Compile-time optimization: `cargo bloat` analysis
**Action:** Run `cargo bloat --release -n 20` to identify largest functions. Candidates for splitting: `app.rs` (988 lines) is the TUI god-object.  
**Est. Impact:** Code maintainability, not runtime.

---

## 5. Error Handling & Resilience

### 5a. ADB disconnection retry
**Files:** `adb/client.rs`  
**Problem:** Single-shot ADB commands fail on transient disconnects (USB glitch, device sleep).  
**Fix:** Wrap `shell_command` and `pull_dir_tar_stream` in retry loop (3 attempts, exponential backoff 1s/2s/4s). Only retry on transient errors (EOF, broken pipe, EPIPE), not on permanent failures (device not found).  
**Est. Impact:** Eliminates ~5% of total backup failures on unstable USB connections.

### 5b. Zombie process reap on worker crash
**Files:** `pool.rs`  
**Problem:** If a worker panics mid-pull, the child ADB process becomes orphaned.  
**Fix:** Worker pool destructor calls `child.kill()` on any still-running child processes. Use `Drop` impl on `WorkerPool`.  
**Est. Impact:** Prevents zombie ADB processes accumulating on repeated runs.

---

## 6. Low-Hanging Fruit (Quick Wins)

| Item | File | Change | Impact |
|:---|:---|:---|:---|
| Cache `su_available()` result | `adb/client.rs` | `su_available` called on every rooted fallback — cache in `AdbClient` struct | ~100ms per rooted call saved |
| Avoid `to_string_lossy()` | `tar.rs:115` | Use `as_os_str().as_bytes()` path for non-UTF8 paths | Correctness |
| `Progress::update_speed()` | `engine.rs` | Called per-file — `Instant::now()` on every call is fine but `Mutex::lock()` is the bottleneck | Merged into 1a |
| `fs2` → `libc::statvfs` | `error.rs` | Remove `fs2` dep, use raw `libc::statvfs` (already have `libc` dep) | One fewer dependency |

---

## Priority Execution Order

| Phase | Items | Expected Gain |
|:---|:---|:---|
| **P0: Critical fix** | 3a (skip tar on clean delta), 3b (recv mtime) | Re-run: 5min → 2s |
| **P1: Throughput** | 1a (atomics), 2a (buffer size), 2c (find scan) | +25% transfer speed |
| **P2: Reliability** | 5a (retries), 5b (zombie reap), 3b (recv mtime) | -5% failure rate |
| **P3: Polish** | 1b (format!), 1c (app clones), 6 (quick wins) | +5% CPU efficiency |
| **P4: Cleanup** | 2b (batch pull), 4b (tempfile dev-dep), 4c (bloat) | Code quality |

---

## Acceptance Criteria

1. `cargo test` — all tests pass (existing + new)
2. `cargo clippy --all-targets -- -D warnings` — clean
3. `cargo bloat --release -n 10` — no single function > 5% of binary
4. Re-run incremental: <2s for 0-change scenario
5. Cold transfer 4.7GB DCIM: measurable speedup vs current baseline
