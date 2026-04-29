use std::path::Path;
use std::process::{self, Output};

use tempfile::TempDir;

fn git(dir: &Path, args: &[&str]) {
    let output = process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .expect("failed to run git");
    assert!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn make_repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    git(dir.path(), &["init"]);
    git(dir.path(), &["config", "user.email", "test@test.com"]);
    git(dir.path(), &["config", "user.name", "Test"]);
    dir
}

fn run_waft(repo: &Path, backend: &str, args: &[&str]) -> Output {
    process::Command::new(env!("CARGO_BIN_EXE_waft"))
        .env("WAFT_GIT_BACKEND", backend)
        .args(args)
        .current_dir(repo)
        .output()
        .expect("failed to run waft")
}

#[test]
fn list_output_matches_between_backends() {
    let repo = make_repo();
    std::fs::write(
        repo.path().join(".gitignore"),
        "*.env\n!public.env\ntracked.env\n",
    )
    .unwrap();
    std::fs::write(repo.path().join(".worktreeinclude"), "*.env\n").unwrap();
    std::fs::write(repo.path().join("tracked.env"), "tracked\n").unwrap();
    git(
        repo.path(),
        &["add", "-f", ".gitignore", ".worktreeinclude", "tracked.env"],
    );
    git(repo.path(), &["commit", "-m", "setup"]);

    std::fs::write(repo.path().join(".env"), "a\n").unwrap();
    std::fs::write(repo.path().join("secret.env"), "b\n").unwrap();
    std::fs::write(repo.path().join("public.env"), "c\n").unwrap();

    let source = repo.path().to_string_lossy().to_string();
    let gix = run_waft(repo.path(), "gix", &["list", "--source", &source]);
    let cli = run_waft(repo.path(), "cli", &["list", "--source", &source]);

    assert!(
        gix.status.success(),
        "gix backend failed: {}",
        String::from_utf8_lossy(&gix.stderr)
    );
    assert!(
        cli.status.success(),
        "cli backend failed: {}",
        String::from_utf8_lossy(&cli.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&gix.stdout),
        String::from_utf8_lossy(&cli.stdout),
        "list output mismatch between gix and cli backends"
    );
}

/// Both backends must skip nested Git checkouts: registered submodules
/// (gitlink entries in the index) and independent nested clones (their own
/// `.git` directory). Otherwise the gix backend would copy files out of
/// those repositories — see PR #3 review feedback.
#[test]
fn list_skips_nested_git_checkouts_for_both_backends() {
    let repo = make_repo();
    std::fs::write(repo.path().join(".gitignore"), "*.env\n").unwrap();
    std::fs::write(repo.path().join(".worktreeinclude"), "*.env\n").unwrap();
    git(repo.path(), &["add", ".gitignore", ".worktreeinclude"]);

    // Register a submodule-shaped entry (gitlink) without needing a real
    // second repo. `update-index --cacheinfo 160000` is enough for git's
    // ls-files walker to recognize `sub/` as a submodule and skip it.
    let sub = repo.path().join("sub");
    std::fs::create_dir_all(&sub).unwrap();
    std::fs::write(sub.join(".git"), "gitdir: ../.git/modules/sub\n").unwrap();
    std::fs::write(sub.join("inner.env"), "inner\n").unwrap();
    git(
        repo.path(),
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            "160000,1111111111111111111111111111111111111111,sub",
        ],
    );

    git(repo.path(), &["commit", "-m", "setup"]);

    // A normal candidate at the top level — should appear.
    std::fs::write(repo.path().join("top.env"), "top\n").unwrap();

    // A nested independent checkout: its own `.git` *directory*.
    let nested = repo.path().join("nested");
    std::fs::create_dir_all(&nested).unwrap();
    git(&nested, &["init"]);
    std::fs::write(nested.join("inner.env"), "inner\n").unwrap();

    let source = repo.path().to_string_lossy().to_string();
    let gix = run_waft(repo.path(), "gix", &["list", "--source", &source]);
    let cli = run_waft(repo.path(), "cli", &["list", "--source", &source]);

    assert!(
        gix.status.success(),
        "gix backend failed: {}",
        String::from_utf8_lossy(&gix.stderr)
    );
    assert!(
        cli.status.success(),
        "cli backend failed: {}",
        String::from_utf8_lossy(&cli.stderr)
    );
    let gix_out = String::from_utf8_lossy(&gix.stdout).into_owned();
    let cli_out = String::from_utf8_lossy(&cli.stdout).into_owned();

    assert!(
        gix_out.contains("top.env"),
        "expected top.env in gix output, got:\n{gix_out}"
    );
    assert!(
        !gix_out.contains("sub/inner.env"),
        "gix backend leaked submodule contents:\n{gix_out}"
    );
    assert!(
        !gix_out.contains("nested/inner.env"),
        "gix backend leaked nested-repo contents:\n{gix_out}"
    );

    assert_eq!(
        gix_out, cli_out,
        "list output mismatch between gix and cli backends"
    );
}

/// Both backends must agree on the all-ignored fallback when no
/// `.worktreeinclude` exists. F2-style fixture: ignored file at root and
/// inside an ignored directory.
#[test]
fn list_all_ignored_when_missing_matches_between_backends() {
    let repo = make_repo();
    std::fs::write(repo.path().join(".gitignore"), ".env\ncache/\n").unwrap();
    git(repo.path(), &["add", ".gitignore"]);
    git(repo.path(), &["commit", "-m", "init"]);
    std::fs::write(repo.path().join(".env"), "secret\n").unwrap();
    std::fs::create_dir_all(repo.path().join("cache")).unwrap();
    std::fs::write(repo.path().join("cache/build.bin"), "data\n").unwrap();

    let source = repo.path().to_string_lossy().to_string();
    let args = &["list", "--compat-profile", "wt", "--source", &source];
    let gix = run_waft(repo.path(), "gix", args);
    let cli = run_waft(repo.path(), "cli", args);

    assert!(
        gix.status.success(),
        "gix backend failed: {}",
        String::from_utf8_lossy(&gix.stderr)
    );
    assert!(
        cli.status.success(),
        "cli backend failed: {}",
        String::from_utf8_lossy(&cli.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&gix.stdout),
        String::from_utf8_lossy(&cli.stdout),
        "wt all-ignored output mismatch between gix and cli backends"
    );
}

/// Both backends must agree on the existence check that gates the
/// `when_missing` fallback. With a `.worktreeinclude` present, the wt
/// profile must NOT switch to all-ignored.
#[test]
fn list_existence_gate_matches_between_backends() {
    let repo = make_repo();
    std::fs::write(repo.path().join(".gitignore"), ".env\ncache/\n").unwrap();
    // Empty .worktreeinclude file: present but selects nothing.
    std::fs::write(repo.path().join(".worktreeinclude"), "").unwrap();
    git(repo.path(), &["add", ".gitignore", ".worktreeinclude"]);
    git(repo.path(), &["commit", "-m", "init"]);
    std::fs::write(repo.path().join(".env"), "secret\n").unwrap();
    std::fs::create_dir_all(repo.path().join("cache")).unwrap();
    std::fs::write(repo.path().join("cache/build.bin"), "data\n").unwrap();

    let source = repo.path().to_string_lossy().to_string();
    let args = &["list", "--compat-profile", "wt", "--source", &source];
    let gix = run_waft(repo.path(), "gix", args);
    let cli = run_waft(repo.path(), "cli", args);

    assert!(gix.status.success() && cli.status.success());
    let gix_out = String::from_utf8_lossy(&gix.stdout);
    let cli_out = String::from_utf8_lossy(&cli.stdout);
    // Both backends should see the existing .worktreeinclude and stay in
    // explicit-selection mode (which selects nothing here).
    assert!(
        gix_out.trim().is_empty(),
        "gix backend wrongly fell back to all-ignored: {gix_out}"
    );
    assert_eq!(gix_out, cli_out);
}

#[test]
fn info_output_matches_between_backends() {
    let repo = make_repo();
    std::fs::write(
        repo.path().join(".gitignore"),
        "*.env\n!public.env\ntracked.env\n",
    )
    .unwrap();
    std::fs::write(repo.path().join(".worktreeinclude"), "*.env\n").unwrap();
    std::fs::write(repo.path().join("tracked.env"), "tracked\n").unwrap();
    git(
        repo.path(),
        &["add", "-f", ".gitignore", ".worktreeinclude", "tracked.env"],
    );
    git(repo.path(), &["commit", "-m", "setup"]);

    std::fs::write(repo.path().join(".env"), "a\n").unwrap();
    std::fs::write(repo.path().join("secret.env"), "b\n").unwrap();
    std::fs::write(repo.path().join("public.env"), "c\n").unwrap();
    std::fs::write(repo.path().join("note.txt"), "d\n").unwrap();

    let source = repo.path().to_string_lossy().to_string();
    let gix = run_waft(
        repo.path(),
        "gix",
        &[
            "info",
            "--source",
            &source,
            ".env",
            "secret.env",
            "public.env",
            "tracked.env",
            "note.txt",
        ],
    );
    let cli = run_waft(
        repo.path(),
        "cli",
        &[
            "info",
            "--source",
            &source,
            ".env",
            "secret.env",
            "public.env",
            "tracked.env",
            "note.txt",
        ],
    );

    assert!(
        gix.status.success(),
        "gix backend failed: {}",
        String::from_utf8_lossy(&gix.stderr)
    );
    assert!(
        cli.status.success(),
        "cli backend failed: {}",
        String::from_utf8_lossy(&cli.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&gix.stdout),
        String::from_utf8_lossy(&cli.stdout),
        "info output mismatch between gix and cli backends"
    );
}
