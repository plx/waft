//! Info command integration tests.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::path::Path;
use std::process;
use tempfile::TempDir;

fn wiff() -> Command {
    Command::cargo_bin("wiff").unwrap()
}

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

#[test]
fn info_tracked_file() {
    let repo = make_repo();
    write_file(repo.path(), "README.md", "hello");
    git(repo.path(), &["add", "README.md"]);
    git(repo.path(), &["commit", "-m", "init"]);

    wiff()
        .args([
            "info",
            "--source",
            repo.path().to_str().unwrap(),
            "README.md",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("tracked: yes"))
        .stdout(predicate::str::contains("eligible_to_copy: no"));
}

#[test]
fn info_ignored_and_included() {
    let repo = make_repo();
    write_file(repo.path(), ".gitignore", ".env\n");
    write_file(repo.path(), ".worktreeinclude", ".env\n");
    write_file(repo.path(), ".env", "SECRET=foo");
    git(repo.path(), &["add", ".gitignore", ".worktreeinclude"]);
    git(repo.path(), &["commit", "-m", "setup"]);

    wiff()
        .args(["info", "--source", repo.path().to_str().unwrap(), ".env"])
        .assert()
        .success()
        .stdout(predicate::str::contains("tracked: no"))
        .stdout(predicate::str::contains("gitignore: ignored"))
        .stdout(predicate::str::contains("worktreeinclude: included"))
        .stdout(predicate::str::contains("eligible_to_copy: yes"));
}

#[test]
fn info_not_ignored_not_eligible() {
    let repo = make_repo();
    write_file(repo.path(), ".worktreeinclude", "*.env\n");
    write_file(repo.path(), "README.md", "hello");
    git(repo.path(), &["add", ".worktreeinclude"]);
    git(repo.path(), &["commit", "-m", "setup"]);

    // README.md is not ignored and not matched by .worktreeinclude
    wiff()
        .args([
            "info",
            "--source",
            repo.path().to_str().unwrap(),
            "README.md",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("eligible_to_copy: no"));
}

#[test]
fn info_missing_file() {
    let repo = make_repo();
    write_file(repo.path(), ".gitignore", "*.env\n");
    write_file(repo.path(), ".worktreeinclude", "*.env\n");
    git(repo.path(), &["add", ".gitignore", ".worktreeinclude"]);
    git(repo.path(), &["commit", "-m", "setup"]);

    wiff()
        .args([
            "info",
            "--source",
            repo.path().to_str().unwrap(),
            "nonexistent.env",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("source_exists: no"))
        .stdout(predicate::str::contains("eligible_to_copy: no"));
}

#[test]
fn info_multiple_paths() {
    let repo = make_repo();
    write_file(repo.path(), ".gitignore", "*.env\n*.log\n");
    write_file(repo.path(), ".worktreeinclude", "*.env\n");
    write_file(repo.path(), ".env", "secret");
    write_file(repo.path(), "debug.log", "log");
    git(repo.path(), &["add", ".gitignore", ".worktreeinclude"]);
    git(repo.path(), &["commit", "-m", "setup"]);

    let output = wiff()
        .args([
            "info",
            "--source",
            repo.path().to_str().unwrap(),
            ".env",
            "debug.log",
        ])
        .assert()
        .success();

    let stdout = String::from_utf8(output.get_output().stdout.clone()).unwrap();
    // .env should be eligible
    assert!(stdout.contains("path: .env"));
    // debug.log is ignored but not in .worktreeinclude, so not eligible
    assert!(stdout.contains("path: debug.log"));
}
