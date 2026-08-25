//! A local, rotating log file with the same rotation semantics as
//! `Mac/Log.swift`'s `RotatingLogFile` (rotate, never upload, user-
//! shareable), located under the Windows per-user app-data path, per the
//! spec's local-logging Assumption.
//!
//! Ported *policy*, not code (different language, per `design.md`): one
//! live file plus exactly one previous generation, replaced (not
//! accumulated) on rotation, so at most `2 * max_bytes` survives on disk —
//! rotating rather than hard-capping so the *newest* bytes always survive,
//! which is what a bug report needs.
//!
//! Pure `std::fs` file I/O — no Windows-specific API needed for the
//! rotation policy itself, so it is exercised directly against a real
//! temporary directory in tests rather than through an injectable trait.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// A local, size-capped, rotating log file. At most one previous generation
/// is retained (`<base>.log` + `<base>-previous.log`), matching
/// `Mac/Log.swift`'s `RotatingLogFile` shape exactly.
pub struct RotatingLogFile {
    directory: PathBuf,
    file_path: PathBuf,
    rotated_path: PathBuf,
    max_bytes: u64,
}

impl RotatingLogFile {
    pub fn new(directory: impl Into<PathBuf>, base_name: &str, max_bytes: u64) -> Self {
        let directory = directory.into();
        let file_path = directory.join(format!("{base_name}.log"));
        let rotated_path = directory.join(format!("{base_name}-previous.log"));
        Self {
            directory,
            file_path,
            rotated_path,
            max_bytes,
        }
    }

    /// Appends `data`, rotating first if it would push the live file past
    /// `max_bytes`. Returns the underlying I/O error rather than panicking
    /// when the directory can't be created or the file can't be written
    /// (e.g. a permissions problem) — logging must never crash the app it's
    /// diagnosing.
    pub fn append(&self, data: &[u8]) -> io::Result<()> {
        fs::create_dir_all(&self.directory)?;

        let current_size = fs::metadata(&self.file_path).map(|m| m.len()).unwrap_or(0);
        if current_size + data.len() as u64 > self.max_bytes {
            self.rotate()?;
        }

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.file_path)?;
        file.write_all(data)
    }

    /// Replaces the previous generation with the current one (an atomic
    /// rename, so a crash mid-rotation never loses both), then lets the
    /// next `append` recreate an empty current file.
    fn rotate(&self) -> io::Result<()> {
        if self.file_path.exists() {
            fs::rename(&self.file_path, &self.rotated_path)?;
        }
        Ok(())
    }

    /// The log files that currently exist on disk, for a "reveal in
    /// Explorer" action (mirrors `Mac/Log.swift`'s `existingFiles()`).
    pub fn existing_files(&self) -> Vec<PathBuf> {
        [&self.file_path, &self.rotated_path]
            .into_iter()
            .filter(|p| p.exists())
            .cloned()
            .collect()
    }
}

/// The Windows per-user app-data path for the log directory, per the spec's
/// Assumption ("under the Windows per-user app-data path"). Falls back to
/// the current directory if `LOCALAPPDATA` is unset, which should not
/// happen on a real Windows session — this is glue, not policy, so it is
/// not unit-tested (nothing to assert beyond "reads an env var").
pub fn default_log_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(".").to_path_buf());
    base.join("OpenDisplay").join("Logs")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    /// A fresh, unique scratch directory per test, cleaned up on drop.
    struct ScratchDir(PathBuf);

    impl ScratchDir {
        fn new(label: &str) -> Self {
            let n = COUNTER.fetch_add(1, Ordering::SeqCst);
            let path = std::env::temp_dir().join(format!("opendisplay-log-test-{label}-{n}"));
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for ScratchDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_normal_append_within_the_cap_writes_without_rotating() {
        let dir = ScratchDir::new("normal-append");
        let log = RotatingLogFile::new(dir.path(), "opendisplay", 1024);

        log.append(b"hello\n").expect("append succeeds");

        let contents = fs::read(dir.path().join("opendisplay.log")).unwrap();
        assert_eq!(contents, b"hello\n");
        assert!(!dir.path().join("opendisplay-previous.log").exists());
    }

    #[test]
    fn writing_past_max_bytes_rotates_the_live_file_into_previous() {
        let dir = ScratchDir::new("rotation-trigger");
        // max_bytes=10: the first 8-byte write fits; the second 8-byte
        // write would push the live file to 16 bytes, over the cap, so it
        // must rotate first.
        let log = RotatingLogFile::new(dir.path(), "opendisplay", 10);

        log.append(b"AAAAAAAA").expect("first append succeeds");
        log.append(b"BBBBBBBB").expect("second append rotates, then succeeds");

        let previous = fs::read(dir.path().join("opendisplay-previous.log")).unwrap();
        let current = fs::read(dir.path().join("opendisplay.log")).unwrap();
        assert_eq!(previous, b"AAAAAAAA", "the pre-rotation generation must survive as -previous");
        assert_eq!(current, b"BBBBBBBB", "the post-rotation file must start fresh with the new write");
    }

    #[test]
    fn only_one_previous_generation_is_ever_retained() {
        let dir = ScratchDir::new("retention-count");
        let log = RotatingLogFile::new(dir.path(), "opendisplay", 10);

        // Three rounds, each triggering a rotation: A, then B (rotates A
        // into previous), then C (rotates B into previous, discarding A).
        log.append(b"AAAAAAAA").unwrap();
        log.append(b"BBBBBBBB").unwrap();
        log.append(b"CCCCCCCC").unwrap();

        let files_in_dir: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(files_in_dir.len(), 2, "exactly current + previous, never a third generation");

        let previous = fs::read(dir.path().join("opendisplay-previous.log")).unwrap();
        assert_eq!(previous, b"BBBBBBBB", "previous must hold the immediately-prior generation, not the oldest one");
    }

    #[test]
    fn a_directory_creation_failure_returns_an_error_instead_of_panicking() {
        let dir = ScratchDir::new("write-failure");
        fs::create_dir_all(dir.path()).unwrap();
        let blocker = dir.path().join("blocker");
        fs::write(&blocker, b"not a directory").unwrap();
        // The log directory is a child of a plain file, so create_dir_all
        // must fail — this simulates an unwritable log location without
        // relying on platform-specific permission bits.
        let unwritable_dir = blocker.join("subdir");
        let log = RotatingLogFile::new(unwritable_dir, "opendisplay", 1024);

        let result = log.append(b"should not panic");

        assert!(result.is_err(), "append must return Err, not panic, when the log location is unwritable");
    }

    #[test]
    fn existing_files_lists_only_files_that_are_actually_present() {
        let dir = ScratchDir::new("existing-files");
        let log = RotatingLogFile::new(dir.path(), "opendisplay", 1024);

        assert!(log.existing_files().is_empty(), "nothing written yet");

        log.append(b"line\n").unwrap();
        let existing = log.existing_files();
        assert_eq!(existing, vec![dir.path().join("opendisplay.log")]);
    }
}
