//! Filesystem helpers.
//!
//! The asset pipeline and editor write files that must never be observed in a
//! half-written state (a crash mid-write must not corrupt a project). The
//! engine's invariant is therefore: durable writes go through [`atomic_write`].

use std::io;
use std::path::{Path, PathBuf};
use std::process;

/// Writes `bytes` to `path` atomically.
///
/// The data is written to a sibling temporary file and then `rename`d over the
/// destination. `rename` within a directory is atomic on POSIX filesystems, so
/// a reader sees either the complete old file or the complete new file — never
/// a partial write.
pub fn atomic_write(path: impl AsRef<Path>, bytes: &[u8]) -> io::Result<()> {
    let path = path.as_ref();
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no file name"))?;

    let mut tmp = PathBuf::from(dir);
    tmp.push(format!(
        ".{}.tmp.{}",
        file_name.to_string_lossy(),
        process::id()
    ));

    // Scope the handle so it is flushed and closed before the rename.
    {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }

    match std::fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(e) => {
            // Best-effort cleanup; the rename failure is the error that matters.
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// Reads an entire file into a byte vector.
pub fn read(path: impl AsRef<Path>) -> io::Result<Vec<u8>> {
    std::fs::read(path)
}

/// Returns `true` if `path` exists and is a regular file.
pub fn is_file(path: impl AsRef<Path>) -> bool {
    path.as_ref().is_file()
}

/// Creates `path` and every missing parent directory.
pub fn create_dir_all(path: impl AsRef<Path>) -> io::Result<()> {
    std::fs::create_dir_all(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_round_trips() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("engine-platform-test-{}.bin", process::id()));
        atomic_write(&path, b"deterministic forge").unwrap();
        assert_eq!(read(&path).unwrap(), b"deterministic forge");
        assert!(is_file(&path));
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn atomic_write_leaves_no_temp_file() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("engine-platform-clean-{}.bin", process::id()));
        atomic_write(&path, b"x").unwrap();
        let tmp = dir.join(format!(
            ".engine-platform-clean-{}.bin.tmp.{}",
            process::id(),
            process::id()
        ));
        assert!(!tmp.exists());
        std::fs::remove_file(&path).unwrap();
    }
}
