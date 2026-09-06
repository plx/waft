//! Profile-driven fixture integration tests.
//!
//! Each fixture from the worktreeinclude config matrix is exercised under
//! every supported `--compat-profile` value and Git backend.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

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

fn write_file(dir: &Path, rel_path: &str, content: &str) {
    let path = dir.join(rel_path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&path, content).unwrap();
}

/// Run `waft list` through every supported backend, assert parity, and return
/// the set of listed paths (one per stdout line, blank lines trimmed).
fn list_paths(source: &Path, extra_args: &[&str]) -> BTreeSet<String> {
    let outputs: Vec<(String, BTreeSet<String>)> = ["gix", "cli"]
        .into_iter()
        .map(|backend| {
            let mut cmd = waft();
            cmd.env("WAFT_GIT_BACKEND", backend)
                .timeout(std::time::Duration::from_secs(15))
                .args(["list", "--source"])
                .arg(source)
                .args(extra_args);
            let assert = cmd.assert().success();
            let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
            let paths = stdout
                .lines()
                .map(|line| line.trim().to_string())
                .filter(|line| !line.is_empty())
                .collect();
            (backend.to_string(), paths)
        })
        .collect();
    assert_eq!(
        outputs[0].1, outputs[1].1,
        "{} and {} backend results differ",
        outputs[0].0, outputs[1].0
    );
    outputs[0].1.clone()
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

// --- Empty-selection hint ---
//
// An empty result is ambiguous: it can mean "your rules select nothing" or
// "no rules were consulted". Only the second is worth a nudge, and it splits
// further — no rule file at all, one the active semantics never read, one the
// symlink policy skipped — because each names a different thing to change.
// The note must be true of the case it fires on, so each case is pinned.

/// The exact note the `claude` profile emits for F2.
const CLAUDE_MISSING_RULE_FILE_NOTE: &str = "note: no .worktreeinclude found; the claude profile selects nothing without one (see waft validate)";

fn run_list(source: &Path, extra_args: &[&str]) -> std::process::Output {
    waft()
        .args(["list", "--source"])
        .arg(source)
        .args(extra_args)
        .output()
        .unwrap()
}

#[test]
fn list_notes_a_missing_rule_file_without_polluting_stdout() {
    let repo = setup_f2();
    let output = run_list(repo.path(), &["--compat-profile", "claude"]);

    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(CLAUDE_MISSING_RULE_FILE_NOTE), "{stderr}");
    // `list` writes machine-readable output to stdout; the note must not
    // reach it.
    assert_eq!(String::from_utf8_lossy(&output.stdout), "");
}

#[test]
fn list_names_the_active_profile_in_the_note() {
    let repo = setup_f2();
    let output = run_list(repo.path(), &["--compat-profile", "git"]);

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("the git profile selects nothing without one"),
        "{stderr}"
    );
}

#[test]
fn list_note_is_suppressed_under_quiet() {
    let repo = setup_f2();
    let output = run_list(repo.path(), &["--compat-profile", "claude", "--quiet"]);

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

/// Under `wt` (and `--when-missing-worktreeinclude all-ignored`) an absent
/// rule file still selects files, so its absence is never the explanation for
/// an empty result.
#[test]
fn list_does_not_note_a_missing_rule_file_for_all_ignored_profiles() {
    let repo = make_repo();
    write_file(repo.path(), ".gitignore", ".env\n");
    git(repo.path(), &["add", ".gitignore"]);
    git(repo.path(), &["commit", "-m", "init"]);
    // Nothing ignored exists, so `wt` selects nothing either — but for a
    // reason the note would misdescribe.

    let output = run_list(repo.path(), &["--compat-profile", "wt"]);

    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("no .worktreeinclude found"), "{stderr}");
}

/// A rule file that selects nothing is a legitimate configuration, not a
/// missing one.
#[test]
fn list_does_not_note_a_rule_file_that_selects_nothing() {
    let repo = make_repo();
    write_file(repo.path(), ".gitignore", ".env\n");
    write_file(repo.path(), ".worktreeinclude", "nothing-matches-this\n");
    git(repo.path(), &["add", ".gitignore", ".worktreeinclude"]);
    git(repo.path(), &["commit", "-m", "init"]);
    write_file(repo.path(), ".env", "secret\n");

    let output = run_list(repo.path(), &["--compat-profile", "claude"]);

    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(stderr, "", "a present rule file needs no note");
}

/// `claude-2026-04` reads the root rule file only. A rule file that exists
/// exclusively in a subdirectory is therefore never consulted, and the run is
/// the same silent, empty, exit-0 result the note exists to explain — so it
/// must speak, and it must not claim the file is absent.
#[test]
fn list_notes_a_rule_file_the_root_only_semantics_never_read() {
    let repo = make_repo();
    write_file(repo.path(), ".gitignore", ".env\n");
    write_file(repo.path(), "sub/.worktreeinclude", ".env\n");
    git(repo.path(), &["add", ".gitignore", "sub/.worktreeinclude"]);
    git(repo.path(), &["commit", "-m", "init"]);
    write_file(repo.path(), ".env", "secret\n");
    write_file(repo.path(), "sub/.env", "secret\n");

    let output = run_list(repo.path(), &["--compat-profile", "claude"]);

    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(
            "note: no .worktreeinclude is consulted in the repository root; claude-2026-04 semantics ignore nested rule files"
        ),
        "{stderr}"
    );
    // The file exists; denying that would send the user looking for it.
    assert!(!stderr.contains("no .worktreeinclude found"), "{stderr}");
    assert_eq!(String::from_utf8_lossy(&output.stdout), "");
}

/// The same fixture under per-directory semantics selects the nested file, so
/// there is nothing to explain.
#[test]
fn list_does_not_note_a_nested_rule_file_the_git_semantics_read() {
    let repo = make_repo();
    write_file(repo.path(), ".gitignore", ".env\n");
    write_file(repo.path(), "sub/.worktreeinclude", ".env\n");
    git(repo.path(), &["add", ".gitignore", "sub/.worktreeinclude"]);
    git(repo.path(), &["commit", "-m", "init"]);
    write_file(repo.path(), "sub/.env", "secret\n");

    let output = run_list(repo.path(), &["--compat-profile", "git"]);

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("sub/.env"),
        "expected the nested rule file to select sub/.env"
    );
}

/// A symlinked rule file under `symlink-policy = ignore` (the `git` preset)
/// reads as absent to the selection gate. Reporting that as "no
/// .worktreeinclude found" tells the user a file they can see does not exist;
/// the note must name the policy that skipped it instead.
#[test]
#[cfg(unix)]
fn list_note_names_the_symlink_policy_that_skipped_a_rule_file() {
    let repo = make_repo();
    write_file(repo.path(), ".gitignore", ".env\n");
    write_file(repo.path(), "rules.txt", ".env\n");
    std::os::unix::fs::symlink("rules.txt", repo.path().join(".worktreeinclude")).unwrap();
    git(repo.path(), &["add", ".gitignore", "rules.txt"]);
    git(repo.path(), &["commit", "-m", "init"]);
    write_file(repo.path(), ".env", "secret\n");

    let output = run_list(repo.path(), &["--compat-profile", "git"]);

    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("note: the active symlink policy skips a .worktreeinclude symlink"),
        "{stderr}"
    );
    assert!(!stderr.contains("no .worktreeinclude found"), "{stderr}");

    // And the remedy the note names actually works.
    let followed = run_list(
        repo.path(),
        &[
            "--compat-profile",
            "git",
            "--worktreeinclude-symlink-policy",
            "follow",
        ],
    );
    assert!(
        String::from_utf8_lossy(&followed.stdout).contains(".env"),
        "following the symlink should select .env"
    );
}

/// A symlinked *root* rule file under root-only semantics, with a regular
/// nested one to satisfy the repo-wide existence gate. The gate says a rule
/// file exists, and the root-only check says the root file is not consulted —
/// but the reason is the symlink policy, not the file's absence. "No
/// .worktreeinclude in the repository root" would be false, and its remedy
/// (Git semantics) leaves the root symlink just as ignored, so the run stays
/// empty. The symlink note is both true here and actually curative.
#[cfg(unix)]
#[test]
fn list_note_names_the_symlink_policy_when_root_only_semantics_skip_the_root_symlink() {
    let repo = make_repo();
    write_file(repo.path(), ".gitignore", ".env\n");
    write_file(repo.path(), "rules.txt", ".env\n");
    std::os::unix::fs::symlink("rules.txt", repo.path().join(".worktreeinclude")).unwrap();
    // Regular, non-symlinked, and nested: this is what makes the repo-wide
    // existence check answer "found" while the root file stays unread.
    write_file(repo.path(), "sub/.worktreeinclude", ".env\n");
    git(
        repo.path(),
        &["add", ".gitignore", "rules.txt", "sub/.worktreeinclude"],
    );
    git(repo.path(), &["commit", "-m", "init"]);
    write_file(repo.path(), ".env", "secret\n");

    let ignored = run_list(
        repo.path(),
        &[
            "--compat-profile",
            "claude",
            "--worktreeinclude-symlink-policy",
            "ignore",
        ],
    );

    assert!(ignored.status.success());
    let stderr = String::from_utf8_lossy(&ignored.stderr);
    assert!(
        stderr.contains("note: the active symlink policy skips a .worktreeinclude symlink"),
        "{stderr}"
    );
    // The root file is right there; denying it sends the user to a remedy
    // that does not apply.
    assert!(
        !stderr.contains("no .worktreeinclude is consulted in the repository root"),
        "{stderr}"
    );
    assert_eq!(String::from_utf8_lossy(&ignored.stdout), "");

    // The remedy the wrong note would have recommended does not fix this run:
    // Git semantics read the nested file, and the root symlink stays skipped.
    let git_semantics = run_list(
        repo.path(),
        &[
            "--compat-profile",
            "claude",
            "--worktreeinclude-semantics",
            "git",
            "--worktreeinclude-symlink-policy",
            "ignore",
        ],
    );
    assert_eq!(
        String::from_utf8_lossy(&git_semantics.stdout),
        "",
        "switching to Git semantics should not rescue an ignored root symlink"
    );

    // The remedy the note does name works: root-only semantics read the
    // symlinked root file once the policy stops skipping it.
    let followed = run_list(
        repo.path(),
        &[
            "--compat-profile",
            "claude",
            "--worktreeinclude-symlink-policy",
            "follow",
        ],
    );
    assert!(
        String::from_utf8_lossy(&followed.stdout).contains(".env"),
        "following the symlink should let the root rule file select .env; got {:?}",
        String::from_utf8_lossy(&followed.stdout)
    );
}

/// The empty-selection diagnosis asks the backend for the repo-wide existence
/// answer and for the gitlink set, so a note that is only right on one backend
/// is a bug. Runs `waft list` on every supported backend, pins stdout empty,
/// and returns the stderr they agree on.
fn note_from_every_backend(source: &Path, extra_args: &[&str]) -> String {
    let notes: Vec<(&str, String)> = ["gix", "cli"]
        .into_iter()
        .map(|backend| {
            let output = waft()
                .env("WAFT_GIT_BACKEND", backend)
                .timeout(std::time::Duration::from_secs(15))
                .args(["list", "--source"])
                .arg(source)
                .args(extra_args)
                .output()
                .unwrap();
            assert!(output.status.success(), "{backend} backend: {output:?}");
            assert_eq!(
                String::from_utf8_lossy(&output.stdout),
                "",
                "{backend} backend polluted stdout"
            );
            (
                backend,
                String::from_utf8_lossy(&output.stderr).into_owned(),
            )
        })
        .collect();
    assert_eq!(
        notes[0].1, notes[1].1,
        "{} and {} backends disagree on the note",
        notes[0].0, notes[1].0
    );
    notes[0].1.clone()
}

/// The hint names an investigation. It does not claim validation will pass.
fn assert_conditional_symlink_note(stderr: &str) {
    assert!(
        stderr.contains("policy skips a .worktreeinclude symlink"),
        "{stderr}"
    );
    assert!(
        stderr.contains("inspect its target before trying"),
        "{stderr}"
    );
    assert!(
        stderr.contains("then run waft validate with that policy"),
        "{stderr}"
    );
    assert!(!stderr.contains("no .worktreeinclude found"), "{stderr}");
    assert!(!stderr.contains("to use it"), "{stderr}");
}

/// Validation continues to reject unreadable rule files under Follow.
fn following_symlinks_fails_to_read_the_rule_file(source: &Path, profile: &str) {
    let output = run_list(
        source,
        &[
            "--compat-profile",
            profile,
            "--worktreeinclude-symlink-policy",
            "follow",
        ],
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success() && stderr.contains("cannot read file"),
        "expected `follow` to fail on an unreadable rule file; got {:?} / {stderr}",
        output.status
    );
}

/// A dangling link is still an observed policy skip, not a promised cure.
#[cfg(unix)]
#[test]
fn list_conditionally_diagnoses_a_dangling_root_symlink() {
    let repo = make_repo();
    write_file(repo.path(), ".gitignore", ".env\n");
    std::os::unix::fs::symlink("no-such-rules.txt", repo.path().join(".worktreeinclude")).unwrap();
    git(repo.path(), &["add", ".gitignore"]);
    git(repo.path(), &["commit", "-m", "init"]);
    write_file(repo.path(), ".env", "secret\n");

    let stderr = note_from_every_backend(
        repo.path(),
        &[
            "--compat-profile",
            "claude",
            "--worktreeinclude-symlink-policy",
            "ignore",
        ],
    );

    assert_conditional_symlink_note(&stderr);
    following_symlinks_fails_to_read_the_rule_file(repo.path(), "claude");
}

/// The same for a link that resolves to a directory: the walk sees a symlink
/// named `.worktreeinclude`, but nothing behind it is a rule file to read.
#[cfg(unix)]
#[test]
fn list_conditionally_diagnoses_a_root_symlink_to_a_directory() {
    let repo = make_repo();
    write_file(repo.path(), ".gitignore", ".env\n");
    write_file(repo.path(), "rules.d/keep", "");
    std::os::unix::fs::symlink("rules.d", repo.path().join(".worktreeinclude")).unwrap();
    git(repo.path(), &["add", ".gitignore", "rules.d/keep"]);
    git(repo.path(), &["commit", "-m", "init"]);
    write_file(repo.path(), ".env", "secret\n");

    let stderr = note_from_every_backend(
        repo.path(),
        &[
            "--compat-profile",
            "claude",
            "--worktreeinclude-symlink-policy",
            "ignore",
        ],
    );

    assert_conditional_symlink_note(&stderr);
    following_symlinks_fails_to_read_the_rule_file(repo.path(), "claude");
}

/// A regular nested rule file must not hide the ignored root symlink.
#[cfg(unix)]
#[test]
fn list_conditionally_diagnoses_an_unreadable_root_symlink_under_root_only_semantics() {
    use std::os::unix::fs::PermissionsExt;

    let repo = make_repo();
    write_file(repo.path(), ".gitignore", ".env\n");
    write_file(repo.path(), "rules.txt", ".env\n");
    write_file(repo.path(), "sub/.worktreeinclude", ".env\n");
    std::os::unix::fs::symlink("rules.txt", repo.path().join(".worktreeinclude")).unwrap();
    git(
        repo.path(),
        &["add", ".gitignore", "rules.txt", "sub/.worktreeinclude"],
    );
    git(repo.path(), &["commit", "-m", "init"]);
    write_file(repo.path(), ".env", "secret\n");

    let rules = repo.path().join("rules.txt");
    fs::set_permissions(&rules, fs::Permissions::from_mode(0o000)).unwrap();
    if fs::read_to_string(&rules).is_ok() {
        // Running as a user mode bits do not constrain (root): the case under
        // test cannot be constructed here.
        return;
    }

    let stderr = note_from_every_backend(
        repo.path(),
        &[
            "--compat-profile",
            "claude",
            "--worktreeinclude-symlink-policy",
            "ignore",
        ],
    );

    assert_conditional_symlink_note(&stderr);
    following_symlinks_fails_to_read_the_rule_file(repo.path(), "claude");
}

/// Invalid UTF-8 must not turn a conditional hint into a promise.
#[cfg(unix)]
#[test]
fn list_conditionally_diagnoses_a_root_symlink_to_invalid_utf8() {
    let repo = make_repo();
    write_file(repo.path(), ".gitignore", ".env\n");
    fs::write(repo.path().join("rules.bin"), b"\xff\xfe.env\n").unwrap();
    std::os::unix::fs::symlink("rules.bin", repo.path().join(".worktreeinclude")).unwrap();
    git(repo.path(), &["add", ".gitignore", "rules.bin"]);
    git(repo.path(), &["commit", "-m", "init"]);
    write_file(repo.path(), ".env", "secret\n");

    let stderr = note_from_every_backend(
        repo.path(),
        &[
            "--compat-profile",
            "claude",
            "--worktreeinclude-symlink-policy",
            "ignore",
        ],
    );

    assert_conditional_symlink_note(&stderr);
    following_symlinks_fails_to_read_the_rule_file(repo.path(), "claude");
}

/// A readable rule-file symlink that exists only in a subdirectory, read by
/// root-only semantics under the policy that skips symlinks. Following it is
/// necessary and not sufficient: `claude-2026-04` never opens a nested rule
/// file, so `--worktreeinclude-symlink-policy follow` alone moves the run from
/// one silent empty result to another. Git semantics alone are equally inert,
/// because the file they would read is the one `ignore` skips. Only the pair
/// selects anything, so only the pair is honest to print.
#[cfg(unix)]
#[test]
fn list_pairs_both_knobs_for_a_nested_rule_file_symlink_under_root_only_semantics() {
    let repo = make_repo();
    write_file(repo.path(), ".gitignore", ".env\n");
    write_file(repo.path(), "sub/rules.txt", ".env\n");
    std::os::unix::fs::symlink("rules.txt", repo.path().join("sub/.worktreeinclude")).unwrap();
    git(
        repo.path(),
        &["add", ".gitignore", "sub/rules.txt", "sub/.worktreeinclude"],
    );
    git(repo.path(), &["commit", "-m", "init"]);
    write_file(repo.path(), "sub/.env", "secret\n");

    let stderr = note_from_every_backend(
        repo.path(),
        &[
            "--compat-profile",
            "claude",
            "--worktreeinclude-symlink-policy",
            "ignore",
        ],
    );

    assert!(
        !stderr.contains("pass --worktreeinclude-symlink-policy follow to use it"),
        "following alone leaves root-only semantics ignoring this file, so the \
         bare follow remedy must not be advertised: {stderr}"
    );
    assert!(
        stderr.contains("the active symlink policy skips a nested .worktreeinclude symlink"),
        "{stderr}"
    );
    assert!(
        stderr.contains(
            "trying --worktreeinclude-semantics git --worktreeinclude-symlink-policy follow"
        ),
        "the note must name the combination that actually reads the file: {stderr}"
    );

    // The remedy the note names selects the file.
    let both = run_list(
        repo.path(),
        &[
            "--compat-profile",
            "claude",
            "--worktreeinclude-semantics",
            "git",
            "--worktreeinclude-symlink-policy",
            "follow",
        ],
    );
    assert!(both.status.success(), "{both:?}");
    assert!(
        String::from_utf8_lossy(&both.stdout).contains("sub/.env"),
        "both knobs together should select sub/.env; got {:?}",
        String::from_utf8_lossy(&both.stdout)
    );

    // And neither half of it does on its own — the asymmetry that makes the
    // pairing, rather than either single flag, the true remedy.
    let follow_only = run_list(
        repo.path(),
        &[
            "--compat-profile",
            "claude",
            "--worktreeinclude-symlink-policy",
            "follow",
        ],
    );
    assert_eq!(
        String::from_utf8_lossy(&follow_only.stdout),
        "",
        "root-only semantics must not read a nested rule file just because the \
         policy stopped skipping it"
    );
    let git_semantics_only = run_list(
        repo.path(),
        &[
            "--compat-profile",
            "claude",
            "--worktreeinclude-semantics",
            "git",
            "--worktreeinclude-symlink-policy",
            "ignore",
        ],
    );
    assert_eq!(
        String::from_utf8_lossy(&git_semantics_only.stdout),
        "",
        "Git semantics must not rescue a rule file the active policy skips"
    );
}

/// A broken root symlink still prevents a follow run from validating.
#[cfg(unix)]
#[test]
fn list_keeps_the_hint_conditional_when_the_root_rule_file_symlink_is_broken() {
    let repo = make_repo();
    write_file(repo.path(), ".gitignore", ".env\n");
    write_file(repo.path(), "sub/rules.txt", ".env\n");
    std::os::unix::fs::symlink("rules.txt", repo.path().join("sub/.worktreeinclude")).unwrap();
    std::os::unix::fs::symlink("no-such-rules.txt", repo.path().join(".worktreeinclude")).unwrap();
    git(
        repo.path(),
        &["add", ".gitignore", "sub/rules.txt", "sub/.worktreeinclude"],
    );
    git(repo.path(), &["commit", "-m", "init"]);
    write_file(repo.path(), "sub/.env", "secret\n");

    let stderr = note_from_every_backend(
        repo.path(),
        &[
            "--compat-profile",
            "claude",
            "--worktreeinclude-symlink-policy",
            "ignore",
        ],
    );

    assert_conditional_symlink_note(&stderr);
    // Both halves of the remedy the previous test names, applied here, fail
    // outright rather than selecting anything.
    following_symlinks_fails_to_read_the_rule_file(repo.path(), "claude");
    let both = run_list(
        repo.path(),
        &[
            "--compat-profile",
            "claude",
            "--worktreeinclude-semantics",
            "git",
            "--worktreeinclude-symlink-policy",
            "follow",
        ],
    );
    assert!(
        !both.status.success()
            && String::from_utf8_lossy(&both.stderr).contains("cannot read file"),
        "expected the paired remedy to fail on the dangling root link; got {both:?}"
    );
}

/// Diagnosis must not read even a large sparse symlink target.
#[cfg(unix)]
#[test]
fn list_does_not_read_an_implausibly_large_symlink_target_to_diagnose() {
    use std::os::unix::fs::MetadataExt;

    let repo = make_repo();
    write_file(repo.path(), ".gitignore", ".env\n");
    let big = repo.path().join("rules.big");
    let Ok(file) = fs::File::create(&big) else {
        return;
    };
    if file.set_len(2 * 1024 * 1024 * 1024).is_err() {
        return;
    }
    drop(file);
    // Only meaningful while the file stays sparse; a filesystem that
    // materialized two gigabytes here cannot host this case cheaply.
    match fs::metadata(&big) {
        Ok(metadata) if metadata.blocks() * 512 <= 64 * 1024 * 1024 => {}
        _ => return,
    }
    std::os::unix::fs::symlink("rules.big", repo.path().join(".worktreeinclude")).unwrap();
    git(repo.path(), &["add", ".gitignore"]);
    git(repo.path(), &["commit", "-m", "init"]);
    write_file(repo.path(), ".env", "secret\n");

    let started = std::time::Instant::now();
    let stderr = note_from_every_backend(repo.path(), &["--compat-profile", "git"]);
    let elapsed = started.elapsed();

    assert_conditional_symlink_note(&stderr);
    assert!(
        elapsed < std::time::Duration::from_secs(30),
        "diagnosing an empty selection should not read the symlink target; took {elapsed:?}"
    );
}

/// `wt` selects every git-ignored untracked file without a rule file, so only
/// an explicit `--when-missing-worktreeinclude blank` can blank it. Naming the
/// profile there would point at the one setting that is not responsible.
#[test]
fn list_note_names_the_knob_not_the_profile_when_a_knob_blanks_selection() {
    let repo = make_repo();
    write_file(repo.path(), ".gitignore", ".env\n");
    git(repo.path(), &["add", ".gitignore"]);
    git(repo.path(), &["commit", "-m", "init"]);
    write_file(repo.path(), ".env", "secret\n");

    let output = run_list(
        repo.path(),
        &[
            "--compat-profile",
            "wt",
            "--when-missing-worktreeinclude",
            "blank",
        ],
    );

    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(
            "note: no .worktreeinclude found; --when-missing-worktreeinclude blank selects nothing without one"
        ),
        "{stderr}"
    );
    assert!(!stderr.contains("the wt profile"), "{stderr}");

    // Same repo, same profile, without the knob: `wt` selects the file, which
    // is what makes blaming the profile wrong.
    let unblanked = run_list(repo.path(), &["--compat-profile", "wt"]);
    assert!(
        String::from_utf8_lossy(&unblanked.stdout).contains(".env"),
        "wt without the knob should select .env"
    );
}

#[test]
fn info_notes_a_missing_rule_file() {
    let repo = setup_f2();
    let output = waft()
        .arg("-C")
        .arg(repo.path())
        .args(["info", ".env"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(CLAUDE_MISSING_RULE_FILE_NOTE), "{stderr}");
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

#[cfg(target_os = "macos")]
#[test]
fn tooling_filter_protects_native_case_alias_with_ignore_case_false() {
    let repo = make_repo();
    write_file(repo.path(), ".gitignore", ".Conductor/\n");
    write_file(repo.path(), ".worktreeinclude", ".Conductor/**/*.key\n");
    git(repo.path(), &["add", ".gitignore", ".worktreeinclude"]);
    git(
        repo.path(),
        &["commit", "-m", "configure mixed-case tool state"],
    );
    write_file(repo.path(), ".Conductor/state/dev.key", "key-data\n");
    assert!(
        repo.path().join(".conductor/state/dev.key").exists(),
        "macOS safety regression requires a case-insensitive test volume"
    );
    git(repo.path(), &["config", "core.ignoreCase", "false"]);

    let paths = list_paths(repo.path(), &["--compat-profile", "wt"]);
    assert!(
        paths.is_empty(),
        "tooling-v1 must follow the native filesystem's case aliases; got {paths:?}"
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

#[test]
fn f3_wt_profile_keeps_both_files() {
    let repo = setup_f3();
    let paths = list_paths(repo.path(), &["--compat-profile", "wt"]);
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

#[test]
fn f4_wt_profile_picks_up_all_ignored() {
    // wt always uses the all-ignored set; both `foo` and `config/foo`
    // are ignored by .gitignore.
    let repo = setup_f4();
    let paths = list_paths(repo.path(), &["--compat-profile", "wt"]);
    let expected: BTreeSet<String> = ["foo".to_string(), "config/foo".to_string()]
        .into_iter()
        .collect();
    assert_eq!(paths, expected);
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

#[test]
fn f5_wt_profile_literal_negation_drops_file() {
    // Wt honors literal-name negations: `!private.key` in
    // `secrets/.worktreeinclude` removes `secrets/private.key`.
    let repo = setup_f5();
    let paths = list_paths(repo.path(), &["--compat-profile", "wt"]);
    assert!(paths.is_empty(), "f5 wt should be empty; got {paths:?}");
}

#[test]
fn wt_ignores_literal_negations_inside_nested_repositories() {
    let repo = make_repo();
    write_file(repo.path(), ".gitignore", "victim.env\n");
    write_file(
        repo.path(),
        ".worktreeinclude",
        "# activate explicit mode\n",
    );
    git(repo.path(), &["add", ".gitignore", ".worktreeinclude"]);
    git(repo.path(), &["commit", "-m", "outer rules"]);
    write_file(repo.path(), "victim.env", "outer secret\n");

    let nested = repo.path().join("nested");
    fs::create_dir(&nested).unwrap();
    git(&nested, &["init"]);
    write_file(&nested, ".worktreeinclude", "!../victim.env\n");

    let paths = list_paths(repo.path(), &["--compat-profile", "wt"]);
    assert_eq!(
        paths,
        ["victim.env".to_string()].into_iter().collect(),
        "a nested repository must not change the outer wt selection"
    );
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

/// UTF-8 readability never established valid ignore syntax; diagnosis only
/// describes the active policy and leaves validation to the chosen settings.
#[cfg(unix)]
#[test]
fn ignored_invalid_syntax_symlink_hint_is_conditional() {
    let repo = make_repo();
    write_file(repo.path(), ".gitignore", ".env\n");
    write_file(repo.path(), "rules.txt", "\\\n");
    std::os::unix::fs::symlink("rules.txt", repo.path().join(".worktreeinclude")).unwrap();
    git(repo.path(), &["add", ".gitignore", "rules.txt"]);
    git(repo.path(), &["commit", "-m", "init"]);
    write_file(repo.path(), ".env", "secret\n");
    let note = note_from_every_backend(repo.path(), &["--compat-profile", "git"]);
    assert_conditional_symlink_note(&note);
    let followed = run_list(
        repo.path(),
        &[
            "--compat-profile",
            "git",
            "--worktreeinclude-symlink-policy",
            "follow",
        ],
    );
    assert!(
        !followed.status.success(),
        "invalid syntax must still fail validation"
    );
}

/// One valid target does not prove that every encountered rule symlink can
/// be validated. Keep both target files and selection semantics unchanged.
#[cfg(unix)]
#[test]
fn ignored_mixed_rule_symlinks_do_not_promise_success() {
    let repo = make_repo();
    write_file(repo.path(), ".gitignore", ".env\n");
    write_file(repo.path(), "rules.txt", ".env\n");
    write_file(repo.path(), "sub/keep", "");
    std::os::unix::fs::symlink("rules.txt", repo.path().join(".worktreeinclude")).unwrap();
    std::os::unix::fs::symlink("missing", repo.path().join("sub/.worktreeinclude")).unwrap();
    git(repo.path(), &["add", ".gitignore", "rules.txt", "sub/keep"]);
    git(repo.path(), &["commit", "-m", "init"]);
    write_file(repo.path(), ".env", "secret\n");
    let note = note_from_every_backend(repo.path(), &["--compat-profile", "git"]);
    assert_conditional_symlink_note(&note);
    following_symlinks_fails_to_read_the_rule_file(repo.path(), "git");
    assert_eq!(
        fs::read_to_string(repo.path().join(".env")).unwrap(),
        "secret\n"
    );
    assert_eq!(
        fs::read_to_string(repo.path().join("rules.txt")).unwrap(),
        ".env\n"
    );
}

#[cfg(unix)]
#[test]
fn ignored_rule_symlink_to_fifo_does_not_block_diagnosis() {
    let repo = make_repo();
    write_file(repo.path(), ".gitignore", ".env\n");
    git(repo.path(), &["add", ".gitignore"]);
    git(repo.path(), &["commit", "-m", "init"]);
    assert!(
        std::process::Command::new("mkfifo")
            .arg(repo.path().join("rules.pipe"))
            .status()
            .unwrap()
            .success()
    );
    std::os::unix::fs::symlink("rules.pipe", repo.path().join(".worktreeinclude")).unwrap();
    let note = note_from_every_backend(repo.path(), &["--compat-profile", "git"]);
    assert_conditional_symlink_note(&note);
}
