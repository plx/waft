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

// --- Destination classification tests ---

/// Create a main repo with a linked worktree for info --dest tests.
/// Returns (main_dir, worktree_tempdir). The linked worktree is at wt_dir.path().join("linked").
fn setup_worktrees() -> (TempDir, TempDir) {
    let main_dir = make_repo();

    write_file(main_dir.path(), ".gitignore", ".env\n*.secret\nconfig\nnested/secret.env\n");
    write_file(main_dir.path(), ".worktreeinclude", ".env\n*.secret\nconfig\nnested/secret.env\n");
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

/// When a destination file is tracked in the dest worktree, info should report
/// "tracked-conflict" and "skip (tracked conflict)" — not the generic
/// "exists (differs)" / "skip (conflict)".
#[test]
fn info_dest_tracked_conflict() {
    let (main_dir, wt_dir) = setup_worktrees();
    let wt_path = wt_dir.path().join("linked");

    // Create .env in source (main worktree)
    write_file(main_dir.path(), ".env", "SOURCE_SECRET=foo");

    // Track .env in the linked worktree (force-add since it's gitignored, then commit)
    write_file(&wt_path, ".env", "DEST_SECRET=bar");
    git(&wt_path, &["add", "-f", ".env"]);
    git(&wt_path, &["commit", "-m", "track env in dest"]);

    wiff()
        .args([
            "info",
            "--source",
            main_dir.path().to_str().unwrap(),
            "--dest",
            wt_path.to_str().unwrap(),
            ".env",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("destination: tracked-conflict"))
        .stdout(predicate::str::contains("planned_action: skip (tracked conflict)"));
}

/// When a destination file exists, differs, and is NOT tracked, info should
/// report "untracked-conflict".
#[test]
fn info_dest_untracked_conflict() {
    let (main_dir, wt_dir) = setup_worktrees();
    let wt_path = wt_dir.path().join("linked");

    write_file(main_dir.path(), ".env", "SOURCE_SECRET=foo");
    // .env exists in dest but is NOT tracked (just written, not git-added)
    write_file(&wt_path, ".env", "DIFFERENT_SECRET=bar");

    wiff()
        .args([
            "info",
            "--source",
            main_dir.path().to_str().unwrap(),
            "--dest",
            wt_path.to_str().unwrap(),
            ".env",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("destination: untracked-conflict"))
        .stdout(predicate::str::contains("planned_action: skip (untracked conflict)"));
}

/// When a destination file is byte-identical to source, info should report
/// "up-to-date" / "no-op".
#[test]
fn info_dest_up_to_date() {
    let (main_dir, wt_dir) = setup_worktrees();
    let wt_path = wt_dir.path().join("linked");

    write_file(main_dir.path(), ".env", "SAME_SECRET=foo");
    write_file(&wt_path, ".env", "SAME_SECRET=foo");

    wiff()
        .args([
            "info",
            "--source",
            main_dir.path().to_str().unwrap(),
            "--dest",
            wt_path.to_str().unwrap(),
            ".env",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("destination: up-to-date"))
        .stdout(predicate::str::contains("planned_action: no-op"));
}

/// When destination exists but is a directory (not a file), info should report
/// "type-conflict".
#[test]
fn info_dest_type_conflict() {
    let (main_dir, wt_dir) = setup_worktrees();
    let wt_path = wt_dir.path().join("linked");

    write_file(main_dir.path(), "config", "my config");
    // In dest, "config" is a directory, not a file
    fs::create_dir_all(wt_path.join("config")).unwrap();

    wiff()
        .args([
            "info",
            "--source",
            main_dir.path().to_str().unwrap(),
            "--dest",
            wt_path.to_str().unwrap(),
            "config",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("destination: type-conflict"))
        .stdout(predicate::str::contains("planned_action: skip (type conflict)"));
}

/// When the destination's parent path contains a symlink, info should report
/// "unsafe-path".
#[cfg(unix)]
#[test]
fn info_dest_unsafe_path() {
    let (main_dir, wt_dir) = setup_worktrees();
    let wt_path = wt_dir.path().join("linked");

    write_file(main_dir.path(), "nested/secret.env", "SECRET=x");

    // In dest, "nested" is a symlink — making the path unsafe
    let symlink_target = tempfile::TempDir::new().unwrap();
    std::os::unix::fs::symlink(symlink_target.path(), wt_path.join("nested")).unwrap();

    wiff()
        .args([
            "info",
            "--source",
            main_dir.path().to_str().unwrap(),
            "--dest",
            wt_path.to_str().unwrap(),
            "nested/secret.env",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("destination: unsafe-path"))
        .stdout(predicate::str::contains("planned_action: skip (unsafe path)"));
}

/// When destination does not exist and file is eligible, info should report
/// "missing" / "copy".
#[test]
fn info_dest_missing() {
    let (main_dir, wt_dir) = setup_worktrees();
    let wt_path = wt_dir.path().join("linked");

    write_file(main_dir.path(), ".env", "SECRET=foo");

    wiff()
        .args([
            "info",
            "--source",
            main_dir.path().to_str().unwrap(),
            "--dest",
            wt_path.to_str().unwrap(),
            ".env",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("destination: missing"))
        .stdout(predicate::str::contains("planned_action: copy"));
}

/// When source is missing but destination exists, info should not misclassify
/// the destination as an untracked conflict (edge case: classify_destination
/// assumes source is a regular file).
#[test]
fn info_dest_with_missing_source() {
    let (main_dir, wt_dir) = setup_worktrees();
    let wt_path = wt_dir.path().join("linked");

    // .env exists in dest but NOT in source
    write_file(&wt_path, ".env", "DEST_ONLY=bar");

    wiff()
        .args([
            "info",
            "--source",
            main_dir.path().to_str().unwrap(),
            "--dest",
            wt_path.to_str().unwrap(),
            ".env",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("source_exists: no"))
        .stdout(predicate::str::contains("destination: exists"))
        // Should NOT report "untracked-conflict" for missing source
        .stdout(predicate::str::contains("untracked-conflict").not());
}

/// `info` must run the validation phase just like `copy` and `list`.
/// When validation finds errors (e.g., an invalid .gitignore pattern),
/// `info` should exit non-zero and print the error.
#[test]
fn info_fails_when_validation_has_errors() {
    let repo = make_repo();
    // A dangling backslash is an invalid gitignore pattern
    write_file(repo.path(), ".gitignore", "\\\n");
    write_file(repo.path(), ".worktreeinclude", "*.env\n");
    write_file(repo.path(), ".env", "SECRET=foo");
    git(repo.path(), &["add", ".gitignore", ".worktreeinclude"]);
    git(repo.path(), &["commit", "-m", "setup"]);

    wiff()
        .args(["info", "--source", repo.path().to_str().unwrap(), ".env"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("error:"))
        // Validation should block before any info output is produced
        .stdout(predicate::str::contains("path:").not());
}

/// When validation passes, `info` should still succeed normally.
#[test]
fn info_succeeds_when_validation_passes() {
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
        .stdout(predicate::str::contains("eligible_to_copy: yes"));
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
