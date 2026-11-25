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

use waft_verified::context as kernel;

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

    // List all worktrees. From here on every path also gets an identity key
    // (see `PathIdentities`), so the verified kernel decides on plain data.
    let worktrees = git.list_worktrees(&toplevel)?;
    let mut identities = PathIdentities::default();
    let records: Vec<kernel::WorktreeRecord> = worktrees
        .iter()
        .map(|w| kernel::WorktreeRecord {
            key: identities.key(&w.path),
            is_main: w.is_main,
            is_bare: w.is_bare,
        })
        .collect();
    let kernel_command = kernel_command_kind(command);
    let main_index = match kernel::select_main(kernel_command, &records) {
        kernel::MainSelection::Main(index) => index,
        kernel::MainSelection::NoMain => {
            return Err(Error::Git {
                message: "no main worktree found".to_string(),
            });
        }
        kernel::MainSelection::BareMainCopy => {
            return Err(Error::Context {
                message: "copy is unsupported when the repository's main worktree is bare; use a non-bare main worktree as the copy source".to_string(),
            });
        }
    };
    // The kernel proves `main_index` is the first record flagged as main, so
    // it indexes both `records` and `worktrees`.
    let main_wt = worktrees[main_index].path.clone();
    let main = kernel::Observed {
        key: records[main_index].key,
        path: main_wt.clone(),
    };

    let known_worktrees: Vec<PathBuf> = worktrees.iter().map(|w| w.path.clone()).collect();

    // Ask Git for the toplevel of each root argument, in the same order and
    // with the same error propagation as always.
    let toplevels = if let Some(src) = source_path {
        let src_toplevel = git.show_toplevel(&src)?;
        let dst_toplevel = dest_path.map(|d| git.show_toplevel(&d)).transpose()?;
        kernel::Toplevels::Explicit {
            source: identities.observe(src_toplevel),
            dest: dst_toplevel.map(|d| identities.observe(d)),
        }
    } else if let Some(dst) = dest_path {
        // No explicit source: the kernel uses the main worktree as source
        let dst_toplevel = git.show_toplevel(&dst)?;
        kernel::Toplevels::DestOnly {
            dest: identities.observe(dst_toplevel),
        }
    } else {
        // No explicit source or dest — auto-detect from cwd
        let cwd_toplevel = git.show_toplevel(&cwd)?;
        kernel::Toplevels::FromCwd {
            cwd: identities.observe(cwd_toplevel),
        }
    };

    // Choose source and destination and enforce the copy rules. The kernel
    // (`waft_verified::context`) proves the precedence order and that a
    // resolved copy reads from main and writes into another family member.
    let verdict = kernel::resolve_roots(kernel_command, &records, main, toplevels);
    let (source_root, dest_root) = match verdict {
        kernel::RootsVerdict::Resolved { source, dest } => {
            (source.path, dest.map(|dest| dest.path))
        }
        kernel::RootsVerdict::CopyNeedsDest => {
            return Err(Error::Context {
                message:
                    "copy requires a destination worktree (use --dest or run from a linked worktree)"
                        .to_string(),
            });
        }
        kernel::RootsVerdict::CopySourceNotMain { source } => {
            return Err(Error::Context {
                message: format!(
                    "copy source must be the main worktree ({}), got {}",
                    main_wt.display(),
                    source.path.display()
                ),
            });
        }
        kernel::RootsVerdict::SameSourceAndDest { source } => {
            return Err(Error::SameSourceAndDest { path: source.path });
        }
        // Proven unreachable (`theorem_copy_dest_is_main_unreachable`):
        // for copy, "source must be main" and "source must differ from
        // dest" are checked first. Kept so the rule stays explicit.
        kernel::RootsVerdict::CopyDestIsMain => {
            return Err(Error::Context {
                message: "copy destination must be a linked worktree, not the main worktree"
                    .to_string(),
            });
        }
        kernel::RootsVerdict::NotInFamily { source, dest } => {
            return Err(Error::NotInWorktreeFamily {
                src: source.path,
                dest: dest.path,
            });
        }
    };

    // Ask the backend whether this checkout folds case. Every decision about
    // whether two *repository paths* are the same path — tracked-path
    // protection, gitlink boundaries, candidate matching — reads this one
    // answer, so they cannot disagree.
    //
    // Exclusion pattern matching deliberately does not: it treats this as a
    // lower bound and stays case-insensitive on macOS and Windows regardless,
    // because a missed safety exclusion leaks a file while a spurious one only
    // withholds one. See `crate::policy_filter::effective_case_insensitive`.
    let core_ignore_case = git.checkout_folds_case(&source_root)?;

    Ok(RepoContext {
        source_root,
        dest_root,
        main_worktree: main_wt,
        known_worktrees,
        core_ignore_case,
    })
}

/// Convert a command kind into the verified kernel's mirror of it.
fn kernel_command_kind(command: CommandKind) -> kernel::CommandKind {
    match command {
        CommandKind::Copy => kernel::CommandKind::Copy,
        CommandKind::List => kernel::CommandKind::List,
        CommandKind::Info => kernel::CommandKind::Info,
        CommandKind::Validate => kernel::CommandKind::Validate,
    }
}

/// Convert the verified kernel's command kind back into this crate's.
pub(crate) fn command_kind_from_kernel(command: kernel::CommandKind) -> CommandKind {
    match command {
        kernel::CommandKind::Copy => CommandKind::Copy,
        kernel::CommandKind::List => CommandKind::List,
        kernel::CommandKind::Info => CommandKind::Info,
        kernel::CommandKind::Validate => CommandKind::Validate,
    }
}

/// Identity keys for paths under `Path` equality.
///
/// `Path` equality compares component sequences, so it is an equivalence
/// relation, and interning by linear search gives two paths the same key
/// exactly when they compare equal. The verified context kernel relies on
/// this: it compares keys where this module used to compare `PathBuf`s.
#[derive(Default)]
struct PathIdentities {
    seen: Vec<PathBuf>,
}

impl PathIdentities {
    /// The key of `path`: the index of the first interned path equal to it.
    fn key(&mut self, path: &Path) -> usize {
        if let Some(index) = self.seen.iter().position(|seen| seen.as_path() == path) {
            return index;
        }
        self.seen.push(path.to_path_buf());
        self.seen.len() - 1
    }

    /// Pair an owned path with its key.
    fn observe(&mut self, path: PathBuf) -> kernel::Observed<PathBuf> {
        kernel::Observed {
            key: self.key(&path),
            path,
        }
    }
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

    #[test]
    fn copy_rejects_bare_main_with_actionable_error() {
        let main = main_repo_path();
        let linked = linked_repo_path();
        let git = MockGit::new(vec![
            WorktreeRecord {
                path: main.clone(),
                is_main: true,
                is_bare: true,
            },
            WorktreeRecord {
                path: linked.clone(),
                is_main: false,
                is_bare: false,
            },
        ]);

        let err =
            resolve_context(&git, Some(&main), Some(&linked), None, CommandKind::Copy).unwrap_err();
        assert!(err.to_string().contains("main worktree is bare"));
    }

    #[test]
    fn kernel_command_kind_maps_each_variant_to_its_mirror() {
        let pairs = [
            (CommandKind::Copy, kernel::CommandKind::Copy),
            (CommandKind::List, kernel::CommandKind::List),
            (CommandKind::Info, kernel::CommandKind::Info),
            (CommandKind::Validate, kernel::CommandKind::Validate),
        ];
        for (command, mirror) in pairs {
            assert_eq!(kernel_command_kind(command), mirror);
            assert_eq!(command_kind_from_kernel(mirror), command);
        }
        // Distinct commands stay distinct.
        for (a, _) in pairs {
            for (b, _) in pairs {
                assert_eq!(a == b, kernel_command_kind(a) == kernel_command_kind(b));
            }
        }
    }

    #[test]
    fn path_identities_follow_path_equality() {
        let mut ids = PathIdentities::default();
        let a = ids.key(Path::new("/repo/main"));
        // `Path` equality is component-wise: these spell the same path.
        assert_eq!(ids.key(Path::new("/repo/main/")), a);
        assert_eq!(ids.key(Path::new("/repo/./main")), a);
        assert_eq!(ids.key(Path::new("/repo//main")), a);
        let b = ids.key(Path::new("/repo/linked"));
        assert_ne!(a, b);
        assert_ne!(ids.key(Path::new("/repo/main/..")), a);
        assert_eq!(ids.key(Path::new("/repo/linked")), b);
        let observed = ids.observe(PathBuf::from("/repo/main/"));
        assert_eq!(observed.key, a);
        // The observation keeps the exact spelling it was given.
        assert_eq!(observed.path.as_os_str(), "/repo/main/");
    }

    #[test]
    fn no_main_worktree_is_reported_before_any_root_check() {
        let linked = linked_repo_path();
        let git = MockGit::new(vec![WorktreeRecord {
            path: linked.clone(),
            is_main: false,
            is_bare: false,
        }]);
        let err = resolve_context(&git, Some(&linked), Some(&linked), None, CommandKind::Copy)
            .unwrap_err();
        assert!(err.to_string().contains("no main worktree found"));
    }

    #[test]
    fn copy_rejects_linked_source_before_same_or_family_checks() {
        let main = main_repo_path();
        let linked = linked_repo_path();
        let git = MockGit::new(main_and_linked());
        let err =
            resolve_context(&git, Some(&linked), Some(&main), None, CommandKind::Copy).unwrap_err();
        let message = err.to_string();
        assert!(message.contains("copy source must be the main worktree"));
        assert!(message.contains(&main.display().to_string()));
        assert!(message.contains(&linked.display().to_string()));
    }

    #[test]
    fn dest_only_uses_main_as_source() {
        let main = main_repo_path();
        let linked = linked_repo_path();
        let git = MockGit::new(main_and_linked());
        let ctx = resolve_context(&git, None, Some(&linked), None, CommandKind::Copy).unwrap();
        assert_eq!(ctx.source_root, main);
        assert_eq!(ctx.dest_root, Some(linked));
        assert_eq!(ctx.main_worktree, main);
    }
}
