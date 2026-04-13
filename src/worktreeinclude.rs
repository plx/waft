//! `.worktreeinclude` explanation engine.
//!
//! This module evaluates paths against `.worktreeinclude` files for
//! explanation and validation purposes. It is **not** the authoritative
//! selector — Git CLI is authoritative for actual candidate selection.
//!
//! The engine collects applicable `.worktreeinclude` files from the repo
//! root to the queried path's parent, builds a matcher for each file
//! rooted at its directory, and evaluates with last-match-wins semantics
//! (shallow to deep, last match within each file).

use std::fs;
use std::path::{Path, PathBuf};

use ignore::gitignore::{Gitignore, GitignoreBuilder};

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
        let trimmed = line_text.trim();

        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        if builder
            .add_line(Some(file_path.to_path_buf()), line_text)
            .is_ok()
        {
            line_info.push((line_1based, trimmed.to_string()));
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
) -> WorktreeincludeStatus {
    let path_within_repo = Path::new(rel_path);

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
        if !wti_path.is_file() {
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

    // Evaluate each file, shallowest to deepest, last match wins
    let full_path: PathBuf = repo_root.join(rel_path).components().collect();
    let mut last_result = WorktreeincludeStatus::NoMatch;

    for ctx in &contexts {
        if let Some(result) = evaluate_against_context(ctx, &full_path, rel_path, is_dir, repo_root)
        {
            last_result = result;
        }
    }

    // Cross-file caveat: if the final result is ExcludedByNegation, check
    // whether ANY applicable file selected a parent directory of this path.
    // Per Git semantics, a negation cannot deselect a file whose parent
    // directory is selected — regardless of which file the patterns come from.
    if matches!(
        last_result,
        WorktreeincludeStatus::ExcludedByNegation { .. }
    ) && let Some(override_result) = cross_file_ancestor_check(&contexts, rel_path, repo_root)
    {
        return override_result;
    }

    last_result
}

/// Evaluate a single `.worktreeinclude` file's matcher against a path.
///
/// Uses Git-style last-match-wins semantics and enforces the within-file
/// negation caveat: a negation pattern cannot de-select a file whose parent
/// directory is already selected by an earlier pattern in the same file.
///
/// Returns `Some(status)` if any pattern matches, `None` otherwise.
fn evaluate_against_context(
    ctx: &FileMatchContext,
    full_path: &Path,
    rel_path: &str,
    is_dir: bool,
    repo_root: &Path,
) -> Option<WorktreeincludeStatus> {
    let matched = ctx.matcher.matched_path_or_any_parents(full_path, is_dir);

    if matched.is_ignore() {
        let glob = matched.inner().unwrap();
        let (line, pattern) = glob_line_info(glob, &ctx.line_info);
        Some(WorktreeincludeStatus::Included {
            file: ctx.file_path.clone(),
            line,
            pattern,
        })
    } else if matched.is_whitelist() {
        // Within-file caveat: check if this file's own patterns select a
        // parent directory — if so, negation cannot override.
        let path_within = Path::new(rel_path);
        let mut ancestor = path_within.parent();
        while let Some(p) = ancestor {
            if p.as_os_str().is_empty() {
                break;
            }
            let ancestor_full: PathBuf = repo_root.join(p).components().collect();
            let ancestor_match = ctx.matcher.matched(&ancestor_full, true);
            if ancestor_match.is_ignore() {
                let parent_glob = ancestor_match.inner().unwrap();
                let (line, pattern) = glob_line_info(parent_glob, &ctx.line_info);
                return Some(WorktreeincludeStatus::Included {
                    file: ctx.file_path.clone(),
                    line,
                    pattern,
                });
            }
            ancestor = p.parent();
        }

        let glob = matched.inner().unwrap();
        let (line, pattern) = glob_line_info(glob, &ctx.line_info);
        Some(WorktreeincludeStatus::ExcludedByNegation {
            file: ctx.file_path.clone(),
            line,
            pattern,
        })
    } else {
        None
    }
}

/// Check all file contexts for an ancestor directory selection that would
/// override a negation result (the cross-file negation caveat).
fn cross_file_ancestor_check(
    contexts: &[FileMatchContext],
    rel_path: &str,
    repo_root: &Path,
) -> Option<WorktreeincludeStatus> {
    let path_within = Path::new(rel_path);
    let mut ancestor = path_within.parent();
    while let Some(p) = ancestor {
        if p.as_os_str().is_empty() {
            break;
        }
        let ancestor_full: PathBuf = repo_root.join(p).components().collect();
        for ctx in contexts {
            let ancestor_match = ctx.matcher.matched(&ancestor_full, true);
            if ancestor_match.is_ignore() {
                let glob = ancestor_match.inner().unwrap();
                let (line, pattern) = glob_line_info(glob, &ctx.line_info);
                return Some(WorktreeincludeStatus::Included {
                    file: ctx.file_path.clone(),
                    line,
                    pattern,
                });
            }
        }
        ancestor = p.parent();
    }
    None
}

/// Map a matched `Glob` back to its line number and pattern text.
///
/// Searches `line_info` (populated in add-order parallel to the builder's
/// glob list) for the entry whose trimmed text matches `glob.original()`.
fn glob_line_info(
    glob: &ignore::gitignore::Glob,
    line_info: &[(usize, String)],
) -> (usize, String) {
    let original = glob.original();
    // The original in the Glob has trailing whitespace stripped but may
    // retain leading whitespace. Our line_info stores trimmed text.
    let original_trimmed = original.trim();
    for (line, pattern) in line_info.iter().rev() {
        if pattern == original_trimmed {
            return (*line, pattern.clone());
        }
    }
    // Fallback — should not happen with well-formed input
    (0, original.to_string())
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

        let result = explain(dir.path(), ".env", false, false);
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

        let result = explain(dir.path(), "production.env", false, false);
        assert!(matches!(result, WorktreeincludeStatus::Included { .. }));
    }

    #[test]
    fn no_match() {
        let dir = setup_repo();
        fs::write(dir.path().join(".worktreeinclude"), "*.env\n").unwrap();

        let result = explain(dir.path(), "README.md", false, false);
        assert!(matches!(result, WorktreeincludeStatus::NoMatch));
    }

    #[test]
    fn negation_deselects() {
        let dir = setup_repo();
        fs::write(dir.path().join(".worktreeinclude"), "*.env\n!test.env\n").unwrap();

        let result = explain(dir.path(), "test.env", false, false);
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

        let result = explain(dir.path(), "production.env", false, false);
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

        let result = explain(dir.path(), "config/prod.env", false, false);
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
        let result = explain(dir.path(), "app.env", false, false);
        assert!(
            matches!(result, WorktreeincludeStatus::Included { .. }),
            "root .worktreeinclude should still apply to non-nested paths, got {result:?}"
        );
    }

    #[test]
    fn doublestar_pattern() {
        let dir = setup_repo();
        fs::write(dir.path().join(".worktreeinclude"), "**/*.secret\n").unwrap();

        let result = explain(dir.path(), "a/b/c/key.secret", false, false);
        assert!(matches!(result, WorktreeincludeStatus::Included { .. }));
    }

    #[test]
    fn directory_only_pattern_matches_dir() {
        let dir = setup_repo();
        fs::write(dir.path().join(".worktreeinclude"), "build/\n").unwrap();

        let result = explain(dir.path(), "build", true, false);
        assert!(matches!(result, WorktreeincludeStatus::Included { .. }));
    }

    #[test]
    fn directory_only_pattern_does_not_match_file() {
        let dir = setup_repo();
        fs::write(dir.path().join(".worktreeinclude"), "build/\n").unwrap();

        let result = explain(dir.path(), "build", false, false);
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
        let result = explain(dir.path(), "root-only.env", false, false);
        assert!(matches!(result, WorktreeincludeStatus::Included { .. }));

        // Should NOT match in subdirectory
        let result = explain(dir.path(), "sub/root-only.env", false, false);
        assert!(
            matches!(result, WorktreeincludeStatus::NoMatch),
            "anchored pattern should not match in subdirectory, got {result:?}"
        );
    }

    #[test]
    fn no_worktreeinclude_files() {
        let dir = setup_repo();
        let result = explain(dir.path(), "anything.env", false, false);
        assert!(matches!(result, WorktreeincludeStatus::NoMatch));
    }

    #[test]
    fn case_insensitive_matching() {
        let dir = setup_repo();
        fs::write(dir.path().join(".worktreeinclude"), "*.ENV\n").unwrap();

        let result = explain(dir.path(), "test.env", false, true);
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

        let result = explain(dir.path(), "config/foo", false, false);
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

        let result = explain(dir.path(), "dir/keep", false, false);
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
        let result = explain(dir.path(), "sub/secrets/private.key", false, false);
        assert!(
            matches!(result, WorktreeincludeStatus::Included { .. }),
            "anchored dir pattern in nested file should select files inside, got {result:?}"
        );

        // Git caveat: negation cannot override directory selection
        let result = explain(dir.path(), "sub/secrets/public", false, false);
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
        let result = explain(dir.path(), "foo", false, false);
        assert!(
            matches!(result, WorktreeincludeStatus::NoMatch),
            "anchored pattern in config/.worktreeinclude should not match root-level foo, got {result:?}"
        );

        // Should NOT match foo in a sibling directory
        let result = explain(dir.path(), "other/foo", false, false);
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

        let result = explain(dir.path(), "a/b/c/secret.key", false, false);
        assert!(
            matches!(result, WorktreeincludeStatus::Included { .. }),
            "deeply nested anchored pattern should match, got {result:?}"
        );

        // Should not match at a different depth
        let result = explain(dir.path(), "a/b/secret.key", false, false);
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
        let result = explain(dir.path(), "deploy/configs/local.yml", false, false);
        assert!(
            matches!(result, WorktreeincludeStatus::Included { .. }),
            "negation caveat should apply in nested .worktreeinclude, got {result:?}"
        );

        // Another file inside the directory should be selected normally
        let result = explain(dir.path(), "deploy/configs/prod.yml", false, false);
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

        let result = explain(dir.path(), "secrets/private.key", false, false);
        assert!(
            matches!(result, WorktreeincludeStatus::Included { .. }),
            "cross-file caveat: dir selected by root should block nested negation, got {result:?}"
        );
    }

    #[test]
    fn negation_without_dir_pattern_still_deselects() {
        // When the positive match is a file pattern (not directory), negation
        // should work normally — the caveat only applies to directory patterns.
        let dir = setup_repo();
        fs::write(dir.path().join(".worktreeinclude"), "*.env\n!staging.env\n").unwrap();

        let result = explain(dir.path(), "staging.env", false, false);
        assert!(
            matches!(result, WorktreeincludeStatus::ExcludedByNegation { .. }),
            "negation should deselect when positive is file pattern (no caveat), got {result:?}"
        );

        // Other .env files should still be selected
        let result = explain(dir.path(), "production.env", false, false);
        assert!(
            matches!(result, WorktreeincludeStatus::Included { .. }),
            "non-negated .env should still be included, got {result:?}"
        );
    }
}
