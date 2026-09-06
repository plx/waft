//! Git backend trait and its two implementations.
//!
//! All Git interactions go through the [`GitBackend`] trait, which lets the
//! planner and other modules be tested without real Git repos. Two
//! interchangeable implementations live here:
//!
//! - [`GitGix`] (default): in-process via the `gix` crate.
//! - [`GitCli`]: shells out to the `git` binary. Selected by setting
//!   `WAFT_GIT_BACKEND=cli`.
//!
//! Backend parity tests in `tests/backend_parity.rs` pin both implementations
//! to the same observable behavior.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

use unicode_casefold::UnicodeCaseFold;
use unicode_normalization::UnicodeNormalization;

use crate::config::{SymlinkPolicy, WorktreeincludeSemantics};
use crate::error::{Error, Result};
use crate::path::RepoRelPath;

/// Record from `git worktree list --porcelain -z`.
#[derive(Debug, Clone)]
pub struct WorktreeRecord {
    /// Absolute path of the worktree.
    pub path: PathBuf,
    /// Whether this is the main worktree (listed first by Git).
    pub is_main: bool,
    /// Whether the worktree is bare.
    pub is_bare: bool,
}

/// Record from `git check-ignore --stdin -z -v -n`.
#[derive(Debug, Clone)]
pub struct IgnoreCheckRecord {
    /// The path that was checked.
    pub path: RepoRelPath,
    /// Whether the effective match excludes the path.
    pub ignored: bool,
    /// If the path matched an ignore rule, details about the match. This may
    /// describe a negated rule even when `ignored` is false.
    pub match_info: Option<IgnoreMatchInfo>,
}

/// Details about an ignore rule match.
#[derive(Debug, Clone)]
pub struct IgnoreMatchInfo {
    /// The file containing the matching rule.
    pub source_file: PathBuf,
    /// Line number of the matching rule (1-based).
    pub line: usize,
    /// The pattern text.
    pub pattern: String,
}

/// Abstraction over Git CLI operations.
pub trait GitBackend {
    /// Resolve the worktree root for a path.
    fn show_toplevel(&self, path: &Path) -> Result<PathBuf>;

    /// List all worktrees for the repo at `source_root`.
    fn list_worktrees(&self, source_root: &Path) -> Result<Vec<WorktreeRecord>>;

    /// Return the set of tracked paths (from the index) among the given paths.
    fn tracked_paths(
        &self,
        source_root: &Path,
        paths: &[RepoRelPath],
    ) -> Result<HashSet<RepoRelPath>>;

    /// Return the worktree-specific index path when the backend can expose it.
    ///
    /// The executor holds the corresponding `.lock` file across its final
    /// tracked-state check and publication so normal Git writers cannot change
    /// trackedness in between. Test and specialized backends may return `None`.
    fn index_path(&self, _source_root: &Path) -> Result<Option<PathBuf>> {
        Ok(None)
    }

    /// Return registered submodule paths from the index (mode 160000 gitlinks).
    fn gitlinks(&self, source_root: &Path) -> Result<HashSet<String>>;

    /// Batch-check ignore status for the given paths.
    fn check_ignore(
        &self,
        source_root: &Path,
        paths: &[RepoRelPath],
    ) -> Result<Vec<IgnoreCheckRecord>>;

    /// List files that match `.worktreeinclude` patterns (candidates for copy).
    ///
    /// `symlink_policy` decides whether symlinked `.worktreeinclude` files
    /// are followed (`Follow`/`Error`) or ignored (`Ignore`). `semantics`
    /// selects the matcher engine.
    fn list_worktreeinclude_candidates(
        &self,
        source_root: &Path,
        semantics: WorktreeincludeSemantics,
        symlink_policy: SymlinkPolicy,
    ) -> Result<Vec<RepoRelPath>>;

    /// List all untracked files under `source_root` that are git-ignored.
    ///
    /// Used by the `when_missing = all-ignored` mode as the candidate set when
    /// no `.worktreeinclude` file exists anywhere in the repo.
    fn list_ignored_untracked(&self, source_root: &Path) -> Result<Vec<RepoRelPath>>;

    /// Return whether any `.worktreeinclude` file exists anywhere in the repo
    /// (excluding nested git checkouts and registered submodules).
    ///
    /// Used to gate `when_missing` behavior. `symlink_policy = Ignore` causes
    /// symlinked `.worktreeinclude` files to NOT count toward existence
    /// (consistent with their being treated as absent during selection).
    fn worktreeinclude_exists_anywhere(
        &self,
        source_root: &Path,
        symlink_policy: SymlinkPolicy,
    ) -> Result<bool>;

    /// Read a boolean Git config value.
    fn read_bool_config(&self, source_root: &Path, key: &str) -> Result<bool>;

    /// Read a Git config value as a string. Returns `None` if the key is unset.
    fn read_config(&self, source_root: &Path, key: &str) -> Result<Option<String>>;

    /// Whether this checkout treats differently-cased spellings of a path as
    /// naming the same file.
    ///
    /// This single answer drives tracked-path protection and repository
    /// boundary comparisons, so the two cannot disagree. The real backends
    /// override this to distinguish an explicitly configured
    /// `core.ignoreCase = false` from an absent key; see
    /// [`case_folding_applies`].
    ///
    /// The real backends resolve this **once per repository per backend
    /// instance** and reuse the answer for the rest of that instance's life.
    /// Re-reading it would put a config lookup — a subprocess, for
    /// [`GitCli`] — on the per-file path the executor takes while holding the
    /// destination index lock, and the answer is a property of the checkout's
    /// filesystem that Git records at creation time. A `core.ignoreCase` edit
    /// made after the first query is therefore not observed by that instance;
    /// construct a new backend to pick it up.
    ///
    /// Note that this governs *repository path* comparisons. Exclusion
    /// pattern matching keeps its own deliberately more conservative rule; see
    /// [`crate::policy_filter::effective_case_insensitive`].
    fn checkout_folds_case(&self, source_root: &Path) -> Result<bool> {
        self.read_bool_config(source_root, "core.ignoreCase")
    }

    /// Whether this backend reads Git's ambient default global excludes file.
    ///
    /// Real backends return true. In-memory test/specialized backends default
    /// to false so validation does not unexpectedly consult the host account.
    fn reads_default_global_excludes(&self) -> bool {
        false
    }
}

/// Which [`GitBackend`] implementation `WAFT_GIT_BACKEND` selects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GitBackendKind {
    /// In-process `gix` backend; the default.
    Gix,
    /// `git` subprocess backend.
    Cli,
}

/// Valid `WAFT_GIT_BACKEND` values, in the order they are reported to users.
const GIT_BACKEND_VALUES: &str = "\"gix\", \"cli\"";

/// Parse a `WAFT_GIT_BACKEND` value.
///
/// Surrounding whitespace is trimmed and the name is matched
/// ASCII-case-insensitively, so `cli`, `CLI`, and `" cli "` all select the
/// subprocess backend. Anything else is rejected: silently falling back to
/// the default backend on a typo would change which implementation enforces
/// waft's tracked-path and boundary checks without telling anyone.
fn parse_git_backend_kind(value: &str) -> Result<GitBackendKind> {
    match value.trim().to_ascii_lowercase().as_str() {
        "gix" => Ok(GitBackendKind::Gix),
        "cli" => Ok(GitBackendKind::Cli),
        other => Err(Error::Config {
            message: format!(
                "WAFT_GIT_BACKEND=\"{other}\" is not a known Git backend; \
                 valid values are {GIT_BACKEND_VALUES}"
            ),
        }),
    }
}

/// Create the configured Git backend.
///
/// Uses the in-process `gix` backend when `WAFT_GIT_BACKEND` is unset. See
/// [`parse_git_backend_kind`] for the accepted values; an unrecognized value
/// is a hard error.
pub fn default_git_backend() -> Result<Box<dyn GitBackend>> {
    let kind = match std::env::var("WAFT_GIT_BACKEND") {
        Ok(value) => parse_git_backend_kind(&value)?,
        Err(std::env::VarError::NotPresent) => GitBackendKind::Gix,
        Err(std::env::VarError::NotUnicode(value)) => {
            return Err(Error::Config {
                message: format!(
                    "WAFT_GIT_BACKEND={value:?} is not valid Unicode; \
                     valid values are {GIT_BACKEND_VALUES}"
                ),
            });
        }
    };
    Ok(match kind {
        GitBackendKind::Gix => Box::new(GitGix::new()) as Box<dyn GitBackend>,
        GitBackendKind::Cli => Box::new(GitCli::new()),
    })
}

/// Git backend that shells out to the `git` CLI.
#[derive(Debug, Default)]
pub struct GitCli {
    cache: RepoStateCache,
}

impl GitCli {
    /// Create a new `GitCli` backend.
    pub fn new() -> Self {
        Self::default()
    }

    /// Return this repository's index-derived state, reusing the cached copy
    /// while the index file is unchanged.
    fn index_state(&self, source_root: &Path) -> Result<Arc<RepoIndexState>> {
        let index_path = self
            .cache
            .index_path(source_root, || self.resolve_index_path(source_root))?;
        let ignore_case = self.cached_checkout_folds_case(source_root)?;
        self.cache.index_state(source_root, &index_path, || {
            // One `ls-files -s` yields both the tracked names and the
            // gitlink (mode 160000) entries, so a run needs a single
            // subprocess for all tracked-state questions about this index.
            let output = self.run_git(source_root, &["ls-files", "-s", "-z", "--full-name"])?;
            RepoIndexState::from_ls_files_stage_output(&output, ignore_case)
        })
    }

    /// Resolve `core.ignoreCase` once per repository; see
    /// [`GitBackend::checkout_folds_case`] for the contract.
    ///
    /// Kept out of the index-fingerprinted snapshot deliberately: the config
    /// is not the index, so an index write must not re-run the config read,
    /// and a config edit must not appear to be observed just because the index
    /// happened to move.
    fn cached_checkout_folds_case(&self, source_root: &Path) -> Result<bool> {
        self.cache
            .checkout_folds_case(source_root, || self.read_checkout_folds_case(source_root))
    }

    fn resolve_index_path(&self, source_root: &Path) -> Result<PathBuf> {
        let output = self.run_git(source_root, &["rev-parse", "--git-path", "index"])?;
        let path_bytes = trim_git_line_ending(&output);
        let raw = path_buf_from_git_bytes(path_bytes, "Git index path")?;
        Ok(if raw.is_absolute() {
            raw
        } else {
            source_root.join(raw)
        })
    }

    fn read_checkout_folds_case(&self, source_root: &Path) -> Result<bool> {
        // `git config --bool` exits non-zero when the key is unset, which
        // `run_git` reports as an error; treat that as "no recorded answer".
        let configured = self
            .run_git(
                source_root,
                &["config", "--bool", "--get", "core.ignoreCase"],
            )
            .ok()
            .and_then(|bytes| match String::from_utf8_lossy(&bytes).trim() {
                "true" => Some(true),
                "false" => Some(false),
                _ => None,
            });
        Ok(case_folding_applies(configured))
    }

    fn run_git(&self, root: &Path, args: &[&str]) -> Result<Vec<u8>> {
        let output = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .map_err(|e| Error::Io {
                context: format!("running git {}", args.join(" ")),
                source: e,
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::Git {
                message: format!("git {} failed: {}", args.join(" "), stderr.trim()),
            });
        }

        Ok(output.stdout)
    }

    fn run_git_with_stdin(&self, root: &Path, args: &[&str], stdin_data: &[u8]) -> Result<Vec<u8>> {
        use std::io::Write;
        use std::process::Stdio;

        let mut child = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| Error::Io {
                context: format!("spawning git {}", args.join(" ")),
                source: e,
            })?;

        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(stdin_data).map_err(|e| Error::Io {
                context: "writing to git stdin".to_string(),
                source: e,
            })?;
        }

        let output = child.wait_with_output().map_err(|e| Error::Io {
            context: format!("waiting for git {}", args.join(" ")),
            source: e,
        })?;

        // check-ignore exits 1 when no paths match, which is not an error for us
        if !output.status.success() && output.status.code() != Some(1) {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::Git {
                message: format!("git {} failed: {}", args.join(" "), stderr.trim()),
            });
        }

        Ok(output.stdout)
    }
}

/// Git backend implemented with the `gix` crate.
///
/// During migration, operations not yet ported may still delegate to [`GitCli`].
#[derive(Debug, Default)]
pub struct GitGix {
    cache: RepoStateCache,
}

impl GitGix {
    /// Create a new `GitGix` backend.
    pub fn new() -> Self {
        Self::default()
    }

    /// Return this repository's index-derived state, reusing the cached copy
    /// while the index file is unchanged.
    fn index_state(&self, source_root: &Path) -> Result<Arc<RepoIndexState>> {
        let index_path = self.cache.index_path(source_root, || {
            Ok(self.discover_repo(source_root)?.index_path())
        })?;
        let ignore_case = self.cached_checkout_folds_case(source_root)?;
        self.cache.index_state(source_root, &index_path, || {
            let repo = self.discover_repo(source_root)?;
            let index = repo.index_or_empty().map_err(|e| Error::Git {
                message: format!(
                    "gix failed to read index for {}: {e}",
                    source_root.display()
                ),
            })?;
            RepoIndexState::from_gix_index(&index, ignore_case)
        })
    }

    /// Resolve `core.ignoreCase` once per repository; see
    /// [`GitBackend::checkout_folds_case`] for the contract.
    fn cached_checkout_folds_case(&self, source_root: &Path) -> Result<bool> {
        self.cache.checkout_folds_case(source_root, || {
            let repo = self.discover_repo(source_root)?;
            Ok(case_folding_applies(
                repo.config_snapshot().boolean("core.ignoreCase"),
            ))
        })
    }

    fn discover_repo(&self, path: &Path) -> Result<gix::Repository> {
        gix::discover(path).map_err(|error| discovery_error(path, error))
    }

    fn normalize_ignore_source(path: &Path, source_root: &Path) -> PathBuf {
        path.strip_prefix(source_root)
            .map(Path::to_path_buf)
            .unwrap_or_else(|_| path.to_path_buf())
    }
}

/// A backend's own account of a discovery failure.
///
/// Only ever carried as the source of [`Error::NotAGitRepository`], where the
/// user-facing sentence is deliberately plain and the backend's wording is
/// what `--verbose` reveals.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
struct DiscoveryDetail(String);

/// Classify a `gix` discovery failure.
///
/// "Nothing here is a repository" is the one discovery failure a user can act
/// on without knowing anything about waft's internals, so it becomes a
/// first-class error with a plain message. Every other failure — an
/// unreadable directory, an untrusted repository — keeps the backend's own
/// description, because those are genuinely about `gix` and not about the
/// user's working directory.
fn discovery_error(path: &Path, error: gix::discover::Error) -> Error {
    let missing = matches!(
        &error,
        gix::discover::Error::Discover(
            gix::discover::upwards::Error::NoGitRepository { .. }
                | gix::discover::upwards::Error::NoGitRepositoryWithinCeiling { .. }
                | gix::discover::upwards::Error::NoGitRepositoryWithinFs { .. }
        )
    );
    if missing {
        return Error::NotAGitRepository {
            searched_from: path.to_path_buf(),
            source: Box::new(error),
        };
    }
    Error::Git {
        message: format!(
            "gix failed to discover repository from {}: {error}",
            path.display()
        ),
    }
}

/// Classify a `git rev-parse` failure the same way [`discovery_error`] does.
///
/// The CLI reports a discovery miss only in prose, so the phrase is the whole
/// signal. Anything else — a broken `git`, a permissions problem — is passed
/// through untouched rather than mislabeled as "no repository here".
fn cli_discovery_error(path: &Path, error: Error) -> Error {
    let Error::Git { message } = &error else {
        return error;
    };
    if !message.to_lowercase().contains("not a git repository") {
        return error;
    }
    Error::NotAGitRepository {
        searched_from: path.to_path_buf(),
        source: Box::new(DiscoveryDetail(message.clone())),
    }
}

/// Canonicalize a repo-root path and strip the Windows `\\?\` verbatim prefix
/// so both backends produce paths in the same form (critical for
/// `strip_prefix` and display parity between backends).
fn normalize_repo_path(path: &Path) -> PathBuf {
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    crate::path::without_windows_verbatim_prefix(&canonical)
}

/// Convert Git's raw path bytes without changing their spelling.
///
/// Unix paths are byte strings, so preserve every byte. Other supported
/// platforms require Unicode paths; fail closed instead of replacing invalid
/// bytes and potentially aliasing two distinct names.
fn path_buf_from_git_bytes(bytes: &[u8], context: &str) -> Result<PathBuf> {
    #[cfg(unix)]
    {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let _ = context;
        Ok(PathBuf::from(OsStr::from_bytes(bytes)))
    }

    #[cfg(not(unix))]
    {
        let path = std::str::from_utf8(bytes).map_err(|error| Error::InvalidPath {
            message: format!("{context} is not valid UTF-8: {error}"),
        })?;
        Ok(PathBuf::from(path))
    }
}

fn trim_git_line_ending(mut bytes: &[u8]) -> &[u8] {
    if let Some(trimmed) = bytes.strip_suffix(b"\n") {
        bytes = trimmed;
    }
    if let Some(trimmed) = bytes.strip_suffix(b"\r") {
        bytes = trimmed;
    }
    bytes
}

fn gitlinks_from_gix_index(index: &gix::index::State) -> Result<HashSet<String>> {
    index
        .entries()
        .iter()
        .filter(|entry| entry.mode == gix::index::entry::Mode::COMMIT)
        .map(|entry| {
            RepoRelPath::from_git_bytes(entry.path(index).as_ref())
                .map(|path| path.as_str().to_string())
        })
        .collect()
}

impl GitBackend for GitCli {
    fn show_toplevel(&self, path: &Path) -> Result<PathBuf> {
        let output = self
            .run_git(path, &["rev-parse", "--show-toplevel"])
            .map_err(|error| cli_discovery_error(path, error))?;
        let path_bytes = trim_git_line_ending(&output);
        let raw = path_buf_from_git_bytes(path_bytes, "repository root")?;
        Ok(normalize_repo_path(&raw))
    }

    fn list_worktrees(&self, source_root: &Path) -> Result<Vec<WorktreeRecord>> {
        let output = self.run_git(source_root, &["worktree", "list", "--porcelain", "-z"])?;
        let mut records = parse_worktree_list(&output)?;
        // Git echoes each worktree's registered path verbatim, which keeps any
        // symlinked ancestor in the spelling. `show_toplevel` and the gix
        // backend both hand back canonical roots, so normalize here too:
        // otherwise main-vs-linked classification and the anchored copy engine
        // compare a symlinked path against a canonical one.
        for record in &mut records {
            record.path = normalize_repo_path(&record.path);
        }
        Ok(records)
    }

    fn tracked_paths(
        &self,
        source_root: &Path,
        paths: &[RepoRelPath],
    ) -> Result<HashSet<RepoRelPath>> {
        if paths.is_empty() {
            return Ok(HashSet::new());
        }

        // Do not pass the candidates as pathspecs here. Git's pathspec lookup
        // can remain case-sensitive even when core.ignoreCase is true, which
        // would let `SECRET.env` be treated as untracked when the index
        // contains `secret.env`. Enumerate the index once and return the
        // caller's spelling for every matching query.
        let state = self.index_state(source_root)?;
        Ok(state.tracked.select(source_root, paths))
    }

    fn index_path(&self, source_root: &Path) -> Result<Option<PathBuf>> {
        self.cache
            .index_path(source_root, || self.resolve_index_path(source_root))
            .map(Some)
    }

    fn gitlinks(&self, source_root: &Path) -> Result<HashSet<String>> {
        Ok(self.index_state(source_root)?.gitlinks.clone())
    }

    fn check_ignore(
        &self,
        source_root: &Path,
        paths: &[RepoRelPath],
    ) -> Result<Vec<IgnoreCheckRecord>> {
        if paths.is_empty() {
            return Ok(Vec::new());
        }

        // Build NUL-delimited stdin
        let mut stdin_data = Vec::new();
        for path in paths {
            stdin_data.extend_from_slice(path.as_str().as_bytes());
            stdin_data.push(0);
        }

        let output = self.run_git_with_stdin(
            source_root,
            &["check-ignore", "--stdin", "-z", "-v", "-n"],
            &stdin_data,
        )?;

        parse_check_ignore_output(&output)
    }

    fn list_worktreeinclude_candidates(
        &self,
        source_root: &Path,
        semantics: WorktreeincludeSemantics,
        symlink_policy: SymlinkPolicy,
    ) -> Result<Vec<RepoRelPath>> {
        // `git ls-files --exclude-per-directory` only implements Git's
        // nested rule semantics. Claude is intentionally root-only and Wt
        // has its own subtractive selection algorithm, so every profile uses
        // the same semantics engine as the in-process backend.
        cli_list_candidates_with_engine(self, source_root, semantics, symlink_policy)
    }

    fn list_ignored_untracked(&self, source_root: &Path) -> Result<Vec<RepoRelPath>> {
        let output = self.run_git(
            source_root,
            &[
                "ls-files",
                "--others",
                "--ignored",
                "--exclude-standard",
                "--full-name",
                "-z",
            ],
        )?;

        let mut result = Vec::new();
        for entry in output.split(|&b| b == 0) {
            if !entry.is_empty() {
                result.push(RepoRelPath::from_git_bytes(entry)?);
            }
        }
        Ok(result)
    }

    fn worktreeinclude_exists_anywhere(
        &self,
        source_root: &Path,
        symlink_policy: SymlinkPolicy,
    ) -> Result<bool> {
        // Use a filesystem walk that mirrors `is_nested_git_boundary` rules,
        // querying the index for gitlinks via `git ls-files -s`. This keeps
        // both backends in agreement on which subtrees count as "in the
        // repo" for purposes of this check.
        let state = self.index_state(source_root)?;
        Ok(walk_for_first_worktreeinclude(
            source_root,
            &state.gitlinks,
            state.ignore_case,
            symlink_policy,
        ))
    }

    fn checkout_folds_case(&self, source_root: &Path) -> Result<bool> {
        self.cached_checkout_folds_case(source_root)
    }

    fn read_bool_config(&self, source_root: &Path, key: &str) -> Result<bool> {
        let output = self.run_git(source_root, &["config", "--bool", key]);
        match output {
            Ok(bytes) => {
                let s = String::from_utf8_lossy(&bytes);
                Ok(s.trim() == "true")
            }
            Err(_) => {
                // Config key not set defaults to false
                Ok(false)
            }
        }
    }

    fn read_config(&self, source_root: &Path, key: &str) -> Result<Option<String>> {
        let output = self.run_git(source_root, &["config", key]);
        match output {
            Ok(bytes) => {
                let s = String::from_utf8_lossy(&bytes);
                let trimmed = s.trim();
                if trimmed.is_empty() {
                    Ok(None)
                } else {
                    Ok(Some(trimmed.to_string()))
                }
            }
            Err(_) => {
                // Config key not set
                Ok(None)
            }
        }
    }

    fn reads_default_global_excludes(&self) -> bool {
        true
    }
}

impl GitBackend for GitGix {
    fn show_toplevel(&self, path: &Path) -> Result<PathBuf> {
        let repo = self.discover_repo(path)?;
        let workdir = repo.workdir().ok_or_else(|| Error::Git {
            message: format!(
                "cannot resolve worktree toplevel for bare repository at {}",
                repo.path().display()
            ),
        })?;
        Ok(normalize_repo_path(workdir))
    }

    fn list_worktrees(&self, source_root: &Path) -> Result<Vec<WorktreeRecord>> {
        let repo = self.discover_repo(source_root)?;
        let main_repo = repo.main_repo().map_err(|e| Error::Git {
            message: format!(
                "gix failed to open main repository for {}: {e}",
                source_root.display()
            ),
        })?;

        let main_path = main_repo
            .workdir()
            .map(normalize_repo_path)
            .unwrap_or_else(|| normalize_repo_path(main_repo.path()));

        let mut records = vec![WorktreeRecord {
            path: main_path.clone(),
            is_main: true,
            is_bare: main_repo.is_bare(),
        }];

        let linked = main_repo.worktrees().map_err(|e| Error::Io {
            context: format!(
                "listing linked worktrees in {}",
                main_repo.common_dir().display()
            ),
            source: e,
        })?;

        for proxy in linked {
            let path = proxy.base().map_err(|e| Error::Io {
                context: format!("reading linked worktree at {}", proxy.git_dir().display()),
                source: e,
            })?;
            let path = normalize_repo_path(&path);
            if path == main_path {
                continue;
            }
            records.push(WorktreeRecord {
                path,
                is_main: false,
                is_bare: false,
            });
        }

        Ok(records)
    }

    fn tracked_paths(
        &self,
        source_root: &Path,
        paths: &[RepoRelPath],
    ) -> Result<HashSet<RepoRelPath>> {
        if paths.is_empty() {
            return Ok(HashSet::new());
        }

        let state = self.index_state(source_root)?;
        Ok(state.tracked.select(source_root, paths))
    }

    fn index_path(&self, source_root: &Path) -> Result<Option<PathBuf>> {
        self.cache
            .index_path(source_root, || {
                Ok(self.discover_repo(source_root)?.index_path())
            })
            .map(Some)
    }

    fn gitlinks(&self, source_root: &Path) -> Result<HashSet<String>> {
        Ok(self.index_state(source_root)?.gitlinks.clone())
    }

    fn check_ignore(
        &self,
        source_root: &Path,
        paths: &[RepoRelPath],
    ) -> Result<Vec<IgnoreCheckRecord>> {
        if paths.is_empty() {
            return Ok(Vec::new());
        }

        let repo = self.discover_repo(source_root)?;
        let worktree = repo.worktree().ok_or_else(|| Error::Git {
            message: format!(
                "cannot run ignore checks for bare repository at {}",
                repo.path().display()
            ),
        })?;
        let mut excludes = worktree.excludes(None).map_err(|e| Error::Git {
            message: format!(
                "gix failed to initialize exclude stack for {}: {e}",
                source_root.display()
            ),
        })?;
        let tracked = self.tracked_paths(source_root, paths)?;

        let mut records = Vec::with_capacity(paths.len());
        for path in paths {
            let (ignored, match_info) = if tracked.contains(path) {
                (false, None)
            } else {
                let abs = path.to_path(source_root);
                let mode = if abs.is_dir() {
                    Some(gix::index::entry::Mode::DIR)
                } else {
                    None
                };
                let platform = excludes
                    .at_path(Path::new(path.as_str()), mode)
                    .map_err(|e| Error::Io {
                        context: format!("matching ignore patterns for {}", path.as_str()),
                        source: e,
                    })?;

                let matched = platform.matching_exclude_pattern();
                let ignored = matched
                    .as_ref()
                    .is_some_and(|matched| !matched.pattern.is_negative());
                let match_info = matched.map(|m| IgnoreMatchInfo {
                    source_file: m
                        .source
                        .map(|p| Self::normalize_ignore_source(p, source_root))
                        .unwrap_or_default(),
                    line: m.sequence_number,
                    pattern: m.pattern.to_string(),
                });
                (ignored, match_info)
            };

            records.push(IgnoreCheckRecord {
                path: path.clone(),
                ignored,
                match_info,
            });
        }

        Ok(records)
    }

    fn list_worktreeinclude_candidates(
        &self,
        source_root: &Path,
        semantics: WorktreeincludeSemantics,
        symlink_policy: SymlinkPolicy,
    ) -> Result<Vec<RepoRelPath>> {
        // Submodules registered with `git submodule add` are stored in the
        // index as entries with mode 160000 (gitlink). `git ls-files` skips
        // these when walking the worktree, and so must we.
        let state = self.index_state(source_root)?;
        let ignore_case = state.ignore_case;
        let tracked_paths = &state.tracked;
        let gitlinks = &state.gitlinks;

        let engine = crate::worktreeinclude_engine::engine_for(semantics);
        let mut candidates = Vec::new();
        for entry in walkdir::WalkDir::new(source_root)
            .into_iter()
            .filter_entry(|e| !is_nested_git_boundary(e, source_root, gitlinks, ignore_case))
        {
            let entry = entry.map_err(|e| Error::Git {
                message: format!("failed walking {}: {e}", source_root.display()),
            })?;

            if entry.file_type().is_dir() {
                continue;
            }

            #[cfg(unix)]
            let rel_path = native_repo_relative_path(entry.path(), source_root)?;
            #[cfg(not(unix))]
            let rel = RepoRelPath::normalize(entry.path(), source_root)?;
            #[cfg(not(unix))]
            let rel_path = Path::new(rel.as_str());

            // Git paths are raw bytes on Unix. Check trackedness and
            // worktreeinclude selection before crossing the UTF-8-only
            // RepoRelPath boundary, so an unrelated raw-byte filename cannot
            // abort discovery.
            #[cfg(unix)]
            {
                use std::os::unix::ffi::OsStrExt;
                if tracked_paths.contains(source_root, rel_path.as_os_str().as_bytes()) {
                    continue;
                }
            }

            let selected = matches!(
                engine.evaluate_path(source_root, rel_path, false, ignore_case, symlink_policy),
                crate::model::WorktreeincludeStatus::Included { .. }
            );
            if !selected {
                continue;
            }

            // Selected non-UTF-8 names still fail closed here: downstream
            // policy, planning, and copy operations require an unambiguous
            // normalized repository path.
            #[cfg(unix)]
            let rel = RepoRelPath::normalize(entry.path(), source_root)?;
            if tracked_paths.contains(source_root, rel.as_str().as_bytes()) {
                continue;
            }
            candidates.push(rel);
        }

        candidates.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        Ok(candidates)
    }

    fn list_ignored_untracked(&self, source_root: &Path) -> Result<Vec<RepoRelPath>> {
        let repo = self.discover_repo(source_root)?;
        let worktree = repo.worktree().ok_or_else(|| Error::Git {
            message: format!(
                "cannot enumerate ignored files for bare repository at {}",
                repo.path().display()
            ),
        })?;
        let mut excludes = worktree.excludes(None).map_err(|e| Error::Git {
            message: format!(
                "gix failed to initialize exclude stack for {}: {e}",
                source_root.display()
            ),
        })?;

        let state = self.index_state(source_root)?;
        let ignore_case = state.ignore_case;
        let tracked_paths = &state.tracked;
        let gitlinks = &state.gitlinks;

        let mut result = Vec::new();
        for entry in walkdir::WalkDir::new(source_root)
            .into_iter()
            .filter_entry(|e| !is_nested_git_boundary(e, source_root, gitlinks, ignore_case))
        {
            let entry = entry.map_err(|e| Error::Git {
                message: format!("failed walking {}: {e}", source_root.display()),
            })?;

            if entry.file_type().is_dir() {
                continue;
            }

            // Match ignore state using the native pathname first. This lets
            // an unrelated, unignored non-UTF-8 Unix name remain outside the
            // candidate set just as it does with `git ls-files`; selected
            // names still fail closed when converted to RepoRelPath below.
            #[cfg(unix)]
            let rel_path = native_repo_relative_path(entry.path(), source_root)?;
            #[cfg(not(unix))]
            let rel = RepoRelPath::normalize(entry.path(), source_root)?;
            #[cfg(not(unix))]
            let rel_path = Path::new(rel.as_str());
            let platform = excludes.at_path(rel_path, None).map_err(|e| Error::Io {
                context: format!("matching ignore patterns for {}", entry.path().display()),
                source: e,
            })?;

            if !platform.is_excluded() {
                continue;
            }

            #[cfg(unix)]
            let rel = RepoRelPath::normalize(entry.path(), source_root)?;
            if tracked_paths.contains(source_root, rel.as_str().as_bytes()) {
                continue;
            }
            result.push(rel);
        }

        result.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        Ok(result)
    }

    fn worktreeinclude_exists_anywhere(
        &self,
        source_root: &Path,
        symlink_policy: SymlinkPolicy,
    ) -> Result<bool> {
        let state = self.index_state(source_root)?;
        Ok(walk_for_first_worktreeinclude(
            source_root,
            &state.gitlinks,
            state.ignore_case,
            symlink_policy,
        ))
    }

    fn checkout_folds_case(&self, source_root: &Path) -> Result<bool> {
        self.cached_checkout_folds_case(source_root)
    }

    fn read_bool_config(&self, source_root: &Path, key: &str) -> Result<bool> {
        let repo = self.discover_repo(source_root)?;
        Ok(repo.config_snapshot().boolean(key).unwrap_or(false))
    }

    fn read_config(&self, source_root: &Path, key: &str) -> Result<Option<String>> {
        let repo = self.discover_repo(source_root)?;
        let value = repo.config_snapshot().string(key);
        match value {
            Some(v) => {
                let bytes: &[u8] = v.as_ref();
                let trimmed = String::from_utf8_lossy(bytes).trim().to_string();
                if trimmed.is_empty() {
                    Ok(None)
                } else {
                    Ok(Some(trimmed))
                }
            }
            None => Ok(None),
        }
    }

    fn reads_default_global_excludes(&self) -> bool {
        true
    }
}

/// Everything a run needs to know about one repository's index.
///
/// Grouping these together lets a backend answer trackedness and submodule
/// boundary questions from a single index read.
#[derive(Debug)]
struct RepoIndexState {
    /// Tracked-name lookup built from every index entry.
    tracked: TrackedPathLookup,
    /// Registered submodule paths (mode 160000 gitlinks).
    gitlinks: HashSet<String>,
    /// Whether this checkout folds case; see [`case_folding_applies`].
    ///
    /// Carried here so walkers reading a snapshot get the same answer the
    /// tracked lookup was built with. It is resolved from the config by
    /// [`RepoStateCache::checkout_folds_case`], not from the index, and is
    /// stable for the life of a backend instance.
    ignore_case: bool,
}

impl RepoIndexState {
    /// Build from `git ls-files -s -z --full-name` output.
    ///
    /// Each record is `<mode> <object> <stage>\t<path>`, so one invocation
    /// supplies both the tracked names and the gitlink entries. Only gitlink
    /// paths are converted to [`RepoRelPath`]; tracked names stay raw bytes so
    /// an unrelated non-UTF-8 index entry cannot fail the whole lookup.
    fn from_ls_files_stage_output(output: &[u8], ignore_case: bool) -> Result<Self> {
        let mut tracked_paths: Vec<&[u8]> = Vec::new();
        let mut gitlinks = HashSet::new();
        for record in output.split(|&b| b == 0) {
            if record.is_empty() {
                continue;
            }
            let Some(tab) = record.iter().position(|byte| *byte == b'\t') else {
                continue;
            };
            let (metadata, path) = record.split_at(tab);
            let path = &path[1..];
            if metadata.starts_with(b"160000 ") {
                gitlinks.insert(RepoRelPath::from_git_bytes(path)?.as_str().to_string());
            }
            tracked_paths.push(path);
        }
        Ok(Self {
            tracked: TrackedPathLookup::new(tracked_paths, ignore_case),
            gitlinks,
            ignore_case,
        })
    }

    fn from_gix_index(index: &gix::index::State, ignore_case: bool) -> Result<Self> {
        Ok(Self {
            tracked: TrackedPathLookup::new(
                index
                    .entries()
                    .iter()
                    .map(|entry| entry.path(index).as_ref()),
                ignore_case,
            ),
            gitlinks: gitlinks_from_gix_index(index)?,
            ignore_case,
        })
    }
}

/// Identifying stat data for a Git index file.
///
/// Git publishes a new index by renaming `index.lock` over `index`, so any
/// committed change to trackedness gives the index a new inode; size and
/// modification time catch the remaining unusual writers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct IndexFingerprint {
    identity: Option<FilesystemIdentity>,
    size: u64,
    modified: Option<std::time::SystemTime>,
}

fn index_fingerprint(index_path: &Path) -> Option<IndexFingerprint> {
    let metadata = std::fs::symlink_metadata(index_path).ok()?;
    Some(IndexFingerprint {
        identity: filesystem_identity(index_path),
        size: metadata.len(),
        modified: metadata.modified().ok(),
    })
}

/// Per-backend memo of index-derived repository state.
///
/// The executor calls `tracked_paths` once per published file while holding
/// the destination index lock. Rebuilding the lookup there costs a full index
/// read every time — and for the CLI backend, several subprocess spawns under
/// the lock. Instead each backend builds the state once per repository and
/// revalidates it with a single `stat` of the index file, which keeps the
/// under-lock recheck honest: a cache hit means the same index bytes the
/// snapshot was built from are still in place.
///
/// `core.ignoreCase` is memoized separately and *not* fingerprinted. It comes
/// from the config, not the index, so folding it into the index snapshot would
/// tie a config answer to an unrelated file's mtime: an index write would
/// re-run the config read, and a config edit would appear to take effect only
/// if the index happened to move. Resolving it exactly once per repository is
/// both cheaper and honest about what is guaranteed; see
/// [`GitBackend::checkout_folds_case`].
#[derive(Debug, Default)]
struct RepoStateCache {
    index_paths: Mutex<HashMap<PathBuf, PathBuf>>,
    case_folding: Mutex<HashMap<PathBuf, bool>>,
    states: Mutex<HashMap<PathBuf, (IndexFingerprint, Arc<RepoIndexState>)>>,
}

impl RepoStateCache {
    /// Resolve the index path for `source_root` once per backend instance.
    fn index_path(
        &self,
        source_root: &Path,
        resolve: impl FnOnce() -> Result<PathBuf>,
    ) -> Result<PathBuf> {
        if let Some(cached) = lock(&self.index_paths).get(source_root) {
            return Ok(cached.clone());
        }
        let resolved = resolve()?;
        lock(&self.index_paths).insert(source_root.to_path_buf(), resolved.clone());
        Ok(resolved)
    }

    /// Resolve `core.ignoreCase` for `source_root` once per backend instance.
    ///
    /// Every later query returns the first answer, so one run cannot apply
    /// folded protection to some paths and exact matching to others.
    fn checkout_folds_case(
        &self,
        source_root: &Path,
        resolve: impl FnOnce() -> Result<bool>,
    ) -> Result<bool> {
        if let Some(cached) = lock(&self.case_folding).get(source_root) {
            return Ok(*cached);
        }
        let resolved = resolve()?;
        lock(&self.case_folding).insert(source_root.to_path_buf(), resolved);
        Ok(resolved)
    }

    /// Return the cached state when the index file is still the one the state
    /// was built from, otherwise build a fresh state and cache that.
    fn index_state(
        &self,
        source_root: &Path,
        index_path: &Path,
        build: impl FnOnce() -> Result<RepoIndexState>,
    ) -> Result<Arc<RepoIndexState>> {
        let before = index_fingerprint(index_path);
        if let Some(before) = before
            && let Some((fingerprint, state)) = lock(&self.states).get(source_root)
            && *fingerprint == before
        {
            return Ok(Arc::clone(state));
        }

        let state = Arc::new(build()?);
        // Only cache when the index did not change while it was being read;
        // otherwise the snapshot cannot be attributed to either fingerprint.
        if let Some(before) = before
            && index_fingerprint(index_path) == Some(before)
        {
            lock(&self.states).insert(source_root.to_path_buf(), (before, Arc::clone(&state)));
        }
        Ok(state)
    }
}

/// Take a lock, recovering from poisoning.
///
/// A panic elsewhere must not turn a cache into a hard failure; the cached
/// values are plain data and remain consistent.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Walk the source tree looking for the first `.worktreeinclude` file,
/// skipping nested git checkouts/submodules.
///
/// Pure filesystem walk; the only Git-specific inputs are the gitlinks set and
/// the checkout's case sensitivity. Under `SymlinkPolicy::Ignore`, symlinked
/// rule files do not count toward existence (consistent with their being
/// treated as absent during selection).
fn walk_for_first_worktreeinclude(
    source_root: &Path,
    gitlinks: &HashSet<String>,
    ignore_case: bool,
    symlink_policy: SymlinkPolicy,
) -> bool {
    for entry in walkdir::WalkDir::new(source_root)
        .into_iter()
        .filter_entry(|e| !is_nested_git_boundary(e, source_root, gitlinks, ignore_case))
    {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        if entry.file_type().is_dir() {
            continue;
        }
        if !crate::walk::special_filename_matches(entry.file_name(), ".worktreeinclude") {
            continue;
        }
        if entry.file_type().is_symlink() {
            if symlink_policy == SymlinkPolicy::Ignore {
                continue;
            }
        } else if !entry.file_type().is_file() {
            continue;
        }
        return true;
    }
    false
}

/// CLI-backend candidate enumeration that mirrors the gix walker and invokes
/// the selected semantics engine for every path.
fn cli_list_candidates_with_engine(
    cli: &GitCli,
    source_root: &Path,
    semantics: WorktreeincludeSemantics,
    symlink_policy: SymlinkPolicy,
) -> Result<Vec<RepoRelPath>> {
    // Tracked paths must be excluded from candidates, the same as the index
    // check used by the gix backend, and gitlinks bound the walk. Both come
    // from one cached index read rather than per-path or per-run subprocesses.
    let state = cli.index_state(source_root)?;
    let ignore_case = state.ignore_case;
    let tracked = &state.tracked;
    let gitlinks = &state.gitlinks;

    let engine = crate::worktreeinclude_engine::engine_for(semantics);
    let mut candidates = Vec::new();
    for entry in walkdir::WalkDir::new(source_root)
        .into_iter()
        .filter_entry(|e| !is_nested_git_boundary(e, source_root, gitlinks, ignore_case))
    {
        let entry = entry.map_err(|e| Error::Git {
            message: format!("failed walking {}: {e}", source_root.display()),
        })?;
        if entry.file_type().is_dir() {
            continue;
        }
        #[cfg(unix)]
        let rel_path = native_repo_relative_path(entry.path(), source_root)?;
        #[cfg(not(unix))]
        let rel = RepoRelPath::normalize(entry.path(), source_root)?;
        #[cfg(not(unix))]
        let rel_path = Path::new(rel.as_str());

        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            if tracked.contains(source_root, rel_path.as_os_str().as_bytes()) {
                continue;
            }
        }

        let selected = matches!(
            engine.evaluate_path(source_root, rel_path, false, ignore_case, symlink_policy),
            crate::model::WorktreeincludeStatus::Included { .. }
        );
        if !selected {
            continue;
        }

        #[cfg(unix)]
        let rel = RepoRelPath::normalize(entry.path(), source_root)?;
        if tracked.contains(source_root, rel.as_str().as_bytes()) {
            continue;
        }
        candidates.push(rel);
    }

    candidates.sort_by(|a, b| a.as_str().cmp(b.as_str()));
    Ok(candidates)
}

/// Return a byte-preserving repository-relative path for Unix discovery.
///
/// Windows paths are normalized through `RepoRelPath` before matching, which
/// also reconciles canonical `\\?\` spellings with Git's ordinary roots.
#[cfg(unix)]
fn native_repo_relative_path<'a>(path: &'a Path, source_root: &Path) -> Result<&'a Path> {
    path.strip_prefix(source_root)
        .map_err(|error| Error::InvalidPath {
            message: format!(
                "{} is outside repository root {}: {error}",
                path.display(),
                source_root.display()
            ),
        })
}

/// Compare normalized repository paths using the same case semantics as
/// tracked-path protection.
///
/// This is crate-visible so other repository-boundary checks can avoid
/// drifting back to ASCII-only comparisons.
pub(crate) fn repo_paths_equivalent(left: &str, right: &str, ignore_case: bool) -> bool {
    repo_path_bytes_equal(left.as_bytes(), right.as_bytes(), ignore_case)
}

/// One repository path's "is this a filesystem alias of some other path?"
/// question, with everything that depends only on that path resolved up front.
///
/// Callers ask this of a whole candidate set at once. The two costly parts
/// depend only on the probed path: proving it is a *non-exact* spelling reads
/// one directory per path component, and resolving its identity is a `stat`.
/// Both are hoisted here so testing a candidate set costs one `stat` per
/// candidate instead of a directory walk per candidate.
///
/// Requiring at least one missing exact directory entry distinguishes a
/// case/normalization alias from two explicitly named hard links.
pub(crate) struct RepoPathAliasProbe<'a> {
    source_root: &'a Path,
    alias: &'a RepoRelPath,
    /// Identity of `alias`, or `None` when it cannot be an alias of anything:
    /// it already names an existing entry exactly, or it does not resolve.
    identity: Option<FilesystemIdentity>,
}

impl<'a> RepoPathAliasProbe<'a> {
    /// Resolve the path-dependent half of the question.
    pub(crate) fn new(source_root: &'a Path, alias: &'a RepoRelPath) -> Self {
        let identity = if repo_path_has_exact_spelling(source_root, alias) {
            None
        } else {
            repo_path_filesystem_identity(source_root, alias.as_str().as_bytes())
        };
        Self {
            source_root,
            alias,
            identity,
        }
    }

    /// Whether the filesystem resolves the probed path and `canonical` to the
    /// same entry, the probed path being the non-exact spelling of the two.
    pub(crate) fn resolves_to(&self, canonical: &RepoRelPath) -> bool {
        let Some(identity) = self.identity else {
            return false;
        };
        if canonical == self.alias {
            return false;
        }
        repo_path_filesystem_identity(self.source_root, canonical.as_str().as_bytes())
            == Some(identity)
    }
}

fn repo_path_has_exact_spelling(source_root: &Path, path: &RepoRelPath) -> bool {
    let mut directory = source_root.to_path_buf();
    for component in path.as_str().split('/') {
        let Ok(mut entries) = std::fs::read_dir(&directory) else {
            return false;
        };
        if !entries.any(|entry| {
            entry
                .ok()
                .is_some_and(|entry| entry.file_name() == std::ffi::OsStr::new(component))
        }) {
            return false;
        }
        directory.push(component);
    }
    true
}

/// Compare repository paths using exact bytes first, then conservative
/// normalized Unicode folding when Git is configured case-insensitively.
fn repo_path_bytes_equal(left: &[u8], right: &[u8], ignore_case: bool) -> bool {
    left == right || (ignore_case && case_folded_repo_path(left) == case_folded_repo_path(right))
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum CaseFoldedRepoPath {
    Utf8(String),
    NonUtf8(Vec<u8>),
}

fn case_folded_repo_path(path: &[u8]) -> CaseFoldedRepoPath {
    match std::str::from_utf8(path) {
        Ok(path) => {
            // Unicode's stable caseless matching transform is
            // NFD(toCasefold(NFD(path))). `char::to_lowercase` is not enough:
            // Greek final sigma (ς) and sigma (σ), for example, are aliases
            // on normal case-insensitive macOS volumes but lowercase to
            // distinct code points.
            let decomposed = path.nfd().collect::<String>();
            let folded = decomposed.as_str().case_fold().collect::<String>();
            CaseFoldedRepoPath::Utf8(folded.nfd().collect())
        }
        Err(_) => CaseFoldedRepoPath::NonUtf8(
            path.iter()
                .copied()
                .map(|byte| byte.to_ascii_lowercase())
                .collect(),
        ),
    }
}

/// Decide whether differently-cased spellings name the same repository path.
///
/// `configured` is `core.ignoreCase` as recorded in the repository config, or
/// `None` when the key is absent. Git probes the checkout's filesystem when it
/// creates a repository and records the answer there, so an explicit value is
/// authoritative for that checkout and is exactly what Git itself obeys: a
/// case-sensitive volume with `core.ignoreCase = false` really does hold
/// `Secret.env` and `secret.env` as two distinct files, and treating them as
/// one silently drops a legitimate candidate.
///
/// Only when the key is absent — a repository whose config Git did not write —
/// does the platform's usual filesystem behavior decide, so a hand-assembled
/// macOS or Windows checkout still gets the conservative answer.
pub(crate) fn case_folding_applies(configured: Option<bool>) -> bool {
    configured.unwrap_or(cfg!(any(target_os = "macos", windows)))
}

/// Precomputed tracked-name lookup.
///
/// Every query is a hash lookup against the index names, plus — only when a
/// query collides with a tracked name under Unicode case folding — a
/// filesystem-identity check against the 0-1 colliding index entries. Work per
/// query is therefore bounded by the size of that one folded-name bucket,
/// never by the size of the index.
#[derive(Debug)]
struct TrackedPathLookup {
    exact: HashSet<Vec<u8>>,
    folded: HashMap<CaseFoldedRepoPath, Vec<Vec<u8>>>,
    /// Whether a folded-name collision alone proves trackedness, i.e. whether
    /// the checkout folds case; see [`case_folding_applies`].
    protect_folded_names: bool,
}

impl TrackedPathLookup {
    fn new<'a>(paths: impl IntoIterator<Item = &'a [u8]>, ignore_case: bool) -> Self {
        let mut exact = HashSet::new();
        let mut folded: HashMap<CaseFoldedRepoPath, Vec<Vec<u8>>> = HashMap::new();

        for path in paths {
            exact.insert(path.to_vec());
            folded
                .entry(case_folded_repo_path(path))
                .or_default()
                .push(path.to_vec());
        }

        Self {
            exact,
            folded,
            protect_folded_names: ignore_case,
        }
    }

    /// Return the subset of `paths` that the index tracks, in the caller's
    /// spelling.
    fn select(&self, source_root: &Path, paths: &[RepoRelPath]) -> HashSet<RepoRelPath> {
        paths
            .iter()
            .filter(|path| self.contains(source_root, path.as_str().as_bytes()))
            .cloned()
            .collect()
    }

    fn contains(&self, source_root: &Path, query: &[u8]) -> bool {
        if self.exact.contains(query) {
            return true;
        }

        // No tracked name folds onto this query, so no spelling of it can be
        // the tracked one. This is the overwhelmingly common answer and costs
        // one hash lookup.
        let Some(bucket) = self.folded.get(&case_folded_repo_path(query)) else {
            return false;
        };

        if self.protect_folded_names {
            return true;
        }

        // Git says this checkout is case-sensitive, so a differently-cased
        // spelling is a different file unless the filesystem disagrees.
        // Confirm against the colliding index entries only.
        let Some(query_identity) = repo_path_filesystem_identity(source_root, query) else {
            return false;
        };
        bucket
            .iter()
            .any(|path| repo_path_filesystem_identity(source_root, path) == Some(query_identity))
    }
}

fn repo_path_filesystem_identity(source_root: &Path, path: &[u8]) -> Option<FilesystemIdentity> {
    let path = repo_bytes_to_path(source_root, path)?;
    filesystem_identity(&path)
}

#[cfg(unix)]
fn repo_bytes_to_path(source_root: &Path, path: &[u8]) -> Option<PathBuf> {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    Some(source_root.join(OsStr::from_bytes(path)))
}

#[cfg(not(unix))]
fn repo_bytes_to_path(source_root: &Path, path: &[u8]) -> Option<PathBuf> {
    Some(source_root.join(std::str::from_utf8(path).ok()?))
}

/// Opaque per-object filesystem identity: two pathnames with the same identity
/// name the same object.
#[cfg(unix)]
type FilesystemIdentity = (u64, u64);

#[cfg(unix)]
fn filesystem_identity(path: &Path) -> Option<FilesystemIdentity> {
    use std::os::unix::fs::MetadataExt;
    let metadata = std::fs::symlink_metadata(path).ok()?;
    Some((metadata.dev(), metadata.ino()))
}

#[cfg(windows)]
type FilesystemIdentity = (u64, u64);

#[cfg(windows)]
fn filesystem_identity(path: &Path) -> Option<FilesystemIdentity> {
    let handle = winapi_util::Handle::from_path_any(path).ok()?;
    let info = winapi_util::file::information(&handle).ok()?;
    Some((info.volume_serial_number(), info.file_index()))
}

#[cfg(not(any(unix, windows)))]
type FilesystemIdentity = ();

#[cfg(not(any(unix, windows)))]
fn filesystem_identity(_path: &Path) -> Option<FilesystemIdentity> {
    None
}

/// Return true when `entry` is a directory that should not be descended into
/// because it is either a `.git` directory or sits at the root of a nested
/// Git checkout (registered submodule or nested clone).
///
/// Recursing into nested checkouts would copy untracked/ignored files out of
/// those repositories — which `git ls-files --others --ignored` does not do
/// without `--recurse-submodules`. Mirroring git's exact rules keeps the gix
/// backend in parity with the CLI backend and satisfies the v1 spec rule of
/// not recursing into submodules or nested Git repositories.
///
/// What gets skipped:
/// - The repo's own `.git` directory.
/// - Subdirectories with a `.git` *directory* (an independent nested clone).
/// - Subdirectories registered as gitlinks (proper submodules).
///
/// What does *not* get skipped (matching CLI behavior):
/// - The walk root itself, even though it has its own `.git`.
/// - Subdirectories whose only Git marker is a bare `.git` *file* with no
///   matching gitlink in the index. Git CLI treats these as ordinary
///   directories, and so do we.
fn is_nested_git_boundary(
    entry: &walkdir::DirEntry,
    source_root: &Path,
    gitlinks: &HashSet<String>,
    ignore_case: bool,
) -> bool {
    if !entry.file_type().is_dir() {
        return false;
    }
    crate::walk::is_git_boundary_dir(
        entry.path(),
        entry.depth(),
        source_root,
        gitlinks,
        ignore_case,
    )
}

/// Parse the output of `git worktree list --porcelain -z`.
///
/// With `-z`, every attribute line's terminating newline is replaced with NUL,
/// and the blank line separating records also becomes NUL. So the byte stream
/// is a sequence of NUL-terminated fields:
///
/// ```text
/// worktree /path\0HEAD sha\0branch ref\0\0worktree /path2\0bare\0\0
/// ```
///
/// A `worktree <path>` field starts a new record. Subsequent fields (`HEAD`,
/// `branch`, `bare`, `detached`) are attributes of the current record.
/// An empty field (from the double-NUL record separator) finalizes the record.
/// The first record is always the main worktree.
fn parse_worktree_list(output: &[u8]) -> Result<Vec<WorktreeRecord>> {
    let mut worktrees = Vec::new();

    let mut current_path: Option<PathBuf> = None;
    let mut current_is_bare = false;

    for field in output.split(|byte| *byte == 0) {
        if field.is_empty() {
            // Empty field = record separator. Finalize current record if any.
            if let Some(path) = current_path.take() {
                let is_main = worktrees.is_empty();
                worktrees.push(WorktreeRecord {
                    path,
                    is_main,
                    is_bare: current_is_bare,
                });
                current_is_bare = false;
            }
            continue;
        }

        if let Some(path_bytes) = field.strip_prefix(b"worktree ") {
            // A new record starts. Finalize any pending record first (handles
            // streams that lack the trailing double-NUL).
            if let Some(path) = current_path.take() {
                let is_main = worktrees.is_empty();
                worktrees.push(WorktreeRecord {
                    path,
                    is_main,
                    is_bare: current_is_bare,
                });
                current_is_bare = false;
            }
            current_path = Some(path_buf_from_git_bytes(path_bytes, "worktree path")?);
        } else if field == b"bare" {
            current_is_bare = true;
        }
        // Other fields (HEAD, branch, detached) are ignored for now.
    }

    // Finalize any trailing record (e.g., if output lacks trailing NUL).
    if let Some(path) = current_path.take() {
        let is_main = worktrees.is_empty();
        worktrees.push(WorktreeRecord {
            path,
            is_main,
            is_bare: current_is_bare,
        });
    }

    if worktrees.is_empty() {
        return Err(Error::Git {
            message: "no worktrees found".to_string(),
        });
    }

    Ok(worktrees)
}

/// Parse the output of `git check-ignore --stdin -z -v -n`.
///
/// With `-z`, the output uses NUL as field separator. Each record has four
/// fields: source, linenum, pattern, pathname. For non-matching paths
/// (enabled by `-n`), source, linenum, and pattern are empty.
fn parse_check_ignore_output(output: &[u8]) -> Result<Vec<IgnoreCheckRecord>> {
    if output.is_empty() {
        return Ok(Vec::new());
    }

    let mut records = Vec::new();
    let fields: Vec<&[u8]> = output.split(|&b| b == 0).collect();

    // Each record is 4 fields: source, linenum, pattern, pathname
    let mut i = 0;
    while i + 3 < fields.len() {
        let source = String::from_utf8_lossy(fields[i]).to_string();
        let linenum_str = String::from_utf8_lossy(fields[i + 1]).to_string();
        let pattern = String::from_utf8_lossy(fields[i + 2]).to_string();
        let path = RepoRelPath::from_git_bytes(fields[i + 3])?;

        let match_info = if source.is_empty() && linenum_str.is_empty() {
            None
        } else {
            let line = linenum_str.parse::<usize>().unwrap_or(0);
            Some(IgnoreMatchInfo {
                source_file: PathBuf::from(source),
                line,
                pattern,
            })
        };

        let ignored = match_info
            .as_ref()
            .is_some_and(|info| !info.pattern.starts_with('!'));
        records.push(IgnoreCheckRecord {
            path,
            ignored,
            match_info,
        });
        i += 4;
    }

    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- backend selection ----

    #[test]
    fn git_backend_selection_accepts_both_names_in_any_ascii_case() {
        for (value, expected) in [
            ("gix", GitBackendKind::Gix),
            ("GIX", GitBackendKind::Gix),
            ("cli", GitBackendKind::Cli),
            ("Cli", GitBackendKind::Cli),
            ("  cli\n", GitBackendKind::Cli),
        ] {
            assert_eq!(parse_git_backend_kind(value).unwrap(), expected);
        }
    }

    #[test]
    fn git_backend_selection_rejects_anything_else() {
        for value in ["", "gx", "gitcli", "git", "cli,gix", "true"] {
            let error = parse_git_backend_kind(value)
                .err()
                .unwrap_or_else(|| panic!("{value:?} must not select a backend"))
                .to_string();
            assert!(
                error.contains("WAFT_GIT_BACKEND") && error.contains("\"gix\", \"cli\""),
                "error for {value:?} must name the variable and valid values: {error}"
            );
        }
    }

    // ---- case-sensitivity policy ----

    /// An explicitly configured value is authoritative in both directions;
    /// only an absent key defers to the platform.
    #[test]
    fn case_folding_policy_follows_the_repository_configuration() {
        assert!(case_folding_applies(Some(true)));
        assert!(!case_folding_applies(Some(false)));
        assert_eq!(
            case_folding_applies(None),
            cfg!(any(target_os = "macos", windows)),
            "an unconfigured repository falls back to the platform default"
        );
    }

    // ---- tracked-path lookup ----

    fn tracked_lookup(paths: &[&str], ignore_case: bool) -> TrackedPathLookup {
        TrackedPathLookup::new(paths.iter().map(|path| path.as_bytes()), ignore_case)
    }

    /// A root that cannot exist keeps these assertions independent of the host
    /// filesystem: no identity confirmation can ever succeed.
    fn unreachable_root() -> &'static Path {
        Path::new("/waft-nonexistent-root-for-unit-tests")
    }

    #[test]
    fn tracked_lookup_matches_exact_index_spellings() {
        let lookup = tracked_lookup(&["secret.env", "cfg/app.env"], false);
        assert!(lookup.contains(unreachable_root(), b"secret.env"));
        assert!(lookup.contains(unreachable_root(), b"cfg/app.env"));
        assert!(!lookup.contains(unreachable_root(), b"other.env"));
    }

    #[test]
    fn tracked_lookup_protects_folded_names_on_a_case_folding_checkout() {
        let lookup = tracked_lookup(&["secret.env"], true);
        assert!(lookup.contains(unreachable_root(), b"SECRET.env"));
        assert!(!lookup.contains(unreachable_root(), "Ä.env".as_bytes()));

        let unicode = tracked_lookup(&["ä.env"], true);
        assert!(unicode.contains(unreachable_root(), "Ä.env".as_bytes()));
    }

    /// The regression this fixes: on a case-sensitive checkout a distinct file
    /// whose name folds onto a tracked one must stay eligible, even though the
    /// host running the test may itself fold case.
    #[test]
    fn tracked_lookup_keeps_exact_semantics_on_a_case_sensitive_checkout() {
        let lookup = tracked_lookup(&["secret.env"], false);
        assert!(!lookup.contains(unreachable_root(), b"SECRET.env"));

        let unicode = tracked_lookup(&["ä.env"], false);
        assert!(!unicode.contains(unreachable_root(), "Ä.env".as_bytes()));
    }

    /// Only the colliding folded-name bucket is ever consulted, so an index
    /// full of unrelated names contributes no per-query work — and no
    /// unrelated name can be mistaken for the query.
    #[test]
    fn tracked_lookup_ignores_index_entries_outside_the_folded_bucket() {
        let temp = tempfile::TempDir::new().unwrap();
        std::fs::write(temp.path().join("tracked.env"), "x").unwrap();
        std::fs::hard_link(
            temp.path().join("tracked.env"),
            temp.path().join("alias.env"),
        )
        .unwrap();

        let lookup = tracked_lookup(&["tracked.env"], false);
        assert!(lookup.contains(temp.path(), b"tracked.env"));
        assert!(
            !lookup.contains(temp.path(), b"alias.env"),
            "a separately named hard link is a separate path"
        );
    }

    #[test]
    fn tracked_lookup_selects_queries_in_the_callers_spelling() {
        let lookup = tracked_lookup(&["secret.env"], true);
        let query = RepoRelPath::from_normalized("SECRET.env".to_string());
        let other = RepoRelPath::from_normalized("public.env".to_string());
        let selected = lookup.select(unreachable_root(), &[query.clone(), other]);
        assert_eq!(selected.len(), 1);
        assert!(selected.contains(&query));
    }

    // ---- index state parsing ----

    #[test]
    fn index_state_reads_tracked_names_and_gitlinks_from_one_listing() {
        let output = b"100644 aaaaaaaa 0\tsecret.env\x00160000 bbbbbbbb 0\tvendor/sub\x00100644 cccccccc 0\tcfg/app.env\x00";
        let state = RepoIndexState::from_ls_files_stage_output(output, false).unwrap();

        assert!(state.tracked.contains(unreachable_root(), b"secret.env"));
        assert!(state.tracked.contains(unreachable_root(), b"cfg/app.env"));
        assert!(
            state.tracked.contains(unreachable_root(), b"vendor/sub"),
            "a gitlink entry is still a tracked path"
        );
        assert_eq!(
            state.gitlinks,
            HashSet::from(["vendor/sub".to_string()]),
            "only mode 160000 entries are submodule boundaries"
        );
    }

    #[cfg(unix)]
    #[test]
    fn index_state_keeps_non_utf8_tracked_names_without_failing() {
        let mut output = Vec::new();
        output.extend_from_slice(b"100644 aaaaaaaa 0\tsecret-\xff.env\x00");
        let state = RepoIndexState::from_ls_files_stage_output(&output, false).unwrap();
        assert!(
            state
                .tracked
                .contains(unreachable_root(), b"secret-\xff.env")
        );
    }

    // ---- index-validated caching ----

    #[test]
    fn cached_index_state_is_reused_until_the_index_changes() {
        let temp = tempfile::TempDir::new().unwrap();
        let index = temp.path().join("index");
        std::fs::write(&index, b"first").unwrap();

        let cache = RepoStateCache::default();
        let builds = std::cell::Cell::new(0);
        let build = || {
            builds.set(builds.get() + 1);
            RepoIndexState::from_ls_files_stage_output(b"100644 a 0\tsecret.env\x00", false)
        };

        cache.index_state(temp.path(), &index, build).unwrap();
        cache.index_state(temp.path(), &index, build).unwrap();
        assert_eq!(builds.get(), 1, "an unchanged index must not be re-read");

        // Git publishes a new index by renaming a lock file into place, which
        // is what the fingerprint is designed to notice.
        let replacement = temp.path().join("index.lock");
        std::fs::write(&replacement, b"second and longer").unwrap();
        std::fs::rename(&replacement, &index).unwrap();

        cache.index_state(temp.path(), &index, build).unwrap();
        assert_eq!(builds.get(), 2, "a replaced index must be re-read");
    }

    #[test]
    fn missing_index_state_is_rebuilt_rather_than_cached() {
        let temp = tempfile::TempDir::new().unwrap();
        let index = temp.path().join("index");

        let cache = RepoStateCache::default();
        let builds = std::cell::Cell::new(0);
        let build = || {
            builds.set(builds.get() + 1);
            RepoIndexState::from_ls_files_stage_output(b"", false)
        };

        cache.index_state(temp.path(), &index, build).unwrap();
        cache.index_state(temp.path(), &index, build).unwrap();
        assert_eq!(
            builds.get(),
            2,
            "without an index file there is nothing to validate a cache against"
        );
    }

    #[test]
    fn cached_index_path_is_resolved_once_per_repository() {
        let cache = RepoStateCache::default();
        let resolutions = std::cell::Cell::new(0);
        let resolve = || {
            resolutions.set(resolutions.get() + 1);
            Ok(PathBuf::from("/repo/.git/index"))
        };

        let root = Path::new("/repo");
        assert_eq!(
            cache.index_path(root, resolve).unwrap(),
            PathBuf::from("/repo/.git/index")
        );
        assert_eq!(
            cache.index_path(root, resolve).unwrap(),
            PathBuf::from("/repo/.git/index")
        );
        assert_eq!(resolutions.get(), 1);
    }

    /// `core.ignoreCase` is memoized separately from the index snapshot, so it
    /// is read once per repository however many index rebuilds intervene — and
    /// separately per repository, since it is a per-checkout property.
    #[test]
    fn cached_case_folding_answer_is_resolved_once_per_repository() {
        let cache = RepoStateCache::default();
        let reads = std::cell::Cell::new(0);
        let folding = Path::new("/folding");
        let exact = Path::new("/exact");

        let resolve = |answer: bool| {
            let reads = &reads;
            move || {
                reads.set(reads.get() + 1);
                Ok(answer)
            }
        };

        assert!(cache.checkout_folds_case(folding, resolve(true)).unwrap());
        assert!(cache.checkout_folds_case(folding, resolve(false)).unwrap());
        assert_eq!(reads.get(), 1, "the second query must not re-read config");

        assert!(!cache.checkout_folds_case(exact, resolve(false)).unwrap());
        assert_eq!(
            reads.get(),
            2,
            "a different repository is a different answer"
        );
        assert!(
            cache.checkout_folds_case(folding, resolve(false)).unwrap(),
            "one repository's answer must not be overwritten by another's"
        );
        assert_eq!(reads.get(), 2);
    }

    #[test]
    fn repository_path_equivalence_normalizes_unicode_case() {
        assert!(repo_paths_equivalent("Ä.env", "a\u{308}.env", true));
        assert!(repo_paths_equivalent("σ.env", "ς.env", true));
        assert!(!repo_paths_equivalent("Ä.env", "a\u{308}.env", false));
        assert!(!repo_paths_equivalent("σ.env", "ς.env", false));
        assert!(repo_paths_equivalent("same/path", "same/path", false));
    }

    #[test]
    fn repository_path_equivalence_preserves_ascii_folding_for_non_utf8() {
        assert_eq!(
            case_folded_repo_path(b"DIR/\xffA.env"),
            case_folded_repo_path(b"dir/\xffa.env")
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn filesystem_alias_detection_handles_case_aliases() {
        let temp = tempfile::TempDir::new().unwrap();
        std::fs::write(temp.path().join("Secret.env"), "x").unwrap();
        assert!(
            temp.path().join("secret.env").exists(),
            "macOS safety regression requires a case-insensitive test volume"
        );
        let canonical = RepoRelPath::from_normalized("Secret.env".to_string());
        let alias = RepoRelPath::from_normalized("secret.env".to_string());
        assert!(RepoPathAliasProbe::new(temp.path(), &alias).resolves_to(&canonical));
    }

    #[cfg(unix)]
    #[test]
    fn filesystem_alias_detection_does_not_conflate_named_hard_links() {
        let temp = tempfile::TempDir::new().unwrap();
        std::fs::write(temp.path().join("first.env"), "x").unwrap();
        std::fs::hard_link(
            temp.path().join("first.env"),
            temp.path().join("second.env"),
        )
        .unwrap();
        let first = RepoRelPath::from_normalized("first.env".to_string());
        let second = RepoRelPath::from_normalized("second.env".to_string());
        assert!(!RepoPathAliasProbe::new(temp.path(), &second).resolves_to(&first));
    }

    /// The probe hoists everything that depends only on the probed path out of
    /// the candidate loop. That is sound only if the hoisted decision — "can
    /// this path be an alias of anything at all?" — is genuinely independent
    /// of the candidate, so pin both ways it can come out `no`.
    #[test]
    fn alias_probe_settles_candidate_independent_questions_up_front() {
        let temp = tempfile::TempDir::new().unwrap();
        std::fs::create_dir(temp.path().join("dir")).unwrap();
        std::fs::write(temp.path().join("dir/present.env"), "x").unwrap();
        let present = RepoRelPath::from_normalized("dir/present.env".to_string());

        // Names an existing entry exactly, so it is that path rather than an
        // alias of some other one.
        assert!(
            RepoPathAliasProbe::new(temp.path(), &present)
                .identity
                .is_none()
        );

        // Resolves to nothing, so no candidate can be the same entry.
        let missing = RepoRelPath::from_normalized("dir/absent.env".to_string());
        let probe = RepoPathAliasProbe::new(temp.path(), &missing);
        assert!(probe.identity.is_none());
        assert!(!probe.resolves_to(&present));

        // A component that does not resolve is equally conclusive, and must
        // not be mistaken for "exact spelling" by the directory walk.
        let under_missing_dir = RepoRelPath::from_normalized("absent/present.env".to_string());
        assert!(
            RepoPathAliasProbe::new(temp.path(), &under_missing_dir)
                .identity
                .is_none()
        );
    }

    #[test]
    fn parse_worktree_list_single() {
        let output = b"worktree /home/user/repo\0HEAD abc123\0branch refs/heads/main\0\0";
        let wts = parse_worktree_list(output).unwrap();
        assert_eq!(wts.len(), 1);
        assert_eq!(wts[0].path, PathBuf::from("/home/user/repo"));
        assert!(wts[0].is_main);
        assert!(!wts[0].is_bare);
    }

    #[test]
    fn parse_worktree_list_multiple() {
        let output = b"worktree /home/user/repo\0HEAD abc123\0branch refs/heads/main\0\0worktree /home/user/repo-wt\0HEAD abc123\0branch refs/heads/feature\0\0";
        let wts = parse_worktree_list(output).unwrap();
        assert_eq!(wts.len(), 2);
        assert!(wts[0].is_main);
        assert!(!wts[1].is_main);
        assert_eq!(wts[1].path, PathBuf::from("/home/user/repo-wt"));
    }

    #[cfg(unix)]
    #[test]
    fn parse_worktree_list_preserves_non_utf8_root_bytes() {
        use std::os::unix::ffi::OsStrExt;

        let output = b"worktree /tmp/repo-\xff\0HEAD abc123\0branch refs/heads/main\0\0";
        let wts = parse_worktree_list(output).unwrap();
        assert_eq!(
            wts[0].path.as_os_str().as_bytes(),
            b"/tmp/repo-\xff",
            "worktree discovery must not replace raw pathname bytes"
        );
    }

    #[test]
    fn parse_worktree_list_bare() {
        let output = b"worktree /home/user/repo.git\0bare\0\0";
        let wts = parse_worktree_list(output).unwrap();
        assert_eq!(wts.len(), 1);
        assert!(wts[0].is_bare);
    }

    #[test]
    fn parse_worktree_list_empty_fails() {
        let err = parse_worktree_list(b"").unwrap_err();
        assert!(err.to_string().contains("no worktrees"));
    }

    // ---- Tests using actual `git worktree list --porcelain -z` format ----
    // With -z, each attribute is NUL-terminated and the blank-line record
    // separator becomes a NUL (yielding double-NUL between records).

    #[test]
    fn parse_worktree_list_real_z_single() {
        // Real -z format: each field NUL-terminated, double-NUL at end of record
        let output = b"worktree /home/user/repo\0HEAD abc123\0branch refs/heads/main\0\0";
        let wts = parse_worktree_list(output).unwrap();
        assert_eq!(wts.len(), 1);
        assert_eq!(wts[0].path, PathBuf::from("/home/user/repo"));
        assert!(wts[0].is_main);
        assert!(!wts[0].is_bare);
    }

    #[test]
    fn parse_worktree_list_real_z_multiple() {
        // Two worktrees in real -z format
        let output = b"worktree /home/user/repo\0HEAD abc123\0branch refs/heads/main\0\0worktree /home/user/repo-wt\0HEAD def456\0branch refs/heads/feature\0\0";
        let wts = parse_worktree_list(output).unwrap();
        assert_eq!(wts.len(), 2);
        assert_eq!(wts[0].path, PathBuf::from("/home/user/repo"));
        assert!(wts[0].is_main);
        assert!(!wts[0].is_bare);
        assert_eq!(wts[1].path, PathBuf::from("/home/user/repo-wt"));
        assert!(!wts[1].is_main);
        assert!(!wts[1].is_bare);
    }

    #[test]
    fn parse_worktree_list_real_z_bare() {
        // Bare repo in real -z format — bare attribute is its own NUL-terminated field
        let output = b"worktree /home/user/repo.git\0bare\0\0";
        let wts = parse_worktree_list(output).unwrap();
        assert_eq!(wts.len(), 1);
        assert_eq!(wts[0].path, PathBuf::from("/home/user/repo.git"));
        assert!(wts[0].is_main);
        assert!(wts[0].is_bare);
    }

    #[test]
    fn parse_worktree_list_real_z_bare_with_linked() {
        // Bare main worktree + linked worktree
        let output = b"worktree /home/user/repo.git\0bare\0\0worktree /home/user/wt\0HEAD abc123\0branch refs/heads/feature\0\0";
        let wts = parse_worktree_list(output).unwrap();
        assert_eq!(wts.len(), 2);
        assert!(wts[0].is_bare);
        assert!(wts[0].is_main);
        assert!(!wts[1].is_bare);
        assert!(!wts[1].is_main);
    }

    #[test]
    fn parse_worktree_list_real_z_detached_head() {
        // Detached HEAD worktree (has HEAD and "detached" instead of "branch")
        let output = b"worktree /home/user/repo\0HEAD abc123\0branch refs/heads/main\0\0worktree /home/user/wt\0HEAD def456\0detached\0\0";
        let wts = parse_worktree_list(output).unwrap();
        assert_eq!(wts.len(), 2);
        assert_eq!(wts[1].path, PathBuf::from("/home/user/wt"));
    }

    #[test]
    fn parse_worktree_list_real_z_locked_and_prunable() {
        // Worktree with locked and prunable attributes (unknown fields are ignored)
        let output = b"worktree /home/user/repo\0HEAD abc123\0branch refs/heads/main\0\0worktree /home/user/wt\0HEAD def456\0branch refs/heads/feature\0locked\0prunable\0\0";
        let wts = parse_worktree_list(output).unwrap();
        assert_eq!(wts.len(), 2);
        assert_eq!(wts[1].path, PathBuf::from("/home/user/wt"));
        assert!(!wts[1].is_bare);
    }

    #[test]
    fn parse_worktree_list_real_z_path_with_spaces() {
        let output =
            b"worktree /home/user/my project/repo\0HEAD abc123\0branch refs/heads/main\0\0";
        let wts = parse_worktree_list(output).unwrap();
        assert_eq!(wts.len(), 1);
        assert_eq!(wts[0].path, PathBuf::from("/home/user/my project/repo"));
    }

    #[test]
    fn parse_worktree_list_real_z_no_trailing_double_nul() {
        // Handles output without trailing double-NUL (robustness)
        let output = b"worktree /home/user/repo\0HEAD abc123\0branch refs/heads/main";
        let wts = parse_worktree_list(output).unwrap();
        assert_eq!(wts.len(), 1);
        assert_eq!(wts[0].path, PathBuf::from("/home/user/repo"));
    }

    #[test]
    fn parse_check_ignore_matched() {
        // source\0linenum\0pattern\0pathname\0
        let output = b".gitignore\x005\x00*.log\x00debug.log\x00";
        let records = parse_check_ignore_output(output).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].path.as_str(), "debug.log");
        assert!(records[0].ignored);
        let info = records[0].match_info.as_ref().unwrap();
        assert_eq!(info.source_file, PathBuf::from(".gitignore"));
        assert_eq!(info.line, 5);
        assert_eq!(info.pattern, "*.log");
    }

    #[test]
    fn parse_check_ignore_non_matching() {
        // Empty source, linenum, pattern for non-matching path
        let output = b"\x00\x00\x00src/main.rs\x00";
        let records = parse_check_ignore_output(output).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].path.as_str(), "src/main.rs");
        assert!(!records[0].ignored);
        assert!(records[0].match_info.is_none());
    }

    #[test]
    fn parse_check_ignore_negation_is_not_ignored() {
        let output = b".gitignore\x002\x00!keep.env\x00keep.env\x00";
        let records = parse_check_ignore_output(output).unwrap();
        assert_eq!(records.len(), 1);
        assert!(!records[0].ignored);
        assert_eq!(records[0].match_info.as_ref().unwrap().pattern, "!keep.env");
    }

    #[test]
    fn parse_check_ignore_multiple() {
        let output = b".gitignore\x003\x00*.log\x00app.log\x00\x00\x00\x00README.md\x00";
        let records = parse_check_ignore_output(output).unwrap();
        assert_eq!(records.len(), 2);
        assert!(records[0].ignored);
        assert!(!records[1].ignored);
        assert!(records[0].match_info.is_some());
        assert!(records[1].match_info.is_none());
    }

    #[test]
    fn parse_check_ignore_empty() {
        let records = parse_check_ignore_output(b"").unwrap();
        assert!(records.is_empty());
    }

    // ---- nested-repo skip behavior for list_worktreeinclude_candidates ----

    fn isolated_git_command() -> std::process::Command {
        const GIT_ROUTING_ENV: &[&str] = &[
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_INDEX_FILE",
            "GIT_OBJECT_DIRECTORY",
            "GIT_ALTERNATE_OBJECT_DIRECTORIES",
            "GIT_CEILING_DIRECTORIES",
            "GIT_DISCOVERY_ACROSS_FILESYSTEM",
            "GIT_CONFIG",
            "GIT_CONFIG_PARAMETERS",
            "GIT_NAMESPACE",
            "GIT_PREFIX",
            "GIT_SHALLOW_FILE",
            "GIT_QUARANTINE_PATH",
            "GIT_LITERAL_PATHSPECS",
            "GIT_GLOB_PATHSPECS",
            "GIT_NOGLOB_PATHSPECS",
            "GIT_ICASE_PATHSPECS",
            "GIT_INDEX_VERSION",
            "GIT_DEFAULT_HASH",
            "GIT_DEFAULT_REF_FORMAT",
        ];

        static XDG_HOME: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
        let xdg_home = XDG_HOME
            .get_or_init(|| tempfile::TempDir::new().expect("isolated test config directory"));
        let mut command = std::process::Command::new("git");
        command
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env(
                "GIT_CONFIG_GLOBAL",
                if cfg!(windows) { "NUL" } else { "/dev/null" },
            )
            .env("GIT_CONFIG_COUNT", "0")
            .env("XDG_CONFIG_HOME", xdg_home.path());
        for key in GIT_ROUTING_ENV {
            command.env_remove(key);
        }
        command
    }

    fn run_git(dir: &Path, args: &[&str]) {
        let output = isolated_git_command()
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .expect("failed to spawn git");
        assert!(
            output.status.success(),
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn init_repo(dir: &Path) {
        run_git(dir, &["init"]);
        run_git(dir, &["config", "user.email", "test@test.com"]);
        run_git(dir, &["config", "user.name", "Test"]);
    }

    fn write_file(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, content).unwrap();
    }

    /// A subdirectory registered as a submodule (a gitlink entry in the
    /// index, mode 160000) must not be enumerated as a candidate source.
    /// `git ls-files --others --ignored` does not recurse into submodules
    /// without `--recurse-submodules`, and the v1 spec forbids it outright.
    #[test]
    fn list_candidates_skips_submodule_registered_as_gitlink() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();
        init_repo(root);

        write_file(&root.join(".gitignore"), "*.env\n");
        write_file(&root.join(".worktreeinclude"), "*.env\n");
        write_file(&root.join("top.env"), "top\n");

        // Build a minimal submodule-shaped layout: directory with a `.git`
        // file plus an index gitlink entry pointing at it. We use
        // `update-index --cacheinfo` to register the gitlink without needing
        // a fully-initialized second repository.
        let sub = root.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        write_file(&sub.join(".git"), "gitdir: ../.git/modules/sub\n");
        write_file(&sub.join("inner.env"), "inner\n");

        run_git(root, &["add", ".gitignore", ".worktreeinclude"]);
        run_git(
            root,
            &[
                "update-index",
                "--add",
                "--cacheinfo",
                "160000,1111111111111111111111111111111111111111,sub",
            ],
        );
        run_git(root, &["commit", "-m", "setup"]);

        let backend = GitGix::new();
        let candidates = backend
            .list_worktreeinclude_candidates(
                root,
                crate::config::WorktreeincludeSemantics::Git,
                crate::config::SymlinkPolicy::Follow,
            )
            .unwrap();
        let names: Vec<&str> = candidates.iter().map(|p| p.as_str()).collect();

        assert!(
            names.contains(&"top.env"),
            "expected top.env in candidates, got: {names:?}"
        );
        assert!(
            !names.iter().any(|n| n.starts_with("sub/")),
            "submodule contents must not be enumerated, got: {names:?}"
        );
    }

    /// A bare `.git` *file* alone (no gitlink in the index, no `.gitmodules`)
    /// is not a submodule from Git's perspective, and `git ls-files --others`
    /// does recurse into such directories. Match that CLI behavior.
    #[test]
    fn list_candidates_recurses_into_unregistered_dot_git_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();
        init_repo(root);

        write_file(&root.join(".gitignore"), "*.env\n");
        write_file(&root.join(".worktreeinclude"), "*.env\n");
        write_file(&root.join("top.env"), "top\n");

        let sub = root.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        write_file(&sub.join(".git"), "gitdir: /nonexistent\n");
        write_file(&sub.join("inner.env"), "inner\n");

        run_git(root, &["add", ".gitignore", ".worktreeinclude"]);
        run_git(root, &["commit", "-m", "setup"]);

        let backend = GitGix::new();
        let candidates = backend
            .list_worktreeinclude_candidates(
                root,
                crate::config::WorktreeincludeSemantics::Git,
                crate::config::SymlinkPolicy::Follow,
            )
            .unwrap();
        let names: Vec<&str> = candidates.iter().map(|p| p.as_str()).collect();

        assert!(
            names.contains(&"sub/inner.env"),
            "expected sub/inner.env to be enumerated (not a registered \
             submodule), got: {names:?}"
        );
    }

    /// A nested independent Git checkout (its own `.git` *directory*) must
    /// also be skipped — same reasoning as submodules: contents belong to a
    /// different repository and copying them would leak files.
    #[test]
    fn list_candidates_skips_nested_repo_with_dot_git_dir() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();
        init_repo(root);

        write_file(&root.join(".gitignore"), "*.env\n");
        write_file(&root.join(".worktreeinclude"), "*.env\n");
        write_file(&root.join("top.env"), "top\n");

        let nested = root.join("nested");
        std::fs::create_dir_all(&nested).unwrap();
        init_repo(&nested);
        write_file(&nested.join("inner.env"), "inner\n");

        run_git(root, &["add", ".gitignore", ".worktreeinclude"]);
        run_git(root, &["commit", "-m", "setup"]);

        let backend = GitGix::new();
        let candidates = backend
            .list_worktreeinclude_candidates(
                root,
                crate::config::WorktreeincludeSemantics::Git,
                crate::config::SymlinkPolicy::Follow,
            )
            .unwrap();
        let names: Vec<&str> = candidates.iter().map(|p| p.as_str()).collect();

        assert!(
            names.contains(&"top.env"),
            "expected top.env in candidates, got: {names:?}"
        );
        assert!(
            !names.iter().any(|n| n.starts_with("nested/")),
            "nested-repo contents must not be enumerated, got: {names:?}"
        );
    }

    /// Sanity check: the skip logic does not over-fire on normal nested
    /// directories (no `.git` marker inside).
    #[test]
    fn list_candidates_recurses_into_normal_subdirs() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();
        init_repo(root);

        write_file(&root.join(".gitignore"), "*.env\n");
        write_file(&root.join(".worktreeinclude"), "*.env\n");
        write_file(&root.join("config/dev.env"), "dev\n");

        run_git(root, &["add", ".gitignore", ".worktreeinclude"]);
        run_git(root, &["commit", "-m", "setup"]);

        let backend = GitGix::new();
        let candidates = backend
            .list_worktreeinclude_candidates(
                root,
                crate::config::WorktreeincludeSemantics::Git,
                crate::config::SymlinkPolicy::Follow,
            )
            .unwrap();
        let names: Vec<&str> = candidates.iter().map(|p| p.as_str()).collect();

        assert!(
            names.contains(&"config/dev.env"),
            "expected config/dev.env in candidates, got: {names:?}"
        );
    }
}
