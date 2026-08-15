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

/// A transient index writer (IDE, fsmonitor, background `git status`) must not
/// turn into a sporadic per-file failure.
#[test]
fn copy_retries_past_a_transient_destination_index_lock() {
    let (main_dir, wt_dir) = setup_worktrees();
    let wt_path = wt_dir.path().join("linked");
    write_file(main_dir.path(), ".env", "SECRET=value\n");

    let index = git_path(&wt_path, "index");
    let mut lock_name = index.as_os_str().to_os_string();
    lock_name.push(".lock");
    let lock = std::path::PathBuf::from(lock_name);
    fs::write(&lock, b"transient writer\n").unwrap();

    let releaser = {
        let lock = lock.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(60));
            let _ = fs::remove_file(&lock);
        })
    };

    waft()
        .args([
            "copy",
            "--source",
            main_dir.path().to_str().unwrap(),
            "--dest",
            wt_path.to_str().unwrap(),
        ])
        .assert()
        .success();
    releaser.join().unwrap();

    assert_eq!(
        fs::read_to_string(wt_path.join(".env")).unwrap(),
        "SECRET=value\n"
    );
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

/// Replacing an existing destination needs the anchored publication path, so
/// it is a Unix capability. See
/// `copy_overwrite_is_planned_as_unsupported_off_unix` for the contract
/// everywhere else.
#[cfg(unix)]
#[test]
fn copy_overwrite_replaces_differing_untracked_destination() {
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
        .success()
        .stderr(predicate::str::contains("replaced: .env"))
        .stderr(predicate::str::contains("1 replaced"));

    assert_eq!(
        fs::read_to_string(wt_path.join(".env")).unwrap(),
        "SOURCE_SECRET\n"
    );
}

#[cfg(unix)]
#[test]
fn copy_overwrite_continues_past_a_conflict_and_copies_the_rest() {
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
        .success()
        .stderr(predicate::str::contains("copied: a.secret"))
        .stderr(predicate::str::contains("replaced: z.secret"));

    assert_eq!(
        fs::read_to_string(wt_path.join("a.secret")).unwrap(),
        "COPY_CANDIDATE\n"
    );
    assert_eq!(
        fs::read_to_string(wt_path.join("z.secret")).unwrap(),
        "SOURCE_CONFLICT\n"
    );
}

/// The migration case for destinations written by pre-release waft, which
/// always published mode `0600`.
#[cfg(unix)]
#[test]
fn copy_overwrite_repairs_permission_only_destination_without_rewriting_it() {
    use std::os::unix::fs::MetadataExt;
    use std::os::unix::fs::PermissionsExt;

    let (main_dir, wt_dir) = setup_worktrees();
    let wt_path = wt_dir.path().join("linked");

    write_file(main_dir.path(), ".env", "SECRET=same\n");
    write_file(&wt_path, ".env", "SECRET=same\n");
    fs::set_permissions(
        main_dir.path().join(".env"),
        fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    fs::set_permissions(wt_path.join(".env"), fs::Permissions::from_mode(0o600)).unwrap();
    let before = fs::metadata(wt_path.join(".env")).unwrap().ino();

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
        .stderr(predicate::str::contains("repaired permissions: .env"))
        .stderr(predicate::str::contains("1 permissions repaired"));

    let after = fs::metadata(wt_path.join(".env")).unwrap();
    assert_eq!(after.permissions().mode() & 0o777, 0o644);
    assert_eq!(
        fs::read_to_string(wt_path.join(".env")).unwrap(),
        "SECRET=same\n"
    );
    assert_eq!(
        after.ino(),
        before,
        "a permissions repair must not rewrite the file"
    );
}

#[cfg(unix)]
#[test]
fn copy_names_permission_only_conflict_and_its_remedy_when_skipping() {
    use std::os::unix::fs::PermissionsExt;

    let (main_dir, wt_dir) = setup_worktrees();
    let wt_path = wt_dir.path().join("linked");

    write_file(main_dir.path(), ".env", "SECRET=same\n");
    write_file(&wt_path, ".env", "SECRET=same\n");
    fs::set_permissions(
        main_dir.path().join(".env"),
        fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    fs::set_permissions(wt_path.join(".env"), fs::Permissions::from_mode(0o600)).unwrap();

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
        .stdout(predicate::str::contains(
            "skip: .env (content equal, permissions differ; --overwrite repairs the permissions)",
        ));

    // Without the flag the destination is left exactly as it was.
    assert_eq!(
        fs::metadata(wt_path.join(".env"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[cfg(unix)]
#[test]
fn copy_overwrite_dry_run_names_planned_replacements_without_touching_anything() {
    let (main_dir, wt_dir) = setup_worktrees();
    let wt_path = wt_dir.path().join("linked");

    write_file(main_dir.path(), ".env", "SOURCE_SECRET\n");
    write_file(&wt_path, ".env", "DEST_SECRET\n");

    waft()
        .args([
            "copy",
            "--dry-run",
            "--overwrite",
            "--source",
            main_dir.path().to_str().unwrap(),
            "--dest",
            wt_path.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "replace: .env (untracked conflict)",
        ));

    assert_eq!(
        fs::read_to_string(wt_path.join(".env")).unwrap(),
        "DEST_SECRET\n"
    );
}

/// A source that cannot be snapshotted is one file's failure. The dry run has
/// to say so on stderr and exit the way the real run would, or `--dry-run
/// --quiet` becomes a silent success that hides a file waft cannot copy.
#[cfg(unix)]
#[test]
fn copy_dry_run_reports_planning_failures_and_exits_nonzero() {
    use std::os::unix::fs::PermissionsExt;

    let (main_dir, wt_dir) = setup_worktrees();
    let wt_path = wt_dir.path().join("linked");

    write_file(main_dir.path(), ".env", "SECRET=value\n");
    write_file(main_dir.path(), "keep.secret", "READABLE\n");
    let unreadable = main_dir.path().join(".env");
    fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o000)).unwrap();
    if fs::read(&unreadable).is_ok() {
        // Running as root, where permissions cannot make a file unreadable.
        return;
    }

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
        .failure()
        .stdout(predicate::str::contains("copy: keep.secret"))
        .stderr(predicate::str::contains("FAILED: .env"));

    // `--quiet` suppresses the plan, never the reason the run exits nonzero.
    waft()
        .args([
            "copy",
            "--dry-run",
            "--quiet",
            "--source",
            main_dir.path().to_str().unwrap(),
            "--dest",
            wt_path.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("FAILED: .env"));

    assert!(!wt_path.join("keep.secret").exists());

    // The real run reports the same failure and still copies everything else.
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
        .stderr(predicate::str::contains("FAILED: .env"));

    assert_eq!(
        fs::read_to_string(wt_path.join("keep.secret")).unwrap(),
        "READABLE\n",
        "a source waft cannot read must not stop the rest of the run"
    );

    fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o600)).unwrap();
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

#[cfg(unix)]
#[test]
fn copy_overwrite_with_cow_also_replaces() {
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
        .success()
        .stderr(predicate::str::contains("replaced: .env"));

    assert_eq!(fs::read_to_string(wt_path.join(".env")).unwrap(), "NEW\n");
}

/// Where waft cannot replace an existing destination, planning says so, so the
/// dry run and the real run report the same thing and exit the same way. The
/// destination is left exactly as it was in both cases.
#[cfg(not(unix))]
#[test]
fn copy_overwrite_is_planned_as_unsupported_off_unix() {
    let (main_dir, wt_dir) = setup_worktrees();
    let wt_path = wt_dir.path().join("linked");

    write_file(main_dir.path(), ".env", "SOURCE_SECRET\n");
    write_file(&wt_path, ".env", "DEST_SECRET\n");

    for extra_args in [vec!["--overwrite"], vec!["--overwrite", "--dry-run"]] {
        let mut args = vec![
            "copy",
            "--source",
            main_dir.path().to_str().unwrap(),
            "--dest",
            wt_path.to_str().unwrap(),
        ];
        args.extend(extra_args);

        waft()
            .args(args)
            .assert()
            .failure()
            .stderr(predicate::str::contains("FAILED: .env"))
            .stderr(predicate::str::contains(
                "replacing an existing destination with --overwrite is not supported on this platform",
            ));

        assert_eq!(
            fs::read_to_string(wt_path.join(".env")).unwrap(),
            "DEST_SECRET\n"
        );
    }
}

/// Even under `--overwrite`, a tracked destination is never written. The
/// tracked recheck runs under Git's index lock immediately before publication.
#[test]
fn copy_overwrite_never_replaces_tracked_destination_that_differs() {
    let (main_dir, wt_dir) = setup_worktrees();
    let wt_path = wt_dir.path().join("linked");

    write_file(main_dir.path(), ".env", "SOURCE_SECRET\n");
    write_file(&wt_path, ".env", "DEST_TRACKED\n");
    git(&wt_path, &["add", "-f", ".env"]);
    git(&wt_path, &["commit", "-m", "track .env in dest"]);

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
        .stderr(predicate::str::contains("skipped"));

    assert_eq!(
        fs::read_to_string(wt_path.join(".env")).unwrap(),
        "DEST_TRACKED\n"
    );
}

/// Even under `--overwrite`, a tracked destination whose permissions are the
/// only difference is left alone.
#[cfg(unix)]
#[test]
fn copy_overwrite_never_repairs_permissions_of_tracked_destination() {
    use std::os::unix::fs::PermissionsExt;

    let (main_dir, wt_dir) = setup_worktrees();
    let wt_path = wt_dir.path().join("linked");

    write_file(main_dir.path(), ".env", "SECRET=same\n");
    write_file(&wt_path, ".env", "SECRET=same\n");
    git(&wt_path, &["add", "-f", ".env"]);
    git(&wt_path, &["commit", "-m", "track .env in dest"]);
    fs::set_permissions(
        main_dir.path().join(".env"),
        fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    fs::set_permissions(wt_path.join(".env"), fs::Permissions::from_mode(0o600)).unwrap();

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
        .stderr(predicate::str::contains("skipped"));

    assert_eq!(
        fs::metadata(wt_path.join(".env"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
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

/// The destination checkout's own `core.ignoreCase` decides whether
/// `SECRET.env` names the tracked `secret.env`. With the tracked entry absent
/// from the worktree there is no filesystem identity to compare, so the
/// configured answer is the only signal — and it must be obeyed in both
/// directions: protect the alias on a case-folding checkout, and copy a
/// genuinely distinct file on a case-sensitive one.
#[test]
fn copy_follows_configured_case_sensitivity_for_a_missing_dest_alias() {
    for ignore_case in ["true", "false"] {
        let (main_dir, wt_dir) = setup_worktrees();
        let wt_path = wt_dir.path().join("linked");

        write_file(main_dir.path(), ".gitignore", "*.env\n");
        write_file(main_dir.path(), ".worktreeinclude", "*.env\n");
        git(main_dir.path(), &["add", ".gitignore", ".worktreeinclude"]);
        git(main_dir.path(), &["commit", "-m", "select env files"]);
        git(main_dir.path(), &["config", "core.ignoreCase", ignore_case]);

        write_file(main_dir.path(), "SECRET.env", "SOURCE_SECRET\n");
        write_file(&wt_path, "secret.env", "DEST_TRACKED\n");
        git(&wt_path, &["add", "-f", "secret.env"]);
        git(&wt_path, &["commit", "-m", "track lower-case destination"]);
        fs::remove_file(wt_path.join("secret.env")).unwrap();

        let assertion = waft()
            .args([
                "copy",
                "--source",
                main_dir.path().to_str().unwrap(),
                "--dest",
                wt_path.to_str().unwrap(),
            ])
            .assert()
            .success();

        if ignore_case == "true" {
            assertion.stderr(predicate::str::contains("skip"));
            assert!(!wt_path.join("secret.env").exists());
            assert!(!wt_path.join("SECRET.env").exists());
        } else {
            assertion.stderr(predicate::str::contains("copied: SECRET.env"));
            assert_eq!(
                fs::read_to_string(wt_path.join("SECRET.env")).unwrap(),
                "SOURCE_SECRET\n"
            );
        }
    }
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
