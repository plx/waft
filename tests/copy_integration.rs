//! Copy command integration tests using real Git worktrees.

use predicates::prelude::*;
use std::fs;
use std::path::Path;
use std::process;
use tempfile::TempDir;

mod support;

use support::waft;

fn make_repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    git(dir.path(), &["init"]);
    git(dir.path(), &["config", "user.email", "test@test.com"]);
    git(dir.path(), &["config", "user.name", "Test"]);
    dir
}

fn git(dir: &Path, args: &[&str]) {
    let output = support::git_command()
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

fn git_path(dir: &Path, name: &str) -> std::path::PathBuf {
    let output = support::git_command()
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "--git-path", name])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git rev-parse --git-path {name} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let path = std::path::PathBuf::from(
        String::from_utf8_lossy(&output.stdout)
            .trim_end_matches(['\n', '\r'])
            .to_string(),
    );
    if path.is_absolute() {
        path
    } else {
        dir.join(path)
    }
}

fn write_file(dir: &Path, rel_path: &str, content: &str) {
    let path = dir.join(rel_path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&path, content).unwrap();
}

fn run_copy(source: &Path, dest: &Path) -> process::Output {
    waft()
        .args([
            "copy",
            "--source",
            source.to_str().unwrap(),
            "--dest",
            dest.to_str().unwrap(),
        ])
        .output()
        .unwrap()
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

fn setup_with_safe_full_dir() -> (TempDir, TempDir) {
    let (main_dir, wt_dir) = setup_worktrees();

    write_file(main_dir.path(), ".gitignore", "cfg/\n.env\n");
    write_file(main_dir.path(), ".worktreeinclude", "cfg/\n.env\n");
    write_file(main_dir.path(), "cfg/a.conf", "a\n");
    write_file(main_dir.path(), "cfg/b.conf", "b\n");
    write_file(main_dir.path(), "cfg/nested/c.conf", "c\n");
    write_file(main_dir.path(), ".env", "X=1\n");
    git(main_dir.path(), &["add", ".gitignore", ".worktreeinclude"]);
    git(main_dir.path(), &["commit", "-m", "safe full dir fixture"]);

    (main_dir, wt_dir)
}

#[test]
fn copy_basic() {
    let (main_dir, wt_dir) = setup_worktrees();
    let wt_path = wt_dir.path().join("linked");

    // Create an ignored file in the main worktree
    write_file(main_dir.path(), ".env", "SECRET=value\n");

    // Run copy
    waft()
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
fn copy_fails_closed_while_destination_index_is_locked() {
    let (main_dir, wt_dir) = setup_worktrees();
    let wt_path = wt_dir.path().join("linked");
    write_file(main_dir.path(), ".env", "SECRET=value\n");

    let index = git_path(&wt_path, "index");
    let mut lock_name = index.as_os_str().to_os_string();
    lock_name.push(".lock");
    let lock = std::path::PathBuf::from(lock_name);
    fs::write(&lock, b"held by concurrent Git\n").unwrap();

    waft()
        .args([
            "copy",
            "--source",
            main_dir.path().to_str().unwrap(),
            "--dest",
            wt_path.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("destination Git index is locked"));

    assert!(!wt_path.join(".env").exists());
    assert!(lock.exists(), "waft must not remove another process's lock");
}

#[test]
fn copy_dry_run_does_not_copy() {
    let (main_dir, wt_dir) = setup_worktrees();
    let wt_path = wt_dir.path().join("linked");

    write_file(main_dir.path(), ".env", "SECRET=value\n");

    waft()
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
fn quiet_dry_run_suppresses_plan_output() {
    let (main_dir, wt_dir) = setup_worktrees();
    let wt_path = wt_dir.path().join("linked");
    write_file(main_dir.path(), ".env", "SECRET=value\n");

    waft()
        .args([
            "copy",
            "--isolated",
            "--dry-run",
            "--quiet",
            "--source",
            main_dir.path().to_str().unwrap(),
            "--dest",
            wt_path.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::is_empty());

    assert!(!wt_path.join(".env").exists());
}

#[test]
fn copy_identical_file_is_noop() {
    let (main_dir, wt_dir) = setup_worktrees();
    let wt_path = wt_dir.path().join("linked");

    write_file(main_dir.path(), ".env", "SECRET=same\n");
    write_file(&wt_path, ".env", "SECRET=same\n");

    waft()
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

    waft()
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
fn copy_overwrite_flag_fails_closed_on_existing_untracked_file() {
    let (main_dir, wt_dir) = setup_worktrees();
    let wt_path = wt_dir.path().join("linked");

    write_file(main_dir.path(), ".env", "SOURCE_SECRET\n");
    write_file(&wt_path, ".env", "DEST_SECRET\n");

    waft()
        .args([
            "copy",
            "--overwrite",
            "--source",
            main_dir.path().to_str().unwrap(),
            "--dest",
            wt_path.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "--overwrite cannot safely replace existing untracked destination",
        ));

    // The destination remains untouched; the user must review and remove it.
    assert_eq!(
        fs::read_to_string(wt_path.join(".env")).unwrap(),
        "DEST_SECRET\n"
    );
}

#[test]
fn copy_overwrite_conflict_aborts_the_whole_plan_before_mutation() {
    let (main_dir, wt_dir) = setup_worktrees();
    let wt_path = wt_dir.path().join("linked");

    write_file(main_dir.path(), "a.secret", "COPY_CANDIDATE\n");
    write_file(main_dir.path(), "z.secret", "SOURCE_CONFLICT\n");
    write_file(&wt_path, "z.secret", "DEST_CONFLICT\n");

    waft()
        .args([
            "copy",
            "--overwrite",
            "--source",
            main_dir.path().to_str().unwrap(),
            "--dest",
            wt_path.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "--overwrite cannot safely replace existing untracked destination",
        ));

    assert!(
        !wt_path.join("a.secret").exists(),
        "a missing destination must not be copied before the conflict is reported"
    );
    assert_eq!(
        fs::read_to_string(wt_path.join("z.secret")).unwrap(),
        "DEST_CONFLICT\n"
    );
}

#[test]
fn copy_requires_destination() {
    let main_dir = make_repo();
    write_file(main_dir.path(), ".gitignore", ".env\n");
    write_file(main_dir.path(), ".worktreeinclude", ".env\n");
    git(main_dir.path(), &["add", "-A"]);
    git(main_dir.path(), &["commit", "-m", "init"]);

    waft()
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

    // Run waft with no subcommand from the linked worktree directory
    // This should auto-detect source=main, dest=linked and do a copy
    waft()
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
    waft()
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
fn copy_with_simple_strategy_writes_correct_content() {
    let (main_dir, wt_dir) = setup_worktrees();
    let wt_path = wt_dir.path().join("linked");

    write_file(main_dir.path(), ".env", "PAYLOAD=simple\n");

    waft()
        .args([
            "--copy-strategy",
            "simple-copy",
            "copy",
            "--source",
            main_dir.path().to_str().unwrap(),
            "--dest",
            wt_path.to_str().unwrap(),
        ])
        .assert()
        .success();

    assert_eq!(
        fs::read_to_string(wt_path.join(".env")).unwrap(),
        "PAYLOAD=simple\n"
    );
}

#[test]
fn copy_with_cow_strategy_writes_correct_content() {
    let (main_dir, wt_dir) = setup_worktrees();
    let wt_path = wt_dir.path().join("linked");

    write_file(main_dir.path(), ".env", "PAYLOAD=cow\n");

    // reflink_or_copy ensures content lands even on filesystems that don't
    // support cloning, so this test is a meaningful smoke check on every
    // platform.
    waft()
        .args([
            "--copy-strategy",
            "cow-copy",
            "copy",
            "--source",
            main_dir.path().to_str().unwrap(),
            "--dest",
            wt_path.to_str().unwrap(),
        ])
        .assert()
        .success();

    assert_eq!(
        fs::read_to_string(wt_path.join(".env")).unwrap(),
        "PAYLOAD=cow\n"
    );
}

#[test]
fn copy_strategy_via_env_var() {
    let (main_dir, wt_dir) = setup_worktrees();
    let wt_path = wt_dir.path().join("linked");

    write_file(main_dir.path(), ".env", "PAYLOAD=env\n");

    waft()
        .env("WAFT_COPY_STRATEGY", "cow-copy")
        .args([
            "copy",
            "--source",
            main_dir.path().to_str().unwrap(),
            "--dest",
            wt_path.to_str().unwrap(),
        ])
        .assert()
        .success();

    assert_eq!(
        fs::read_to_string(wt_path.join(".env")).unwrap(),
        "PAYLOAD=env\n"
    );
}

#[test]
fn copy_overwrite_with_cow_also_fails_closed() {
    let (main_dir, wt_dir) = setup_worktrees();
    let wt_path = wt_dir.path().join("linked");

    write_file(main_dir.path(), ".env", "NEW\n");
    write_file(&wt_path, ".env", "OLD\n");

    waft()
        .args([
            "--copy-strategy",
            "cow-copy",
            "copy",
            "--overwrite",
            "--source",
            main_dir.path().to_str().unwrap(),
            "--dest",
            wt_path.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "--overwrite cannot safely replace existing untracked destination",
        ));

    assert_eq!(fs::read_to_string(wt_path.join(".env")).unwrap(), "OLD\n");
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
    waft()
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

#[test]
fn copy_never_overwrites_case_folded_tracked_destination() {
    let (main_dir, wt_dir) = setup_worktrees();
    let wt_path = wt_dir.path().join("linked");

    write_file(main_dir.path(), ".gitignore", "*.env\n");
    write_file(main_dir.path(), ".worktreeinclude", "*.env\n");
    git(main_dir.path(), &["add", ".gitignore", ".worktreeinclude"]);
    git(main_dir.path(), &["commit", "-m", "select env files"]);
    git(main_dir.path(), &["config", "core.ignoreCase", "true"]);

    write_file(main_dir.path(), "SECRET.env", "SOURCE_SECRET\n");
    write_file(&wt_path, "secret.env", "DEST_TRACKED\n");
    git(&wt_path, &["add", "-f", "secret.env"]);
    git(&wt_path, &["commit", "-m", "track case-folded destination"]);

    waft()
        .args([
            "copy",
            "--overwrite",
            "--source",
            main_dir.path().to_str().unwrap(),
            "--dest",
            wt_path.to_str().unwrap(),
        ])
        .assert()
        .success();

    assert_eq!(
        fs::read_to_string(wt_path.join("secret.env")).unwrap(),
        "DEST_TRACKED\n"
    );
}

#[test]
fn copy_never_overwrites_unicode_case_folded_tracked_destination() {
    let (main_dir, wt_dir) = setup_worktrees();
    let wt_path = wt_dir.path().join("linked");

    write_file(main_dir.path(), ".gitignore", "*.env\n");
    write_file(main_dir.path(), ".worktreeinclude", "*.env\n");
    git(main_dir.path(), &["add", ".gitignore", ".worktreeinclude"]);
    git(main_dir.path(), &["commit", "-m", "select env files"]);
    git(main_dir.path(), &["config", "core.ignoreCase", "true"]);

    write_file(main_dir.path(), "Ä.env", "SOURCE_SECRET\n");
    write_file(&wt_path, "ä.env", "DEST_TRACKED\n");
    git(&wt_path, &["add", "-f", "ä.env"]);
    git(
        &wt_path,
        &["commit", "-m", "track Unicode case-folded destination"],
    );

    waft()
        .args([
            "copy",
            "--overwrite",
            "--source",
            main_dir.path().to_str().unwrap(),
            "--dest",
            wt_path.to_str().unwrap(),
        ])
        .assert()
        .success();

    assert_eq!(
        fs::read_to_string(wt_path.join("ä.env")).unwrap(),
        "DEST_TRACKED\n"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn copy_uses_filesystem_identity_when_ignore_case_is_false() {
    let (main_dir, wt_dir) = setup_worktrees();
    let wt_path = wt_dir.path().join("linked");
    let probe = wt_path.join(".waft-case-probe");
    fs::write(&probe, "probe").unwrap();
    let case_insensitive = wt_path.join(".WAFT-CASE-PROBE").exists();
    fs::remove_file(probe).unwrap();
    assert!(
        case_insensitive,
        "macOS safety regression requires a case-insensitive test volume"
    );

    write_file(main_dir.path(), ".gitignore", "*.env\n");
    write_file(main_dir.path(), ".worktreeinclude", "*.env\n");
    git(main_dir.path(), &["add", ".gitignore", ".worktreeinclude"]);
    git(main_dir.path(), &["commit", "-m", "select env files"]);
    git(main_dir.path(), &["config", "core.ignoreCase", "false"]);

    write_file(main_dir.path(), "SECRET.env", "SOURCE_SECRET\n");
    write_file(&wt_path, "secret.env", "DEST_TRACKED\n");
    git(&wt_path, &["add", "-f", "secret.env"]);
    git(
        &wt_path,
        &["commit", "-m", "track filesystem-aliased destination"],
    );

    waft()
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

    assert_eq!(
        fs::read_to_string(wt_path.join("secret.env")).unwrap(),
        "DEST_TRACKED\n"
    );
}

#[cfg(any(target_os = "macos", windows))]
#[test]
fn copy_protects_missing_case_alias_when_ignore_case_is_false() {
    let (main_dir, wt_dir) = setup_worktrees();
    let wt_path = wt_dir.path().join("linked");

    write_file(main_dir.path(), ".gitignore", "*.env\n");
    write_file(main_dir.path(), ".worktreeinclude", "*.env\n");
    git(main_dir.path(), &["add", ".gitignore", ".worktreeinclude"]);
    git(main_dir.path(), &["commit", "-m", "select env files"]);
    git(main_dir.path(), &["config", "core.ignoreCase", "false"]);

    write_file(main_dir.path(), "SECRET.env", "SOURCE_SECRET\n");
    write_file(&wt_path, "secret.env", "DEST_TRACKED\n");
    git(&wt_path, &["add", "-f", "secret.env"]);
    git(&wt_path, &["commit", "-m", "track lower-case destination"]);
    fs::remove_file(wt_path.join("secret.env")).unwrap();

    waft()
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

    assert!(!wt_path.join("secret.env").exists());
    assert!(!wt_path.join("SECRET.env").exists());
}

#[test]
fn copy_streams_each_selected_file_in_fresh_subtree() {
    let (main_dir, wt_dir) = setup_with_safe_full_dir();
    let wt_path = wt_dir.path().join("linked");

    let output = run_copy(main_dir.path(), &wt_path);
    assert!(
        output.status.success(),
        "copy failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(stderr.contains("copied: cfg/a.conf"), "{stderr}");
    assert!(stderr.contains("copied: cfg/nested/c.conf"), "{stderr}");
    assert_eq!(
        fs::read_to_string(wt_path.join("cfg/a.conf")).unwrap(),
        "a\n"
    );
    assert_eq!(
        fs::read_to_string(wt_path.join("cfg/nested/c.conf")).unwrap(),
        "c\n"
    );
    assert_eq!(fs::read_to_string(wt_path.join(".env")).unwrap(), "X=1\n");
}

#[test]
fn copy_streams_selected_files_into_existing_dst_dir() {
    let (main_dir, wt_dir) = setup_with_safe_full_dir();
    let wt_path = wt_dir.path().join("linked");
    fs::create_dir_all(wt_path.join("cfg")).unwrap();

    let output = run_copy(main_dir.path(), &wt_path);
    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(stderr.contains("copied: cfg/a.conf"), "{stderr}");
    assert_eq!(
        fs::read_to_string(wt_path.join("cfg/b.conf")).unwrap(),
        "b\n"
    );
}

#[test]
fn copy_streaming_manifest_is_idempotent() {
    let (main_dir, wt_dir) = setup_with_safe_full_dir();
    let wt_path = wt_dir.path().join("linked");

    let first = run_copy(main_dir.path(), &wt_path);
    assert!(first.status.success());
    assert!(String::from_utf8_lossy(&first.stderr).contains("copied: cfg/a.conf"));

    let second = run_copy(main_dir.path(), &wt_path);
    assert!(second.status.success());
    let stderr = String::from_utf8_lossy(&second.stderr);
    assert!(stderr.contains("4 up-to-date"), "{stderr}");
}

#[test]
fn copy_manifest_does_not_write_missing_tracked_dest_file() {
    let (main_dir, wt_dir) = setup_with_safe_full_dir();
    let wt_path = wt_dir.path().join("linked");

    write_file(&wt_path, "cfg/a.conf", "tracked\n");
    git(&wt_path, &["add", "-f", "cfg/a.conf"]);
    git(&wt_path, &["commit", "-m", "track dest cfg file"]);
    fs::remove_file(wt_path.join("cfg/a.conf")).unwrap();

    let output = run_copy(main_dir.path(), &wt_path);
    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(stderr.contains("skipped"), "{stderr}");
    assert!(!wt_path.join("cfg/a.conf").exists());
    assert_eq!(
        fs::read_to_string(wt_path.join("cfg/b.conf")).unwrap(),
        "b\n"
    );
}

#[cfg(unix)]
#[test]
fn copy_manifest_skips_selected_symlink() {
    let (main_dir, wt_dir) = setup_worktrees();
    let wt_path = wt_dir.path().join("linked");

    write_file(main_dir.path(), ".gitignore", "cfg/\n");
    write_file(main_dir.path(), ".worktreeinclude", "cfg/\n");
    write_file(main_dir.path(), "cfg/a.conf", "a\n");
    std::os::unix::fs::symlink("target", main_dir.path().join("cfg/link.env")).unwrap();
    git(main_dir.path(), &["add", ".gitignore", ".worktreeinclude"]);
    git(main_dir.path(), &["commit", "-m", "symlink fixture"]);

    let output = run_copy(main_dir.path(), &wt_path);
    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(stderr.contains("skipped"), "{stderr}");
    assert_eq!(
        fs::read_to_string(wt_path.join("cfg/a.conf")).unwrap(),
        "a\n"
    );
    assert!(!wt_path.join("cfg/link.env").exists());
}

#[test]
fn copy_manifest_does_not_copy_gitlink_contents() {
    let (main_dir, wt_dir) = setup_worktrees();
    let wt_path = wt_dir.path().join("linked");

    write_file(main_dir.path(), ".gitignore", "cfg/\nsafe/\n");
    write_file(main_dir.path(), ".worktreeinclude", "cfg/\nsafe/\n");
    write_file(main_dir.path(), "cfg/a.conf", "a\n");
    write_file(main_dir.path(), "cfg/sub/inner.env", "inner\n");
    write_file(main_dir.path(), "safe/a.conf", "safe\n");
    write_file(
        main_dir.path(),
        "cfg/sub/.git",
        "gitdir: ../../.git/modules/sub\n",
    );
    git(
        main_dir.path(),
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            "160000,1111111111111111111111111111111111111111,cfg/sub",
        ],
    );
    git(main_dir.path(), &["add", ".gitignore", ".worktreeinclude"]);
    git(main_dir.path(), &["commit", "-m", "gitlink fixture"]);

    let output = run_copy(main_dir.path(), &wt_path);
    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(stderr.contains("copied: safe/a.conf"), "{stderr}");
    assert!(stderr.contains("copied: cfg/a.conf"), "{stderr}");
    assert_eq!(
        fs::read_to_string(wt_path.join("cfg/a.conf")).unwrap(),
        "a\n"
    );
    assert!(!wt_path.join("cfg/sub/inner.env").exists());
}

#[test]
fn copy_manifest_does_not_copy_nested_repo_contents() {
    let (main_dir, wt_dir) = setup_worktrees();
    let wt_path = wt_dir.path().join("linked");

    write_file(main_dir.path(), ".gitignore", "cfg/\nsafe/\n");
    write_file(main_dir.path(), ".worktreeinclude", "cfg/\nsafe/\n");
    write_file(main_dir.path(), "cfg/a.conf", "a\n");
    write_file(main_dir.path(), "cfg/nested/inner.env", "inner\n");
    write_file(main_dir.path(), "safe/a.conf", "safe\n");
    git(&main_dir.path().join("cfg/nested"), &["init"]);
    git(main_dir.path(), &["add", ".gitignore", ".worktreeinclude"]);
    git(main_dir.path(), &["commit", "-m", "nested repo fixture"]);

    let output = run_copy(main_dir.path(), &wt_path);
    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(stderr.contains("copied: safe/a.conf"), "{stderr}");
    assert!(stderr.contains("copied: cfg/a.conf"), "{stderr}");
    assert_eq!(
        fs::read_to_string(wt_path.join("cfg/a.conf")).unwrap(),
        "a\n"
    );
    assert!(!wt_path.join("cfg/nested/inner.env").exists());
}

#[test]
fn copy_manifest_does_not_create_empty_dirs() {
    let (main_dir, wt_dir) = setup_worktrees();
    let wt_path = wt_dir.path().join("linked");

    write_file(main_dir.path(), ".gitignore", "cfg/\n");
    write_file(main_dir.path(), ".worktreeinclude", "cfg/\n");
    write_file(main_dir.path(), "cfg/a.conf", "a\n");
    fs::create_dir_all(main_dir.path().join("cfg/empty")).unwrap();
    git(main_dir.path(), &["add", ".gitignore", ".worktreeinclude"]);
    git(main_dir.path(), &["commit", "-m", "empty dir fixture"]);

    let output = run_copy(main_dir.path(), &wt_path);
    assert!(output.status.success());
    assert_eq!(
        fs::read_to_string(wt_path.join("cfg/a.conf")).unwrap(),
        "a\n"
    );
    assert!(!wt_path.join("cfg/empty").exists());
}

#[test]
fn copy_manifest_handles_nested_dir_with_missing_parent() {
    let (main_dir, wt_dir) = setup_worktrees();
    let wt_path = wt_dir.path().join("linked");

    write_file(main_dir.path(), ".gitignore", "outer/\n");
    write_file(main_dir.path(), ".worktreeinclude", "outer/cfg/\n");
    write_file(main_dir.path(), "outer/cfg/a.conf", "a\n");
    write_file(main_dir.path(), "outer/tracked.txt", "tracked\n");
    git(main_dir.path(), &["add", ".gitignore", ".worktreeinclude"]);
    git(
        main_dir.path(),
        &["commit", "-m", "nested manifest fixture"],
    );

    let output = run_copy(main_dir.path(), &wt_path);
    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(stderr.contains("copied: outer/cfg/a.conf"), "{stderr}");
    assert_eq!(
        fs::read_to_string(wt_path.join("outer/cfg/a.conf")).unwrap(),
        "a\n"
    );
}
