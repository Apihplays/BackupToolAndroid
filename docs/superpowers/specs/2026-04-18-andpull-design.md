# andpull — ADB Media Pull Tool Design Spec

**Date:** 2026-04-18
**Language:** Rust
**Platform:** Windows

## Overview

`andpull` is a TUI-based tool for pulling media files from Android devices over ADB.
It supports bulk folder pulls and selective folder browsing, with resume support and
robust error handling for large-scale transfers (thousands of files, 10-100 GB).

## Requirements

- **Bulk pull**: Pull entire directory trees from device
- **Selective pull**: Browse device filesystem in TUI, select folders to pull
- **Transport**: Works over USB and WiFi ADB transparently
- **Resume**: Interrupted transfers can be resumed from where they left off
- **Error handling**: Individual file failures are skipped and logged, never crash the app
- **Scale**: Handles thousands of files and 100+ GB gracefully

## Architecture

```
TUI Layer (ratatui + crossterm)
    ↕
Application Logic (app state machine)
    ├── Scanner (file tree enumeration)
    ├── Transfer Engine (tar stream / sync RECV)
    └── Resume Manager (state persistence)
    ↕
ADB Protocol Layer (adb_client / custom sync)
    ↕
ADB Server (localhost:5037) → Device (USB/WiFi)
```

## Transfer Strategy

1. **Tar streaming** for bulk pulls: `adb exec-out tar cf - /path` piped to local
   extraction. 5-10x faster than individual file pulls due to eliminated per-file
   protocol overhead.

2. **Sync RECV** for selective/individual pulls: Uses ADB sync protocol to pull
   files in 64KB chunks with streaming disk writes.

3. **Smart resume**: State file tracks completed files. On resume, diffs against
   device and skips already-completed files.

## Project Structure

```
andpull/
├── Cargo.toml
├── src/
│   ├── main.rs           # Entry point, arg parsing
│   ├── app.rs            # App state machine
│   ├── adb/
│   │   ├── mod.rs
│   │   ├── client.rs     # ADB server connection
│   │   ├── sync.rs       # Sync protocol (STAT/RECV/LIST)
│   │   └── shell.rs      # Shell commands (tar streaming)
│   ├── scanner/
│   │   ├── mod.rs
│   │   └── tree.rs       # File tree data structure
│   ├── transfer/
│   │   ├── mod.rs
│   │   ├── engine.rs     # Transfer orchestrator
│   │   ├── tar.rs        # Tar stream bulk puller
│   │   └── recv.rs       # Individual file sync puller
│   ├── state/
│   │   ├── mod.rs
│   │   └── resume.rs     # State persistence
│   ├── tui/
│   │   ├── mod.rs
│   │   ├── browser.rs    # File tree browser
│   │   ├── progress.rs   # Transfer progress view
│   │   ├── summary.rs    # Final report view
│   │   └── widgets.rs    # Custom widgets
│   └── error.rs          # Typed error hierarchy
```

## Crate Dependencies

| Crate | Purpose |
|-------|---------|
| ratatui + crossterm | TUI rendering & input |
| tokio | Async runtime |
| adb_client | ADB protocol communication |
| serde + serde_json | State serialization |
| anyhow + thiserror | Error handling |
| chrono | Timestamps |
| sha2 | Optional checksum verification |

## Data Structures

### FileNode (device file tree)
```rust
struct FileNode {
    name: String,
    path: String,        // full device path
    is_dir: bool,
    size: u64,
    mtime: u64,
    children: Vec<FileNode>,
    selected: bool,
}
```

### TransferState (resume persistence)
```rust
struct TransferState {
    source: String,
    destination: String,
    started_at: DateTime<Utc>,
    completed_files: Vec<CompletedFile>,
    failed_files: Vec<FailedFile>,
}
```

## TUI Views

1. **Device Select** — pick from connected devices
2. **File Browser** — tree navigation with selection checkboxes
3. **Transfer Progress** — real-time progress, speed, ETA
4. **Summary** — final report with stats and error log

## Key Bindings

| Key | Action |
|-----|--------|
| ↑/↓ j/k | Navigate |
| Space | Toggle select |
| Enter | Expand/collapse |
| a | Select all |
| n | Select none |
| f | Filter media types |
| s | Start transfer |
| r | Resume previous |
| Tab | Switch view |
| q/Esc | Quit |

## Error Handling

- Typed errors via thiserror (Connection, Permission, Transfer, Checksum, DiskFull)
- Individual file errors never crash the app
- Failed files logged to state file for retry on resume
- Max 3 retry attempts per file before marking as permanently failed

## Performance Targets

- Saturate USB 2.0 bandwidth (~35 MB/s) for bulk transfers
- Saturate USB 3.0 bandwidth (~150 MB/s) where possible
- <64KB memory per file during transfer (streaming writes)
- File tree rendering at 60fps in TUI
