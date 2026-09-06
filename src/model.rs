//! Domain types for planning and reporting.

use std::path::PathBuf;

use crate::path::RepoRelPath;

/// Resolved repository context for all operations.
#[derive(Debug, Clone)]
pub struct RepoContext {
    /// Root of the source (main) worktree.
    pub source_root: PathBuf,
    /// Root of the destination (linked) worktree, if applicable.
    pub dest_root: Option<PathBuf>,
    /// Root of the main worktree.
    pub main_worktree: PathBuf,
    /// All known worktree roots for this repository.
    pub known_worktrees: Vec<PathBuf>,
    /// Whether the source checkout treats differently-cased spellings as the
    /// same path — `core.ignoreCase`, resolved by
    /// [`crate::git::case_folding_applies`].
    ///
    /// Governs repository-path comparisons: tracked-path protection, gitlink
    /// boundaries, and candidate matching. Exclusion pattern matching takes
    /// this only as a lower bound; see
    /// [`crate::policy_filter::effective_case_insensitive`].
    pub core_ignore_case: bool,
}

// --- Validation types ---

/// Severity of a validation issue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationSeverity {
    /// A problem that should be noted but does not block.
    Warning,
    /// A problem that prevents proceeding.
    Error,
}

/// A single validation finding.
#[derive(Debug, Clone)]
pub struct ValidationIssue {
    /// How severe this issue is.
    pub severity: ValidationSeverity,
    /// The file where the issue was found.
    pub file: PathBuf,
    /// Line number within the file, if known.
    pub line: Option<usize>,
    /// Human-readable description of the issue.
    pub message: String,
}

/// Result of validating ignore and worktreeinclude files.
#[derive(Debug, Clone, Default)]
pub struct ValidationReport {
    /// All issues found during validation.
    pub issues: Vec<ValidationIssue>,
}

impl ValidationReport {
    /// Returns `true` if any error-severity issues were found.
    pub fn has_errors(&self) -> bool {
        self.issues
            .iter()
            .any(|i| matches!(i.severity, ValidationSeverity::Error))
    }

    /// Count of error-severity issues.
    pub fn error_count(&self) -> usize {
        self.issues
            .iter()
            .filter(|i| matches!(i.severity, ValidationSeverity::Error))
            .count()
    }
}

// --- Copy planning types ---

/// A complete copy plan, ready for execution or dry-run display.
#[derive(Debug)]
pub struct CopyPlan {
    /// The context this plan was built for.
    pub context: RepoContext,
    /// Validation results collected during planning.
    pub validation: ValidationReport,
    /// Planned entries, sorted by repo-relative path.
    pub entries: Vec<PlannedEntry>,
    /// Whether this is a dry-run plan.
    pub dry_run: bool,
}

/// A single entry in a copy plan.
#[derive(Debug)]
pub enum PlannedEntry {
    /// A file to be copied.
    Copy(CopyOp),
    /// A file that needs no action.
    NoOp(NoOpEntry),
    /// A file that will be skipped.
    Skip(SkipEntry),
    /// A file that could not be planned. Reported and counted as a per-file
    /// failure so one unreadable entry cannot abort an otherwise valid run.
    Failure(FailureEntry),
}

impl PlannedEntry {
    /// Get the repo-relative path for this entry.
    pub fn rel_path(&self) -> &RepoRelPath {
        match self {
            PlannedEntry::Copy(op) => &op.rel_path,
            PlannedEntry::NoOp(entry) => &entry.rel_path,
            PlannedEntry::Skip(entry) => &entry.rel_path,
            PlannedEntry::Failure(entry) => &entry.rel_path,
        }
    }
}

/// A file whose plan could not be produced.
#[derive(Debug)]
pub struct FailureEntry {
    /// Repo-relative path.
    pub rel_path: RepoRelPath,
    /// Human-readable description of why planning failed for this file.
    pub message: String,
}

/// Details for a file that will be copied.
#[derive(Debug)]
pub struct CopyOp {
    /// Repo-relative path of the file.
    pub rel_path: RepoRelPath,
    /// Absolute source path.
    pub src_abs: PathBuf,
    /// Absolute destination path.
    pub dst_abs: PathBuf,
    /// Exact source state observed while planning.
    pub expected_source: FileSnapshot,
    /// Destination state this operation was planned against.
    pub expected_destination: DestinationExpectation,
}

/// Destination state an executor must still observe before publishing a copy.
///
/// Every variant that names an existing destination carries the exact state
/// observed while planning. Execution re-opens the destination and refuses to
/// touch it unless it still matches that state bit for bit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DestinationExpectation {
    /// The destination did not exist during planning and must not be replaced.
    Missing,
    /// The destination existed, was untracked, and its content differed from
    /// the source. Unix `--overwrite` uses atomic exchange where available,
    /// otherwise displacement followed by no-clobber publication.
    ReplaceExisting(FileSnapshot),
    /// The destination existed, was untracked, and its content matched the
    /// source while its permission bits did not. `--overwrite` repairs the
    /// permissions in place without rewriting content.
    RepairPermissions(FileSnapshot),
}

impl DestinationExpectation {
    /// The planning-time snapshot of an existing destination, if any.
    pub(crate) fn existing_snapshot(&self) -> Option<&FileSnapshot> {
        match self {
            DestinationExpectation::Missing => None,
            DestinationExpectation::ReplaceExisting(snapshot)
            | DestinationExpectation::RepairPermissions(snapshot) => Some(snapshot),
        }
    }
}

/// What a successful publication actually did to the destination pathname.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublishOutcome {
    /// A destination that did not exist was created.
    Created,
    /// An existing destination's content was replaced.
    Replaced,
    /// An existing destination's permission bits were repaired; its content
    /// already matched the source and was never rewritten.
    PermissionsRepaired,
}

/// A stable, bounded-memory fingerprint of a regular file.
///
/// This is an internal concurrency guard rather than a cryptographic digest.
/// The fields are intentionally private so callers cannot forge an expected
/// destination state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileSnapshot {
    pub(crate) len: u64,
    pub(crate) content_fingerprint: u64,
    pub(crate) permissions: u32,
    pub(crate) identity: Option<(u64, u64)>,
}

impl FileSnapshot {
    pub(crate) fn new(
        len: u64,
        content_fingerprint: u64,
        permissions: u32,
        identity: Option<(u64, u64)>,
    ) -> Self {
        Self {
            len,
            content_fingerprint,
            permissions,
            identity,
        }
    }

    /// Whether two snapshots describe the same bytes, ignoring permissions and
    /// identity.
    ///
    /// This is what makes "content equal, permissions differ" checkable at
    /// publication time: planning proves byte equality with a full comparison,
    /// but the source and destination snapshots are separate reads. Carrying
    /// the equality forward as comparable data lets the publication path
    /// re-establish it instead of trusting a classification made earlier.
    pub(crate) fn content_matches(&self, other: &FileSnapshot) -> bool {
        self.len == other.len && self.content_fingerprint == other.content_fingerprint
    }
}

/// A file that needs no action.
#[derive(Debug)]
pub struct NoOpEntry {
    /// Repo-relative path.
    pub rel_path: RepoRelPath,
    /// Why no action is needed.
    pub reason: NoOpReason,
}

/// Why a file needs no action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoOpReason {
    /// Destination content and relevant permissions match the source.
    UpToDate,
}

/// A file that will be skipped.
#[derive(Debug)]
pub struct SkipEntry {
    /// Repo-relative path.
    pub rel_path: RepoRelPath,
    /// Why the file is being skipped.
    pub reason: SkipReason,
}

/// Why a file is being skipped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipReason {
    /// Destination has an untracked file that differs and must be preserved.
    UntrackedConflict,
    /// Destination content already matches the source but its permission bits
    /// do not. This is the state left behind by pre-`0.1` waft builds, which
    /// always published mode `0600`.
    PermissionsDiffer,
    /// Destination path is tracked in the destination worktree.
    TrackedConflict,
    /// Destination exists but is not a regular file.
    TypeConflict,
    /// Destination parent path contains a symlink.
    UnsafePath,
    /// Source is not a regular file.
    UnsupportedSourceType,
}

impl SkipReason {
    /// A short human-readable phrase naming this reason, including the remedy
    /// where one exists.
    ///
    /// The conflict itself reads identically everywhere; only the `--overwrite`
    /// remedy is withheld on platforms where it cannot act on an existing
    /// destination, so no output advertises a fix that would fail.
    pub fn describe(&self) -> &'static str {
        match self {
            SkipReason::UntrackedConflict => {
                if crate::fs::overwrite_supported() {
                    "untracked conflict; --overwrite replaces the destination"
                } else {
                    "untracked conflict"
                }
            }
            SkipReason::PermissionsDiffer => {
                if crate::fs::overwrite_supported() {
                    "content equal, permissions differ; --overwrite repairs the permissions"
                } else {
                    "content equal, permissions differ"
                }
            }
            SkipReason::TrackedConflict => "tracked conflict",
            SkipReason::TypeConflict => "type conflict",
            SkipReason::UnsafePath => "unsafe path",
            SkipReason::UnsupportedSourceType => "unsupported source type",
        }
    }
}

impl std::fmt::Display for SkipReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.describe())
    }
}

// --- Destination state ---

/// Classification of a destination path's state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DestinationState {
    /// Destination does not exist.
    Missing,
    /// Destination content and relevant permissions match the source.
    UpToDate,
    /// Destination content matches the source but its permission bits differ.
    /// Distinct from [`DestinationState::UntrackedConflict`] because
    /// `--overwrite` repairs it without rewriting any content.
    PermissionsDiffer,
    /// Destination has an untracked file whose content differs.
    UntrackedConflict,
    /// Destination path is tracked.
    TrackedConflict,
    /// Destination exists but is not a regular file.
    TypeConflict,
    /// Destination parent contains a symlink.
    UnsafePath,
}

// --- Git ignore decision types ---

/// Result of checking a path's Git ignore status.
#[derive(Debug, Clone)]
pub enum GitIgnoreStatus {
    /// The path is tracked in the Git index.
    Tracked,
    /// The path is ignored by a Git ignore rule.
    Ignored {
        /// The ignore file containing the matching rule.
        source_file: PathBuf,
        /// Line number of the matching rule.
        line: usize,
        /// The pattern text that matched.
        pattern: String,
    },
    /// The path is not ignored by any rule.
    NotIgnored,
}

// --- Worktreeinclude decision types ---

/// Result of evaluating a path against `.worktreeinclude` files.
#[derive(Debug, Clone)]
pub enum WorktreeincludeStatus {
    /// The path is selected by a `.worktreeinclude` pattern.
    Included {
        /// The `.worktreeinclude` file containing the matching rule.
        file: PathBuf,
        /// Line number of the matching rule.
        line: usize,
        /// The pattern text that matched.
        pattern: String,
    },
    /// The path was selected but then negated.
    ExcludedByNegation {
        /// The `.worktreeinclude` file containing the negation rule.
        file: PathBuf,
        /// Line number of the negation rule.
        line: usize,
        /// The negation pattern text.
        pattern: String,
    },
    /// No `.worktreeinclude` pattern matched the path.
    NoMatch,
}

// --- Source kind ---

/// What kind of filesystem entity the source path is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    /// A regular file.
    File,
    /// A directory.
    Directory,
    /// A symbolic link.
    Symlink,
    /// Something else (device, FIFO, socket, etc.).
    Other,
    /// The source does not exist.
    Missing,
}

// --- Info report ---

/// Detailed status report for a single path.
#[derive(Debug)]
pub struct InfoReport {
    /// Normalized repo-relative path.
    pub rel_path: RepoRelPath,
    /// Whether the source path exists.
    pub source_exists: bool,
    /// What kind of entity the source is.
    pub source_kind: SourceKind,
    /// Whether the path is tracked in Git.
    pub tracked: bool,
    /// Git ignore status.
    pub gitignore: GitIgnoreStatus,
    /// Worktreeinclude status.
    pub worktreeinclude: WorktreeincludeStatus,
    /// Whether this file is eligible to copy.
    pub eligible_to_copy: bool,
    /// Destination state, if a destination is known.
    pub destination: Option<DestinationState>,
}

// --- Copy execution result ---

/// Result of executing a single copy operation.
#[derive(Debug)]
pub struct CopyResult {
    /// The operation that was executed.
    pub rel_path: RepoRelPath,
    /// Result display/counting kind.
    pub kind: CopyResultKind,
    /// Whether the copy succeeded.
    pub outcome: CopyOutcome,
}

/// Shape of the copy result for reporting.
#[derive(Debug)]
pub enum CopyResultKind {
    /// A single-file copy result.
    File,
}

/// Outcome of a single copy attempt.
#[derive(Debug)]
pub enum CopyOutcome {
    /// File was successfully published to a destination that did not exist.
    Copied,
    /// An existing untracked destination was atomically replaced.
    Replaced,
    /// An existing untracked destination's permission bits were repaired; its
    /// content already matched the source.
    PermissionsRepaired,
    /// Copy failed with an error.
    Failed {
        /// Description of the failure.
        message: String,
    },
}

impl From<PublishOutcome> for CopyOutcome {
    fn from(outcome: PublishOutcome) -> Self {
        match outcome {
            PublishOutcome::Created => CopyOutcome::Copied,
            PublishOutcome::Replaced => CopyOutcome::Replaced,
            PublishOutcome::PermissionsRepaired => CopyOutcome::PermissionsRepaired,
        }
    }
}

/// Summary of a copy execution run.
#[derive(Debug)]
pub struct CopyReport {
    /// Results for each file.
    pub results: Vec<CopyResult>,
    /// Number of files successfully created at a missing destination.
    pub copied: usize,
    /// Number of existing destinations replaced under `--overwrite`.
    pub replaced: usize,
    /// Number of existing destinations whose permissions were repaired.
    pub permissions_repaired: usize,
    /// Number of files that failed.
    pub failed: usize,
    /// Number of files skipped.
    pub skipped: usize,
    /// Number of files that were already up to date.
    pub up_to_date: usize,
}

impl CopyReport {
    /// Total number of destinations this run successfully published or
    /// repaired.
    pub fn succeeded(&self) -> usize {
        self.copied + self.replaced + self.permissions_repaired
    }
}
