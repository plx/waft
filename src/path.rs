//! Repo-relative path type and normalization.

use std::fmt;
use std::path::{Component, Path, PathBuf};

use crate::error::{Error, Result};

/// A normalized, repo-relative path.
///
/// Invariants:
/// - No leading `/` or `\`
/// - No `.` or `..` components
/// - Uses `/` as separator (even on Windows)
/// - Never empty (the repo root is not a valid file path)
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RepoRelPath {
    inner: String,
}

impl RepoRelPath {
    /// Create a `RepoRelPath` from a pre-normalized string.
    ///
    /// This does **not** validate — use [`RepoRelPath::normalize`] for untrusted input.
    pub(crate) fn from_normalized(s: String) -> Self {
        Self { inner: s }
    }

    /// Create a repository-relative path from Git's raw path bytes.
    ///
    /// waft's public output and configuration formats are UTF-8. Reject an
    /// unrepresentable path instead of lossily converting it and risking a
    /// collision with a different destination filename.
    pub(crate) fn from_git_bytes(bytes: &[u8]) -> Result<Self> {
        let value = std::str::from_utf8(bytes).map_err(|_| Error::InvalidPath {
            message: "repository path is not valid UTF-8 and is unsupported".to_string(),
        })?;
        validate_text_path(value)?;
        Ok(Self {
            inner: value.to_string(),
        })
    }

    /// Normalize a path relative to a repo root.
    ///
    /// Accepts absolute or relative paths. Rejects paths that escape the repo
    /// via `..` or resolve to the repo root itself.
    pub fn normalize(path: &Path, repo_root: &Path) -> Result<Self> {
        let abs = if path.is_absolute() {
            path.to_path_buf()
        } else {
            repo_root.join(path)
        };

        // Logical normalization (resolve `.` and `..` without touching the filesystem)
        let normalized = logical_normalize(&without_windows_verbatim_prefix(&abs));

        // Must be under repo_root
        let repo_normalized = logical_normalize(&without_windows_verbatim_prefix(repo_root));
        let rel = normalized
            .strip_prefix(&repo_normalized)
            .map_err(|_| Error::InvalidPath {
                message: format!(
                    "path {} is outside the repository root {}",
                    path.display(),
                    repo_root.display()
                ),
            })?;

        if rel.as_os_str().is_empty() {
            return Err(Error::InvalidPath {
                message: "path resolves to the repository root".to_string(),
            });
        }

        // Convert to forward-slash string
        let mut parts = Vec::new();
        for component in rel.components() {
            let Component::Normal(os) = component else {
                unreachable!("normalized path should only have Normal components");
            };
            let value = os.to_str().ok_or_else(|| Error::InvalidPath {
                message: format!(
                    "path {} is not valid UTF-8 and is unsupported",
                    path.display()
                ),
            })?;
            parts.push(value.to_string());
        }
        let s = parts.join("/");
        validate_text_path(&s)?;

        Ok(Self { inner: s })
    }

    /// Return the path as a string slice.
    pub fn as_str(&self) -> &str {
        &self.inner
    }

    /// Convert to a `PathBuf` relative to the given root.
    pub fn to_path(&self, root: &Path) -> PathBuf {
        root.join(self.inner.replace('/', std::path::MAIN_SEPARATOR_STR))
    }
}

/// Remove the extended-length prefix returned by Windows canonicalization.
///
/// Windows considers `C:\repo` and `\\?\C:\repo` the same path, but
/// `Path::strip_prefix` does not. Normalizing both spellings at repository
/// boundaries keeps containment checks byte-preserving on Unix while making
/// canonicalized Windows inputs comparable with Git's ordinary paths.
#[cfg(windows)]
pub(crate) fn without_windows_verbatim_prefix(path: &Path) -> PathBuf {
    let Some(path) = path.to_str() else {
        return path.to_path_buf();
    };
    if let Some(unc) = path.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{unc}"))
    } else if let Some(ordinary) = path.strip_prefix(r"\\?\") {
        PathBuf::from(ordinary)
    } else {
        PathBuf::from(path)
    }
}

#[cfg(not(windows))]
pub(crate) fn without_windows_verbatim_prefix(path: &Path) -> PathBuf {
    path.to_path_buf()
}

fn validate_text_path(value: &str) -> Result<()> {
    if value.contains(['\n', '\r', '\0']) {
        return Err(Error::InvalidPath {
            message: "repository paths containing NUL or newlines are unsupported".to_string(),
        });
    }
    Ok(())
}

impl fmt::Display for RepoRelPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.inner)
    }
}

impl AsRef<str> for RepoRelPath {
    fn as_ref(&self) -> &str {
        &self.inner
    }
}

/// Logically normalize a path by resolving `.` and `..` without filesystem access.
fn logical_normalize(path: &Path) -> PathBuf {
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            Component::Prefix(p) => components.push(Component::Prefix(p)),
            Component::RootDir => {
                components.retain(|c| matches!(c, Component::Prefix(_)));
                components.push(Component::RootDir);
            }
            Component::CurDir => {}
            Component::ParentDir => {
                if let Some(last) = components.last() {
                    if matches!(last, Component::Normal(_)) {
                        components.pop();
                    } else {
                        components.push(Component::ParentDir);
                    }
                } else {
                    components.push(Component::ParentDir);
                }
            }
            Component::Normal(_) => components.push(component),
        }
    }
    components.iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_simple_relative() {
        let root = Path::new("/repo");
        let p = RepoRelPath::normalize(Path::new("src/main.rs"), root).unwrap();
        assert_eq!(p.as_str(), "src/main.rs");
    }

    #[test]
    fn normalize_absolute_under_root() {
        let root = Path::new("/repo");
        let p = RepoRelPath::normalize(Path::new("/repo/src/lib.rs"), root).unwrap();
        assert_eq!(p.as_str(), "src/lib.rs");
    }

    #[test]
    fn normalize_with_dot_components() {
        let root = Path::new("/repo");
        let p = RepoRelPath::normalize(Path::new("./src/./main.rs"), root).unwrap();
        assert_eq!(p.as_str(), "src/main.rs");
    }

    #[test]
    fn normalize_with_dotdot_staying_inside() {
        let root = Path::new("/repo");
        let p = RepoRelPath::normalize(Path::new("src/sub/../main.rs"), root).unwrap();
        assert_eq!(p.as_str(), "src/main.rs");
    }

    #[test]
    fn reject_path_escaping_root() {
        let root = Path::new("/repo");
        let err = RepoRelPath::normalize(Path::new("../outside"), root).unwrap_err();
        assert!(err.to_string().contains("outside the repository root"));
    }

    #[test]
    fn reject_absolute_outside_root() {
        let root = Path::new("/repo");
        let err = RepoRelPath::normalize(Path::new("/other/file"), root).unwrap_err();
        assert!(err.to_string().contains("outside the repository root"));
    }

    #[test]
    fn reject_repo_root_itself() {
        let root = Path::new("/repo");
        let err = RepoRelPath::normalize(Path::new("."), root).unwrap_err();
        assert!(err.to_string().contains("resolves to the repository root"));
    }

    #[test]
    fn reject_repo_root_absolute() {
        let root = Path::new("/repo");
        let err = RepoRelPath::normalize(Path::new("/repo"), root).unwrap_err();
        assert!(err.to_string().contains("resolves to the repository root"));
    }

    #[test]
    fn to_path_reconstructs_absolute() {
        let root = Path::new("/repo");
        let p = RepoRelPath::normalize(Path::new("src/main.rs"), root).unwrap();
        assert_eq!(p.to_path(root), PathBuf::from("/repo/src/main.rs"));
    }

    #[test]
    fn display_shows_forward_slashes() {
        let p = RepoRelPath::from_normalized("a/b/c".to_string());
        assert_eq!(format!("{p}"), "a/b/c");
    }

    #[test]
    fn ordering_is_lexical() {
        let a = RepoRelPath::from_normalized("a/b".to_string());
        let b = RepoRelPath::from_normalized("a/c".to_string());
        let c = RepoRelPath::from_normalized("b/a".to_string());
        assert!(a < b);
        assert!(b < c);
    }

    #[test]
    fn normalize_deeply_nested_dotdot() {
        let root = Path::new("/repo");
        let p = RepoRelPath::normalize(Path::new("a/b/c/../../d/e"), root).unwrap();
        assert_eq!(p.as_str(), "a/d/e");
    }

    #[test]
    fn reject_dotdot_escaping_via_deep_path() {
        let root = Path::new("/repo");
        let err = RepoRelPath::normalize(Path::new("a/../../outside"), root).unwrap_err();
        assert!(err.to_string().contains("outside the repository root"));
    }

    #[test]
    fn reject_newline_path_that_would_corrupt_line_output() {
        let root = Path::new("/repo");
        let err = RepoRelPath::normalize(Path::new("line\nbreak.env"), root).unwrap_err();
        assert!(err.to_string().contains("newlines"));
    }

    #[cfg(unix)]
    #[test]
    fn reject_non_utf8_path_without_lossy_collision() {
        use std::os::unix::ffi::OsStrExt;

        let root = Path::new("/repo");
        let invalid = std::ffi::OsStr::from_bytes(b"bad-\xff.env");
        let err = RepoRelPath::normalize(Path::new(invalid), root).unwrap_err();
        assert!(err.to_string().contains("not valid UTF-8"));
    }

    #[cfg(windows)]
    #[test]
    fn normalize_accepts_verbatim_path_under_ordinary_root() {
        let root = Path::new(r"C:\repo");
        let path = Path::new(r"\\?\C:\repo\config\.env");
        let normalized = RepoRelPath::normalize(path, root).unwrap();
        assert_eq!(normalized.as_str(), "config/.env");
    }

    #[cfg(windows)]
    #[test]
    fn normalize_accepts_verbatim_unc_path_under_ordinary_root() {
        let root = Path::new(r"\\server\share\repo");
        let path = Path::new(r"\\?\UNC\server\share\repo\.env");
        let normalized = RepoRelPath::normalize(path, root).unwrap();
        assert_eq!(normalized.as_str(), ".env");
    }
}
