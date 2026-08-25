# andpull 📱⚡

> Blazingly fast ADB media pull and push tool with a beautiful Terminal User Interface (TUI) written in Rust.

`andpull` is designed to be the fastest, most reliable way to sync files between your PC and Android device over ADB. It features smart delta-syncs, concurrent workers, and a dual-pane file browser that makes it incredibly easy to manage files on both sides.

## ✨ Features

- **TUI File Explorer:** Dual-pane interface (Local PC & Android Device) for easy navigation.
- **Bidirectional Sync:** Seamlessly pull files from your phone to your PC, or push files from your PC back to your phone.
- **Blazing Fast Transfers:** Utilizes concurrent workers and `tar` streaming (where supported) to maximize transfer speeds.
- **Smart Deduplication & Integrity:** Uses ultra-fast `xxhash-rust` (xxh3) to track file hashes, preventing redundant transfers and verifying file integrity after syncing.
- **Delta Sync:** Remembers what you've already transferred. Stop and resume syncs instantly.
- **Advanced Filtering & Search:** Quickly filter for media files or search for specific file patterns before mass selecting.

## 🚀 Prerequisites

1. **Rust & Cargo**: Make sure you have the Rust toolchain installed. ([Install Rust](https://rustup.rs/))
2. **ADB (Android Debug Bridge)**: ADB must be installed and available in your system's `PATH`.
   - Ensure your Android device has **USB Debugging** enabled in Developer Options.
   - Connect your device via USB (or wireless ADB) and ensure it shows up when running `adb devices`.

## 🛠️ Installation & Usage

Clone the repository and run it directly with Cargo:

```bash
git clone https://github.com/Apihplays/BackupToolAndroid.git
cd BackupToolAndroid
cargo run --release
```

## 🎮 Keybindings

Once the TUI is running, you can navigate using the following keys:

| Key | Action |
|-----|--------|
| `↑` / `↓` / `k` / `j` | Navigate up/down the list |
| `Tab` | Switch between Android Device and Local PC panes |
| `Enter` | Expand/Collapse folders |
| `Space` | Select/Deselect a file or folder |
| `s` | Start Sync (Pull if Android is active, Push if Local is active) |
| `/` | Search / Filter files |
| `f` | Toggle Media-only filter |
| `Del` / `x` | Delete selected files/folders |
| `q` / `Esc` | Quit the application |

## 🏗️ Technical Details

- **UI Framework:** Built with [ratatui](https://github.com/ratatui-org/ratatui) & crossterm.
- **State Management:** Sync history is saved locally via `serde_json`, tracking sizes, modification times, and xxh3 hashes.
- **Delta Sync:** mtime + size comparison for true incremental transfers — re-runs skip unchanged files in seconds.
- **Streaming Tar Fast-Path:** Extracts device tar streams on-the-fly (no temp file) with per-file state tracking.
- **Rooted Fallback:** Automatically uses `su` when ADB commands hit permission walls (Android 16+).
- **Transfer Engine:** Dynamically auto-detects device connection types and scales the `WorkerPool` (USB: 8 workers, WiFi: 2) to avoid overloading the ADB daemon. Override with `--workers N`.

## 📝 License

This project is licensed under the MIT License.
