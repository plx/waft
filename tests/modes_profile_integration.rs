//! Profile-driven fixture integration tests.
//!
//! Each fixture from the worktreeinclude config matrix is exercised under
//! every supported `--compat-profile` value. Currently covers F2; later PRs
//! extend coverage as semantics engines and exclude filters land.

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

/// Run `waft list` with the given extra args, returning the set of listed
/// paths (one per stdout line, blank lines trimmed).
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

// --- Scenario F2: no-worktreeinclude ---
//
// Setup:
//   .gitignore: .env, cache/
//   no .worktreeinclude
//   source files: .env, cache/build.bin
//
// Expected:
//   claude: {}
//   git: {}
//   wt: {.env, cache/build.bin}

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
fn f2_claude_profile_blank() {
    let repo = setup_f2();
    let paths = list_paths(repo.path(), &["--compat-profile", "claude"]);
    assert!(
        paths.is_empty(),
        "claude profile should select nothing for F2; got {paths:?}"
    );
}

#[test]
fn f2_git_profile_blank() {
    let repo = setup_f2();
    let paths = list_paths(repo.path(), &["--compat-profile", "git"]);
    assert!(
        paths.is_empty(),
        "git profile should select nothing for F2; got {paths:?}"
    );
}

#[test]
fn f2_wt_profile_all_ignored() {
    let repo = setup_f2();
    let paths = list_paths(repo.path(), &["--compat-profile", "wt"]);
    let expected: BTreeSet<String> = [".env".to_string(), "cache/build.bin".to_string()]
        .into_iter()
        .collect();
    assert_eq!(
        paths, expected,
        "wt profile should list every ignored untracked file for F2"
    );
}

// --- Scenario F7: tool-state-directory ---
//
// Setup:
//   .gitignore: .conductor/
//   .worktreeinclude: .conductor/**/*.key
//   source files: .conductor/state/dev.key
//
// Expected:
//   claude: {.conductor/state/dev.key}
//   git: {.conductor/state/dev.key}
//   wt: {} (filtered by tooling-v1 builtin set)

fn setup_f7() -> TempDir {
    let repo = make_repo();
    write_file(repo.path(), ".gitignore", ".conductor/\n");
    write_file(repo.path(), ".worktreeinclude", ".conductor/**/*.key\n");
    git(repo.path(), &["add", ".gitignore", ".worktreeinclude"]);
    git(repo.path(), &["commit", "-m", "init"]);
    write_file(repo.path(), ".conductor/state/dev.key", "key-data\n");
    repo
}

#[test]
fn f7_claude_profile_keeps_conductor_key() {
    let repo = setup_f7();
    let paths = list_paths(repo.path(), &["--compat-profile", "claude"]);
    let expected: BTreeSet<String> = [".conductor/state/dev.key".to_string()]
        .into_iter()
        .collect();
    assert_eq!(paths, expected);
}

#[test]
fn f7_git_profile_keeps_conductor_key() {
    let repo = setup_f7();
    let paths = list_paths(repo.path(), &["--compat-profile", "git"]);
    let expected: BTreeSet<String> = [".conductor/state/dev.key".to_string()]
        .into_iter()
        .collect();
    assert_eq!(paths, expected);
}

#[test]
fn f7_wt_profile_drops_conductor_key() {
    let repo = setup_f7();
    let paths = list_paths(repo.path(), &["--compat-profile", "wt"]);
    assert!(
        paths.is_empty(),
        "wt profile should drop .conductor/* via tooling-v1 builtin set; got {paths:?}"
    );
}

// --- Scenario F8: symlinked-worktreeinclude (Unix only) ---
//
// Setup:
//   .gitignore: .env
//   symlink .worktreeinclude -> real.wti
//   real.wti: .env
//   source files: .env
//
// Expected outcomes:
//   claude (symlink_policy=follow): {.env}
//   git (symlink_policy=ignore): {} (symlinked rule file ignored)
//   wt (symlink_policy=follow): {.env}

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
fn f8_claude_profile_follows_symlink() {
    let repo = setup_f8();
    let paths = list_paths(repo.path(), &["--compat-profile", "claude"]);
    let expected: BTreeSet<String> = [".env".to_string()].into_iter().collect();
    assert_eq!(paths, expected);
}

#[cfg(unix)]
#[test]
fn f8_git_profile_ignores_symlink() {
    let repo = setup_f8();
    let paths = list_paths(repo.path(), &["--compat-profile", "git"]);
    // git preset's symlink_policy=ignore plus when_missing=blank produces {}.
    assert!(
        paths.is_empty(),
        "git profile should ignore symlinked rule file; got {paths:?}"
    );
}

#[cfg(unix)]
#[test]
fn f8_wt_profile_follows_symlink() {
    let repo = setup_f8();
    let paths = list_paths(repo.path(), &["--compat-profile", "wt"]);
    let expected: BTreeSet<String> = [".env".to_string()].into_iter().collect();
    assert_eq!(paths, expected);
}
