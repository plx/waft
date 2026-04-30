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

        // Reserve a unique tmp name in the destination's directory. The
        // empty placeholder file is removed before placing content so the
        // OS-level reflink primitives (clonefile / FICLONE) can create a
        // fresh inode at that path.
        let tmp = tempfile::Builder::new()
            .prefix(".waft-copy-")
            .tempfile_in(parent)?;
        let tmp_path = tmp.into_temp_path();
        fs::remove_file(&tmp_path)?;

        match place_content(src, &tmp_path, strategy) {
            Ok(()) => {
                tmp_path.persist(dst).map_err(|e| e.error)?;
                Ok(())
            }
            Err(e) => {
                // tmp_path's Drop removes whatever (if anything) is at the
                // path. If the placement step never created a file, the
                // remove is a no-op; if it did, we leave nothing behind.
                Err(e)
            }
        }
    }
}

/// Place `src`'s content at `tmp_path` according to `strategy`.
///
/// On entry, no file should exist at `tmp_path` — both reflink and the
/// fallback streaming copy create the file.
fn place_content(src: &Path, tmp_path: &Path, strategy: CopyStrategy) -> io::Result<()> {
    match strategy {
        CopyStrategy::SimpleCopy => {
            fs::copy(src, tmp_path)?;
            Ok(())
        }
        CopyStrategy::CowCopy => {
            reflink_copy::reflink_or_copy(src, tmp_path).map(|_| ())
        }
        CopyStrategy::Auto => {
            if cfg!(target_os = "macos") {
                reflink_copy::reflink_or_copy(src, tmp_path).map(|_| ())
            } else {
                fs::copy(src, tmp_path)?;
                Ok(())
            }
        }
    }
}
