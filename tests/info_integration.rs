//! Info command integration tests.

use predicates::prelude::*;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

mod support;

use support::waft;

fn waft_in(dir: &Path) -> assert_cmd::Command {
    let mut command = waft();
    command.current_dir(dir);
    command
}

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

fn write_file(dir: &Path, rel_path: &str, content: &str) {
    let path = dir.join(rel_path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&path, content).unwrap();
}

#[test]
fn info_verbose_emits_resolved_policy() {
    let repo = make_repo();
    write_file(repo.path(), "README.md", "hello");
    git(repo.path(), &["add", "README.md"]);
    git(repo.path(), &["commit", "-m", "init"]);

    waft_in(repo.path())
        .args([
            "info",
            "-v",
            "--compat-profile",
            "wt",
            "--source",
            repo.path().to_str().unwrap(),
            "README.md",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("policy:"))
        .stdout(predicate::str::contains("profile: wt"))
        .stdout(predicate::str::contains("when_missing: all-ignored"))
        .stdout(predicate::str::contains("semantics: wt-0.39"))
        .stdout(predicate::str::contains("symlink_policy: follow"))
        .stdout(predicate::str::contains("builtin_exclude_set: tooling-v1"));
}

#[test]
fn info_non_verbose_omits_policy_block() {
    let repo = make_repo();
    write_file(repo.path(), "README.md", "hello");
    git(repo.path(), &["add", "README.md"]);
    git(repo.path(), &["commit", "-m", "init"]);

    waft_in(repo.path())
        .args([
            "info",
            "--source",
            repo.path().to_str().unwrap(),
            "README.md",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("policy:").not());
}

#[test]
fn info_tracked_file() {
    let repo = make_repo();
    write_file(repo.path(), "README.md", "hello");
    git(repo.path(), &["add", "README.md"]);
    git(repo.path(), &["commit", "-m", "init"]);

    waft_in(repo.path())
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

    waft_in(repo.path())
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
    waft_in(repo.path())
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

    waft_in(repo.path())
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
fn wt_glob_negation_explanation_matches_effective_selection() {
    let repo = make_repo();
    write_file(repo.path(), ".gitignore", "*.tmp\n");
    write_file(repo.path(), ".worktreeinclude", "!*.tmp\n");
    write_file(repo.path(), "cache.tmp", "selected");
    git(repo.path(), &["add", ".gitignore", ".worktreeinclude"]);
    git(repo.path(), &["commit", "-m", "configure wt glob negation"]);

    waft_in(repo.path())
        .args([
            "info",
            "--compat-profile",
            "wt",
            "--source",
            repo.path().to_str().unwrap(),
            "cache.tmp",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("eligible_to_copy: yes"))
        .stdout(predicate::str::contains(
            "worktreeinclude: selected (effective wt-0.39 policy",
        ))
        .stdout(predicate::str::contains("worktreeinclude: excluded").not());

    waft_in(repo.path())
        .args([
            "list",
            "--verbose",
            "--compat-profile",
            "wt",
            "--source",
            repo.path().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("cache.tmp"))
        .stdout(predicate::str::contains(
            "selected (effective wt-0.39 policy",
        ));
}

#[test]
fn wt_literal_negation_explanation_reports_effective_exclusion() {
    let repo = make_repo();
    write_file(repo.path(), ".gitignore", "*.tmp\n");
    write_file(repo.path(), ".worktreeinclude", "!cache.tmp\n");
    write_file(repo.path(), "cache.tmp", "excluded");
    git(repo.path(), &["add", ".gitignore", ".worktreeinclude"]);
    git(
        repo.path(),
        &["commit", "-m", "configure wt literal negation"],
    );

    waft_in(repo.path())
        .args([
            "info",
            "--compat-profile",
            "wt",
            "--source",
            repo.path().to_str().unwrap(),
            "cache.tmp",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("eligible_to_copy: no"))
        .stdout(predicate::str::contains("worktreeinclude: excluded"));
}

#[cfg(target_os = "macos")]
#[test]
fn info_agrees_with_list_for_native_case_alias_when_ignore_case_is_false() {
    let repo = make_repo();
    write_file(repo.path(), ".gitignore", "*.env\n");
    write_file(repo.path(), ".worktreeinclude", "*.env\n");
    write_file(repo.path(), "Secret.env", "selected");
    assert!(
        repo.path().join("secret.env").exists(),
        "macOS safety regression requires a case-insensitive test volume"
    );
    git(repo.path(), &["add", ".gitignore", ".worktreeinclude"]);
    git(repo.path(), &["commit", "-m", "configure case alias"]);
    git(repo.path(), &["config", "core.ignoreCase", "false"]);

    for backend in ["gix", "cli"] {
        let output = waft_in(repo.path())
            .env("WAFT_GIT_BACKEND", backend)
            .args([
                "info",
                "--source",
                repo.path().to_str().unwrap(),
                "secret.env",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{backend} info failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("eligible_to_copy: yes"),
            "{backend} info contradicted the canonical eligible alias: {stdout}"
        );
    }
}

// --- Destination classification tests ---

/// Create a main repo with a linked worktree for info --dest tests.
/// Returns (main_dir, worktree_tempdir). The linked worktree is at wt_dir.path().join("linked").
fn setup_worktrees() -> (TempDir, TempDir) {
    let main_dir = make_repo();

    write_file(
        main_dir.path(),
        ".gitignore",
        ".env\n*.secret\nconfig\nnested/secret.env\n",
    );
    write_file(
        main_dir.path(),
        ".worktreeinclude",
        ".env\n*.secret\nconfig\nnested/secret.env\n",
    );
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

    waft_in(main_dir.path())
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
        .stdout(predicate::str::contains(
            "planned_action: skip (tracked conflict)",
        ));
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

    waft_in(main_dir.path())
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
        .stdout(predicate::str::contains(
            "planned_action: skip (untracked conflict)",
        ));
}

/// When a destination file is byte-identical to source, info should report
/// "up-to-date" / "no-op".
#[test]
fn info_dest_up_to_date() {
    let (main_dir, wt_dir) = setup_worktrees();
    let wt_path = wt_dir.path().join("linked");

    write_file(main_dir.path(), ".env", "SAME_SECRET=foo");
    write_file(&wt_path, ".env", "SAME_SECRET=foo");

    waft_in(main_dir.path())
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

    waft_in(main_dir.path())
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
        .stdout(predicate::str::contains(
            "planned_action: skip (type conflict)",
        ));
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

    waft_in(main_dir.path())
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
        .stdout(predicate::str::contains(
            "planned_action: skip (unsafe path)",
        ));
}

/// When destination does not exist and file is eligible, info should report
/// "missing" / "copy".
#[test]
fn info_dest_missing() {
    let (main_dir, wt_dir) = setup_worktrees();
    let wt_path = wt_dir.path().join("linked");

    write_file(main_dir.path(), ".env", "SECRET=foo");

    waft_in(main_dir.path())
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

    waft_in(main_dir.path())
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

    waft_in(repo.path())
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

    waft_in(repo.path())
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

    let output = waft_in(repo.path())
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

#[test]
fn info_uses_effective_wt_fallback_eligibility() {
    let repo = make_repo();
    write_file(repo.path(), ".gitignore", ".env\n");
    write_file(repo.path(), ".env", "secret");
    git(repo.path(), &["add", ".gitignore"]);
    git(repo.path(), &["commit", "-m", "setup"]);

    waft_in(repo.path())
        .args([
            "info",
            "--isolated",
            "--compat-profile",
            "wt",
            "--source",
            repo.path().to_str().unwrap(),
            ".env",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "worktreeinclude: selected (effective policy)",
        ))
        .stdout(predicate::str::contains("eligible_to_copy: yes"));
}

#[test]
fn info_honors_post_selection_excludes() {
    let repo = make_repo();
    write_file(repo.path(), ".gitignore", ".env\n");
    write_file(repo.path(), ".worktreeinclude", ".env\n");
    write_file(repo.path(), ".env", "secret");
    git(repo.path(), &["add", ".gitignore", ".worktreeinclude"]);
    git(repo.path(), &["commit", "-m", "setup"]);

    waft_in(repo.path())
        .args([
            "info",
            "--isolated",
            "--extra-exclude",
            ".env",
            "--source",
            repo.path().to_str().unwrap(),
            ".env",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("worktreeinclude: included"))
        .stdout(predicate::str::contains("eligible_to_copy: no"));
}

#[test]
fn info_relative_paths_honor_c_directory() {
    let repo = make_repo();
    write_file(repo.path(), ".gitignore", "sub/.env\n");
    write_file(repo.path(), ".worktreeinclude", "sub/.env\n");
    write_file(repo.path(), "sub/.env", "secret");
    git(repo.path(), &["add", ".gitignore", ".worktreeinclude"]);
    git(repo.path(), &["commit", "-m", "setup"]);

    waft_in(repo.path())
        .args([
            "info",
            "--isolated",
            "-C",
            repo.path().join("sub").to_str().unwrap(),
            "--source",
            repo.path().to_str().unwrap(),
            ".env",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("path: sub/.env"))
        .stdout(predicate::str::contains("eligible_to_copy: yes"));
}

#[cfg(unix)]
#[test]
fn symlink_sources_are_never_reported_as_eligible() {
    let repo = make_repo();
    write_file(repo.path(), ".gitignore", "linked.env\n");
    write_file(repo.path(), ".worktreeinclude", "linked.env\n");
    write_file(repo.path(), "target.env", "secret");
    std::os::unix::fs::symlink("target.env", repo.path().join("linked.env")).unwrap();
    git(repo.path(), &["add", ".gitignore", ".worktreeinclude"]);
    git(repo.path(), &["commit", "-m", "setup"]);

    waft_in(repo.path())
        .args([
            "info",
            "--isolated",
            "--source",
            repo.path().to_str().unwrap(),
            "linked.env",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("source_kind: symlink"))
        .stdout(predicate::str::contains("eligible_to_copy: no"));

    waft_in(repo.path())
        .args([
            "list",
            "--isolated",
            "--source",
            repo.path().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::is_empty());
}

#[test]
fn quiet_info_suppresses_non_error_output() {
    let repo = make_repo();
    write_file(repo.path(), "README.md", "fixture");
    git(repo.path(), &["add", "README.md"]);
    git(repo.path(), &["commit", "-m", "setup"]);

    waft_in(repo.path())
        .args([
            "info",
            "--isolated",
            "--quiet",
            "--source",
            repo.path().to_str().unwrap(),
            "README.md",
        ])
        .assert()
        .success()
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::is_empty());
}

#[test]
fn info_rejects_relative_path_when_c_is_outside_source() {
    let repo = make_repo();
    write_file(repo.path(), "README.md", "source");
    git(repo.path(), &["add", "README.md"]);
    git(repo.path(), &["commit", "-m", "setup"]);

    let outside = TempDir::new().unwrap();
    write_file(outside.path(), "README.md", "outside");

    waft()
        .args([
            "info",
            "--isolated",
            "-C",
            outside.path().to_str().unwrap(),
            "--source",
            repo.path().to_str().unwrap(),
            "README.md",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("outside the repository root"));
}
