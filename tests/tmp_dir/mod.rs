#![allow(dead_code)]

use std::ffi::CString;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

// A unique temporary directory for a test.
//
// O_TMPFILE can only create unnamed regular files, but these tests need real
// directory trees. The anonymous file is therefore used to reserve a unique
// name: the directory is named after the anonymous inode and the open fd keeps
// that inode alive for as long as the directory exists, so no two tests can
// pick the same name. The directory is removed when the guard is dropped.
pub struct TempDir {
    path: PathBuf,
    _token: OwnedFd,
}

impl TempDir {
    pub fn new() -> Self {
        let base = Path::new("/tmp/zxbs-tests");
        std::fs::create_dir_all(base).unwrap();

        loop {
            let token = open_tmpfile(base);
            let inode = tmpfile_inode(&token);
            let path = base.join(format!("{inode:x}"));

            match std::fs::create_dir(&path) {
                Ok(()) => {
                    return Self {
                        path,
                        _token: token,
                    };
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => panic!("failed to create {}: {e}", path.display()),
            }
        }
    }

    pub fn as_str(&self) -> &str {
        self.path.to_str().unwrap()
    }

    pub fn join(&self, path: impl AsRef<Path>) -> PathBuf {
        self.path.join(path)
    }
}

impl Default for TempDir {
    fn default() -> Self {
        Self::new()
    }
}

impl std::ops::Deref for TempDir {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        if let Err(e) = std::fs::remove_dir_all(&self.path)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            eprintln!("failed to remove {}: {e}", self.path.display());
        }
    }
}

fn open_tmpfile(dir: &Path) -> OwnedFd {
    let dir = CString::new(dir.as_os_str().as_bytes()).unwrap();

    let fd = unsafe {
        libc::open(
            dir.as_ptr(),
            libc::O_TMPFILE | libc::O_EXCL | libc::O_RDWR,
            libc::S_IWUSR | libc::S_IRUSR,
        )
    };

    if fd < 0 {
        panic!(
            "failed to create an O_TMPFILE in {}: {}",
            dir.to_string_lossy(),
            std::io::Error::last_os_error()
        );
    }

    unsafe { OwnedFd::from_raw_fd(fd) }
}

fn tmpfile_inode(fd: &OwnedFd) -> u64 {
    let mut stat: libc::stat = unsafe { std::mem::zeroed() };
    let result = unsafe { libc::fstat(fd.as_raw_fd(), &mut stat) };

    assert_eq!(
        result,
        0,
        "fstat failed: {}",
        std::io::Error::last_os_error()
    );

    stat.st_ino
}
