//! Unit-style tests for config parsing and validation.
//!
//! These cover the schema's enum acceptance/rejection surface, unknown-key
//! protection, and the array/scalar merge behavior of [`ResolvedPolicy`]
//! at the layer-application level.

use waft::config::{
    BuiltinExcludeSet, CompatProfile, ConfigLayer, CopyStrategy, PolicyResolutionInputs,
    ResolvedPolicy, SymlinkPolicy, WhenMissingWorktreeinclude, WorktreeincludeSemantics,
    layer_from_env_iter, parse_toml,
};

// --- Default policy ---

#[test]
fn default_policy_matches_claude_preset() {
    let p = ResolvedPolicy::default();
    assert_eq!(p.profile, CompatProfile::Claude);
    // OOTB defaults match the matrix's claude preset.
    assert_eq!(p.when_missing, WhenMissingWorktreeinclude::Blank);
    assert_eq!(p.semantics, WorktreeincludeSemantics::Claude202604);
    assert_eq!(p.symlink_policy, SymlinkPolicy::Follow);
    assert_eq!(p.builtin_exclude_set, BuiltinExcludeSet::None);
    assert!(p.extra_excludes.is_empty());
    assert_eq!(p.copy_strategy, CopyStrategy::Auto);
}

// --- Enum parsing happy paths ---

#[test]
fn parse_each_compat_profile() {
    for (s, expected) in [
        ("claude", CompatProfile::Claude),
        ("git", CompatProfile::Git),
        ("wt", CompatProfile::Wt),
    ] {
        let toml = format!("[compat]\nprofile = \"{s}\"\n");
        let layer = parse_toml("inline", &toml).unwrap();
        assert_eq!(layer.profile, Some(expected), "profile {s}");
    }
}

#[test]
fn parse_each_when_missing() {
    for (s, expected) in [
        ("blank", WhenMissingWorktreeinclude::Blank),
        ("all-ignored", WhenMissingWorktreeinclude::AllIgnored),
    ] {
        let toml = format!("[worktreeinclude]\nwhen_missing = \"{s}\"\n");
        let layer = parse_toml("inline", &toml).unwrap();
        assert_eq!(layer.when_missing, Some(expected));
    }
}

#[test]
fn parse_each_semantics() {
    for (s, expected) in [
        ("claude-2026-04", WorktreeincludeSemantics::Claude202604),
        ("git", WorktreeincludeSemantics::Git),
        ("wt-0.39", WorktreeincludeSemantics::Wt039),
    ] {
        let toml = format!("[worktreeinclude]\nsemantics = \"{s}\"\n");
        let layer = parse_toml("inline", &toml).unwrap();
        assert_eq!(layer.semantics, Some(expected));
    }
}

#[test]
fn parse_each_symlink_policy() {
    for (s, expected) in [
        ("follow", SymlinkPolicy::Follow),
        ("ignore", SymlinkPolicy::Ignore),
        ("error", SymlinkPolicy::Error),
    ] {
        let toml = format!("[worktreeinclude]\nsymlink_policy = \"{s}\"\n");
        let layer = parse_toml("inline", &toml).unwrap();
        assert_eq!(layer.symlink_policy, Some(expected));
    }
}

#[test]
fn parse_each_builtin_set() {
    for (s, expected) in [
        ("none", BuiltinExcludeSet::None),
        ("tooling-v1", BuiltinExcludeSet::ToolingV1),
    ] {
        let toml = format!("[exclude]\nbuiltin_set = \"{s}\"\n");
        let layer = parse_toml("inline", &toml).unwrap();
        assert_eq!(layer.builtin_exclude_set, Some(expected));
    }
}

#[test]
fn parse_each_copy_strategy() {
    for (s, expected) in [
        ("auto", CopyStrategy::Auto),
        ("simple-copy", CopyStrategy::SimpleCopy),
        ("cow-copy", CopyStrategy::CowCopy),
    ] {
        let toml = format!("[copy]\nstrategy = \"{s}\"\n");
        let layer = parse_toml("inline", &toml).unwrap();
        assert_eq!(layer.copy_strategy, Some(expected));
    }
}

#[test]
fn parse_copy_strategy_aliases() {
    // Convenience aliases people are likely to type.
    for (s, expected) in [
        ("default", CopyStrategy::Auto),
        ("simple", CopyStrategy::SimpleCopy),
        ("cow", CopyStrategy::CowCopy),
        ("reflink", CopyStrategy::CowCopy),
    ] {
        let toml = format!("[copy]\nstrategy = \"{s}\"\n");
        let layer = parse_toml("inline", &toml).unwrap();
        assert_eq!(layer.copy_strategy, Some(expected), "alias {s}");
    }
}

// --- Enum parsing rejections ---

#[test]
fn invalid_compat_profile_rejected() {
    let err = parse_toml("inline", "[compat]\nprofile = \"unknown\"\n").unwrap_err();
    assert!(err.to_string().contains("unknown"));
}

#[test]
fn invalid_when_missing_rejected() {
    let err = parse_toml(
        "inline",
        "[worktreeinclude]\nwhen_missing = \"sometimes\"\n",
    )
    .unwrap_err();
    assert!(err.to_string().contains("sometimes"));
}

#[test]
fn invalid_semantics_rejected() {
    let err = parse_toml("inline", "[worktreeinclude]\nsemantics = \"made-up\"\n").unwrap_err();
    assert!(err.to_string().contains("made-up"));
}

#[test]
fn invalid_symlink_policy_rejected() {
    let err = parse_toml(
        "inline",
        "[worktreeinclude]\nsymlink_policy = \"swallow\"\n",
    )
    .unwrap_err();
    assert!(err.to_string().contains("swallow"));
}

#[test]
fn invalid_builtin_set_rejected() {
    let err = parse_toml("inline", "[exclude]\nbuiltin_set = \"v0\"\n").unwrap_err();
    assert!(err.to_string().contains("v0"));
}

#[test]
fn invalid_copy_strategy_rejected() {
    let err = parse_toml("inline", "[copy]\nstrategy = \"warp\"\n").unwrap_err();
    assert!(err.to_string().contains("warp"));
}

#[test]
fn unknown_copy_key_rejected() {
    let err = parse_toml("inline", "[copy]\nspeedup = true\n").unwrap_err();
    assert!(err.to_string().contains("speedup"));
}

// --- TOML structural protection ---

#[test]
fn unknown_top_level_key_rejected() {
    let err = parse_toml("inline", "garbage = 1\n").unwrap_err();
    assert!(err.to_string().contains("garbage"));
}

#[test]
fn unknown_section_rejected() {
    let err = parse_toml("inline", "[mystery]\nfoo = 1\n").unwrap_err();
    assert!(err.to_string().contains("mystery"));
}

#[test]
fn unknown_compat_key_rejected() {
    let err = parse_toml("inline", "[compat]\nlevel = 5\n").unwrap_err();
    assert!(err.to_string().contains("level"));
}

#[test]
fn unknown_worktreeinclude_key_rejected() {
    let err = parse_toml("inline", "[worktreeinclude]\nturbo = true\n").unwrap_err();
    assert!(err.to_string().contains("turbo"));
}

#[test]
fn unknown_exclude_key_rejected() {
    let err = parse_toml("inline", "[exclude]\nrelics = []\n").unwrap_err();
    assert!(err.to_string().contains("relics"));
}

#[test]
fn unsupported_version_rejected() {
    let err = parse_toml("inline", "version = 9\n").unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("version") && msg.contains("9"), "msg: {msg}");
}

#[test]
fn explicit_version_one_accepted() {
    let layer = parse_toml("inline", "version = 1\n").unwrap();
    assert_eq!(layer, ConfigLayer::default());
}

#[test]
fn empty_file_yields_empty_layer() {
    let layer = parse_toml("inline", "").unwrap();
    assert_eq!(layer, ConfigLayer::default());
}

#[test]
fn extra_excludes_array_parsed() {
    let layer = parse_toml(
        "inline",
        r#"
[exclude]
extra = ["a", "b/", "c/**/*.log"]
replace_extra = false
"#,
    )
    .unwrap();
    assert_eq!(
        layer.extra_excludes,
        vec!["a".to_string(), "b/".to_string(), "c/**/*.log".to_string()]
    );
    assert_eq!(layer.replace_extra_excludes, Some(false));
}

#[test]
fn extra_excludes_must_be_strings() {
    let err = parse_toml("inline", "[exclude]\nextra = [1, 2]\n").unwrap_err();
    assert!(err.to_string().to_lowercase().contains("string"));
}

// --- Layer composition ---

#[test]
fn scalar_overwrite_in_precedence_order() {
    let lower = ConfigLayer {
        profile: Some(CompatProfile::Git),
        when_missing: Some(WhenMissingWorktreeinclude::Blank),
        ..ConfigLayer::default()
    };
    let upper = ConfigLayer {
        profile: Some(CompatProfile::Wt),
        ..ConfigLayer::default()
    };
    let policy = ResolvedPolicy::from_layers([&lower, &upper]);
    assert_eq!(policy.profile, CompatProfile::Wt);
    // Lower's set value is preserved when upper does not override it.
    assert_eq!(policy.when_missing, WhenMissingWorktreeinclude::Blank);
}

#[test]
fn extras_append_across_layers() {
    let user = ConfigLayer {
        extra_excludes: vec!["a".into()],
        ..ConfigLayer::default()
    };
    let project = ConfigLayer {
        extra_excludes: vec!["b".into()],
        ..ConfigLayer::default()
    };
    let env = ConfigLayer {
        extra_excludes: vec!["c".into()],
        ..ConfigLayer::default()
    };
    let cli = ConfigLayer {
        extra_excludes: vec!["d".into()],
        ..ConfigLayer::default()
    };
    let policy = ResolvedPolicy::from_layers([&user, &project, &env, &cli]);
    assert_eq!(
        policy.extra_excludes,
        vec![
            "a".to_string(),
            "b".to_string(),
            "c".to_string(),
            "d".to_string()
        ]
    );
}

#[test]
fn replace_extra_drops_lower_layers() {
    let lower = ConfigLayer {
        extra_excludes: vec!["x".into(), "y".into()],
        ..ConfigLayer::default()
    };
    let middle = ConfigLayer {
        extra_excludes: vec!["m".into()],
        replace_extra_excludes: Some(true),
        ..ConfigLayer::default()
    };
    let upper = ConfigLayer {
        extra_excludes: vec!["u".into()],
        ..ConfigLayer::default()
    };
    let policy = ResolvedPolicy::from_layers([&lower, &middle, &upper]);
    assert_eq!(
        policy.extra_excludes,
        vec!["m".to_string(), "u".to_string()]
    );
}

#[test]
fn replace_extra_in_top_layer_clears_all() {
    let lower = ConfigLayer {
        extra_excludes: vec!["a".into(), "b".into()],
        ..ConfigLayer::default()
    };
    let upper = ConfigLayer {
        extra_excludes: vec![],
        replace_extra_excludes: Some(true),
        ..ConfigLayer::default()
    };
    let policy = ResolvedPolicy::from_layers([&lower, &upper]);
    assert!(policy.extra_excludes.is_empty());
}

// --- Env layer ---

#[test]
fn env_layer_recognizes_all_known_vars() {
    let env: Vec<(String, String)> = vec![
        ("WAFT_COMPAT_PROFILE".into(), "wt".into()),
        (
            "WAFT_WHEN_MISSING_WORKTREEINCLUDE".into(),
            "all-ignored".into(),
        ),
        ("WAFT_WORKTREEINCLUDE_SEMANTICS".into(), "git".into()),
        (
            "WAFT_WORKTREEINCLUDE_SYMLINK_POLICY".into(),
            "ignore".into(),
        ),
        ("WAFT_BUILTIN_EXCLUDE_SET".into(), "tooling-v1".into()),
        ("WAFT_EXTRA_EXCLUDE".into(), "x,y".into()),
        ("WAFT_REPLACE_EXTRA_EXCLUDES".into(), "1".into()),
        ("WAFT_COPY_STRATEGY".into(), "cow-copy".into()),
    ];
    let layer = layer_from_env_iter(env).unwrap();
    assert_eq!(layer.profile, Some(CompatProfile::Wt));
    assert_eq!(
        layer.when_missing,
        Some(WhenMissingWorktreeinclude::AllIgnored)
    );
    assert_eq!(layer.semantics, Some(WorktreeincludeSemantics::Git));
    assert_eq!(layer.symlink_policy, Some(SymlinkPolicy::Ignore));
    assert_eq!(
        layer.builtin_exclude_set,
        Some(BuiltinExcludeSet::ToolingV1)
    );
    assert_eq!(layer.extra_excludes, vec!["x".to_string(), "y".to_string()]);
    assert_eq!(layer.replace_extra_excludes, Some(true));
    assert_eq!(layer.copy_strategy, Some(CopyStrategy::CowCopy));
}

#[test]
fn env_layer_invalid_copy_strategy_rejected() {
    let env = vec![("WAFT_COPY_STRATEGY".into(), "warp".into())];
    let err = layer_from_env_iter(env).unwrap_err();
    assert!(err.to_string().contains("warp"));
}

#[test]
fn env_layer_invalid_value_is_error() {
    let env = vec![("WAFT_COMPAT_PROFILE".into(), "supreme".into())];
    let err = layer_from_env_iter(env).unwrap_err();
    assert!(err.to_string().contains("supreme"));
}

#[test]
fn env_layer_replace_extra_bool_parsing() {
    for s in ["1", "true", "TRUE", "Yes", "on"] {
        let env = vec![("WAFT_REPLACE_EXTRA_EXCLUDES".into(), s.to_string())];
        let layer = layer_from_env_iter(env).unwrap();
        assert_eq!(
            layer.replace_extra_excludes,
            Some(true),
            "input {s:?} should parse as true"
        );
    }
}

#[test]
fn empty_extra_exclude_env_yields_empty_list() {
    let env = vec![("WAFT_EXTRA_EXCLUDE".into(), "".into())];
    let layer = layer_from_env_iter(env).unwrap();
    assert!(layer.extra_excludes.is_empty());
}

// --- Preset expansion smoke tests (full coverage in config_resolution_unit) ---

#[test]
fn preset_expands_unset_knobs() {
    // No profile anywhere: default profile (`claude`) preset applies.
    let no_profile = ConfigLayer::default();
    let policy = ResolvedPolicy::from_layers([&no_profile]);
    assert_eq!(policy.profile, CompatProfile::Claude);
    assert_eq!(policy.symlink_policy, SymlinkPolicy::Follow);
    assert_eq!(policy.builtin_exclude_set, BuiltinExcludeSet::None);
    assert_eq!(policy.semantics, WorktreeincludeSemantics::Claude202604);

    // Wt profile: preset expands all unset knobs.
    let wt = ConfigLayer {
        profile: Some(CompatProfile::Wt),
        ..ConfigLayer::default()
    };
    let policy = ResolvedPolicy::from_layers([&wt]);
    assert_eq!(policy.symlink_policy, SymlinkPolicy::Follow);
    assert_eq!(policy.builtin_exclude_set, BuiltinExcludeSet::ToolingV1);
    assert_eq!(policy.when_missing, WhenMissingWorktreeinclude::AllIgnored);
    assert_eq!(policy.semantics, WorktreeincludeSemantics::Wt039);
}

#[test]
fn explicit_knob_beats_preset_regardless_of_layer() {
    let user = ConfigLayer {
        symlink_policy: Some(SymlinkPolicy::Error),
        ..ConfigLayer::default()
    };
    let cli = ConfigLayer {
        profile: Some(CompatProfile::Wt),
        ..ConfigLayer::default()
    };
    let policy = ResolvedPolicy::from_layers([&user, &cli]);
    // Wt preset says Follow but the user's explicit Error wins.
    assert_eq!(policy.symlink_policy, SymlinkPolicy::Error);
}

// --- Resolution wrapper ---

#[test]
fn copy_strategy_layer_precedence() {
    // CLI > env > project > user; latest scalar wins.
    let user = ConfigLayer {
        copy_strategy: Some(CopyStrategy::SimpleCopy),
        ..ConfigLayer::default()
    };
    let project = ConfigLayer {
        copy_strategy: Some(CopyStrategy::CowCopy),
        ..ConfigLayer::default()
    };
    let policy = ResolvedPolicy::from_layers([&user, &project]);
    assert_eq!(policy.copy_strategy, CopyStrategy::CowCopy);
}

#[test]
fn copy_strategy_unset_falls_back_to_default() {
    // Resolving with no layer setting copy_strategy yields Auto.
    let layer = ConfigLayer {
        profile: Some(CompatProfile::Git),
        ..ConfigLayer::default()
    };
    let policy = ResolvedPolicy::from_layers([&layer]);
    assert_eq!(policy.copy_strategy, CopyStrategy::Auto);
}

#[test]
fn resolve_via_inputs_applies_in_order() {
    let inputs = PolicyResolutionInputs {
        defaults: ConfigLayer::default(),
        user: Some(ConfigLayer {
            profile: Some(CompatProfile::Git),
            ..ConfigLayer::default()
        }),
        project: vec![ConfigLayer {
            profile: Some(CompatProfile::Wt),
            ..ConfigLayer::default()
        }],
        env: ConfigLayer {
            profile: Some(CompatProfile::Claude),
            ..ConfigLayer::default()
        },
        cli: ConfigLayer {
            when_missing: Some(WhenMissingWorktreeinclude::AllIgnored),
            ..ConfigLayer::default()
        },
    };
    let policy = inputs.resolve();
    // env wins for profile (CLI didn't set it).
    assert_eq!(policy.profile, CompatProfile::Claude);
    // CLI wins for when_missing.
    assert_eq!(policy.when_missing, WhenMissingWorktreeinclude::AllIgnored);
}
