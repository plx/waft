//! Read-only copy planning.
//!
//! The planner takes discovered eligible files and classifies each
//! destination path, producing a [`CopyPlan`] that the executor can
//! apply. The planner **never** mutates the filesystem.
//!
//! The decisions are made by Verus-verified kernels in the `waft-verified`
//! crate: `planner::destination` classifies a destination path,
//! `planner::entry` decides each path's planned entry, and `tally` does the
//! dry-run accounting and exit status. This module performs the filesystem
//! queries those kernels name, in the order they name them, and converts
//! between the kernels' plain data and waft's model types.

use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};

use waft_verified::planner::destination::{
    self as destination_kernel, CompareObs, DestObs, DestQuery, DestState, DestStep,
};
use waft_verified::planner::entry::{
    self as entry_kernel, EntryObs, EntryPlan, EntryQuery, EntryStep, Expectation, Failure,
    SkipKind, Snapshot, SourceKindObs,
};
use waft_verified::tally::{self, EntryKind};

use crate::eligibility_groups::EligibilityGroups;
use crate::error::Result;
use crate::fs::{FileComparison, FileSystem, SourceKind};
use crate::git::GitBackend;
use crate::model::{
    CopyOp, CopyPlan, DestinationExpectation, DestinationState, FailureEntry, FileSnapshot,
    NoOpEntry, NoOpReason, PlannedEntry, RepoContext, SkipEntry, SkipReason, ValidationReport,
};
use crate::path::RepoRelPath;

/// What planning records for an `--overwrite` intent the platform cannot carry
/// out.
const OVERWRITE_UNSUPPORTED: &str =
    "replacing an existing destination with --overwrite is not supported on this platform";

/// Build a copy plan for the given eligible paths.
///
/// `groups` should be computed from the intersection of worktreeinclude-selected
/// and git-ignored paths.
pub fn plan(
    ctx: &RepoContext,
    validation: ValidationReport,
    groups: EligibilityGroups,
    git: &dyn GitBackend,
    fs: &dyn FileSystem,
    overwrite: bool,
    dry_run: bool,
) -> Result<CopyPlan> {
    plan_with_overwrite_support(
        ctx,
        validation,
        groups,
        git,
        fs,
        overwrite,
        dry_run,
        crate::fs::overwrite_supported(),
    )
}

/// [`plan`] with the platform's replacement capability supplied explicitly.
///
/// Taking the capability as a parameter keeps the non-Unix planning path
/// testable from the Unix hosts waft's tests actually run on.
#[allow(clippy::too_many_arguments)]
pub(crate) fn plan_with_overwrite_support(
    ctx: &RepoContext,
    validation: ValidationReport,
    groups: EligibilityGroups,
    git: &dyn GitBackend,
    fs: &dyn FileSystem,
    overwrite: bool,
    dry_run: bool,
    overwrite_supported: bool,
) -> Result<CopyPlan> {
    let dest_root = match &ctx.dest_root {
        Some(d) => d,
        None => {
            return Ok(CopyPlan {
                context: ctx.clone(),
                validation,
                entries: Vec::new(),
                dry_run,
            });
        }
    };

    let mut all_manifest_files = groups.remaining_files.clone();
    for dir in &groups.full_dirs {
        all_manifest_files.extend(dir.files.iter().cloned());
    }
    all_manifest_files.sort();
    all_manifest_files.dedup();

    // Check which manifest paths are tracked in the destination worktree.
    let dest_tracked = git.tracked_paths(dest_root, &all_manifest_files)?;

    let mut entries: Vec<PlannedEntry> = Vec::new();
    let mut per_file_paths = groups.remaining_files;

    // Always execute the selected manifest as individual checked file
    // operations. A whole-directory clone observes the live source tree and
    // can therefore copy files that appeared after candidate selection.
    for dir in groups.full_dirs {
        per_file_paths.extend(dir.files);
    }

    per_file_paths.sort();
    per_file_paths.dedup();

    for rel_path in per_file_paths {
        let src_abs = rel_path.to_path(&ctx.source_root);
        let dst_abs = rel_path.to_path(dest_root);

        // The verified entry kernel decides what to ask and what the answers
        // mean. A source that was examined and is not a regular file is
        // skipped; a source that could not be examined at all was eligible
        // when it was discovered, so it is a per-file failure rather than a
        // file quietly dropped from the plan and the exit status. Existing
        // destinations are only replaced or repaired under `--overwrite` on a
        // platform that can do it, pinned by snapshots of both files.
        let decision = plan_entry(
            &rel_path,
            &src_abs,
            &dst_abs,
            &dest_tracked,
            fs,
            overwrite,
            overwrite_supported,
        );
        entries.push(planned_entry(rel_path, src_abs, dst_abs, decision));
    }

    // Sort entries deterministically by repo-relative path
    entries.sort_by(|a, b| a.rel_path().cmp(b.rel_path()));

    Ok(CopyPlan {
        context: ctx.clone(),
        validation,
        entries,
        dry_run,
    })
}

/// Plan one eligible path by driving the verified entry kernel.
///
/// Each round performs exactly the query `planner::entry::plan_entry_step`
/// names and records its answer in the field that query owns. The kernel
/// proves that this asks the same questions, in the same order, as a lazy
/// evaluation of its decision table, and that the plan it returns moves the
/// observed snapshots and errors into the entry.
fn plan_entry(
    rel_path: &RepoRelPath,
    src_abs: &Path,
    dst_abs: &Path,
    dest_tracked: &HashSet<RepoRelPath>,
    fs: &dyn FileSystem,
    overwrite: bool,
    overwrite_supported: bool,
) -> EntryPlan<FileSnapshot, io::Error> {
    let mut observed = EntryObs::new();
    loop {
        observed = match entry_kernel::plan_entry_step(observed, overwrite, overwrite_supported) {
            EntryStep::Done(decision) => return decision,
            EntryStep::Query(mut observed, query) => {
                match query {
                    EntryQuery::SourceKind => {
                        observed.source_kind = Some(fs.source_kind(src_abs).map(source_kind_obs));
                    }
                    EntryQuery::Destination => {
                        observed.destination = Some(classify_destination_state(
                            rel_path,
                            src_abs,
                            dst_abs,
                            dest_tracked,
                            fs,
                        ));
                    }
                    EntryQuery::ParentHasSymlink => {
                        observed.parent_has_symlink = Some(fs.parent_has_symlink(dst_abs));
                    }
                    EntryQuery::SourceSnapshot => {
                        observed.source_snapshot =
                            Some(fs.file_snapshot(src_abs).map(kernel_snapshot));
                    }
                    EntryQuery::DestinationSnapshot => {
                        observed.destination_snapshot =
                            Some(fs.file_snapshot(dst_abs).map(kernel_snapshot));
                    }
                }
                observed
            }
        };
    }
}

/// Turn the kernel's plan for one path into a [`PlannedEntry`].
fn planned_entry(
    rel_path: RepoRelPath,
    src_abs: PathBuf,
    dst_abs: PathBuf,
    decision: EntryPlan<FileSnapshot, io::Error>,
) -> PlannedEntry {
    match decision {
        EntryPlan::Copy {
            source,
            destination,
        } => PlannedEntry::Copy(CopyOp {
            rel_path,
            src_abs,
            dst_abs,
            expected_source: source,
            expected_destination: match destination {
                Expectation::Missing => DestinationExpectation::Missing,
                Expectation::ReplaceExisting(snapshot) => {
                    DestinationExpectation::ReplaceExisting(snapshot)
                }
                Expectation::RepairPermissions(snapshot) => {
                    DestinationExpectation::RepairPermissions(snapshot)
                }
            },
        }),
        EntryPlan::NoOp => PlannedEntry::NoOp(NoOpEntry {
            rel_path,
            reason: NoOpReason::UpToDate,
        }),
        EntryPlan::Skip(kind) => PlannedEntry::Skip(SkipEntry {
            rel_path,
            reason: skip_reason(kind),
        }),
        EntryPlan::Fail(failure) => {
            // A path that vanished or became unreadable is one file's
            // problem, not the run's: record it and keep planning.
            let message = match failure {
                Failure::ExamineSource(error) => {
                    format!("failed to examine source {}: {error}", src_abs.display())
                }
                Failure::SnapshotSource(error) => {
                    format!("failed to snapshot source {}: {error}", src_abs.display())
                }
                Failure::SnapshotDestination(error) => format!(
                    "failed to snapshot destination {}: {error}",
                    dst_abs.display()
                ),
                // The plan, its `--dry-run` rendering, the executed run, and
                // the exit status have to describe the same thing. A platform
                // with no way to replace or repair an existing destination
                // says so once, here, instead of promising a replacement that
                // only fails when it is attempted.
                Failure::OverwriteUnsupported => OVERWRITE_UNSUPPORTED.to_string(),
            };
            PlannedEntry::Failure(FailureEntry { rel_path, message })
        }
    }
}

/// Classify the state of a destination path.
pub(crate) fn classify_destination(
    rel_path: &RepoRelPath,
    src_abs: &Path,
    dst_abs: &Path,
    dest_tracked: &HashSet<RepoRelPath>,
    fs: &dyn FileSystem,
) -> DestinationState {
    destination_state(classify_destination_state(
        rel_path,
        src_abs,
        dst_abs,
        dest_tracked,
        fs,
    ))
}

/// Classify a destination by driving the verified destination kernel.
///
/// Trackedness is known before any filesystem query. Each round then
/// performs exactly the query `planner::destination::classify_step` names;
/// the kernel proves this is the short-circuiting order tracked, symlinked
/// parent, existence, symlink, regular file, comparison, and that an answer
/// given early is the one the remaining queries could not change.
fn classify_destination_state(
    rel_path: &RepoRelPath,
    src_abs: &Path,
    dst_abs: &Path,
    dest_tracked: &HashSet<RepoRelPath>,
    fs: &dyn FileSystem,
) -> DestState {
    let mut observed = DestObs::new(dest_tracked.contains(rel_path));
    loop {
        match destination_kernel::classify_step(&observed) {
            DestStep::Done(state) => return state,
            DestStep::Query(DestQuery::ParentHasSymlink) => {
                observed.parent_has_symlink = Some(fs.parent_has_symlink(dst_abs));
            }
            DestStep::Query(DestQuery::Exists) => {
                observed.exists = Some(fs.exists(dst_abs));
            }
            DestStep::Query(DestQuery::IsSymlink) => {
                observed.is_symlink = Some(fs.is_symlink(dst_abs));
            }
            DestStep::Query(DestQuery::IsFile) => {
                observed.is_file = Some(fs.is_file(dst_abs));
            }
            DestStep::Query(DestQuery::Compare) => {
                // Compares content and relevant permissions without retaining
                // both complete files in memory.
                observed.compare = Some(compare_obs(fs.compare_files(src_abs, dst_abs)));
            }
        }
    }
}

/// A comparison result as the destination kernel sees it.
fn compare_obs(result: io::Result<FileComparison>) -> CompareObs {
    match result {
        Ok(FileComparison::Equal) => CompareObs::Equal,
        Ok(FileComparison::PermissionsDiffer) => CompareObs::PermissionsDiffer,
        Ok(FileComparison::ContentDiffers) => CompareObs::ContentDiffers,
        Err(_) => CompareObs::Error,
    }
}

/// A source kind as the entry kernel sees it.
fn source_kind_obs(kind: SourceKind) -> SourceKindObs {
    match kind {
        SourceKind::RegularFile => SourceKindObs::RegularFile,
        SourceKind::Symlink => SourceKindObs::Symlink,
        SourceKind::Other => SourceKindObs::Other,
    }
}

/// A snapshot as the entry kernel sees it: its content key and the snapshot.
fn kernel_snapshot(snapshot: FileSnapshot) -> Snapshot<FileSnapshot> {
    Snapshot {
        key: snapshot.content_key(),
        payload: snapshot,
    }
}

/// The model's destination state for the kernel's.
fn destination_state(state: DestState) -> DestinationState {
    match state {
        DestState::Missing => DestinationState::Missing,
        DestState::UpToDate => DestinationState::UpToDate,
        DestState::PermissionsDiffer => DestinationState::PermissionsDiffer,
        DestState::UntrackedConflict => DestinationState::UntrackedConflict,
        DestState::TrackedConflict => DestinationState::TrackedConflict,
        DestState::TypeConflict => DestinationState::TypeConflict,
        DestState::UnsafePath => DestinationState::UnsafePath,
    }
}

/// The model's skip reason for the kernel's.
fn skip_reason(kind: SkipKind) -> SkipReason {
    match kind {
        SkipKind::UntrackedConflict => SkipReason::UntrackedConflict,
        SkipKind::PermissionsDiffer => SkipReason::PermissionsDiffer,
        SkipKind::TrackedConflict => SkipReason::TrackedConflict,
        SkipKind::TypeConflict => SkipReason::TypeConflict,
        SkipKind::UnsafePath => SkipReason::UnsafePath,
        SkipKind::UnsupportedSourceType => SkipReason::UnsupportedSourceType,
    }
}

/// The accounting kind of a planned entry.
fn entry_kind(entry: &PlannedEntry) -> EntryKind {
    match entry {
        PlannedEntry::Copy(op) => match op.expected_destination {
            DestinationExpectation::Missing => EntryKind::CopyMissing,
            DestinationExpectation::ReplaceExisting(_) => EntryKind::CopyReplace,
            DestinationExpectation::RepairPermissions(_) => EntryKind::CopyRepair,
        },
        PlannedEntry::NoOp(_) => EntryKind::NoOp,
        PlannedEntry::Skip(_) => EntryKind::Skip,
        PlannedEntry::Failure(_) => EntryKind::Failure,
    }
}

/// The accounting kinds of a plan's entries, in order.
fn entry_kinds(entries: &[PlannedEntry]) -> Vec<EntryKind> {
    entries.iter().map(entry_kind).collect()
}

/// The per-file planning failures in a plan, as `(failed, total)`.
///
/// This mirrors [`crate::executor::report_has_failures`] so a dry run reports
/// and exits exactly like the run it is describing: a file that planning could
/// not describe is a failure whether or not anything is written. The verified
/// `tally` kernel proves the pair agrees with the executor's dry-run counters.
pub fn planning_failures(plan: &CopyPlan) -> Option<(usize, usize)> {
    tally::planning_failures(&entry_kinds(&plan.entries))
}

/// Render a dry-run plan.
///
/// The plan itself goes to stdout and is suppressed by `quiet`; per-file
/// failures go to stderr unconditionally, because `quiet` suppresses routine
/// progress, never the reason a run is about to exit nonzero.
pub fn render_dry_run(plan: &CopyPlan, quiet: bool) {
    let kinds = entry_kinds(&plan.entries);

    for (entry, &kind) in plan.entries.iter().zip(&kinds) {
        if !tally::shows_entry(kind, quiet) {
            continue;
        }
        match entry {
            PlannedEntry::Copy(op) => match &op.expected_destination {
                DestinationExpectation::Missing => println!("copy: {}", op.rel_path),
                DestinationExpectation::ReplaceExisting(_) => {
                    println!("replace: {} (untracked conflict)", op.rel_path);
                }
                DestinationExpectation::RepairPermissions(_) => println!(
                    "repair permissions: {} (content equal, permissions differ)",
                    op.rel_path
                ),
            },
            PlannedEntry::NoOp(entry) => {
                println!("no-op: {} ({:?})", entry.rel_path, entry.reason);
            }
            PlannedEntry::Skip(entry) => println!("skip: {} ({})", entry.rel_path, entry.reason),
            PlannedEntry::Failure(entry) => {
                // Same channel and prefix as an executed run's failures, so a
                // dry run that will exit nonzero says so even under `--quiet`.
                eprintln!("FAILED: {}: {}", entry.rel_path, entry.message);
            }
        }
    }

    let counts = tally::tally(&kinds);
    let Some(clauses) = tally::dry_run_summary(&counts, quiet) else {
        return;
    };
    let tally::Tally {
        copies,
        replacements,
        repairs,
        skips,
        noops,
        failures,
    } = counts;

    let mut summary = format!("dry run: {copies} to copy");
    if clauses.replacements {
        summary.push_str(&format!(", {replacements} to replace"));
    }
    if clauses.repairs {
        summary.push_str(&format!(", {repairs} permission repair(s)"));
    }
    summary.push_str(&format!(", {skips} to skip, {noops} up-to-date"));
    if clauses.failures {
        summary.push_str(&format!(", {failures} failed"));
    }
    eprintln!("{summary}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eligibility_groups::EligibilityGroups;
    use crate::fs::FileSystem;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::io;
    use std::path::{Path, PathBuf};

    /// Mock filesystem for testing the planner.
    struct MockFs {
        files: RefCell<HashMap<PathBuf, Vec<u8>>>,
        dirs: RefCell<HashSet<PathBuf>>,
        symlinks: RefCell<HashSet<PathBuf>>,
    }

    impl MockFs {
        fn new() -> Self {
            Self {
                files: RefCell::new(HashMap::new()),
                dirs: RefCell::new(HashSet::new()),
                symlinks: RefCell::new(HashSet::new()),
            }
        }

        fn add_file(&self, path: &str, content: &[u8]) {
            self.files
                .borrow_mut()
                .insert(PathBuf::from(path), content.to_vec());
        }

        fn add_dir(&self, path: &str) {
            self.dirs.borrow_mut().insert(PathBuf::from(path));
        }

        #[allow(dead_code)]
        fn add_symlink(&self, path: &str) {
            self.symlinks.borrow_mut().insert(PathBuf::from(path));
        }
    }

    impl FileSystem for MockFs {
        fn exists(&self, path: &Path) -> bool {
            self.files.borrow().contains_key(path)
                || self.dirs.borrow().contains(path)
                || self.symlinks.borrow().contains(path)
        }

        fn is_file(&self, path: &Path) -> bool {
            self.files.borrow().contains_key(path)
        }

        fn is_dir(&self, path: &Path) -> bool {
            self.dirs.borrow().contains(path)
        }

        fn is_symlink(&self, path: &Path) -> bool {
            self.symlinks.borrow().contains(path)
        }

        fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
            self.files
                .borrow()
                .get(path)
                .cloned()
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "not found"))
        }

        fn parent_has_symlink(&self, path: &Path) -> bool {
            let mut current = path.to_path_buf();
            while let Some(parent) = current.parent() {
                if parent == current {
                    break;
                }
                if self.symlinks.borrow().contains(parent) {
                    return true;
                }
                current = parent.to_path_buf();
            }
            false
        }

        fn copy_file(
            &self,
            request: crate::fs::CopyFileRequest<'_>,
            _before_publish: &mut dyn FnMut() -> io::Result<()>,
        ) -> io::Result<crate::model::PublishOutcome> {
            let src = request.rel_path.to_path(request.source_root);
            let dst = request.rel_path.to_path(request.destination_root);
            let data = self
                .files
                .borrow()
                .get(&src)
                .cloned()
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "not found"))?;
            let replacing = self.files.borrow().contains_key(&dst);
            self.files.borrow_mut().insert(dst, data);
            Ok(if replacing {
                crate::model::PublishOutcome::Replaced
            } else {
                crate::model::PublishOutcome::Created
            })
        }
    }

    // Minimal mock git backend for planner tests
    struct MockPlannerGit {
        tracked: HashSet<RepoRelPath>,
    }

    impl MockPlannerGit {
        fn new(tracked: Vec<&str>) -> Self {
            Self {
                tracked: tracked
                    .into_iter()
                    .map(|s| RepoRelPath::from_normalized(s.to_string()))
                    .collect(),
            }
        }
    }

    impl GitBackend for MockPlannerGit {
        fn show_toplevel(&self, _path: &Path) -> Result<PathBuf> {
            Ok(PathBuf::from("/repo"))
        }
        fn list_worktrees(&self, _source_root: &Path) -> Result<Vec<crate::git::WorktreeRecord>> {
            Ok(vec![])
        }
        fn tracked_paths(
            &self,
            _source_root: &Path,
            _paths: &[RepoRelPath],
        ) -> Result<HashSet<RepoRelPath>> {
            Ok(self.tracked.clone())
        }
        fn gitlinks(&self, _source_root: &Path) -> Result<HashSet<String>> {
            Ok(HashSet::new())
        }
        fn check_ignore(
            &self,
            _source_root: &Path,
            _paths: &[RepoRelPath],
        ) -> Result<Vec<crate::git::IgnoreCheckRecord>> {
            Ok(vec![])
        }
        fn list_worktreeinclude_candidates(
            &self,
            _source_root: &Path,
            _semantics: crate::config::WorktreeincludeSemantics,
            _symlink_policy: crate::config::SymlinkPolicy,
        ) -> Result<Vec<RepoRelPath>> {
            Ok(vec![])
        }
        fn list_ignored_untracked(&self, _source_root: &Path) -> Result<Vec<RepoRelPath>> {
            Ok(vec![])
        }
        fn worktreeinclude_exists_anywhere(
            &self,
            _source_root: &Path,
            _symlink_policy: crate::config::SymlinkPolicy,
        ) -> Result<bool> {
            Ok(false)
        }
        fn read_bool_config(&self, _source_root: &Path, _key: &str) -> Result<bool> {
            Ok(false)
        }

        fn read_config(&self, _source_root: &Path, _key: &str) -> Result<Option<String>> {
            Ok(None)
        }
    }

    fn test_ctx() -> RepoContext {
        RepoContext {
            source_root: PathBuf::from("/source"),
            dest_root: Some(PathBuf::from("/dest")),
            main_worktree: PathBuf::from("/source"),
            known_worktrees: vec![PathBuf::from("/source"), PathBuf::from("/dest")],
            core_ignore_case: false,
        }
    }

    fn rel(path: &str) -> RepoRelPath {
        RepoRelPath::from_normalized(path.to_string())
    }

    fn groups_with(
        full_dirs: Vec<(&str, Vec<&str>)>,
        remaining_files: Vec<&str>,
    ) -> EligibilityGroups {
        EligibilityGroups {
            full_dirs: full_dirs
                .into_iter()
                .map(|(dir, files)| crate::eligibility_groups::EligibleDir {
                    rel_path: rel(dir),
                    files: files.into_iter().map(rel).collect(),
                })
                .collect(),
            remaining_files: remaining_files.into_iter().map(rel).collect(),
        }
    }

    #[test]
    fn plan_missing_dest_copies() {
        let fs = MockFs::new();
        fs.add_file("/source/.env", b"secret");

        let git = MockPlannerGit::new(vec![]);
        let ctx = test_ctx();
        let paths = vec![RepoRelPath::from_normalized(".env".to_string())];

        let plan = plan(
            &ctx,
            ValidationReport::default(),
            EligibilityGroups::from_files(paths),
            &git,
            &fs,
            false,
            false,
        )
        .unwrap();
        assert_eq!(plan.entries.len(), 1);
        assert!(matches!(
            plan.entries[0],
            PlannedEntry::Copy(CopyOp {
                expected_destination: DestinationExpectation::Missing,
                ..
            })
        ));
    }

    #[test]
    fn plan_up_to_date_is_noop() {
        let fs = MockFs::new();
        fs.add_file("/source/.env", b"same");
        fs.add_file("/dest/.env", b"same");

        let git = MockPlannerGit::new(vec![]);
        let ctx = test_ctx();
        let paths = vec![RepoRelPath::from_normalized(".env".to_string())];

        let plan = plan(
            &ctx,
            ValidationReport::default(),
            EligibilityGroups::from_files(paths),
            &git,
            &fs,
            false,
            false,
        )
        .unwrap();
        assert_eq!(plan.entries.len(), 1);
        assert!(matches!(plan.entries[0], PlannedEntry::NoOp(_)));
    }

    #[test]
    fn plan_untracked_conflict_skips_without_overwrite() {
        let fs = MockFs::new();
        fs.add_file("/source/.env", b"source");
        fs.add_file("/dest/.env", b"different");

        let git = MockPlannerGit::new(vec![]);
        let ctx = test_ctx();
        let paths = vec![RepoRelPath::from_normalized(".env".to_string())];

        let plan = plan(
            &ctx,
            ValidationReport::default(),
            EligibilityGroups::from_files(paths),
            &git,
            &fs,
            false,
            false,
        )
        .unwrap();
        assert_eq!(plan.entries.len(), 1);
        match &plan.entries[0] {
            PlannedEntry::Skip(s) => assert_eq!(s.reason, SkipReason::UntrackedConflict),
            other => panic!("expected Skip, got {other:?}"),
        }
    }

    /// The replacement-planning logic itself, exercised on every platform.
    ///
    /// The capability is supplied explicitly because the public [`plan`] reads
    /// it from the host: on a platform without the anchored replacement path
    /// this conflict is a planning failure instead, which
    /// `plan_reports_overwrite_as_unsupported_where_the_platform_cannot_replace`
    /// covers.
    #[test]
    fn plan_untracked_conflict_becomes_a_snapshot_pinned_replacement_with_overwrite() {
        let fs = MockFs::new();
        fs.add_file("/source/.env", b"source");
        fs.add_file("/dest/.env", b"different");

        let git = MockPlannerGit::new(vec![]);
        let ctx = test_ctx();
        let paths = vec![RepoRelPath::from_normalized(".env".to_string())];

        let plan = plan_with_overwrite_support(
            &ctx,
            ValidationReport::default(),
            EligibilityGroups::from_files(paths),
            &git,
            &fs,
            true,
            false,
            true,
        )
        .unwrap();

        assert_eq!(plan.entries.len(), 1);
        let PlannedEntry::Copy(op) = &plan.entries[0] else {
            panic!("expected a planned replacement, got {:?}", plan.entries[0]);
        };
        let DestinationExpectation::ReplaceExisting(snapshot) = &op.expected_destination else {
            panic!("expected a replacement expectation");
        };
        // The plan pins the destination it observed, so execution can refuse
        // to act on anything else.
        assert_eq!(snapshot, &fs.file_snapshot(&op.dst_abs).unwrap());
    }

    /// A platform that cannot replace an existing destination has to say so
    /// while planning. Otherwise `--dry-run` prints a replacement that the run
    /// will never perform, and the two exit differently.
    #[test]
    fn plan_reports_overwrite_as_unsupported_where_the_platform_cannot_replace() {
        let fs = MockFs::new();
        fs.add_file("/source/.env", b"source");
        fs.add_file("/dest/.env", b"different");

        let git = MockPlannerGit::new(vec![]);
        let ctx = test_ctx();

        let plan = plan_with_overwrite_support(
            &ctx,
            ValidationReport::default(),
            EligibilityGroups::from_files(vec![rel(".env")]),
            &git,
            &fs,
            true,
            false,
            false,
        )
        .unwrap();

        assert_eq!(plan.entries.len(), 1);
        match &plan.entries[0] {
            PlannedEntry::Failure(failure) => {
                assert_eq!(failure.rel_path.as_str(), ".env");
                assert_eq!(
                    failure.message,
                    "replacing an existing destination with --overwrite is not supported on this platform"
                );
            }
            other => panic!("expected a per-file planning failure, got {other:?}"),
        }
        // A dry run of this plan reports and exits exactly like the real run.
        assert_eq!(planning_failures(&plan), Some((1, 1)));
    }

    /// Only the `--overwrite` intent is unavailable there. The conflict itself
    /// is still an ordinary skip, and nothing about the run fails.
    #[test]
    fn plan_still_skips_a_conflict_without_overwrite_where_replacement_is_unsupported() {
        let fs = MockFs::new();
        fs.add_file("/source/.env", b"source");
        fs.add_file("/dest/.env", b"different");

        let git = MockPlannerGit::new(vec![]);
        let ctx = test_ctx();

        let plan = plan_with_overwrite_support(
            &ctx,
            ValidationReport::default(),
            EligibilityGroups::from_files(vec![rel(".env")]),
            &git,
            &fs,
            false,
            false,
            false,
        )
        .unwrap();

        assert_eq!(plan.entries.len(), 1);
        match &plan.entries[0] {
            PlannedEntry::Skip(skip) => assert_eq!(skip.reason, SkipReason::UntrackedConflict),
            other => panic!("expected Skip, got {other:?}"),
        }
        assert_eq!(planning_failures(&plan), None);
    }

    #[test]
    fn plan_source_that_vanished_fails_only_that_file() {
        struct VanishingSourceFs {
            inner: MockFs,
        }

        impl FileSystem for VanishingSourceFs {
            fn exists(&self, path: &Path) -> bool {
                self.inner.exists(path)
            }
            fn is_file(&self, path: &Path) -> bool {
                self.inner.is_file(path)
            }
            fn is_dir(&self, path: &Path) -> bool {
                self.inner.is_dir(path)
            }
            fn is_symlink(&self, path: &Path) -> bool {
                self.inner.is_symlink(path)
            }
            fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
                self.inner.read(path)
            }
            fn parent_has_symlink(&self, path: &Path) -> bool {
                self.inner.parent_has_symlink(path)
            }
            fn file_snapshot(&self, path: &Path) -> io::Result<crate::model::FileSnapshot> {
                if path == Path::new("/source/gone.env") {
                    // Simulates the source disappearing between selection and
                    // snapshotting.
                    return Err(io::Error::new(io::ErrorKind::NotFound, "vanished"));
                }
                self.inner.file_snapshot(path)
            }
            fn copy_file(
                &self,
                request: crate::fs::CopyFileRequest<'_>,
                before_publish: &mut dyn FnMut() -> io::Result<()>,
            ) -> io::Result<crate::model::PublishOutcome> {
                self.inner.copy_file(request, before_publish)
            }
        }

        let inner = MockFs::new();
        inner.add_file("/source/gone.env", b"gone");
        inner.add_file("/source/kept.env", b"kept");
        let fs = VanishingSourceFs { inner };

        let git = MockPlannerGit::new(vec![]);
        let ctx = test_ctx();
        let paths = vec![rel("gone.env"), rel("kept.env")];

        let plan = plan(
            &ctx,
            ValidationReport::default(),
            EligibilityGroups::from_files(paths),
            &git,
            &fs,
            false,
            false,
        )
        .unwrap();

        assert_eq!(plan.entries.len(), 2);
        match &plan.entries[0] {
            PlannedEntry::Failure(failure) => {
                assert_eq!(failure.rel_path.as_str(), "gone.env");
                assert!(failure.message.contains("vanished"), "{failure:?}");
            }
            other => panic!("expected a per-file failure, got {other:?}"),
        }
        assert!(matches!(plan.entries[1], PlannedEntry::Copy(_)));

        let report =
            crate::executor::execute(&plan, &fs, &git, crate::config::CopyStrategy::SimpleCopy);
        assert_eq!(report.failed, 1);
        assert_eq!(report.copied, 1);
        assert!(crate::executor::report_has_failures(&report).is_some());
    }

    /// The same loss one step earlier: the source is already gone when the
    /// planner asks what it is. Answering "unsupported source type" there would
    /// drop an eligible file from the plan and still exit zero, which is the
    /// silent omission the per-file failure exists to prevent.
    #[test]
    fn plan_source_that_vanished_before_the_type_check_fails_only_that_file() {
        let fs = MockFs::new();
        // `gone.env` was eligible when discovery listed it; nothing in the
        // filesystem answers for it now.
        fs.add_file("/source/kept.env", b"kept");

        let git = MockPlannerGit::new(vec![]);
        let ctx = test_ctx();
        let paths = vec![rel("gone.env"), rel("kept.env")];

        let dry_run = plan(
            &ctx,
            ValidationReport::default(),
            EligibilityGroups::from_files(paths.clone()),
            &git,
            &fs,
            false,
            true,
        )
        .unwrap();
        let plan = plan(
            &ctx,
            ValidationReport::default(),
            EligibilityGroups::from_files(paths),
            &git,
            &fs,
            false,
            false,
        )
        .unwrap();

        assert_eq!(plan.entries.len(), 2);
        match &plan.entries[0] {
            PlannedEntry::Failure(failure) => {
                assert_eq!(failure.rel_path.as_str(), "gone.env");
                assert!(
                    failure.message.contains("gone.env"),
                    "the failure must name the source it could not examine: {failure:?}"
                );
            }
            other => panic!("expected a per-file failure, got {other:?}"),
        }
        assert!(matches!(plan.entries[1], PlannedEntry::Copy(_)));

        // The dry run describes exactly what the executed run reports, and
        // exits the same way.
        assert_eq!(planning_failures(&dry_run), Some((1, 2)));

        let report =
            crate::executor::execute(&plan, &fs, &git, crate::config::CopyStrategy::SimpleCopy);
        assert_eq!(report.failed, 1);
        assert_eq!(report.copied, 1);
        assert!(crate::executor::report_has_failures(&report).is_some());
    }

    #[test]
    fn plan_tracked_conflict_always_skips() {
        let fs = MockFs::new();
        fs.add_file("/source/.env", b"source");

        let git = MockPlannerGit::new(vec![".env"]);
        let ctx = test_ctx();
        let paths = vec![RepoRelPath::from_normalized(".env".to_string())];

        let plan = plan(
            &ctx,
            ValidationReport::default(),
            EligibilityGroups::from_files(paths),
            &git,
            &fs,
            true,
            false,
        )
        .unwrap();
        assert_eq!(plan.entries.len(), 1);
        match &plan.entries[0] {
            PlannedEntry::Skip(s) => assert_eq!(s.reason, SkipReason::TrackedConflict),
            other => panic!("expected Skip, got {other:?}"),
        }
    }

    #[test]
    fn plan_entries_sorted_deterministically() {
        let fs = MockFs::new();
        fs.add_file("/source/c.env", b"c");
        fs.add_file("/source/a.env", b"a");
        fs.add_file("/source/b.env", b"b");

        let git = MockPlannerGit::new(vec![]);
        let ctx = test_ctx();
        let paths = vec![
            RepoRelPath::from_normalized("c.env".to_string()),
            RepoRelPath::from_normalized("a.env".to_string()),
            RepoRelPath::from_normalized("b.env".to_string()),
        ];

        let plan = plan(
            &ctx,
            ValidationReport::default(),
            EligibilityGroups::from_files(paths),
            &git,
            &fs,
            false,
            false,
        )
        .unwrap();
        let names: Vec<&str> = plan.entries.iter().map(|e| e.rel_path().as_str()).collect();
        assert_eq!(names, vec!["a.env", "b.env", "c.env"]);
    }

    #[test]
    fn plan_expands_full_directory_to_checked_file_operations() {
        let fs = MockFs::new();
        fs.add_file("/source/cfg/a.conf", b"a");
        fs.add_file("/source/cfg/b.conf", b"b");
        let git = MockPlannerGit::new(vec![]);
        let ctx = test_ctx();

        let plan = plan(
            &ctx,
            ValidationReport::default(),
            groups_with(vec![("cfg", vec!["cfg/a.conf", "cfg/b.conf"])], vec![]),
            &git,
            &fs,
            false,
            false,
        )
        .unwrap();

        assert_eq!(plan.entries.len(), 2);
        assert!(plan.entries.iter().all(|entry| {
            matches!(
                entry,
                PlannedEntry::Copy(CopyOp {
                    expected_destination: DestinationExpectation::Missing,
                    ..
                })
            )
        }));
    }

    #[test]
    fn plan_falls_back_when_dst_dir_exists() {
        let fs = MockFs::new();
        fs.add_file("/source/cfg/a.conf", b"a");
        fs.add_file("/source/cfg/b.conf", b"b");
        fs.add_dir("/dest/cfg");
        let git = MockPlannerGit::new(vec![]);
        let ctx = test_ctx();

        let plan = plan(
            &ctx,
            ValidationReport::default(),
            groups_with(vec![("cfg", vec!["cfg/a.conf", "cfg/b.conf"])], vec![]),
            &git,
            &fs,
            false,
            false,
        )
        .unwrap();

        assert_eq!(plan.entries.len(), 2);
        assert!(
            plan.entries
                .iter()
                .all(|entry| matches!(entry, PlannedEntry::Copy(_)))
        );
    }

    #[test]
    fn plan_falls_back_when_dst_parent_has_symlink() {
        let fs = MockFs::new();
        fs.add_file("/source/parent/cfg/a.conf", b"a");
        fs.add_symlink("/dest/parent");
        let git = MockPlannerGit::new(vec![]);
        let ctx = test_ctx();

        let plan = plan(
            &ctx,
            ValidationReport::default(),
            groups_with(vec![("parent/cfg", vec!["parent/cfg/a.conf"])], vec![]),
            &git,
            &fs,
            false,
            false,
        )
        .unwrap();

        match &plan.entries[0] {
            PlannedEntry::Skip(skip) => assert_eq!(skip.reason, SkipReason::UnsafePath),
            other => panic!("expected unsafe-path skip, got {other:?}"),
        }
    }

    #[test]
    fn plan_falls_back_when_any_covered_dest_file_is_tracked_even_if_missing() {
        let fs = MockFs::new();
        fs.add_file("/source/cfg/a.conf", b"a");
        fs.add_file("/source/cfg/b.conf", b"b");
        let git = MockPlannerGit::new(vec!["cfg/a.conf"]);
        let ctx = test_ctx();

        let plan = plan(
            &ctx,
            ValidationReport::default(),
            groups_with(vec![("cfg", vec!["cfg/a.conf", "cfg/b.conf"])], vec![]),
            &git,
            &fs,
            false,
            false,
        )
        .unwrap();

        assert_eq!(plan.entries.len(), 2);
        assert!(plan.entries.iter().any(|entry| matches!(
            entry,
            PlannedEntry::Skip(skip) if skip.reason == SkipReason::TrackedConflict
        )));
        assert!(
            plan.entries
                .iter()
                .any(|entry| matches!(entry, PlannedEntry::Copy(_)))
        );
    }

    #[test]
    fn plan_mixed_full_and_partial() {
        let fs = MockFs::new();
        fs.add_file("/source/.env", b"x");
        fs.add_file("/source/cfg/a.conf", b"a");
        let git = MockPlannerGit::new(vec![]);
        let ctx = test_ctx();

        let plan = plan(
            &ctx,
            ValidationReport::default(),
            groups_with(vec![("cfg", vec!["cfg/a.conf"])], vec![".env"]),
            &git,
            &fs,
            false,
            false,
        )
        .unwrap();

        assert_eq!(plan.entries.len(), 2);
        assert!(
            plan.entries
                .iter()
                .all(|entry| matches!(entry, PlannedEntry::Copy(_)))
        );
    }

    #[test]
    fn plan_full_directory_still_checks_each_source_file_type() {
        let fs = MockFs::new();
        // The manifest names a file; the source tree has a directory there.
        fs.add_dir("/source/cfg/a.conf");
        let git = MockPlannerGit::new(vec![]);
        let ctx = test_ctx();

        let plan = plan(
            &ctx,
            ValidationReport::default(),
            groups_with(vec![("cfg", vec!["cfg/a.conf"])], vec![]),
            &git,
            &fs,
            false,
            false,
        )
        .unwrap();

        assert!(matches!(
            plan.entries[0],
            PlannedEntry::Skip(SkipEntry {
                reason: SkipReason::UnsupportedSourceType,
                ..
            })
        ));
    }

    #[test]
    fn plan_counts_expanded_manifest_files_in_dry_run() {
        let fs = MockFs::new();
        fs.add_file("/source/cfg/a.conf", b"a");
        fs.add_file("/source/cfg/b.conf", b"b");
        let git = MockPlannerGit::new(vec![]);
        let ctx = test_ctx();

        let plan = plan(
            &ctx,
            ValidationReport::default(),
            groups_with(vec![("cfg", vec!["cfg/a.conf", "cfg/b.conf"])], vec![]),
            &git,
            &fs,
            false,
            true,
        )
        .unwrap();
        let report =
            crate::executor::execute(&plan, &fs, &git, crate::config::CopyStrategy::SimpleCopy);

        assert_eq!(report.copied, 2);
    }

    // --- Glue between the verified kernels and the model ---

    #[test]
    fn destination_state_maps_each_kernel_state_to_its_namesake() {
        let pairs = [
            (DestState::Missing, DestinationState::Missing),
            (DestState::UpToDate, DestinationState::UpToDate),
            (
                DestState::PermissionsDiffer,
                DestinationState::PermissionsDiffer,
            ),
            (
                DestState::UntrackedConflict,
                DestinationState::UntrackedConflict,
            ),
            (
                DestState::TrackedConflict,
                DestinationState::TrackedConflict,
            ),
            (DestState::TypeConflict, DestinationState::TypeConflict),
            (DestState::UnsafePath, DestinationState::UnsafePath),
        ];
        for (kernel, model) in pairs {
            assert_eq!(destination_state(kernel), model, "{kernel:?}");
        }
    }

    #[test]
    fn skip_reason_maps_each_kernel_kind_to_its_namesake() {
        let pairs = [
            (SkipKind::UntrackedConflict, SkipReason::UntrackedConflict),
            (SkipKind::PermissionsDiffer, SkipReason::PermissionsDiffer),
            (SkipKind::TrackedConflict, SkipReason::TrackedConflict),
            (SkipKind::TypeConflict, SkipReason::TypeConflict),
            (SkipKind::UnsafePath, SkipReason::UnsafePath),
            (
                SkipKind::UnsupportedSourceType,
                SkipReason::UnsupportedSourceType,
            ),
        ];
        for (kernel, model) in pairs {
            assert_eq!(skip_reason(kernel), model, "{kernel:?}");
        }
    }

    #[test]
    fn observations_map_each_filesystem_answer() {
        assert_eq!(compare_obs(Ok(FileComparison::Equal)), CompareObs::Equal);
        assert_eq!(
            compare_obs(Ok(FileComparison::PermissionsDiffer)),
            CompareObs::PermissionsDiffer
        );
        assert_eq!(
            compare_obs(Ok(FileComparison::ContentDiffers)),
            CompareObs::ContentDiffers
        );
        assert_eq!(
            compare_obs(Err(io::Error::other("unreadable"))),
            CompareObs::Error
        );
        assert_eq!(
            source_kind_obs(SourceKind::RegularFile),
            SourceKindObs::RegularFile
        );
        assert_eq!(source_kind_obs(SourceKind::Symlink), SourceKindObs::Symlink);
        assert_eq!(source_kind_obs(SourceKind::Other), SourceKindObs::Other);

        let snapshot = FileSnapshot::new(7, 0xfeed, 0o644, Some((1, 2)));
        let kernel = kernel_snapshot(snapshot.clone());
        assert_eq!(kernel.key.len, 7);
        assert_eq!(kernel.key.fingerprint, 0xfeed);
        assert_eq!(kernel.payload, snapshot);
    }

    #[test]
    fn entry_kind_maps_each_planned_entry_shape() {
        let snapshot = || FileSnapshot::new(1, 1, 0, None);
        let copy = |expected_destination| {
            PlannedEntry::Copy(CopyOp {
                rel_path: rel("a"),
                src_abs: PathBuf::from("/source/a"),
                dst_abs: PathBuf::from("/dest/a"),
                expected_source: snapshot(),
                expected_destination,
            })
        };
        let cases = [
            (
                copy(DestinationExpectation::Missing),
                EntryKind::CopyMissing,
            ),
            (
                copy(DestinationExpectation::ReplaceExisting(snapshot())),
                EntryKind::CopyReplace,
            ),
            (
                copy(DestinationExpectation::RepairPermissions(snapshot())),
                EntryKind::CopyRepair,
            ),
            (
                PlannedEntry::NoOp(NoOpEntry {
                    rel_path: rel("a"),
                    reason: NoOpReason::UpToDate,
                }),
                EntryKind::NoOp,
            ),
            (
                PlannedEntry::Skip(SkipEntry {
                    rel_path: rel("a"),
                    reason: SkipReason::TypeConflict,
                }),
                EntryKind::Skip,
            ),
            (
                PlannedEntry::Failure(FailureEntry {
                    rel_path: rel("a"),
                    message: "x".to_string(),
                }),
                EntryKind::Failure,
            ),
        ];
        for (entry, kind) in cases {
            assert_eq!(entry_kind(&entry), kind, "{entry:?}");
        }
    }

    /// A filesystem that records which queries planning makes, in order, and
    /// can be told what a comparison and the destination snapshot report.
    struct RecordingFs {
        inner: MockFs,
        log: RefCell<Vec<&'static str>>,
        comparison: Option<FileComparison>,
        dest_snapshot: Option<crate::model::FileSnapshot>,
    }

    impl RecordingFs {
        fn new(inner: MockFs) -> Self {
            Self {
                inner,
                log: RefCell::new(Vec::new()),
                comparison: None,
                dest_snapshot: None,
            }
        }

        fn take_log(&self) -> Vec<&'static str> {
            std::mem::take(&mut *self.log.borrow_mut())
        }
    }

    impl FileSystem for RecordingFs {
        fn exists(&self, path: &Path) -> bool {
            self.log.borrow_mut().push("exists");
            self.inner.exists(path)
        }
        fn is_file(&self, path: &Path) -> bool {
            self.log.borrow_mut().push("is_file");
            self.inner.is_file(path)
        }
        fn is_dir(&self, path: &Path) -> bool {
            self.log.borrow_mut().push("is_dir");
            self.inner.is_dir(path)
        }
        fn is_symlink(&self, path: &Path) -> bool {
            self.log.borrow_mut().push("is_symlink");
            self.inner.is_symlink(path)
        }
        fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
            self.inner.read(path)
        }
        fn compare_files(&self, left: &Path, right: &Path) -> io::Result<FileComparison> {
            self.log.borrow_mut().push("compare_files");
            match self.comparison {
                Some(comparison) => Ok(comparison),
                None => self.inner.compare_files(left, right),
            }
        }
        fn file_snapshot(&self, path: &Path) -> io::Result<crate::model::FileSnapshot> {
            if path.starts_with("/dest") {
                self.log.borrow_mut().push("file_snapshot(dest)");
                if let Some(snapshot) = &self.dest_snapshot {
                    return Ok(snapshot.clone());
                }
            } else {
                self.log.borrow_mut().push("file_snapshot(source)");
            }
            self.inner.file_snapshot(path)
        }
        fn source_kind(&self, path: &Path) -> io::Result<SourceKind> {
            self.log.borrow_mut().push("source_kind");
            self.inner.source_kind(path)
        }
        fn parent_has_symlink(&self, path: &Path) -> bool {
            self.log.borrow_mut().push("parent_has_symlink");
            self.inner.parent_has_symlink(path)
        }
        fn copy_file(
            &self,
            request: crate::fs::CopyFileRequest<'_>,
            before_publish: &mut dyn FnMut() -> io::Result<()>,
        ) -> io::Result<crate::model::PublishOutcome> {
            self.inner.copy_file(request, before_publish)
        }
    }

    fn plan_one(fs: &RecordingFs, tracked: Vec<&str>, overwrite: bool) -> PlannedEntry {
        let git = MockPlannerGit::new(tracked);
        let mut plan = plan_with_overwrite_support(
            &test_ctx(),
            ValidationReport::default(),
            EligibilityGroups::from_files(vec![rel(".env")]),
            &git,
            fs,
            overwrite,
            false,
            true,
        )
        .unwrap();
        assert_eq!(plan.entries.len(), 1);
        plan.entries.remove(0)
    }

    /// The queries planning makes, in order, for each kind of destination.
    /// These are the sequences `planner::entry::entry_queries` and
    /// `planner::destination::dest_queries` specify.
    #[test]
    fn planning_queries_the_filesystem_in_the_specified_order() {
        // Missing destination: classification, then the parent re-check, then
        // the source snapshot.
        let fs = RecordingFs::new(MockFs::new());
        fs.inner.add_file("/source/.env", b"x");
        assert!(matches!(
            plan_one(&fs, vec![], false),
            PlannedEntry::Copy(_)
        ));
        assert_eq!(
            fs.take_log(),
            [
                "source_kind",
                "parent_has_symlink",
                "exists",
                "parent_has_symlink",
                "file_snapshot(source)",
            ]
        );

        // Tracked destination: no destination query at all.
        let fs = RecordingFs::new(MockFs::new());
        fs.inner.add_file("/source/.env", b"x");
        fs.inner.add_file("/dest/.env", b"y");
        assert!(matches!(
            plan_one(&fs, vec![".env"], true),
            PlannedEntry::Skip(SkipEntry {
                reason: SkipReason::TrackedConflict,
                ..
            })
        ));
        assert_eq!(fs.take_log(), ["source_kind"]);

        // Existing regular file, `--overwrite`: the full classification, then
        // the source snapshot before the destination snapshot.
        let fs = RecordingFs::new(MockFs::new());
        fs.inner.add_file("/source/.env", b"x");
        fs.inner.add_file("/dest/.env", b"y");
        assert!(matches!(plan_one(&fs, vec![], true), PlannedEntry::Copy(_)));
        assert_eq!(
            fs.take_log(),
            [
                "source_kind",
                "parent_has_symlink",
                "exists",
                "is_symlink",
                "is_file",
                "compare_files",
                "file_snapshot(source)",
                "file_snapshot(dest)",
            ]
        );

        // A destination symlink is a type conflict without asking `is_file`.
        let fs = RecordingFs::new(MockFs::new());
        fs.inner.add_file("/source/.env", b"x");
        fs.inner.add_symlink("/dest/.env");
        assert!(matches!(
            plan_one(&fs, vec![], true),
            PlannedEntry::Skip(SkipEntry {
                reason: SkipReason::TypeConflict,
                ..
            })
        ));
        assert_eq!(
            fs.take_log(),
            ["source_kind", "parent_has_symlink", "exists", "is_symlink",]
        );
    }

    /// A permission-only difference is repaired only while the pinned
    /// snapshots still agree on content; otherwise it is a replacement.
    #[test]
    fn plan_repairs_permissions_only_while_pinned_snapshots_agree() {
        let make = |dest_snapshot: Option<crate::model::FileSnapshot>| {
            let mut fs = RecordingFs::new(MockFs::new());
            fs.inner.add_file("/source/.env", b"same");
            fs.inner.add_file("/dest/.env", b"same");
            fs.comparison = Some(FileComparison::PermissionsDiffer);
            fs.dest_snapshot = dest_snapshot;
            fs
        };

        let fs = make(None);
        match plan_one(&fs, vec![], true) {
            PlannedEntry::Copy(CopyOp {
                expected_destination: DestinationExpectation::RepairPermissions(snapshot),
                ..
            }) => assert_eq!(
                snapshot,
                fs.inner.file_snapshot(Path::new("/dest/.env")).unwrap()
            ),
            other => panic!("expected a permission repair, got {other:?}"),
        }

        // The destination was rewritten between the comparison and the
        // snapshot: same length, different content.
        let rewritten = crate::model::FileSnapshot::new(4, 0, 0, None);
        let fs = make(Some(rewritten.clone()));
        match plan_one(&fs, vec![], true) {
            PlannedEntry::Copy(CopyOp {
                expected_destination: DestinationExpectation::ReplaceExisting(snapshot),
                ..
            }) => assert_eq!(snapshot, rewritten),
            other => panic!("expected a replacement, got {other:?}"),
        }

        // Without `--overwrite` it is the permissions skip.
        let fs = make(None);
        assert!(matches!(
            plan_one(&fs, vec![], false),
            PlannedEntry::Skip(SkipEntry {
                reason: SkipReason::PermissionsDiffer,
                ..
            })
        ));
    }
}
