//! Integration tests with real Git repos in temp directories.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::path::Path;
use std::process;
use tempfile::TempDir;

fn wiff() -> Command {
    Command::cargo_bin("wiff").unwrap()
}

/// Create a git repo in a temp dir, returning the TempDir handle.
fn make_repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    git(dir.path(), &["init"]);
    git(dir.path(), &["config", "user.email", "test@test.com"]);
    git(dir.path(), &["config", "user.name", "Test"]);
    dir
}

fn git(dir: &Path, args: &[&str]) {
    let output = process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn write_file(dir: &Path, rel_path: &str, content: &str) {
    let path = dir.join(rel_path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&path, content).unwrap();
}

#[allow(dead_code)]
fn commit_all(dir: &Path, msg: &str) {
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-m", msg, "--allow-empty"]);
}

// --- list tests ---

#[test]
fn list_root_worktreeinclude_copies_env() {
    let repo = make_repo();

    // Create .gitignore that ignores .env
    write_file(repo.path(), ".gitignore", ".env\n");
    // Create .worktreeinclude that selects .env
    write_file(repo.path(), ".worktreeinclude", ".env\n");
    // Create the actual .env file
    write_file(repo.path(), ".env", "SECRET=foo\n");
    // Commit the gitignore and worktreeinclude (but not .env)
    git(repo.path(), &["add", ".gitignore", ".worktreeinclude"]);
    git(repo.path(), &["commit", "-m", "setup"]);

    wiff()
        .args(["list", "--source", repo.path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains(".env"));
}

#[test]
fn list_empty_when_no_worktreeinclude() {
    let repo = make_repo();
    write_file(repo.path(), ".gitignore", "*.log\n");
    write_file(repo.path(), "debug.log", "log data");
    git(repo.path(), &["add", ".gitignore"]);
    git(repo.path(), &["commit", "-m", "setup"]);

    wiff()
        .args(["list", "--source", repo.path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::is_empty());
}

#[test]
fn list_tracked_file_excluded() {
    let repo = make_repo();
    // Force-track .env before adding .gitignore
    write_file(repo.path(), ".env", "SECRET=foo\n");
    git(repo.path(), &["add", "-f", ".env"]);
    git(repo.path(), &["commit", "-m", "track env"]);
    // Now add .gitignore and .worktreeinclude
    write_file(repo.path(), ".gitignore", ".env\n");
    write_file(repo.path(), ".worktreeinclude", ".env\n");
    git(repo.path(), &["add", ".gitignore", ".worktreeinclude"]);
    git(repo.path(), &["commit", "-m", "setup"]);

    // Even though .env is in both .gitignore and .worktreeinclude,
    // it's tracked, so git check-ignore won't report it as ignored.
    wiff()
        .args(["list", "--source", repo.path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains(".env").not());
}

#[test]
fn list_nested_worktreeinclude_override() {
    let repo = make_repo();

    // Root ignores all .env files
    write_file(repo.path(), ".gitignore", "*.env\n");
    // Root worktreeinclude selects all .env files
    write_file(repo.path(), ".worktreeinclude", "*.env\n");
    // Nested worktreeinclude deselects .env in config/
    write_file(repo.path(), "config/.worktreeinclude", "!*.env\n");

    write_file(repo.path(), "root.env", "root");
    write_file(repo.path(), "config/sub.env", "sub");

    git(
        repo.path(),
        &[
            "add",
            ".gitignore",
            ".worktreeinclude",
            "config/.worktreeinclude",
        ],
    );
    git(repo.path(), &["commit", "-m", "setup"]);

    let output = wiff()
        .args(["list", "--source", repo.path().to_str().unwrap()])
        .assert()
        .success();

    let stdout = String::from_utf8(output.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("root.env"), "root.env should be listed");
    // config/sub.env is deselected by nested .worktreeinclude negation
    assert!(
        !stdout.contains("config/sub.env"),
        "config/sub.env should be deselected by nested negation"
    );
}

#[test]
fn list_doublestar_pattern() {
    let repo = make_repo();

    write_file(repo.path(), ".gitignore", "**/*.secret\n");
    write_file(repo.path(), ".worktreeinclude", "**/*.secret\n");
    write_file(repo.path(), "a/b/key.secret", "secret data");

    git(repo.path(), &["add", ".gitignore", ".worktreeinclude"]);
    git(repo.path(), &["commit", "-m", "setup"]);

    wiff()
        .args(["list", "--source", repo.path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("a/b/key.secret"));
}

#[test]
fn list_output_sorted() {
    let repo = make_repo();

    write_file(repo.path(), ".gitignore", "*.env\n");
    write_file(repo.path(), ".worktreeinclude", "*.env\n");
    write_file(repo.path(), "c.env", "c");
    write_file(repo.path(), "a.env", "a");
    write_file(repo.path(), "b.env", "b");

    git(repo.path(), &["add", ".gitignore", ".worktreeinclude"]);
    git(repo.path(), &["commit", "-m", "setup"]);

    let output = wiff()
        .args(["list", "--source", repo.path().to_str().unwrap()])
        .assert()
        .success();

    let stdout = String::from_utf8(output.get_output().stdout.clone()).unwrap();
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines, vec!["a.env", "b.env", "c.env"]);
}

// --- validate tests ---

#[test]
fn validate_passes_with_valid_files() {
    let repo = make_repo();
    write_file(repo.path(), ".gitignore", "*.log\n");
    write_file(repo.path(), ".worktreeinclude", "*.env\n");
    git(repo.path(), &["add", ".gitignore", ".worktreeinclude"]);
    git(repo.path(), &["commit", "-m", "setup"]);

    wiff()
        .args(["validate", "--source", repo.path().to_str().unwrap()])
        .assert()
        .success()
        .stderr(predicate::str::contains("validation passed"));
}
