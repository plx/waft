//! Read-only copy planning.
//!
//! The planner takes discovered eligible files and classifies each
//! destination path, producing a [`CopyPlan`] that the executor can
//! apply. The planner **never** mutates the filesystem.

use std::collections::HashSet;

use crate::eligibility_groups::EligibilityGroups;
use crate::error::Result;
use crate::fs::{FileComparison, FileSystem, SourceKind};
use crate::git::GitBackend;
use crate::model::{
    CopyOp, CopyPlan, DestinationExpectation, DestinationState, FailureEntry, NoOpEntry,
    NoOpReason, PlannedEntry, RepoContext, SkipEntry, SkipReason, ValidationReport,
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

        // Check source type. A source that was examined and is not a regular
        // file — a directory, a symlink, a device — is nothing waft copies, and
        // skipping it is the whole answer. A source that could not be examined
        // at all is a different thing: it was eligible when it was discovered,
        // so it has vanished or become unreachable since, and reporting that as
        // an unsupported type would drop a file the caller asked for from both
        // the plan and the exit status. That is the same per-file failure the
        // snapshot below records, moved to the first place the loss can be
        // seen.
        match fs.source_kind(&src_abs) {
            Ok(SourceKind::RegularFile) => {}
            Ok(SourceKind::Symlink | SourceKind::Other) => {
                entries.push(PlannedEntry::Skip(SkipEntry {
                    rel_path,
                    reason: SkipReason::UnsupportedSourceType,
                }));
                continue;
            }
            Err(error) => {
                entries.push(PlannedEntry::Failure(FailureEntry {
                    rel_path,
                    message: format!("failed to examine source {}: {error}", src_abs.display()),
                }));
                continue;
            }
        }

        // Classify destination state
        let dest_state = classify_destination(&rel_path, &src_abs, &dst_abs, &dest_tracked, fs);

        match dest_state {
            DestinationState::Missing => {
                // Check for symlinked parent in destination
                if fs.parent_has_symlink(&dst_abs) {
                    entries.push(PlannedEntry::Skip(SkipEntry {
                        rel_path,
                        reason: SkipReason::UnsafePath,
                    }));
                } else {
                    // A source that vanished or became unreadable between
                    // selection and snapshotting is one file's problem, not the
                    // run's. Record it as a per-file failure and keep planning.
                    let expected_source = match fs.file_snapshot(&src_abs) {
                        Ok(snapshot) => snapshot,
                        Err(error) => {
                            entries.push(PlannedEntry::Failure(FailureEntry {
                                rel_path,
                                message: format!(
                                    "failed to snapshot source {}: {error}",
                                    src_abs.display()
                                ),
                            }));
                            continue;
                        }
                    };
                    entries.push(PlannedEntry::Copy(CopyOp {
                        rel_path,
                        src_abs,
                        dst_abs,
                        expected_source,
                        expected_destination: DestinationExpectation::Missing,
                    }));
                }
            }
            DestinationState::UpToDate => {
                entries.push(PlannedEntry::NoOp(NoOpEntry {
                    rel_path,
                    reason: NoOpReason::UpToDate,
                }));
            }
            DestinationState::UntrackedConflict | DestinationState::PermissionsDiffer => {
                let permissions_only = dest_state == DestinationState::PermissionsDiffer;
                if !overwrite {
                    entries.push(PlannedEntry::Skip(SkipEntry {
                        rel_path,
                        reason: if permissions_only {
                            SkipReason::PermissionsDiffer
                        } else {
                            SkipReason::UntrackedConflict
                        },
                    }));
                    continue;
                }

                if !overwrite_supported {
                    // The plan, its `--dry-run` rendering, the executed run,
                    // and the exit status have to describe the same thing. A
                    // platform with no way to replace or repair an existing
                    // destination says so once, here, instead of promising a
                    // replacement that only fails when it is attempted.
                    entries.push(PlannedEntry::Failure(FailureEntry {
                        rel_path,
                        message: OVERWRITE_UNSUPPORTED.to_string(),
                    }));
                    continue;
                }

                // Both snapshots pin the exact bytes and mode the plan was
                // built against. Execution re-opens each one and refuses to act
                // unless it still matches, so a file that changes in between is
                // a per-file failure rather than a clobber.
                let expected_source = match fs.file_snapshot(&src_abs) {
                    Ok(snapshot) => snapshot,
                    Err(error) => {
                        entries.push(PlannedEntry::Failure(FailureEntry {
                            rel_path,
                            message: format!(
                                "failed to snapshot source {}: {error}",
                                src_abs.display()
                            ),
                        }));
                        continue;
                    }
                };
                let expected_dest_snapshot = match fs.file_snapshot(&dst_abs) {
                    Ok(snapshot) => snapshot,
                    Err(error) => {
                        entries.push(PlannedEntry::Failure(FailureEntry {
                            rel_path,
                            message: format!(
                                "failed to snapshot destination {}: {error}",
                                dst_abs.display()
                            ),
                        }));
                        continue;
                    }
                };
                // Classification proved byte equality, but that proof and the
                // two snapshots above are separate reads. Only keep the repair
                // intent if the pinned snapshots themselves still agree on
                // content; otherwise something was rewritten in between and
                // this is an ordinary replacement.
                let expected_destination = if permissions_only
                    && expected_dest_snapshot.content_matches(&expected_source)
                {
                    DestinationExpectation::RepairPermissions(expected_dest_snapshot)
                } else {
                    DestinationExpectation::ReplaceExisting(expected_dest_snapshot)
                };
                entries.push(PlannedEntry::Copy(CopyOp {
                    rel_path,
                    src_abs,
                    dst_abs,
                    expected_source,
                    expected_destination,
                }));
            }
            DestinationState::TrackedConflict => {
                entries.push(PlannedEntry::Skip(SkipEntry {
                    rel_path,
                    reason: SkipReason::TrackedConflict,
                }));
            }
            DestinationState::TypeConflict => {
                entries.push(PlannedEntry::Skip(SkipEntry {
                    rel_path,
                    reason: SkipReason::TypeConflict,
                }));
            }
            DestinationState::UnsafePath => {
                entries.push(PlannedEntry::Skip(SkipEntry {
                    rel_path,
                    reason: SkipReason::UnsafePath,
                }));
            }
        }
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

/// Classify the state of a destination path.
pub(crate) fn classify_destination(
    rel_path: &RepoRelPath,
    src_abs: &std::path::Path,
    dst_abs: &std::path::Path,
    dest_tracked: &HashSet<RepoRelPath>,
    fs: &dyn FileSystem,
) -> DestinationState {
    // Check if tracked in destination
    if dest_tracked.contains(rel_path) {
        return DestinationState::TrackedConflict;
    }

    // Check if destination parent has symlinks
    if fs.parent_has_symlink(dst_abs) {
        return DestinationState::UnsafePath;
    }

    if !fs.exists(dst_abs) {
        return DestinationState::Missing;
    }

    // Destination exists
    if fs.is_symlink(dst_abs) || !fs.is_file(dst_abs) {
        return DestinationState::TypeConflict;
    }

    // It's a regular file — compare content and relevant permissions without
    // retaining both complete files in memory.
    match fs.compare_files(src_abs, dst_abs) {
        Ok(FileComparison::Equal) => DestinationState::UpToDate,
        Ok(FileComparison::PermissionsDiffer) => DestinationState::PermissionsDiffer,
        Ok(FileComparison::ContentDiffers) | Err(_) => DestinationState::UntrackedConflict,
    }
}

/// The per-file planning failures in a plan, as `(failed, total)`.
///
/// This mirrors [`crate::executor::report_has_failures`] so a dry run reports
/// and exits exactly like the run it is describing: a file that planning could
/// not describe is a failure whether or not anything is written.
pub fn planning_failures(plan: &CopyPlan) -> Option<(usize, usize)> {
    let failed = plan
        .entries
        .iter()
        .filter(|entry| matches!(entry, PlannedEntry::Failure(_)))
        .count();
    if failed == 0 {
        return None;
    }
    let actionable = plan
        .entries
        .iter()
        .filter(|entry| matches!(entry, PlannedEntry::Copy(_)))
        .count();
    Some((failed, failed + actionable))
}

/// Render a dry-run plan.
///
/// The plan itself goes to stdout and is suppressed by `quiet`; per-file
/// failures go to stderr unconditionally, because `quiet` suppresses routine
/// progress, never the reason a run is about to exit nonzero.
pub fn render_dry_run(plan: &CopyPlan, quiet: bool) {
    let mut copies = 0usize;
    let mut replacements = 0usize;
    let mut repairs = 0usize;
    let mut skips = 0usize;
    let mut noops = 0usize;
    let mut failures = 0usize;

    for entry in &plan.entries {
        match entry {
            PlannedEntry::Copy(op) => match &op.expected_destination {
                DestinationExpectation::Missing => {
                    copies += 1;
                    if !quiet {
                        println!("copy: {}", op.rel_path);
                    }
                }
                DestinationExpectation::ReplaceExisting(_) => {
                    replacements += 1;
                    if !quiet {
                        println!("replace: {} (untracked conflict)", op.rel_path);
                    }
                }
                DestinationExpectation::RepairPermissions(_) => {
                    repairs += 1;
                    if !quiet {
                        println!(
                            "repair permissions: {} (content equal, permissions differ)",
                            op.rel_path
                        );
                    }
                }
            },
            PlannedEntry::NoOp(entry) => {
                noops += 1;
                if !quiet {
                    println!("no-op: {} ({:?})", entry.rel_path, entry.reason);
                }
            }
            PlannedEntry::Skip(entry) => {
                skips += 1;
                if !quiet {
                    println!("skip: {} ({})", entry.rel_path, entry.reason);
                }
            }
            PlannedEntry::Failure(entry) => {
                failures += 1;
                // Same channel and prefix as an executed run's failures, so a
                // dry run that will exit nonzero says so even under `--quiet`.
                eprintln!("FAILED: {}: {}", entry.rel_path, entry.message);
            }
        }
    }

    if quiet {
        return;
    }

    let mut summary = format!("dry run: {copies} to copy");
    if replacements > 0 {
        summary.push_str(&format!(", {replacements} to replace"));
    }
    if repairs > 0 {
        summary.push_str(&format!(", {repairs} permission repair(s)"));
    }
    summary.push_str(&format!(", {skips} to skip, {noops} up-to-date"));
    if failures > 0 {
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
}
