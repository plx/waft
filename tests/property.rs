//! Property and differential tests comparing wiff behavior against Git.

use std::collections::BTreeSet;
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

fn wiff_list(dir: &Path) -> BTreeSet<String> {
    let output = process::Command::new(env!("CARGO_BIN_EXE_wiff"))
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
/// git check-ignore — the same algorithm wiff uses internally.
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

    let wiff_result = wiff_list(repo.path());
    let git_result = git_oracle_eligible(repo.path());
    assert_eq!(wiff_result, git_result, "wiff and git oracle disagree");
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

    let wiff_result = wiff_list(repo.path());
    let git_result = git_oracle_eligible(repo.path());
    assert_eq!(
        wiff_result, git_result,
        "wiff and git oracle disagree on nested override"
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

    let wiff_result = wiff_list(repo.path());
    let git_result = git_oracle_eligible(repo.path());
    assert_eq!(wiff_result, git_result);
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

    let wiff_result = wiff_list(repo.path());
    let git_result = git_oracle_eligible(repo.path());
    assert_eq!(
        wiff_result, git_result,
        "wiff and git oracle disagree on negation chain"
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

    let wiff_result = wiff_list(repo.path());
    let git_result = git_oracle_eligible(repo.path());
    assert_eq!(
        wiff_result, git_result,
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

    let wiff_result = wiff_list(repo.path());
    let git_result = git_oracle_eligible(repo.path());
    assert_eq!(wiff_result, git_result, ".git/info/exclude should work");
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
    fn prop_wiff_matches_git_oracle(
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

        let wiff_result = wiff_list(repo.path());
        let git_result = git_oracle_eligible(repo.path());
        prop_assert_eq!(wiff_result, git_result, "wiff and git oracle must agree");
    }
}
