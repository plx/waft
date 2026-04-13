//! Property and differential tests comparing waft behavior against Git.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::process;
use tempfile::TempDir;

use proptest::prelude::*;

fn git(dir: &Path, args: &[&str]) -> bool {
    process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn write_file(dir: &Path, rel_path: &str, content: &str) {
    let path = dir.join(rel_path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&path, content).unwrap();
}

fn make_repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    git(dir.path(), &["init"]);
    git(dir.path(), &["config", "user.email", "test@test.com"]);
    git(dir.path(), &["config", "user.name", "Test"]);
    dir
}

fn waft_list(dir: &Path) -> BTreeSet<String> {
    let output = process::Command::new(env!("CARGO_BIN_EXE_waft"))
        .args(["list", "--source", dir.to_str().unwrap()])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| l.to_string())
        .collect()
}

/// Query git ls-files for worktreeinclude candidates, then filter through
/// git check-ignore — the same algorithm waft uses internally.
fn git_oracle_eligible(dir: &Path) -> BTreeSet<String> {
    // Step 1: get worktreeinclude candidates
    let output = process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "ls-files",
            "--others",
            "--ignored",
            "--exclude-per-directory=.worktreeinclude",
            "--full-name",
            "-z",
        ])
        .output()
        .unwrap();
    let candidates: Vec<String> = output
        .stdout
        .split(|&b| b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).to_string())
        .collect();

    if candidates.is_empty() {
        return BTreeSet::new();
    }

    // Step 2: batch through check-ignore
    let mut child = process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["check-ignore", "--stdin", "-z", "-v", "-n"])
        .stdin(process::Stdio::piped())
        .stdout(process::Stdio::piped())
        .spawn()
        .unwrap();

    {
        use std::io::Write;
        let stdin = child.stdin.as_mut().unwrap();
        for c in &candidates {
            stdin.write_all(c.as_bytes()).unwrap();
            stdin.write_all(&[0]).unwrap();
        }
    }

    let output = child.wait_with_output().unwrap();
    let fields: Vec<&[u8]> = output.stdout.split(|&b| b == 0).collect();

    let mut eligible = BTreeSet::new();
    let mut i = 0;
    while i + 3 < fields.len() {
        let source = String::from_utf8_lossy(fields[i]).to_string();
        let _linenum = String::from_utf8_lossy(fields[i + 1]).to_string();
        let _pattern = String::from_utf8_lossy(fields[i + 2]).to_string();
        let pathname = String::from_utf8_lossy(fields[i + 3]).to_string();

        // Non-empty source means it matched an ignore rule
        if !source.is_empty() {
            eligible.insert(pathname);
        }
        i += 4;
    }

    eligible
}

// --- Deterministic differential tests ---

#[test]
fn differential_simple_env() {
    let repo = make_repo();
    write_file(repo.path(), ".gitignore", ".env\n");
    write_file(repo.path(), ".worktreeinclude", ".env\n");
    write_file(repo.path(), ".env", "secret");
    git(repo.path(), &["add", ".gitignore", ".worktreeinclude"]);
    git(repo.path(), &["commit", "-m", "setup"]);

    let waft_result = waft_list(repo.path());
    let git_result = git_oracle_eligible(repo.path());
    assert_eq!(waft_result, git_result, "waft and git oracle disagree");
}

#[test]
fn differential_nested_override() {
    let repo = make_repo();
    write_file(repo.path(), ".gitignore", "*.env\n*.secret\n");
    write_file(repo.path(), ".worktreeinclude", "*.env\n*.secret\n");
    write_file(repo.path(), "sub/.worktreeinclude", "!*.secret\n");
    write_file(repo.path(), "root.env", "r");
    write_file(repo.path(), "root.secret", "r");
    write_file(repo.path(), "sub/nested.env", "n");
    write_file(repo.path(), "sub/nested.secret", "n");
    git(
        repo.path(),
        &[
            "add",
            ".gitignore",
            ".worktreeinclude",
            "sub/.worktreeinclude",
        ],
    );
    git(repo.path(), &["commit", "-m", "setup"]);

    let waft_result = waft_list(repo.path());
    let git_result = git_oracle_eligible(repo.path());
    assert_eq!(
        waft_result, git_result,
        "waft and git oracle disagree on nested override"
    );
}

#[test]
fn differential_doublestar() {
    let repo = make_repo();
    write_file(repo.path(), ".gitignore", "**/*.key\n");
    write_file(repo.path(), ".worktreeinclude", "**/*.key\n");
    write_file(repo.path(), "a/b/c/deep.key", "k");
    write_file(repo.path(), "shallow.key", "k");
    git(repo.path(), &["add", ".gitignore", ".worktreeinclude"]);
    git(repo.path(), &["commit", "-m", "setup"]);

    let waft_result = waft_list(repo.path());
    let git_result = git_oracle_eligible(repo.path());
    assert_eq!(waft_result, git_result);
}

#[test]
fn differential_negation_chain() {
    let repo = make_repo();
    write_file(repo.path(), ".gitignore", "*.log\n");
    write_file(
        repo.path(),
        ".worktreeinclude",
        "*.log\n!debug.log\nimportant.log\n",
    );
    write_file(repo.path(), "app.log", "a");
    write_file(repo.path(), "debug.log", "d");
    write_file(repo.path(), "important.log", "i");
    git(repo.path(), &["add", ".gitignore", ".worktreeinclude"]);
    git(repo.path(), &["commit", "-m", "setup"]);

    let waft_result = waft_list(repo.path());
    let git_result = git_oracle_eligible(repo.path());
    assert_eq!(
        waft_result, git_result,
        "waft and git oracle disagree on negation chain"
    );
}

#[test]
fn differential_tracked_file_excluded() {
    let repo = make_repo();
    write_file(repo.path(), ".env", "secret");
    git(repo.path(), &["add", "-f", ".env"]);
    git(repo.path(), &["commit", "-m", "track env"]);
    write_file(repo.path(), ".gitignore", ".env\n");
    write_file(repo.path(), ".worktreeinclude", ".env\n");
    git(repo.path(), &["add", ".gitignore", ".worktreeinclude"]);
    git(repo.path(), &["commit", "-m", "add ignore"]);

    let waft_result = waft_list(repo.path());
    let git_result = git_oracle_eligible(repo.path());
    assert_eq!(
        waft_result, git_result,
        "tracked file should be excluded by both"
    );
}

#[test]
fn differential_git_info_exclude() {
    let repo = make_repo();
    write_file(repo.path(), ".worktreeinclude", "*.tmp\n");
    write_file(repo.path(), "test.tmp", "t");
    // Use .git/info/exclude instead of .gitignore
    let info_dir = repo.path().join(".git/info");
    fs::create_dir_all(&info_dir).unwrap();
    fs::write(info_dir.join("exclude"), "*.tmp\n").unwrap();
    git(repo.path(), &["add", ".worktreeinclude"]);
    git(repo.path(), &["commit", "-m", "setup"]);

    let waft_result = waft_list(repo.path());
    let git_result = git_oracle_eligible(repo.path());
    assert_eq!(waft_result, git_result, ".git/info/exclude should work");
}

// --- Property-based test ---

/// Generate a simple filename that is safe for Git.
fn safe_filename() -> impl Strategy<Value = String> {
    prop::string::string_regex("[a-z][a-z0-9]{0,5}\\.(env|log|key|tmp|secret)").unwrap()
}

/// Generate a simple gitignore-compatible pattern.
fn simple_pattern() -> impl Strategy<Value = String> {
    prop_oneof![
        safe_filename().prop_map(|f| f),
        Just("*.env".to_string()),
        Just("*.log".to_string()),
        Just("*.key".to_string()),
        Just("*.tmp".to_string()),
        Just("*.secret".to_string()),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(10))]

    #[test]
    fn prop_waft_matches_git_oracle(
        gitignore_patterns in prop::collection::vec(simple_pattern(), 1..4),
        wti_patterns in prop::collection::vec(simple_pattern(), 1..4),
        files in prop::collection::vec(safe_filename(), 1..6),
    ) {
        let repo = make_repo();

        // Write .gitignore
        let gi_content = gitignore_patterns.join("\n") + "\n";
        write_file(repo.path(), ".gitignore", &gi_content);

        // Write .worktreeinclude
        let wti_content = wti_patterns.join("\n") + "\n";
        write_file(repo.path(), ".worktreeinclude", &wti_content);

        // Create files
        for file in &files {
            write_file(repo.path(), file, "content");
        }

        git(repo.path(), &["add", ".gitignore", ".worktreeinclude"]);
        git(repo.path(), &["commit", "-m", "setup"]);

        let waft_result = waft_list(repo.path());
        let git_result = git_oracle_eligible(repo.path());
        prop_assert_eq!(waft_result, git_result, "waft and git oracle must agree");
    }
}

// --- Differential explanation-parity tests ---
//
// These tests compare per-path explanation tuples (source file, line, pattern)
// from `waft info` against `git check-ignore -v -n`, as required by
// ImplementationPlan.txt step 13 lines 256-260.

/// Parsed explanation tuple from either waft or git.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ExplanationTuple {
    /// Basename of the source file (e.g., ".gitignore")
    source_basename: String,
    /// Line number of the matching rule
    line: usize,
    /// The pattern that matched
    pattern: String,
}

/// Parse `waft info` output for a single path, extracting the gitignore
/// explanation tuple.  The format is:
///   gitignore: ignored (.gitignore:5: *.log)
/// Returns None if the path is not ignored.
fn parse_waft_info_explanation(stdout: &str, path: &str) -> Option<ExplanationTuple> {
    // Find the block for this path
    let mut in_block = false;
    for line in stdout.lines() {
        if line.starts_with("path: ") {
            in_block = line.trim() == format!("path: {path}");
            continue;
        }
        if !in_block {
            continue;
        }
        // Look for: gitignore: ignored (<source>:<line>: <pattern>)
        if let Some(rest) = line.strip_prefix("gitignore: ignored (") {
            let rest = rest.trim_end_matches(')');
            // Format: <source_file>:<line>: <pattern>
            let first_colon = rest.find(':')?;
            let source = &rest[..first_colon];
            let after_source = &rest[first_colon + 1..];
            let second_colon = after_source.find(':')?;
            let line_str = &after_source[..second_colon];
            let pattern = after_source[second_colon + 1..].trim();

            let source_basename = Path::new(source)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();

            return Some(ExplanationTuple {
                source_basename,
                line: line_str.trim().parse().ok()?,
                pattern: pattern.to_string(),
            });
        }
    }
    None
}

/// Query `git check-ignore -v -n -z` for a set of paths and return
/// explanation tuples keyed by pathname.
fn git_check_ignore_explanations(dir: &Path, paths: &[&str]) -> BTreeMap<String, ExplanationTuple> {
    if paths.is_empty() {
        return BTreeMap::new();
    }

    let mut child = process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["check-ignore", "--stdin", "-z", "-v", "-n"])
        .stdin(process::Stdio::piped())
        .stdout(process::Stdio::piped())
        .spawn()
        .unwrap();

    {
        use std::io::Write;
        let stdin = child.stdin.as_mut().unwrap();
        for p in paths {
            stdin.write_all(p.as_bytes()).unwrap();
            stdin.write_all(&[0]).unwrap();
        }
    }

    let output = child.wait_with_output().unwrap();
    let fields: Vec<&[u8]> = output.stdout.split(|&b| b == 0).collect();

    let mut result = BTreeMap::new();
    let mut i = 0;
    while i + 3 < fields.len() {
        let source = String::from_utf8_lossy(fields[i]).to_string();
        let linenum_str = String::from_utf8_lossy(fields[i + 1]).to_string();
        let pattern = String::from_utf8_lossy(fields[i + 2]).to_string();
        let pathname = String::from_utf8_lossy(fields[i + 3]).to_string();

        if !source.is_empty() {
            let source_basename = Path::new(&source)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            result.insert(
                pathname,
                ExplanationTuple {
                    source_basename,
                    line: linenum_str.trim().parse().unwrap_or(0),
                    pattern,
                },
            );
        }
        i += 4;
    }

    result
}

/// Run `waft info` for multiple paths and return the full stdout.
fn waft_info(dir: &Path, paths: &[&str]) -> String {
    let mut cmd = process::Command::new(env!("CARGO_BIN_EXE_waft"));
    cmd.args(["info", "--source", dir.to_str().unwrap()]);
    for p in paths {
        cmd.arg(p);
    }
    let output = cmd.output().unwrap();
    assert!(
        output.status.success(),
        "waft info failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).to_string()
}

/// Helper: set up a repo with given .gitignore, .worktreeinclude, and files,
/// then compare per-path gitignore explanation tuples from waft info against
/// git check-ignore -v -n.
fn assert_explanation_parity(gitignore: &str, worktreeinclude: &str, files: &[&str]) {
    let repo = make_repo();
    write_file(repo.path(), ".gitignore", gitignore);
    write_file(repo.path(), ".worktreeinclude", worktreeinclude);
    for f in files {
        write_file(repo.path(), f, "content");
    }
    git(repo.path(), &["add", ".gitignore", ".worktreeinclude"]);
    git(repo.path(), &["commit", "-m", "setup"]);

    let git_explanations = git_check_ignore_explanations(repo.path(), files);
    let waft_stdout = waft_info(repo.path(), files);

    for f in files {
        let git_exp = git_explanations.get(*f);
        let waft_exp = parse_waft_info_explanation(&waft_stdout, f);

        assert_eq!(
            git_exp.cloned(),
            waft_exp,
            "explanation parity mismatch for path '{}'\n\
             git check-ignore says: {:?}\n\
             waft info says: {:?}\n\
             waft stdout:\n{}",
            f,
            git_exp,
            waft_exp,
            waft_stdout,
        );
    }
}

#[test]
fn explanation_parity_simple_env() {
    assert_explanation_parity(".env\n", ".env\n", &[".env"]);
}

#[test]
fn explanation_parity_glob_patterns() {
    assert_explanation_parity(
        "*.log\n*.env\n",
        "*.log\n*.env\n",
        &["app.log", "debug.env", "README.md"],
    );
}

#[test]
fn explanation_parity_multi_line() {
    // The matching rule should be on the correct line
    assert_explanation_parity(
        "first.txt\nsecond.log\nthird.env\n",
        "first.txt\nsecond.log\nthird.env\n",
        &["first.txt", "second.log", "third.env", "unmatched.rs"],
    );
}

#[test]
fn explanation_parity_negation() {
    assert_explanation_parity(
        "*.log\n!debug.log\nimportant.log\n",
        "*.log\n",
        &["app.log", "debug.log", "important.log"],
    );
}

#[test]
fn explanation_parity_doublestar() {
    assert_explanation_parity(
        "**/*.key\n",
        "**/*.key\n",
        &["shallow.key", "a/deep.key", "a/b/deeper.key"],
    );
}

#[test]
fn explanation_parity_directory_slash() {
    // A pattern ending in / only matches directories in gitignore.
    // Files named "logs" should NOT match "logs/" pattern.
    assert_explanation_parity("logs/\n*.tmp\n", "*.tmp\n", &["test.tmp", "logs"]);
}

#[test]
fn explanation_parity_git_info_exclude() {
    // When ignore rules come from .git/info/exclude instead of .gitignore,
    // the source file in the explanation should reflect that.
    let repo = make_repo();
    write_file(repo.path(), ".worktreeinclude", "*.tmp\n");
    write_file(repo.path(), "test.tmp", "content");

    let info_dir = repo.path().join(".git/info");
    fs::create_dir_all(&info_dir).unwrap();
    fs::write(info_dir.join("exclude"), "*.tmp\n").unwrap();

    git(repo.path(), &["add", ".worktreeinclude"]);
    git(repo.path(), &["commit", "-m", "setup"]);

    let files = &["test.tmp"];
    let git_explanations = git_check_ignore_explanations(repo.path(), files);
    let waft_stdout = waft_info(repo.path(), files);

    let git_exp = git_explanations.get("test.tmp");
    let waft_exp = parse_waft_info_explanation(&waft_stdout, "test.tmp");

    // Both should agree that the source is "exclude" (from .git/info/exclude)
    assert_eq!(
        git_exp.cloned(),
        waft_exp,
        "explanation parity mismatch for .git/info/exclude case\n\
         git: {:?}\nwaft: {:?}",
        git_exp,
        waft_exp,
    );
}
