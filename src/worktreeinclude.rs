//! Per-directory `.worktreeinclude` matcher used by the `git` semantics
//! engine and by the explanation paths in `info` / `list -v`.
//!
//! This module evaluates paths against `.worktreeinclude` files using
//! Git-style per-directory exclude semantics. It is **not** the
//! authoritative selector for *which files actually get copied* — the Git
//! backend's `check_ignore` is the final authority on git-ignored
//! membership, and which engine in [`crate::worktreeinclude_engine`] is
//! consulted depends on the active compat profile.
//!
//! The engine collects applicable `.worktreeinclude` files from the repo
//! root to the queried path's parent, builds a matcher for each file
//! rooted at its directory, and evaluates with last-match-wins semantics
//! (shallow to deep, last match within each file). The `claude-2026-04`
//! engine consumes [`evaluate_root_only`] instead, which inspects only the
//! repository's root-level rule file.

use std::fs;
use std::path::{Path, PathBuf};

use ignore::Match;
use ignore::gitignore::{Gitignore, GitignoreBuilder, Glob};
use waft_verified::rules::lines;
use waft_verified::rules::precedence::{self, MatchKind, RuleMatchOracle, Verdict};
use waft_verified::selection::{self, RuleFileProbe};

use crate::config::SymlinkPolicy;
use crate::model::WorktreeincludeStatus;

/// Compiled context for a single `.worktreeinclude` file: its matcher, line
/// provenance data, and source path.
struct FileMatchContext {
    file_path: PathBuf,
    matcher: Gitignore,
    line_info: Vec<(usize, String)>,
}

/// Build a [`FileMatchContext`] from a `.worktreeinclude` file's content.
fn build_file_match_context(
    file_path: &Path,
    file_dir: &Path,
    content: &str,
    case_insensitive: bool,
) -> Option<FileMatchContext> {
    let mut builder = GitignoreBuilder::new(file_dir);
    if case_insensitive {
        let _ = builder.case_insensitive(true);
    }

    let mut line_info: Vec<(usize, String)> = Vec::new();

    for (line_num_0, line_text) in content.lines().enumerate() {
        let line_1based = line_num_0 + 1;

        // Git only treats `#` in the first column as a comment. Leading
        // whitespace is significant and may be part of a valid filename.
        // The verified kernel decides whether the line is a pattern and the
        // length of its text without trailing whitespace.
        let chars: Vec<char> = line_text.chars().collect();
        let Some(pattern_len) = lines::rule_line_pattern_len(&chars) else {
            continue;
        };

        if builder
            .add_line(Some(file_path.to_path_buf()), line_text)
            .is_ok()
        {
            line_info.push((line_1based, chars[..pattern_len].iter().collect()));
        }
    }

    let matcher = builder.build().ok()?;
    Some(FileMatchContext {
        file_path: file_path.to_path_buf(),
        matcher,
        line_info,
    })
}

/// Explain whether a path is selected by `.worktreeinclude` files.
///
/// Collects `.worktreeinclude` files from `repo_root` down to the path's
/// parent directory, evaluates them shallowest-to-deepest with last-match-wins,
/// and returns a structured explanation.
///
/// Enforces the Git negation caveat both within and across files: if any
/// applicable file selects a parent directory, negation patterns (in the same
/// file or deeper files) cannot deselect files inside that directory.
pub fn explain(
    repo_root: &Path,
    rel_path: &str,
    is_dir: bool,
    case_insensitive: bool,
    symlink_policy: SymlinkPolicy,
) -> WorktreeincludeStatus {
    explain_path(
        repo_root,
        Path::new(rel_path),
        is_dir,
        case_insensitive,
        symlink_policy,
    )
}

/// Native-path variant of [`explain`] used during filesystem discovery.
///
/// Unix filenames are byte strings, so an unselected non-UTF-8 entry must be
/// matchable without first forcing it through the UTF-8-only `RepoRelPath`
/// boundary.
pub fn explain_path(
    repo_root: &Path,
    rel_path: &Path,
    is_dir: bool,
    case_insensitive: bool,
    symlink_policy: SymlinkPolicy,
) -> WorktreeincludeStatus {
    let path_within_repo = rel_path;

    // Collect directories from repo root to the path's parent
    let mut dirs_to_check = Vec::new();
    dirs_to_check.push(repo_root.to_path_buf());

    if let Some(parent) = path_within_repo.parent() {
        let mut current = PathBuf::new();
        for component in parent.components() {
            current.push(component);
            dirs_to_check.push(repo_root.join(&current));
        }
    }

    // Build matcher contexts for all applicable files
    let mut contexts: Vec<FileMatchContext> = Vec::new();
    for dir in &dirs_to_check {
        let wti_path = dir.join(".worktreeinclude");
        // A rule file must resolve to a regular file; the Ignore policy
        // treats symlinked rule files as if absent.
        if !rule_file_consulted(&wti_path, symlink_policy) {
            continue;
        }

        let content = match fs::read_to_string(&wti_path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        if let Some(ctx) = build_file_match_context(&wti_path, dir, &content, case_insensitive) {
            contexts.push(ctx);
        }
    }

    // Evaluate each file, shallowest to deepest, last match wins, with the
    // within-file and cross-file negation caveats. The decision is the
    // verified `rules::precedence::explain` kernel; this module only answers
    // its matcher queries and renders the verdict.
    let full_path: PathBuf = repo_root.join(rel_path).components().collect();
    let mut matchers = ContextMatchers::new(repo_root, &contexts, &full_path, rel_path, is_dir);
    let verdict = precedence::explain(&mut matchers);
    status_from_verdict(&contexts, verdict)
}

/// The applicable rule files' matchers, answering the verified precedence
/// kernel's queries (its [`RuleMatchOracle`] trust boundary).
struct ContextMatchers<'a> {
    repo_root: &'a Path,
    contexts: &'a [FileMatchContext],
    full_path: &'a Path,
    is_dir: bool,
    /// The non-empty ancestor directories of the queried path, parent first.
    ancestors: Vec<&'a Path>,
}

impl<'a> ContextMatchers<'a> {
    fn new(
        repo_root: &'a Path,
        contexts: &'a [FileMatchContext],
        full_path: &'a Path,
        rel_path: &'a Path,
        is_dir: bool,
    ) -> Self {
        let mut ancestors = Vec::new();
        let mut ancestor = rel_path.parent();
        while let Some(p) = ancestor {
            if p.as_os_str().is_empty() {
                break;
            }
            ancestors.push(p);
            ancestor = p.parent();
        }
        ContextMatchers {
            repo_root,
            contexts,
            full_path,
            is_dir,
            ancestors,
        }
    }
}

impl<'a> RuleMatchOracle<&'a Glob> for ContextMatchers<'a> {
    fn contexts(&self) -> usize {
        self.contexts.len()
    }

    fn ancestors(&self) -> usize {
        self.ancestors.len()
    }

    fn query_path(&mut self, ctx: usize) -> Option<(MatchKind, &'a Glob)> {
        let contexts: &'a [FileMatchContext] = self.contexts;
        path_match_kind(
            contexts[ctx]
                .matcher
                .matched_path_or_any_parents(self.full_path, self.is_dir),
        )
    }

    fn query_ancestor(&mut self, anc: usize, ctx: usize) -> Option<&'a Glob> {
        let contexts: &'a [FileMatchContext] = self.contexts;
        let ancestor_full: PathBuf = self
            .repo_root
            .join(self.ancestors[anc])
            .components()
            .collect();
        match contexts[ctx].matcher.matched(&ancestor_full, true) {
            Match::Ignore(glob) => Some(glob),
            Match::None | Match::Whitelist(_) => None,
        }
    }
}

/// Convert a rule file's verdict for the queried path to kernel data.
fn path_match_kind(matched: Match<&Glob>) -> Option<(MatchKind, &Glob)> {
    match matched {
        Match::None => None,
        Match::Ignore(glob) => Some((MatchKind::Ignore, glob)),
        Match::Whitelist(glob) => Some((MatchKind::Whitelist, glob)),
    }
}

/// Render the kernel's verdict with the deciding rule file and line.
fn status_from_verdict(
    contexts: &[FileMatchContext],
    verdict: Verdict<&Glob>,
) -> WorktreeincludeStatus {
    match verdict {
        Verdict::NoMatch => WorktreeincludeStatus::NoMatch,
        Verdict::Included {
            ctx,
            by: _,
            witness,
        } => {
            let ctx = &contexts[ctx];
            let (line, pattern) = glob_line_info(witness, &ctx.line_info);
            WorktreeincludeStatus::Included {
                file: ctx.file_path.clone(),
                line,
                pattern,
            }
        }
        Verdict::ExcludedByNegation { ctx, witness } => {
            let ctx = &contexts[ctx];
            let (line, pattern) = glob_line_info(witness, &ctx.line_info);
            WorktreeincludeStatus::ExcludedByNegation {
                file: ctx.file_path.clone(),
                line,
                pattern,
            }
        }
    }
}

/// Map a matched `Glob` back to its line number and pattern text.
///
/// Searches `line_info` (populated in add-order parallel to the builder's
/// glob list) for the entry whose text matches `glob.original()`.
fn glob_line_info(
    glob: &ignore::gitignore::Glob,
    line_info: &[(usize, String)],
) -> (usize, String) {
    let original = glob.original();
    match lines::last_line_with_text(line_info, original) {
        Some(index) => {
            let (line, pattern) = &line_info[index];
            (*line, pattern.clone())
        }
        // Fallback — reached for a line ending in an escaped space, whose
        // recorded text differs from the glob's original text (see
        // `lines::lemma_quirk_escaped_trailing_space_loses_provenance`).
        None => (0, original.to_string()),
    }
}

/// Evaluate `rel_path` using ONLY the repo-root `.worktreeinclude` file.
///
/// Used by the [`Claude202604Semantics`] engine: Claude observes the root
/// rule file and ignores nested ones. Symlink policy is honored at the
/// root file (`Ignore` skips a symlinked root file).
///
/// [`Claude202604Semantics`]: crate::worktreeinclude_engine::Claude202604Semantics
pub fn evaluate_root_only(
    repo_root: &Path,
    rel_path: &str,
    is_dir: bool,
    case_insensitive: bool,
    symlink_policy: SymlinkPolicy,
) -> WorktreeincludeStatus {
    evaluate_root_only_path(
        repo_root,
        Path::new(rel_path),
        is_dir,
        case_insensitive,
        symlink_policy,
    )
}

/// Native-path variant of [`evaluate_root_only`] used during filesystem
/// discovery.
pub fn evaluate_root_only_path(
    repo_root: &Path,
    rel_path: &Path,
    is_dir: bool,
    case_insensitive: bool,
    symlink_policy: SymlinkPolicy,
) -> WorktreeincludeStatus {
    if !root_rule_file_is_consulted(repo_root, symlink_policy) {
        return WorktreeincludeStatus::NoMatch;
    }
    let wti_path = repo_root.join(".worktreeinclude");

    let content = match fs::read_to_string(&wti_path) {
        Ok(c) => c,
        Err(_) => return WorktreeincludeStatus::NoMatch,
    };
    let ctx = match build_file_match_context(&wti_path, repo_root, &content, case_insensitive) {
        Some(c) => c,
        None => return WorktreeincludeStatus::NoMatch,
    };

    let full_path: PathBuf = repo_root.join(rel_path).components().collect();
    let contexts = [ctx];
    let mut matchers = ContextMatchers::new(repo_root, &contexts, &full_path, rel_path, is_dir);
    let verdict = precedence::explain_root_only(&mut matchers);
    status_from_verdict(&contexts, verdict)
}

/// Whether the repo-root `.worktreeinclude` is a file the root-only engine
/// would actually read.
///
/// This is the existence gate of [`evaluate_root_only_path`], which calls it,
/// so the two cannot drift. Callers outside the engine use it to tell "there
/// are no rules here" apart from "the rules that exist are somewhere this
/// engine never looks" — a distinction the repo-wide existence check in
/// `GitBackend::worktreeinclude_exists_anywhere` cannot make.
pub fn root_rule_file_is_consulted(repo_root: &Path, symlink_policy: SymlinkPolicy) -> bool {
    rule_file_consulted(&repo_root.join(".worktreeinclude"), symlink_policy)
}

/// The per-path engines' consult gate for the rule file at `wti_path`,
/// decided by the verified `selection::rule_file_consulted` kernel.
fn rule_file_consulted(wti_path: &Path, symlink_policy: SymlinkPolicy) -> bool {
    selection::rule_file_consulted(
        &mut RuleFilePath(wti_path),
        crate::worktreeinclude_engine::kernel_symlink_policy(symlink_policy),
    )
}

/// The filesystem facts about one rule-file location (the consult gate's
/// [`RuleFileProbe`] trust boundary).
struct RuleFilePath<'a>(&'a Path);

impl RuleFileProbe for RuleFilePath<'_> {
    fn probe_is_file(&mut self) -> bool {
        self.0.is_file()
    }

    /// True if the path itself is a symlink (without following).
    fn probe_is_symlink(&mut self) -> bool {
        fs::symlink_metadata(self.0)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup_repo() -> TempDir {
        TempDir::new().unwrap()
    }

    #[test]
    fn simple_match() {
        let dir = setup_repo();
        fs::write(dir.path().join(".worktreeinclude"), ".env\n").unwrap();

        let result = explain(dir.path(), ".env", false, false, SymlinkPolicy::Follow);
        match result {
            WorktreeincludeStatus::Included { pattern, line, .. } => {
                assert_eq!(pattern, ".env");
                assert_eq!(line, 1);
            }
            other => panic!("expected Included, got {other:?}"),
        }
    }

    #[test]
    fn glob_match() {
        let dir = setup_repo();
        fs::write(dir.path().join(".worktreeinclude"), "*.env\n").unwrap();

        let result = explain(
            dir.path(),
            "production.env",
            false,
            false,
            SymlinkPolicy::Follow,
        );
        assert!(matches!(result, WorktreeincludeStatus::Included { .. }));
    }

    #[test]
    fn no_match() {
        let dir = setup_repo();
        fs::write(dir.path().join(".worktreeinclude"), "*.env\n").unwrap();

        let result = explain(dir.path(), "README.md", false, false, SymlinkPolicy::Follow);
        assert!(matches!(result, WorktreeincludeStatus::NoMatch));
    }

    #[test]
    fn leading_space_before_hash_is_a_literal_pattern() {
        let dir = setup_repo();
        fs::write(dir.path().join(".worktreeinclude"), " #literal.env\n").unwrap();

        let result = explain(
            dir.path(),
            " #literal.env",
            false,
            false,
            SymlinkPolicy::Follow,
        );
        match result {
            WorktreeincludeStatus::Included { pattern, line, .. } => {
                assert_eq!(pattern, " #literal.env");
                assert_eq!(line, 1);
            }
            other => panic!("expected literal leading-space pattern, got {other:?}"),
        }
    }

    #[test]
    fn negation_deselects() {
        let dir = setup_repo();
        fs::write(dir.path().join(".worktreeinclude"), "*.env\n!test.env\n").unwrap();

        let result = explain(dir.path(), "test.env", false, false, SymlinkPolicy::Follow);
        match result {
            WorktreeincludeStatus::ExcludedByNegation { pattern, line, .. } => {
                assert_eq!(pattern, "!test.env");
                assert_eq!(line, 2);
            }
            other => panic!("expected ExcludedByNegation, got {other:?}"),
        }
    }

    #[test]
    fn last_match_wins_within_file() {
        let dir = setup_repo();
        fs::write(
            dir.path().join(".worktreeinclude"),
            "*.env\n!*.env\nproduction.env\n",
        )
        .unwrap();

        let result = explain(
            dir.path(),
            "production.env",
            false,
            false,
            SymlinkPolicy::Follow,
        );
        match result {
            WorktreeincludeStatus::Included { pattern, line, .. } => {
                assert_eq!(pattern, "production.env");
                assert_eq!(line, 3);
            }
            other => panic!("expected Included, got {other:?}"),
        }
    }

    #[test]
    fn nested_file_overrides_root() {
        let dir = setup_repo();
        let subdir = dir.path().join("config");
        fs::create_dir(&subdir).unwrap();

        // Root says include all .env
        fs::write(dir.path().join(".worktreeinclude"), "*.env\n").unwrap();
        // Nested says exclude .env in config/
        fs::write(subdir.join(".worktreeinclude"), "!*.env\n").unwrap();

        let result = explain(
            dir.path(),
            "config/prod.env",
            false,
            false,
            SymlinkPolicy::Follow,
        );
        assert!(
            matches!(result, WorktreeincludeStatus::ExcludedByNegation { .. }),
            "nested .worktreeinclude should override root, got {result:?}"
        );
    }

    #[test]
    fn nested_file_does_not_affect_sibling() {
        let dir = setup_repo();
        let subdir = dir.path().join("config");
        fs::create_dir(&subdir).unwrap();

        fs::write(dir.path().join(".worktreeinclude"), "*.env\n").unwrap();
        fs::write(subdir.join(".worktreeinclude"), "!*.env\n").unwrap();

        // A .env file NOT in config/ should still match root
        let result = explain(dir.path(), "app.env", false, false, SymlinkPolicy::Follow);
        assert!(
            matches!(result, WorktreeincludeStatus::Included { .. }),
            "root .worktreeinclude should still apply to non-nested paths, got {result:?}"
        );
    }

    #[test]
    fn doublestar_pattern() {
        let dir = setup_repo();
        fs::write(dir.path().join(".worktreeinclude"), "**/*.secret\n").unwrap();

        let result = explain(
            dir.path(),
            "a/b/c/key.secret",
            false,
            false,
            SymlinkPolicy::Follow,
        );
        assert!(matches!(result, WorktreeincludeStatus::Included { .. }));
    }

    #[test]
    fn directory_only_pattern_matches_dir() {
        let dir = setup_repo();
        fs::write(dir.path().join(".worktreeinclude"), "build/\n").unwrap();

        let result = explain(dir.path(), "build", true, false, SymlinkPolicy::Follow);
        assert!(matches!(result, WorktreeincludeStatus::Included { .. }));
    }

    #[test]
    fn directory_only_pattern_does_not_match_file() {
        let dir = setup_repo();
        fs::write(dir.path().join(".worktreeinclude"), "build/\n").unwrap();

        let result = explain(dir.path(), "build", false, false, SymlinkPolicy::Follow);
        assert!(
            matches!(result, WorktreeincludeStatus::NoMatch),
            "directory-only pattern should not match files, got {result:?}"
        );
    }

    #[test]
    fn anchored_pattern() {
        let dir = setup_repo();
        fs::write(dir.path().join(".worktreeinclude"), "/root-only.env\n").unwrap();

        // Should match at root
        let result = explain(
            dir.path(),
            "root-only.env",
            false,
            false,
            SymlinkPolicy::Follow,
        );
        assert!(matches!(result, WorktreeincludeStatus::Included { .. }));

        // Should NOT match in subdirectory
        let result = explain(
            dir.path(),
            "sub/root-only.env",
            false,
            false,
            SymlinkPolicy::Follow,
        );
        assert!(
            matches!(result, WorktreeincludeStatus::NoMatch),
            "anchored pattern should not match in subdirectory, got {result:?}"
        );
    }

    #[test]
    fn no_worktreeinclude_files() {
        let dir = setup_repo();
        let result = explain(
            dir.path(),
            "anything.env",
            false,
            false,
            SymlinkPolicy::Follow,
        );
        assert!(matches!(result, WorktreeincludeStatus::NoMatch));
    }

    #[test]
    fn case_insensitive_matching() {
        let dir = setup_repo();
        fs::write(dir.path().join(".worktreeinclude"), "*.ENV\n").unwrap();

        let result = explain(dir.path(), "test.env", false, true, SymlinkPolicy::Follow);
        assert!(
            matches!(result, WorktreeincludeStatus::Included { .. }),
            "case-insensitive matching should work, got {result:?}"
        );
    }

    #[test]
    fn nested_anchored_pattern_matches_relative_to_file_dir() {
        // Bug: config/.worktreeinclude with `/foo` should match `config/foo`
        // because `/foo` is anchored to the directory containing the file.
        let dir = setup_repo();
        let subdir = dir.path().join("config");
        fs::create_dir(&subdir).unwrap();

        fs::write(subdir.join(".worktreeinclude"), "/foo\n").unwrap();

        let result = explain(
            dir.path(),
            "config/foo",
            false,
            false,
            SymlinkPolicy::Follow,
        );
        assert!(
            matches!(result, WorktreeincludeStatus::Included { .. }),
            "anchored pattern in nested .worktreeinclude should match relative to its dir, got {result:?}"
        );
    }

    #[test]
    fn git_negation_caveat_dir_pattern_wins_over_file_negation() {
        // Git caveat: "It is not possible to re-include a file if a parent
        // directory of that file is excluded." In .worktreeinclude semantics:
        // if `dir/` selects the directory, `!dir/keep` cannot deselect a file
        // inside it.
        let dir = setup_repo();
        fs::write(dir.path().join(".worktreeinclude"), "dir/\n!dir/keep\n").unwrap();

        let result = explain(dir.path(), "dir/keep", false, false, SymlinkPolicy::Follow);
        assert!(
            matches!(result, WorktreeincludeStatus::Included { .. }),
            "dir/ selection should win over !dir/keep negation (Git caveat), got {result:?}"
        );
    }

    #[test]
    fn deeper_file_anchored_and_negated_combination() {
        // Combined test: nested .worktreeinclude with anchored pattern and
        // negation, verifying both path computation and caveat semantics.
        let dir = setup_repo();
        let subdir = dir.path().join("sub");
        fs::create_dir(&subdir).unwrap();

        fs::write(
            subdir.join(".worktreeinclude"),
            "/secrets/\n!/secrets/public\n",
        )
        .unwrap();

        // secrets/ directory pattern should select files inside it
        let result = explain(
            dir.path(),
            "sub/secrets/private.key",
            false,
            false,
            SymlinkPolicy::Follow,
        );
        assert!(
            matches!(result, WorktreeincludeStatus::Included { .. }),
            "anchored dir pattern in nested file should select files inside, got {result:?}"
        );

        // Git caveat: negation cannot override directory selection
        let result = explain(
            dir.path(),
            "sub/secrets/public",
            false,
            false,
            SymlinkPolicy::Follow,
        );
        assert!(
            matches!(result, WorktreeincludeStatus::Included { .. }),
            "negation cannot deselect file inside selected directory (Git caveat), got {result:?}"
        );
    }

    #[test]
    fn nested_anchored_pattern_does_not_match_outside_dir() {
        // Anchored pattern in config/.worktreeinclude should NOT match paths
        // outside the config/ directory.
        let dir = setup_repo();
        let subdir = dir.path().join("config");
        fs::create_dir(&subdir).unwrap();

        fs::write(subdir.join(".worktreeinclude"), "/foo\n").unwrap();

        // Should NOT match foo at root (anchored to config/, not repo root)
        let result = explain(dir.path(), "foo", false, false, SymlinkPolicy::Follow);
        assert!(
            matches!(result, WorktreeincludeStatus::NoMatch),
            "anchored pattern in config/.worktreeinclude should not match root-level foo, got {result:?}"
        );

        // Should NOT match foo in a sibling directory
        let result = explain(dir.path(), "other/foo", false, false, SymlinkPolicy::Follow);
        assert!(
            matches!(result, WorktreeincludeStatus::NoMatch),
            "anchored pattern in config/.worktreeinclude should not match other/foo, got {result:?}"
        );
    }

    #[test]
    fn deeply_nested_anchored_pattern() {
        // Verify anchored patterns work correctly at deeper nesting levels
        let dir = setup_repo();
        let deep = dir.path().join("a").join("b").join("c");
        fs::create_dir_all(&deep).unwrap();

        fs::write(deep.join(".worktreeinclude"), "/secret.key\n").unwrap();

        let result = explain(
            dir.path(),
            "a/b/c/secret.key",
            false,
            false,
            SymlinkPolicy::Follow,
        );
        assert!(
            matches!(result, WorktreeincludeStatus::Included { .. }),
            "deeply nested anchored pattern should match, got {result:?}"
        );

        // Should not match at a different depth
        let result = explain(
            dir.path(),
            "a/b/secret.key",
            false,
            false,
            SymlinkPolicy::Follow,
        );
        assert!(
            matches!(result, WorktreeincludeStatus::NoMatch),
            "deeply nested anchored pattern should not match at wrong depth, got {result:?}"
        );
    }

    #[test]
    fn negation_caveat_in_nested_worktreeinclude() {
        // Verify the Git negation caveat works correctly in nested
        // .worktreeinclude files, not just root-level ones.
        let dir = setup_repo();
        let subdir = dir.path().join("deploy");
        fs::create_dir(&subdir).unwrap();

        fs::write(
            subdir.join(".worktreeinclude"),
            "configs/\n!configs/local.yml\n",
        )
        .unwrap();

        // File inside selected directory — caveat should prevent negation
        let result = explain(
            dir.path(),
            "deploy/configs/local.yml",
            false,
            false,
            SymlinkPolicy::Follow,
        );
        assert!(
            matches!(result, WorktreeincludeStatus::Included { .. }),
            "negation caveat should apply in nested .worktreeinclude, got {result:?}"
        );

        // Another file inside the directory should be selected normally
        let result = explain(
            dir.path(),
            "deploy/configs/prod.yml",
            false,
            false,
            SymlinkPolicy::Follow,
        );
        assert!(
            matches!(result, WorktreeincludeStatus::Included { .. }),
            "file inside selected dir in nested .worktreeinclude should be included, got {result:?}"
        );
    }

    #[test]
    fn cross_file_negation_caveat() {
        // Git caveat applies cross-file: if root .worktreeinclude selects a
        // directory with `dir/`, a nested dir/.worktreeinclude cannot deselect
        // files inside it via negation. This mirrors Git's behavior where an
        // excluded parent directory prevents re-inclusion from deeper files.
        let dir = setup_repo();
        let subdir = dir.path().join("secrets");
        fs::create_dir(&subdir).unwrap();

        // Root selects the entire secrets/ directory
        fs::write(dir.path().join(".worktreeinclude"), "secrets/\n").unwrap();
        // Nested file tries to deselect a specific file
        fs::write(subdir.join(".worktreeinclude"), "!private.key\n").unwrap();

        let result = explain(
            dir.path(),
            "secrets/private.key",
            false,
            false,
            SymlinkPolicy::Follow,
        );
        assert!(
            matches!(result, WorktreeincludeStatus::Included { .. }),
            "cross-file caveat: dir selected by root should block nested negation, got {result:?}"
        );
    }

    #[test]
    fn path_match_kind_preserves_kind_and_glob() {
        let dir = setup_repo();
        let mut builder = GitignoreBuilder::new(dir.path());
        builder.add_line(None, "*.env").unwrap();
        builder.add_line(None, "!keep.env").unwrap();
        let matcher = builder.build().unwrap();

        match path_match_kind(matcher.matched("a.env", false)) {
            Some((MatchKind::Ignore, glob)) => assert_eq!(glob.original(), "*.env"),
            other => panic!("expected an ignore match, got {other:?}"),
        }
        match path_match_kind(matcher.matched("keep.env", false)) {
            Some((MatchKind::Whitelist, glob)) => assert_eq!(glob.original(), "!keep.env"),
            other => panic!("expected a whitelist match, got {other:?}"),
        }
        assert!(path_match_kind(matcher.matched("README.md", false)).is_none());
    }

    #[test]
    fn rule_line_kernel_matches_trim_end_grammar() {
        // Reference: the grammar the kernel replaced.
        let reference = |line: &str| {
            let pattern_text = line.trim_end();
            if pattern_text.is_empty() || line.starts_with('#') {
                None
            } else {
                Some(pattern_text.to_string())
            }
        };
        let samples = [
            "",
            " ",
            "\t",
            "#x",
            " #x",
            "foo",
            "foo  ",
            "foo\t",
            "foo\\ ",
            "\u{a0}foo\u{a0}",
            "\u{3000}",
            "a b ",
            "!x",
            "\u{85}",
            "#",
            "x\u{2028}",
            "\u{feff}x",
        ];
        for line in samples {
            let chars: Vec<char> = line.chars().collect();
            let actual = lines::rule_line_pattern_len(&chars)
                .map(|len| chars[..len].iter().collect::<String>());
            assert_eq!(actual, reference(line), "line {line:?}");
        }
    }

    #[test]
    fn line_lookup_returns_last_matching_entry() {
        let info = vec![
            (1, "a".to_string()),
            (3, "b".to_string()),
            (5, "a".to_string()),
        ];
        assert_eq!(lines::last_line_with_text(&info, "a"), Some(2));
        assert_eq!(lines::last_line_with_text(&info, "b"), Some(1));
        assert_eq!(lines::last_line_with_text(&info, "c"), None);
        assert_eq!(lines::last_line_with_text(&[], "a"), None);
    }

    #[test]
    fn quirk_escaped_trailing_space_reports_line_zero() {
        // Pins current behavior (lines::
        // lemma_quirk_escaped_trailing_space_loses_provenance).
        let dir = setup_repo();
        fs::write(dir.path().join(".worktreeinclude"), "foo\\ \n").unwrap();

        let result = explain(dir.path(), "foo ", false, false, SymlinkPolicy::Follow);
        match result {
            WorktreeincludeStatus::Included { line, pattern, .. } => {
                assert_eq!(line, 0);
                assert_eq!(pattern, "foo\\ ");
            }
            other => panic!("expected the current Included at line 0, got {other:?}"),
        }
    }

    // The next four tests pin current behavior that differs from Git; each
    // is proved as a divergence lemma in `waft_verified::rules::reference`.

    #[test]
    fn quirk_nested_file_reaches_outside_its_directory() {
        let dir = setup_repo();
        let nested = dir.path().join("app").join("config");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join(".worktreeinclude"), "*\n!secret.key\n").unwrap();

        let result = explain(
            dir.path(),
            "app/config/secret.key",
            false,
            false,
            SymlinkPolicy::Follow,
        );
        match result {
            WorktreeincludeStatus::Included { pattern, .. } => assert_eq!(pattern, "*"),
            other => panic!("expected the current Included (*), got {other:?}"),
        }
    }

    #[test]
    fn quirk_cross_file_caveat_ignores_deeper_reinclusion() {
        let dir = setup_repo();
        fs::create_dir(dir.path().join("a")).unwrap();
        fs::write(dir.path().join(".worktreeinclude"), "a/b/\n").unwrap();
        fs::write(dir.path().join("a").join(".worktreeinclude"), "!b/\n!b/x\n").unwrap();

        let result = explain(dir.path(), "a/b/x", false, false, SymlinkPolicy::Follow);
        match result {
            WorktreeincludeStatus::Included { pattern, .. } => assert_eq!(pattern, "a/b/"),
            other => panic!("expected the current Included (a/b/), got {other:?}"),
        }
    }

    #[test]
    fn quirk_ancestor_negation_shadows_shallower_selection() {
        let dir = setup_repo();
        fs::create_dir(dir.path().join("sub")).unwrap();
        fs::write(dir.path().join(".worktreeinclude"), "*.env\n").unwrap();
        fs::write(dir.path().join("sub").join(".worktreeinclude"), "!x/\n").unwrap();

        let result = explain(
            dir.path(),
            "sub/x/a.env",
            false,
            false,
            SymlinkPolicy::Follow,
        );
        match result {
            WorktreeincludeStatus::ExcludedByNegation { pattern, .. } => {
                assert_eq!(pattern, "!x/")
            }
            other => panic!("expected the current ExcludedByNegation (!x/), got {other:?}"),
        }
    }

    #[test]
    fn quirk_bare_slash_selects_everything() {
        let dir = setup_repo();
        fs::write(dir.path().join(".worktreeinclude"), "/\n").unwrap();

        for result in [
            explain(
                dir.path(),
                "deep/any/file.txt",
                false,
                false,
                SymlinkPolicy::Follow,
            ),
            evaluate_root_only(
                dir.path(),
                "deep/any/file.txt",
                false,
                false,
                SymlinkPolicy::Follow,
            ),
        ] {
            assert!(
                matches!(result, WorktreeincludeStatus::Included { .. }),
                "expected the current Included, got {result:?}"
            );
        }
    }

    #[test]
    fn negation_without_dir_pattern_still_deselects() {
        // When the positive match is a file pattern (not directory), negation
        // should work normally — the caveat only applies to directory patterns.
        let dir = setup_repo();
        fs::write(dir.path().join(".worktreeinclude"), "*.env\n!staging.env\n").unwrap();

        let result = explain(
            dir.path(),
            "staging.env",
            false,
            false,
            SymlinkPolicy::Follow,
        );
        assert!(
            matches!(result, WorktreeincludeStatus::ExcludedByNegation { .. }),
            "negation should deselect when positive is file pattern (no caveat), got {result:?}"
        );

        // Other .env files should still be selected
        let result = explain(
            dir.path(),
            "production.env",
            false,
            false,
            SymlinkPolicy::Follow,
        );
        assert!(
            matches!(result, WorktreeincludeStatus::Included { .. }),
            "non-negated .env should still be included, got {result:?}"
        );
    }
}
