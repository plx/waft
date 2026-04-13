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

// --- list verbose tests ---

#[test]
fn list_verbose_shows_source_size() {
    let repo = make_repo();

    write_file(repo.path(), ".gitignore", ".env\n");
    write_file(repo.path(), ".worktreeinclude", ".env\n");
    write_file(repo.path(), ".env", "SECRET=foo\n");
    git(repo.path(), &["add", ".gitignore", ".worktreeinclude"]);
    git(repo.path(), &["commit", "-m", "setup"]);

    wiff()
        .args(["list", "--source", repo.path().to_str().unwrap(), "-v"])
        .assert()
        .success()
        .stdout(predicate::str::contains("size:"));
}

#[test]
fn list_verbose_shows_gitignore_info() {
    let repo = make_repo();

    write_file(repo.path(), ".gitignore", ".env\n");
    write_file(repo.path(), ".worktreeinclude", ".env\n");
    write_file(repo.path(), ".env", "SECRET=foo\n");
    git(repo.path(), &["add", ".gitignore", ".worktreeinclude"]);
    git(repo.path(), &["commit", "-m", "setup"]);

    wiff()
        .args(["list", "--source", repo.path().to_str().unwrap(), "-v"])
        .assert()
        .success()
        .stdout(predicate::str::contains("gitignore:"))
        .stdout(predicate::str::contains(".gitignore"))
        .stdout(predicate::str::contains(".env"));
}

#[test]
fn list_verbose_shows_worktreeinclude_info() {
    let repo = make_repo();

    write_file(repo.path(), ".gitignore", ".env\n");
    write_file(repo.path(), ".worktreeinclude", ".env\n");
    write_file(repo.path(), ".env", "SECRET=foo\n");
    git(repo.path(), &["add", ".gitignore", ".worktreeinclude"]);
    git(repo.path(), &["commit", "-m", "setup"]);

    wiff()
        .args(["list", "--source", repo.path().to_str().unwrap(), "-v"])
        .assert()
        .success()
        .stdout(predicate::str::contains("worktreeinclude:"))
        .stdout(predicate::str::contains(".worktreeinclude"));
}

#[test]
fn list_verbose_no_predicted_action_without_dest() {
    let repo = make_repo();

    write_file(repo.path(), ".gitignore", ".env\n");
    write_file(repo.path(), ".worktreeinclude", ".env\n");
    write_file(repo.path(), ".env", "SECRET=foo\n");
    git(repo.path(), &["add", ".gitignore", ".worktreeinclude"]);
    git(repo.path(), &["commit", "-m", "setup"]);

    let output = wiff()
        .args(["list", "--source", repo.path().to_str().unwrap(), "-v"])
        .assert()
        .success();

    let stdout = String::from_utf8(output.get_output().stdout.clone()).unwrap();
    assert!(
        !stdout.contains("action:"),
        "should not show predicted action without --dest"
    );
}

fn setup_list_worktrees() -> (TempDir, TempDir) {
    let main_dir = make_repo();

    write_file(main_dir.path(), ".gitignore", ".env\n");
    write_file(main_dir.path(), ".worktreeinclude", ".env\n");
    git(main_dir.path(), &["add", ".gitignore", ".worktreeinclude"]);
    git(main_dir.path(), &["commit", "-m", "init"]);

    let wt_dir = TempDir::new().unwrap();
    let wt_path = wt_dir.path().join("linked");
    git(
        main_dir.path(),
        &[
            "worktree",
            "add",
            wt_path.to_str().unwrap(),
            "-b",
            "linked-branch",
        ],
    );

    (main_dir, wt_dir)
}

#[test]
fn list_verbose_dest_missing_shows_copy() {
    let (main_dir, wt_dir) = setup_list_worktrees();
    let wt_path = wt_dir.path().join("linked");

    write_file(main_dir.path(), ".env", "SECRET=foo");

    wiff()
        .args([
            "list",
            "--source",
            main_dir.path().to_str().unwrap(),
            "--dest",
            wt_path.to_str().unwrap(),
            "-v",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("action: copy"));
}

#[test]
fn list_verbose_dest_up_to_date_shows_noop() {
    let (main_dir, wt_dir) = setup_list_worktrees();
    let wt_path = wt_dir.path().join("linked");

    write_file(main_dir.path(), ".env", "SAME_SECRET=foo");
    write_file(&wt_path, ".env", "SAME_SECRET=foo");

    wiff()
        .args([
            "list",
            "--source",
            main_dir.path().to_str().unwrap(),
            "--dest",
            wt_path.to_str().unwrap(),
            "-v",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("action: no-op"));
}

#[test]
fn list_verbose_dest_untracked_conflict_shows_skip() {
    let (main_dir, wt_dir) = setup_list_worktrees();
    let wt_path = wt_dir.path().join("linked");

    write_file(main_dir.path(), ".env", "SOURCE_SECRET=foo");
    write_file(&wt_path, ".env", "DIFFERENT_SECRET=bar");

    wiff()
        .args([
            "list",
            "--source",
            main_dir.path().to_str().unwrap(),
            "--dest",
            wt_path.to_str().unwrap(),
            "-v",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "action: skip (untracked conflict)",
        ));
}

#[test]
fn list_verbose_dest_tracked_conflict_shows_skip() {
    let (main_dir, wt_dir) = setup_list_worktrees();
    let wt_path = wt_dir.path().join("linked");

    write_file(main_dir.path(), ".env", "SOURCE_SECRET=foo");
    // Track .env in the linked worktree
    write_file(&wt_path, ".env", "DEST_SECRET=bar");
    git(&wt_path, &["add", "-f", ".env"]);
    git(&wt_path, &["commit", "-m", "track env in dest"]);

    wiff()
        .args([
            "list",
            "--source",
            main_dir.path().to_str().unwrap(),
            "--dest",
            wt_path.to_str().unwrap(),
            "-v",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("action: skip (tracked conflict)"));
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

#[cfg(unix)]
#[test]
fn validate_rejects_symlinked_worktreeinclude() {
    let repo = make_repo();
    write_file(repo.path(), "real.wti", "*.env\n");
    std::os::unix::fs::symlink(
        repo.path().join("real.wti"),
        repo.path().join(".worktreeinclude"),
    )
    .unwrap();

    wiff()
        .args(["validate", "--source", repo.path().to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("symlinked .worktreeinclude"));
}
