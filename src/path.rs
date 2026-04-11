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
    pub fn from_normalized(s: String) -> Self {
        Self { inner: s }
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
        let normalized = logical_normalize(&abs);

        // Must be under repo_root
        let repo_normalized = logical_normalize(repo_root);
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
        let s = rel
            .components()
            .map(|c| match c {
                Component::Normal(os) => os.to_string_lossy().to_string(),
                _ => unreachable!("normalized path should only have Normal components"),
            })
            .collect::<Vec<_>>()
            .join("/");

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
}
