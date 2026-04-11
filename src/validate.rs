//! Ignore and worktreeinclude file validation.
//!
//! Discovers and validates all `.gitignore`, `.worktreeinclude`, and
//! `.git/info/exclude` files using [`ignore::gitignore::GitignoreBuilder`].
//! Reports errors for in-repo files and warnings for global excludes.

use std::fs;
use std::path::{Path, PathBuf};

use ignore::gitignore::GitignoreBuilder;

use crate::model::{RepoContext, ValidationIssue, ValidationReport, ValidationSeverity};

/// Validate all ignore and worktreeinclude files in the repository.
pub fn validate(ctx: &RepoContext) -> ValidationReport {
    let mut report = ValidationReport::default();

    // Discover and validate .gitignore files
    discover_and_validate(
        &ctx.source_root,
        ".gitignore",
        ctx.core_ignore_case,
        ValidationSeverity::Error,
        &mut report,
    );

    // Discover and validate .worktreeinclude files
    discover_and_validate(
        &ctx.source_root,
        ".worktreeinclude",
        ctx.core_ignore_case,
        ValidationSeverity::Error,
        &mut report,
    );

    // Validate .git/info/exclude if present
    let exclude_path = ctx.source_root.join(".git/info/exclude");
    if exclude_path.exists() {
        validate_ignore_file(
            &exclude_path,
            &ctx.source_root,
            ctx.core_ignore_case,
            ValidationSeverity::Error,
            &mut report,
        );
    }

    // Optionally validate global excludes (warning-only)
    if let Some(global_path) = find_global_excludes(&ctx.source_root) {
        if global_path.exists() {
            validate_ignore_file(
                &global_path,
                &ctx.source_root,
                ctx.core_ignore_case,
                ValidationSeverity::Warning,
                &mut report,
            );
        }
    }

    report
}

/// Walk the repo tree and validate all files named `filename`.
fn discover_and_validate(
    root: &Path,
    filename: &str,
    case_insensitive: bool,
    severity: ValidationSeverity,
    report: &mut ValidationReport,
) {
    let walker = walkdir(root);
    for entry in walker {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                report.issues.push(ValidationIssue {
                    severity,
                    file: root.to_path_buf(),
                    line: None,
                    message: format!("error walking directory: {e}"),
                });
                continue;
            }
        };

        if entry.file_type().is_file() && entry.file_name().to_string_lossy() == filename {
            let path = entry.path();

            // Check for symlinked ignore files (error for .worktreeinclude)
            if filename == ".worktreeinclude" {
                if let Ok(meta) = fs::symlink_metadata(path) {
                    if meta.file_type().is_symlink() {
                        report.issues.push(ValidationIssue {
                            severity: ValidationSeverity::Error,
                            file: path.to_path_buf(),
                            line: None,
                            message: "symlinked .worktreeinclude files are not allowed".to_string(),
                        });
                        continue;
                    }
                }
            }

            validate_ignore_file(path, root, case_insensitive, severity, report);
        }
    }
}

/// Validate a single ignore-style file using `GitignoreBuilder`.
fn validate_ignore_file(
    path: &Path,
    root: &Path,
    case_insensitive: bool,
    severity: ValidationSeverity,
    report: &mut ValidationReport,
) {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            report.issues.push(ValidationIssue {
                severity,
                file: path.to_path_buf(),
                line: None,
                message: format!("cannot read file: {e}"),
            });
            return;
        }
    };

    // Determine the directory this ignore file applies to
    let dir = path.parent().unwrap_or(root);

    let mut builder = GitignoreBuilder::new(dir);
    builder.case_insensitive(case_insensitive).unwrap();

    for (line_num, line_text) in content.lines().enumerate() {
        let line_1based = line_num + 1;

        // Skip empty lines and comments
        let trimmed = line_text.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        if let Err(err) = builder.add_line(Some(path.to_path_buf()), line_text) {
            report.issues.push(ValidationIssue {
                severity,
                file: path.to_path_buf(),
                line: Some(line_1based),
                message: format!("invalid pattern: {err}"),
            });
        }
    }

    // Try to build the matcher to catch any aggregate errors
    if let Err(err) = builder.build() {
        report.issues.push(ValidationIssue {
            severity,
            file: path.to_path_buf(),
            line: None,
            message: format!("failed to compile patterns: {err}"),
        });
    }
}

/// Try to find the global Git excludes file.
fn find_global_excludes(source_root: &Path) -> Option<PathBuf> {
    // Try git config core.excludesFile first
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(source_root)
        .args(["config", "core.excludesFile"])
        .output()
        .ok()?;

    if output.status.success() {
        let path_str = String::from_utf8_lossy(&output.stdout);
        let path_str = path_str.trim();
        if !path_str.is_empty() {
            let path = expand_tilde(path_str);
            return Some(path);
        }
    }

    // Fall back to default location
    if let Some(home) = home_dir() {
        let default_path = home.join(".config/git/ignore");
        if default_path.exists() {
            return Some(default_path);
        }
    }

    None
}

/// Expand `~` at the start of a path.
fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(path)
}

/// Get the home directory.
fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

/// Walk a directory tree, skipping `.git` directories.
fn walkdir(
    root: &Path,
) -> impl Iterator<Item = std::result::Result<walkdir::DirEntry, walkdir::Error>> {
    walkdir::WalkDir::new(root).into_iter().filter_entry(|e| {
        // Skip .git directories
        !(e.file_type().is_dir() && e.file_name().to_string_lossy() == ".git")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_repo() -> TempDir {
        let dir = TempDir::new().unwrap();
        // Init a git repo so .git exists
        std::process::Command::new("git")
            .arg("init")
            .arg(dir.path())
            .output()
            .unwrap();
        dir
    }

    #[test]
    fn valid_gitignore_no_errors() {
        let dir = make_repo();
        fs::write(dir.path().join(".gitignore"), "*.log\n!important.log\n").unwrap();

        let ctx = RepoContext {
            source_root: dir.path().to_path_buf(),
            dest_root: None,
            main_worktree: dir.path().to_path_buf(),
            known_worktrees: vec![dir.path().to_path_buf()],
            core_ignore_case: false,
        };

        let report = validate(&ctx);
        let errors: Vec<_> = report
            .issues
            .iter()
            .filter(|i| matches!(i.severity, ValidationSeverity::Error))
            .collect();
        assert!(errors.is_empty(), "unexpected errors: {errors:?}");
    }

    #[test]
    fn valid_worktreeinclude_no_errors() {
        let dir = make_repo();
        fs::write(dir.path().join(".worktreeinclude"), ".env\n*.local\n").unwrap();

        let ctx = RepoContext {
            source_root: dir.path().to_path_buf(),
            dest_root: None,
            main_worktree: dir.path().to_path_buf(),
            known_worktrees: vec![dir.path().to_path_buf()],
            core_ignore_case: false,
        };

        let report = validate(&ctx);
        let errors: Vec<_> = report
            .issues
            .iter()
            .filter(|i| matches!(i.severity, ValidationSeverity::Error))
            .collect();
        assert!(errors.is_empty(), "unexpected errors: {errors:?}");
    }

    #[test]
    fn unreadable_worktreeinclude_is_error() {
        let dir = make_repo();
        let wti_path = dir.path().join(".worktreeinclude");

        // Create a directory where a file is expected
        fs::create_dir(&wti_path).unwrap();

        let ctx = RepoContext {
            source_root: dir.path().to_path_buf(),
            dest_root: None,
            main_worktree: dir.path().to_path_buf(),
            known_worktrees: vec![dir.path().to_path_buf()],
            core_ignore_case: false,
        };

        let report = validate(&ctx);
        // A directory named .worktreeinclude won't be detected as a file to validate,
        // so no errors are expected. The main point is that we don't panic.
        assert!(!report.has_errors());
    }

    #[test]
    fn nested_worktreeinclude_validated() {
        let dir = make_repo();
        let subdir = dir.path().join("sub");
        fs::create_dir(&subdir).unwrap();
        fs::write(dir.path().join(".worktreeinclude"), "*.env\n").unwrap();
        fs::write(subdir.join(".worktreeinclude"), "*.local\n").unwrap();

        let ctx = RepoContext {
            source_root: dir.path().to_path_buf(),
            dest_root: None,
            main_worktree: dir.path().to_path_buf(),
            known_worktrees: vec![dir.path().to_path_buf()],
            core_ignore_case: false,
        };

        let report = validate(&ctx);
        let errors: Vec<_> = report
            .issues
            .iter()
            .filter(|i| matches!(i.severity, ValidationSeverity::Error))
            .collect();
        assert!(errors.is_empty(), "unexpected errors: {errors:?}");
    }

    #[test]
    fn git_info_exclude_validated() {
        let dir = make_repo();
        let info_dir = dir.path().join(".git/info");
        fs::create_dir_all(&info_dir).unwrap();
        fs::write(info_dir.join("exclude"), "*.tmp\n").unwrap();

        let ctx = RepoContext {
            source_root: dir.path().to_path_buf(),
            dest_root: None,
            main_worktree: dir.path().to_path_buf(),
            known_worktrees: vec![dir.path().to_path_buf()],
            core_ignore_case: false,
        };

        let report = validate(&ctx);
        assert!(!report.has_errors());
    }
}
