use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};

use crate::adb::client::{quote_shell, AdbClient};
use crate::error::{AppError, AppResult};
use crate::state::StateManager;
use crate::transfer::engine::TransferProgress;

/// Stats from a tar pull operation.
pub struct TarPullStats {
    pub files_pulled: u64,
    pub bytes_transferred: u64,
}

/// Pulls an entire directory using tar streaming over ADB exec-out.
/// This is dramatically faster than individual file pulls because it
/// eliminates the per-file protocol overhead.
pub struct TarPuller;

impl TarPuller {
    /// Pull an entire directory from device using tar streaming.
    ///
    /// Streams tar data from the device and extracts entries on-the-fly
    /// (no temp file). Each extracted file is recorded in `state_manager`
    /// for delta-sync on re-runs. Non-zero device-side exit is treated
    /// as a hard error — truncated tars are never silently accepted.
    pub fn pull_dir(
        client: &AdbClient,
        remote_dir: &str,
        local_dir: &str,
        progress: &Arc<Mutex<TransferProgress>>,
        state_manager: &mut StateManager,
    ) -> AppResult<TarPullStats> {
        std::fs::create_dir_all(local_dir)?;

        // Start tar stream, falling back to rooted on permission failure.
        let mut child = match client.pull_dir_tar_stream(remote_dir) {
            Ok(c) => c,
            Err(_) if client.su_available() => {
                let cmd = format!(
                    "su -c 'cd {} && tar cf - .'",
                    quote_shell(remote_dir)
                );
                client.shell_stream(&cmd)?
            }
            Err(e) => return Err(e),
        };

        let mut stdout = child.stdout.take().ok_or_else(|| AppError::Transfer {
            path: remote_dir.to_string(),
            reason: "Failed to capture tar output stream".into(),
        })?;

        // Take stderr so we can report device-side errors.
        let mut stderr = child.stderr.take();

        // Stream-extract: read headers + data directly, no temp file.
        let mut reader = StreamingTarReader::new(&mut stdout);
        let mut files_pulled: u64 = 0;
        let mut bytes_transferred: u64 = 0;

        loop {
            // Check cancellation.
            {
                let p = progress.lock().unwrap();
                if p.is_cancelled {
                    let _ = child.kill();
                    let _ = child.wait(); // Reap zombie process.
                    return Err(AppError::Transfer {
                        path: remote_dir.to_string(),
                        reason: "Transfer cancelled".into(),
                    });
                }
            }

            match reader.stream_next_entry(local_dir)? {
                Some(entry) => {
                    files_pulled += 1;
                    bytes_transferred += entry.size;

                    // Update progress with per-file visibility.
                    {
                        let mut p = progress.lock().unwrap();
                        p.completed_files += 1;
                        p.transferred_bytes += entry.size;
                        p.current_file = entry.name.clone();
                        p.update_speed();
                    }

                    // Record per-file state for delta-sync on re-runs.
                    let remote_path = if remote_dir.ends_with('/') {
                        format!("{}{}", remote_dir, entry.name)
                    } else {
                        format!("{}/{}", remote_dir, entry.name)
                    };
                    let local_path = Path::new(local_dir).join(&entry.name);
                    let local_str = local_path.to_string_lossy().to_string();

                    state_manager.mark_file_completed(
                        &remote_path,
                        entry.size,
                        0, // mtime unknown from tar header parsing
                        None,
                    );
                    state_manager.update_sync_record(
                        &remote_path,
                        entry.size,
                        &local_str,
                        None,
                        0, // mtime unknown from tar header
                    );
                }
                None => break, // End of archive.
            }
        }

        // Wait for adb process and check exit status.
        let status = child.wait().map_err(|e| AppError::Transfer {
            path: remote_dir.to_string(),
            reason: format!("Failed to wait for tar process: {}", e),
        })?;

        if !status.success() {
            // Capture stderr tail for diagnostics.
            let stderr_tail = if let Some(ref mut s) = stderr {
                let mut buf = String::new();
                let _ = s.read_to_string(&mut buf);
                // Take last 512 chars to avoid huge output.
                let len = buf.len();
                if len > 512 {
                    buf = buf[len - 512..].to_string();
                }
                buf
            } else {
                String::new()
            };

            return Err(AppError::Transfer {
                path: remote_dir.to_string(),
                reason: format!(
                    "device tar exited with {} — stderr tail: {:?}",
                    status, stderr_tail
                ),
            });
        }

        // Check if we extracted anything — an empty archive with zero
        // exit could mean the directory was genuinely empty, but a
        // truncated stream often means the process was killed mid-transfer.
        if files_pulled == 0 {
            // Only warn if we transferred bytes (partial stream).
            if bytes_transferred > 0 {
                return Err(AppError::Transfer {
                    path: remote_dir.to_string(),
                    reason: format!(
                        "tar stream contained {} bytes but zero extractable files — \
                         possible truncation",
                        bytes_transferred
                    ),
                });
            }
        }

        Ok(TarPullStats {
            files_pulled,
            bytes_transferred,
        })
    }
}

/// Metadata about a single tar entry (no data buffer).
struct TarEntryMeta {
    name: String,
    size: u64,
    is_dir: bool,
}

/// Streaming tar reader that extracts entries on-the-fly from a `Read` source.
///
/// Unlike the old approach that buffered the entire archive to a temp file,
/// this reads headers + data sequentially and writes each file directly to
/// disk as it arrives from the device.
struct StreamingTarReader<R: Read> {
    reader: R,
}

impl<R: Read> StreamingTarReader<R> {
    fn new(reader: R) -> Self {
        Self { reader }
    }

    /// Read the next tar entry and stream its data directly to `dest_dir`.
    /// Returns entry metadata on success, or `Ok(None)` at end-of-archive.
    fn stream_next_entry(&mut self, dest_dir: &str) -> AppResult<Option<TarEntryMeta>> {
        let mut header = [0u8; 512];

        match self.reader.read_exact(&mut header) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(e) => return Err(AppError::Io(e)),
        }

        // End-of-archive marker (two 512-byte zero blocks, but we check one).
        if header.iter().all(|&b| b == 0) {
            return Ok(None);
        }

        // Parse file name (bytes 0–99).
        let name_bytes = &header[0..100];
        let name = String::from_utf8_lossy(name_bytes)
            .trim_end_matches('\0')
            .to_string();

        if name.is_empty() {
            return Ok(None);
        }

        // Parse file size (bytes 124–135, octal).
        let size_str = String::from_utf8_lossy(&header[124..136])
            .trim_end_matches('\0')
            .trim()
            .to_string();
        let size = u64::from_str_radix(&size_str, 8).unwrap_or(0);

        // Parse type flag (byte 156).
        let type_flag = header[156];
        let is_dir = type_flag == b'5' || name.ends_with('/');

        // Handle long file names (GNU tar extension, type 'L' / 0x4c).
        // When the name field holds the magic `././@LongLink`, the real
        // filename is in the *next* entry's data block.
        if name == "././@LongLink" || name.ends_with("@LongLink") {
            // The long name is stored as the data of this pseudo-entry.
            let mut long_name_buf = vec![0u8; size as usize];
            if size > 0 {
                self.reader.read_exact(&mut long_name_buf)?;
            }
            // Skip padding.
            let padding = (512 - (size % 512)) % 512;
            if padding > 0 {
                let mut skip = vec![0u8; padding as usize];
                self.reader.read_exact(&mut skip)?;
            }
            // Now read the *real* header that follows.
            self.reader.read_exact(&mut header)?;
            let real_name = String::from_utf8_lossy(&header[0..100])
                .trim_end_matches('\0')
                .to_string();
            // Re-parse size from the real header.
            let real_size_str = String::from_utf8_lossy(&header[124..136])
                .trim_end_matches('\0')
                .trim()
                .to_string();
            let real_size = u64::from_str_radix(&real_size_str, 8).unwrap_or(0);
            let real_type = header[156];
            let real_is_dir = real_type == b'5' || real_name.ends_with('/');

            // Use the long name but with the real header's size/type.
            let name_from_long = String::from_utf8_lossy(&long_name_buf)
                .trim_end_matches('\0')
                .replace("./", "")
                .trim_start_matches('/')
                .to_string();

            // Stream the real entry's data.
            self.stream_entry_data(dest_dir, &name_from_long, real_size)?;

            return Ok(Some(TarEntryMeta {
                name: name_from_long,
                size: real_size,
                is_dir: real_is_dir,
            }));
        }

        // UStar prefix field (bytes 345–499).
        let prefix = String::from_utf8_lossy(&header[345..500])
            .trim_end_matches('\0')
            .to_string();
        let full_name = if prefix.is_empty() {
            name
        } else {
            format!("{}/{}", prefix, name)
        };
        let cleaned = full_name
            .replace("./", "")
            .trim_start_matches('/')
            .to_string();

        // Stream data directly to disk.
        self.stream_entry_data(dest_dir, &cleaned, size)?;

        Ok(Some(TarEntryMeta {
            name: cleaned,
            size,
            is_dir,
        }))
    }

    /// Stream `size` bytes of entry data to a file under `dest_dir`,
    /// creating parent directories as needed. Skips padding afterward.
    fn stream_entry_data(
        &mut self,
        dest_dir: &str,
        name: &str,
        size: u64,
    ) -> AppResult<()> {
        if size == 0 || name.ends_with('/') {
            // Directory or empty file — nothing to stream.
            // Still skip padding if present.
            let padding = (512 - (size % 512)) % 512;
            if padding > 0 {
                let mut skip = vec![0u8; padding as usize];
                self.reader.read_exact(&mut skip)?;
            }
            return Ok(());
        }

        let full_path = Path::new(dest_dir).join(name);
        if let Some(parent) = full_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let mut out = std::fs::File::create(&full_path)?;
        let mut remaining = size;
        let mut buf = [0u8; 64 * 1024];

        while remaining > 0 {
            let to_read = remaining.min(buf.len() as u64) as usize;
            match self.reader.read(&mut buf[..to_read]) {
                Ok(0) => {
                    return Err(AppError::Transfer {
                        path: name.to_string(),
                        reason: format!(
                            "unexpected EOF while reading entry '{}' — \
                             expected {} bytes, got {}",
                            name,
                            size,
                            size - remaining
                        ),
                    });
                }
                Ok(n) => {
                    out.write_all(&buf[..n])?;
                    remaining -= n as u64;
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(AppError::Io(e)),
            }
        }

        // Skip padding to 512-byte boundary.
        let padding = (512 - (size % 512)) % 512;
        if padding > 0 {
            let mut skip = vec![0u8; padding as usize];
            self.reader.read_exact(&mut skip)?;
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// Build a minimal POSIX tar archive in memory from a list of
    /// (name, content_bytes) pairs.
    fn build_tar(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buf = Vec::new();
        for (name, data) in entries {
            let mut header = [0u8; 512];
            // Name (bytes 0–99)
            let name_bytes = name.as_bytes();
            let name_len = name_bytes.len().min(100);
            header[..name_len].copy_from_slice(&name_bytes[..name_len]);
            // Mode (bytes 100–107) — 0644
            header[100..108].copy_from_slice(b"0000644\0");
            // UID (bytes 108–115)
            header[108..116].copy_from_slice(b"0000000\0");
            // GID (bytes 116–123)
            header[116..124].copy_from_slice(b"0000000\0");
            // Size (bytes 124–135, octal)
            let size_oct = format!("{:011o}\0", data.len());
            header[124..136].copy_from_slice(size_oct.as_bytes());
            // Mtime (bytes 136–147)
            header[136..148].copy_from_slice(b"00000000000\0");
            // Type flag (byte 156) — '0' = regular file
            header[156] = b'0';
            // UStar magic (bytes 257–262)
            header[257..263].copy_from_slice(b"ustar\0");
            // UStar version (bytes 263–265)
            header[263..265].copy_from_slice(b"00");

            // Compute checksum.
            // Checksum field (bytes 148–155) is initially spaces.
            for b in &mut header[148..156] {
                *b = b' ';
            }
            let chk: u32 = header.iter().map(|&b| b as u32).sum();
            // Standard tar checksum: 6 octal digits + NUL + space = 8 bytes.
            let chk_oct = format!("{:06o}\0 ", chk);
            header[148..156].copy_from_slice(&chk_oct.as_bytes()[..8]);

            buf.extend_from_slice(&header);
            buf.extend_from_slice(data);
            // Pad to 512-byte boundary.
            let padding = (512 - (data.len() % 512)) % 512;
            buf.extend(std::iter::repeat_n(0u8, padding));
        }
        // End-of-archive marker: two zero blocks.
        buf.extend(std::iter::repeat_n(0u8, 1024));
        buf
    }

    #[test]
    fn streaming_extract_single_file() {
        let tar_data = build_tar(&[("hello.txt", b"hello world")]);
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().to_str().unwrap();

        let mut reader = StreamingTarReader::new(Cursor::new(&tar_data));
        let mut files = Vec::new();
        while let Some(meta) = reader.stream_next_entry(dest).unwrap() {
            files.push(meta);
        }

        assert_eq!(files.len(), 1);
        assert_eq!(files[0].name, "hello.txt");
        assert_eq!(files[0].size, 11);
        assert!(!files[0].is_dir);
        assert_eq!(std::fs::read(tmp.path().join("hello.txt")).unwrap(), b"hello world");
    }

    #[test]
    fn streaming_extract_multiple_files() {
        let tar_data = build_tar(&[
            ("a.txt", b"aaa"),
            ("b.txt", b"bb"),
            ("c.txt", b"c"),
        ]);
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().to_str().unwrap();

        let mut reader = StreamingTarReader::new(Cursor::new(&tar_data));
        let mut count = 0u64;
        while reader.stream_next_entry(dest).unwrap().is_some() {
            count += 1;
        }

        assert_eq!(count, 3);
        assert_eq!(std::fs::read(tmp.path().join("a.txt")).unwrap(), b"aaa");
        assert_eq!(std::fs::read(tmp.path().join("b.txt")).unwrap(), b"bb");
        assert_eq!(std::fs::read(tmp.path().join("c.txt")).unwrap(), b"c");
    }

    #[test]
    fn streaming_extract_empty_archive() {
        // Two zero blocks = valid empty archive.
        let tar_data = vec![0u8; 1024];
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().to_str().unwrap();

        let mut reader = StreamingTarReader::new(Cursor::new(&tar_data));
        let result = reader.stream_next_entry(dest).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn streaming_extract_nested_dirs() {
        let tar_data = build_tar(&[
            ("subdir/nested/file.txt", b"deep"),
        ]);
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().to_str().unwrap();

        let mut reader = StreamingTarReader::new(Cursor::new(&tar_data));
        let meta = reader.stream_next_entry(dest).unwrap().unwrap();
        assert_eq!(meta.name, "subdir/nested/file.txt");
        assert_eq!(
            std::fs::read(tmp.path().join("subdir/nested/file.txt")).unwrap(),
            b"deep"
        );
    }

    #[test]
    fn streaming_extract_zero_size_file() {
        let tar_data = build_tar(&[("empty.txt", b"")]);
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().to_str().unwrap();

        let mut reader = StreamingTarReader::new(Cursor::new(&tar_data));
        let meta = reader.stream_next_entry(dest).unwrap().unwrap();
        assert_eq!(meta.name, "empty.txt");
        assert_eq!(meta.size, 0);
    }

    #[test]
    fn streaming_extract_truncated_tar_errors() {
        // Build a valid header for a 100-byte file but only provide 10 bytes of data.
        let mut tar_data = build_tar(&[("big.txt", &[0u8; 100])]);
        tar_data.truncate(600); // Chop off most of the data + padding + EOF marker.

        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().to_str().unwrap();

        let mut reader = StreamingTarReader::new(Cursor::new(&tar_data));
        let result = reader.stream_next_entry(dest);
        assert!(result.is_err());
    }

    /// Build a tar with a GNU long-name entry (>100 chars) and verify extraction.
    #[test]
    fn streaming_extract_gnu_longname() {
        let long_name = format!(".trashed-1787713854-{}.jpg", "A".repeat(80));
        assert!(long_name.len() > 100, "long name must exceed 100 chars");

        // Build a tar with a @LongLink pseudo-entry followed by the real entry.
        let mut tar_data = Vec::new();

        // 1. @LongLink entry — type 'L' (0x4c), data = the long filename.
        {
            let name_bytes = b"././@LongLink";
            let data = long_name.as_bytes();
            let mut header = [0u8; 512];
            header[..name_bytes.len()].copy_from_slice(name_bytes);
            header[100..108].copy_from_slice(b"0000644\0");
            header[108..116].copy_from_slice(b"0000000\0");
            header[116..124].copy_from_slice(b"0000000\0");
            let size_oct = format!("{:011o}\0", data.len());
            header[124..136].copy_from_slice(size_oct.as_bytes());
            header[136..148].copy_from_slice(b"00000000000\0");
            header[156] = b'L'; // GNU longlink type
            header[257..263].copy_from_slice(b"ustar\0");
            header[263..265].copy_from_slice(b"00");
            for b in &mut header[148..156] { *b = b' '; }
            let chk: u32 = header.iter().map(|&b| b as u32).sum();
            // Standard tar checksum: 6 octal digits + NUL + space = 8 bytes.
            let chk_oct = format!("{:06o}\0 ", chk);
            header[148..156].copy_from_slice(&chk_oct.as_bytes()[..8]);
            tar_data.extend_from_slice(&header);
            tar_data.extend_from_slice(data);
            let padding = (512 - (data.len() % 512)) % 512;
            tar_data.extend(std::iter::repeat_n(0u8, padding));
        }

        // 2. Real file entry (header says name is short, but we use the long name).
        {
            let file_data = b"photo content";
            let mut header = [0u8; 512];
            // Short placeholder name — will be overwritten by long name logic.
            header[..1].copy_from_slice(b"x");
            header[100..108].copy_from_slice(b"0000644\0");
            header[108..116].copy_from_slice(b"0000000\0");
            header[116..124].copy_from_slice(b"0000000\0");
            let size_oct = format!("{:011o}\0", file_data.len());
            header[124..136].copy_from_slice(size_oct.as_bytes());
            header[136..148].copy_from_slice(b"00000000000\0");
            header[156] = b'0';
            header[257..263].copy_from_slice(b"ustar\0");
            header[263..265].copy_from_slice(b"00");
            for b in &mut header[148..156] { *b = b' '; }
            let chk: u32 = header.iter().map(|&b| b as u32).sum();
            // Standard tar checksum: 6 octal digits + NUL + space = 8 bytes.
            let chk_oct = format!("{:06o}\0 ", chk);
            header[148..156].copy_from_slice(&chk_oct.as_bytes()[..8]);
            tar_data.extend_from_slice(&header);
            tar_data.extend_from_slice(file_data);
            let padding = (512 - (file_data.len() % 512)) % 512;
            tar_data.extend(std::iter::repeat_n(0u8, padding));
        }

        // End-of-archive.
        tar_data.extend(std::iter::repeat_n(0u8, 1024));

        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().to_str().unwrap();

        let mut reader = StreamingTarReader::new(Cursor::new(&tar_data));
        let meta = reader.stream_next_entry(dest).unwrap().unwrap();
        assert_eq!(meta.name, long_name);
        assert!(!meta.is_dir);

        let content = std::fs::read(tmp.path().join(&long_name)).unwrap();
        assert_eq!(content, b"photo content");
    }
}
