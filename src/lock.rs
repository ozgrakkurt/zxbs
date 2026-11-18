use std::fs::{File, OpenOptions, TryLockError};
use std::path::Path;

// An exclusive lock on a build directory.
//
// The lock is held for as long as this value is alive and released when it is
// dropped (or when the process exits).
pub struct BuildLock {
    _file: File,
}

impl BuildLock {
    // Blocks until this process holds the exclusive lock on `build_path`.
    pub fn acquire(lock_path: &Path) -> BuildLock {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path)
            .unwrap_or_else(|e| {
                panic!(
                    "Failed to open build lock file `{}`: {}",
                    lock_path.display(),
                    e
                )
            });

        match file.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => {
                eprintln!("Blocking waiting on build lock at: {}", lock_path.display());

                file.lock().unwrap_or_else(|e| {
                    panic!(
                        "Failed to lock the build lock at: `{}`: {}",
                        lock_path.display(),
                        e
                    )
                });

                eprintln!("Acquired the build lock at: {}", lock_path.display());
            }
            Err(TryLockError::Error(e)) => {
                panic!(
                    "Failed to lock the build lock at`{}`: {}",
                    lock_path.display(),
                    e
                )
            }
        }

        BuildLock { _file: file }
    }
}
