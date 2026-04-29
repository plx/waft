//! Unit tests for compat-profile preset expansion and explicit-override
//! precedence.
//!
//! Documented contract: explicit knob > preset > legacy default. Profiles
//! never set `extra_excludes`; that array is purely layered.

use waft::config::{
    BuiltinExcludeSet, CompatProfile, ConfigLayer, PolicyResolutionInputs, ResolvedPolicy,
    SymlinkPolicy, WhenMissingWorktreeinclude, WorktreeincludeSemantics,
};

fn resolve(layers: &[&ConfigLayer]) -> ResolvedPolicy {
    ResolvedPolicy::from_layers(layers.iter().copied())
}

// --- Default behavior with no profile set ---

#[test]
fn no_profile_anywhere_falls_back_to_legacy_defaults() {
    let policy = ResolvedPolicy::default();
    assert_eq!(policy.profile, CompatProfile::Claude);
    assert_eq!(policy.when_missing, WhenMissingWorktreeinclude::Blank);
    // Legacy default uses the Git engine to preserve pre-modes behavior;
    // distinct from the claude preset's `claude-2026-04`. The semantics
    // value flips when the default profile flips in the final PR.
    assert_eq!(policy.semantics, WorktreeincludeSemantics::Git);
    // Legacy default: pre-modes code rejected symlinks. Distinct from the
    // claude preset's `follow`, which only takes effect when claude profile
    // is explicitly chosen (until the final default flip).
    assert_eq!(policy.symlink_policy, SymlinkPolicy::Error);
    assert_eq!(policy.builtin_exclude_set, BuiltinExcludeSet::None);
    assert!(policy.extra_excludes.is_empty());
}

// --- Each preset expands as documented ---

#[test]
fn claude_preset_expansion() {
    let layer = ConfigLayer {
        profile: Some(CompatProfile::Claude),
        ..ConfigLayer::default()
    };
    let policy = resolve(&[&layer]);
    assert_eq!(policy.profile, CompatProfile::Claude);
    assert_eq!(policy.when_missing, WhenMissingWorktreeinclude::Blank);
    assert_eq!(policy.semantics, WorktreeincludeSemantics::Claude202604);
    assert_eq!(policy.symlink_policy, SymlinkPolicy::Follow);
    assert_eq!(policy.builtin_exclude_set, BuiltinExcludeSet::None);
}

#[test]
fn git_preset_expansion() {
    let layer = ConfigLayer {
        profile: Some(CompatProfile::Git),
        ..ConfigLayer::default()
    };
    let policy = resolve(&[&layer]);
    assert_eq!(policy.profile, CompatProfile::Git);
    assert_eq!(policy.when_missing, WhenMissingWorktreeinclude::Blank);
    assert_eq!(policy.semantics, WorktreeincludeSemantics::Git);
    assert_eq!(policy.symlink_policy, SymlinkPolicy::Ignore);
    assert_eq!(policy.builtin_exclude_set, BuiltinExcludeSet::None);
}

#[test]
fn wt_preset_expansion() {
    let layer = ConfigLayer {
        profile: Some(CompatProfile::Wt),
        ..ConfigLayer::default()
    };
    let policy = resolve(&[&layer]);
    assert_eq!(policy.profile, CompatProfile::Wt);
    assert_eq!(policy.when_missing, WhenMissingWorktreeinclude::AllIgnored);
    assert_eq!(policy.semantics, WorktreeincludeSemantics::Wt039);
    assert_eq!(policy.symlink_policy, SymlinkPolicy::Follow);
    assert_eq!(policy.builtin_exclude_set, BuiltinExcludeSet::ToolingV1);
}

// --- Explicit knob beats preset ---

#[test]
fn explicit_knob_in_same_layer_overrides_preset() {
    let layer = ConfigLayer {
        profile: Some(CompatProfile::Wt),
        // override one preset value while leaving others alone
        symlink_policy: Some(SymlinkPolicy::Error),
        ..ConfigLayer::default()
    };
    let policy = resolve(&[&layer]);
    assert_eq!(policy.profile, CompatProfile::Wt);
    // Wt preset says Follow, but explicit knob wins.
    assert_eq!(policy.symlink_policy, SymlinkPolicy::Error);
    // Other Wt preset values still apply.
    assert_eq!(policy.when_missing, WhenMissingWorktreeinclude::AllIgnored);
    assert_eq!(policy.builtin_exclude_set, BuiltinExcludeSet::ToolingV1);
}

#[test]
fn explicit_knob_in_lower_layer_still_beats_preset() {
    // user file sets symlink_policy=ignore; CLI selects wt profile
    let user = ConfigLayer {
        symlink_policy: Some(SymlinkPolicy::Ignore),
        ..ConfigLayer::default()
    };
    let cli = ConfigLayer {
        profile: Some(CompatProfile::Wt),
        ..ConfigLayer::default()
    };
    let policy = resolve(&[&user, &cli]);
    assert_eq!(policy.profile, CompatProfile::Wt);
    // Wt preset's symlink_policy would be Follow, but the user file's
    // explicit Ignore wins because explicit > preset, regardless of layer.
    assert_eq!(policy.symlink_policy, SymlinkPolicy::Ignore);
    // Other Wt preset values still apply.
    assert_eq!(policy.semantics, WorktreeincludeSemantics::Wt039);
}

#[test]
fn explicit_knob_in_higher_layer_overrides_preset() {
    // CLI selects git profile; env explicitly sets when_missing
    let env = ConfigLayer {
        when_missing: Some(WhenMissingWorktreeinclude::AllIgnored),
        ..ConfigLayer::default()
    };
    let cli = ConfigLayer {
        profile: Some(CompatProfile::Git),
        ..ConfigLayer::default()
    };
    let policy = resolve(&[&env, &cli]);
    assert_eq!(policy.profile, CompatProfile::Git);
    // Git preset's when_missing is Blank, but env explicitly set
    // AllIgnored. Explicit beats preset.
    assert_eq!(policy.when_missing, WhenMissingWorktreeinclude::AllIgnored);
    // Other Git preset values still apply.
    assert_eq!(policy.semantics, WorktreeincludeSemantics::Git);
    assert_eq!(policy.symlink_policy, SymlinkPolicy::Ignore);
}

// --- Profile precedence across layers ---

#[test]
fn profile_set_in_higher_layer_replaces_lower() {
    let lower = ConfigLayer {
        profile: Some(CompatProfile::Git),
        ..ConfigLayer::default()
    };
    let higher = ConfigLayer {
        profile: Some(CompatProfile::Wt),
        ..ConfigLayer::default()
    };
    let policy = resolve(&[&lower, &higher]);
    assert_eq!(policy.profile, CompatProfile::Wt);
    // Wt preset (not Git's) applies for unset knobs.
    assert_eq!(policy.when_missing, WhenMissingWorktreeinclude::AllIgnored);
    assert_eq!(policy.builtin_exclude_set, BuiltinExcludeSet::ToolingV1);
}

#[test]
fn profile_in_lower_layer_used_if_higher_does_not_set_one() {
    let lower = ConfigLayer {
        profile: Some(CompatProfile::Wt),
        ..ConfigLayer::default()
    };
    let higher = ConfigLayer {
        // Doesn't set profile; expected to inherit Wt from lower.
        when_missing: Some(WhenMissingWorktreeinclude::Blank),
        ..ConfigLayer::default()
    };
    let policy = resolve(&[&lower, &higher]);
    assert_eq!(policy.profile, CompatProfile::Wt);
    // Higher layer's explicit when_missing beats the Wt preset's AllIgnored.
    assert_eq!(policy.when_missing, WhenMissingWorktreeinclude::Blank);
}

// --- Extras + presets ---

#[test]
fn extras_unaffected_by_preset_choice() {
    let layer = ConfigLayer {
        profile: Some(CompatProfile::Wt),
        extra_excludes: vec!["a".into(), "b".into()],
        ..ConfigLayer::default()
    };
    let policy = resolve(&[&layer]);
    assert_eq!(
        policy.extra_excludes,
        vec!["a".to_string(), "b".to_string()]
    );
    // Preset still applies for the other knobs.
    assert_eq!(policy.builtin_exclude_set, BuiltinExcludeSet::ToolingV1);
}

#[test]
fn replace_extras_works_under_preset() {
    let lower = ConfigLayer {
        extra_excludes: vec!["lower".into()],
        ..ConfigLayer::default()
    };
    let higher = ConfigLayer {
        profile: Some(CompatProfile::Wt),
        extra_excludes: vec!["higher".into()],
        replace_extra_excludes: Some(true),
        ..ConfigLayer::default()
    };
    let policy = resolve(&[&lower, &higher]);
    assert_eq!(policy.extra_excludes, vec!["higher".to_string()]);
    assert_eq!(policy.builtin_exclude_set, BuiltinExcludeSet::ToolingV1);
}

// --- Resolution via PolicyResolutionInputs ---

#[test]
fn resolution_inputs_apply_layer_order_with_preset() {
    let inputs = PolicyResolutionInputs {
        defaults: ConfigLayer::default(),
        // user: pick git profile
        user: Some(ConfigLayer {
            profile: Some(CompatProfile::Git),
            ..ConfigLayer::default()
        }),
        // project: replace with wt profile
        project: vec![ConfigLayer {
            profile: Some(CompatProfile::Wt),
            ..ConfigLayer::default()
        }],
        // env: override one knob
        env: ConfigLayer {
            symlink_policy: Some(SymlinkPolicy::Error),
            ..ConfigLayer::default()
        },
        // CLI: override another knob
        cli: ConfigLayer {
            when_missing: Some(WhenMissingWorktreeinclude::Blank),
            ..ConfigLayer::default()
        },
    };
    let policy = inputs.resolve();
    assert_eq!(policy.profile, CompatProfile::Wt);
    // semantics not explicitly set; falls back to Wt preset
    assert_eq!(policy.semantics, WorktreeincludeSemantics::Wt039);
    // env explicit: Error
    assert_eq!(policy.symlink_policy, SymlinkPolicy::Error);
    // CLI explicit: Blank
    assert_eq!(policy.when_missing, WhenMissingWorktreeinclude::Blank);
    // builtin_exclude_set not explicit: Wt preset
    assert_eq!(policy.builtin_exclude_set, BuiltinExcludeSet::ToolingV1);
}
