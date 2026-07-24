use std::path::Path;
use std::process::Output;

use tempfile::TempDir;
use waft::git::{GitBackend, GitCli, GitGix};

mod support;

fn git(dir: &Path, args: &[&str]) {
    let output = support::git_command()
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
    support::std_command(env!("CARGO_BIN_EXE_waft"))
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
    let output = String::from_utf8_lossy(&gix.stdout);
    assert!(output.contains(".env"));
    assert!(output.contains("secret.env"));
    assert!(
        !output.contains("public.env"),
        "negated Git match must not be eligible"
    );
}

#[test]
fn tracked_paths_respect_core_ignore_case_for_both_backends() {
    use waft::path::RepoRelPath;

    let repo = make_repo();
    std::fs::write(repo.path().join("secret.env"), "tracked\n").unwrap();
    git(repo.path(), &["add", "secret.env"]);
    git(repo.path(), &["commit", "-m", "track lower-case path"]);
    git(repo.path(), &["config", "core.ignoreCase", "true"]);

    let query = RepoRelPath::normalize(Path::new("SECRET.env"), repo.path()).unwrap();
    for backend in [
        &GitGix::new() as &dyn GitBackend,
        &GitCli::new() as &dyn GitBackend,
    ] {
        let tracked = backend
            .tracked_paths(repo.path(), std::slice::from_ref(&query))
            .unwrap();
        assert!(
            tracked.contains(&query),
            "backend failed to protect differently-cased tracked path"
        );
    }
}

#[test]
fn tracked_paths_use_normalized_unicode_folding_for_both_backends() {
    use waft::path::RepoRelPath;

    let repo = make_repo();
    std::fs::write(repo.path().join("ä.env"), "tracked\n").unwrap();
    git(repo.path(), &["add", "ä.env"]);
    git(
        repo.path(),
        &["commit", "-m", "track unicode lower-case path"],
    );
    git(repo.path(), &["config", "core.ignoreCase", "true"]);

    let query = RepoRelPath::normalize(Path::new("Ä.env"), repo.path()).unwrap();
    for backend in [
        &GitGix::new() as &dyn GitBackend,
        &GitCli::new() as &dyn GitBackend,
    ] {
        let tracked = backend
            .tracked_paths(repo.path(), std::slice::from_ref(&query))
            .unwrap();
        assert!(
            tracked.contains(&query),
            "backend failed to protect Unicode case-folded tracked path"
        );
    }
}

#[cfg(target_os = "macos")]
#[test]
fn tracked_paths_protect_native_sigma_alias_with_ignore_case_false() {
    use waft::path::RepoRelPath;

    let repo = make_repo();
    let tracked_path = repo.path().join("σ.env");
    std::fs::write(&tracked_path, "tracked\n").unwrap();
    assert!(
        repo.path().join("ς.env").exists(),
        "macOS safety regression requires a case-insensitive test volume"
    );
    git(repo.path(), &["add", "σ.env"]);
    git(repo.path(), &["commit", "-m", "track sigma spelling"]);
    git(repo.path(), &["config", "core.ignoreCase", "false"]);

    let query = RepoRelPath::normalize(Path::new("ς.env"), repo.path()).unwrap();
    for backend in [
        &GitGix::new() as &dyn GitBackend,
        &GitCli::new() as &dyn GitBackend,
    ] {
        let tracked = backend
            .tracked_paths(repo.path(), std::slice::from_ref(&query))
            .unwrap();
        assert!(
            tracked.contains(&query),
            "backend failed to protect the filesystem's sigma alias"
        );
    }

    // The conservative folded-name guard must also work while the tracked
    // worktree entry is absent and identity comparison is impossible.
    std::fs::remove_file(tracked_path).unwrap();
    for backend in [
        &GitGix::new() as &dyn GitBackend,
        &GitCli::new() as &dyn GitBackend,
    ] {
        let tracked = backend
            .tracked_paths(repo.path(), std::slice::from_ref(&query))
            .unwrap();
        assert!(
            tracked.contains(&query),
            "backend failed to protect the missing sigma alias"
        );
    }
}

#[cfg(unix)]
#[test]
fn tracked_paths_use_filesystem_identity_even_when_ignore_case_is_false() {
    use waft::path::RepoRelPath;

    let repo = make_repo();
    let tracked_path = repo.path().join("tracked.env");
    std::fs::write(&tracked_path, "tracked\n").unwrap();
    git(repo.path(), &["add", "tracked.env"]);
    git(repo.path(), &["commit", "-m", "track original hard link"]);
    std::fs::hard_link(&tracked_path, repo.path().join("alias.env")).unwrap();
    git(repo.path(), &["config", "core.ignoreCase", "false"]);

    let query = RepoRelPath::normalize(Path::new("alias.env"), repo.path()).unwrap();
    for backend in [
        &GitGix::new() as &dyn GitBackend,
        &GitCli::new() as &dyn GitBackend,
    ] {
        let tracked = backend
            .tracked_paths(repo.path(), std::slice::from_ref(&query))
            .unwrap();
        assert!(
            tracked.contains(&query),
            "backend failed to protect a filesystem alias of a tracked path"
        );
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
#[test]
fn repository_roots_with_invalid_utf8_are_preserved_by_both_backends() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let parent = TempDir::new().unwrap();
    let repo = parent.path().join(OsStr::from_bytes(b"repo-\xff"));
    std::fs::create_dir(&repo).unwrap();
    git(&repo, &["init"]);

    let expected = std::fs::canonicalize(&repo).unwrap();
    for backend in [
        &GitGix::new() as &dyn GitBackend,
        &GitCli::new() as &dyn GitBackend,
    ] {
        assert_eq!(
            backend.show_toplevel(&repo).unwrap(),
            expected,
            "backend changed raw bytes in the repository root"
        );
        assert_eq!(
            backend.list_worktrees(&repo).unwrap()[0].path,
            expected,
            "backend changed raw bytes in worktree-list output"
        );
    }
}

#[cfg(unix)]
#[test]
fn invalid_utf8_candidate_fails_closed_for_both_backends() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let repo = make_repo();
    std::fs::write(repo.path().join(".gitignore"), "*\n").unwrap();
    std::fs::write(repo.path().join(".worktreeinclude"), "*\n").unwrap();
    git(
        repo.path(),
        &["add", "-f", ".gitignore", ".worktreeinclude"],
    );
    git(repo.path(), &["commit", "-m", "select ignored files"]);
    let invalid_path = repo.path().join(OsStr::from_bytes(b"secret-\xff.env"));
    if let Err(error) = std::fs::write(&invalid_path, "x") {
        if cfg!(target_os = "macos") {
            // APFS/HFS+ reject malformed UTF-8 names before waft can inspect
            // them. Linux CI exercises the fail-closed behavior.
            return;
        }
        panic!("creating invalid UTF-8 fixture failed: {error}");
    }

    let source = repo.path().to_string_lossy().to_string();
    for backend in ["gix", "cli"] {
        let output = run_waft(repo.path(), backend, &["list", "--source", &source]);
        assert!(
            !output.status.success(),
            "{backend} backend accepted a non-UTF-8 candidate"
        );
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("not valid UTF-8"),
            "{backend} backend returned an unclear error: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[cfg(unix)]
#[test]
fn unrelated_unselected_invalid_utf8_name_does_not_break_explicit_selection() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let repo = make_repo();
    std::fs::write(repo.path().join(".gitignore"), ".env\n").unwrap();
    std::fs::write(repo.path().join(".worktreeinclude"), ".env\n").unwrap();
    git(repo.path(), &["add", ".gitignore", ".worktreeinclude"]);
    git(repo.path(), &["commit", "-m", "select one valid candidate"]);
    std::fs::write(repo.path().join(".env"), "selected").unwrap();
    if let Err(error) = std::fs::write(
        repo.path().join(OsStr::from_bytes(b"unrelated-\xff.txt")),
        "unselected",
    ) {
        if cfg!(target_os = "macos") {
            return;
        }
        panic!("creating invalid UTF-8 fixture failed: {error}");
    }

    let source = repo.path().to_string_lossy().to_string();
    for backend in ["gix", "cli"] {
        let output = run_waft(repo.path(), backend, &["list", "--source", &source]);
        assert!(
            output.status.success(),
            "{backend} backend failed on an unselected non-UTF-8 name: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            String::from_utf8_lossy(&output.stdout).contains(".env"),
            "{backend} backend omitted the selected UTF-8 candidate"
        );
    }
}

#[cfg(unix)]
#[test]
fn unrelated_unignored_invalid_utf8_name_does_not_break_fallback_parity() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let repo = make_repo();
    std::fs::write(repo.path().join(".gitignore"), "valid.env\n").unwrap();
    git(repo.path(), &["add", ".gitignore"]);
    git(repo.path(), &["commit", "-m", "ignore one valid candidate"]);
    std::fs::write(repo.path().join("valid.env"), "selected").unwrap();
    if let Err(error) = std::fs::write(
        repo.path().join(OsStr::from_bytes(b"unrelated-\xff.txt")),
        "unselected",
    ) {
        if cfg!(target_os = "macos") {
            return;
        }
        panic!("creating invalid UTF-8 fixture failed: {error}");
    }

    let expected = vec!["valid.env".to_string()];
    for backend in [
        &GitGix::new() as &dyn GitBackend,
        &GitCli::new() as &dyn GitBackend,
    ] {
        let paths = backend.list_ignored_untracked(repo.path()).unwrap();
        assert_eq!(
            paths
                .into_iter()
                .map(|path| path.as_str().to_string())
                .collect::<Vec<_>>(),
            expected,
            "unselected raw names must not affect all-ignored enumeration"
        );
    }
}

#[test]
fn candidate_filename_whitespace_matches_filesystem_spelling_for_both_backends() {
    let repo = make_repo();
    std::fs::write(repo.path().join(".gitignore"), "*\n").unwrap();
    git(repo.path(), &["add", "-f", ".gitignore"]);
    git(repo.path(), &["commit", "-m", "ignore fixture files"]);
    let requested_name = " secret.env ";
    std::fs::write(repo.path().join(requested_name), "x").unwrap();
    let actual_name = std::fs::read_dir(repo.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .find(|name| name.to_string_lossy().starts_with(" secret.env"))
        .and_then(|name| name.into_string().ok())
        .expect("candidate should exist with a representable filesystem name");
    #[cfg(not(windows))]
    assert_eq!(actual_name, requested_name);
    let expected = format!("{actual_name}\n");

    let source = repo.path().to_string_lossy().to_string();
    for backend in ["gix", "cli"] {
        let output = run_waft(
            repo.path(),
            backend,
            &["list", "--compat-profile", "wt", "--source", &source],
        );
        assert!(
            output.status.success(),
            "{backend} backend failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            expected,
            "{backend} backend did not report the filesystem's filename spelling"
        );
    }
}

#[cfg(target_os = "macos")]
#[test]
fn mixed_case_worktreeinclude_alias_is_discovered_by_both_backends() {
    let repo = make_repo();
    std::fs::write(repo.path().join(".gitignore"), "*.env\n").unwrap();
    std::fs::write(repo.path().join(".WorktreeInclude"), "*.env\n").unwrap();
    assert!(
        repo.path().join(".worktreeinclude").exists(),
        "macOS safety regression requires a case-insensitive test volume"
    );
    git(repo.path(), &["add", ".gitignore", ".WorktreeInclude"]);
    git(repo.path(), &["commit", "-m", "mixed-case control file"]);
    std::fs::write(repo.path().join("secret.env"), "selected").unwrap();

    let source = repo.path().to_string_lossy().to_string();
    for backend in ["gix", "cli"] {
        let output = run_waft(
            repo.path(),
            backend,
            &["list", "--isolated", "--source", &source],
        );
        assert!(
            output.status.success(),
            "{backend} failed mixed-case discovery: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            "secret.env\n",
            "{backend} did not apply the native .worktreeinclude alias"
        );
    }
}

#[cfg(target_os = "macos")]
#[test]
fn wt_literal_negation_uses_mixed_case_control_file_alias() {
    let repo = make_repo();
    std::fs::write(repo.path().join(".gitignore"), "secret.env\n").unwrap();
    std::fs::write(repo.path().join(".WorktreeInclude"), "!secret.env\n").unwrap();
    assert!(
        repo.path().join(".worktreeinclude").exists(),
        "macOS safety regression requires a case-insensitive test volume"
    );
    git(repo.path(), &["add", ".gitignore", ".WorktreeInclude"]);
    git(repo.path(), &["commit", "-m", "mixed-case wt rule"]);
    std::fs::write(repo.path().join("secret.env"), "excluded").unwrap();

    let source = repo.path().to_string_lossy().to_string();
    for backend in ["gix", "cli"] {
        let output = run_waft(
            repo.path(),
            backend,
            &[
                "list",
                "--isolated",
                "--compat-profile",
                "wt",
                "--source",
                &source,
            ],
        );
        assert!(output.status.success());
        assert!(
            output.stdout.is_empty(),
            "{backend} ignored the mixed-case wt literal negation"
        );
    }
}

#[test]
fn negated_gitignore_match_is_never_eligible_for_both_backends() {
    let repo = make_repo();
    std::fs::write(repo.path().join(".gitignore"), "*.env\n!keep.env\n").unwrap();
    std::fs::write(repo.path().join(".worktreeinclude"), "*.env\n").unwrap();
    git(
        repo.path(),
        &["add", "-f", ".gitignore", ".worktreeinclude"],
    );
    git(repo.path(), &["commit", "-m", "configure ignored files"]);
    std::fs::write(repo.path().join("drop.env"), "ignored\n").unwrap();
    std::fs::write(repo.path().join("keep.env"), "unignored\n").unwrap();

    let source = repo.path().to_string_lossy().to_string();
    for backend in ["gix", "cli"] {
        let output = run_waft(repo.path(), backend, &["list", "--source", &source]);
        assert!(
            output.status.success(),
            "{backend} backend failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            "drop.env\n",
            "{backend} backend treated a negated Git match as ignored"
        );
    }
}

#[test]
fn all_ignored_fallback_excludes_negated_gitignore_match_for_both_backends() {
    let repo = make_repo();
    std::fs::write(repo.path().join(".gitignore"), "*.env\n!keep.env\n").unwrap();
    git(repo.path(), &["add", "-f", ".gitignore"]);
    git(repo.path(), &["commit", "-m", "configure ignored files"]);
    std::fs::write(repo.path().join("drop.env"), "ignored\n").unwrap();
    std::fs::write(repo.path().join("keep.env"), "unignored\n").unwrap();

    let source = repo.path().to_string_lossy().to_string();
    for backend in ["gix", "cli"] {
        let output = run_waft(
            repo.path(),
            backend,
            &["list", "--compat-profile", "wt", "--source", &source],
        );
        assert!(
            output.status.success(),
            "{backend} backend failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            "drop.env\n",
            "{backend} all-ignored fallback included a negated Git match"
        );
    }
}

#[cfg(any(target_os = "macos", windows))]
#[test]
fn tracked_paths_protect_missing_case_alias_when_ignore_case_is_false() {
    use waft::path::RepoRelPath;

    let repo = make_repo();
    std::fs::write(repo.path().join("secret.env"), "tracked\n").unwrap();
    git(repo.path(), &["add", "secret.env"]);
    git(repo.path(), &["commit", "-m", "track lower-case path"]);
    std::fs::remove_file(repo.path().join("secret.env")).unwrap();
    git(repo.path(), &["config", "core.ignoreCase", "false"]);

    let query = RepoRelPath::normalize(Path::new("SECRET.env"), repo.path()).unwrap();
    for backend in [
        &GitGix::new() as &dyn GitBackend,
        &GitCli::new() as &dyn GitBackend,
    ] {
        let tracked = backend
            .tracked_paths(repo.path(), std::slice::from_ref(&query))
            .unwrap();
        assert!(
            tracked.contains(&query),
            "backend failed to protect a missing case alias of a tracked path"
        );
    }
}

#[test]
fn claude_root_only_nested_negation_matches_between_backends() {
    let repo = make_repo();
    std::fs::create_dir_all(repo.path().join("sub")).unwrap();
    std::fs::write(repo.path().join(".gitignore"), "*.env\n").unwrap();
    std::fs::write(repo.path().join(".worktreeinclude"), "*.env\n").unwrap();
    std::fs::write(repo.path().join("sub/.worktreeinclude"), "!nested.env\n").unwrap();
    git(
        repo.path(),
        &[
            "add",
            ".gitignore",
            ".worktreeinclude",
            "sub/.worktreeinclude",
        ],
    );
    git(repo.path(), &["commit", "-m", "nested rules"]);
    std::fs::write(repo.path().join("root.env"), "root\n").unwrap();
    std::fs::write(repo.path().join("sub/nested.env"), "nested\n").unwrap();

    let source = repo.path().to_string_lossy().to_string();
    let args = &["list", "--compat-profile", "claude", "--source", &source];
    let gix = run_waft(repo.path(), "gix", args);
    let cli = run_waft(repo.path(), "cli", args);

    assert!(gix.status.success() && cli.status.success());
    assert_eq!(
        String::from_utf8_lossy(&gix.stdout),
        String::from_utf8_lossy(&cli.stdout)
    );
    let output = String::from_utf8_lossy(&gix.stdout);
    assert!(output.contains("root.env"));
    assert!(output.contains("sub/nested.env"));
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

#[cfg(target_os = "macos")]
#[test]
fn mixed_case_dot_git_alias_remains_a_nested_repository_boundary() {
    let repo = make_repo();
    std::fs::write(repo.path().join(".gitignore"), "*.env\n").unwrap();
    std::fs::write(repo.path().join(".worktreeinclude"), "**/*.env\n").unwrap();
    git(repo.path(), &["add", ".gitignore", ".worktreeinclude"]);
    git(repo.path(), &["commit", "-m", "outer rules"]);

    let nested = repo.path().join("nested");
    std::fs::create_dir(&nested).unwrap();
    git(&nested, &["init"]);
    std::fs::rename(nested.join(".git"), nested.join(".git-temporary")).unwrap();
    std::fs::rename(nested.join(".git-temporary"), nested.join(".Git")).unwrap();
    assert!(
        nested.join(".git").exists(),
        "macOS safety regression requires a case-insensitive test volume"
    );
    std::fs::write(nested.join("secret.env"), "nested secret").unwrap();

    let source = repo.path().to_string_lossy().to_string();
    for backend in ["gix", "cli"] {
        let output = run_waft(repo.path(), backend, &["list", "--source", &source]);
        assert!(output.status.success());
        assert!(
            output.stdout.is_empty(),
            "{backend} descended through a mixed-case .git alias"
        );
    }
}

#[test]
fn gitlinks_parity() {
    let repo = make_repo();
    std::fs::create_dir_all(repo.path().join("sub")).unwrap();
    git(
        repo.path(),
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            "160000,1111111111111111111111111111111111111111,sub",
        ],
    );

    let gix = GitGix::new().gitlinks(repo.path()).unwrap();
    let cli = GitCli::new().gitlinks(repo.path()).unwrap();

    assert_eq!(gix, cli);
    assert!(gix.contains("sub"));
}

#[cfg(unix)]
#[test]
fn invalid_utf8_gitlink_fails_closed_for_both_backends() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let repo = make_repo();
    let cacheinfo = OsStr::from_bytes(b"160000,1111111111111111111111111111111111111111,sub-\xff");
    let output = support::git_command()
        .arg("-C")
        .arg(repo.path())
        .args(["update-index", "--add", "--cacheinfo"])
        .arg(cacheinfo)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "failed to create raw gitlink fixture: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    for backend in [
        &GitGix::new() as &dyn GitBackend,
        &GitCli::new() as &dyn GitBackend,
    ] {
        let error = backend.gitlinks(repo.path()).unwrap_err();
        assert!(
            error.to_string().contains("not valid UTF-8"),
            "backend returned an unclear raw gitlink error: {error}"
        );
    }
}

#[test]
fn gitlink_enumeration_fails_closed_on_corrupt_index_for_both_backends() {
    let repo = make_repo();
    std::fs::write(repo.path().join("tracked"), "x").unwrap();
    git(repo.path(), &["add", "tracked"]);
    git(repo.path(), &["commit", "-m", "create index"]);
    // Keep the file long enough that dependency parsers can report a normal
    // signature/checksum error instead of exercising their short-buffer
    // precondition paths.
    std::fs::write(repo.path().join(".git/index"), vec![0_u8; 1024]).unwrap();

    for backend in [
        &GitGix::new() as &dyn GitBackend,
        &GitCli::new() as &dyn GitBackend,
    ] {
        assert!(
            backend.gitlinks(repo.path()).is_err(),
            "backend treated an unreadable index as having no gitlinks"
        );
    }
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
/// `when_missing` fallback for the claude/git profiles. With a
/// `.worktreeinclude` present, those profiles must NOT switch to
/// all-ignored even if the rule file selects nothing.
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
    // Use the git profile here: claude/git both have when_missing=blank
    // and stay in explicit-selection mode when a rule file exists.
    let args = &["list", "--compat-profile", "git", "--source", &source];
    let gix = run_waft(repo.path(), "gix", args);
    let cli = run_waft(repo.path(), "cli", args);

    assert!(gix.status.success() && cli.status.success());
    let gix_out = String::from_utf8_lossy(&gix.stdout);
    let cli_out = String::from_utf8_lossy(&cli.stdout);
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
