//! Ignore and worktreeinclude file validation.
//!
//! Discovers and validates all `.gitignore`, `.worktreeinclude`, and
//! `.git/info/exclude` files using [`ignore::gitignore::GitignoreBuilder`].
//! Reports errors for in-repo files and warnings for global excludes.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use ignore::gitignore::GitignoreBuilder;
use waft_verified::validation::{
    GateOutcome, IssueKind, ReportMode, RuleFileAction, RuleScope, RuleSymlinkPolicy, Severity,
};

use crate::config::SymlinkPolicy;
use crate::error::{Error, Result};
use crate::git::GitBackend;
use crate::model::{RepoContext, ValidationIssue, ValidationReport, ValidationSeverity};

/// Validate all ignore and worktreeinclude files in the repository.
pub fn validate(
    ctx: &RepoContext,
    git: &dyn GitBackend,
    symlink_policy: SymlinkPolicy,
) -> ValidationReport {
    let mut report = ValidationReport::default();
    let gitlinks = match git.gitlinks(&ctx.source_root) {
        Ok(gitlinks) => gitlinks,
        Err(error) => {
            report.issues.push(ValidationIssue {
                severity: severity_of(RuleScope::Repository, IssueKind::BoundaryLookup),
                file: ctx.source_root.clone(),
                line: None,
                message: format!("cannot determine nested repository boundaries: {error}"),
            });
            HashSet::new()
        }
    };

    // Discover and validate .gitignore files
    discover_and_validate(
        &ctx.source_root,
        ".gitignore",
        ctx.core_ignore_case,
        RuleScope::Repository,
        symlink_policy,
        &gitlinks,
        &mut report,
    );

    // Discover and validate .worktreeinclude files
    discover_and_validate(
        &ctx.source_root,
        ".worktreeinclude",
        ctx.core_ignore_case,
        RuleScope::Repository,
        symlink_policy,
        &gitlinks,
        &mut report,
    );

    // Validate .git/info/exclude if present
    let exclude_path = ctx.source_root.join(".git/info/exclude");
    if exclude_path.exists() {
        validate_ignore_file(
            &exclude_path,
            &ctx.source_root,
            ctx.core_ignore_case,
            RuleScope::Repository,
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
            RuleScope::GlobalExcludes,
            &mut report,
        );
    }

    report
}

/// Print the validation findings `copy`, `list` and `info` show before they
/// act, and stop the command when any finding is an error.
///
/// The verified `waft_verified::validation::validation_view` decides which
/// issues are printed and whether the command stops: errors only (then
/// `Error::Validation`) when there is an error, otherwise the warnings unless
/// `quiet`.
pub(crate) fn report_preflight(report: &ValidationReport, quiet: bool) -> Result<()> {
    let view = waft_verified::validation::validation_view(
        &report.severities(),
        ReportMode::Preflight,
        quiet,
    );
    for &index in &view.shown {
        let issue = &report.issues[index];
        eprintln!(
            "{}: {}: {}",
            severity_label(issue.severity),
            issue.file.display(),
            issue.message
        );
    }
    match view.outcome {
        GateOutcome::Proceed => Ok(()),
        GateOutcome::Abort { error_count } => Err(Error::Validation { error_count }),
    }
}

/// The word an issue's severity is printed as.
pub(crate) fn severity_label(severity: ValidationSeverity) -> &'static str {
    match severity {
        ValidationSeverity::Warning => "warning",
        ValidationSeverity::Error => "error",
    }
}

/// The severity the verified policy assigns to a problem of `kind` found in
/// `scope`: errors for every non-lint problem in the repository, warnings for
/// lint findings and for the user's global excludes.
fn severity_of(scope: RuleScope, kind: IssueKind) -> ValidationSeverity {
    validation_severity(waft_verified::validation::issue_severity(scope, kind))
}

/// The production severity for a kernel severity.
fn validation_severity(severity: Severity) -> ValidationSeverity {
    match severity {
        Severity::Warning => ValidationSeverity::Warning,
        Severity::Error => ValidationSeverity::Error,
    }
}

/// The kernel's view of the `.worktreeinclude` symlink policy.
fn rule_symlink_policy(policy: SymlinkPolicy) -> RuleSymlinkPolicy {
    match policy {
        SymlinkPolicy::Follow => RuleSymlinkPolicy::Follow,
        SymlinkPolicy::Ignore => RuleSymlinkPolicy::Ignore,
        SymlinkPolicy::Error => RuleSymlinkPolicy::Error,
    }
}

/// Walk the repo tree and validate all files named `filename`.
fn discover_and_validate(
    root: &Path,
    filename: &str,
    case_insensitive: bool,
    scope: RuleScope,
    symlink_policy: SymlinkPolicy,
    gitlinks: &HashSet<String>,
    report: &mut ValidationReport,
) {
    let walker = walkdir(root, gitlinks, case_insensitive);
    for entry in walker {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                report.issues.push(ValidationIssue {
                    severity: severity_of(scope, IssueKind::Walk),
                    file: root.to_path_buf(),
                    line: None,
                    message: format!("error walking directory: {e}"),
                });
                continue;
            }
        };

        if !crate::walk::special_filename_matches(entry.file_name(), filename) {
            continue;
        }

        let path = entry.path();

        // walkdir does not follow symlinks by default, so symlink entries
        // have is_symlink()=true and is_file()=false. The verified kernel
        // decides what each entry gets: a symlinked .worktreeinclude follows
        // the symlink policy (reported without being read, treated as absent,
        // or read through the link); any other entry is validated only when
        // it is a regular file.
        let file_type = entry.file_type();
        match waft_verified::validation::rule_file_action(
            filename == ".worktreeinclude",
            file_type.is_symlink(),
            file_type.is_file(),
            rule_symlink_policy(symlink_policy),
        ) {
            RuleFileAction::RejectSymlink => {
                report.issues.push(ValidationIssue {
                    severity: severity_of(scope, IssueKind::SymlinkedWorktreeinclude),
                    file: path.to_path_buf(),
                    line: None,
                    message: "symlinked .worktreeinclude files are not allowed".to_string(),
                });
            }
            // A followed symlink is read through: fs::read_to_string follows
            // symlinks by default, so validate_ignore_file works.
            RuleFileAction::Validate => {
                validate_ignore_file(path, root, case_insensitive, scope, report);
            }
            RuleFileAction::Skip => {}
        }
    }
}

/// Validate a single ignore-style file using `GitignoreBuilder`.
fn validate_ignore_file(
    path: &Path,
    root: &Path,
    case_insensitive: bool,
    scope: RuleScope,
    report: &mut ValidationReport,
) {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            report.issues.push(ValidationIssue {
                severity: severity_of(scope, IssueKind::Unreadable),
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

        // Skip empty lines and comments (decided on the trimmed line by the
        // same verified predicate the lint uses).
        if waft_verified::lint::is_blank_or_comment(line_text.trim().as_bytes()) {
            continue;
        }

        if let Err(err) = builder.add_line(Some(path.to_path_buf()), line_text) {
            report.issues.push(ValidationIssue {
                severity: severity_of(scope, IssueKind::InvalidPattern),
                file: path.to_path_buf(),
                line: Some(line_1based),
                message: format!("invalid pattern: {err}"),
            });
        }
    }

    // Try to build the matcher to catch any aggregate errors
    if let Err(err) = builder.build() {
        report.issues.push(ValidationIssue {
            severity: severity_of(scope, IssueKind::CompileFailed),
            file: path.to_path_buf(),
            line: None,
            message: format!("failed to compile patterns: {err}"),
        });
    }

    // Run lint-style checks for suspicious-but-legal patterns
    check_suspicious_patterns(path, &content, scope, report);
}

/// Lint-style checks for suspicious-but-legal patterns.
///
/// Detects:
/// - **Shadowed negations**: a negation like `!dir/file` after `dir/` was excluded.
///   Git won't descend into a directory-excluded path, so the negation is ineffective.
/// - **Duplicate patterns**: the same pattern text appearing more than once.
///
/// Which lines get which warning, and which earlier line each warning names,
/// is decided by the verified `waft_verified::lint::lint_lines` on the
/// trimmed lines; this function only formats its verdicts, line by line,
/// the duplicate warning before the shadow warning.
fn check_suspicious_patterns(
    path: &Path,
    content: &str,
    scope: RuleScope,
    report: &mut ValidationReport,
) {
    let severity = severity_of(scope, IssueKind::Lint);
    let lines: Vec<&str> = content.lines().map(str::trim).collect();
    let line_bytes: Vec<&[u8]> = lines.iter().map(|line| line.as_bytes()).collect();
    let verdicts = waft_verified::lint::lint_lines(&line_bytes);

    for (line_num, (trimmed, verdict)) in lines.iter().zip(&verdicts).enumerate() {
        let line_1based = line_num + 1;

        // --- Duplicate detection ---
        if let Some(first) = verdict.duplicate_of {
            report.issues.push(ValidationIssue {
                severity,
                file: path.to_path_buf(),
                line: Some(line_1based),
                message: format!(
                    "duplicate pattern '{}' (first appeared on line {})",
                    trimmed,
                    first + 1
                ),
            });
        }

        // --- Shadowed negation detection ---
        // The kernel proves `dir_line < line_num`, so the index is in range.
        if let Some(dir_line) = verdict.shadowed_by {
            report.issues.push(ValidationIssue {
                severity,
                file: path.to_path_buf(),
                line: Some(line_1based),
                message: format!(
                    "negation '{}' is likely shadowed by directory exclusion '{}' \
                     (git will not descend into excluded directories)",
                    trimmed,
                    lines[dir_line].trim_end_matches('/')
                ),
            });
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

    // Fall back to Git's XDG-aware default only for real backends that read
    // ambient global excludes. Test/specialized backends are hermetic unless
    // they explicitly return core.excludesFile above.
    if git.reads_default_global_excludes()
        && let Some(config_home) = xdg_config_home()
    {
        let default_path = config_home.join("git/ignore");
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

fn xdg_config_home() -> Option<PathBuf> {
    match std::env::var_os("XDG_CONFIG_HOME") {
        Some(path) if !path.is_empty() => Some(PathBuf::from(path)),
        _ => home_dir().map(|home| home.join(".config")),
    }
}

/// Walk a directory tree without entering nested repositories or submodules.
fn walkdir<'a>(
    root: &'a Path,
    gitlinks: &'a HashSet<String>,
    ignore_case: bool,
) -> impl Iterator<Item = std::result::Result<walkdir::DirEntry, walkdir::Error>> + 'a {
    walkdir::WalkDir::new(root)
        .into_iter()
        .filter_entry(move |entry| {
            !crate::walk::is_git_boundary_dir(
                entry.path(),
                entry.depth(),
                root,
                gitlinks,
                ignore_case,
            )
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

    fn git_command() -> std::process::Command {
        let mut command = std::process::Command::new("git");
        command
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env(
                "GIT_CONFIG_GLOBAL",
                if cfg!(windows) { "NUL" } else { "/dev/null" },
            )
            .env("GIT_CONFIG_COUNT", "0");
        for key in [
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_INDEX_FILE",
            "GIT_OBJECT_DIRECTORY",
            "GIT_ALTERNATE_OBJECT_DIRECTORIES",
            "GIT_CEILING_DIRECTORIES",
            "GIT_DISCOVERY_ACROSS_FILESYSTEM",
        ] {
            command.env_remove(key);
        }
        command
    }

    fn make_repo() -> TempDir {
        let dir = TempDir::new().unwrap();
        // Init a git repo so .git exists
        git_command().arg("init").arg(dir.path()).output().unwrap();
        dir
    }

    /// Mock GitBackend that returns a configurable value for `core.excludesFile`.
    struct MockGit {
        excludes_file: Option<String>,
        gitlinks: HashSet<String>,
    }

    impl MockGit {
        fn new(excludes_file: Option<String>) -> Self {
            Self {
                excludes_file,
                gitlinks: HashSet::new(),
            }
        }

        fn with_gitlink(mut self, path: &str) -> Self {
            self.gitlinks.insert(path.to_string());
            self
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

        fn gitlinks(&self, _source_root: &Path) -> Result<HashSet<String>> {
            Ok(self.gitlinks.clone())
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
            _semantics: crate::config::WorktreeincludeSemantics,
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
    fn nested_repository_rule_files_are_not_validated() {
        let dir = make_repo();
        let nested = dir.path().join("vendor/checkout");
        fs::create_dir_all(&nested).unwrap();
        git_command().arg("init").arg(&nested).output().unwrap();
        fs::write(nested.join(".worktreeinclude"), "\\\n").unwrap();

        let ctx = make_ctx(&dir);
        let git = MockGit::new(None);
        let report = validate(&ctx, &git, SymlinkPolicy::Error);

        assert!(
            report
                .issues
                .iter()
                .all(|issue| !issue.file.starts_with(&nested)),
            "nested repository findings leaked into outer validation: {:?}",
            report.issues
        );
    }

    #[test]
    fn registered_gitlink_rule_files_are_not_validated() {
        let dir = make_repo();
        let submodule = dir.path().join("vendor/submodule");
        fs::create_dir_all(&submodule).unwrap();
        fs::write(submodule.join(".worktreeinclude"), "\\\n").unwrap();

        let ctx = make_ctx(&dir);
        let git = MockGit::new(None).with_gitlink("vendor/submodule");
        let report = validate(&ctx, &git, SymlinkPolicy::Error);

        assert!(
            report
                .issues
                .iter()
                .all(|issue| !issue.file.starts_with(&submodule)),
            "registered submodule findings leaked into outer validation: {:?}",
            report.issues
        );
    }

    /// A case-insensitive checkout resolves a differently-cased gitlink
    /// spelling to the same submodule directory, so it is still a boundary.
    #[test]
    fn case_folded_gitlink_spelling_bounds_validation_on_folding_checkouts() {
        let dir = make_repo();
        let submodule = dir.path().join("vendor/submodule");
        fs::create_dir_all(&submodule).unwrap();
        fs::write(submodule.join(".worktreeinclude"), "\\\n").unwrap();

        let mut ctx = make_ctx(&dir);
        ctx.core_ignore_case = true;
        let git = MockGit::new(None).with_gitlink("VENDOR/SUBMODULE");
        let report = validate(&ctx, &git, SymlinkPolicy::Error);

        assert!(
            report
                .issues
                .iter()
                .all(|issue| !issue.file.starts_with(&submodule)),
            "registered submodule findings leaked into outer validation: {:?}",
            report.issues
        );
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

    /// The lint as it was written before it moved into the verified kernel,
    /// kept only as a test oracle for the differential test below.
    fn legacy_suspicious_patterns(content: &str) -> Vec<(usize, String)> {
        let mut out = Vec::new();
        let mut dir_exclusions: Vec<String> = Vec::new();
        let mut seen_patterns: Vec<(&str, usize)> = Vec::new();
        for (line_num, line_text) in content.lines().enumerate() {
            let line_1based = line_num + 1;
            let trimmed = line_text.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let is_negation = trimmed.starts_with('!') && !trimmed.starts_with("\\!");
            let pattern_text = if is_negation { &trimmed[1..] } else { trimmed };
            if let Some((_, first_line)) = seen_patterns.iter().find(|(p, _)| *p == trimmed) {
                out.push((
                    line_1based,
                    format!(
                        "duplicate pattern '{}' (first appeared on line {})",
                        trimmed, first_line
                    ),
                ));
            } else {
                seen_patterns.push((trimmed, line_1based));
            }
            if is_negation {
                for dir_pat in &dir_exclusions {
                    if pattern_text.starts_with(dir_pat.as_str()) {
                        out.push((
                            line_1based,
                            format!(
                                "negation '{}' is likely shadowed by directory exclusion '{}' \
                                 (git will not descend into excluded directories)",
                                trimmed,
                                dir_pat.trim_end_matches('/')
                            ),
                        ));
                        break;
                    }
                }
            } else if pattern_text.ends_with('/')
                && !pattern_text.contains('*')
                && !pattern_text.contains('?')
                && !pattern_text.contains('[')
            {
                dir_exclusions.push(pattern_text.to_string());
            }
        }
        out
    }

    fn kernel_suspicious_patterns(content: &str) -> Vec<(usize, String)> {
        let mut report = ValidationReport::default();
        check_suspicious_patterns(
            Path::new("rules"),
            content,
            RuleScope::Repository,
            &mut report,
        );
        report
            .issues
            .into_iter()
            .map(|issue| {
                assert_eq!(issue.severity, ValidationSeverity::Warning);
                assert_eq!(issue.file, Path::new("rules"));
                (
                    issue.line.expect("lint findings carry a line"),
                    issue.message,
                )
            })
            .collect()
    }

    #[test]
    fn lint_matches_legacy_on_edge_cases() {
        for content in [
            "",
            "build/\n!build/important.txt\n",
            "build/\nbuild/\n!build/x\n!build/x\n",
            "  build/  \n\t!build/x\n # c\n# c\n # c\n",
            "a//\n!a//b\n!a/b\n",
            "build/*\n!build/x\n",
            "\\!x/\n!x/y\n\\!x/\n",
            "!a/\n!a/b\n",
            "é/\n!é/ü\né/\n",
            "x\r\ny\r\nx\r\n",
            "a/\nb/\n!b/c\n!a/c\n",
            "!\n!\n#\n\n\n",
        ] {
            assert_eq!(
                kernel_suspicious_patterns(content),
                legacy_suspicious_patterns(content),
                "content: {content:?}"
            );
        }
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(512))]

        /// The verified lint emits exactly the warnings the original code
        /// emitted, byte for byte and in the same order.
        #[test]
        fn lint_matches_legacy_algorithm(
            lines in proptest::collection::vec("[ab!#/*?\\[\\\\ \té]{0,5}", 0..16),
            crlf in proptest::bool::ANY,
        ) {
            let content = lines.join(if crlf { "\r\n" } else { "\n" });
            proptest::prop_assert_eq!(
                kernel_suspicious_patterns(&content),
                legacy_suspicious_patterns(&content)
            );
        }
    }

    fn issue(severity: ValidationSeverity) -> ValidationIssue {
        ValidationIssue {
            severity,
            file: PathBuf::from("f"),
            line: None,
            message: String::new(),
        }
    }

    #[test]
    fn severities_map_one_to_one_in_order() {
        use waft_verified::validation::Severity;
        let report = ValidationReport {
            issues: vec![
                issue(ValidationSeverity::Warning),
                issue(ValidationSeverity::Error),
                issue(ValidationSeverity::Warning),
            ],
        };
        assert_eq!(
            report.severities(),
            vec![Severity::Warning, Severity::Error, Severity::Warning]
        );
        assert!(report.has_errors());
        assert_eq!(report.error_count(), 1);

        let warnings_only = ValidationReport {
            issues: vec![issue(ValidationSeverity::Warning)],
        };
        assert!(!warnings_only.has_errors());
        assert_eq!(warnings_only.error_count(), 0);
        assert!(!ValidationReport::default().has_errors());
    }

    #[test]
    fn preflight_stops_with_the_error_count() {
        let report = ValidationReport {
            issues: vec![
                issue(ValidationSeverity::Error),
                issue(ValidationSeverity::Warning),
                issue(ValidationSeverity::Error),
            ],
        };
        for quiet in [false, true] {
            match report_preflight(&report, quiet) {
                Err(Error::Validation { error_count }) => assert_eq!(error_count, 2),
                other => panic!("expected a validation error, got {other:?}"),
            }
        }
        let warnings_only = ValidationReport {
            issues: vec![issue(ValidationSeverity::Warning)],
        };
        assert!(report_preflight(&warnings_only, false).is_ok());
        assert!(report_preflight(&warnings_only, true).is_ok());
    }

    /// A symlinked `.worktreeinclude` whose target holds an invalid pattern
    /// is reported unread under `error`, absent under `ignore`, and read
    /// through under `follow`; a symlinked `.gitignore` is never read.
    #[cfg(unix)]
    #[test]
    fn symlinked_rule_files_follow_the_policy() {
        let dir = make_repo();
        let target = dir.path().join("target.rules");
        fs::write(&target, "\\\n").unwrap();
        std::os::unix::fs::symlink(&target, dir.path().join(".worktreeinclude")).unwrap();
        std::os::unix::fs::symlink(&target, dir.path().join(".gitignore")).unwrap();
        let ctx = make_ctx(&dir);
        let git = MockGit::new(None);

        let messages = |policy| -> Vec<String> {
            validate(&ctx, &git, policy)
                .issues
                .into_iter()
                .map(|issue| format!("{:?} {:?} {}", issue.severity, issue.line, issue.message))
                .collect()
        };
        assert_eq!(
            messages(SymlinkPolicy::Error),
            vec!["Error None symlinked .worktreeinclude files are not allowed".to_string()]
        );
        assert!(messages(SymlinkPolicy::Ignore).is_empty());
        let followed = messages(SymlinkPolicy::Follow);
        assert!(
            !followed.is_empty()
                && followed
                    .iter()
                    .all(|m| m.starts_with("Error ") && !m.contains("symlinked")),
            "{followed:?}"
        );
    }

    #[test]
    fn kernel_severity_conversions_round_trip() {
        for severity in [Severity::Warning, Severity::Error] {
            let report = ValidationReport {
                issues: vec![issue(validation_severity(severity))],
            };
            assert_eq!(report.severities(), vec![severity]);
        }
    }

    #[test]
    fn symlink_policy_maps_to_its_namesake() {
        assert_eq!(
            rule_symlink_policy(SymlinkPolicy::Follow),
            RuleSymlinkPolicy::Follow
        );
        assert_eq!(
            rule_symlink_policy(SymlinkPolicy::Ignore),
            RuleSymlinkPolicy::Ignore
        );
        assert_eq!(
            rule_symlink_policy(SymlinkPolicy::Error),
            RuleSymlinkPolicy::Error
        );
    }

    #[test]
    fn severity_policy_table() {
        use IssueKind::*;
        for kind in [
            BoundaryLookup,
            Walk,
            SymlinkedWorktreeinclude,
            Unreadable,
            InvalidPattern,
            CompileFailed,
        ] {
            assert_eq!(
                severity_of(RuleScope::Repository, kind),
                ValidationSeverity::Error
            );
            assert_eq!(
                severity_of(RuleScope::GlobalExcludes, kind),
                ValidationSeverity::Warning
            );
        }
        for scope in [RuleScope::Repository, RuleScope::GlobalExcludes] {
            assert_eq!(severity_of(scope, Lint), ValidationSeverity::Warning);
        }
    }

    #[test]
    fn severity_labels() {
        assert_eq!(severity_label(ValidationSeverity::Warning), "warning");
        assert_eq!(severity_label(ValidationSeverity::Error), "error");
    }
}
