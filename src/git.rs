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

    /// Whether this backend reads Git's ambient default global excludes file.
    ///
    /// Real backends return true. In-memory test/specialized backends default
    /// to false so validation does not unexpectedly consult the host account.
    fn reads_default_global_excludes(&self) -> bool {
        false
    }
}

/// Create the configured Git backend.
///
/// Uses the in-process `gix` backend by default.
/// Set `WAFT_GIT_BACKEND=cli` to use the Git CLI backend as a fallback.
pub fn default_git_backend() -> Box<dyn GitBackend> {
    if std::env::var("WAFT_GIT_BACKEND").as_deref() == Ok("cli") {
        Box::new(GitCli::new())
    } else {
        Box::new(GitGix::new())
    }
}

/// Git backend that shells out to the `git` CLI.
#[derive(Debug, Default)]
pub struct GitCli;

impl GitCli {
    /// Create a new `GitCli` backend.
    pub fn new() -> Self {
        Self
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
pub struct GitGix;

impl GitGix {
    /// Create a new `GitGix` backend.
    pub fn new() -> Self {
        Self
    }

    fn discover_repo(&self, path: &Path) -> Result<gix::Repository> {
        gix::discover(path).map_err(|e| Error::Git {
            message: format!(
                "gix failed to discover repository from {}: {e}",
                path.display()
            ),
        })
    }

    fn normalize_ignore_source(path: &Path, source_root: &Path) -> PathBuf {
        path.strip_prefix(source_root)
            .map(Path::to_path_buf)
            .unwrap_or_else(|_| path.to_path_buf())
    }
}

/// Canonicalize a repo-root path and strip the Windows `\\?\` UNC prefix
/// so both backends produce paths in the same form (critical for
/// `strip_prefix` and display parity between backends).
fn normalize_repo_path(path: &Path) -> PathBuf {
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    strip_unc_prefix(&canonical)
}

#[cfg(windows)]
fn strip_unc_prefix(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        if let Some(unc_rest) = rest.strip_prefix(r"UNC\") {
            PathBuf::from(format!(r"\\{unc_rest}"))
        } else {
            PathBuf::from(rest)
        }
    } else {
        path.to_path_buf()
    }
}

#[cfg(not(windows))]
fn strip_unc_prefix(path: &Path) -> PathBuf {
    path.to_path_buf()
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
        let output = self.run_git(path, &["rev-parse", "--show-toplevel"])?;
        let path_bytes = trim_git_line_ending(&output);
        let raw = path_buf_from_git_bytes(path_bytes, "repository root")?;
        Ok(normalize_repo_path(&raw))
    }

    fn list_worktrees(&self, source_root: &Path) -> Result<Vec<WorktreeRecord>> {
        let output = self.run_git(source_root, &["worktree", "list", "--porcelain", "-z"])?;
        parse_worktree_list(&output)
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
        let output = self.run_git(source_root, &["ls-files", "--cached", "--full-name", "-z"])?;
        let ignore_case = self.read_bool_config(source_root, "core.ignoreCase")?;
        let index_paths = TrackedPathLookup::new(
            output.split(|&b| b == 0).filter(|entry| !entry.is_empty()),
            ignore_case,
        );
        let mut result = HashSet::new();
        for path in paths {
            if index_paths.contains(source_root, path.as_str().as_bytes()) {
                result.insert(path.clone());
            }
        }
        Ok(result)
    }

    fn index_path(&self, source_root: &Path) -> Result<Option<PathBuf>> {
        let output = self.run_git(source_root, &["rev-parse", "--git-path", "index"])?;
        let path_bytes = trim_git_line_ending(&output);
        let raw = path_buf_from_git_bytes(path_bytes, "Git index path")?;
        let path = if raw.is_absolute() {
            raw
        } else {
            source_root.join(raw)
        };
        Ok(Some(path))
    }

    fn gitlinks(&self, source_root: &Path) -> Result<HashSet<String>> {
        read_gitlinks_via_cli(self, source_root)
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
        let gitlinks = read_gitlinks_via_cli(self, source_root)?;
        Ok(walk_for_first_worktreeinclude(
            source_root,
            &gitlinks,
            symlink_policy,
        ))
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

        let repo = self.discover_repo(source_root)?;
        let index = repo.index_or_empty().map_err(|e| Error::Git {
            message: format!(
                "gix failed to read index for {}: {e}",
                source_root.display()
            ),
        })?;

        let mut tracked = HashSet::new();
        let ignore_case = repo
            .config_snapshot()
            .boolean("core.ignoreCase")
            .unwrap_or(false);
        let index_paths = TrackedPathLookup::new(
            index
                .entries()
                .iter()
                .map(|entry| entry.path(&index).as_ref()),
            ignore_case,
        );
        for path in paths {
            if index_paths.contains(source_root, path.as_str().as_bytes()) {
                tracked.insert(path.clone());
            }
        }

        Ok(tracked)
    }

    fn index_path(&self, source_root: &Path) -> Result<Option<PathBuf>> {
        Ok(Some(self.discover_repo(source_root)?.index_path()))
    }

    fn gitlinks(&self, source_root: &Path) -> Result<HashSet<String>> {
        let repo = self.discover_repo(source_root)?;
        let index = repo.index_or_empty().map_err(|e| Error::Git {
            message: format!(
                "gix failed to read index for {}: {e}",
                source_root.display()
            ),
        })?;

        gitlinks_from_gix_index(&index)
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
        let repo = self.discover_repo(source_root)?;
        let index = repo.index_or_empty().map_err(|e| Error::Git {
            message: format!(
                "gix failed to read index for {}: {e}",
                source_root.display()
            ),
        })?;
        let ignore_case = repo
            .config_snapshot()
            .boolean("core.ignoreCase")
            .unwrap_or(false);
        let tracked_paths = TrackedPathLookup::new(
            index
                .entries()
                .iter()
                .map(|entry| entry.path(&index).as_ref()),
            ignore_case,
        );

        // Submodules registered with `git submodule add` are stored in the
        // index as entries with mode 160000 (gitlink). `git ls-files` skips
        // these when walking the worktree, and so must we.
        let gitlinks = gitlinks_from_gix_index(&index)?;

        let engine = crate::worktreeinclude_engine::engine_for(semantics);
        let mut candidates = Vec::new();
        for entry in walkdir::WalkDir::new(source_root)
            .into_iter()
            .filter_entry(|e| !is_nested_git_boundary(e, source_root, &gitlinks))
        {
            let entry = entry.map_err(|e| Error::Git {
                message: format!("failed walking {}: {e}", source_root.display()),
            })?;

            if entry.file_type().is_dir() {
                continue;
            }

            let rel = RepoRelPath::normalize(entry.path(), source_root)?;

            if tracked_paths.contains(source_root, rel.as_str().as_bytes()) {
                continue;
            }

            let selected = matches!(
                engine.evaluate(
                    source_root,
                    rel.as_str(),
                    false,
                    ignore_case,
                    symlink_policy
                ),
                crate::model::WorktreeincludeStatus::Included { .. }
            );
            if selected {
                candidates.push(rel);
            }
        }

        candidates.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        Ok(candidates)
    }

    fn list_ignored_untracked(&self, source_root: &Path) -> Result<Vec<RepoRelPath>> {
        let repo = self.discover_repo(source_root)?;
        let index = repo.index_or_empty().map_err(|e| Error::Git {
            message: format!(
                "gix failed to read index for {}: {e}",
                source_root.display()
            ),
        })?;
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

        let gitlinks = gitlinks_from_gix_index(&index)?;
        let ignore_case = repo
            .config_snapshot()
            .boolean("core.ignoreCase")
            .unwrap_or(false);
        let tracked_paths = TrackedPathLookup::new(
            index
                .entries()
                .iter()
                .map(|entry| entry.path(&index).as_ref()),
            ignore_case,
        );

        let mut result = Vec::new();
        for entry in walkdir::WalkDir::new(source_root)
            .into_iter()
            .filter_entry(|e| !is_nested_git_boundary(e, source_root, &gitlinks))
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
            let rel_path =
                entry
                    .path()
                    .strip_prefix(source_root)
                    .map_err(|error| Error::InvalidPath {
                        message: format!(
                            "{} is outside repository root {}: {error}",
                            entry.path().display(),
                            source_root.display()
                        ),
                    })?;
            let platform = excludes.at_path(rel_path, None).map_err(|e| Error::Io {
                context: format!("matching ignore patterns for {}", entry.path().display()),
                source: e,
            })?;

            if !platform.is_excluded() {
                continue;
            }

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
        let repo = self.discover_repo(source_root)?;
        let index = repo.index_or_empty().map_err(|e| Error::Git {
            message: format!(
                "gix failed to read index for {}: {e}",
                source_root.display()
            ),
        })?;
        let gitlinks = gitlinks_from_gix_index(&index)?;
        Ok(walk_for_first_worktreeinclude(
            source_root,
            &gitlinks,
            symlink_policy,
        ))
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
                let trimmed = String::from_utf8_lossy(v.as_ref().as_ref())
                    .trim()
                    .to_string();
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

/// Read the set of gitlink paths (mode 160000) from the index using the Git
/// CLI. Used by the [`GitCli`] backend when it needs the same submodule
/// boundary information that the gix backend gets from its in-process index.
fn read_gitlinks_via_cli(cli: &GitCli, source_root: &Path) -> Result<HashSet<String>> {
    // `git ls-files -s -z` emits one entry per line in the form
    // `<mode> <hash> <stage>\t<path>` with NUL separators.
    let output = cli.run_git(source_root, &["ls-files", "-s", "-z"])?;
    let mut links = HashSet::new();
    for record in output.split(|&b| b == 0) {
        if record.is_empty() {
            continue;
        }
        // Format: "160000 <hash> <stage>\t<path>"
        let Some(tab) = record.iter().position(|byte| *byte == b'\t') else {
            continue;
        };
        if record[..tab].starts_with(b"160000 ") {
            let path = RepoRelPath::from_git_bytes(&record[tab + 1..])?;
            links.insert(path.as_str().to_string());
        }
    }
    Ok(links)
}

/// Walk the source tree looking for the first `.worktreeinclude` file,
/// skipping nested git checkouts/submodules.
///
/// Pure filesystem walk; the only Git-specific input is the gitlinks set.
/// Under `SymlinkPolicy::Ignore`, symlinked rule files do not count toward
/// existence (consistent with their being treated as absent during
/// selection).
fn walk_for_first_worktreeinclude(
    source_root: &Path,
    gitlinks: &HashSet<String>,
    symlink_policy: SymlinkPolicy,
) -> bool {
    for entry in walkdir::WalkDir::new(source_root)
        .into_iter()
        .filter_entry(|e| !is_nested_git_boundary(e, source_root, gitlinks))
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
            return true;
        }
        if entry.file_type().is_file() {
            return true;
        }
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
    let gitlinks = read_gitlinks_via_cli(cli, source_root)?;
    let ignore_case = cli
        .read_bool_config(source_root, "core.ignoreCase")
        .unwrap_or(false);

    // Tracked paths must be excluded from candidates, the same as the index
    // check used by the gix backend. Use `git ls-files --cached` for a
    // single CLI invocation rather than per-path checks.
    let cached = cli.run_git(source_root, &["ls-files", "--cached", "-z"])?;
    let tracked = TrackedPathLookup::new(
        cached.split(|&b| b == 0).filter(|entry| !entry.is_empty()),
        ignore_case,
    );

    let engine = crate::worktreeinclude_engine::engine_for(semantics);
    let mut candidates = Vec::new();
    for entry in walkdir::WalkDir::new(source_root)
        .into_iter()
        .filter_entry(|e| !is_nested_git_boundary(e, source_root, &gitlinks))
    {
        let entry = entry.map_err(|e| Error::Git {
            message: format!("failed walking {}: {e}", source_root.display()),
        })?;
        if entry.file_type().is_dir() {
            continue;
        }
        let rel = RepoRelPath::normalize(entry.path(), source_root)?;
        if tracked.contains(source_root, rel.as_str().as_bytes()) {
            continue;
        }

        let selected = matches!(
            engine.evaluate(
                source_root,
                rel.as_str(),
                false,
                ignore_case,
                symlink_policy,
            ),
            crate::model::WorktreeincludeStatus::Included { .. }
        );
        if selected {
            candidates.push(rel);
        }
    }

    candidates.sort_by(|a, b| a.as_str().cmp(b.as_str()));
    Ok(candidates)
}

/// Compare normalized repository paths using the same case semantics as
/// tracked-path protection.
///
/// This is crate-visible so other repository-boundary checks can avoid
/// drifting back to ASCII-only comparisons.
pub(crate) fn repo_paths_equivalent(left: &str, right: &str, ignore_case: bool) -> bool {
    repo_path_bytes_equal(left.as_bytes(), right.as_bytes(), ignore_case)
}

/// Return true when `alias` is a non-exact spelling that the filesystem
/// resolves to the same entry as `canonical`.
///
/// Requiring at least one missing exact directory entry distinguishes a
/// case/normalization alias from two explicitly named hard links.
pub(crate) fn repo_paths_alias_on_filesystem(
    source_root: &Path,
    canonical: &RepoRelPath,
    alias: &RepoRelPath,
) -> bool {
    if canonical == alias || repo_path_has_exact_spelling(source_root, alias) {
        return false;
    }

    let canonical_info = repo_path_filesystem_info(source_root, canonical.as_str().as_bytes());
    let alias_info = repo_path_filesystem_info(source_root, alias.as_str().as_bytes());
    canonical_info
        .zip(alias_info)
        .is_some_and(|(canonical, alias)| canonical.identity == alias.identity)
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

/// Precomputed tracked-name lookup with lazy filesystem-identity checks.
///
/// Exact and protected case-folded queries are constant-time. On platforms
/// where folding is not automatically protective, only tracked names in the
/// query's folded-name bucket are opened to detect a filesystem alias.
/// macOS/Windows native aliases and Unix hard-linked queries may require a
/// wider identity scan.
struct TrackedPathLookup {
    exact: HashSet<Vec<u8>>,
    protected_folded: Option<HashSet<CaseFoldedRepoPath>>,
    always_folded: HashMap<CaseFoldedRepoPath, Vec<Vec<u8>>>,
}

impl TrackedPathLookup {
    fn new<'a>(paths: impl IntoIterator<Item = &'a [u8]>, ignore_case: bool) -> Self {
        let mut exact = HashSet::new();
        // A false core.ignoreCase cannot prove that a macOS volume or Windows
        // directory is case-sensitive, especially when both spellings are
        // currently absent and there is no filesystem identity to compare.
        // Prefer a conservative skip on those platforms. Linux keeps Git's
        // configured semantics and uses the lazy identity checks below.
        let protect_folded_names = ignore_case || cfg!(target_os = "macos") || cfg!(windows);
        let mut protected_folded = protect_folded_names.then(HashSet::new);
        let mut always_folded: HashMap<CaseFoldedRepoPath, Vec<Vec<u8>>> = HashMap::new();

        for path in paths {
            exact.insert(path.to_vec());
            let folded_path = case_folded_repo_path(path);
            if let Some(protected_folded) = &mut protected_folded {
                protected_folded.insert(folded_path.clone());
            }
            always_folded
                .entry(folded_path)
                .or_default()
                .push(path.to_vec());
        }

        Self {
            exact,
            protected_folded,
            always_folded,
        }
    }

    fn contains(&self, source_root: &Path, query: &[u8]) -> bool {
        if self.exact.contains(query) {
            return true;
        }

        let folded_query = case_folded_repo_path(query);
        if self
            .protected_folded
            .as_ref()
            .is_some_and(|folded| folded.contains(&folded_query))
        {
            return true;
        }

        let Some(query_info) = repo_path_filesystem_info(source_root, query) else {
            return false;
        };

        if let Some(bucket) = self.always_folded.get(&folded_query)
            && bucket.iter().any(|path| {
                repo_path_filesystem_info(source_root, path)
                    .is_some_and(|info| info.identity == query_info.identity)
            })
        {
            return true;
        }

        // A filesystem's native caseless comparison can be broader than the
        // Unicode version compiled into this binary. On macOS and Windows,
        // compare identities across every folded bucket when the query
        // resolves; on other Unix systems the wider scan is needed for hard
        // links.
        (cfg!(target_os = "macos") || cfg!(windows) || query_info.hard_link_count > 1)
            && self
                .always_folded
                .iter()
                .filter(|(folded, _)| *folded != &folded_query)
                .flat_map(|(_, paths)| paths)
                .any(|path| {
                    repo_path_filesystem_info(source_root, path)
                        .is_some_and(|info| info.identity == query_info.identity)
                })
    }
}

fn repo_path_filesystem_info(source_root: &Path, path: &[u8]) -> Option<FilesystemInfo> {
    let path = repo_bytes_to_path(source_root, path)?;
    filesystem_info(&path)
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

#[cfg(unix)]
type FilesystemIdentity = (u64, u64);

#[cfg(unix)]
fn filesystem_info(path: &Path) -> Option<FilesystemInfo> {
    use std::os::unix::fs::MetadataExt;
    let metadata = std::fs::symlink_metadata(path).ok()?;
    Some(FilesystemInfo {
        identity: (metadata.dev(), metadata.ino()),
        hard_link_count: metadata.nlink(),
    })
}

#[cfg(windows)]
type FilesystemIdentity = (u64, u64);

#[cfg(windows)]
fn filesystem_info(path: &Path) -> Option<FilesystemInfo> {
    let handle = winapi_util::Handle::from_path_any(path).ok()?;
    let info = winapi_util::file::information(&handle).ok()?;
    Some(FilesystemInfo {
        identity: (info.volume_serial_number(), info.file_index()),
        hard_link_count: info.number_of_links(),
    })
}

#[cfg(not(any(unix, windows)))]
type FilesystemIdentity = ();

#[cfg(not(any(unix, windows)))]
fn filesystem_info(_path: &Path) -> Option<FilesystemInfo> {
    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FilesystemInfo {
    identity: FilesystemIdentity,
    hard_link_count: u64,
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
) -> bool {
    if !entry.file_type().is_dir() {
        return false;
    }
    crate::walk::is_git_boundary_dir(entry.path(), entry.depth(), source_root, gitlinks)
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
        assert!(repo_paths_alias_on_filesystem(
            temp.path(),
            &canonical,
            &alias
        ));
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
        assert!(!repo_paths_alias_on_filesystem(
            temp.path(),
            &first,
            &second
        ));
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
