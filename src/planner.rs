//! Read-only copy planning.
//!
//! The planner takes discovered eligible files and classifies each
//! destination path, producing a [`CopyPlan`] that the executor can
//! apply. The planner **never** mutates the filesystem.

use std::collections::HashSet;

use crate::eligibility_groups::EligibilityGroups;
use crate::error::Result;
use crate::fs::FileSystem;
use crate::git::GitBackend;
use crate::model::{
    CopyDirOp, CopyOp, CopyPlan, DestinationState, NoOpEntry, NoOpReason, PlannedEntry,
    RepoContext, SkipEntry, SkipReason, ValidationReport,
};
use crate::path::RepoRelPath;

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

    for dir in groups.full_dirs {
        let dst_abs = dir.rel_path.to_path(dest_root);
        let has_tracked_dest = dir.files.iter().any(|file| dest_tracked.contains(file));
        if has_tracked_dest
            || fs.parent_has_symlink(&dst_abs)
            || fs.exists(&dst_abs)
            || fs.is_symlink(&dst_abs)
        {
            per_file_paths.extend(dir.files);
            continue;
        }

        entries.push(PlannedEntry::CopyDir(CopyDirOp {
            src_abs: dir.rel_path.to_path(&ctx.source_root),
            dst_abs,
            rel_path: dir.rel_path,
            files: dir.files,
        }));
    }

    per_file_paths.sort();
    per_file_paths.dedup();

    for rel_path in per_file_paths {
        let src_abs = rel_path.to_path(&ctx.source_root);
        let dst_abs = rel_path.to_path(dest_root);

        // Check source type
        if !fs.is_file(&src_abs) {
            entries.push(PlannedEntry::Skip(SkipEntry {
                rel_path,
                reason: SkipReason::UnsupportedSourceType,
            }));
            continue;
        }

        // Check if source is a symlink
        if fs.is_symlink(&src_abs) {
            entries.push(PlannedEntry::Skip(SkipEntry {
                rel_path,
                reason: SkipReason::UnsupportedSourceType,
            }));
            continue;
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
                    entries.push(PlannedEntry::Copy(CopyOp {
                        rel_path,
                        src_abs,
                        dst_abs,
                    }));
                }
            }
            DestinationState::UpToDate => {
                entries.push(PlannedEntry::NoOp(NoOpEntry {
                    rel_path,
                    reason: NoOpReason::UpToDate,
                }));
            }
            DestinationState::UntrackedConflict => {
                if overwrite {
                    entries.push(PlannedEntry::Copy(CopyOp {
                        rel_path,
                        src_abs,
                        dst_abs,
                    }));
                } else {
                    entries.push(PlannedEntry::Skip(SkipEntry {
                        rel_path,
                        reason: SkipReason::UntrackedConflict,
                    }));
                }
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
    if !fs.is_file(dst_abs) {
        return DestinationState::TypeConflict;
    }

    // It's a regular file — check if identical
    let src_data = fs.read(src_abs);
    let dst_data = fs.read(dst_abs);

    match (src_data, dst_data) {
        (Ok(s), Ok(d)) if s == d => DestinationState::UpToDate,
        _ => DestinationState::UntrackedConflict,
    }
}

/// Render a dry-run plan to stdout.
pub fn render_dry_run(plan: &CopyPlan) {
    for entry in &plan.entries {
        match entry {
            PlannedEntry::Copy(op) => {
                println!("copy: {}", op.rel_path);
            }
            PlannedEntry::CopyDir(op) => {
                println!("copy-dir: {} ({} files)", op.rel_path, op.file_count());
            }
            PlannedEntry::NoOp(entry) => {
                println!("no-op: {} ({:?})", entry.rel_path, entry.reason);
            }
            PlannedEntry::Skip(entry) => {
                println!("skip: {} ({:?})", entry.rel_path, entry.reason);
            }
        }
    }

    let copies = plan
        .entries
        .iter()
        .fold(0usize, |count, entry| match entry {
            PlannedEntry::Copy(_) => count + 1,
            PlannedEntry::CopyDir(op) => count + op.file_count(),
            _ => count,
        });
    let skips = plan
        .entries
        .iter()
        .filter(|e| matches!(e, PlannedEntry::Skip(_)))
        .count();
    let noops = plan
        .entries
        .iter()
        .filter(|e| matches!(e, PlannedEntry::NoOp(_)))
        .count();

    eprintln!(
        "dry run: {} to copy, {} to skip, {} up-to-date",
        copies, skips, noops
    );
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

        fn create_dir_all(&self, path: &Path) -> io::Result<()> {
            self.dirs.borrow_mut().insert(path.to_path_buf());
            Ok(())
        }

        fn copy_file(
            &self,
            src: &Path,
            dst: &Path,
            _strategy: crate::config::CopyStrategy,
        ) -> io::Result<()> {
            let data = self
                .files
                .borrow()
                .get(src)
                .cloned()
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "not found"))?;
            self.files.borrow_mut().insert(dst.to_path_buf(), data);
            Ok(())
        }

        fn copy_dir_exact(
            &self,
            _src: &Path,
            _dst: &Path,
            _expected_files: &[PathBuf],
            _strategy: crate::config::CopyStrategy,
        ) -> io::Result<()> {
            Ok(())
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
        assert!(matches!(plan.entries[0], PlannedEntry::Copy(_)));
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

    #[test]
    fn plan_untracked_conflict_copies_with_overwrite() {
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
            true,
            false,
        )
        .unwrap();
        assert_eq!(plan.entries.len(), 1);
        assert!(matches!(plan.entries[0], PlannedEntry::Copy(_)));
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
    fn plan_emits_copydir_for_missing_untracked_dst() {
        let fs = MockFs::new();
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

        assert!(matches!(plan.entries[0], PlannedEntry::CopyDir(_)));
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
                .any(|entry| matches!(entry, PlannedEntry::CopyDir(_)))
        );
        assert!(
            plan.entries
                .iter()
                .any(|entry| matches!(entry, PlannedEntry::Copy(_)))
        );
    }

    #[test]
    fn plan_copydir_skips_source_file_type_checks_for_dir_itself() {
        let fs = MockFs::new();
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

        assert!(matches!(plan.entries[0], PlannedEntry::CopyDir(_)));
    }

    #[test]
    fn plan_counts_copydir_by_manifest_size_in_dry_run() {
        let fs = MockFs::new();
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
        let report = crate::executor::execute(&plan, &fs, crate::config::CopyStrategy::SimpleCopy);

        assert_eq!(report.copied, 2);
    }
}
