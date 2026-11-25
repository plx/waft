//! Verified kernel: file comparison, metadata stability, and the small
//! filesystem decisions around anchored publication.
//!
//! Production (`src/fs.rs`) performs every `open`, `read`, `stat`, `mkdirat`
//! and `renameat2`; these kernels decide what the answers mean:
//!
//! - [`compare`]: how two regular files compare (`FileComparison`): equal,
//!   permissions-only, or content differs. Drives `RealFs::compare_files`
//!   chunk by chunk and the default `FileSystem::compare_files`.
//! - [`stability`]: whether observations of a file through its path and an
//!   open handle describe one unchanged file (`same_file_state`,
//!   `open_stable_regular_file`, `ensure_open_file_unchanged`,
//!   `ensure_open_handle_unchanged`).
//! - [`errno`]: which `renameat2` errnos send publication to its fallbacks
//!   (`rename_flag_unsupported`).
//! - [`steps`]: whether a copy attempts a reflink, how many fresh staging
//!   names are tried, and what a `mkdirat` answer means for the anchored
//!   directory walk.
//!
//! Each submodule documents its theorems and trust assumptions.

pub mod compare;
pub mod errno;
pub mod stability;
pub mod steps;
