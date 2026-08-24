// Best-effort app-data (`/data/data/<pkg>`) tar streaming via root.
//
// The device-side `tar` is streamed over `adb exec-out su -c tar -cf - -C
// /data/data '<pkg>'`. Because the stream is a raw pipe, a nonzero adb exit
// after partial output is indistinguishable from a short/corrupt archive, so
// this module treats size==0 as "skip" and everything else as best-effort.

use std::io::Read;
use std::path::Path;

use crate::adb::client::{quote_shell, AdbClient};
use crate::error::AppResult;

/// Stream `/data/data/<pkg>` from the device into a local tar file.
///
/// Returns the number of bytes written. Callers should treat a zero return as
/// "no usable archive" (skip restore); a partial stream cannot be detected
/// from here — see [`backup_appdata_best_effort`].
pub fn backup_appdata(client: &AdbClient, pkg: &str, out_tar: &Path) -> AppResult<u64> {
    let cmd = format!("su -c 'tar -cf - -C /data/data {}'", quote_shell(pkg));
    let mut child = client.shell_stream(&cmd)?;

    let result = match child.stdout.take() {
        Some(stdout) => write_stream(stdout, out_tar),
        None => Ok(0),
    };

    // Reap adb; a nonzero exit is not fatal here (partial tar is reported by
    // size checks at the caller).
    let _ = child.wait();
    result
}

/// Tolerant wrapper around [`backup_appdata`].
///
/// Device-side failures (no su, missing tar, adb errors mid-stream) are
/// converted to `Ok(0)` so a backup run never aborts over app-data. Only a
/// local file I/O failure returns `Err`, carrying the warning text otherwise
/// unused. Because a nonzero adb exit after a partial stream is
/// indistinguishable from a short tar, callers must treat `Ok(0)` (or a very
/// small size) as "skip app-data restore".
pub fn backup_appdata_best_effort(
    client: &AdbClient,
    pkg: &str,
    out_tar: &Path,
) -> Result<u64, String> {
    match backup_appdata(client, pkg, out_tar) {
        Ok(n) => Ok(n),
        // Local file I/O failure is the only hard error surfaced to callers.
        Err(crate::error::AppError::Io(io)) => Err(format!(
            "local I/O failure writing {}: {io}",
            out_tar.display()
        )),
        // Device-side failure (no su, no tar, adb error): warn and skip.
        Err(e) => {
            eprintln!("appdata backup skipped for {pkg}: {e}");
            Ok(0)
        }
    }
}

/// Copy a reader's bytes to a file, creating parent directories first.
/// Shared by production code and offline unit tests.
pub fn write_stream<R: Read>(mut r: R, out: &Path) -> AppResult<u64> {
    if let Some(parent) = out.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let mut f = std::fs::File::create(out)?;
    Ok(std::io::copy(&mut r, &mut f)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_exact_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out.tar");
        let data: Vec<u8> = (0..=255u8).cycle().take(5000).collect();
        let n = write_stream(std::io::Cursor::new(&data), &out).unwrap();
        assert_eq!(n, 5000);
        assert_eq!(std::fs::read(&out).unwrap(), data);
    }

    #[test]
    fn creates_missing_parent_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("a/b/c/backup.tar");
        let data = b"ustar-ish payload".to_vec();
        write_stream(std::io::Cursor::new(&data), &out).unwrap();
        assert_eq!(std::fs::read(&out).unwrap(), data);
    }

    #[test]
    fn empty_stream_writes_empty_file() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("empty.tar");
        let n = write_stream(std::io::empty(), &out).unwrap();
        assert_eq!(n, 0);
        assert!(out.exists());
    }
}
