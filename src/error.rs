//! Error types for waft.

use std::path::PathBuf;

/// Top-level error type for waft operations.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// An I/O error occurred.
    #[error("{context}: {source}")]
    Io {
        /// What was being done when the error occurred.
        context: String,
        /// The underlying I/O error.
        source: std::io::Error,
    },

    /// A Git command failed.
    ///
    /// The prefix deliberately omits the word "error": callers print these
    /// as `error: {self}`, and `error: git error: …` reads like a bug report
    /// rather than a message.
    #[error("git: {message}")]
    Git {
        /// Description of what went wrong.
        message: String,
    },

    /// No Git repository contains the path waft was asked to work from.
    ///
    /// This is the single most common way to invoke waft wrongly, so it gets
    /// a plain sentence instead of the backend's discovery diagnostics. Those
    /// stay reachable through [`std::error::Error::source`] and are printed
    /// under `-v`/`--verbose`.
    #[error("not inside a Git repository (searched from {})", searched_from.display())]
    NotAGitRepository {
        /// Path discovery started from.
        searched_from: PathBuf,
        /// The backend's own account of the discovery failure.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync + 'static>,
    },

    /// Path validation failed.
    #[error("invalid path: {message}")]
    InvalidPath {
        /// Description of the path problem.
        message: String,
    },

    /// Context resolution failed.
    #[error("{message}")]
    Context {
        /// Description of the context problem.
        message: String,
    },

    /// Validation found errors that prevent proceeding.
    #[error("validation failed with {error_count} error(s)")]
    Validation {
        /// Number of errors found.
        error_count: usize,
    },

    /// A feature is not yet implemented.
    #[error("{command} is not yet implemented")]
    NotImplemented {
        /// The command that is not yet implemented.
        command: String,
    },

    /// Copy execution had failures.
    #[error("copy failed: {failed} of {total} file(s) failed")]
    CopyFailed {
        /// Number of files that failed.
        failed: usize,
        /// Total number of files attempted.
        total: usize,
    },

    /// The source and destination are the same.
    #[error("source and destination are the same: {path}")]
    SameSourceAndDest {
        /// The path that is the same.
        path: PathBuf,
    },

    /// The destination is not a worktree of the source.
    #[error("destination {dest} is not a linked worktree of {src}")]
    NotInWorktreeFamily {
        /// Source worktree path.
        src: PathBuf,
        /// Destination worktree path.
        dest: PathBuf,
    },

    /// Configuration parsing or validation failed.
    ///
    /// Prefixed like [`Error::Git`], for the same reason.
    #[error("config: {message}")]
    Config {
        /// Description of the problem.
        message: String,
    },
}

/// Result type alias for waft operations.
pub type Result<T> = std::result::Result<T, Error>;
