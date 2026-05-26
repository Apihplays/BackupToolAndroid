use std::io::Read;
use std::path::Path;

use xxhash_rust::xxh3::Xxh3;

use crate::error::{AppError, AppResult};

/// Size of the read buffer for hashing (64 KB matches ADB sync chunk size).
const HASH_BUFFER_SIZE: usize = 64 * 1024;

/// Compute xxh3-128 hash of a local file, returning a hex string.
/// Streams the file in 64KB chunks so memory usage stays constant regardless of file size.
pub fn compute_file_hash(path: &Path) -> AppResult<String> {
    let file = std::fs::File::open(path).map_err(|e| AppError::Transfer {
        path: path.to_string_lossy().into_owned(),
        reason: format!("Cannot open file for hashing: {}", e),
    })?;

    let mut reader = std::io::BufReader::new(file);
    let mut hasher = Xxh3::new();
    let mut buffer = [0u8; HASH_BUFFER_SIZE];

    loop {
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => hasher.update(&buffer[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => {
                return Err(AppError::Transfer {
                    path: path.to_string_lossy().into_owned(),
                    reason: format!("IO error during hashing: {}", e),
                });
            }
        }
    }

    Ok(format!("{:016x}", hasher.digest128()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_hash_empty_file() {
        let dir = std::env::temp_dir().join("andpull_test_hash");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("empty.bin");
        std::fs::write(&path, b"").unwrap();

        let hash = compute_file_hash(&path).unwrap();
        assert!(!hash.is_empty());
        assert_eq!(hash.len(), 32); // 128-bit = 32 hex chars

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_hash_deterministic() {
        let dir = std::env::temp_dir().join("andpull_test_hash");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("deterministic.bin");
        std::fs::write(&path, b"hello andpull world").unwrap();

        let hash1 = compute_file_hash(&path).unwrap();
        let hash2 = compute_file_hash(&path).unwrap();
        assert_eq!(hash1, hash2);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_hash_different_content() {
        let dir = std::env::temp_dir().join("andpull_test_hash");
        let _ = std::fs::create_dir_all(&dir);

        let path_a = dir.join("a.bin");
        let path_b = dir.join("b.bin");
        std::fs::write(&path_a, b"content A").unwrap();
        std::fs::write(&path_b, b"content B").unwrap();

        let hash_a = compute_file_hash(&path_a).unwrap();
        let hash_b = compute_file_hash(&path_b).unwrap();
        assert_ne!(hash_a, hash_b);

        let _ = std::fs::remove_file(&path_a);
        let _ = std::fs::remove_file(&path_b);
    }

    #[test]
    fn test_hash_large_file() {
        let dir = std::env::temp_dir().join("andpull_test_hash");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("large.bin");

        // Write 256KB of patterned data (spans multiple buffer reads)
        let mut f = std::fs::File::create(&path).unwrap();
        let pattern: Vec<u8> = (0..=255).collect();
        for _ in 0..1024 {
            f.write_all(&pattern).unwrap();
        }
        drop(f);

        let hash = compute_file_hash(&path).unwrap();
        assert_eq!(hash.len(), 32);

        // Verify determinism across buffer boundaries
        let hash2 = compute_file_hash(&path).unwrap();
        assert_eq!(hash, hash2);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_hash_nonexistent_file() {
        let path = Path::new("/tmp/andpull_does_not_exist_12345.bin");
        let result = compute_file_hash(path);
        assert!(result.is_err());
    }
}
