//! Targeted CLI-flag override tests.
//!
//! For each fixture, verify that explicit knob flags reliably override the
//! preset selected by `--compat-profile`. Currently covers F2's
//! `--when-missing-worktreeinclude`; later PRs add overrides for other knobs.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::process;

use assert_cmd::Command;
use tempfile::TempDir;

fn waft() -> Command {
    Command::cargo_bin("waft").unwrap()
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

fn list_paths(source: &Path, extra_args: &[&str]) -> BTreeSet<String> {
    let mut cmd = waft();
    // Run from the source repo so project-level `.waft.toml` discovery
    // (which walks upward from cwd) sees configs committed in the test
    // fixture.
    cmd.current_dir(source)
        .args(["list", "--source"])
        .arg(source);
    cmd.args(extra_args);
    let assert = cmd.assert().success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    stdout
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

fn setup_f2() -> TempDir {
    let repo = make_repo();
    write_file(repo.path(), ".gitignore", ".env\ncache/\n");
    git(repo.path(), &["add", ".gitignore"]);
    git(repo.path(), &["commit", "-m", "init"]);
    write_file(repo.path(), ".env", "secret\n");
    write_file(repo.path(), "cache/build.bin", "data\n");
    repo
}

#[test]
fn f2_claude_with_all_ignored_override() {
    // Claude preset's when_missing is "blank"; the override flips it.
    let repo = setup_f2();
    let paths = list_paths(
        repo.path(),
        &[
            "--compat-profile",
            "claude",
            "--when-missing-worktreeinclude",
            "all-ignored",
        ],
    );
    let expected: BTreeSet<String> = [".env".to_string(), "cache/build.bin".to_string()]
        .into_iter()
        .collect();
    assert_eq!(paths, expected);
}

#[test]
fn f2_wt_with_blank_override() {
    // Wt preset's when_missing is "all-ignored"; the override flips it.
    let repo = setup_f2();
    let paths = list_paths(
        repo.path(),
        &[
            "--compat-profile",
            "wt",
            "--when-missing-worktreeinclude",
            "blank",
        ],
    );
    assert!(
        paths.is_empty(),
        "wt + --when-missing-worktreeinclude=blank should select nothing; got {paths:?}"
    );
}

// --- F7 builtin-set overrides ---

fn setup_f7() -> TempDir {
    let repo = make_repo();
    write_file(repo.path(), ".gitignore", ".conductor/\n");
    write_file(repo.path(), ".worktreeinclude", ".conductor/**/*.key\n");
    git(repo.path(), &["add", ".gitignore", ".worktreeinclude"]);
    git(repo.path(), &["commit", "-m", "init"]);
    write_file(repo.path(), ".conductor/state/dev.key", "data\n");
    repo
}

#[test]
fn f7_claude_with_tooling_v1_drops_key() {
    // Claude preset uses builtin_exclude_set=none; flip it on.
    let repo = setup_f7();
    let paths = list_paths(
        repo.path(),
        &[
            "--compat-profile",
            "claude",
            "--builtin-exclude-set",
            "tooling-v1",
        ],
    );
    assert!(
        paths.is_empty(),
        "claude + tooling-v1 should drop .conductor/*; got {paths:?}"
    );
}

#[test]
fn f7_wt_with_none_keeps_key() {
    // Wt preset uses tooling-v1; flip it off.
    let repo = setup_f7();
    let paths = list_paths(
        repo.path(),
        &["--compat-profile", "wt", "--builtin-exclude-set", "none"],
    );
    let expected: BTreeSet<String> = [".conductor/state/dev.key".to_string()]
        .into_iter()
        .collect();
    assert_eq!(paths, expected);
}

// --- Extra-excludes / replace-extra ---

fn setup_simple_two_files() -> TempDir {
    let repo = make_repo();
    write_file(repo.path(), ".gitignore", "*.env\n*.log\n");
    write_file(repo.path(), ".worktreeinclude", "*.env\n*.log\n");
    git(repo.path(), &["add", ".gitignore", ".worktreeinclude"]);
    git(repo.path(), &["commit", "-m", "init"]);
    write_file(repo.path(), "keep.env", "k\n");
    write_file(repo.path(), "drop.log", "d\n");
    repo
}

#[test]
fn extra_exclude_drops_matching_path() {
    let repo = setup_simple_two_files();
    let paths = list_paths(
        repo.path(),
        &["--compat-profile", "claude", "--extra-exclude", "*.log"],
    );
    let expected: BTreeSet<String> = ["keep.env".to_string()].into_iter().collect();
    assert_eq!(paths, expected);
}

#[test]
fn extra_exclude_repeatable() {
    let repo = setup_simple_two_files();
    let paths = list_paths(
        repo.path(),
        &[
            "--compat-profile",
            "claude",
            "--extra-exclude",
            "*.log",
            "--extra-exclude",
            "*.env",
        ],
    );
    assert!(
        paths.is_empty(),
        "two --extra-exclude flags should drop both files; got {paths:?}"
    );
}

// --- F8 symlink-policy overrides (Unix only) ---

#[cfg(unix)]
fn setup_f8() -> TempDir {
    let repo = make_repo();
    write_file(repo.path(), ".gitignore", ".env\n");
    write_file(repo.path(), "real.wti", ".env\n");
    std::os::unix::fs::symlink("real.wti", repo.path().join(".worktreeinclude")).unwrap();
    git(
        repo.path(),
        &["add", "-f", ".gitignore", "real.wti", ".worktreeinclude"],
    );
    git(repo.path(), &["commit", "-m", "init"]);
    write_file(repo.path(), ".env", "secret\n");
    repo
}

#[cfg(unix)]
#[test]
fn f8_symlink_policy_error_fails() {
    let repo = setup_f8();
    waft()
        .current_dir(repo.path())
        .args([
            "list",
            "--source",
            repo.path().to_str().unwrap(),
            "--compat-profile",
            "claude",
            "--worktreeinclude-symlink-policy",
            "error",
        ])
        .assert()
        .failure();
}

#[cfg(unix)]
#[test]
fn f8_symlink_policy_follow_selects_env() {
    let repo = setup_f8();
    let paths = list_paths(
        repo.path(),
        &[
            "--compat-profile",
            "claude",
            "--worktreeinclude-symlink-policy",
            "follow",
        ],
    );
    let expected: BTreeSet<String> = [".env".to_string()].into_iter().collect();
    assert_eq!(paths, expected);
}

#[cfg(unix)]
#[test]
fn f8_symlink_policy_ignore_selects_nothing_under_blank() {
    // Claude when_missing=blank means with the symlink hidden, no candidate
    // remains.
    let repo = setup_f8();
    let paths = list_paths(
        repo.path(),
        &[
            "--compat-profile",
            "claude",
            "--worktreeinclude-symlink-policy",
            "ignore",
        ],
    );
    assert!(
        paths.is_empty(),
        "claude + ignore should select nothing; got {paths:?}"
    );
}

// --- Semantics override (F3) ---

fn setup_f3() -> TempDir {
    let repo = make_repo();
    write_file(repo.path(), ".gitignore", "*.env\n");
    write_file(repo.path(), ".worktreeinclude", "*.env\n");
    write_file(repo.path(), "config/.worktreeinclude", "!*.env\n");
    git(
        repo.path(),
        &[
            "add",
            ".gitignore",
            ".worktreeinclude",
            "config/.worktreeinclude",
        ],
    );
    git(repo.path(), &["commit", "-m", "init"]);
    write_file(repo.path(), "root.env", "r\n");
    write_file(repo.path(), "config/sub.env", "s\n");
    repo
}

#[test]
fn f3_claude_with_git_semantics_excludes_subdir() {
    let repo = setup_f3();
    let paths = list_paths(
        repo.path(),
        &[
            "--compat-profile",
            "claude",
            "--worktreeinclude-semantics",
            "git",
        ],
    );
    let expected: BTreeSet<String> = ["root.env".to_string()].into_iter().collect();
    assert_eq!(paths, expected);
}

#[test]
fn f3_git_with_claude_semantics_keeps_both() {
    let repo = setup_f3();
    let paths = list_paths(
        repo.path(),
        &[
            "--compat-profile",
            "git",
            "--worktreeinclude-semantics",
            "claude-2026-04",
        ],
    );
    let expected: BTreeSet<String> = ["root.env".to_string(), "config/sub.env".to_string()]
        .into_iter()
        .collect();
    assert_eq!(paths, expected);
}

#[test]
fn replace_extra_excludes_drops_inherited() {
    // Same fixture, but rely on a project .waft.toml setting an extra
    // exclude that the CLI then replaces with a different one.
    let repo = setup_simple_two_files();
    write_file(
        repo.path(),
        ".waft.toml",
        "[exclude]\nextra = [\"*.env\"]\n",
    );
    git(repo.path(), &["add", "-f", ".waft.toml"]);
    git(repo.path(), &["commit", "-m", "add waft toml"]);

    // Without override: project drops .env, .log remains.
    let paths = list_paths(repo.path(), &["--compat-profile", "claude"]);
    let expected: BTreeSet<String> = ["drop.log".to_string()].into_iter().collect();
    assert_eq!(paths, expected);

    // CLI --replace-extra-excludes with a different list: only the CLI
    // exclude applies, project's `*.env` is dropped from inheritance.
    let paths = list_paths(
        repo.path(),
        &[
            "--compat-profile",
            "claude",
            "--extra-exclude",
            "*.log",
            "--replace-extra-excludes",
        ],
    );
    let expected: BTreeSet<String> = ["keep.env".to_string()].into_iter().collect();
    assert_eq!(paths, expected);
}
