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

/// Probe whether `dir`'s filesystem resolves differently-cased spellings to
/// the same file, so tests can state which semantics the host can exercise.
fn filesystem_folds_case(dir: &Path) -> bool {
    let probe = dir.join("waft-case-probe");
    std::fs::write(&probe, b"probe").unwrap();
    let folded = dir.join("WAFT-CASE-PROBE").exists();
    std::fs::remove_file(&probe).unwrap();
    folded
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

/// A folded-name collision that `core.ignoreCase = false` says is a distinct
/// path is still protected when the filesystem itself resolves both spellings
/// to one file. The confirmation is targeted: only the colliding index entry
/// is consulted, never the whole index.
#[cfg(target_os = "macos")]
#[test]
fn tracked_paths_confirm_native_sigma_alias_through_filesystem_identity() {
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
}

/// The tracked-path lookup no longer scans the whole index for filesystem
/// aliases, so two deliberately distinct hard-linked names are two paths.
/// Only the one Git tracks is protected.
#[cfg(unix)]
#[test]
fn tracked_paths_treat_named_hard_links_as_distinct_paths() {
    use waft::path::RepoRelPath;

    let repo = make_repo();
    let tracked_path = repo.path().join("tracked.env");
    std::fs::write(&tracked_path, "tracked\n").unwrap();
    git(repo.path(), &["add", "tracked.env"]);
    git(repo.path(), &["commit", "-m", "track original hard link"]);
    std::fs::hard_link(&tracked_path, repo.path().join("alias.env")).unwrap();

    let alias = RepoRelPath::normalize(Path::new("alias.env"), repo.path()).unwrap();
    let tracked_query = RepoRelPath::normalize(Path::new("tracked.env"), repo.path()).unwrap();
    for backend in [
        &GitGix::new() as &dyn GitBackend,
        &GitCli::new() as &dyn GitBackend,
    ] {
        let tracked = backend
            .tracked_paths(repo.path(), &[alias.clone(), tracked_query.clone()])
            .unwrap();
        assert!(
            tracked.contains(&tracked_query),
            "backend failed to report the tracked path itself"
        );
        assert!(
            !tracked.contains(&alias),
            "a separately named hard link is not the tracked path"
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

/// With no worktree entry to compare identities against, the repository's own
/// `core.ignoreCase` decides whether a differently-cased spelling names the
/// tracked path. Both backends must read the same answer from the config, on
/// every host filesystem.
#[test]
fn tracked_paths_follow_configured_case_sensitivity_for_missing_aliases() {
    use waft::path::RepoRelPath;

    let repo = make_repo();
    std::fs::write(repo.path().join("secret.env"), "tracked\n").unwrap();
    git(repo.path(), &["add", "secret.env"]);
    git(repo.path(), &["commit", "-m", "track lower-case path"]);
    std::fs::remove_file(repo.path().join("secret.env")).unwrap();

    let query = RepoRelPath::normalize(Path::new("SECRET.env"), repo.path()).unwrap();

    git(repo.path(), &["config", "core.ignoreCase", "true"]);
    for backend in [
        &GitGix::new() as &dyn GitBackend,
        &GitCli::new() as &dyn GitBackend,
    ] {
        let tracked = backend
            .tracked_paths(repo.path(), std::slice::from_ref(&query))
            .unwrap();
        assert!(
            tracked.contains(&query),
            "a case-folding checkout must protect the differently-cased spelling"
        );
    }

    git(repo.path(), &["config", "core.ignoreCase", "false"]);
    for backend in [
        &GitGix::new() as &dyn GitBackend,
        &GitCli::new() as &dyn GitBackend,
    ] {
        let tracked = backend
            .tracked_paths(repo.path(), std::slice::from_ref(&query))
            .unwrap();
        assert!(
            !tracked.contains(&query),
            "a case-sensitive checkout must treat the differently-cased \
             spelling as a distinct, untracked path"
        );
    }
}

/// On a genuinely case-sensitive volume, a distinct untracked file whose name
/// folds onto a tracked one stays eligible.
#[test]
fn tracked_paths_keep_distinct_case_spellings_on_case_sensitive_volumes() {
    use waft::path::RepoRelPath;

    let repo = make_repo();
    if filesystem_folds_case(repo.path()) {
        // Both spellings would be one file here; the decision itself is
        // covered by the unit tests and by the missing-alias test above.
        return;
    }

    std::fs::write(repo.path().join("secret.env"), "tracked\n").unwrap();
    git(repo.path(), &["add", "secret.env"]);
    git(repo.path(), &["commit", "-m", "track lower-case path"]);
    git(repo.path(), &["config", "core.ignoreCase", "false"]);
    std::fs::write(repo.path().join("SECRET.env"), "untracked\n").unwrap();

    let query = RepoRelPath::normalize(Path::new("SECRET.env"), repo.path()).unwrap();
    for backend in [
        &GitGix::new() as &dyn GitBackend,
        &GitCli::new() as &dyn GitBackend,
    ] {
        let tracked = backend
            .tracked_paths(repo.path(), std::slice::from_ref(&query))
            .unwrap();
        assert!(
            !tracked.contains(&query),
            "a distinct file on a case-sensitive volume must not be reported \
             as tracked"
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

/// Git echoes each linked worktree's recorded path verbatim, and that record
/// can legitimately hold a symlinked spelling (a worktree relocated under a
/// symlinked mount, or repaired by hand). `show_toplevel` and the gix backend
/// both hand back canonical roots, so the CLI backend must normalize too:
/// otherwise main-vs-linked classification and the anchored copy engine end up
/// comparing a symlinked path against a canonical one.
#[cfg(unix)]
#[test]
fn symlinked_worktree_records_are_normalized_by_both_backends() {
    let parent = TempDir::new().unwrap();
    let real = parent.path().join("real");
    std::fs::create_dir(&real).unwrap();
    git(&real, &["init"]);
    git(&real, &["config", "user.email", "test@test.com"]);
    git(&real, &["config", "user.name", "Test"]);
    std::fs::write(real.join("tracked.txt"), "x\n").unwrap();
    git(&real, &["add", "tracked.txt"]);
    git(&real, &["commit", "-m", "init"]);

    let worktrees = parent.path().join("worktrees");
    std::fs::create_dir(&worktrees).unwrap();
    let linked = worktrees.join("linked");
    git(
        &real,
        &["worktree", "add", linked.to_str().unwrap(), "-b", "feature"],
    );

    // Re-record the linked worktree through a symlinked ancestor, and reach
    // the repository itself through another symlink.
    let worktrees_alias = parent.path().join("worktrees-alias");
    std::os::unix::fs::symlink(&worktrees, &worktrees_alias).unwrap();
    let gitdir_record = real.join(".git/worktrees/linked/gitdir");
    std::fs::write(
        &gitdir_record,
        format!("{}\n", worktrees_alias.join("linked/.git").display()),
    )
    .unwrap();
    let symlinked_root = parent.path().join("via-symlink");
    std::os::unix::fs::symlink(&real, &symlinked_root).unwrap();

    let expected_main = std::fs::canonicalize(&real).unwrap();
    let expected_linked = std::fs::canonicalize(&linked).unwrap();

    for backend in [
        &GitGix::new() as &dyn GitBackend,
        &GitCli::new() as &dyn GitBackend,
    ] {
        assert_eq!(
            backend.show_toplevel(&symlinked_root).unwrap(),
            expected_main,
            "backend returned a non-canonical toplevel through a symlink"
        );

        let records = backend.list_worktrees(&symlinked_root).unwrap();
        let main = records
            .iter()
            .find(|record| record.is_main)
            .expect("main worktree");
        assert_eq!(
            main.path, expected_main,
            "backend returned a non-canonical main worktree path"
        );
        assert!(
            records
                .iter()
                .any(|record| !record.is_main && record.path == expected_linked),
            "backend did not normalize the symlinked worktree record: {:?}",
            records.iter().map(|r| &r.path).collect::<Vec<_>>()
        );
    }
}

/// `WAFT_GIT_BACKEND` selects which implementation enforces waft's safety
/// checks, so a typo must fail loudly rather than fall back to the default.
#[test]
fn unknown_git_backend_selection_fails_with_the_valid_values() {
    let repo = make_repo();
    std::fs::write(repo.path().join(".worktreeinclude"), "*.env\n").unwrap();

    let source = repo.path().to_string_lossy().to_string();
    let output = run_waft(repo.path(), "gxi", &["list", "--source", &source]);

    assert!(
        !output.status.success(),
        "a misspelled backend name must not silently use the default"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("WAFT_GIT_BACKEND") && stderr.contains("\"gix\", \"cli\""),
        "error must name the variable and its valid values, got: {stderr}"
    );
}

/// Backend names are trimmed and matched without regard to ASCII case.
#[test]
fn git_backend_selection_accepts_case_and_padding_variants() {
    let repo = make_repo();
    std::fs::write(repo.path().join(".gitignore"), "*.env\n").unwrap();
    std::fs::write(repo.path().join(".worktreeinclude"), "*.env\n").unwrap();
    git(repo.path(), &["add", ".gitignore", ".worktreeinclude"]);
    git(repo.path(), &["commit", "-m", "setup"]);
    std::fs::write(repo.path().join("secret.env"), "s\n").unwrap();

    let source = repo.path().to_string_lossy().to_string();
    for backend in ["CLI", " cli ", "GIX", "gix"] {
        let output = run_waft(repo.path(), backend, &["list", "--source", &source]);
        assert!(
            output.status.success(),
            "backend {backend:?} was rejected: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&output.stdout), "secret.env\n");
    }
}

/// The executor rechecks trackedness of each destination path while holding
/// the index lock, and that recheck now reads a cached index snapshot rather
/// than the index itself. The cache is only sound if a real `git add` between
/// two queries invalidates it, so pin that against real repositories and real
/// backends rather than against a synthetic index file.
#[test]
fn a_reused_backend_instance_observes_a_path_becoming_tracked() {
    use waft::path::RepoRelPath;

    for backend_name in ["gix", "cli"] {
        let repo = make_repo();
        std::fs::write(repo.path().join(".gitignore"), "*.env\n").unwrap();
        git(repo.path(), &["add", "-f", ".gitignore"]);
        git(repo.path(), &["commit", "-m", "setup"]);
        std::fs::write(repo.path().join("secret.env"), "s\n").unwrap();

        // One instance for the whole exchange: this is the executor's usage,
        // and a fresh instance per query would not exercise the cache at all.
        let backend: Box<dyn GitBackend> = match backend_name {
            "gix" => Box::new(GitGix::new()),
            _ => Box::new(GitCli::new()),
        };
        let query = RepoRelPath::normalize(Path::new("secret.env"), repo.path()).unwrap();
        let paths = std::slice::from_ref(&query);

        assert!(
            !backend
                .tracked_paths(repo.path(), paths)
                .unwrap()
                .contains(&query),
            "{backend_name}: an ignored, unadded file must not read as tracked"
        );

        git(repo.path(), &["add", "-f", "secret.env"]);

        assert!(
            backend
                .tracked_paths(repo.path(), paths)
                .unwrap()
                .contains(&query),
            "{backend_name}: the same instance must observe the new index, \
             or the under-lock recheck would publish over a tracked path"
        );

        git(repo.path(), &["rm", "--cached", "-q", "secret.env"]);

        assert!(
            !backend
                .tracked_paths(repo.path(), paths)
                .unwrap()
                .contains(&query),
            "{backend_name}: the cached snapshot outlived the index it came from"
        );
    }
}

/// Companion to the above for the gitlink half of the same snapshot: a
/// registered submodule appearing in the index must be visible to a backend
/// instance that already answered a question about that repository.
#[test]
fn a_reused_backend_instance_observes_a_new_gitlink() {
    let submodule = make_repo();
    std::fs::write(submodule.path().join("inner.txt"), "i\n").unwrap();
    git(submodule.path(), &["add", "-f", "inner.txt"]);
    git(submodule.path(), &["commit", "-m", "inner"]);

    for backend_name in ["gix", "cli"] {
        let repo = make_repo();
        std::fs::write(repo.path().join("root.txt"), "r\n").unwrap();
        git(repo.path(), &["add", "-f", "root.txt"]);
        git(repo.path(), &["commit", "-m", "setup"]);

        let backend: Box<dyn GitBackend> = match backend_name {
            "gix" => Box::new(GitGix::new()),
            _ => Box::new(GitCli::new()),
        };

        assert!(
            backend.gitlinks(repo.path()).unwrap().is_empty(),
            "{backend_name}: no submodule is registered yet"
        );

        let name = format!("vendor-{backend_name}");
        git(
            repo.path(),
            &[
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "add",
                "-q",
                &submodule.path().to_string_lossy(),
                &name,
            ],
        );

        assert!(
            backend.gitlinks(repo.path()).unwrap().contains(&name),
            "{backend_name}: the same instance must observe the new gitlink"
        );
    }
}

/// `core.ignoreCase` comes from the config, not the index, so it must not be
/// revalidated by the index fingerprint: writing the index must neither
/// re-read it nor make a config edit appear to take effect. The backend
/// contract is that each instance resolves it once; a run therefore cannot
/// apply folded protection to some paths and exact matching to others.
#[test]
fn case_sensitivity_is_resolved_once_per_backend_instance() {
    use waft::path::RepoRelPath;

    let repo = make_repo();
    std::fs::write(repo.path().join("secret.env"), "tracked\n").unwrap();
    git(repo.path(), &["add", "secret.env"]);
    git(repo.path(), &["commit", "-m", "track lower-case path"]);
    std::fs::remove_file(repo.path().join("secret.env")).unwrap();

    let query = RepoRelPath::normalize(Path::new("SECRET.env"), repo.path()).unwrap();
    let paths = std::slice::from_ref(&query);

    for backend_name in ["gix", "cli"] {
        git(repo.path(), &["config", "core.ignoreCase", "true"]);
        let backend: Box<dyn GitBackend> = match backend_name {
            "gix" => Box::new(GitGix::new()),
            _ => Box::new(GitCli::new()),
        };
        assert!(
            backend.checkout_folds_case(repo.path()).unwrap(),
            "{backend_name}: the configured answer must be read"
        );

        // Change the config *and* the index. The index change is observed
        // (the file becomes untracked below is not what we assert here) but
        // it must not drag a re-read of the config along with it.
        git(repo.path(), &["config", "core.ignoreCase", "false"]);
        std::fs::write(repo.path().join("other.txt"), "o\n").unwrap();
        git(repo.path(), &["add", "-f", "other.txt"]);

        assert!(
            backend.checkout_folds_case(repo.path()).unwrap(),
            "{backend_name}: the case answer must stay fixed for this \
             instance rather than tracking the index's mtime"
        );
        assert!(
            backend
                .tracked_paths(repo.path(), paths)
                .unwrap()
                .contains(&query),
            "{backend_name}: tracked-path protection must use the same fixed \
             answer, not a freshly re-read one"
        );

        // A new instance is how the changed configuration is picked up.
        let reloaded: Box<dyn GitBackend> = match backend_name {
            "gix" => Box::new(GitGix::new()),
            _ => Box::new(GitCli::new()),
        };
        assert!(
            !reloaded.checkout_folds_case(repo.path()).unwrap(),
            "{backend_name}: a fresh instance must read the current config"
        );
        assert!(
            !reloaded
                .tracked_paths(repo.path(), paths)
                .unwrap()
                .contains(&query),
            "{backend_name}: a case-sensitive checkout treats the \
             differently-cased spelling as a distinct, untracked path"
        );

        git(repo.path(), &["rm", "--cached", "-q", "other.txt"]);
        let _ = std::fs::remove_file(repo.path().join("other.txt"));
    }
}
