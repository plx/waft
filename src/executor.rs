//! Copy plan execution.
//!
//! The executor consumes a `CopyPlan` and applies file copy entries via the
//! filesystem abstraction. The chosen [`CopyStrategy`]
//! determines whether destinations are produced by streaming byte copies or
//! reflink (COW) clones where supported, with atomic temp-and-rename
//! semantics handled inside the filesystem layer.

use std::fs::{self, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

use crate::config::CopyStrategy;
use crate::fs::{CopyFileRequest, FileSystem};
use crate::git::GitBackend;
use crate::model::{CopyOutcome, CopyPlan, CopyReport, CopyResult, CopyResultKind, PlannedEntry};

/// Execute a copy plan, returning a report of outcomes.
///
/// If `dry_run` is set on the plan, no filesystem mutations are performed
/// and all copies are reported as successful.
pub fn execute(
    plan: &CopyPlan,
    fs: &dyn FileSystem,
    git: &dyn GitBackend,
    strategy: CopyStrategy,
) -> CopyReport {
    let mut results = Vec::new();
    let mut copied = 0usize;
    let mut failed = 0usize;
    let mut skipped = 0usize;
    let mut up_to_date = 0usize;

    for entry in &plan.entries {
        match entry {
            PlannedEntry::Copy(op) => {
                if plan.dry_run {
                    copied += 1;
                    results.push(CopyResult {
                        rel_path: op.rel_path.clone(),
                        kind: CopyResultKind::File,
                        outcome: CopyOutcome::Copied,
                    });
                    continue;
                }

                let Some(destination_root) = plan.context.dest_root.as_deref() else {
                    failed += 1;
                    results.push(CopyResult {
                        rel_path: op.rel_path.clone(),
                        kind: CopyResultKind::File,
                        outcome: CopyOutcome::Failed {
                            message: format!("{}: copy plan has no destination root", op.rel_path),
                        },
                    });
                    continue;
                };
                let mut index_lock = None;
                let result = {
                    let mut before_publish = || {
                        let index_path =
                            git.index_path(destination_root).map_err(io::Error::other)?;
                        index_lock = index_path
                            .as_deref()
                            .map(GitIndexLock::acquire)
                            .transpose()?;
                        ensure_destination_untracked(plan, git, std::slice::from_ref(&op.rel_path))
                            .map_err(io::Error::other)
                    };
                    execute_copy(
                        op,
                        &plan.context.source_root,
                        destination_root,
                        fs,
                        strategy,
                        &mut before_publish,
                    )
                };
                // Release only after the filesystem primitive has returned.
                drop(index_lock);
                match result {
                    Ok(()) => {
                        copied += 1;
                        results.push(CopyResult {
                            rel_path: op.rel_path.clone(),
                            kind: CopyResultKind::File,
                            outcome: CopyOutcome::Copied,
                        });
                    }
                    Err(msg) => {
                        failed += 1;
                        results.push(CopyResult {
                            rel_path: op.rel_path.clone(),
                            kind: CopyResultKind::File,
                            outcome: CopyOutcome::Failed { message: msg },
                        });
                    }
                }
            }
            PlannedEntry::NoOp(_) => {
                up_to_date += 1;
            }
            PlannedEntry::Skip(_) => {
                skipped += 1;
            }
        }
    }

    CopyReport {
        results,
        copied,
        failed,
        skipped,
        up_to_date,
    }
}

/// A cooperative Git index writer lock.
///
/// Git writers create `<index>.lock` before atomically replacing the index.
/// Holding the same lock across the final tracked check and file publication
/// closes that race against normal Git operations. Direct, non-cooperative
/// mutation of the index remains outside this protocol.
struct GitIndexLock {
    path: PathBuf,
    file: Option<fs::File>,
}

impl GitIndexLock {
    fn acquire(index_path: &Path) -> io::Result<Self> {
        let mut lock_name = index_path.as_os_str().to_os_string();
        lock_name.push(".lock");
        let path = PathBuf::from(lock_name);
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|error| {
                if error.kind() == io::ErrorKind::AlreadyExists {
                    io::Error::new(
                        io::ErrorKind::WouldBlock,
                        format!(
                            "destination Git index is locked at {}; refusing to publish",
                            path.display()
                        ),
                    )
                } else {
                    io::Error::new(
                        error.kind(),
                        format!(
                            "failed to lock destination Git index at {}: {error}",
                            path.display()
                        ),
                    )
                }
            })?;
        Ok(Self {
            path,
            file: Some(file),
        })
    }
}

impl Drop for GitIndexLock {
    fn drop(&mut self) {
        // Windows cannot unlink an open file; close first on every platform.
        drop(self.file.take());
        let _ = fs::remove_file(&self.path);
    }
}

fn ensure_destination_untracked(
    plan: &CopyPlan,
    git: &dyn GitBackend,
    paths: &[crate::path::RepoRelPath],
) -> Result<(), String> {
    let dest_root = plan.context.dest_root.as_ref().ok_or_else(|| {
        "copy plan has no destination for execution-time tracked-file check".to_string()
    })?;
    let tracked = git.tracked_paths(dest_root, paths).map_err(|error| {
        format!("failed to recheck destination tracked state before copying: {error}")
    })?;
    if let Some(path) = paths.iter().find(|path| tracked.contains(*path)) {
        return Err(format!(
            "{path}: destination became tracked after planning; refusing to overwrite"
        ));
    }
    Ok(())
}

/// Execute a single copy operation.
fn execute_copy(
    op: &crate::model::CopyOp,
    source_root: &std::path::Path,
    destination_root: &std::path::Path,
    fs: &dyn FileSystem,
    strategy: CopyStrategy,
    before_publish: &mut dyn FnMut() -> io::Result<()>,
) -> Result<(), String> {
    fs.copy_file(
        CopyFileRequest {
            source_root,
            destination_root,
            rel_path: &op.rel_path,
            strategy,
            expected_source: &op.expected_source,
            expected_destination: &op.expected_destination,
        },
        before_publish,
    )
    .map_err(|e| format!("{}: failed to copy: {e}", op.rel_path))?;

    Ok(())
}

/// Render a copy report to stderr.
pub fn render_report(report: &CopyReport, quiet: bool) {
    for result in &report.results {
        match &result.outcome {
            CopyOutcome::Failed { message } => {
                // Quiet suppresses routine progress, never actionable errors.
                eprintln!("FAILED: {message}");
            }
            CopyOutcome::Copied if !quiet => match &result.kind {
                CopyResultKind::File => {
                    eprintln!("copied: {}", result.rel_path);
                }
            },
            CopyOutcome::Copied => {}
        }
    }

    if !quiet {
        eprintln!(
            "{} copied, {} failed, {} skipped, {} up-to-date",
            report.copied, report.failed, report.skipped, report.up_to_date
        );
    }
}

/// Check if the copy report has any failures, returning an appropriate exit status.
pub fn report_has_failures(report: &CopyReport) -> Option<(usize, usize)> {
    if report.failed > 0 {
        Some((report.failed, report.copied + report.failed))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        CopyOp, CopyPlan, DestinationExpectation, FileSnapshot, RepoContext, ValidationReport,
    };
    use crate::path::RepoRelPath;
    use std::cell::{Cell, RefCell};
    use std::collections::HashSet;
    use std::io;
    use std::path::{Path, PathBuf};

    #[derive(Debug, Default)]
    struct MockGit {
        tracked: HashSet<RepoRelPath>,
        tracked_starting_at_check: Option<usize>,
        tracked_checks: Cell<usize>,
        index_path: Option<PathBuf>,
    }

    impl GitBackend for MockGit {
        fn show_toplevel(&self, _path: &Path) -> crate::error::Result<PathBuf> {
            unreachable!()
        }

        fn list_worktrees(
            &self,
            _source_root: &Path,
        ) -> crate::error::Result<Vec<crate::git::WorktreeRecord>> {
            unreachable!()
        }

        fn tracked_paths(
            &self,
            _source_root: &Path,
            paths: &[RepoRelPath],
        ) -> crate::error::Result<HashSet<RepoRelPath>> {
            let check = self.tracked_checks.get() + 1;
            self.tracked_checks.set(check);
            if self
                .tracked_starting_at_check
                .is_some_and(|first_tracked_check| check < first_tracked_check)
            {
                return Ok(HashSet::new());
            }
            Ok(paths
                .iter()
                .filter(|path| self.tracked.contains(*path))
                .cloned()
                .collect())
        }

        fn index_path(&self, _source_root: &Path) -> crate::error::Result<Option<PathBuf>> {
            Ok(self.index_path.clone())
        }

        fn gitlinks(&self, _source_root: &Path) -> crate::error::Result<HashSet<String>> {
            unreachable!()
        }

        fn check_ignore(
            &self,
            _source_root: &Path,
            _paths: &[RepoRelPath],
        ) -> crate::error::Result<Vec<crate::git::IgnoreCheckRecord>> {
            unreachable!()
        }

        fn list_worktreeinclude_candidates(
            &self,
            _source_root: &Path,
            _semantics: crate::config::WorktreeincludeSemantics,
            _symlink_policy: crate::config::SymlinkPolicy,
        ) -> crate::error::Result<Vec<RepoRelPath>> {
            unreachable!()
        }

        fn list_ignored_untracked(
            &self,
            _source_root: &Path,
        ) -> crate::error::Result<Vec<RepoRelPath>> {
            unreachable!()
        }

        fn worktreeinclude_exists_anywhere(
            &self,
            _source_root: &Path,
            _symlink_policy: crate::config::SymlinkPolicy,
        ) -> crate::error::Result<bool> {
            unreachable!()
        }

        fn read_bool_config(&self, _source_root: &Path, _key: &str) -> crate::error::Result<bool> {
            unreachable!()
        }

        fn read_config(
            &self,
            _source_root: &Path,
            _key: &str,
        ) -> crate::error::Result<Option<String>> {
            unreachable!()
        }
    }

    #[derive(Debug, Default)]
    struct MockFs {
        copy_file_calls: RefCell<Vec<(PathBuf, PathBuf, DestinationExpectation)>>,
        fail_copy_file: bool,
        symlink_parents: HashSet<PathBuf>,
        symlinks: HashSet<PathBuf>,
        non_files: HashSet<PathBuf>,
        expected_index_lock: Option<PathBuf>,
    }

    impl FileSystem for MockFs {
        fn exists(&self, _path: &Path) -> bool {
            false
        }

        fn is_file(&self, path: &Path) -> bool {
            !self.non_files.contains(path) && !self.symlinks.contains(path)
        }

        fn is_dir(&self, _path: &Path) -> bool {
            true
        }

        fn is_symlink(&self, path: &Path) -> bool {
            self.symlinks.contains(path)
        }

        fn read(&self, _path: &Path) -> io::Result<Vec<u8>> {
            Ok(Vec::new())
        }

        fn parent_has_symlink(&self, path: &Path) -> bool {
            let mut current = path.to_path_buf();
            while let Some(parent) = current.parent() {
                if parent == current {
                    break;
                }
                if self.symlink_parents.contains(parent) {
                    return true;
                }
                current = parent.to_path_buf();
            }
            false
        }

        fn copy_file(
            &self,
            request: CopyFileRequest<'_>,
            before_publish: &mut dyn FnMut() -> io::Result<()>,
        ) -> io::Result<()> {
            if self.fail_copy_file {
                return Err(io::Error::other("copy failed"));
            }
            let src = request.rel_path.to_path(request.source_root);
            let dst = request.rel_path.to_path(request.destination_root);
            if self.symlinks.contains(&src) || self.non_files.contains(&src) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "source is not a regular file",
                ));
            }
            if self.parent_has_symlink(&dst) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "destination parent contains a symlink",
                ));
            }
            before_publish()?;
            if let Some(lock) = &self.expected_index_lock
                && !lock.exists()
            {
                return Err(io::Error::other(
                    "destination index lock was not held during publication",
                ));
            }
            self.copy_file_calls.borrow_mut().push((
                src,
                dst,
                request.expected_destination.clone(),
            ));
            Ok(())
        }
    }

    fn rel(path: &str) -> RepoRelPath {
        RepoRelPath::from_normalized(path.to_string())
    }

    fn plan(entry: PlannedEntry, dry_run: bool) -> CopyPlan {
        CopyPlan {
            context: RepoContext {
                source_root: PathBuf::from("/source"),
                dest_root: Some(PathBuf::from("/dest")),
                main_worktree: PathBuf::from("/source"),
                known_worktrees: Vec::new(),
                core_ignore_case: false,
            },
            validation: ValidationReport::default(),
            entries: vec![entry],
            dry_run,
        }
    }

    fn copy_entry() -> PlannedEntry {
        PlannedEntry::Copy(CopyOp {
            rel_path: rel(".env"),
            src_abs: PathBuf::from("/source/.env"),
            dst_abs: PathBuf::from("/dest/nested/.env"),
            expected_source: FileSnapshot::new(0, 0, 0, None),
            expected_destination: DestinationExpectation::Missing,
        })
    }

    #[test]
    fn execute_file_passes_expected_destination_to_conditional_copy() {
        let fs = MockFs::default();
        let report = execute(
            &plan(copy_entry(), false),
            &fs,
            &MockGit::default(),
            CopyStrategy::SimpleCopy,
        );

        assert_eq!(report.copied, 1);
        let calls = fs.copy_file_calls.borrow();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].2, DestinationExpectation::Missing);
    }

    #[test]
    fn execute_file_rechecks_source_type_after_creating_parent() {
        let fs = MockFs {
            symlinks: HashSet::from([PathBuf::from("/source/.env")]),
            ..MockFs::default()
        };
        let report = execute(
            &plan(copy_entry(), false),
            &fs,
            &MockGit::default(),
            CopyStrategy::SimpleCopy,
        );

        assert_eq!(report.failed, 1);
        assert!(fs.copy_file_calls.borrow().is_empty());
    }

    #[test]
    fn execute_file_rechecks_destination_parent() {
        let fs = MockFs {
            symlink_parents: HashSet::from([PathBuf::from("/dest")]),
            ..MockFs::default()
        };
        let report = execute(
            &plan(copy_entry(), false),
            &fs,
            &MockGit::default(),
            CopyStrategy::SimpleCopy,
        );

        assert_eq!(report.failed, 1);
        assert!(fs.copy_file_calls.borrow().is_empty());
    }

    #[test]
    fn execute_file_records_conditional_copy_failure() {
        let fs = MockFs {
            fail_copy_file: true,
            ..MockFs::default()
        };
        let report = execute(
            &plan(copy_entry(), false),
            &fs,
            &MockGit::default(),
            CopyStrategy::SimpleCopy,
        );

        assert_eq!(report.failed, 1);
        assert_eq!(report.copied, 0);
        assert!(report_has_failures(&report).is_some());
    }

    #[test]
    fn execute_refuses_path_that_became_tracked_after_planning() {
        let fs = MockFs::default();
        let git = MockGit {
            tracked: HashSet::from([rel(".env")]),
            ..MockGit::default()
        };

        let report = execute(
            &plan(copy_entry(), false),
            &fs,
            &git,
            CopyStrategy::SimpleCopy,
        );

        assert_eq!(report.failed, 1);
        assert!(fs.copy_file_calls.borrow().is_empty());
        let CopyOutcome::Failed { message } = &report.results[0].outcome else {
            panic!("expected tracked-state failure");
        };
        assert!(message.contains("became tracked"));
    }

    #[test]
    fn execute_checks_tracked_state_once_immediately_before_publish() {
        let fs = MockFs::default();
        let git = MockGit {
            tracked: HashSet::from([rel(".env")]),
            tracked_starting_at_check: Some(1),
            ..MockGit::default()
        };

        let report = execute(
            &plan(copy_entry(), false),
            &fs,
            &git,
            CopyStrategy::SimpleCopy,
        );

        assert_eq!(git.tracked_checks.get(), 1);
        assert_eq!(report.failed, 1);
        assert!(fs.copy_file_calls.borrow().is_empty());
        let CopyOutcome::Failed { message } = &report.results[0].outcome else {
            panic!("expected pre-publication tracked-state failure");
        };
        assert!(message.contains("became tracked"));
    }

    #[test]
    fn execute_holds_destination_index_lock_through_publication() {
        let temp = tempfile::TempDir::new().unwrap();
        let index = temp.path().join("index");
        let lock = temp.path().join("index.lock");
        let fs = MockFs {
            expected_index_lock: Some(lock.clone()),
            ..MockFs::default()
        };
        let git = MockGit {
            index_path: Some(index),
            ..MockGit::default()
        };

        let report = execute(
            &plan(copy_entry(), false),
            &fs,
            &git,
            CopyStrategy::SimpleCopy,
        );

        assert_eq!(report.copied, 1);
        assert!(
            !lock.exists(),
            "index lock should be released after publish"
        );
    }
}
