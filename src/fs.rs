//! Filesystem abstraction for testability.

use std::fs;
use std::io;
use std::path::Path;

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

    /// Write data to a temp file in the same directory, then rename atomically.
    fn atomic_write(&self, path: &Path, data: &[u8]) -> io::Result<()>;

    /// Copy file permissions from src to dst (best-effort).
    fn copy_permissions(&self, src: &Path, dst: &Path) -> io::Result<()>;
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

    fn atomic_write(&self, path: &Path, data: &[u8]) -> io::Result<()> {
        use std::io::Write;

        let parent = path.parent().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "path has no parent directory")
        })?;

        // Create a temp file in the same directory
        let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
        tmp.write_all(data)?;
        tmp.flush()?;

        // Atomically rename into place
        tmp.persist(path).map_err(|e| e.error)?;
        Ok(())
    }

    fn copy_permissions(&self, src: &Path, dst: &Path) -> io::Result<()> {
        let metadata = fs::metadata(src)?;
        let permissions = metadata.permissions();
        fs::set_permissions(dst, permissions)?;
        Ok(())
    }
}
