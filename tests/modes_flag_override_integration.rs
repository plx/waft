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
    cmd.args(["list", "--source"]).arg(source);
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
