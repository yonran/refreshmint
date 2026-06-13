//! Crash-safe whole-file replacement.
//!
//! Write the new contents to a temp file in the *same directory*, fsync it, then
//! atomically `rename` it over the destination. `rename(2)` is atomic within a
//! single filesystem, so a process kill (the [crate::run_shutdown_cleanup] hook
//! only fires on a graceful quit) — or a power loss after the fsync — leaves
//! either the old file or the new file on disk, never a torn one.
//!
//! This matters because the GL mutation paths in [crate::post] rewrite the whole
//! `general.journal` and then `git commit`; an in-place `fs::write` truncated by
//! a kill would leave an unparseable journal and would also break the in-process
//! rollback in those functions (which assumes a write either fully succeeds or
//! fully fails).
//!
//! Account journals go through [crate::account_journal::write_journal_at_path],
//! which delegates here. The JSON config writers in [crate::account_config] and
//! [crate::bookkeeping] implement the same temp-file + rename pattern inline.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

/// Atomically replace `path`'s contents with `contents`.
///
/// Callers must serialize writes to the same `path` (the ledger does so via the
/// GL / login file locks); the temp file name is derived from `path` and is not
/// collision-safe against a concurrent writer of the same file.
pub fn write_atomic(path: &Path, contents: &[u8]) -> io::Result<()> {
    let dir = path.parent().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "path has no parent directory")
    })?;
    fs::create_dir_all(dir)?;

    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no file name"))?;
    // Hidden sibling in the same directory so the rename stays on one
    // filesystem. Truncating-create reclaims any temp left by a prior crash.
    let temp_path = dir.join(format!(".{file_name}.tmp"));

    let write_result = (|| -> io::Result<()> {
        let mut file: File = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&temp_path)?;
        file.write_all(contents)?;
        // fsync the data before the rename so the rename can't be persisted
        // ahead of the contents on power loss.
        file.sync_all()?;
        Ok(())
    })();
    if let Err(err) = write_result {
        let _ = fs::remove_file(&temp_path);
        return Err(err);
    }

    if let Err(err) = fs::rename(&temp_path, path) {
        let _ = fs::remove_file(&temp_path);
        return Err(err);
    }

    // Best-effort: persist the directory entry so the rename itself survives a
    // power loss. Ignored on platforms/filesystems that reject directory fsync.
    if let Ok(dir_handle) = File::open(dir) {
        let _ = dir_handle.sync_all();
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(prefix: &str) -> std::path::PathBuf {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("refreshmint-{prefix}-{}-{now}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn write_atomic_creates_overwrites_and_leaves_no_temp() {
        let dir = temp_dir("fs-atomic");
        let path = dir.join("general.journal");

        write_atomic(&path, b"first").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "first");

        // Overwriting replaces the contents in full.
        write_atomic(&path, b"second longer contents").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "second longer contents");

        // No temp file is left behind after a successful write.
        let leftovers: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "unexpected temp files: {leftovers:?}");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_atomic_errors_when_path_has_no_parent() {
        // The filesystem root has no parent; this should error rather than panic.
        assert!(write_atomic(Path::new("/"), b"x").is_err());
    }
}
