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
use crate::model::{
    CopyOutcome, CopyPlan, CopyReport, CopyResult, CopyResultKind, DestinationExpectation,
    PlannedEntry, PublishOutcome,
};

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
    let mut replaced = 0usize;
    let mut permissions_repaired = 0usize;
    let mut failed = 0usize;
    let mut skipped = 0usize;
    let mut up_to_date = 0usize;

    for entry in &plan.entries {
        match entry {
            PlannedEntry::Copy(op) => {
                if plan.dry_run {
                    let outcome = match &op.expected_destination {
                        DestinationExpectation::Missing => {
                            copied += 1;
                            CopyOutcome::Copied
                        }
                        DestinationExpectation::ReplaceExisting(_) => {
                            replaced += 1;
                            CopyOutcome::Replaced
                        }
                        DestinationExpectation::RepairPermissions(_) => {
                            permissions_repaired += 1;
                            CopyOutcome::PermissionsRepaired
                        }
                    };
                    results.push(CopyResult {
                        rel_path: op.rel_path.clone(),
                        kind: CopyResultKind::File,
                        outcome,
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
                    Ok(outcome) => {
                        match outcome {
                            PublishOutcome::Created => copied += 1,
                            PublishOutcome::Replaced => replaced += 1,
                            PublishOutcome::PermissionsRepaired => permissions_repaired += 1,
                        }
                        results.push(CopyResult {
                            rel_path: op.rel_path.clone(),
                            kind: CopyResultKind::File,
                            outcome: outcome.into(),
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
            PlannedEntry::Failure(entry) => {
                // Planning could not describe this file. Report it like any
                // other per-file failure so the run's exit status reflects it
                // while every other entry still executes.
                failed += 1;
                results.push(CopyResult {
                    rel_path: entry.rel_path.clone(),
                    kind: CopyResultKind::File,
                    outcome: CopyOutcome::Failed {
                        message: format!("{}: {}", entry.rel_path, entry.message),
                    },
                });
            }
        }
    }

    CopyReport {
        results,
        copied,
        replaced,
        permissions_repaired,
        failed,
        skipped,
        up_to_date,
    }
}

/// Delays between attempts to take the destination index lock.
///
/// An IDE, fsmonitor, or background `git status` holds `index.lock` for a few
/// milliseconds at a time. Retrying briefly keeps those from turning into
/// sporadic per-file failures without hiding a genuinely stuck lock.
const INDEX_LOCK_RETRY_DELAYS: &[std::time::Duration] = &[
    std::time::Duration::from_millis(50),
    std::time::Duration::from_millis(100),
];

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

        // Install the handlers and stage the path *before* the file exists, so
        // arming afterwards is a single atomic store. Anything between creating
        // the lock and arming is a window where an interrupt would leave the
        // lock behind, and this makes that window as small as it can be.
        interrupt_cleanup::stage(&path);

        let mut attempt = 0usize;
        let file = loop {
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => break file,
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    let Some(delay) = INDEX_LOCK_RETRY_DELAYS.get(attempt) else {
                        return Err(io::Error::new(
                            io::ErrorKind::WouldBlock,
                            format!(
                                "destination Git index is locked at {} after {} attempt(s); refusing to publish",
                                path.display(),
                                INDEX_LOCK_RETRY_DELAYS.len() + 1
                            ),
                        ));
                    };
                    attempt += 1;
                    std::thread::sleep(*delay);
                }
                Err(error) => {
                    return Err(io::Error::new(
                        error.kind(),
                        format!(
                            "failed to lock destination Git index at {}: {error}",
                            path.display()
                        ),
                    ));
                }
            }
        };

        // The lock now exists and is ours, so the staged path becomes live: an
        // interrupt during the publish window will not leave a stale lock
        // behind for the next Git command.
        interrupt_cleanup::arm();
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
        // Disarm last. An interrupt arriving mid-`Drop` then still finds the
        // path armed and unlinks it; unlinking an already-removed path is a
        // harmless `ENOENT`, whereas disarming first would leave a window in
        // which neither the handler nor this function removes the lock.
        interrupt_cleanup::disarm();
    }
}

/// Signal-safe removal of the one lock file this process may hold.
///
/// Unix installs `SIGINT`/`SIGTERM` handlers that unlink the recorded path and
/// then re-raise the signal under its previous disposition. On other platforms
/// this degrades to a no-op: an interrupt can still leave a stale
/// `.git/**/index.lock` that the user must delete.
#[cfg(unix)]
mod interrupt_cleanup {
    use std::cell::UnsafeCell;
    use std::mem::MaybeUninit;
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;
    use std::ptr;
    use std::sync::Once;
    use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

    /// Longer than `PATH_MAX` on every platform waft supports.
    const PATH_CAPACITY: usize = 4096;

    const EMPTY: u8 = 0;
    const WRITING: u8 = 1;
    const STAGED: u8 = 2;
    const ARMED: u8 = 3;

    struct SignalCell<T>(UnsafeCell<T>);
    // Access is confined to the single-threaded acquire/release path and to a
    // signal handler that only reads once `STATE` says the value is complete.
    unsafe impl<T> Sync for SignalCell<T> {}

    static STATE: AtomicU8 = AtomicU8::new(EMPTY);
    static PATH: SignalCell<[u8; PATH_CAPACITY]> = SignalCell(UnsafeCell::new([0; PATH_CAPACITY]));

    static HANDLERS_INSTALLED: Once = Once::new();
    static INTERRUPT_PREVIOUS_READY: AtomicBool = AtomicBool::new(false);
    static TERMINATE_PREVIOUS_READY: AtomicBool = AtomicBool::new(false);
    static PREVIOUS_INTERRUPT: SignalCell<MaybeUninit<libc::sigaction>> =
        SignalCell(UnsafeCell::new(MaybeUninit::uninit()));
    static PREVIOUS_TERMINATE: SignalCell<MaybeUninit<libc::sigaction>> =
        SignalCell(UnsafeCell::new(MaybeUninit::uninit()));

    /// Record `path` as the lock this process is about to create.
    ///
    /// Staging is separate from arming so that the caller can write the path
    /// and install the handlers *before* the lock file exists, leaving only a
    /// single atomic store between "the lock is ours" and "an interrupt will
    /// clean it up".
    pub(super) fn stage(path: &Path) {
        let bytes = path.as_os_str().as_bytes();
        if bytes.len() >= PATH_CAPACITY {
            // Nothing safe to record; normal `Drop` cleanup still applies, and
            // leaving the state `EMPTY` keeps `arm` from publishing a stale
            // path from an earlier lock.
            STATE.store(EMPTY, Ordering::SeqCst);
            return;
        }
        install_handlers();
        STATE.store(WRITING, Ordering::SeqCst);
        // SAFETY: `STATE` is not `ARMED`, so no handler will read the buffer
        // while it is being written, and only one lock is live at a time.
        unsafe {
            let buffer = &mut *PATH.0.get();
            buffer[..bytes.len()].copy_from_slice(bytes);
            buffer[bytes.len()] = 0;
        }
        STATE.store(STAGED, Ordering::SeqCst);
    }

    /// Promote the staged path to the live lock.
    ///
    /// Does nothing unless a path was successfully staged, so a lock whose path
    /// could not be recorded never adopts a previous lock's path.
    pub(super) fn arm() {
        let _ = STATE.compare_exchange(STAGED, ARMED, Ordering::SeqCst, Ordering::SeqCst);
    }

    /// Stop treating any path as the live lock.
    pub(super) fn disarm() {
        STATE.store(EMPTY, Ordering::SeqCst);
    }

    /// Unlink the recorded lock path, if one is armed.
    ///
    /// Async-signal-safe: an atomic load plus `unlink(2)`.
    pub(super) fn remove_armed_lock() {
        if STATE.load(Ordering::SeqCst) != ARMED {
            return;
        }
        // SAFETY: `ARMED` means the buffer holds a complete NUL-terminated
        // path that stays valid until `disarm`.
        unsafe {
            libc::unlink(PATH.0.get().cast::<libc::c_char>());
        }
    }

    extern "C" fn handle_interrupt(signal: libc::c_int) {
        // This handler is only installed for signals whose previous
        // disposition was not `SIG_IGN`, so reaching here means the process is
        // going to act on the signal rather than sail past it. Removing the
        // lock first is therefore safe: nothing resumes publishing afterwards.
        remove_armed_lock();
        // Restore what was installed before waft and let it decide the
        // process's fate.
        unsafe {
            let (previous, ready) = match signal {
                libc::SIGINT => (PREVIOUS_INTERRUPT.0.get(), &INTERRUPT_PREVIOUS_READY),
                _ => (PREVIOUS_TERMINATE.0.get(), &TERMINATE_PREVIOUS_READY),
            };
            if ready.load(Ordering::SeqCst) {
                libc::sigaction(signal, (*previous).as_ptr(), ptr::null_mut());
            } else {
                let mut default: libc::sigaction = std::mem::zeroed();
                default.sa_sigaction = libc::SIG_DFL;
                libc::sigaction(signal, &default, ptr::null_mut());
            }
            libc::raise(signal);
        }
    }

    /// Installed once, from `stage`, before any lock file exists. Nothing is
    /// armed yet, so a signal arriving during installation finds nothing to
    /// clean up and simply follows its previous disposition.
    fn install_handlers() {
        HANDLERS_INSTALLED.call_once(|| unsafe {
            let mut action: libc::sigaction = std::mem::zeroed();
            action.sa_sigaction = handle_interrupt as usize;
            action.sa_flags = libc::SA_RESTART;
            libc::sigemptyset(&mut action.sa_mask);

            if install_unless_ignored(libc::SIGINT, &action, PREVIOUS_INTERRUPT.0.get()) {
                INTERRUPT_PREVIOUS_READY.store(true, Ordering::SeqCst);
            }
            if install_unless_ignored(libc::SIGTERM, &action, PREVIOUS_TERMINATE.0.get()) {
                TERMINATE_PREVIOUS_READY.store(true, Ordering::SeqCst);
            }
        });
    }

    /// Install `action` for `signal`, recording the previous disposition.
    ///
    /// A signal that the process inherited as ignored is left ignored. Handling
    /// it would both make an inherited `SIG_IGN` observable and — because
    /// `raise` on an ignored signal returns — let the handler delete the live
    /// index lock and then resume publishing without it, which is exactly the
    /// race the lock exists to prevent. `nohup` and non-job-control background
    /// jobs make this the normal case for `SIGINT`.
    ///
    /// Returns whether waft's handler is now installed for `signal`.
    pub(super) unsafe fn install_unless_ignored(
        signal: libc::c_int,
        action: &libc::sigaction,
        previous: *mut MaybeUninit<libc::sigaction>,
    ) -> bool {
        unsafe {
            // `sigaction` fills the old disposition before the new handler can
            // run, so the saved value is complete by the time it is readable.
            if libc::sigaction(signal, action, (*previous).as_mut_ptr()) != 0 {
                return false;
            }
            let old = (*previous).assume_init_ref();
            if old.sa_sigaction == libc::SIG_IGN {
                libc::sigaction(signal, old, ptr::null_mut());
                return false;
            }
            true
        }
    }
}

#[cfg(not(unix))]
mod interrupt_cleanup {
    use std::path::Path;

    pub(super) fn stage(_path: &Path) {}

    pub(super) fn arm() {}

    pub(super) fn disarm() {}
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
) -> Result<PublishOutcome, String> {
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
    .map_err(|e| format!("{}: failed to copy: {e}", op.rel_path))
}

/// Render a copy report to stderr.
pub fn render_report(report: &CopyReport, quiet: bool) {
    for result in &report.results {
        match &result.outcome {
            CopyOutcome::Failed { message } => {
                // Quiet suppresses routine progress, never actionable errors.
                eprintln!("FAILED: {message}");
            }
            _ if quiet => {}
            CopyOutcome::Copied => match &result.kind {
                CopyResultKind::File => eprintln!("copied: {}", result.rel_path),
            },
            CopyOutcome::Replaced => match &result.kind {
                CopyResultKind::File => eprintln!("replaced: {}", result.rel_path),
            },
            CopyOutcome::PermissionsRepaired => match &result.kind {
                CopyResultKind::File => {
                    eprintln!("repaired permissions: {}", result.rel_path)
                }
            },
        }
    }

    if !quiet {
        // The replace/repair clauses only appear when they happened, so the
        // common run keeps its familiar one-line summary.
        let mut summary = format!("{} copied", report.copied);
        if report.replaced > 0 {
            summary.push_str(&format!(", {} replaced", report.replaced));
        }
        if report.permissions_repaired > 0 {
            summary.push_str(&format!(
                ", {} permissions repaired",
                report.permissions_repaired
            ));
        }
        summary.push_str(&format!(
            ", {} failed, {} skipped, {} up-to-date",
            report.failed, report.skipped, report.up_to_date
        ));
        eprintln!("{summary}");
    }
}

/// Check if the copy report has any failures, returning an appropriate exit status.
pub fn report_has_failures(report: &CopyReport) -> Option<(usize, usize)> {
    if report.failed > 0 {
        Some((report.failed, report.succeeded() + report.failed))
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
        ) -> io::Result<PublishOutcome> {
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
            Ok(match request.expected_destination {
                DestinationExpectation::Missing => PublishOutcome::Created,
                DestinationExpectation::ReplaceExisting(_) => PublishOutcome::Replaced,
                DestinationExpectation::RepairPermissions(_) => PublishOutcome::PermissionsRepaired,
            })
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

    /// Serializes the tests that take an index lock or inspect the
    /// interrupt-cleanup state.
    ///
    /// That state is process-global by necessity — a signal handler cannot
    /// consult thread-local storage — and a real run only ever has one lock
    /// live at a time. Test threads have to reproduce that discipline
    /// explicitly instead of racing each other through it.
    fn one_lock_at_a_time() -> std::sync::MutexGuard<'static, ()> {
        static SERIALIZE: std::sync::Mutex<()> = std::sync::Mutex::new(());
        SERIALIZE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    #[test]
    fn index_lock_retries_past_a_transient_writer() {
        let _serialized = one_lock_at_a_time();
        let temp = tempfile::TempDir::new().unwrap();
        let index = temp.path().join("index");
        let lock = temp.path().join("index.lock");
        // Stand in for an IDE or fsmonitor holding the index briefly.
        fs::write(&lock, b"transient writer\n").unwrap();
        let releaser = {
            let lock = lock.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(20));
                fs::remove_file(&lock).unwrap();
            })
        };

        let Ok(acquired) = GitIndexLock::acquire(&index) else {
            panic!("retry should win the lock");
        };
        releaser.join().unwrap();
        assert!(lock.exists());
        drop(acquired);
        assert!(!lock.exists());
    }

    #[test]
    fn index_lock_gives_a_clear_failure_after_exhausting_retries() {
        let _serialized = one_lock_at_a_time();
        let temp = tempfile::TempDir::new().unwrap();
        let index = temp.path().join("index");
        let lock = temp.path().join("index.lock");
        fs::write(&lock, b"held by concurrent Git\n").unwrap();

        let started = std::time::Instant::now();
        let Err(error) = GitIndexLock::acquire(&index) else {
            panic!("a permanently held lock must not be acquired");
        };

        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        assert!(
            error
                .to_string()
                .contains("destination Git index is locked")
        );
        assert!(
            started.elapsed() >= std::time::Duration::from_millis(140),
            "acquisition should retry for roughly 150ms"
        );
        assert!(
            lock.exists(),
            "waft must never remove another process's lock"
        );
    }

    #[cfg(unix)]
    #[test]
    fn interrupt_cleanup_removes_only_the_armed_lock() {
        let _serialized = one_lock_at_a_time();
        let temp = tempfile::TempDir::new().unwrap();
        let armed = temp.path().join("index.lock");
        fs::write(&armed, b"waft\n").unwrap();

        // The signal handler is a thin wrapper around this; calling it
        // directly is the only way to exercise it without killing the test
        // process.
        interrupt_cleanup::stage(&armed);
        interrupt_cleanup::arm();
        interrupt_cleanup::remove_armed_lock();
        assert!(!armed.exists(), "an interrupt must not leave a stale lock");

        fs::write(&armed, b"someone else\n").unwrap();
        interrupt_cleanup::disarm();
        interrupt_cleanup::remove_armed_lock();
        assert!(
            armed.exists(),
            "cleanup must do nothing once the lock is released"
        );

        // Staging happens before the lock file is created, when the path may
        // still belong to whoever currently holds it.
        interrupt_cleanup::stage(&armed);
        interrupt_cleanup::remove_armed_lock();
        assert!(
            armed.exists(),
            "a staged but unacquired lock is not waft's to remove"
        );
        fs::remove_file(&armed).unwrap();
    }

    /// A signal inherited as ignored must stay ignored. If waft handled it,
    /// `raise` would return and the handler would fall back into the publish
    /// window having already deleted the index lock it is still relying on.
    ///
    /// `SIGUSR2` stands in for `SIGINT`/`SIGTERM` so the test never changes the
    /// dispositions the test process actually runs under.
    #[cfg(unix)]
    #[test]
    fn interrupt_handlers_leave_an_inherited_ignore_alone() {
        extern "C" fn never_called(_signal: libc::c_int) {}

        unsafe fn disposition(signal: libc::c_int) -> libc::sighandler_t {
            unsafe {
                let mut current: std::mem::MaybeUninit<libc::sigaction> =
                    std::mem::MaybeUninit::uninit();
                assert_eq!(
                    libc::sigaction(signal, std::ptr::null(), current.as_mut_ptr()),
                    0
                );
                current.assume_init().sa_sigaction
            }
        }

        unsafe fn set(signal: libc::c_int, handler: libc::sighandler_t) {
            unsafe {
                let mut action: libc::sigaction = std::mem::zeroed();
                action.sa_sigaction = handler;
                libc::sigemptyset(&mut action.sa_mask);
                assert_eq!(libc::sigaction(signal, &action, std::ptr::null_mut()), 0);
            }
        }

        unsafe {
            let mut waft_handler: libc::sigaction = std::mem::zeroed();
            waft_handler.sa_sigaction = never_called as usize;
            libc::sigemptyset(&mut waft_handler.sa_mask);
            let mut previous: std::mem::MaybeUninit<libc::sigaction> =
                std::mem::MaybeUninit::uninit();

            set(libc::SIGUSR2, libc::SIG_IGN);
            let installed = interrupt_cleanup::install_unless_ignored(
                libc::SIGUSR2,
                &waft_handler,
                &raw mut previous,
            );
            assert!(!installed, "an inherited SIG_IGN must not be taken over");
            assert_eq!(
                disposition(libc::SIGUSR2),
                libc::SIG_IGN,
                "the ignore must be put back exactly as it was"
            );

            set(libc::SIGUSR2, libc::SIG_DFL);
            let installed = interrupt_cleanup::install_unless_ignored(
                libc::SIGUSR2,
                &waft_handler,
                &raw mut previous,
            );
            assert!(installed, "a default disposition is waft's to handle");
            assert_eq!(disposition(libc::SIGUSR2), never_called as usize);

            set(libc::SIGUSR2, libc::SIG_DFL);
        }
    }

    #[cfg(unix)]
    #[test]
    fn index_lock_arms_and_disarms_interrupt_cleanup() {
        let _serialized = one_lock_at_a_time();
        let temp = tempfile::TempDir::new().unwrap();
        let index = temp.path().join("index");
        let lock = temp.path().join("index.lock");

        let Ok(held) = GitIndexLock::acquire(&index) else {
            panic!("an unlocked index should be lockable");
        };
        assert!(lock.exists());
        // While held, an interrupt would remove exactly this path.
        interrupt_cleanup::remove_armed_lock();
        assert!(!lock.exists());
        drop(held);

        // After release nothing is armed, so a later interrupt is inert.
        fs::write(&lock, b"a later Git writer\n").unwrap();
        interrupt_cleanup::remove_armed_lock();
        assert!(lock.exists());
        fs::remove_file(&lock).unwrap();
    }

    #[test]
    fn execute_holds_destination_index_lock_through_publication() {
        let _serialized = one_lock_at_a_time();
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
