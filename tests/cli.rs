use predicates::prelude::*;

mod support;

use support::waft;

#[test]
fn help_shows_usage() {
    waft()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("worktreeinclude"))
        .stdout(predicate::str::contains("--source"))
        .stdout(predicate::str::contains("--dest"))
        .stdout(predicate::str::contains("--quiet"))
        .stdout(predicate::str::contains("--verbose"));
}

#[test]
fn help_shows_compat_profile_flags() {
    waft()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("--compat-profile"))
        .stdout(predicate::str::contains("--when-missing-worktreeinclude"))
        .stdout(predicate::str::contains("--worktreeinclude-semantics"))
        .stdout(predicate::str::contains("--worktreeinclude-symlink-policy"))
        .stdout(predicate::str::contains("--builtin-exclude-set"))
        .stdout(predicate::str::contains("--extra-exclude"))
        .stdout(predicate::str::contains("--replace-extra-excludes"))
        .stdout(predicate::str::contains("--copy-strategy"))
        .stdout(predicate::str::contains("--config"))
        .stdout(predicate::str::contains("--isolated"));
}

#[test]
fn invalid_copy_strategy_value_rejected() {
    waft()
        .args(["--copy-strategy", "warp"])
        .arg("list")
        .assert()
        .failure()
        .stderr(predicate::str::contains("warp"));
}

#[test]
fn invalid_compat_profile_value_rejected() {
    waft()
        .args(["--compat-profile", "rainbow"])
        .arg("list")
        .assert()
        .failure()
        .stderr(predicate::str::contains("rainbow"));
}

#[test]
fn isolated_conflicts_with_explicit_user_config() {
    waft()
        .args(["--isolated", "--config", "config.toml", "list"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("cannot be used with"));
}

#[test]
fn copy_help_shows_options() {
    waft()
        .args(["copy", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--dry-run"))
        .stdout(predicate::str::contains("--overwrite"));
}

#[test]
fn list_help() {
    waft()
        .args(["list", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("List eligible files"));
}

#[test]
fn info_help_shows_paths_arg() {
    waft()
        .args(["info", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("<PATHS>"));
}

#[test]
fn validate_help() {
    waft()
        .args(["validate", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Validate"));
}

#[test]
fn info_requires_paths() {
    waft()
        .arg("info")
        .assert()
        .failure()
        .stderr(predicate::str::contains("PATHS"));
}

/// Running outside a repository is the most common way to invoke waft
/// wrongly, so it gets a plain sentence rather than the backend's discovery
/// diagnostics — and the same sentence from either backend.
#[test]
fn outside_a_repository_reports_a_plain_message() {
    for backend in ["gix", "cli"] {
        let outside = tempfile::TempDir::new().unwrap();
        waft()
            .env("WAFT_GIT_BACKEND", backend)
            // A ceiling keeps discovery from walking out of the scratch
            // directory into whatever repository may contain the temp root.
            .env("GIT_CEILING_DIRECTORIES", outside.path())
            .current_dir(outside.path())
            .arg("list")
            .assert()
            .failure()
            .stderr(predicate::str::contains(
                "error: not inside a Git repository",
            ))
            .stderr(predicate::str::contains("searched from"))
            .stderr(predicate::str::contains("gix failed").not())
            .stderr(predicate::str::contains("error: git error:").not());
    }
}

/// The backend's own account of the failure stays reachable; it just is not
/// the headline.
#[test]
fn outside_a_repository_explains_itself_under_verbose() {
    let outside = tempfile::TempDir::new().unwrap();
    waft()
        .env("GIT_CEILING_DIRECTORIES", outside.path())
        .current_dir(outside.path())
        .args(["--verbose", "list"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "error: not inside a Git repository",
        ))
        .stderr(predicate::str::contains("caused by:"));
}

#[test]
fn version_flag() {
    waft()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("waft"));
}
