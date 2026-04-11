//! Git backend trait and CLI implementation.
//!
//! This module is the **only** place in wiff that shells out to `git`.
//! All Git interactions go through the [`GitBackend`] trait, allowing
//! the planner and other modules to be tested without real Git repos.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

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

    /// Batch-check ignore status for the given paths.
    fn check_ignore(
        &self,
        source_root: &Path,
        paths: &[RepoRelPath],
    ) -> Result<Vec<IgnoreCheckRecord>>;

    /// List files that match `.worktreeinclude` patterns (candidates for copy).
    fn list_worktreeinclude_candidates(&self, source_root: &Path) -> Result<Vec<RepoRelPath>>;

    /// Read a boolean Git config value.
    fn read_bool_config(&self, source_root: &Path, key: &str) -> Result<bool>;

    /// Read a Git config value as a string. Returns `None` if the key is unset.
    fn read_config(&self, source_root: &Path, key: &str) -> Result<Option<String>>;
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

impl GitBackend for GitCli {
    fn show_toplevel(&self, path: &Path) -> Result<PathBuf> {
        let output = self.run_git(path, &["rev-parse", "--show-toplevel"])?;
        let s = String::from_utf8_lossy(&output);
        Ok(PathBuf::from(s.trim_end_matches(['\n', '\r'])))
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

    fn list_worktreeinclude_candidates(&self, source_root: &Path) -> Result<Vec<RepoRelPath>> {
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

/// Parse the output of `git worktree list --porcelain -z`.
///
/// The format uses NUL as the record separator and newlines within records.
/// Each record starts with `worktree <path>` and may include `bare` or
/// `HEAD`, `branch`, etc. The first record is always the main worktree.
fn parse_worktree_list(output: &[u8]) -> Result<Vec<WorktreeRecord>> {
    let text = String::from_utf8_lossy(output);
    let mut worktrees = Vec::new();

    // Split on NUL for porcelain -z format.
    // Each record is a block of lines separated by NUL.
    let records: Vec<&str> = text.split('\0').collect();

    for record in &records {
        let record = record.trim();
        if record.is_empty() {
            continue;
        }

        let mut path: Option<PathBuf> = None;
        let mut is_bare = false;

        for line in record.lines() {
            let line = line.trim();
            if let Some(p) = line.strip_prefix("worktree ") {
                path = Some(PathBuf::from(p));
            } else if line == "bare" {
                is_bare = true;
            }
        }

        if let Some(path) = path {
            let is_main = worktrees.is_empty(); // first is main
            worktrees.push(WorktreeRecord {
                path,
                is_main,
                is_bare,
            });
        }
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
        let output = b"worktree /home/user/repo\nHEAD abc123\nbranch refs/heads/main\n\0";
        let wts = parse_worktree_list(output).unwrap();
        assert_eq!(wts.len(), 1);
        assert_eq!(wts[0].path, PathBuf::from("/home/user/repo"));
        assert!(wts[0].is_main);
        assert!(!wts[0].is_bare);
    }

    #[test]
    fn parse_worktree_list_multiple() {
        let output = b"worktree /home/user/repo\nHEAD abc123\nbranch refs/heads/main\n\0worktree /home/user/repo-wt\nHEAD abc123\nbranch refs/heads/feature\n\0";
        let wts = parse_worktree_list(output).unwrap();
        assert_eq!(wts.len(), 2);
        assert!(wts[0].is_main);
        assert!(!wts[1].is_main);
        assert_eq!(wts[1].path, PathBuf::from("/home/user/repo-wt"));
    }

    #[test]
    fn parse_worktree_list_bare() {
        let output = b"worktree /home/user/repo.git\nbare\n\0";
        let wts = parse_worktree_list(output).unwrap();
        assert_eq!(wts.len(), 1);
        assert!(wts[0].is_bare);
    }

    #[test]
    fn parse_worktree_list_empty_fails() {
        let err = parse_worktree_list(b"").unwrap_err();
        assert!(err.to_string().contains("no worktrees"));
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
}
