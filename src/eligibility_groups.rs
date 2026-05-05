//! Source-tree grouping for directory-level copy planning.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs;
use std::path::Path;

use crate::error::{Error, Result};
use crate::path::RepoRelPath;

/// A directory whose physical source subtree can be copied as one unit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EligibleDir {
    /// Repo-relative directory path. The repository root is never represented.
    pub rel_path: RepoRelPath,
    /// Eligible regular files covered by this directory.
    pub files: Vec<RepoRelPath>,
}

/// Directory grouping result consumed by the planner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EligibilityGroups {
    /// Maximal fully copyable directories, sorted by repo-relative path.
    pub full_dirs: Vec<EligibleDir>,
    /// Eligible paths not covered by a full directory, sorted.
    pub remaining_files: Vec<RepoRelPath>,
}

impl EligibilityGroups {
    /// Build a grouping with no directory promotion.
    pub fn from_files(mut files: Vec<RepoRelPath>) -> Self {
        files.sort();
        files.dedup();
        Self {
            full_dirs: Vec::new(),
            remaining_files: files,
        }
    }
}

/// Compute maximal fully copyable source directories from eligible paths.
pub fn compute(
    source_root: &Path,
    eligible: Vec<RepoRelPath>,
    gitlinks: &HashSet<String>,
) -> Result<EligibilityGroups> {
    let eligible_set: BTreeSet<RepoRelPath> = eligible.into_iter().collect();
    let mut ctx = WalkContext {
        source_root,
        gitlinks,
        eligible: &eligible_set,
        copyable_dirs: BTreeMap::new(),
    };

    ctx.walk_dir(source_root, 0)?;

    let mut full_dirs = Vec::new();
    for (dir, files) in &ctx.copyable_dirs {
        if has_copyable_ancestor(dir, &ctx.copyable_dirs) {
            continue;
        }
        full_dirs.push(EligibleDir {
            rel_path: RepoRelPath::from_normalized(dir.clone()),
            files: files.iter().cloned().collect(),
        });
    }

    let mut covered = BTreeSet::new();
    for dir in &full_dirs {
        covered.extend(dir.files.iter().cloned());
    }

    let remaining_files = eligible_set.difference(&covered).cloned().collect();

    Ok(EligibilityGroups {
        full_dirs,
        remaining_files,
    })
}

struct WalkContext<'a> {
    source_root: &'a Path,
    gitlinks: &'a HashSet<String>,
    eligible: &'a BTreeSet<RepoRelPath>,
    copyable_dirs: BTreeMap<String, BTreeSet<RepoRelPath>>,
}

#[derive(Debug, Default)]
struct DirState {
    copyable: bool,
    eligible_files: BTreeSet<RepoRelPath>,
}

impl WalkContext<'_> {
    fn walk_dir(&mut self, dir: &Path, depth: usize) -> Result<DirState> {
        let mut blocked = false;
        let mut eligible_files = BTreeSet::new();

        let mut entries = fs::read_dir(dir)
            .map_err(|e| Error::Io {
                context: format!("walking source directory {}", dir.display()),
                source: e,
            })?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| Error::Io {
                context: format!("walking source directory {}", dir.display()),
                source: e,
            })?;
        entries.sort_by_key(|e| e.file_name());

        for entry in entries {
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path).map_err(|e| Error::Io {
                context: format!("reading metadata for {}", path.display()),
                source: e,
            })?;
            let file_type = metadata.file_type();

            if file_type.is_dir() {
                if crate::walk::is_git_boundary_dir(
                    &path,
                    depth + 1,
                    self.source_root,
                    self.gitlinks,
                ) {
                    blocked = true;
                    continue;
                }

                let child = self.walk_dir(&path, depth + 1)?;
                if child.copyable {
                    eligible_files.extend(child.eligible_files);
                } else {
                    blocked = true;
                }
                continue;
            }

            let rel = match RepoRelPath::normalize(&path, self.source_root) {
                Ok(rel) => rel,
                Err(_) => {
                    blocked = true;
                    continue;
                }
            };

            if file_type.is_symlink() {
                blocked = true;
            } else if file_type.is_file() {
                if self.eligible.contains(&rel) {
                    eligible_files.insert(rel);
                } else {
                    blocked = true;
                }
            } else {
                blocked = true;
            }
        }

        let copyable = !blocked && !eligible_files.is_empty();
        if copyable && depth > 0 {
            let rel = RepoRelPath::normalize(dir, self.source_root)?;
            self.copyable_dirs
                .insert(rel.as_str().to_string(), eligible_files.clone());
        }

        Ok(DirState {
            copyable,
            eligible_files,
        })
    }
}

fn has_copyable_ancestor(
    dir: &str,
    copyable_dirs: &BTreeMap<String, BTreeSet<RepoRelPath>>,
) -> bool {
    let mut current = dir;
    while let Some((parent, _)) = current.rsplit_once('/') {
        if copyable_dirs.contains_key(parent) {
            return true;
        }
        current = parent;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn rel(path: &str) -> RepoRelPath {
        RepoRelPath::from_normalized(path.to_string())
    }

    fn write(root: &Path, path: &str) {
        let abs = root.join(path);
        if let Some(parent) = abs.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(abs, "x").unwrap();
    }

    fn groups(root: &Path, eligible: &[&str]) -> EligibilityGroups {
        compute(
            root,
            eligible.iter().copied().map(rel).collect(),
            &HashSet::new(),
        )
        .unwrap()
    }

    #[test]
    fn all_files_under_top_level_dir_marks_that_dir() {
        let tmp = TempDir::new().unwrap();
        write(tmp.path(), "cfg/a.conf");
        write(tmp.path(), "cfg/nested/b.conf");

        let groups = groups(tmp.path(), &["cfg/a.conf", "cfg/nested/b.conf"]);

        assert_eq!(groups.full_dirs.len(), 1);
        assert_eq!(groups.full_dirs[0].rel_path.as_str(), "cfg");
        assert_eq!(groups.remaining_files, Vec::<RepoRelPath>::new());
    }

    #[test]
    fn root_is_never_promoted() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir(tmp.path().join(".git")).unwrap();
        write(tmp.path(), "a.env");

        let groups = groups(tmp.path(), &["a.env"]);

        assert!(groups.full_dirs.is_empty());
        assert_eq!(groups.remaining_files, vec![rel("a.env")]);
    }

    #[test]
    fn partial_directory_does_not_promote() {
        let tmp = TempDir::new().unwrap();
        write(tmp.path(), "cfg/a.conf");
        write(tmp.path(), "cfg/b.conf");

        let groups = groups(tmp.path(), &["cfg/a.conf"]);

        assert!(groups.full_dirs.is_empty());
        assert_eq!(groups.remaining_files, vec![rel("cfg/a.conf")]);
    }

    #[test]
    fn nested_full_subdir_under_partial_root() {
        let tmp = TempDir::new().unwrap();
        write(tmp.path(), "cfg/a.conf");
        write(tmp.path(), "cfg/nested/b.conf");

        let groups = groups(tmp.path(), &["cfg/nested/b.conf"]);

        assert_eq!(groups.full_dirs.len(), 1);
        assert_eq!(groups.full_dirs[0].rel_path.as_str(), "cfg/nested");
        assert!(groups.remaining_files.is_empty());
    }

    #[test]
    fn maximal_only() {
        let tmp = TempDir::new().unwrap();
        write(tmp.path(), "cfg/a.conf");
        write(tmp.path(), "cfg/nested/b.conf");

        let groups = groups(tmp.path(), &["cfg/a.conf", "cfg/nested/b.conf"]);

        assert_eq!(groups.full_dirs.len(), 1);
        assert_eq!(groups.full_dirs[0].rel_path.as_str(), "cfg");
    }

    #[test]
    fn symlink_blocks_ancestor_and_remains_remaining_when_eligible() {
        let tmp = TempDir::new().unwrap();
        write(tmp.path(), "cfg/a.conf");
        #[cfg(unix)]
        std::os::unix::fs::symlink("target", tmp.path().join("cfg/link.env")).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_file("target", tmp.path().join("cfg/link.env")).unwrap();

        let groups = groups(tmp.path(), &["cfg/a.conf", "cfg/link.env"]);

        assert!(groups.full_dirs.is_empty());
        assert_eq!(
            groups.remaining_files,
            vec![rel("cfg/a.conf"), rel("cfg/link.env")]
        );
    }

    #[test]
    fn empty_dir_blocks_ancestor() {
        let tmp = TempDir::new().unwrap();
        write(tmp.path(), "cfg/a.conf");
        fs::create_dir_all(tmp.path().join("cfg/empty")).unwrap();

        let groups = groups(tmp.path(), &["cfg/a.conf"]);

        assert!(groups.full_dirs.is_empty());
        assert_eq!(groups.remaining_files, vec![rel("cfg/a.conf")]);
    }

    #[test]
    fn gitlink_blocks_ancestor() {
        let tmp = TempDir::new().unwrap();
        write(tmp.path(), "cfg/a.conf");
        fs::create_dir_all(tmp.path().join("cfg/sub")).unwrap();
        write(tmp.path(), "cfg/sub/inner.env");
        let mut gitlinks = HashSet::new();
        gitlinks.insert("cfg/sub".to_string());

        let groups = compute(tmp.path(), vec![rel("cfg/a.conf")], &gitlinks).unwrap();

        assert!(groups.full_dirs.is_empty());
        assert_eq!(groups.remaining_files, vec![rel("cfg/a.conf")]);
    }

    #[test]
    fn nested_git_repo_blocks_ancestor() {
        let tmp = TempDir::new().unwrap();
        write(tmp.path(), "cfg/a.conf");
        fs::create_dir_all(tmp.path().join("cfg/nested/.git")).unwrap();
        write(tmp.path(), "cfg/nested/inner.env");

        let groups = groups(tmp.path(), &["cfg/a.conf"]);

        assert!(groups.full_dirs.is_empty());
        assert_eq!(groups.remaining_files, vec![rel("cfg/a.conf")]);
    }

    #[test]
    fn tracked_or_other_ineligible_file_blocks_ancestor() {
        let tmp = TempDir::new().unwrap();
        write(tmp.path(), "cfg/a.conf");
        write(tmp.path(), "cfg/tracked.txt");

        let groups = groups(tmp.path(), &["cfg/a.conf"]);

        assert!(groups.full_dirs.is_empty());
        assert_eq!(groups.remaining_files, vec![rel("cfg/a.conf")]);
    }
}
