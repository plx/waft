//! Ignore and worktreeinclude file validation.
//!
//! Discovers and validates all `.gitignore`, `.worktreeinclude`, and
//! `.git/info/exclude` files using [`ignore::gitignore::GitignoreBuilder`].
//! Reports errors for in-repo files and warnings for global excludes.

use std::fs;
use std::path::{Path, PathBuf};

use ignore::gitignore::GitignoreBuilder;

use crate::config::SymlinkPolicy;
use crate::git::GitBackend;
use crate::model::{RepoContext, ValidationIssue, ValidationReport, ValidationSeverity};

/// Validate all ignore and worktreeinclude files in the repository.
pub fn validate(
    ctx: &RepoContext,
    git: &dyn GitBackend,
    symlink_policy: SymlinkPolicy,
) -> ValidationReport {
    let mut report = ValidationReport::default();

    // Discover and validate .gitignore files
    discover_and_validate(
        &ctx.source_root,
        ".gitignore",
        ctx.core_ignore_case,
        ValidationSeverity::Error,
        symlink_policy,
        &mut report,
    );

    // Discover and validate .worktreeinclude files
    discover_and_validate(
        &ctx.source_root,
        ".worktreeinclude",
        ctx.core_ignore_case,
        ValidationSeverity::Error,
        symlink_policy,
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
    if let Some(global_path) = find_global_excludes(&ctx.source_root, git)
        && global_path.exists()
    {
        validate_ignore_file(
            &global_path,
            &ctx.source_root,
            ctx.core_ignore_case,
            ValidationSeverity::Warning,
            &mut report,
        );
    }

    report
}

/// Walk the repo tree and validate all files named `filename`.
fn discover_and_validate(
    root: &Path,
    filename: &str,
    case_insensitive: bool,
    severity: ValidationSeverity,
    symlink_policy: SymlinkPolicy,
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

        if entry.file_name().to_string_lossy() != filename {
            continue;
        }

        let path = entry.path();

        // walkdir does not follow symlinks by default, so symlink entries
        // have is_symlink()=true and is_file()=false. Handle them explicitly
        // before falling through to the regular-file validation path.
        if filename == ".worktreeinclude" && entry.file_type().is_symlink() {
            match symlink_policy {
                SymlinkPolicy::Error => {
                    report.issues.push(ValidationIssue {
                        severity: ValidationSeverity::Error,
                        file: path.to_path_buf(),
                        line: None,
                        message: "symlinked .worktreeinclude files are not allowed".to_string(),
                    });
                    continue;
                }
                SymlinkPolicy::Ignore => {
                    // Treat as if absent — neither validate nor report.
                    continue;
                }
                SymlinkPolicy::Follow => {
                    // Read through the symlink. fs::read_to_string follows
                    // symlinks by default, so validate_ignore_file works.
                    validate_ignore_file(path, root, case_insensitive, severity, report);
                    continue;
                }
            }
        }

        if entry.file_type().is_file() {
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

    // Run lint-style checks for suspicious-but-legal patterns
    check_suspicious_patterns(path, &content, report);
}

/// Lint-style checks for suspicious-but-legal patterns.
///
/// Detects:
/// - **Shadowed negations**: a negation like `!dir/file` after `dir/` was excluded.
///   Git won't descend into a directory-excluded path, so the negation is ineffective.
/// - **Duplicate patterns**: the same pattern text appearing more than once.
fn check_suspicious_patterns(path: &Path, content: &str, report: &mut ValidationReport) {
    // Track literal directory exclusion patterns (e.g. "build/", "vendor/pkg/")
    let mut dir_exclusions: Vec<String> = Vec::new();
    // Track seen patterns for duplicate detection (pattern text → first line number)
    let mut seen_patterns: Vec<(&str, usize)> = Vec::new();

    for (line_num, line_text) in content.lines().enumerate() {
        let line_1based = line_num + 1;
        let trimmed = line_text.trim();

        // Skip empty lines and comments
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        // Skip lines with escaped leading characters (e.g. `\!pattern`, `\#pattern`)
        let is_negation = trimmed.starts_with('!') && !trimmed.starts_with("\\!");
        let pattern_text = if is_negation { &trimmed[1..] } else { trimmed };

        // --- Duplicate detection ---
        if let Some((_, first_line)) = seen_patterns.iter().find(|(p, _)| *p == trimmed) {
            report.issues.push(ValidationIssue {
                severity: ValidationSeverity::Warning,
                file: path.to_path_buf(),
                line: Some(line_1based),
                message: format!(
                    "duplicate pattern '{}' (first appeared on line {})",
                    trimmed, first_line
                ),
            });
        } else {
            seen_patterns.push((trimmed, line_1based));
        }

        // --- Shadowed negation detection ---
        if is_negation {
            for dir_pat in &dir_exclusions {
                if pattern_text.starts_with(dir_pat.as_str()) {
                    report.issues.push(ValidationIssue {
                        severity: ValidationSeverity::Warning,
                        file: path.to_path_buf(),
                        line: Some(line_1based),
                        message: format!(
                            "negation '{}' is likely shadowed by directory exclusion '{}' \
                             (git will not descend into excluded directories)",
                            trimmed,
                            dir_pat.trim_end_matches('/')
                        ),
                    });
                    break; // One warning per negation is enough
                }
            }
        } else {
            // Track literal directory exclusion patterns (ending with `/`,
            // no glob wildcards). Patterns with `*`, `?`, or `[` are not
            // simple directory exclusions — e.g. `build/*` still allows
            // git to descend into `build/`.
            if pattern_text.ends_with('/')
                && !pattern_text.contains('*')
                && !pattern_text.contains('?')
                && !pattern_text.contains('[')
            {
                dir_exclusions.push(pattern_text.to_string());
            }
        }
    }
}

/// Try to find the global Git excludes file.
fn find_global_excludes(source_root: &Path, git: &dyn GitBackend) -> Option<PathBuf> {
    // Try git config core.excludesFile via the backend
    if let Ok(Some(path_str)) = git.read_config(source_root, "core.excludesFile")
        && !path_str.is_empty()
    {
        let path = expand_tilde(&path_str);
        return Some(path);
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
    if let Some(rest) = path.strip_prefix("~/")
        && let Some(home) = home_dir()
    {
        return home.join(rest);
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
    use crate::error::Result;
    use crate::git::{GitBackend, IgnoreCheckRecord, WorktreeRecord};
    use crate::path::RepoRelPath;
    use std::collections::HashSet;
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

    /// Mock GitBackend that returns a configurable value for `core.excludesFile`.
    struct MockGit {
        excludes_file: Option<String>,
    }

    impl MockGit {
        fn new(excludes_file: Option<String>) -> Self {
            Self { excludes_file }
        }
    }

    impl GitBackend for MockGit {
        fn show_toplevel(&self, _path: &Path) -> Result<PathBuf> {
            unimplemented!()
        }
        fn list_worktrees(&self, _source_root: &Path) -> Result<Vec<WorktreeRecord>> {
            unimplemented!()
        }
        fn tracked_paths(
            &self,
            _source_root: &Path,
            _paths: &[RepoRelPath],
        ) -> Result<HashSet<RepoRelPath>> {
            unimplemented!()
        }
        fn check_ignore(
            &self,
            _source_root: &Path,
            _paths: &[RepoRelPath],
        ) -> Result<Vec<IgnoreCheckRecord>> {
            unimplemented!()
        }
        fn list_worktreeinclude_candidates(
            &self,
            _source_root: &Path,
            _symlink_policy: crate::config::SymlinkPolicy,
        ) -> Result<Vec<RepoRelPath>> {
            unimplemented!()
        }
        fn list_ignored_untracked(&self, _source_root: &Path) -> Result<Vec<RepoRelPath>> {
            unimplemented!()
        }
        fn worktreeinclude_exists_anywhere(
            &self,
            _source_root: &Path,
            _symlink_policy: crate::config::SymlinkPolicy,
        ) -> Result<bool> {
            unimplemented!()
        }
        fn read_bool_config(&self, _source_root: &Path, _key: &str) -> Result<bool> {
            unimplemented!()
        }
        fn read_config(&self, _source_root: &Path, key: &str) -> Result<Option<String>> {
            if key == "core.excludesFile" {
                Ok(self.excludes_file.clone())
            } else {
                Ok(None)
            }
        }
    }

    fn make_ctx(dir: &TempDir) -> RepoContext {
        RepoContext {
            source_root: dir.path().to_path_buf(),
            dest_root: None,
            main_worktree: dir.path().to_path_buf(),
            known_worktrees: vec![dir.path().to_path_buf()],
            core_ignore_case: false,
        }
    }

    #[test]
    fn valid_gitignore_no_errors() {
        let dir = make_repo();
        fs::write(dir.path().join(".gitignore"), "*.log\n!important.log\n").unwrap();
        let ctx = make_ctx(&dir);
        let git = MockGit::new(None);

        let report = validate(&ctx, &git, SymlinkPolicy::Error);
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
        let ctx = make_ctx(&dir);
        let git = MockGit::new(None);

        let report = validate(&ctx, &git, SymlinkPolicy::Error);
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
        fs::create_dir(&wti_path).unwrap();
        let ctx = make_ctx(&dir);
        let git = MockGit::new(None);

        let report = validate(&ctx, &git, SymlinkPolicy::Error);
        assert!(!report.has_errors());
    }

    #[test]
    fn nested_worktreeinclude_validated() {
        let dir = make_repo();
        let subdir = dir.path().join("sub");
        fs::create_dir(&subdir).unwrap();
        fs::write(dir.path().join(".worktreeinclude"), "*.env\n").unwrap();
        fs::write(subdir.join(".worktreeinclude"), "*.local\n").unwrap();
        let ctx = make_ctx(&dir);
        let git = MockGit::new(None);

        let report = validate(&ctx, &git, SymlinkPolicy::Error);
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
        let ctx = make_ctx(&dir);
        let git = MockGit::new(None);

        let report = validate(&ctx, &git, SymlinkPolicy::Error);
        assert!(!report.has_errors());
    }

    #[test]
    fn global_excludes_from_backend_config() {
        let dir = make_repo();
        // Create a global excludes file via the mock backend
        let global_file = dir.path().join("my-global-ignore");
        fs::write(&global_file, "*.log\n").unwrap();
        let ctx = make_ctx(&dir);
        let git = MockGit::new(Some(global_file.to_string_lossy().into_owned()));

        let report = validate(&ctx, &git, SymlinkPolicy::Error);
        // Valid patterns should produce no warnings
        let warnings: Vec<_> = report
            .issues
            .iter()
            .filter(|i| matches!(i.severity, ValidationSeverity::Warning))
            .collect();
        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
    }

    #[test]
    fn global_excludes_invalid_pattern_is_warning() {
        let dir = make_repo();
        let global_file = dir.path().join("my-global-ignore");
        // Write a file with an invalid glob pattern (dangling backslash)
        fs::write(&global_file, "\\\n").unwrap();
        let ctx = make_ctx(&dir);
        let git = MockGit::new(Some(global_file.to_string_lossy().into_owned()));

        let report = validate(&ctx, &git, SymlinkPolicy::Error);
        let warnings: Vec<_> = report
            .issues
            .iter()
            .filter(|i| matches!(i.severity, ValidationSeverity::Warning))
            .collect();
        assert!(
            !warnings.is_empty(),
            "expected a warning for invalid global excludes pattern"
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_worktreeinclude_is_error() {
        let dir = make_repo();
        let real_path = dir.path().join("real.wti");
        fs::write(&real_path, "*.env\n").unwrap();
        std::os::unix::fs::symlink(&real_path, dir.path().join(".worktreeinclude")).unwrap();

        let ctx = make_ctx(&dir);
        let git = MockGit::new(None);
        let report = validate(&ctx, &git, SymlinkPolicy::Error);

        let errors: Vec<_> = report
            .issues
            .iter()
            .filter(|i| {
                matches!(i.severity, ValidationSeverity::Error)
                    && i.message.contains("symlinked .worktreeinclude")
            })
            .collect();
        assert!(
            !errors.is_empty(),
            "expected an error for symlinked .worktreeinclude, got: {:?}",
            report.issues
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_worktreeinclude_in_subdir_is_error() {
        let dir = make_repo();
        let subdir = dir.path().join("sub");
        fs::create_dir(&subdir).unwrap();
        let real_path = subdir.join("real.wti");
        fs::write(&real_path, "*.env\n").unwrap();
        std::os::unix::fs::symlink(&real_path, subdir.join(".worktreeinclude")).unwrap();

        let ctx = make_ctx(&dir);
        let git = MockGit::new(None);
        let report = validate(&ctx, &git, SymlinkPolicy::Error);

        let errors: Vec<_> = report
            .issues
            .iter()
            .filter(|i| {
                matches!(i.severity, ValidationSeverity::Error)
                    && i.message.contains("symlinked .worktreeinclude")
            })
            .collect();
        assert!(
            !errors.is_empty(),
            "expected an error for symlinked .worktreeinclude in subdir, got: {:?}",
            report.issues
        );
    }

    #[test]
    fn shadowed_negation_produces_warning() {
        let dir = make_repo();
        // `build/` excludes the directory; `!build/important.txt` is a negation
        // that git will never evaluate because it won't descend into build/.
        fs::write(
            dir.path().join(".gitignore"),
            "build/\n!build/important.txt\n",
        )
        .unwrap();
        let ctx = make_ctx(&dir);
        let git = MockGit::new(None);

        let report = validate(&ctx, &git, SymlinkPolicy::Error);
        let warnings: Vec<_> = report
            .issues
            .iter()
            .filter(|i| {
                matches!(i.severity, ValidationSeverity::Warning) && i.message.contains("shadowed")
            })
            .collect();
        assert!(
            !warnings.is_empty(),
            "expected a shadowed negation warning, got: {:?}",
            report.issues
        );
        // The warning should point to line 2 (the negation)
        assert_eq!(warnings[0].line, Some(2));
    }

    #[test]
    fn shadowed_negation_nested_path() {
        let dir = make_repo();
        // `vendor/` excludes the directory; `!vendor/lib/keep.so` is shadowed
        fs::write(
            dir.path().join(".gitignore"),
            "vendor/\n!vendor/lib/keep.so\n",
        )
        .unwrap();
        let ctx = make_ctx(&dir);
        let git = MockGit::new(None);

        let report = validate(&ctx, &git, SymlinkPolicy::Error);
        let warnings: Vec<_> = report
            .issues
            .iter()
            .filter(|i| {
                matches!(i.severity, ValidationSeverity::Warning) && i.message.contains("shadowed")
            })
            .collect();
        assert!(
            !warnings.is_empty(),
            "expected a shadowed negation warning for nested path"
        );
    }

    #[test]
    fn wildcard_exclusion_not_shadowed() {
        let dir = make_repo();
        // `build/*` uses a wildcard (not a directory exclusion), so git still
        // descends into build/ and the negation `!build/important.txt` works.
        fs::write(
            dir.path().join(".gitignore"),
            "build/*\n!build/important.txt\n",
        )
        .unwrap();
        let ctx = make_ctx(&dir);
        let git = MockGit::new(None);

        let report = validate(&ctx, &git, SymlinkPolicy::Error);
        let warnings: Vec<_> = report
            .issues
            .iter()
            .filter(|i| {
                matches!(i.severity, ValidationSeverity::Warning) && i.message.contains("shadowed")
            })
            .collect();
        assert!(
            warnings.is_empty(),
            "wildcard exclusion should not trigger shadowed warning: {warnings:?}"
        );
    }

    #[test]
    fn duplicate_pattern_produces_warning() {
        let dir = make_repo();
        fs::write(dir.path().join(".gitignore"), "*.log\n*.tmp\n*.log\n").unwrap();
        let ctx = make_ctx(&dir);
        let git = MockGit::new(None);

        let report = validate(&ctx, &git, SymlinkPolicy::Error);
        let warnings: Vec<_> = report
            .issues
            .iter()
            .filter(|i| {
                matches!(i.severity, ValidationSeverity::Warning) && i.message.contains("duplicate")
            })
            .collect();
        assert!(
            !warnings.is_empty(),
            "expected a duplicate pattern warning, got: {:?}",
            report.issues
        );
        // The warning should point to line 3 (the duplicate)
        assert_eq!(warnings[0].line, Some(3));
    }

    #[test]
    fn no_warnings_for_clean_patterns() {
        let dir = make_repo();
        // All patterns are unique and no shadowed negations
        fs::write(dir.path().join(".gitignore"), "*.log\n*.tmp\ntarget/\n").unwrap();
        let ctx = make_ctx(&dir);
        let git = MockGit::new(None);

        let report = validate(&ctx, &git, SymlinkPolicy::Error);
        let warnings: Vec<_> = report
            .issues
            .iter()
            .filter(|i| matches!(i.severity, ValidationSeverity::Warning))
            .collect();
        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
    }

    #[test]
    fn suspicious_patterns_in_worktreeinclude() {
        let dir = make_repo();
        // Suspicious patterns should also be detected in .worktreeinclude files
        fs::write(
            dir.path().join(".worktreeinclude"),
            "secrets/\n!secrets/public.key\n",
        )
        .unwrap();
        let ctx = make_ctx(&dir);
        let git = MockGit::new(None);

        let report = validate(&ctx, &git, SymlinkPolicy::Error);
        let warnings: Vec<_> = report
            .issues
            .iter()
            .filter(|i| {
                matches!(i.severity, ValidationSeverity::Warning) && i.message.contains("shadowed")
            })
            .collect();
        assert!(
            !warnings.is_empty(),
            "expected a shadowed negation warning in .worktreeinclude"
        );
    }

    #[test]
    fn global_excludes_none_when_config_unset() {
        let dir = make_repo();
        let ctx = make_ctx(&dir);
        let git = MockGit::new(None);

        // No global excludes configured, no warnings expected
        let report = validate(&ctx, &git, SymlinkPolicy::Error);
        let warnings: Vec<_> = report
            .issues
            .iter()
            .filter(|i| matches!(i.severity, ValidationSeverity::Warning))
            .collect();
        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
    }
}
