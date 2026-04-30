//! Filesystem abstraction for testability.

use std::fs;
use std::io;
use std::path::Path;

use crate::config::CopyStrategy;

/// Abstraction over filesystem operations needed by the planner and executor.
pub trait FileSystem {
    /// Check if a path exists.
    fn exists(&self, path: &Path) -> bool;

    /// Check if a path is a regular file.
    fn is_file(&self, path: &Path) -> bool;

    /// Check if a path is a directory.
    fn is_dir(&self, path: &Path) -> bool;

    /// Check if a path is a symlink (without following it).
    fn is_symlink(&self, path: &Path) -> bool;

    /// Read the entire contents of a file.
    fn read(&self, path: &Path) -> io::Result<Vec<u8>>;

    /// Check whether any component of the path (excluding the final component)
    /// is a symlink.
    fn parent_has_symlink(&self, path: &Path) -> bool;

    /// Create all directories leading to `path`.
    fn create_dir_all(&self, path: &Path) -> io::Result<()>;

    /// Copy `src` to `dst` using the given [`CopyStrategy`], atomically
    /// replacing any existing file at `dst`. The implementation streams via
    /// a temp file in the same directory and renames into place so partial
    /// writes are never observable.
    fn copy_file(&self, src: &Path, dst: &Path, strategy: CopyStrategy) -> io::Result<()>;
}

/// Real filesystem implementation.
#[derive(Debug, Default)]
pub struct RealFs;

impl FileSystem for RealFs {
    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn is_file(&self, path: &Path) -> bool {
        path.is_file()
    }

    fn is_dir(&self, path: &Path) -> bool {
        path.is_dir()
    }

    fn is_symlink(&self, path: &Path) -> bool {
        fs::symlink_metadata(path)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false)
    }

    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        fs::read(path)
    }

    fn parent_has_symlink(&self, path: &Path) -> bool {
        let mut current = path.to_path_buf();
        // Walk up from the file's parent, checking each component
        while let Some(parent) = current.parent() {
            if parent == current {
                break;
            }
            if fs::symlink_metadata(parent)
                .map(|m| m.file_type().is_symlink())
                .unwrap_or(false)
            {
                return true;
            }
            current = parent.to_path_buf();
        }
        false
    }

    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        fs::create_dir_all(path)
    }

    fn copy_file(&self, src: &Path, dst: &Path, strategy: CopyStrategy) -> io::Result<()> {
        let parent = dst.parent().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "path has no parent directory")
        })?;

        let try_reflink = match strategy {
            CopyStrategy::SimpleCopy => false,
            CopyStrategy::CowCopy => true,
            CopyStrategy::Auto => cfg!(target_os = "macos"),
        };

        if try_reflink && try_reflink_into(src, dst, parent)? {
            return Ok(());
        }

        // Stream `src` through the temp file's open file descriptor and
        // then atomically rename into place. Because we never reopen the
        // tmp path between create and rename, an attacker with write
        // access to `parent` cannot substitute a symlink to redirect the
        // write to an attacker-chosen target.
        let mut tmp = tempfile::Builder::new()
            .prefix(".waft-copy-")
            .tempfile_in(parent)?;
        let mut src_file = fs::File::open(src)?;
        io::copy(&mut src_file, tmp.as_file_mut())?;
        tmp.into_temp_path().persist(dst).map_err(|e| e.error)?;
        Ok(())
    }
}

/// Try to reflink `src` to `dst` via a freshly-named temp file, returning
/// `Ok(true)` on success and `Ok(false)` if the underlying reflink call
/// failed (so the caller should fall back to a streaming copy).
///
/// The reflink primitives (`clonefile` on macOS, `ioctl_ficlone` on Linux)
/// require a non-existing destination, so we reserve a unique tmp name and
/// remove the placeholder before invoking the primitive. Both backends use
/// `O_EXCL` semantics: if an attacker races to substitute a symlink at the
/// path during the brief window between the `remove_file` call and the
/// reflink call, the reflink fails with `EEXIST` rather than write through
/// the symlink. We treat that failure like any other reflink failure and
/// let the caller fall back to the fd-streamed path, which uses a fresh
/// temp name and never reopens by path.
fn try_reflink_into(src: &Path, dst: &Path, parent: &Path) -> io::Result<bool> {
    let tmp = tempfile::Builder::new()
        .prefix(".waft-copy-")
        .tempfile_in(parent)?;
    let tmp_path = tmp.into_temp_path();
    fs::remove_file(&tmp_path)?;
    match reflink_copy::reflink(src, &tmp_path) {
        Ok(()) => {
            tmp_path.persist(dst).map_err(|e| e.error)?;
            Ok(true)
        }
        Err(_) => Ok(false),
    }
}
