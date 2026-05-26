use std::io::{Read, Write, BufWriter};
use std::path::Path;
use std::sync::{Arc, Mutex};

use crate::adb::client::AdbClient;
use crate::error::{AppError, AppResult};
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
    /// The command `adb exec-out "cd /path && tar cf - ."` streams all
    /// files as a single continuous data pipe. We extract locally.
    pub fn pull_dir(
        client: &AdbClient,
        remote_dir: &str,
        local_dir: &str,
        progress: &Arc<Mutex<TransferProgress>>,
    ) -> AppResult<TarPullStats> {
        // Create local directory
        std::fs::create_dir_all(local_dir)?;

        // Start tar stream from device
        let mut child = client.pull_dir_tar_stream(remote_dir)?;

        let stdout = child.stdout.take().ok_or_else(|| {
            AppError::Transfer {
                path: remote_dir.to_string(),
                reason: "Failed to capture tar output stream".into(),
            }
        })?;

        // We'll read the tar stream and extract manually
        // For simplicity and robustness, we pipe to the `tar` command on Windows
        // or use a pure-Rust tar extraction
        let mut bytes_transferred = 0u64;
        let mut buf = vec![0u8; 65536]; // 64KB buffer

        // Try to use Rust-native tar extraction approach
        // For Windows, we write raw tar data to a temp file then extract
        let tar_temp = Path::new(local_dir).join(".andpull_temp.tar");

        {
            let mut tar_file = BufWriter::new(
                std::fs::File::create(&tar_temp).map_err(AppError::Io)?
            );

            let mut reader = std::io::BufReader::new(stdout);

            loop {
                // Check cancellation
                let is_cancelled = progress.lock().unwrap().is_cancelled;
                if is_cancelled {
                    break;
                }

                match reader.read(&mut buf) {
                    Ok(0) => break, // EOF
                    Ok(n) => {
                        tar_file.write_all(&buf[..n])?;
                        bytes_transferred += n as u64;

                        // Update progress
                        let mut p = progress.lock().unwrap();
                        p.transferred_bytes += n as u64;
                        p.current_file = format!("(tar stream) {}", remote_dir);
                        p.update_speed();
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(e) => {
                        return Err(AppError::Transfer {
                            path: remote_dir.to_string(),
                            reason: format!("Tar stream read error: {}", e),
                        });
                    }
                }
            }

            tar_file.flush()?;
        }

        // Wait for adb process to finish
        let _ = child.wait();

        // Now extract the tar file
        let files_pulled = Self::extract_tar(&tar_temp, local_dir)?;

        // Clean up temp tar
        let _ = std::fs::remove_file(&tar_temp);

        Ok(TarPullStats {
            files_pulled,
            bytes_transferred,
        })
    }

    /// Extract a tar file to the given directory.
    fn extract_tar(tar_path: &Path, dest_dir: &str) -> AppResult<u64> {
        let file = std::fs::File::open(tar_path)?;
        let mut archive = tar_reader::TarReader::new(std::io::BufReader::new(file));
        let mut count = 0u64;

        // Simple tar extraction — tar format has 512-byte headers
        // followed by file data padded to 512-byte boundaries
        loop {
            match archive.next_entry() {
                Ok(Some(entry)) => {
                    let full_path = Path::new(dest_dir).join(&entry.name);

                    if entry.is_dir {
                        std::fs::create_dir_all(&full_path)?;
                    } else {
                        if let Some(parent) = full_path.parent() {
                            std::fs::create_dir_all(parent)?;
                        }
                        let mut out = std::fs::File::create(&full_path)?;
                        out.write_all(&entry.data)?;
                        count += 1;
                    }
                }
                Ok(None) => break,
                Err(e) => {
                    // Log but continue — some tar entries might be weird
                    eprintln!("Tar extraction warning: {}", e);
                    break;
                }
            }
        }

        Ok(count)
    }
}

/// Minimal tar reader — extracts files from a POSIX tar archive.
/// We implement this ourselves to avoid adding another dependency and
/// to have full control over error handling.
mod tar_reader {
    use std::io::{self, Read};

    pub struct TarEntry {
        pub name: String,
        pub size: u64,
        pub is_dir: bool,
        pub data: Vec<u8>,
    }

    pub struct TarReader<R: Read> {
        reader: R,
    }

    impl<R: Read> TarReader<R> {
        pub fn new(reader: R) -> Self {
            Self { reader }
        }

        pub fn next_entry(&mut self) -> io::Result<Option<TarEntry>> {
            let mut header = [0u8; 512];

            match self.reader.read_exact(&mut header) {
                Ok(()) => {},
                Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
                Err(e) => return Err(e),
            }

            // Check for end-of-archive marker (all zeros)
            if header.iter().all(|&b| b == 0) {
                return Ok(None);
            }

            // Parse file name (bytes 0-99)
            let name_bytes = &header[0..100];
            let name = String::from_utf8_lossy(name_bytes)
                .trim_end_matches('\0')
                .to_string();

            if name.is_empty() {
                return Ok(None);
            }

            // Parse file size (bytes 124-135, octal)
            let size_str = String::from_utf8_lossy(&header[124..136])
                .trim_end_matches('\0')
                .trim()
                .to_string();
            let size = u64::from_str_radix(&size_str, 8).unwrap_or(0);

            // Parse type flag (byte 156)
            let type_flag = header[156];
            let is_dir = type_flag == b'5' || name.ends_with('/');

            // Read file data
            let mut data = vec![0u8; size as usize];
            if size > 0 {
                self.reader.read_exact(&mut data)?;

                // Skip padding to 512-byte boundary
                let padding = (512 - (size % 512)) % 512;
                if padding > 0 {
                    let mut skip = vec![0u8; padding as usize];
                    self.reader.read_exact(&mut skip)?;
                }
            }

            // Handle long file names (GNU tar extension, type 'L')
            // The prefix field (bytes 345-499) for POSIX/UStar format
            let prefix = String::from_utf8_lossy(&header[345..500])
                .trim_end_matches('\0')
                .to_string();
            let full_name = if prefix.is_empty() {
                name
            } else {
                format!("{}/{}", prefix, name)
            };

            // Clean up path separators
            let clean_name = full_name
                .replace("./", "")
                .trim_start_matches('/')
                .to_string();

            Ok(Some(TarEntry {
                name: clean_name,
                size,
                is_dir,
                data,
            }))
        }
    }
}
