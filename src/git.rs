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

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use gix::bstr::ByteSlice;

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
    /// If the path matched an ignore rule, details about the match.
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

impl GitBackend for GitCli {
    fn show_toplevel(&self, path: &Path) -> Result<PathBuf> {
        let output = self.run_git(path, &["rev-parse", "--show-toplevel"])?;
        let s = String::from_utf8_lossy(&output);
        let raw = PathBuf::from(s.trim_end_matches(['\n', '\r']));
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

        let mut args: Vec<&str> = vec!["ls-files", "--cached", "--full-name", "-z", "--"];
        let path_strings: Vec<&str> = paths.iter().map(|p| p.as_str()).collect();
        args.extend(path_strings.iter());

        let output = self.run_git(source_root, &args)?;
        let mut result = HashSet::new();
        for entry in output.split(|&b| b == 0) {
            if entry.is_empty() {
                continue;
            }
            let s = String::from_utf8_lossy(entry);
            result.insert(RepoRelPath::from_normalized(s.into_owned()));
        }
        Ok(result)
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
        // The CLI fast path uses `git ls-files --exclude-per-directory`,
        // which is hard-wired to Git's per-directory exclude semantics. As
        // long as the requested semantics engine produces the same result
        // for this fixture (true in PR6 since Claude202604 and Wt039
        // delegate to GitSemantics), the fast path is valid. PR7 / PR8
        // will route divergent engines through the walk path below.
        let semantics_matches_git_cli = matches!(
            semantics,
            WorktreeincludeSemantics::Git
                | WorktreeincludeSemantics::Claude202604
                | WorktreeincludeSemantics::Wt039
        );

        // Under Ignore policy, symlinked .worktreeinclude files must NOT
        // contribute patterns. Git CLI's `--exclude-per-directory` follows
        // symlinks unconditionally, so fall through to a walkdir + matcher
        // path when Ignore is requested.
        if symlink_policy == SymlinkPolicy::Ignore || !semantics_matches_git_cli {
            return cli_list_candidates_skipping_symlinked_rules(self, source_root, semantics);
        }

        let output = self.run_git(
            source_root,
            &[
                "ls-files",
                "--others",
                "--ignored",
                "--exclude-per-directory=.worktreeinclude",
                "--full-name",
                "-z",
            ],
        )?;

        let mut result = Vec::new();
        for entry in output.split(|&b| b == 0) {
            let s = String::from_utf8_lossy(entry);
            let s = s.trim();
            if !s.is_empty() {
                result.push(RepoRelPath::from_normalized(s.to_string()));
            }
        }
        Ok(result)
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
            let s = String::from_utf8_lossy(entry);
            let s = s.trim();
            if !s.is_empty() {
                result.push(RepoRelPath::from_normalized(s.to_string()));
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
        for path in paths {
            let rela = path.as_str().as_bytes().as_bstr();
            if index.entry_by_path(rela).is_some() {
                tracked.insert(path.clone());
            }
        }

        Ok(tracked)
    }

    fn gitlinks(&self, source_root: &Path) -> Result<HashSet<String>> {
        let repo = self.discover_repo(source_root)?;
        let index = repo.index_or_empty().map_err(|e| Error::Git {
            message: format!(
                "gix failed to read index for {}: {e}",
                source_root.display()
            ),
        })?;

        Ok(index
            .entries()
            .iter()
            .filter(|e| e.mode == gix::index::entry::Mode::COMMIT)
            .map(|e| e.path(&index).to_str_lossy().into_owned())
            .collect())
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
            let match_info = if tracked.contains(path) {
                None
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

                platform
                    .matching_exclude_pattern()
                    .map(|m| IgnoreMatchInfo {
                        source_file: m
                            .source
                            .map(|p| Self::normalize_ignore_source(p, source_root))
                            .unwrap_or_default(),
                        line: m.sequence_number,
                        pattern: m.pattern.to_string(),
                    })
            };

            records.push(IgnoreCheckRecord {
                path: path.clone(),
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

        // Submodules registered with `git submodule add` are stored in the
        // index as entries with mode 160000 (gitlink). `git ls-files` skips
        // these when walking the worktree, and so must we.
        let gitlinks: HashSet<String> = index
            .entries()
            .iter()
            .filter(|e| e.mode == gix::index::entry::Mode::COMMIT)
            .map(|e| e.path(&index).to_str_lossy().into_owned())
            .collect();

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

            let rel = match RepoRelPath::normalize(entry.path(), source_root) {
                Ok(p) => p,
                Err(_) => continue,
            };

            let rela_bstr = rel.as_str().as_bytes().as_bstr();
            if index.entry_by_path(rela_bstr).is_some() {
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

        let gitlinks: HashSet<String> = index
            .entries()
            .iter()
            .filter(|e| e.mode == gix::index::entry::Mode::COMMIT)
            .map(|e| e.path(&index).to_str_lossy().into_owned())
            .collect();

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

            let rel = match RepoRelPath::normalize(entry.path(), source_root) {
                Ok(p) => p,
                Err(_) => continue,
            };

            let rela_bstr = rel.as_str().as_bytes().as_bstr();
            if index.entry_by_path(rela_bstr).is_some() {
                continue;
            }

            let abs = rel.to_path(source_root);
            let mode = if abs.is_dir() {
                Some(gix::index::entry::Mode::DIR)
            } else {
                None
            };
            let platform = excludes
                .at_path(Path::new(rel.as_str()), mode)
                .map_err(|e| Error::Io {
                    context: format!("matching ignore patterns for {}", rel.as_str()),
                    source: e,
                })?;

            if platform.matching_exclude_pattern().is_some() {
                result.push(rel);
            }
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
        let gitlinks: HashSet<String> = index
            .entries()
            .iter()
            .filter(|e| e.mode == gix::index::entry::Mode::COMMIT)
            .map(|e| e.path(&index).to_str_lossy().into_owned())
            .collect();
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
}

/// Read the set of gitlink paths (mode 160000) from the index using the Git
/// CLI. Used by the [`GitCli`] backend when it needs the same submodule
/// boundary information that the gix backend gets from its in-process index.
fn read_gitlinks_via_cli(cli: &GitCli, source_root: &Path) -> Result<HashSet<String>> {
    // `git ls-files -s -z` emits one entry per line in the form
    // `<mode> <hash> <stage>\t<path>` with NUL separators.
    let output = match cli.run_git(source_root, &["ls-files", "-s", "-z"]) {
        Ok(out) => out,
        Err(_) => return Ok(HashSet::new()),
    };
    let mut links = HashSet::new();
    for record in output.split(|&b| b == 0) {
        if record.is_empty() {
            continue;
        }
        let s = String::from_utf8_lossy(record);
        // Format: "160000 <hash> <stage>\t<path>"
        let mut parts = s.splitn(2, '\t');
        let header = parts.next().unwrap_or("");
        let path = parts.next().unwrap_or("").to_string();
        if header.starts_with("160000 ") {
            links.insert(path);
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
        if entry.file_name() != ".worktreeinclude" {
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

/// CLI-backend candidate enumeration that mirrors the gix walker.
///
/// Used when the CLI fast path (`git ls-files --exclude-per-directory`) is
/// not safe — either because the requested `SymlinkPolicy::Ignore` requires
/// hiding symlinked rule files or because `semantics` selects an engine
/// whose output diverges from Git's per-directory rules.
fn cli_list_candidates_skipping_symlinked_rules(
    cli: &GitCli,
    source_root: &Path,
    semantics: WorktreeincludeSemantics,
) -> Result<Vec<RepoRelPath>> {
    let gitlinks = read_gitlinks_via_cli(cli, source_root)?;
    let ignore_case = cli
        .read_bool_config(source_root, "core.ignoreCase")
        .unwrap_or(false);

    // Tracked paths must be excluded from candidates, the same as the index
    // check used by the gix backend. Use `git ls-files --cached` for a
    // single CLI invocation rather than per-path checks.
    let cached = cli.run_git(source_root, &["ls-files", "--cached", "-z"])?;
    let tracked: HashSet<String> = cached
        .split(|&b| b == 0)
        .filter(|e| !e.is_empty())
        .map(|e| String::from_utf8_lossy(e).into_owned())
        .collect();

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
        let rel = match RepoRelPath::normalize(entry.path(), source_root) {
            Ok(p) => p,
            Err(_) => continue,
        };
        if tracked.contains(rel.as_str()) {
            continue;
        }

        let selected = matches!(
            engine.evaluate(
                source_root,
                rel.as_str(),
                false,
                ignore_case,
                SymlinkPolicy::Ignore,
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
    let text = String::from_utf8_lossy(output);
    let mut worktrees = Vec::new();

    let mut current_path: Option<PathBuf> = None;
    let mut current_is_bare = false;

    for field in text.split('\0') {
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

        if let Some(p) = field.strip_prefix("worktree ") {
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
            current_path = Some(PathBuf::from(p));
        } else if field == "bare" {
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
        let pathname = String::from_utf8_lossy(fields[i + 3]).to_string();

        let path = RepoRelPath::from_normalized(pathname);

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

        records.push(IgnoreCheckRecord { path, match_info });
        i += 4;
    }

    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(records[0].match_info.is_none());
    }

    #[test]
    fn parse_check_ignore_multiple() {
        let output = b".gitignore\x003\x00*.log\x00app.log\x00\x00\x00\x00README.md\x00";
        let records = parse_check_ignore_output(output).unwrap();
        assert_eq!(records.len(), 2);
        assert!(records[0].match_info.is_some());
        assert!(records[1].match_info.is_none());
    }

    #[test]
    fn parse_check_ignore_empty() {
        let records = parse_check_ignore_output(b"").unwrap();
        assert!(records.is_empty());
    }

    // ---- nested-repo skip behavior for list_worktreeinclude_candidates ----

    fn run_git(dir: &Path, args: &[&str]) {
        let output = std::process::Command::new("git")
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
