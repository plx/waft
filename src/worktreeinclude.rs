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

        if let Some(result) =
            evaluate_file(&wti_path, dir, &content, rel_path, is_dir, case_insensitive)
        {
            last_result = result;
        }
    }

    last_result
}

/// Evaluate a single `.worktreeinclude` file against a path.
///
/// Returns `Some(status)` if any pattern in the file matches, `None` otherwise.
/// Within the file, last-match-wins.
fn evaluate_file(
    file_path: &Path,
    file_dir: &Path,
    content: &str,
    rel_path: &str,
    is_dir: bool,
    case_insensitive: bool,
) -> Option<WorktreeincludeStatus> {
    let mut last_match: Option<WorktreeincludeStatus> = None;

    for (line_num_0, line_text) in content.lines().enumerate() {
        let line_1based = line_num_0 + 1;
        let trimmed = line_text.trim();

        // Skip empty lines and comments
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        // Build a single-pattern matcher for this line
        let mut builder = GitignoreBuilder::new(file_dir);
        if case_insensitive {
            let _ = builder.case_insensitive(true);
        }
        if builder
            .add_line(Some(file_path.to_path_buf()), line_text)
            .is_err()
        {
            continue;
        }

        let matcher = match builder.build() {
            Ok(m) => m,
            Err(_) => continue,
        };

        // Construct full path for matching
        let full_path = file_dir
            .join(rel_path)
            .components()
            .collect::<std::path::PathBuf>();

        let matched = matcher.matched_path_or_any_parents(&full_path, is_dir);

        if matched.is_ignore() {
            // Normal pattern — this is a selection
            last_match = Some(WorktreeincludeStatus::Included {
                file: file_path.to_path_buf(),
                line: line_1based,
                pattern: trimmed.to_string(),
            });
        } else if matched.is_whitelist() {
            // Negation pattern — this de-selects
            last_match = Some(WorktreeincludeStatus::ExcludedByNegation {
                file: file_path.to_path_buf(),
                line: line_1based,
                pattern: trimmed.to_string(),
            });
        }
    }

    last_match
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
}
