//! Source-tree grouping for directory-level copy planning.
//!
//! The walk is I/O and stays here. Which directories are copyable, which of
//! those are reported, and which eligible files remain outside them are
//! decided by the Verus-verified `groups` kernel in `waft-verified`.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs;
use std::path::Path;

use waft_verified::groups::{self, ChildKind, PathKey};

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
        groups::sort_unique(&mut files);
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
    ignore_case: bool,
) -> Result<EligibilityGroups> {
    let eligible_set: BTreeSet<RepoRelPath> = eligible.into_iter().collect();
    let mut ctx = WalkContext {
        source_root,
        gitlinks,
        ignore_case,
        eligible: &eligible_set,
        copyable_dirs: BTreeMap::new(),
    };

    ctx.walk_dir(source_root, 0)?;

    // Report only the outermost copyable directories: the verified kernel
    // keeps a directory exactly when no other copyable directory is a
    // `/`-ancestor of it.
    let dir_keys: Vec<&[u8]> = ctx.copyable_dirs.keys().map(|dir| dir.as_bytes()).collect();
    let outermost = groups::outermost(&dir_keys);
    let mut full_dirs = Vec::new();
    for ((dir, files), keep) in ctx.copyable_dirs.iter().zip(outermost) {
        if !keep {
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

    // Every eligible path not covered by a reported directory remains, in
    // eligible-set order.
    let covered_keys: Vec<&[u8]> = covered.iter().map(PathKey::key_bytes).collect();
    let eligible_keys: Vec<&[u8]> = eligible_set.iter().map(PathKey::key_bytes).collect();
    let uncovered = groups::uncovered(&eligible_keys, &covered_keys);
    let remaining_files = eligible_set
        .iter()
        .zip(uncovered)
        .filter(|(_, uncovered)| *uncovered)
        .map(|(path, _)| path.clone())
        .collect();

    Ok(EligibilityGroups {
        full_dirs,
        remaining_files,
    })
}

struct WalkContext<'a> {
    source_root: &'a Path,
    gitlinks: &'a HashSet<String>,
    /// Whether this checkout folds case, for gitlink boundary comparison.
    ignore_case: bool,
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
        // What each entry turned out to be; the verified kernel decides from
        // these whether this directory is copyable.
        let mut children = Vec::new();
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
                    self.ignore_case,
                ) {
                    children.push(ChildKind::GitBoundary);
                    continue;
                }

                let child = self.walk_dir(&path, depth + 1)?;
                if child.copyable {
                    eligible_files.extend(child.eligible_files);
                }
                children.push(ChildKind::Directory {
                    copyable: child.copyable,
                });
                continue;
            }

            let rel = match RepoRelPath::normalize(&path, self.source_root) {
                Ok(rel) => rel,
                Err(_) => {
                    children.push(ChildKind::Unnormalizable);
                    continue;
                }
            };

            if file_type.is_symlink() {
                children.push(ChildKind::Symlink);
            } else if file_type.is_file() {
                let eligible = self.eligible.contains(&rel);
                if eligible {
                    eligible_files.insert(rel);
                }
                children.push(ChildKind::RegularFile { eligible });
            } else {
                children.push(ChildKind::Other);
            }
        }

        // Copyable exactly when the directory has an entry and every entry is
        // an eligible regular file or a copyable directory. Because a
        // copyable directory always contributes at least one file, this is
        // the same as "nothing blocked it and it has an eligible file".
        let copyable = groups::dir_copyable(&children);
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

/// A repository-relative path as the grouping kernels see it: its UTF-8
/// bytes, which order exactly like the path itself (`String`'s order is
/// byte-lexicographic).
impl PathKey for RepoRelPath {
    fn key_bytes(&self) -> &[u8] {
        self.as_str().as_bytes()
    }
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
            false,
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

        let groups = compute(tmp.path(), vec![rel("cfg/a.conf")], &gitlinks, false).unwrap();

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

    /// `cfg-extra` sorts between `cfg` and `cfg/nested` (`-` is below `/`), so
    /// the nearest copyable directory before `cfg/nested` is not its parent.
    #[test]
    fn outermost_dirs_are_found_across_interleaved_siblings() {
        let tmp = TempDir::new().unwrap();
        write(tmp.path(), "cfg/a.conf");
        write(tmp.path(), "cfg/nested/b.conf");
        write(tmp.path(), "cfg-extra/c.conf");
        write(tmp.path(), "top.env");

        let groups = groups(
            tmp.path(),
            &[
                "cfg/a.conf",
                "cfg/nested/b.conf",
                "cfg-extra/c.conf",
                "top.env",
            ],
        );

        let dirs: Vec<&str> = groups
            .full_dirs
            .iter()
            .map(|dir| dir.rel_path.as_str())
            .collect();
        assert_eq!(dirs, ["cfg", "cfg-extra"]);
        assert_eq!(
            groups.full_dirs[0].files,
            vec![rel("cfg/a.conf"), rel("cfg/nested/b.conf")]
        );
        assert_eq!(groups.remaining_files, vec![rel("top.env")]);
    }

    /// An eligible path the walk saw as a directory is not one of that
    /// directory's files, so it stays in the remaining files.
    #[test]
    fn eligible_path_that_is_a_directory_remains() {
        let tmp = TempDir::new().unwrap();
        write(tmp.path(), "cfg/inner/x.conf");

        let groups = groups(tmp.path(), &["cfg/inner", "cfg/inner/x.conf"]);

        assert_eq!(groups.full_dirs.len(), 1);
        assert_eq!(groups.full_dirs[0].rel_path.as_str(), "cfg");
        assert_eq!(groups.remaining_files, vec![rel("cfg/inner")]);
    }

    /// The kernels do not rely on their inputs being sorted: unsorted input
    /// takes the linear path and gives the same answers.
    #[test]
    fn grouping_kernels_accept_unsorted_input() {
        let dirs: [&[u8]; 4] = [b"a/b/c", b"a", b"a-b", b"a-b/c"];
        assert_eq!(groups::outermost(&dirs), [false, true, true, false]);

        let sorted: [&[u8]; 4] = [b"a", b"a-b", b"a-b/c", b"a/b/c"];
        assert_eq!(groups::outermost(&sorted), [true, true, false, false]);

        let eligible: [&[u8]; 3] = [b"x", b"y", b"z"];
        let covered: [&[u8]; 2] = [b"z", b"x"];
        assert_eq!(groups::uncovered(&eligible, &covered), [false, true, false]);
        let covered_sorted: [&[u8]; 2] = [b"x", b"z"];
        assert_eq!(
            groups::uncovered(&eligible, &covered_sorted),
            [false, true, false]
        );
    }

    #[test]
    fn from_files_sorts_and_deduplicates_like_std() {
        let files = [
            "b/é.env", "a-b/c", "a/b", "a", "b/e.env", "a/b", "Z", "a", "b/é.env",
        ];
        let mut expected: Vec<RepoRelPath> = files.iter().copied().map(rel).collect();
        expected.sort();
        expected.dedup();

        let groups = EligibilityGroups::from_files(files.iter().copied().map(rel).collect());

        assert!(groups.full_dirs.is_empty());
        assert_eq!(groups.remaining_files, expected);
    }

    proptest::proptest! {
        /// The verified sort agrees with `sort` + `dedup` on `RepoRelPath`'s
        /// own order, including multi-byte UTF-8.
        #[test]
        fn sort_unique_matches_std_sort_and_dedup(
            paths in proptest::collection::vec("[a-c/.é-]{1,5}", 0..24)
        ) {
            let mut expected: Vec<RepoRelPath> = paths.iter().cloned().map(RepoRelPath::from_normalized).collect();
            expected.sort();
            expected.dedup();

            let mut actual: Vec<RepoRelPath> = paths.into_iter().map(RepoRelPath::from_normalized).collect();
            groups::sort_unique(&mut actual);

            proptest::prop_assert_eq!(actual, expected);
        }
    }

    #[test]
    fn copyable_verdict_requires_an_entry_and_only_eligible_entries() {
        use ChildKind::*;
        assert!(!groups::dir_copyable(&[]));
        assert!(groups::dir_copyable(&[RegularFile { eligible: true }]));
        assert!(groups::dir_copyable(&[
            Directory { copyable: true },
            RegularFile { eligible: true },
        ]));
        for blocker in [
            GitBoundary,
            Directory { copyable: false },
            Unnormalizable,
            Symlink,
            RegularFile { eligible: false },
            Other,
        ] {
            assert!(
                !groups::dir_copyable(&[RegularFile { eligible: true }, blocker]),
                "{blocker:?}"
            );
        }
    }
}
