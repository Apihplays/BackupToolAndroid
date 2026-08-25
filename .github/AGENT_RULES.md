# Agent Rules — Preventing Device-Path Build Leaks

## Background

On 2026-08-25, 2,205 cargo build artifacts (`.o` files, fingerprints, build-script outputs)
appeared in `/sdcard/Android/media/com.whatsapp/` on the test device. The mechanism was
traced to an unlogged `CARGO_TARGET_DIR` environment variable in the AI agent's execution
sandbox. These rules prevent recurrence.

## Rules

### 1. NEVER set CARGO_TARGET_DIR outside the repository

```bash
# BAD — leaks build artifacts onto the phone
CARGO_TARGET_DIR=/sdcard/Android/media/com.whatsapp cargo build

# GOOD — default (target/ in repo root)
cargo build
```

### 2. Never run cargo with CWD on removable/device paths

```bash
# BAD
cd /sdcard/DCIM && cargo test

# GOOD
cd /path/to/BackupToolAndroid && cargo test
```

### 3. Device testing only via documented andpull commands

```bash
# BAD — ad-hoc adb push/pull with cargo artifacts nearby
adb push target/debug/andpull /sdcard/

# GOOD — use andpull's own backup/restore with a local destination
cargo run --release -- backup --profiles dcim ./test_output/
```

### 4. Before any git commit, verify no unexpected files

```bash
git status  # Check for stray files before committing
git diff --stat  # Review what's actually changing
```

### 5. Run the environment guard before cargo builds

```bash
source scripts/env-guard.sh  # Fails if env vars point at device paths
cargo test
```

## Quick Reference

| Environment Variable | Safe Value | Unsafe Value |
|---|---|---|
| `CARGO_TARGET_DIR` | unset or `./target` | `/sdcard/...` |
| `PWD` | `/home/*/BackupToolAndroid` | `/sdcard/...` |
| `TMPDIR` | `/tmp` | `/sdcard/...` |

If `scripts/env-guard.sh` exits with code 1, stop and investigate before running cargo.
