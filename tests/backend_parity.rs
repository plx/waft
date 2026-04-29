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
