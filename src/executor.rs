//! Copy plan execution.
//!
//! The executor consumes a `CopyPlan` and applies file and directory copy
//! entries via the filesystem abstraction. The chosen [`CopyStrategy`]
//! determines whether destinations are produced by streaming byte copies or
//! reflink (COW) clones where supported, with atomic temp-and-rename
//! semantics handled inside the filesystem layer.

use std::path::PathBuf;

use crate::config::CopyStrategy;
use crate::fs::FileSystem;
use crate::model::{CopyOutcome, CopyPlan, CopyReport, CopyResult, CopyResultKind, PlannedEntry};

/// Execute a copy plan, returning a report of outcomes.
///
/// If `dry_run` is set on the plan, no filesystem mutations are performed
/// and all copies are reported as successful.
pub fn execute(plan: &CopyPlan, fs: &dyn FileSystem, strategy: CopyStrategy) -> CopyReport {
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

                match execute_copy(op, fs, strategy) {
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
            PlannedEntry::CopyDir(op) => {
                let file_count = op.file_count();
                if plan.dry_run {
                    copied += file_count;
                    results.push(CopyResult {
                        rel_path: op.rel_path.clone(),
                        kind: CopyResultKind::Directory { file_count },
                        outcome: CopyOutcome::Copied,
                    });
                    continue;
                }

                match execute_copy_dir(op, fs, strategy) {
                    Ok(()) => {
                        copied += file_count;
                        results.push(CopyResult {
                            rel_path: op.rel_path.clone(),
                            kind: CopyResultKind::Directory { file_count },
                            outcome: CopyOutcome::Copied,
                        });
                    }
                    Err(msg) => {
                        failed += file_count;
                        results.push(CopyResult {
                            rel_path: op.rel_path.clone(),
                            kind: CopyResultKind::Directory { file_count },
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

/// Execute a single copy operation.
fn execute_copy(
    op: &crate::model::CopyOp,
    fs: &dyn FileSystem,
    strategy: CopyStrategy,
) -> Result<(), String> {
    // Never follow source symlinks
    if fs.is_symlink(&op.src_abs) {
        return Err(format!("{}: source is a symlink", op.rel_path));
    }

    // Never write through a symlinked destination parent
    if fs.parent_has_symlink(&op.dst_abs) {
        return Err(format!(
            "{}: destination parent contains a symlink",
            op.rel_path
        ));
    }

    // Create destination directories as needed
    if let Some(parent) = op.dst_abs.parent() {
        fs.create_dir_all(parent)
            .map_err(|e| format!("{}: failed to create directory: {e}", op.rel_path))?;
    }

    fs.copy_file(&op.src_abs, &op.dst_abs, strategy)
        .map_err(|e| format!("{}: failed to copy: {e}", op.rel_path))?;

    Ok(())
}

fn execute_copy_dir(
    op: &crate::model::CopyDirOp,
    fs: &dyn FileSystem,
    strategy: CopyStrategy,
) -> Result<(), String> {
    if fs.parent_has_symlink(&op.dst_abs) {
        return Err(format!(
            "{}: destination parent contains a symlink",
            op.rel_path
        ));
    }
    if let Some(parent) = op.dst_abs.parent() {
        fs.create_dir_all(parent)
            .map_err(|e| format!("{}: failed to create directory: {e}", op.rel_path))?;
    }
    let manifest = files_relative_to_dir(&op.rel_path, &op.files)?;
    fs.copy_dir_exact(&op.src_abs, &op.dst_abs, &manifest, strategy)
        .map_err(|e| format!("{}: failed to copy directory: {e}", op.rel_path))
}

fn files_relative_to_dir(
    dir: &crate::path::RepoRelPath,
    files: &[crate::path::RepoRelPath],
) -> Result<Vec<PathBuf>, String> {
    let prefix = format!("{}/", dir.as_str());
    files
        .iter()
        .map(|file| {
            let rel = file.as_str().strip_prefix(&prefix).ok_or_else(|| {
                format!(
                    "{}: manifest file {} is not under copied directory",
                    dir, file
                )
            })?;
            if rel.is_empty() {
                return Err(format!("{dir}: manifest file is the copied directory"));
            }
            Ok(PathBuf::from(
                rel.replace('/', std::path::MAIN_SEPARATOR_STR),
            ))
        })
        .collect()
}

/// Render a copy report to stderr.
pub fn render_report(report: &CopyReport, quiet: bool) {
    if !quiet {
        for result in &report.results {
            match &result.outcome {
                CopyOutcome::Copied => match &result.kind {
                    CopyResultKind::File => {
                        eprintln!("copied: {}", result.rel_path);
                    }
                    CopyResultKind::Directory { file_count } => {
                        eprintln!("copy-dir: {} ({} files)", result.rel_path, file_count);
                    }
                },
                CopyOutcome::Failed { message } => {
                    eprintln!("FAILED: {message}");
                }
            }
        }

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
    use crate::model::{CopyDirOp, CopyPlan, RepoContext, ValidationReport};
    use crate::path::RepoRelPath;
    use std::cell::RefCell;
    use std::collections::HashSet;
    use std::io;
    use std::path::{Path, PathBuf};

    #[derive(Debug, Default)]
    struct MockFs {
        created_dirs: RefCell<Vec<PathBuf>>,
        copy_dir_calls: RefCell<Vec<(PathBuf, PathBuf, Vec<PathBuf>)>>,
        fail_copy_dir: bool,
        symlink_parents: HashSet<PathBuf>,
    }

    impl FileSystem for MockFs {
        fn exists(&self, _path: &Path) -> bool {
            false
        }

        fn is_file(&self, _path: &Path) -> bool {
            true
        }

        fn is_dir(&self, _path: &Path) -> bool {
            true
        }

        fn is_symlink(&self, _path: &Path) -> bool {
            false
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

        fn create_dir_all(&self, path: &Path) -> io::Result<()> {
            self.created_dirs.borrow_mut().push(path.to_path_buf());
            Ok(())
        }

        fn copy_file(&self, _src: &Path, _dst: &Path, _strategy: CopyStrategy) -> io::Result<()> {
            Ok(())
        }

        fn copy_dir_exact(
            &self,
            src: &Path,
            dst: &Path,
            expected_files: &[PathBuf],
            _strategy: CopyStrategy,
        ) -> io::Result<()> {
            if self.fail_copy_dir {
                return Err(io::Error::other("copy failed"));
            }
            self.copy_dir_calls.borrow_mut().push((
                src.to_path_buf(),
                dst.to_path_buf(),
                expected_files.to_vec(),
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

    fn copydir_entry() -> PlannedEntry {
        PlannedEntry::CopyDir(CopyDirOp {
            rel_path: rel("cfg"),
            src_abs: PathBuf::from("/source/cfg"),
            dst_abs: PathBuf::from("/dest/parent/cfg"),
            files: vec![rel("cfg/a.conf"), rel("cfg/nested/b.conf")],
        })
    }

    #[test]
    fn execute_copydir_calls_fs_and_counts_files() {
        let fs = MockFs::default();
        let report = execute(&plan(copydir_entry(), false), &fs, CopyStrategy::SimpleCopy);

        assert_eq!(report.copied, 2);
        assert_eq!(report.results.len(), 1);
        assert_eq!(fs.copy_dir_calls.borrow().len(), 1);
        assert_eq!(
            fs.copy_dir_calls.borrow()[0].2,
            vec![PathBuf::from("a.conf"), PathBuf::from("nested/b.conf")]
        );
    }

    #[test]
    fn execute_copydir_creates_missing_parent() {
        let fs = MockFs::default();
        let _ = execute(&plan(copydir_entry(), false), &fs, CopyStrategy::SimpleCopy);

        assert!(
            fs.created_dirs
                .borrow()
                .contains(&PathBuf::from("/dest/parent"))
        );
    }

    #[test]
    fn execute_copydir_records_failure_counted_by_file_count() {
        let fs = MockFs {
            fail_copy_dir: true,
            ..MockFs::default()
        };
        let report = execute(&plan(copydir_entry(), false), &fs, CopyStrategy::SimpleCopy);

        assert_eq!(report.copied, 0);
        assert_eq!(report.failed, 2);
        assert!(report_has_failures(&report).is_some());
    }

    #[test]
    fn execute_copydir_dry_run_counts_files_without_calling_fs() {
        let fs = MockFs::default();
        let report = execute(&plan(copydir_entry(), true), &fs, CopyStrategy::SimpleCopy);

        assert_eq!(report.copied, 2);
        assert!(fs.copy_dir_calls.borrow().is_empty());
    }
}
