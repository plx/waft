//! Repo and worktree context resolution.
//!
//! Turns CLI inputs into a validated [`RepoContext`] by querying Git for
//! worktree topology. The resolution rules are:
//!
//! - If running inside a linked worktree with no explicit `--source`/`--dest`,
//!   source = main worktree, dest = current worktree.
//! - If running inside the main worktree, `list` and `info` default source to
//!   current, but `copy` requires `--dest`.
//! - `copy` rejects source == dest and destinations outside the worktree family.

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::git::GitBackend;
use crate::model::RepoContext;

/// What kind of command we're resolving context for.
///
/// This affects whether a destination is required.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandKind {
    /// `copy` requires a destination.
    Copy,
    /// `list` does not require a destination.
    List,
    /// `info` does not require a destination.
    Info,
    /// `validate` does not require a destination.
    Validate,
}

/// Resolve CLI inputs into a validated [`RepoContext`].
pub fn resolve_context(
    git: &dyn GitBackend,
    source_arg: Option<&Path>,
    dest_arg: Option<&Path>,
    directory_arg: Option<&Path>,
    command: CommandKind,
) -> Result<RepoContext> {
    // Determine the working directory for resolution
    let cwd = if let Some(dir) = directory_arg {
        if dir.is_absolute() {
            dir.to_path_buf()
        } else {
            std::env::current_dir()
                .map_err(|e| Error::Io {
                    context: "getting current directory".to_string(),
                    source: e,
                })?
                .join(dir)
        }
    } else {
        std::env::current_dir().map_err(|e| Error::Io {
            context: "getting current directory".to_string(),
            source: e,
        })?
    };

    // Resolve source and dest to absolute paths
    let source_path = source_arg.map(|p| resolve_abs(p, &cwd));
    let dest_path = dest_arg.map(|p| resolve_abs(p, &cwd));

    // Find the repo root from whatever path we have
    let probe_path = source_path
        .as_deref()
        .or(dest_path.as_deref())
        .unwrap_or(&cwd);

    let toplevel = git.show_toplevel(probe_path)?;

    // List all worktrees
    let worktrees = git.list_worktrees(&toplevel)?;
    let main_wt = worktrees
        .iter()
        .find(|w| w.is_main)
        .ok_or_else(|| Error::Git {
            message: "no main worktree found".to_string(),
        })?
        .path
        .clone();

    let known_worktrees: Vec<PathBuf> = worktrees.iter().map(|w| w.path.clone()).collect();

    // Determine source and destination
    let (source_root, dest_root) = if let Some(src) = source_path {
        let src_toplevel = git.show_toplevel(&src)?;
        (
            src_toplevel,
            dest_path.map(|d| git.show_toplevel(&d)).transpose()?,
        )
    } else if let Some(dst) = dest_path {
        // No explicit source, use main worktree as source
        let dst_toplevel = git.show_toplevel(&dst)?;
        (main_wt.clone(), Some(dst_toplevel))
    } else {
        // No explicit source or dest — auto-detect from cwd
        let cwd_toplevel = git.show_toplevel(&cwd)?;
        if cwd_toplevel == main_wt {
            // We're in the main worktree
            (cwd_toplevel, None)
        } else {
            // We're in a linked worktree — source is main, dest is current
            (main_wt.clone(), Some(cwd_toplevel))
        }
    };

    // For copy, destination is required
    if command == CommandKind::Copy && dest_root.is_none() {
        return Err(Error::Context {
            message:
                "copy requires a destination worktree (use --dest or run from a linked worktree)"
                    .to_string(),
        });
    }

    // For copy, enforce source must be the main worktree
    if command == CommandKind::Copy && source_root != main_wt {
        return Err(Error::Context {
            message: format!(
                "copy source must be the main worktree ({}), got {}",
                main_wt.display(),
                source_root.display()
            ),
        });
    }

    // Validate: source must not equal dest
    if let Some(ref dest) = dest_root {
        if source_root == *dest {
            return Err(Error::SameSourceAndDest {
                path: source_root.clone(),
            });
        }

        // For copy, dest must be a linked worktree (not the main one)
        if command == CommandKind::Copy && *dest == main_wt {
            return Err(Error::Context {
                message: "copy destination must be a linked worktree, not the main worktree"
                    .to_string(),
            });
        }

        // Validate: dest must be in the same worktree family
        if !known_worktrees.iter().any(|w| w == dest) {
            return Err(Error::NotInWorktreeFamily {
                src: source_root.clone(),
                dest: dest.clone(),
            });
        }
    }

    // Read core.ignoreCase
    let core_ignore_case = git.read_bool_config(&source_root, "core.ignoreCase")?;

    Ok(RepoContext {
        source_root,
        dest_root,
        main_worktree: main_wt,
        known_worktrees,
        core_ignore_case,
    })
}

fn resolve_abs(path: &Path, cwd: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::{IgnoreCheckRecord, WorktreeRecord};
    use crate::path::RepoRelPath;
    use std::collections::HashSet;

    fn test_path(name: &str) -> PathBuf {
        std::env::current_dir()
            .expect("current dir should be available")
            .join(name)
    }

    fn main_repo_path() -> PathBuf {
        test_path("repo")
    }

    fn linked_repo_path() -> PathBuf {
        test_path("repo-wt")
    }

    fn outside_repo_path() -> PathBuf {
        test_path("other-repo")
    }

    /// A mock Git backend for testing context resolution.
    struct MockGit {
        worktrees: Vec<WorktreeRecord>,
        ignore_case: bool,
    }

    impl MockGit {
        fn new(worktrees: Vec<WorktreeRecord>) -> Self {
            Self {
                worktrees,
                ignore_case: false,
            }
        }
    }

    impl GitBackend for MockGit {
        fn show_toplevel(&self, path: &Path) -> Result<PathBuf> {
            // Return the matching worktree root for the given path
            for wt in &self.worktrees {
                if path.starts_with(&wt.path) || path == wt.path {
                    return Ok(wt.path.clone());
                }
            }
            // For unknown paths, return the path itself (simulates a separate repo)
            Ok(path.to_path_buf())
        }

        fn list_worktrees(&self, _source_root: &Path) -> Result<Vec<WorktreeRecord>> {
            Ok(self.worktrees.clone())
        }

        fn tracked_paths(
            &self,
            _source_root: &Path,
            _paths: &[RepoRelPath],
        ) -> Result<HashSet<RepoRelPath>> {
            Ok(HashSet::new())
        }

        fn gitlinks(&self, _source_root: &Path) -> Result<HashSet<String>> {
            Ok(HashSet::new())
        }

        fn check_ignore(
            &self,
            _source_root: &Path,
            _paths: &[RepoRelPath],
        ) -> Result<Vec<IgnoreCheckRecord>> {
            Ok(Vec::new())
        }

        fn list_worktreeinclude_candidates(
            &self,
            _source_root: &Path,
            _semantics: crate::config::WorktreeincludeSemantics,
            _symlink_policy: crate::config::SymlinkPolicy,
        ) -> Result<Vec<RepoRelPath>> {
            Ok(Vec::new())
        }

        fn list_ignored_untracked(&self, _source_root: &Path) -> Result<Vec<RepoRelPath>> {
            Ok(Vec::new())
        }

        fn worktreeinclude_exists_anywhere(
            &self,
            _source_root: &Path,
            _symlink_policy: crate::config::SymlinkPolicy,
        ) -> Result<bool> {
            Ok(false)
        }

        fn read_bool_config(&self, _source_root: &Path, _key: &str) -> Result<bool> {
            Ok(self.ignore_case)
        }

        fn read_config(&self, _source_root: &Path, _key: &str) -> Result<Option<String>> {
            Ok(None)
        }
    }

    fn main_and_linked() -> Vec<WorktreeRecord> {
        vec![
            WorktreeRecord {
                path: main_repo_path(),
                is_main: true,
                is_bare: false,
            },
            WorktreeRecord {
                path: linked_repo_path(),
                is_main: false,
                is_bare: false,
            },
        ]
    }

    #[test]
    fn explicit_source_and_dest() {
        let main = main_repo_path();
        let linked = linked_repo_path();
        let git = MockGit::new(main_and_linked());
        let ctx = resolve_context(
            &git,
            Some(main.as_path()),
            Some(linked.as_path()),
            None,
            CommandKind::Copy,
        )
        .unwrap();
        assert_eq!(ctx.source_root, main);
        assert_eq!(ctx.dest_root, Some(linked));
    }

    #[test]
    fn copy_rejects_same_source_and_dest() {
        let main = main_repo_path();
        let git = MockGit::new(main_and_linked());
        let err = resolve_context(
            &git,
            Some(main.as_path()),
            Some(main.as_path()),
            None,
            CommandKind::Copy,
        )
        .unwrap_err();
        assert!(err.to_string().contains("same"));
    }

    #[test]
    fn copy_requires_dest_from_main_worktree() {
        let main = main_repo_path();
        let git = MockGit::new(main_and_linked());
        let err =
            resolve_context(&git, Some(main.as_path()), None, None, CommandKind::Copy).unwrap_err();
        assert!(err.to_string().contains("destination"));
    }

    #[test]
    fn list_does_not_require_dest() {
        let main = main_repo_path();
        let git = MockGit::new(main_and_linked());
        let ctx =
            resolve_context(&git, Some(main.as_path()), None, None, CommandKind::List).unwrap();
        assert_eq!(ctx.source_root, main);
        assert!(ctx.dest_root.is_none());
    }

    #[test]
    fn copy_rejects_dest_outside_family() {
        let main = main_repo_path();
        let outside = outside_repo_path();
        let git = MockGit::new(vec![WorktreeRecord {
            path: main.clone(),
            is_main: true,
            is_bare: false,
        }]);
        let err = resolve_context(
            &git,
            Some(main.as_path()),
            Some(outside.as_path()),
            None,
            CommandKind::Copy,
        )
        .unwrap_err();
        assert!(err.to_string().contains("not a linked worktree"));
    }
}
