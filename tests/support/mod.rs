//! Hermetic child-process helpers shared by integration tests.
#![allow(dead_code)]

use std::ffi::OsStr;
use std::process;
use std::sync::OnceLock;

use assert_cmd::Command;
use tempfile::TempDir;

const WAFT_POLICY_ENV: &[&str] = &[
    "WAFT_CONFIG_PATH",
    "WAFT_COMPAT_PROFILE",
    "WAFT_WHEN_MISSING_WORKTREEINCLUDE",
    "WAFT_WORKTREEINCLUDE_SEMANTICS",
    "WAFT_WORKTREEINCLUDE_SYMLINK_POLICY",
    "WAFT_BUILTIN_EXCLUDE_SET",
    "WAFT_EXTRA_EXCLUDE",
    "WAFT_REPLACE_EXTRA_EXCLUDES",
    "WAFT_COPY_STRATEGY",
    "WAFT_GIT_BACKEND",
];

const GIT_ROUTING_ENV: &[&str] = &[
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_CEILING_DIRECTORIES",
    "GIT_DISCOVERY_ACROSS_FILESYSTEM",
    "GIT_CONFIG",
    "GIT_CONFIG_SYSTEM",
    "GIT_CONFIG_PARAMETERS",
    "GIT_NAMESPACE",
    "GIT_PREFIX",
    "GIT_SHALLOW_FILE",
    "GIT_QUARANTINE_PATH",
    "GIT_LITERAL_PATHSPECS",
    "GIT_GLOB_PATHSPECS",
    "GIT_NOGLOB_PATHSPECS",
    "GIT_ICASE_PATHSPECS",
    "GIT_INDEX_VERSION",
    "GIT_DEFAULT_HASH",
    "GIT_DEFAULT_REF_FORMAT",
];

fn git_null_device() -> &'static str {
    if cfg!(windows) { "NUL" } else { "/dev/null" }
}

fn empty_xdg_config_home() -> std::path::PathBuf {
    static XDG_HOME: OnceLock<TempDir> = OnceLock::new();
    XDG_HOME
        .get_or_init(|| TempDir::new().expect("isolated test config directory"))
        .path()
        .to_path_buf()
}

fn empty_home() -> std::path::PathBuf {
    static HOME_DIR: OnceLock<TempDir> = OnceLock::new();
    HOME_DIR
        .get_or_init(|| TempDir::new().expect("isolated test home directory"))
        .path()
        .to_path_buf()
}

fn configure_std(command: &mut process::Command) {
    command
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", git_null_device())
        .env("GIT_CONFIG_COUNT", "0")
        .env("GIT_ATTR_NOSYSTEM", "1")
        .env("XDG_CONFIG_HOME", empty_xdg_config_home())
        .env("HOME", empty_home())
        .env("USERPROFILE", empty_home());
    for key in WAFT_POLICY_ENV {
        command.env_remove(key);
    }
    for key in GIT_ROUTING_ENV {
        command.env_remove(key);
    }
}

fn configure_assert(command: &mut Command) {
    command
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", git_null_device())
        .env("GIT_CONFIG_COUNT", "0")
        .env("GIT_ATTR_NOSYSTEM", "1")
        .env("XDG_CONFIG_HOME", empty_xdg_config_home())
        .env("HOME", empty_home())
        .env("USERPROFILE", empty_home());
    for key in WAFT_POLICY_ENV {
        command.env_remove(key);
    }
    for key in GIT_ROUTING_ENV {
        command.env_remove(key);
    }
}

pub fn git_command() -> process::Command {
    let mut command = process::Command::new("git");
    configure_std(&mut command);
    command
}

pub fn std_command(program: impl AsRef<OsStr>) -> process::Command {
    let mut command = process::Command::new(program);
    configure_std(&mut command);
    command
}

pub fn waft() -> Command {
    let mut command = Command::cargo_bin("waft").expect("waft test binary");
    configure_assert(&mut command);
    command
}
