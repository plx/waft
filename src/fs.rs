//! Filesystem abstraction for testability.

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

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

    /// Copy exactly the manifest of regular files under `src` into a new
    /// destination directory `dst`.
    ///
    /// `expected_files` are relative to `src`. Implementations must not copy
    /// symlinks, special files, empty directories, or entries absent from the
    /// manifest. `dst` must not already exist, and its parent must exist.
    fn copy_dir_exact(
        &self,
        src: &Path,
        dst: &Path,
        expected_files: &[PathBuf],
        strategy: CopyStrategy,
    ) -> io::Result<()>;
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

    fn copy_dir_exact(
        &self,
        src: &Path,
        dst: &Path,
        expected_files: &[PathBuf],
        strategy: CopyStrategy,
    ) -> io::Result<()> {
        match fs::symlink_metadata(dst) {
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "destination already exists",
                ));
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }

        let parent = dst.parent().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "path has no parent directory")
        })?;
        let parent_metadata = fs::symlink_metadata(parent)?;
        if !parent_metadata.file_type().is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "destination parent is not a directory",
            ));
        }

        let normalized_manifest = normalize_manifest(expected_files)?;
        let preflight = preflight_dir_exact(src, &normalized_manifest)?;

        // First try `clonefile` if the source is exact and the strategy
        // permits. `clonefile` requires a non-existing destination, so we
        // allocate a unique tempdir and remove the placeholder before the
        // call. That briefly exposes the name to a symlink race; on any
        // failure (including a racing symlink causing EEXIST) we abandon
        // that name entirely and fall through to the streaming path with
        // a freshly-allocated tempdir, so the fallback never writes into
        // a path an attacker may have substituted.
        if preflight.exact && should_try_dir_clone(strategy) {
            let clone_dir = tempfile::Builder::new()
                .prefix(".waft-copy-dir-")
                .tempdir_in(parent)?;
            let clone_path = clone_dir.keep();
            fs::remove_dir(&clone_path)?;
            match crate::sys::clonefile::clonefile_dir(src, &clone_path) {
                Ok(()) => {
                    if let Err(e) = fs::rename(&clone_path, dst) {
                        let _ = remove_dir_all_if_exists(&clone_path);
                        return Err(e);
                    }
                    return Ok(());
                }
                Err(_) => {
                    let _ = remove_dir_all_if_exists(&clone_path);
                }
            }
        }

        // Streaming fallback. The tempdir is created atomically (mkdtemp)
        // and we never remove it before writing, so we own this path
        // end-to-end and no symlink race window exists.
        let temp_dir = tempfile::Builder::new()
            .prefix(".waft-copy-dir-")
            .tempdir_in(parent)?;
        let temp_path = temp_dir.keep();

        let result = (|| {
            self.copy_manifest_to_temp(src, &temp_path, &normalized_manifest, strategy)?;
            fs::rename(&temp_path, dst)?;
            Ok(())
        })();

        if let Err(error) = result {
            let _ = remove_dir_all_if_exists(&temp_path);
            return Err(error);
        }

        Ok(())
    }
}

impl RealFs {
    fn copy_manifest_to_temp(
        &self,
        src: &Path,
        temp: &Path,
        expected_files: &[PathBuf],
        strategy: CopyStrategy,
    ) -> io::Result<()> {
        for rel in expected_files {
            let src_file = src.join(rel);
            let dst_file = temp.join(rel);
            if let Some(parent) = dst_file.parent() {
                fs::create_dir_all(parent)?;
            }
            self.copy_file(&src_file, &dst_file, strategy)?;
        }
        Ok(())
    }
}

#[derive(Debug)]
struct ManifestPreflight {
    exact: bool,
}

#[derive(Debug, Default)]
struct ScanOutcome {
    exact: bool,
    has_manifest_file: bool,
}

fn normalize_manifest(expected_files: &[PathBuf]) -> io::Result<Vec<PathBuf>> {
    if expected_files.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "directory copy manifest is empty",
        ));
    }

    let mut normalized = expected_files
        .iter()
        .map(|path| {
            if path.is_absolute() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "manifest path must be relative",
                ));
            }
            let mut result = PathBuf::new();
            for component in path.components() {
                match component {
                    Component::Normal(part) => result.push(part),
                    Component::CurDir => {}
                    Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidInput,
                            "manifest path escapes source directory",
                        ));
                    }
                }
            }
            if result.as_os_str().is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "manifest path is empty",
                ));
            }
            Ok(result)
        })
        .collect::<io::Result<Vec<_>>>()?;

    normalized.sort();
    normalized.dedup();
    if normalized.len() != expected_files.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "directory copy manifest contains duplicates",
        ));
    }
    Ok(normalized)
}

fn preflight_dir_exact(src: &Path, expected_files: &[PathBuf]) -> io::Result<ManifestPreflight> {
    let expected: std::collections::BTreeSet<PathBuf> = expected_files.iter().cloned().collect();
    let mut found = std::collections::BTreeSet::new();
    let scan = scan_manifest_dir(src, Path::new(""), &expected, &mut found)?;

    if let Some(rel) = expected.difference(&found).next() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "expected manifest file missing or not regular: {}",
                rel.display()
            ),
        ));
    }

    Ok(ManifestPreflight { exact: scan.exact })
}

fn scan_manifest_dir(
    dir: &Path,
    rel_dir: &Path,
    expected: &std::collections::BTreeSet<PathBuf>,
    found: &mut std::collections::BTreeSet<PathBuf>,
) -> io::Result<ScanOutcome> {
    let mut exact = true;
    let mut has_manifest_file = false;
    let mut entries = fs::read_dir(dir)?.collect::<std::result::Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        let path = entry.path();
        let rel_path = rel_dir.join(entry.file_name());
        let metadata = fs::symlink_metadata(&path)?;
        let file_type = metadata.file_type();

        if file_type.is_symlink() {
            if expected.contains(&rel_path) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!(
                        "expected manifest file is a symlink: {}",
                        rel_path.display()
                    ),
                ));
            }
            exact = false;
        } else if file_type.is_file() {
            if expected.contains(&rel_path) {
                found.insert(rel_path);
                has_manifest_file = true;
            } else {
                exact = false;
            }
        } else if file_type.is_dir() {
            if !manifest_has_descendant(expected, &rel_path) {
                exact = false;
                continue;
            }
            let child = scan_manifest_dir(&path, &rel_path, expected, found)?;
            if child.has_manifest_file {
                has_manifest_file = true;
            } else {
                exact = false;
            }
            if !child.exact {
                exact = false;
            }
        } else {
            if expected.contains(&rel_path) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!(
                        "expected manifest file is not a regular file: {}",
                        rel_path.display()
                    ),
                ));
            }
            exact = false;
        }
    }

    Ok(ScanOutcome {
        exact,
        has_manifest_file,
    })
}

fn manifest_has_descendant(expected: &std::collections::BTreeSet<PathBuf>, rel_dir: &Path) -> bool {
    expected.iter().any(|file| file.starts_with(rel_dir))
}

fn should_try_dir_clone(strategy: CopyStrategy) -> bool {
    match strategy {
        CopyStrategy::SimpleCopy => false,
        CopyStrategy::CowCopy => cfg!(target_os = "macos"),
        CopyStrategy::Auto => cfg!(target_os = "macos"),
    }
}

fn remove_dir_all_if_exists(path: &Path) -> io::Result<()> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    #[test]
    fn realfs_copy_dir_exact_clones_or_copies_tree() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("src");
        let dst = tmp.path().join("dst");
        write(&src.join("a.conf"), "a\n");
        write(&src.join("nested/b.conf"), "b\n");

        RealFs
            .copy_dir_exact(
                &src,
                &dst,
                &[
                    PathBuf::from("a.conf"),
                    PathBuf::from("nested").join("b.conf"),
                ],
                CopyStrategy::SimpleCopy,
            )
            .unwrap();

        assert_eq!(fs::read_to_string(dst.join("a.conf")).unwrap(), "a\n");
        assert_eq!(
            fs::read_to_string(dst.join("nested/b.conf")).unwrap(),
            "b\n"
        );
    }

    #[test]
    fn realfs_copy_dir_exact_refuses_existing_dst() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("src");
        let dst = tmp.path().join("dst");
        write(&src.join("a.conf"), "a\n");
        fs::create_dir(&dst).unwrap();

        let err = RealFs
            .copy_dir_exact(
                &src,
                &dst,
                &[PathBuf::from("a.conf")],
                CopyStrategy::SimpleCopy,
            )
            .unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
    }

    #[test]
    fn realfs_copy_dir_exact_does_not_copy_empty_dirs() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("src");
        let dst = tmp.path().join("dst");
        write(&src.join("a.conf"), "a\n");
        fs::create_dir_all(src.join("empty")).unwrap();

        RealFs
            .copy_dir_exact(
                &src,
                &dst,
                &[PathBuf::from("a.conf")],
                CopyStrategy::SimpleCopy,
            )
            .unwrap();

        assert!(dst.join("a.conf").is_file());
        assert!(!dst.join("empty").exists());
    }

    #[test]
    fn realfs_copy_dir_exact_does_not_descend_unrelated_extra_dirs() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("src");
        let dst = tmp.path().join("dst");
        write(&src.join("a.conf"), "a\n");
        fs::create_dir_all(src.join("extra/nested")).unwrap();
        write(&src.join("extra/nested/ignored.txt"), "ignored\n");

        RealFs
            .copy_dir_exact(
                &src,
                &dst,
                &[PathBuf::from("a.conf")],
                CopyStrategy::SimpleCopy,
            )
            .unwrap();

        assert!(dst.join("a.conf").is_file());
        assert!(!dst.join("extra").exists());
    }

    #[cfg(unix)]
    #[test]
    fn realfs_copy_dir_exact_rejects_expected_symlink() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("src");
        let dst = tmp.path().join("dst");
        fs::create_dir(&src).unwrap();
        std::os::unix::fs::symlink("target", src.join("link.env")).unwrap();

        let err = RealFs
            .copy_dir_exact(
                &src,
                &dst,
                &[PathBuf::from("link.env")],
                CopyStrategy::SimpleCopy,
            )
            .unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert!(!dst.exists());
    }

    #[cfg(unix)]
    #[test]
    fn realfs_copy_dir_exact_cleans_temp_on_failure() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("src");
        let dst = tmp.path().join("dst");
        let unreadable = src.join("a.conf");
        write(&unreadable, "a\n");
        fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o000)).unwrap();

        let result = RealFs.copy_dir_exact(
            &src,
            &dst,
            &[PathBuf::from("a.conf")],
            CopyStrategy::SimpleCopy,
        );

        fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o600)).unwrap();
        if result.is_ok() {
            return;
        }

        assert!(!dst.exists());
        let leftovers = fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".waft-copy-dir-")
            })
            .count();
        assert_eq!(leftovers, 0);
    }
}
