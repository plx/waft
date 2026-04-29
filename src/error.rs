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
    #[error("git error: {message}")]
    Git {
        /// Description of what went wrong.
        message: String,
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
    #[error("config error: {message}")]
    Config {
        /// Description of the problem.
        message: String,
    },
}

/// Result type alias for waft operations.
pub type Result<T> = std::result::Result<T, Error>;
