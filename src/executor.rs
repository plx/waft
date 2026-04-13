//! Copy plan execution with atomic writes.
//!
//! The executor consumes a `CopyPlan` and applies the `CopyOp`
//! entries. It copies file contents via a temp file in the destination
//! directory, then atomically renames into place.

use crate::fs::FileSystem;
use crate::model::{CopyOutcome, CopyPlan, CopyReport, CopyResult, PlannedEntry};

/// Execute a copy plan, returning a report of outcomes.
///
/// If `dry_run` is set on the plan, no filesystem mutations are performed
/// and all copies are reported as successful.
pub fn execute(plan: &CopyPlan, fs: &dyn FileSystem) -> CopyReport {
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
                        outcome: CopyOutcome::Copied,
                    });
                    continue;
                }

                match execute_copy(op, fs) {
                    Ok(()) => {
                        copied += 1;
                        results.push(CopyResult {
                            rel_path: op.rel_path.clone(),
                            outcome: CopyOutcome::Copied,
                        });
                    }
                    Err(msg) => {
                        failed += 1;
                        results.push(CopyResult {
                            rel_path: op.rel_path.clone(),
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
fn execute_copy(op: &crate::model::CopyOp, fs: &dyn FileSystem) -> Result<(), String> {
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

    // Read source data
    let data = fs
        .read(&op.src_abs)
        .map_err(|e| format!("{}: failed to read source: {e}", op.rel_path))?;

    // Atomic write: temp file + rename
    fs.atomic_write(&op.dst_abs, &data)
        .map_err(|e| format!("{}: failed to write destination: {e}", op.rel_path))?;

    // Preserve permissions (best-effort)
    let _ = fs.copy_permissions(&op.src_abs, &op.dst_abs);

    Ok(())
}

/// Render a copy report to stderr.
pub fn render_report(report: &CopyReport, quiet: bool) {
    if !quiet {
        for result in &report.results {
            match &result.outcome {
                CopyOutcome::Copied => {
                    eprintln!("copied: {}", result.rel_path);
                }
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
