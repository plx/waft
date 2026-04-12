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

use ignore::gitignore::GitignoreBuilder;

use crate::model::WorktreeincludeStatus;

/// Explain whether a path is selected by `.worktreeinclude` files.
///
/// Collects `.worktreeinclude` files from `repo_root` down to the path's
/// parent directory, evaluates them shallowest-to-deepest with last-match-wins,
/// and returns a structured explanation.
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

    // Track the last matching result across all files (shallowest to deepest)
    let mut last_result = WorktreeincludeStatus::NoMatch;

    for dir in &dirs_to_check {
        let wti_path = dir.join(".worktreeinclude");
        if !wti_path.is_file() {
            continue;
        }

        let content = match fs::read_to_string(&wti_path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        if let Some(result) = evaluate_file(
            &wti_path,
            dir,
            &content,
            rel_path,
            is_dir,
            case_insensitive,
            repo_root,
        ) {
            last_result = result;
        }
    }

    last_result
}

/// Evaluate a single `.worktreeinclude` file against a path.
///
/// Builds a combined matcher from all patterns in the file, evaluates with
/// Git-style last-match-wins semantics, and enforces the Git negation caveat:
/// a negation pattern cannot de-select a file whose parent directory is
/// already selected by an earlier pattern.
///
/// Returns `Some(status)` if any pattern matches, `None` otherwise.
fn evaluate_file(
    file_path: &Path,
    file_dir: &Path,
    content: &str,
    rel_path: &str,
    is_dir: bool,
    case_insensitive: bool,
    repo_root: &Path,
) -> Option<WorktreeincludeStatus> {
    // Build a single combined matcher for the whole file, tracking line info
    // for each added pattern so we can report provenance.
    let mut builder = GitignoreBuilder::new(file_dir);
    if case_insensitive {
        let _ = builder.case_insensitive(true);
    }

    // line_info[i] corresponds to the i-th glob added to the builder.
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

    let matcher = match builder.build() {
        Ok(m) => m,
        Err(_) => return None,
    };

    // Construct full absolute path for matching — always relative to repo root,
    // not file_dir, to avoid duplicating directory segments for nested files.
    let full_path: PathBuf = repo_root.join(rel_path).components().collect();

    let matched = matcher.matched_path_or_any_parents(&full_path, is_dir);

    if matched.is_ignore() {
        let glob = matched.inner().unwrap();
        let (line, pattern) = glob_line_info(glob, &line_info);
        Some(WorktreeincludeStatus::Included {
            file: file_path.to_path_buf(),
            line,
            pattern,
        })
    } else if matched.is_whitelist() {
        // Git caveat: "It is not possible to re-include a file if a parent
        // directory of that file is excluded." In .worktreeinclude terms,
        // if a parent directory is selected (Ignore match), a negation
        // pattern cannot deselect a file inside it.
        let path_within = Path::new(rel_path);
        let mut ancestor = path_within.parent();
        while let Some(p) = ancestor {
            if p.as_os_str().is_empty() {
                break;
            }
            let ancestor_full: PathBuf = repo_root.join(p).components().collect();
            let ancestor_match = matcher.matched(&ancestor_full, true);
            if ancestor_match.is_ignore() {
                // Parent directory is selected — caveat applies, negation loses
                let parent_glob = ancestor_match.inner().unwrap();
                let (line, pattern) = glob_line_info(parent_glob, &line_info);
                return Some(WorktreeincludeStatus::Included {
                    file: file_path.to_path_buf(),
                    line,
                    pattern,
                });
            }
            ancestor = p.parent();
        }

        let glob = matched.inner().unwrap();
        let (line, pattern) = glob_line_info(glob, &line_info);
        Some(WorktreeincludeStatus::ExcludedByNegation {
            file: file_path.to_path_buf(),
            line,
            pattern,
        })
    } else {
        None
    }
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

        fs::write(subdir.join(".worktreeinclude"), "/secrets/\n!/secrets/public\n").unwrap();

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
    fn negation_without_dir_pattern_still_deselects() {
        // When the positive match is a file pattern (not directory), negation
        // should work normally — the caveat only applies to directory patterns.
        let dir = setup_repo();
        fs::write(
            dir.path().join(".worktreeinclude"),
            "*.env\n!staging.env\n",
        )
        .unwrap();

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
