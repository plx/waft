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

// --- F1: root-simple ---
//
// .gitignore: .env
// .worktreeinclude: .env
// source files: .env
// Expected: claude/git/wt all → {.env}

fn setup_f1() -> TempDir {
    let repo = make_repo();
    write_file(repo.path(), ".gitignore", ".env\n");
    write_file(repo.path(), ".worktreeinclude", ".env\n");
    git(repo.path(), &["add", ".gitignore", ".worktreeinclude"]);
    git(repo.path(), &["commit", "-m", "init"]);
    write_file(repo.path(), ".env", "secret\n");
    repo
}

#[test]
fn f1_git_profile_selects_env() {
    let repo = setup_f1();
    let paths = list_paths(repo.path(), &["--compat-profile", "git"]);
    let expected: BTreeSet<String> = [".env".to_string()].into_iter().collect();
    assert_eq!(paths, expected);
}

#[test]
fn f1_claude_profile_selects_env() {
    let repo = setup_f1();
    let paths = list_paths(repo.path(), &["--compat-profile", "claude"]);
    let expected: BTreeSet<String> = [".env".to_string()].into_iter().collect();
    assert_eq!(paths, expected);
}

#[test]
fn f1_wt_profile_selects_env() {
    let repo = setup_f1();
    let paths = list_paths(repo.path(), &["--compat-profile", "wt"]);
    let expected: BTreeSet<String> = [".env".to_string()].into_iter().collect();
    assert_eq!(paths, expected);
}

// --- F3: nested-worktreeinclude-override under git ---
//
// .gitignore: *.env
// root .worktreeinclude: *.env
// config/.worktreeinclude: !*.env
// source files: root.env, config/sub.env
// Expected git: {root.env}

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
fn f3_git_profile_negation_excludes_subdir() {
    let repo = setup_f3();
    let paths = list_paths(repo.path(), &["--compat-profile", "git"]);
    let expected: BTreeSet<String> = ["root.env".to_string()].into_iter().collect();
    assert_eq!(paths, expected);
}

#[test]
fn f3_claude_profile_keeps_both_files() {
    let repo = setup_f3();
    let paths = list_paths(repo.path(), &["--compat-profile", "claude"]);
    let expected: BTreeSet<String> = ["root.env".to_string(), "config/sub.env".to_string()]
        .into_iter()
        .collect();
    assert_eq!(paths, expected);
}

// --- F4: nested-anchored-pattern under git ---
//
// .gitignore: foo, config/foo
// config/.worktreeinclude: /foo
// source files: foo, config/foo
// Expected git: {config/foo}

fn setup_f4() -> TempDir {
    let repo = make_repo();
    write_file(repo.path(), ".gitignore", "foo\nconfig/foo\n");
    write_file(repo.path(), "config/.worktreeinclude", "/foo\n");
    git(
        repo.path(),
        &["add", ".gitignore", "config/.worktreeinclude"],
    );
    git(repo.path(), &["commit", "-m", "init"]);
    write_file(repo.path(), "foo", "f\n");
    write_file(repo.path(), "config/foo", "f\n");
    repo
}

#[test]
fn f4_git_profile_anchored_pattern_only_matches_subdir() {
    let repo = setup_f4();
    let paths = list_paths(repo.path(), &["--compat-profile", "git"]);
    let expected: BTreeSet<String> = ["config/foo".to_string()].into_iter().collect();
    assert_eq!(paths, expected);
}

#[test]
fn f4_claude_profile_blank_no_root_file() {
    // No root .worktreeinclude, claude ignores nested files.
    // when_missing=blank → {}.
    let repo = setup_f4();
    let paths = list_paths(repo.path(), &["--compat-profile", "claude"]);
    assert!(paths.is_empty(), "f4 claude should be empty; got {paths:?}");
}

// --- F5: cross-file-negation-caveat under git ---
//
// .gitignore: secrets/
// root .worktreeinclude: secrets/
// secrets/.worktreeinclude: !private.key
// source files: secrets/private.key
// Expected git: {secrets/private.key} (negation blocked by parent dir)

fn setup_f5() -> TempDir {
    let repo = make_repo();
    write_file(repo.path(), ".gitignore", "secrets/\n");
    write_file(repo.path(), ".worktreeinclude", "secrets/\n");
    write_file(repo.path(), "secrets/.worktreeinclude", "!private.key\n");
    // -f: secrets/.worktreeinclude lives under a gitignored directory.
    git(
        repo.path(),
        &[
            "add",
            "-f",
            ".gitignore",
            ".worktreeinclude",
            "secrets/.worktreeinclude",
        ],
    );
    git(repo.path(), &["commit", "-m", "init"]);
    write_file(repo.path(), "secrets/private.key", "k\n");
    repo
}

#[test]
fn f5_git_profile_caveat_blocks_nested_negation() {
    let repo = setup_f5();
    let paths = list_paths(repo.path(), &["--compat-profile", "git"]);
    let expected: BTreeSet<String> = ["secrets/private.key".to_string()].into_iter().collect();
    assert_eq!(paths, expected);
}

#[test]
fn f5_claude_profile_ignores_nested_negation() {
    // Claude only consults the root .worktreeinclude (which selects
    // secrets/), so the nested negation has no effect.
    let repo = setup_f5();
    let paths = list_paths(repo.path(), &["--compat-profile", "claude"]);
    let expected: BTreeSet<String> = ["secrets/private.key".to_string()].into_iter().collect();
    assert_eq!(paths, expected);
}

// --- F6: nested-worktree-in-repo (all profiles agree) ---

fn setup_f6() -> TempDir {
    let main_repo = make_repo();
    write_file(main_repo.path(), ".gitignore", ".worktrees/\n");
    write_file(
        main_repo.path(),
        ".worktreeinclude",
        ".worktrees/**/*.env\n",
    );
    git(main_repo.path(), &["add", ".gitignore", ".worktreeinclude"]);
    git(main_repo.path(), &["commit", "-m", "init"]);
    // Create a real linked worktree inside .worktrees/.
    let nested = main_repo.path().join(".worktrees/nested");
    git(
        main_repo.path(),
        &["worktree", "add", nested.to_str().unwrap(), "-b", "feature"],
    );
    write_file(&nested, ".env", "n\n");
    main_repo
}

#[test]
fn f6_git_profile_skips_nested_worktree_contents() {
    let repo = setup_f6();
    let paths = list_paths(repo.path(), &["--compat-profile", "git"]);
    assert!(
        paths.is_empty(),
        "f6 should not enumerate nested worktree contents; got {paths:?}"
    );
}

#[test]
fn f6_claude_profile_skips_nested_worktree_contents() {
    let repo = setup_f6();
    let paths = list_paths(repo.path(), &["--compat-profile", "claude"]);
    assert!(paths.is_empty());
}

#[test]
fn f6_wt_profile_skips_nested_worktree_contents() {
    let repo = setup_f6();
    let paths = list_paths(repo.path(), &["--compat-profile", "wt"]);
    assert!(paths.is_empty());
}
