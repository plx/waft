//! Copy command integration tests using real Git worktrees.

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

/// Create a main repo with a linked worktree.
/// Returns (main_dir, worktree_dir).
fn setup_worktrees() -> (TempDir, TempDir) {
    let main_dir = make_repo();

    // Need an initial commit to create worktrees
    write_file(main_dir.path(), ".gitignore", ".env\n*.secret\n");
    write_file(main_dir.path(), ".worktreeinclude", ".env\n*.secret\n");
    git(main_dir.path(), &["add", ".gitignore", ".worktreeinclude"]);
    git(main_dir.path(), &["commit", "-m", "init"]);

    // Create a linked worktree
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
fn copy_basic() {
    let (main_dir, wt_dir) = setup_worktrees();
    let wt_path = wt_dir.path().join("linked");

    // Create an ignored file in the main worktree
    write_file(main_dir.path(), ".env", "SECRET=value\n");

    // Run copy
    wiff()
        .args([
            "copy",
            "--source",
            main_dir.path().to_str().unwrap(),
            "--dest",
            wt_path.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stderr(predicate::str::contains("copied"));

    // Verify the file was copied
    let dest_env = wt_path.join(".env");
    assert!(
        dest_env.exists(),
        ".env should be copied to linked worktree"
    );
    assert_eq!(fs::read_to_string(&dest_env).unwrap(), "SECRET=value\n");
}

#[test]
fn copy_dry_run_does_not_copy() {
    let (main_dir, wt_dir) = setup_worktrees();
    let wt_path = wt_dir.path().join("linked");

    write_file(main_dir.path(), ".env", "SECRET=value\n");

    wiff()
        .args([
            "copy",
            "--dry-run",
            "--source",
            main_dir.path().to_str().unwrap(),
            "--dest",
            wt_path.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("copy: .env"));

    // Verify the file was NOT copied
    let dest_env = wt_path.join(".env");
    assert!(
        !dest_env.exists(),
        ".env should NOT be copied in dry-run mode"
    );
}

#[test]
fn copy_identical_file_is_noop() {
    let (main_dir, wt_dir) = setup_worktrees();
    let wt_path = wt_dir.path().join("linked");

    write_file(main_dir.path(), ".env", "SECRET=same\n");
    write_file(&wt_path, ".env", "SECRET=same\n");

    wiff()
        .args([
            "copy",
            "--source",
            main_dir.path().to_str().unwrap(),
            "--dest",
            wt_path.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stderr(predicate::str::contains("up-to-date"));
}

#[test]
fn copy_skips_untracked_conflict_without_overwrite() {
    let (main_dir, wt_dir) = setup_worktrees();
    let wt_path = wt_dir.path().join("linked");

    write_file(main_dir.path(), ".env", "SOURCE_SECRET\n");
    write_file(&wt_path, ".env", "DEST_SECRET\n");

    wiff()
        .args([
            "copy",
            "--source",
            main_dir.path().to_str().unwrap(),
            "--dest",
            wt_path.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stderr(predicate::str::contains("skip"));

    // Destination file should be unchanged
    assert_eq!(
        fs::read_to_string(wt_path.join(".env")).unwrap(),
        "DEST_SECRET\n"
    );
}

#[test]
fn copy_overwrites_with_flag() {
    let (main_dir, wt_dir) = setup_worktrees();
    let wt_path = wt_dir.path().join("linked");

    write_file(main_dir.path(), ".env", "SOURCE_SECRET\n");
    write_file(&wt_path, ".env", "DEST_SECRET\n");

    wiff()
        .args([
            "copy",
            "--overwrite",
            "--source",
            main_dir.path().to_str().unwrap(),
            "--dest",
            wt_path.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stderr(predicate::str::contains("copied"));

    // Destination file should now match source
    assert_eq!(
        fs::read_to_string(wt_path.join(".env")).unwrap(),
        "SOURCE_SECRET\n"
    );
}

#[test]
fn copy_requires_destination() {
    let main_dir = make_repo();
    write_file(main_dir.path(), ".gitignore", ".env\n");
    write_file(main_dir.path(), ".worktreeinclude", ".env\n");
    git(main_dir.path(), &["add", "-A"]);
    git(main_dir.path(), &["commit", "-m", "init"]);

    wiff()
        .args(["copy", "--source", main_dir.path().to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("destination"));
}

#[test]
fn no_subcommand_in_linked_worktree_does_copy() {
    let (main_dir, wt_dir) = setup_worktrees();
    let wt_path = wt_dir.path().join("linked");

    write_file(main_dir.path(), ".env", "SECRET=auto\n");

    // Run wiff with no subcommand from the linked worktree directory
    // This should auto-detect source=main, dest=linked and do a copy
    wiff()
        .arg("-C")
        .arg(wt_path.to_str().unwrap())
        .assert()
        .success();

    assert!(wt_path.join(".env").exists(), ".env should be auto-copied");
}

#[test]
fn copy_skips_tracked_destination_conflict() {
    let (main_dir, wt_dir) = setup_worktrees();
    let wt_path = wt_dir.path().join("linked");

    // Create .env in source (main worktree)
    write_file(main_dir.path(), ".env", "SOURCE_SECRET\n");

    // Track .env in the linked worktree (force-add since it's gitignored)
    write_file(&wt_path, ".env", "DEST_TRACKED\n");
    git(&wt_path, &["add", "-f", ".env"]);
    git(&wt_path, &["commit", "-m", "track .env in dest"]);

    // Copy without --overwrite should skip tracked destination
    wiff()
        .args([
            "copy",
            "--source",
            main_dir.path().to_str().unwrap(),
            "--dest",
            wt_path.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stderr(predicate::str::contains("skip"));

    // Destination file should be unchanged
    assert_eq!(
        fs::read_to_string(wt_path.join(".env")).unwrap(),
        "DEST_TRACKED\n"
    );
}

#[test]
fn copy_skips_tracked_destination_even_with_overwrite() {
    let (main_dir, wt_dir) = setup_worktrees();
    let wt_path = wt_dir.path().join("linked");

    // Create .env in source
    write_file(main_dir.path(), ".env", "SOURCE_SECRET\n");

    // Track .env in the linked worktree
    write_file(&wt_path, ".env", "DEST_TRACKED\n");
    git(&wt_path, &["add", "-f", ".env"]);
    git(&wt_path, &["commit", "-m", "track .env in dest"]);

    // Even with --overwrite, tracked destination files must never be overwritten
    wiff()
        .args([
            "copy",
            "--overwrite",
            "--source",
            main_dir.path().to_str().unwrap(),
            "--dest",
            wt_path.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stderr(predicate::str::contains("skip"));

    // Destination file must remain unchanged
    assert_eq!(
        fs::read_to_string(wt_path.join(".env")).unwrap(),
        "DEST_TRACKED\n"
    );
}
