use assert_cmd::Command;
use predicates::prelude::*;

fn wiff() -> Command {
    Command::cargo_bin("wiff").unwrap()
}

#[test]
fn help_shows_usage() {
    wiff()
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
fn copy_help_shows_options() {
    wiff()
        .args(["copy", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--dry-run"))
        .stdout(predicate::str::contains("--overwrite"));
}

#[test]
fn list_help() {
    wiff()
        .args(["list", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("List eligible files"));
}

#[test]
fn info_help_shows_paths_arg() {
    wiff()
        .args(["info", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("<PATHS>"));
}

#[test]
fn validate_help() {
    wiff()
        .args(["validate", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Validate"));
}

#[test]
fn info_requires_paths() {
    wiff()
        .arg("info")
        .assert()
        .failure()
        .stderr(predicate::str::contains("PATHS"));
}

#[test]
fn no_subcommand_dispatches_to_copy() {
    // Running wiff with no subcommand should attempt copy (which is not
    // implemented yet and returns an error)
    wiff()
        .assert()
        .failure()
        .stderr(predicate::str::contains("copy"));
}

#[test]
fn version_flag() {
    wiff()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("wiff"));
}
