//! Integration tests for config discovery and merge precedence.
//!
//! Layers (lowest precedence first): defaults < user file < project files
//! (outermost-first) < env < CLI. These tests exercise discovery against a
//! real on-disk layout.

use std::path::PathBuf;

use tempfile::TempDir;

use waft::config::{
    BuiltinExcludeSet, CompatProfile, ConfigLayer, PolicyResolutionInputs, ResolvedPolicy,
    SymlinkPolicy, WhenMissingWorktreeinclude, WorktreeincludeSemantics, discover_project_configs,
    layer_from_env_iter, load_project_layers, load_user_layer, parse_toml,
};

fn write(path: &std::path::Path, content: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, content).unwrap();
}

#[test]
fn project_discovery_walks_to_repo_root() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    let inner = root.join("a/b");
    std::fs::create_dir_all(&inner).unwrap();
    std::fs::create_dir_all(root.join(".git")).unwrap();
    write(&root.join(".waft.toml"), "[compat]\nprofile = \"git\"\n");
    write(
        &root.join("a/.waft.toml"),
        "[worktreeinclude]\nsemantics = \"git\"\n",
    );
    write(
        &inner.join(".waft.toml"),
        "[exclude]\nbuiltin_set = \"tooling-v1\"\n",
    );

    let paths = discover_project_configs(&inner);
    let expected: Vec<PathBuf> = vec![
        root.join(".waft.toml"),
        root.join("a/.waft.toml"),
        inner.join(".waft.toml"),
    ];
    assert_eq!(paths, expected);
}

#[test]
fn project_discovery_stops_above_repo_root() {
    let tmp = TempDir::new().unwrap();
    let above = tmp.path();
    let repo = above.join("repo");
    let inner = repo.join("src");
    std::fs::create_dir_all(&inner).unwrap();
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    write(
        &above.join(".waft.toml"),
        "[compat]\nprofile = \"wt\"\n", // should NOT be picked up
    );
    write(&repo.join(".waft.toml"), "[compat]\nprofile = \"git\"\n");

    let paths = discover_project_configs(&inner);
    assert_eq!(paths, vec![repo.join(".waft.toml")]);
}

#[test]
fn project_discovery_no_repo_walks_to_filesystem_root() {
    // Without any `.git`, discovery walks to filesystem root. We assert it
    // returns empty when no `.waft.toml` exists in any ancestor.
    let tmp = TempDir::new().unwrap();
    let nested = tmp.path().join("a/b/c");
    std::fs::create_dir_all(&nested).unwrap();
    let paths = discover_project_configs(&nested);
    assert!(paths.is_empty());
}

#[test]
fn user_layer_load_from_path() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("user-config.toml");
    write(
        &path,
        r#"
[compat]
profile = "git"

[exclude]
extra = ["user-extra"]
"#,
    );
    let layer = load_user_layer(Some(path.as_path())).unwrap().unwrap();
    assert_eq!(layer.profile, Some(CompatProfile::Git));
    assert_eq!(layer.extra_excludes, vec!["user-extra".to_string()]);
}

#[test]
fn user_layer_missing_file_yields_none() {
    let layer = load_user_layer(Some(std::path::Path::new("/no/such/file.toml"))).unwrap();
    assert!(layer.is_none());
}

#[test]
fn user_layer_invalid_toml_returns_error() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("bad.toml");
    write(&path, "garbage = 1\n");
    let err = load_user_layer(Some(path.as_path())).unwrap_err();
    assert!(err.to_string().contains("garbage"));
}

#[test]
fn project_layer_loading_returns_in_order() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    let mid = root.join("a");
    let inner = mid.join("b");
    std::fs::create_dir_all(&inner).unwrap();
    std::fs::create_dir_all(root.join(".git")).unwrap();
    write(&root.join(".waft.toml"), "[compat]\nprofile = \"git\"\n");
    write(&mid.join(".waft.toml"), "[compat]\nprofile = \"wt\"\n");
    write(
        &inner.join(".waft.toml"),
        "[compat]\nprofile = \"claude\"\n",
    );

    let paths = discover_project_configs(&inner);
    let layers = load_project_layers(&paths).unwrap();
    assert_eq!(layers.len(), 3);
    assert_eq!(layers[0].profile, Some(CompatProfile::Git));
    assert_eq!(layers[1].profile, Some(CompatProfile::Wt));
    assert_eq!(layers[2].profile, Some(CompatProfile::Claude));
}

#[test]
fn full_precedence_defaults_user_project_env_cli() {
    // defaults < user < project (outermost-first) < env < CLI
    // Build each layer such that we can detect which one "won" for several keys.

    // user: sets profile=Git, semantics=Git
    let user = parse_toml(
        "user",
        r#"
[compat]
profile = "git"

[worktreeinclude]
semantics = "git"
"#,
    )
    .unwrap();

    // project: outermost overrides profile to Wt; innermost overrides semantics to wt-0.39
    let outer = parse_toml(
        "outer",
        r#"
[compat]
profile = "wt"
"#,
    )
    .unwrap();
    let inner = parse_toml(
        "inner",
        r#"
[worktreeinclude]
semantics = "wt-0.39"
"#,
    )
    .unwrap();

    // env: overrides when_missing
    let env = layer_from_env_iter(vec![(
        "WAFT_WHEN_MISSING_WORKTREEINCLUDE".to_string(),
        "all-ignored".to_string(),
    )])
    .unwrap();

    // CLI: overrides symlink_policy AND profile (highest precedence)
    let cli = ConfigLayer {
        profile: Some(CompatProfile::Claude),
        symlink_policy: Some(SymlinkPolicy::Follow),
        ..ConfigLayer::default()
    };

    let policy = PolicyResolutionInputs {
        defaults: ConfigLayer::default(),
        user: Some(user),
        project: vec![outer, inner],
        env,
        cli,
    }
    .resolve();

    // CLI > env > project > user > defaults
    assert_eq!(policy.profile, CompatProfile::Claude); // CLI wins
    assert_eq!(policy.semantics, WorktreeincludeSemantics::Wt039); // innermost project wins
    assert_eq!(
        policy.when_missing,
        WhenMissingWorktreeinclude::AllIgnored // env wins
    );
    assert_eq!(policy.symlink_policy, SymlinkPolicy::Follow); // CLI wins
    // builtin_exclude_set untouched -> default
    assert_eq!(policy.builtin_exclude_set, BuiltinExcludeSet::None);
}

#[test]
fn extras_accumulate_across_user_project_env_cli() {
    let user = parse_toml(
        "user",
        r#"
[exclude]
extra = ["from-user"]
"#,
    )
    .unwrap();
    let project = parse_toml(
        "project",
        r#"
[exclude]
extra = ["from-project"]
"#,
    )
    .unwrap();
    let env = layer_from_env_iter(vec![(
        "WAFT_EXTRA_EXCLUDE".to_string(),
        "from-env-1, from-env-2".to_string(),
    )])
    .unwrap();
    let cli = ConfigLayer {
        extra_excludes: vec!["from-cli".into()],
        ..ConfigLayer::default()
    };

    let policy = PolicyResolutionInputs {
        defaults: ConfigLayer::default(),
        user: Some(user),
        project: vec![project],
        env,
        cli,
    }
    .resolve();

    assert_eq!(
        policy.extra_excludes,
        vec![
            "from-user".to_string(),
            "from-project".to_string(),
            "from-env-1".to_string(),
            "from-env-2".to_string(),
            "from-cli".to_string(),
        ]
    );
}

#[test]
fn replace_extras_at_cli_drops_lower_layers() {
    let user = parse_toml(
        "user",
        r#"
[exclude]
extra = ["u"]
"#,
    )
    .unwrap();
    let env =
        layer_from_env_iter(vec![("WAFT_EXTRA_EXCLUDE".to_string(), "e".to_string())]).unwrap();
    let cli = ConfigLayer {
        extra_excludes: vec!["c".into()],
        replace_extra_excludes: Some(true),
        ..ConfigLayer::default()
    };

    let policy = PolicyResolutionInputs {
        defaults: ConfigLayer::default(),
        user: Some(user),
        project: vec![],
        env,
        cli,
    }
    .resolve();

    assert_eq!(policy.extra_excludes, vec!["c".to_string()]);
}

#[test]
fn defaults_alone_resolve_to_default_policy() {
    let policy = PolicyResolutionInputs {
        defaults: ConfigLayer::default(),
        user: None,
        project: vec![],
        env: ConfigLayer::default(),
        cli: ConfigLayer::default(),
    }
    .resolve();
    assert_eq!(policy, ResolvedPolicy::default());
}
