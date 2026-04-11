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
    /// Value of `core.ignoreCase` in the source repo.
    pub core_ignore_case: bool,
}

// --- Validation types ---

/// Severity of a validation issue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationSeverity {
    /// A problem that prevents proceeding.
    Warning,
    /// A problem that should be noted but does not block.
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
}

impl PlannedEntry {
    /// Get the repo-relative path for this entry.
    pub fn rel_path(&self) -> &RepoRelPath {
        match self {
            PlannedEntry::Copy(op) => &op.rel_path,
            PlannedEntry::NoOp(entry) => &entry.rel_path,
            PlannedEntry::Skip(entry) => &entry.rel_path,
        }
    }
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
    /// Destination file exists and is byte-identical.
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
    /// Destination has an untracked file that differs (requires --overwrite).
    UntrackedConflict,
    /// Destination path is tracked in the destination worktree.
    TrackedConflict,
    /// Destination exists but is not a regular file.
    TypeConflict,
    /// Destination parent path contains a symlink.
    UnsafePath,
    /// Source is not a regular file.
    UnsupportedSourceType,
}

// --- Destination state ---

/// Classification of a destination path's state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DestinationState {
    /// Destination does not exist.
    Missing,
    /// Destination exists and is byte-identical to source.
    UpToDate,
    /// Destination has an untracked file that differs.
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
    /// Whether the copy succeeded.
    pub outcome: CopyOutcome,
}

/// Outcome of a single copy attempt.
#[derive(Debug)]
pub enum CopyOutcome {
    /// File was successfully copied.
    Copied,
    /// Copy failed with an error.
    Failed {
        /// Description of the failure.
        message: String,
    },
}

/// Summary of a copy execution run.
#[derive(Debug)]
pub struct CopyReport {
    /// Results for each file.
    pub results: Vec<CopyResult>,
    /// Number of files successfully copied.
    pub copied: usize,
    /// Number of files that failed.
    pub failed: usize,
    /// Number of files skipped.
    pub skipped: usize,
    /// Number of files that were already up to date.
    pub up_to_date: usize,
}
