use assert_cmd::Command;
use predicates::prelude::*;

fn waft() -> Command {
    Command::cargo_bin("waft").unwrap()
}

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

#[test]
fn no_subcommand_dispatches_to_copy() {
    // Running waft with no subcommand should attempt copy with default args.
    waft()
        .assert()
        .success()
        .stderr(predicate::str::contains("no eligible files found"));
}

#[test]
fn version_flag() {
    waft()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("waft"));
}
