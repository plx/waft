//! Layer merge: compat-profile presets and the precedence rule that turns an
//! ordered list of configuration layers into the effective policy.
//!
//! # What it decides
//!
//! [`resolve_layers`] (production `ResolvedPolicy::from_layers`) and
//! [`resolve_inputs`] (production `PolicyResolutionInputs::resolve`, which
//! also fixes the layer order defaults, user, project, environment, CLI)
//! compute the effective profile, the four profile-owned knobs
//! (`when_missing`, `semantics`, `symlink_policy`, `builtin_exclude_set`),
//! `copy_strategy`, and the merged extra exclude globs.
//! [`default_knobs`] is `ResolvedPolicy::default()` and
//! [`selects_without_rule_file`] decides whether a profile's own preset
//! selects files when no `.worktreeinclude` exists.
//!
//! # Specification
//!
//! `spec_resolve` is the closed form: the profile is the last explicit
//! one (else `claude`); each profile-owned knob is its last explicit setting
//! if that is at or after the last profile selection, else the resolved
//! profile's preset value; `copy_strategy` is its last explicit setting
//! (else `auto`). `spec_extras` is the concatenation, in layer order, of
//! the extras of the last layer with `replace_extra == Some(true)` and of
//! every later layer (all layers when none replaces). The executable code
//! is proven equal to these, and `theorem_closed_form_is_fold` proves the
//! closed form equals the documented operational rule (`apply_layer`
//! folded over the layers).
//!
//! # Theorems
//!
//! - P1 `theorem_profile_last_writer`: profile last-writer, default
//!   `claude`.
//! - P2 `theorem_knob_last_writer`: an explicit knob with no later knob or
//!   profile selection wins; a profile selection with no explicit knob from
//!   it on yields its preset. `theorem_profile_shadows_lower_layers`:
//!   below the last profile selection, nothing but `copy_strategy` can
//!   influence the result.
//! - P3 `theorem_copy_strategy_independent`: `copy_strategy` depends only
//!   on the layers' `copy_strategy` fields.
//! - P4 `theorem_extras_closed_form`,
//!   `theorem_extras_depend_only_on_replace_flags`: extras keep layer
//!   order, within-layer order and multiplicity, start at the last replacing
//!   layer, and `Some(false)` behaves exactly like `None`.
//! - P5 `theorem_default_resolution`, `theorem_neutral_layer`,
//!   `theorem_neutral_layers_resolve_to_default`: no layers give the
//!   default policy, and inserting a layer that sets nothing (such as
//!   `ConfigLayer::default()`) anywhere changes nothing.
//! - P6 `theorem_preset_table`: only `wt` selects without a rule file, and
//!   distinct profiles have distinct presets.
//! - P7 `theorem_precedence`: for the standard stack, explicit CLI values
//!   win; a CLI profile resets every knob the CLI does not set to its preset;
//!   otherwise an explicit environment knob wins; CLI replace keeps only the
//!   CLI's extras.
//! - P8 `theorem_attribution`: a resolved knob that differs from the
//!   resolved profile's preset was set explicitly to that value at or after
//!   the last profile selection.
//!
//! Totality: the executable code has no arithmetic that can overflow and
//! only in-bounds indexing, for any number of layers and extras.
//!
//! # Trust
//!
//! The production lowering of `ConfigLayer` to [`LayerView`] and the lifting
//! of [`Resolution`] back are exhaustive field-for-field copies; vstd's
//! `String::clone` specification (`res == s`) makes the extras equal, not
//! merely equally long.

#[allow(unused_imports)]
use vstd::prelude::*;

use super::vocab::{
    BuiltinExcludeSet, CompatProfile, CopyStrategy, SymlinkPolicy, WhenMissingWorktreeinclude,
    WorktreeincludeSemantics,
};

verus! {

// ---------------------------------------------------------------------------
// Presets

/// Knob values implied by a compat profile preset.
///
/// Profiles never contribute `copy_strategy` or extra excludes: those are
/// owned by the layered configuration alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Preset {
    /// Behavior when no `.worktreeinclude` file exists.
    pub when_missing: WhenMissingWorktreeinclude,
    /// Matcher semantics.
    pub semantics: WorktreeincludeSemantics,
    /// Symlink handling policy.
    pub symlink_policy: SymlinkPolicy,
    /// Built-in exclude set selector.
    pub builtin_exclude_set: BuiltinExcludeSet,
}

/// The preset table.
pub open spec fn spec_preset(profile: CompatProfile) -> Preset {
    match profile {
        CompatProfile::Claude => Preset {
            when_missing: WhenMissingWorktreeinclude::Blank,
            semantics: WorktreeincludeSemantics::Claude202604,
            symlink_policy: SymlinkPolicy::Follow,
            builtin_exclude_set: BuiltinExcludeSet::None,
        },
        CompatProfile::Git => Preset {
            when_missing: WhenMissingWorktreeinclude::Blank,
            semantics: WorktreeincludeSemantics::Git,
            symlink_policy: SymlinkPolicy::Ignore,
            builtin_exclude_set: BuiltinExcludeSet::None,
        },
        CompatProfile::Wt => Preset {
            when_missing: WhenMissingWorktreeinclude::AllIgnored,
            semantics: WorktreeincludeSemantics::Wt039,
            symlink_policy: SymlinkPolicy::Follow,
            builtin_exclude_set: BuiltinExcludeSet::ToolingV1,
        },
    }
}

impl Preset {
    /// The preset for `profile`.
    pub fn for_profile(profile: CompatProfile) -> (r: Preset)
        ensures
            r == spec_preset(profile),
    {
        match profile {
            CompatProfile::Claude => Preset {
                when_missing: WhenMissingWorktreeinclude::Blank,
                semantics: WorktreeincludeSemantics::Claude202604,
                symlink_policy: SymlinkPolicy::Follow,
                builtin_exclude_set: BuiltinExcludeSet::None,
            },
            CompatProfile::Git => Preset {
                when_missing: WhenMissingWorktreeinclude::Blank,
                semantics: WorktreeincludeSemantics::Git,
                symlink_policy: SymlinkPolicy::Ignore,
                builtin_exclude_set: BuiltinExcludeSet::None,
            },
            CompatProfile::Wt => Preset {
                when_missing: WhenMissingWorktreeinclude::AllIgnored,
                semantics: WorktreeincludeSemantics::Wt039,
                symlink_policy: SymlinkPolicy::Follow,
                builtin_exclude_set: BuiltinExcludeSet::ToolingV1,
            },
        }
    }
}

/// Default compat profile applied when no layer sets one.
pub const DEFAULT_PROFILE: CompatProfile = CompatProfile::Claude;

/// Default copy strategy applied when no layer sets one.
pub const DEFAULT_COPY_STRATEGY: CopyStrategy = CopyStrategy::Auto;

/// Whether `profile`'s own preset selects files when no `.worktreeinclude`
/// exists (only the preset is consulted, not any resolved policy).
pub fn selects_without_rule_file(profile: CompatProfile) -> (b: bool)
    ensures
        b == (spec_preset(profile).when_missing == WhenMissingWorktreeinclude::AllIgnored),
{
    let preset = Preset::for_profile(profile);
    matches!(preset.when_missing, WhenMissingWorktreeinclude::AllIgnored)
}

// ---------------------------------------------------------------------------
// Layers and results

/// The scalar knobs one configuration layer sets (`None` = not set).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KnobLayer {
    /// Compat profile preset.
    pub profile: Option<CompatProfile>,
    /// Behavior when no `.worktreeinclude` file exists.
    pub when_missing: Option<WhenMissingWorktreeinclude>,
    /// Matcher semantics.
    pub semantics: Option<WorktreeincludeSemantics>,
    /// Symlink handling policy.
    pub symlink_policy: Option<SymlinkPolicy>,
    /// Built-in exclude set selector.
    pub builtin_exclude_set: Option<BuiltinExcludeSet>,
    /// Copy strategy.
    pub copy_strategy: Option<CopyStrategy>,
    /// Whether this layer's extra excludes replace accumulated ones.
    pub replace_extra: Option<bool>,
}

/// One layer as the merge sees it: its knobs and its extra-exclude globs.
#[derive(Debug, Clone, Copy)]
pub struct LayerView<'a> {
    /// Scalar knobs.
    pub knobs: KnobLayer,
    /// Extra exclude globs, in the layer's own order.
    pub extras: &'a [String],
}

/// The resolved scalar knobs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedKnobs {
    /// Selected compat profile.
    pub profile: CompatProfile,
    /// Behavior when no `.worktreeinclude` file exists.
    pub when_missing: WhenMissingWorktreeinclude,
    /// Matcher semantics.
    pub semantics: WorktreeincludeSemantics,
    /// Symlink handling policy.
    pub symlink_policy: SymlinkPolicy,
    /// Built-in exclude set selector.
    pub builtin_exclude_set: BuiltinExcludeSet,
    /// Copy strategy.
    pub copy_strategy: CopyStrategy,
}

/// The full merge result.
pub struct Resolution {
    /// Resolved scalar knobs.
    pub knobs: ResolvedKnobs,
    /// Extra exclude globs after layer merging.
    pub extra_excludes: Vec<String>,
}

// ---------------------------------------------------------------------------
// Specification: closed forms

/// Index of the last `Some` in `s`, or -1 when every entry is `None`.
pub open spec fn last_idx<T>(s: Seq<Option<T>>) -> int
    decreases s.len(),
{
    if s.len() == 0 {
        -1
    } else if s.last() is Some {
        s.len() - 1
    } else {
        last_idx(s.drop_last())
    }
}

/// Index of the last `true` in `s`, or -1 when there is none.
pub open spec fn last_true(s: Seq<bool>) -> int
    decreases s.len(),
{
    if s.len() == 0 {
        -1
    } else if s.last() {
        s.len() - 1
    } else {
        last_true(s.drop_last())
    }
}

/// A knob's value: the last explicit setting in `col`, provided it is not
/// older than layer `from` (the last profile selection); otherwise
/// `fallback` (that profile's preset value).
pub open spec fn knob_value<T>(col: Seq<Option<T>>, from: int, fallback: T) -> T {
    let j = last_idx(col);
    if j >= 0 && j >= from {
        col[j]->0
    } else {
        fallback
    }
}

/// Column of `profile` settings.
pub open spec fn col_profile(ls: Seq<KnobLayer>) -> Seq<Option<CompatProfile>> {
    ls.map_values(|l: KnobLayer| l.profile)
}

/// Column of `when_missing` settings.
pub open spec fn col_when_missing(ls: Seq<KnobLayer>) -> Seq<Option<WhenMissingWorktreeinclude>> {
    ls.map_values(|l: KnobLayer| l.when_missing)
}

/// Column of `semantics` settings.
pub open spec fn col_semantics(ls: Seq<KnobLayer>) -> Seq<Option<WorktreeincludeSemantics>> {
    ls.map_values(|l: KnobLayer| l.semantics)
}

/// Column of `symlink_policy` settings.
pub open spec fn col_symlink_policy(ls: Seq<KnobLayer>) -> Seq<Option<SymlinkPolicy>> {
    ls.map_values(|l: KnobLayer| l.symlink_policy)
}

/// Column of `builtin_exclude_set` settings.
pub open spec fn col_builtin_exclude_set(ls: Seq<KnobLayer>) -> Seq<Option<BuiltinExcludeSet>> {
    ls.map_values(|l: KnobLayer| l.builtin_exclude_set)
}

/// Column of `copy_strategy` settings.
pub open spec fn col_copy_strategy(ls: Seq<KnobLayer>) -> Seq<Option<CopyStrategy>> {
    ls.map_values(|l: KnobLayer| l.copy_strategy)
}

/// Column of "this layer replaces accumulated extras" flags.
pub open spec fn col_replace(ls: Seq<KnobLayer>) -> Seq<bool> {
    ls.map_values(|l: KnobLayer| l.replace_extra == Some(true))
}

/// Index of the last layer that selects a profile, or -1.
pub open spec fn last_profile_idx(ls: Seq<KnobLayer>) -> int {
    last_idx(col_profile(ls))
}

/// The resolved knobs, in closed form:
///
/// - `profile` is the last layer's explicit profile, else `DEFAULT_PROFILE`;
/// - each profile-owned knob (`when_missing`, `semantics`,
///   `symlink_policy`, `builtin_exclude_set`) is its last explicit setting
///   if that setting is in the last profile-selecting layer or later, else
///   the resolved profile's preset value;
/// - `copy_strategy` is its last explicit setting, else
///   `DEFAULT_COPY_STRATEGY`; profiles never touch it.
pub open spec fn spec_resolve(ls: Seq<KnobLayer>) -> ResolvedKnobs {
    let p = last_profile_idx(ls);
    let profile = knob_value(col_profile(ls), 0, DEFAULT_PROFILE);
    let preset = spec_preset(profile);
    ResolvedKnobs {
        profile,
        when_missing: knob_value(col_when_missing(ls), p, preset.when_missing),
        semantics: knob_value(col_semantics(ls), p, preset.semantics),
        symlink_policy: knob_value(col_symlink_policy(ls), p, preset.symlink_policy),
        builtin_exclude_set: knob_value(col_builtin_exclude_set(ls), p, preset.builtin_exclude_set),
        copy_strategy: knob_value(col_copy_strategy(ls), 0, DEFAULT_COPY_STRATEGY),
    }
}

/// First layer whose extras survive: the last replacing layer, else 0.
pub open spec fn extras_start(ls: Seq<KnobLayer>) -> int {
    let j = last_true(col_replace(ls));
    if j < 0 {
        0
    } else {
        j
    }
}

/// The merged extra excludes, in closed form: every layer's extras from the
/// last replacing layer on, concatenated in layer order.
pub open spec fn spec_extras(ls: Seq<KnobLayer>, xs: Seq<Seq<String>>) -> Seq<String> {
    xs.subrange(extras_start(ls), xs.len() as int).flatten()
}

/// The knobs of each layer view.
pub open spec fn knobs_of(ls: Seq<LayerView>) -> Seq<KnobLayer> {
    ls.map_values(|l: LayerView| l.knobs)
}

/// The extras of each layer view.
pub open spec fn extras_of(ls: Seq<LayerView>) -> Seq<Seq<String>> {
    ls.map_values(|l: LayerView| l.extras@)
}

// ---------------------------------------------------------------------------
// Specification: the operational rule

/// The documented one-layer step: selecting a profile resets every
/// profile-owned knob to that preset, then the layer's explicit knobs apply.
pub open spec fn apply_layer(r: ResolvedKnobs, l: KnobLayer) -> ResolvedKnobs {
    let r1 = match l.profile {
        Some(p) => {
            let preset = spec_preset(p);
            ResolvedKnobs {
                profile: p,
                when_missing: preset.when_missing,
                semantics: preset.semantics,
                symlink_policy: preset.symlink_policy,
                builtin_exclude_set: preset.builtin_exclude_set,
                copy_strategy: r.copy_strategy,
            }
        },
        None => r,
    };
    ResolvedKnobs {
        profile: r1.profile,
        when_missing: match l.when_missing {
            Some(v) => v,
            None => r1.when_missing,
        },
        semantics: match l.semantics {
            Some(v) => v,
            None => r1.semantics,
        },
        symlink_policy: match l.symlink_policy {
            Some(v) => v,
            None => r1.symlink_policy,
        },
        builtin_exclude_set: match l.builtin_exclude_set {
            Some(v) => v,
            None => r1.builtin_exclude_set,
        },
        copy_strategy: match l.copy_strategy {
            Some(v) => v,
            None => r1.copy_strategy,
        },
    }
}

// ---------------------------------------------------------------------------
// Lemmas

/// Characterization of `last_idx`: in range, points at a `Some`, and only
/// `None` follows it.
pub proof fn lemma_last_idx<T>(s: Seq<Option<T>>)
    ensures
        -1 <= last_idx(s) < s.len(),
        last_idx(s) >= 0 ==> s[last_idx(s)] is Some,
        forall|k: int| last_idx(s) < k < s.len() ==> s[k] is None,
    decreases s.len(),
{
    if s.len() > 0 && s.last() is None {
        lemma_last_idx(s.drop_last());
        assert forall|k: int| last_idx(s) < k < s.len() implies s[k] is None by {
            if k < s.len() - 1 {
                assert(s[k] == s.drop_last()[k]);
            }
        }
    }
}

/// `last_idx` after appending one entry.
pub proof fn lemma_last_idx_push<T>(s: Seq<Option<T>>, x: Option<T>)
    ensures
        last_idx(s.push(x)) == if x is Some {
            s.len() as int
        } else {
            last_idx(s)
        },
{
    assert(s.push(x).drop_last() =~= s);
}

/// Characterization of `last_true`, like `lemma_last_idx`.
pub proof fn lemma_last_true(s: Seq<bool>)
    ensures
        -1 <= last_true(s) < s.len(),
        last_true(s) >= 0 ==> s[last_true(s)],
        forall|k: int| last_true(s) < k < s.len() ==> !s[k],
    decreases s.len(),
{
    if s.len() > 0 && !s.last() {
        lemma_last_true(s.drop_last());
        assert forall|k: int| last_true(s) < k < s.len() implies !s[k] by {
            if k < s.len() - 1 {
                assert(s[k] == s.drop_last()[k]);
            }
        }
    }
}

/// `last_true` after appending one entry.
pub proof fn lemma_last_true_push(s: Seq<bool>, x: bool)
    ensures
        last_true(s.push(x)) == if x {
            s.len() as int
        } else {
            last_true(s)
        },
{
    assert(s.push(x).drop_last() =~= s);
}

/// One knob column after appending one layer: the new entry wins if set
/// and not older than `from'`, otherwise the value is unchanged.
pub proof fn lemma_knob_value_push<T>(col: Seq<Option<T>>, x: Option<T>, from: int, fallback: T)
    requires
        from <= col.len(),
    ensures
        x is Some ==> knob_value(col.push(x), from, fallback) == x->0,
        x is None ==> knob_value(col.push(x), from, fallback) == knob_value(col, from, fallback),
{
    lemma_last_idx_push(col, x);
    lemma_last_idx(col);
    if x is None && last_idx(col) >= 0 {
        assert(col.push(x)[last_idx(col)] == col[last_idx(col)]);
    }
}

/// The closed form satisfies the operational step: resolving one more layer
/// is applying that layer to the previous resolution.
pub proof fn lemma_resolve_push(s: Seq<KnobLayer>, l: KnobLayer)
    ensures
        spec_resolve(s.push(l)) == apply_layer(spec_resolve(s), l),
{
    let t = s.push(l);
    assert(col_profile(t) =~= col_profile(s).push(l.profile));
    assert(col_when_missing(t) =~= col_when_missing(s).push(l.when_missing));
    assert(col_semantics(t) =~= col_semantics(s).push(l.semantics));
    assert(col_symlink_policy(t) =~= col_symlink_policy(s).push(l.symlink_policy));
    assert(col_builtin_exclude_set(t) =~= col_builtin_exclude_set(s).push(l.builtin_exclude_set));
    assert(col_copy_strategy(t) =~= col_copy_strategy(s).push(l.copy_strategy));
    lemma_last_idx_push(col_profile(s), l.profile);
    lemma_last_idx(col_profile(s));
    let p = last_profile_idx(s);
    let n = s.len() as int;
    lemma_knob_value_push(col_profile(s), l.profile, 0, DEFAULT_PROFILE);
    lemma_knob_value_push(col_copy_strategy(s), l.copy_strategy, 0, DEFAULT_COPY_STRATEGY);
    let r = spec_resolve(s);
    let r2 = spec_resolve(t);
    if l.profile is Some {
        // The new layer is the last profile selection: only its own explicit
        // knobs can beat its preset.
        let pre = spec_preset(l.profile->0);
        lemma_last_idx(col_when_missing(s));
        lemma_last_idx(col_semantics(s));
        lemma_last_idx(col_symlink_policy(s));
        lemma_last_idx(col_builtin_exclude_set(s));
        lemma_last_idx_push(col_when_missing(s), l.when_missing);
        lemma_last_idx_push(col_semantics(s), l.semantics);
        lemma_last_idx_push(col_symlink_policy(s), l.symlink_policy);
        lemma_last_idx_push(col_builtin_exclude_set(s), l.builtin_exclude_set);
        assert(r2.when_missing == apply_layer(r, l).when_missing);
        assert(r2.semantics == apply_layer(r, l).semantics);
        assert(r2.symlink_policy == apply_layer(r, l).symlink_policy);
        assert(r2.builtin_exclude_set == apply_layer(r, l).builtin_exclude_set);
    } else {
        lemma_knob_value_push(col_when_missing(s), l.when_missing, p, spec_preset(r.profile).when_missing);
        lemma_knob_value_push(col_semantics(s), l.semantics, p, spec_preset(r.profile).semantics);
        lemma_knob_value_push(col_symlink_policy(s), l.symlink_policy, p, spec_preset(r.profile).symlink_policy);
        lemma_knob_value_push(col_builtin_exclude_set(s), l.builtin_exclude_set, p, spec_preset(r.profile).builtin_exclude_set);
    }
}

/// The merged extras after appending one layer: a replacing layer starts
/// over with its own extras, any other layer appends its extras.
pub proof fn lemma_extras_push(s: Seq<KnobLayer>, xs: Seq<Seq<String>>, l: KnobLayer, x: Seq<String>)
    requires
        s.len() == xs.len(),
    ensures
        spec_extras(s.push(l), xs.push(x)) == if l.replace_extra == Some(true) {
            x
        } else {
            spec_extras(s, xs) + x
        },
{
    let t = s.push(l);
    let ys = xs.push(x);
    assert(col_replace(t) =~= col_replace(s).push(l.replace_extra == Some(true)));
    lemma_last_true_push(col_replace(s), l.replace_extra == Some(true));
    lemma_last_true(col_replace(s));
    if l.replace_extra == Some(true) {
        assert(ys.subrange(s.len() as int, ys.len() as int) =~= seq![x]);
        seq![x].lemma_flatten_one_element();
    } else {
        let a = extras_start(s);
        assert(ys.subrange(a, ys.len() as int) =~= xs.subrange(a, xs.len() as int).push(x));
        xs.subrange(a, xs.len() as int).lemma_flatten_push(x);
    }
}

// ---------------------------------------------------------------------------
// Executable merge

/// The resolution of no layers: the default profile's preset, the default
/// copy strategy, and no extras.
pub fn default_knobs() -> (r: ResolvedKnobs)
    ensures
        r == spec_resolve(Seq::empty()),
{
    let preset = Preset::for_profile(DEFAULT_PROFILE);
    ResolvedKnobs {
        profile: DEFAULT_PROFILE,
        when_missing: preset.when_missing,
        semantics: preset.semantics,
        symlink_policy: preset.symlink_policy,
        builtin_exclude_set: preset.builtin_exclude_set,
        copy_strategy: DEFAULT_COPY_STRATEGY,
    }
}

/// Applies one layer's knobs (the operational step).
fn apply_layer_exec(r: &mut ResolvedKnobs, l: &KnobLayer)
    ensures
        *final(r) == apply_layer(*old(r), *l),
{
    if let Some(p) = l.profile {
        let preset = Preset::for_profile(p);
        r.profile = p;
        r.when_missing = preset.when_missing;
        r.semantics = preset.semantics;
        r.symlink_policy = preset.symlink_policy;
        r.builtin_exclude_set = preset.builtin_exclude_set;
    }
    if let Some(v) = l.when_missing {
        r.when_missing = v;
    }
    if let Some(v) = l.semantics {
        r.semantics = v;
    }
    if let Some(v) = l.symlink_policy {
        r.symlink_policy = v;
    }
    if let Some(v) = l.builtin_exclude_set {
        r.builtin_exclude_set = v;
    }
    if let Some(v) = l.copy_strategy {
        r.copy_strategy = v;
    }
}

/// Resolves layers given lowest precedence first.
pub fn resolve_layers(layers: &[LayerView]) -> (r: Resolution)
    ensures
        r.knobs == spec_resolve(knobs_of(layers@)),
        r.extra_excludes@ == spec_extras(knobs_of(layers@), extras_of(layers@)),
{
    let mut knobs = default_knobs();
    let mut extras: Vec<String> = Vec::new();
    let mut i: usize = 0;
    proof {
        assert(knobs_of(layers@).take(0) =~= Seq::<KnobLayer>::empty());
        assert(extras_of(layers@).take(0) =~= Seq::<Seq<String>>::empty());
        assert(Seq::<Seq<String>>::empty().subrange(0, 0) =~= Seq::<Seq<String>>::empty());
    }
    while i < layers.len()
        invariant
            i <= layers@.len(),
            knobs == spec_resolve(knobs_of(layers@).take(i as int)),
            extras@ == spec_extras(knobs_of(layers@).take(i as int), extras_of(layers@).take(i as int)),
        decreases layers@.len() - i,
    {
        let layer = layers[i];
        let ghost before = extras@;
        apply_layer_exec(&mut knobs, &layer.knobs);
        if matches!(layer.knobs.replace_extra, Some(true)) {
            extras.clear();
        }
        let ghost cleared = extras@;
        let mut j: usize = 0;
        while j < layer.extras.len()
            invariant
                j <= layer.extras@.len(),
                extras@ == cleared + layer.extras@.take(j as int),
            decreases layer.extras@.len() - j,
        {
            extras.push(layer.extras[j].clone());
            proof {
                assert(layer.extras@.take(j + 1) =~= layer.extras@.take(j as int).push(layer.extras@[j as int]));
            }
            j += 1;
        }
        proof {
            let ks = knobs_of(layers@);
            let es = extras_of(layers@);
            assert(layer.extras@.take(layer.extras@.len() as int) =~= layer.extras@);
            assert(ks.take(i + 1) =~= ks.take(i as int).push(layer.knobs));
            assert(es.take(i + 1) =~= es.take(i as int).push(layer.extras@));
            lemma_resolve_push(ks.take(i as int), layer.knobs);
            lemma_extras_push(ks.take(i as int), es.take(i as int), layer.knobs, layer.extras@);
            if !(layer.knobs.replace_extra == Some(true)) {
                assert(cleared == before);
            } else {
                assert(cleared =~= Seq::<String>::empty());
            }
        }
        i += 1;
    }
    proof {
        assert(knobs_of(layers@).take(layers@.len() as int) =~= knobs_of(layers@));
        assert(extras_of(layers@).take(layers@.len() as int) =~= extras_of(layers@));
    }
    Resolution { knobs, extra_excludes: extras }
}


/// The layers in precedence order: defaults, the user layer if any, the
/// project layers (outermost first), the environment, then the CLI.
pub open spec fn precedence_order<'a>(
    defaults: LayerView<'a>,
    user: Option<LayerView<'a>>,
    project: Seq<LayerView<'a>>,
    env: LayerView<'a>,
    cli: LayerView<'a>,
) -> Seq<LayerView<'a>> {
    let u = match user {
        Some(u) => seq![u],
        None => Seq::empty(),
    };
    seq![defaults] + u + project + seq![env, cli]
}

/// Resolves the standard layer stack (see `precedence_order`).
pub fn resolve_inputs<'a>(
    defaults: LayerView<'a>,
    user: Option<LayerView<'a>>,
    project: &[LayerView<'a>],
    env: LayerView<'a>,
    cli: LayerView<'a>,
) -> (r: Resolution)
    ensures
        r.knobs == spec_resolve(knobs_of(precedence_order(defaults, user, project@, env, cli))),
        r.extra_excludes@ == spec_extras(
            knobs_of(precedence_order(defaults, user, project@, env, cli)),
            extras_of(precedence_order(defaults, user, project@, env, cli)),
        ),
{
    let ghost u = match user {
        Some(u) => seq![u],
        None => Seq::<LayerView<'a>>::empty(),
    };
    let mut layers: Vec<LayerView<'a>> = Vec::new();
    layers.push(defaults);
    if let Some(u) = user {
        layers.push(u);
    }
    proof {
        assert(layers@ =~= seq![defaults] + u);
    }
    let mut i: usize = 0;
    while i < project.len()
        invariant
            i <= project@.len(),
            layers@ =~= seq![defaults] + u + project@.take(i as int),
        decreases project@.len() - i,
    {
        layers.push(project[i]);
        proof {
            assert(project@.take(i + 1) =~= project@.take(i as int).push(project@[i as int]));
        }
        i += 1;
    }
    layers.push(env);
    layers.push(cli);
    proof {
        assert(project@.take(project@.len() as int) =~= project@);
        assert(layers@ =~= precedence_order(defaults, user, project@, env, cli));
    }
    resolve_layers(layers.as_slice())
}

// ---------------------------------------------------------------------------
// Theorems

/// The operational rule, folded over the layers from the default.
pub open spec fn fold_resolve(ls: Seq<KnobLayer>) -> ResolvedKnobs
    decreases ls.len(),
{
    if ls.len() == 0 {
        spec_resolve(Seq::empty())
    } else {
        apply_layer(fold_resolve(ls.drop_last()), ls.last())
    }
}

/// The closed form equals the documented operational rule ("start from the
/// default preset; selecting a profile resets every profile-owned knob to
/// that preset; explicit knobs in the same layer and later layers then
/// apply").
pub proof fn theorem_closed_form_is_fold(ls: Seq<KnobLayer>)
    ensures
        spec_resolve(ls) == fold_resolve(ls),
    decreases ls.len(),
{
    if ls.len() > 0 {
        theorem_closed_form_is_fold(ls.drop_last());
        lemma_resolve_push(ls.drop_last(), ls.last());
        assert(ls.drop_last().push(ls.last()) =~= ls);
    }
}

/// No layers resolve to the default profile's preset with the default copy
/// strategy and no extras (`ResolvedPolicy::default()`).
pub proof fn theorem_default_resolution()
    ensures
        spec_resolve(Seq::empty()) == (ResolvedKnobs {
            profile: CompatProfile::Claude,
            when_missing: WhenMissingWorktreeinclude::Blank,
            semantics: WorktreeincludeSemantics::Claude202604,
            symlink_policy: SymlinkPolicy::Follow,
            builtin_exclude_set: BuiltinExcludeSet::None,
            copy_strategy: CopyStrategy::Auto,
        }),
        spec_extras(Seq::empty(), Seq::empty()) == Seq::<String>::empty(),
{
    assert(Seq::<Seq<String>>::empty().subrange(0, 0) =~= Seq::<Seq<String>>::empty());
}

/// Preset table facts: only `wt` selects files without a rule file, and
/// distinct profiles have distinct presets (their semantics differ).
pub proof fn theorem_preset_table()
    ensures
        forall|p: CompatProfile|
            (#[trigger] spec_preset(p)).when_missing == WhenMissingWorktreeinclude::AllIgnored
                <==> p == CompatProfile::Wt,
        forall|p: CompatProfile, q: CompatProfile|
            p != q ==> #[trigger] spec_preset(p).semantics != #[trigger] spec_preset(q).semantics,
{
}

/// A `Some` followed only by `None`s is the last `Some`.
pub proof fn lemma_last_idx_unique<T>(s: Seq<Option<T>>, j: int)
    requires
        0 <= j < s.len(),
        s[j] is Some,
        forall|k: int| j < k < s.len() ==> s[k] is None,
    ensures
        last_idx(s) == j,
{
    lemma_last_idx(s);
}

/// Without any `Some`, `last_idx` is -1.
pub proof fn lemma_last_idx_none<T>(s: Seq<Option<T>>)
    requires
        forall|k: int| 0 <= k < s.len() ==> s[k] is None,
    ensures
        last_idx(s) == -1,
{
    lemma_last_idx(s);
}

/// The last explicit setting wins when it is not older than `from`.
pub proof fn lemma_knob_last_writer<T>(col: Seq<Option<T>>, from: int, fallback: T, j: int)
    requires
        0 <= j < col.len(),
        from <= j,
        col[j] is Some,
        forall|k: int| j < k < col.len() ==> col[k] is None,
    ensures
        knob_value(col, from, fallback) == col[j]->0,
{
    lemma_last_idx_unique(col, j);
}

/// With no explicit setting at or after `from`, the fallback applies.
pub proof fn lemma_knob_fallback<T>(col: Seq<Option<T>>, from: int, fallback: T)
    requires
        forall|k: int| 0 <= k < col.len() && from <= k ==> col[k] is None,
    ensures
        knob_value(col, from, fallback) == fallback,
{
    lemma_last_idx(col);
}

/// Two columns that agree from `from` on give the same knob value.
pub proof fn lemma_knob_suffix_agree<T>(x: Seq<Option<T>>, y: Seq<Option<T>>, from: int, fallback: T)
    requires
        x.len() == y.len(),
        0 <= from,
        forall|k: int| from <= k < x.len() ==> x[k] == y[k],
    ensures
        knob_value(x, from, fallback) == knob_value(y, from, fallback),
{
    lemma_last_idx(x);
    lemma_last_idx(y);
    if last_idx(x) >= from {
        lemma_last_idx_unique(y, last_idx(x));
    } else if last_idx(y) >= from {
        lemma_last_idx_unique(x, last_idx(y));
    }
}

/// P1: the resolved profile is the one selected by the last layer that
/// selects any, or `DEFAULT_PROFILE` when no layer does.
pub proof fn theorem_profile_last_writer(ls: Seq<KnobLayer>)
    ensures
        (last_profile_idx(ls) == -1) == (forall|k: int|
            0 <= k < ls.len() ==> (#[trigger] ls[k]).profile is None),
        last_profile_idx(ls) == -1 ==> spec_resolve(ls).profile == DEFAULT_PROFILE,
        last_profile_idx(ls) >= 0 ==> {
            &&& last_profile_idx(ls) < ls.len()
            &&& ls[last_profile_idx(ls)].profile == Some(spec_resolve(ls).profile)
            &&& forall|k: int| last_profile_idx(ls) < k < ls.len() ==> (#[trigger] ls[k]).profile is None
        },
{
    let c = col_profile(ls);
    lemma_last_idx(c);
    assert forall|k: int| 0 <= k < ls.len() implies c[k] == (#[trigger] ls[k]).profile by {}
    if last_profile_idx(ls) == -1 {
        assert forall|k: int| 0 <= k < ls.len() implies (#[trigger] ls[k]).profile is None by {
            assert(c[k] == ls[k].profile);
        }
    } else {
        let p = last_profile_idx(ls);
        assert(c[p] == ls[p].profile);
        assert(!(forall|k: int| 0 <= k < ls.len() ==> (#[trigger] ls[k]).profile is None));
    }
}

/// P2 (shadowing): once the last profile selection is at layer `p`, nothing
/// in the layers below `p` except `copy_strategy` can influence the resolved
/// knobs. In particular an explicit profile-owned knob below the last
/// profile selection never affects the result.
pub proof fn theorem_profile_shadows_lower_layers(a: Seq<KnobLayer>, b: Seq<KnobLayer>)
    requires
        a.len() == b.len(),
        last_profile_idx(a) >= 0,
        forall|k: int| last_profile_idx(a) <= k < a.len() ==> a[k] == b[k],
        forall|k: int|
            0 <= k < last_profile_idx(a) ==> (#[trigger] a[k]).copy_strategy == b[k].copy_strategy,
    ensures
        spec_resolve(a) == spec_resolve(b),
{
    let p = last_profile_idx(a);
    lemma_last_idx(col_profile(a));
    assert(col_profile(b)[p] == col_profile(a)[p]);
    assert forall|k: int| p < k < b.len() implies col_profile(b)[k] is None by {
        assert(col_profile(b)[k] == col_profile(a)[k]);
    }
    lemma_last_idx_unique(col_profile(b), p);
    assert(spec_resolve(a).profile == spec_resolve(b).profile);
    let pre = spec_preset(spec_resolve(a).profile);
    lemma_knob_suffix_agree(col_when_missing(a), col_when_missing(b), p, pre.when_missing);
    lemma_knob_suffix_agree(col_semantics(a), col_semantics(b), p, pre.semantics);
    lemma_knob_suffix_agree(col_symlink_policy(a), col_symlink_policy(b), p, pre.symlink_policy);
    lemma_knob_suffix_agree(
        col_builtin_exclude_set(a),
        col_builtin_exclude_set(b),
        p,
        pre.builtin_exclude_set,
    );
    assert(col_copy_strategy(a) =~= col_copy_strategy(b)) by {
        assert forall|k: int| 0 <= k < a.len() implies col_copy_strategy(a)[k]
            == col_copy_strategy(b)[k] by {
            if k >= p {
                assert(a[k] == b[k]);
            } else {
                assert(a[k].copy_strategy == b[k].copy_strategy);
            }
        }
    }
}

/// P2 (last writer): a profile-owned knob set explicitly at layer `j`, with
/// no later layer setting that knob or selecting a profile, is the resolved
/// value; a profile selected at layer `j`, with no later profile selection
/// and no explicit knob from `j` on, determines the knob through its preset.
pub proof fn theorem_knob_last_writer(ls: Seq<KnobLayer>, j: int)
    requires
        0 <= j < ls.len(),
        forall|k: int| j < k < ls.len() ==> (#[trigger] ls[k]).profile is None,
    ensures
        ls[j].when_missing is Some && (forall|k: int|
            j < k < ls.len() ==> (#[trigger] ls[k]).when_missing is None)
            ==> spec_resolve(ls).when_missing == ls[j].when_missing->0,
        ls[j].semantics is Some && (forall|k: int|
            j < k < ls.len() ==> (#[trigger] ls[k]).semantics is None)
            ==> spec_resolve(ls).semantics == ls[j].semantics->0,
        ls[j].symlink_policy is Some && (forall|k: int|
            j < k < ls.len() ==> (#[trigger] ls[k]).symlink_policy is None)
            ==> spec_resolve(ls).symlink_policy == ls[j].symlink_policy->0,
        ls[j].builtin_exclude_set is Some && (forall|k: int|
            j < k < ls.len() ==> (#[trigger] ls[k]).builtin_exclude_set is None)
            ==> spec_resolve(ls).builtin_exclude_set == ls[j].builtin_exclude_set->0,
        ls[j].profile is Some ==> spec_resolve(ls).profile == ls[j].profile->0,
        ls[j].profile is Some && (forall|k: int|
            j <= k < ls.len() ==> (#[trigger] ls[k]).when_missing is None)
            ==> spec_resolve(ls).when_missing == spec_preset(ls[j].profile->0).when_missing,
        ls[j].profile is Some && (forall|k: int|
            j <= k < ls.len() ==> (#[trigger] ls[k]).semantics is None)
            ==> spec_resolve(ls).semantics == spec_preset(ls[j].profile->0).semantics,
        ls[j].profile is Some && (forall|k: int|
            j <= k < ls.len() ==> (#[trigger] ls[k]).symlink_policy is None)
            ==> spec_resolve(ls).symlink_policy == spec_preset(ls[j].profile->0).symlink_policy,
        ls[j].profile is Some && (forall|k: int|
            j <= k < ls.len() ==> (#[trigger] ls[k]).builtin_exclude_set is None)
            ==> spec_resolve(ls).builtin_exclude_set == spec_preset(
                ls[j].profile->0,
            ).builtin_exclude_set,
{
    let p = last_profile_idx(ls);
    lemma_last_idx(col_profile(ls));
    // The last profile selection is not after `j`.
    if p > j {
        assert(col_profile(ls)[p] == ls[p].profile);
    }
    assert(p <= j);
    let r = spec_resolve(ls);
    let pre = spec_preset(r.profile);
    if ls[j].profile is Some {
        assert forall|k: int| j < k < ls.len() implies col_profile(ls)[k] is None by {
            assert(col_profile(ls)[k] == ls[k].profile);
        }
        lemma_last_idx_unique(col_profile(ls), j);
    }
    if ls[j].when_missing is Some && (forall|k: int| j < k < ls.len() ==> (#[trigger] ls[k]).when_missing is None) {
        assert forall|k: int| j < k < ls.len() implies col_when_missing(ls)[k] is None by {
            assert(col_when_missing(ls)[k] == ls[k].when_missing);
        }
        lemma_knob_last_writer(col_when_missing(ls), p, pre.when_missing, j);
    }
    if ls[j].semantics is Some && (forall|k: int| j < k < ls.len() ==> (#[trigger] ls[k]).semantics is None) {
        assert forall|k: int| j < k < ls.len() implies col_semantics(ls)[k] is None by {
            assert(col_semantics(ls)[k] == ls[k].semantics);
        }
        lemma_knob_last_writer(col_semantics(ls), p, pre.semantics, j);
    }
    if ls[j].symlink_policy is Some && (forall|k: int| j < k < ls.len() ==> (#[trigger] ls[k]).symlink_policy is None) {
        assert forall|k: int| j < k < ls.len() implies col_symlink_policy(ls)[k] is None by {
            assert(col_symlink_policy(ls)[k] == ls[k].symlink_policy);
        }
        lemma_knob_last_writer(col_symlink_policy(ls), p, pre.symlink_policy, j);
    }
    if ls[j].builtin_exclude_set is Some && (forall|k: int| j < k < ls.len() ==> (#[trigger] ls[k]).builtin_exclude_set is None) {
        assert forall|k: int| j < k < ls.len() implies col_builtin_exclude_set(ls)[k] is None by {
            assert(col_builtin_exclude_set(ls)[k] == ls[k].builtin_exclude_set);
        }
        lemma_knob_last_writer(col_builtin_exclude_set(ls), p, pre.builtin_exclude_set, j);
    }
    if ls[j].profile is Some {
        if forall|k: int| j <= k < ls.len() ==> (#[trigger] ls[k]).when_missing is None {
            assert forall|k: int| 0 <= k < ls.len() && p <= k implies col_when_missing(ls)[k] is None by {
                assert(col_when_missing(ls)[k] == ls[k].when_missing);
            }
            lemma_knob_fallback(col_when_missing(ls), p, pre.when_missing);
        }
        if forall|k: int| j <= k < ls.len() ==> (#[trigger] ls[k]).semantics is None {
            assert forall|k: int| 0 <= k < ls.len() && p <= k implies col_semantics(ls)[k] is None by {
                assert(col_semantics(ls)[k] == ls[k].semantics);
            }
            lemma_knob_fallback(col_semantics(ls), p, pre.semantics);
        }
        if forall|k: int| j <= k < ls.len() ==> (#[trigger] ls[k]).symlink_policy is None {
            assert forall|k: int| 0 <= k < ls.len() && p <= k implies col_symlink_policy(ls)[k] is None by {
                assert(col_symlink_policy(ls)[k] == ls[k].symlink_policy);
            }
            lemma_knob_fallback(col_symlink_policy(ls), p, pre.symlink_policy);
        }
        if forall|k: int| j <= k < ls.len() ==> (#[trigger] ls[k]).builtin_exclude_set is None {
            assert forall|k: int| 0 <= k < ls.len() && p <= k implies col_builtin_exclude_set(ls)[k] is None by {
                assert(col_builtin_exclude_set(ls)[k] == ls[k].builtin_exclude_set);
            }
            lemma_knob_fallback(col_builtin_exclude_set(ls), p, pre.builtin_exclude_set);
        }
    }
}

/// P3: `copy_strategy` is the last explicit setting, or
/// `DEFAULT_COPY_STRATEGY`; it depends on nothing but the layers'
/// `copy_strategy` fields (profiles and other knobs never change it).
pub proof fn theorem_copy_strategy_independent(a: Seq<KnobLayer>, b: Seq<KnobLayer>)
    requires
        a.len() == b.len(),
        forall|k: int| 0 <= k < a.len() ==> (#[trigger] a[k]).copy_strategy == b[k].copy_strategy,
    ensures
        spec_resolve(a).copy_strategy == spec_resolve(b).copy_strategy,
        (forall|k: int| 0 <= k < a.len() ==> (#[trigger] a[k]).copy_strategy is None)
            ==> spec_resolve(a).copy_strategy == DEFAULT_COPY_STRATEGY,
{
    assert(col_copy_strategy(a) =~= col_copy_strategy(b)) by {
        assert forall|k: int| 0 <= k < a.len() implies col_copy_strategy(a)[k]
            == col_copy_strategy(b)[k] by {
            assert(a[k].copy_strategy == b[k].copy_strategy);
        }
    }
    if forall|k: int| 0 <= k < a.len() ==> (#[trigger] a[k]).copy_strategy is None {
        assert forall|k: int| 0 <= k < a.len() && 0 <= k implies col_copy_strategy(a)[k] is None by {
            assert(col_copy_strategy(a)[k] == a[k].copy_strategy);
        }
        lemma_knob_fallback(col_copy_strategy(a), 0, DEFAULT_COPY_STRATEGY);
    }
}

/// P4: the merged extras depend only on the per-layer extras and on which
/// layers replace (`replace_extra == Some(true)`); `Some(false)` behaves
/// exactly like `None`, and profiles and knobs never affect extras.
pub proof fn theorem_extras_depend_only_on_replace_flags(
    a: Seq<KnobLayer>,
    b: Seq<KnobLayer>,
    xs: Seq<Seq<String>>,
)
    requires
        a.len() == b.len(),
        forall|k: int|
            0 <= k < a.len() ==> ((#[trigger] a[k]).replace_extra == Some(true)) == (b[k].replace_extra
                == Some(true)),
    ensures
        spec_extras(a, xs) == spec_extras(b, xs),
{
    assert(col_replace(a) =~= col_replace(b)) by {
        assert forall|k: int| 0 <= k < a.len() implies col_replace(a)[k] == col_replace(b)[k] by {
            assert(a[k].replace_extra == Some(true) <==> b[k].replace_extra == Some(true));
        }
    }
}

/// P4: without a replacing layer the extras are every layer's extras in
/// layer order; with one, they are the extras of the last replacing layer
/// and of every later layer, in order.
pub proof fn theorem_extras_closed_form(ls: Seq<KnobLayer>, xs: Seq<Seq<String>>, j: int)
    requires
        ls.len() == xs.len(),
    ensures
        (forall|k: int| 0 <= k < ls.len() ==> (#[trigger] ls[k]).replace_extra != Some(true))
            ==> spec_extras(ls, xs) == xs.flatten(),
        0 <= j < ls.len() && ls[j].replace_extra == Some(true) && (forall|k: int|
            j < k < ls.len() ==> (#[trigger] ls[k]).replace_extra != Some(true))
            ==> spec_extras(ls, xs) == xs.subrange(j, xs.len() as int).flatten(),
{
    let c = col_replace(ls);
    lemma_last_true(c);
    if forall|k: int| 0 <= k < ls.len() ==> (#[trigger] ls[k]).replace_extra != Some(true) {
        if last_true(c) >= 0 {
            assert(c[last_true(c)] == (ls[last_true(c)].replace_extra == Some(true)));
        }
        assert(xs.subrange(0, xs.len() as int) =~= xs);
    }
    if 0 <= j < ls.len() && ls[j].replace_extra == Some(true) && (forall|k: int|
        j < k < ls.len() ==> (#[trigger] ls[k]).replace_extra != Some(true)) {
        assert(c[j] == (ls[j].replace_extra == Some(true)));
        if last_true(c) > j {
            assert(c[last_true(c)] == (ls[last_true(c)].replace_extra == Some(true)));
        }
        if last_true(c) < j {
            assert(c[j]);
        }
    }
}

/// P8 (attribution soundness): a resolved profile-owned knob that differs
/// from the resolved profile's preset was set explicitly, to exactly that
/// value, by a layer at or after the last profile selection. So when the
/// resolved `when_missing` is `Blank` although the profile's preset selects
/// without a rule file, some such layer explicitly set `when_missing =
/// blank`: blaming the knob, not the profile, is truthful.
pub proof fn theorem_attribution(ls: Seq<KnobLayer>)
    ensures
        ({
            let r = spec_resolve(ls);
            r.when_missing != spec_preset(r.profile).when_missing ==> exists|j: int|
                0 <= j < ls.len() && last_profile_idx(ls) <= j && (#[trigger] ls[j]).when_missing
                    == Some(r.when_missing)
        }),
        ({
            let r = spec_resolve(ls);
            r.semantics != spec_preset(r.profile).semantics ==> exists|j: int|
                0 <= j < ls.len() && last_profile_idx(ls) <= j && (#[trigger] ls[j]).semantics
                    == Some(r.semantics)
        }),
        ({
            let r = spec_resolve(ls);
            r.symlink_policy != spec_preset(r.profile).symlink_policy ==> exists|j: int|
                0 <= j < ls.len() && last_profile_idx(ls) <= j && (#[trigger] ls[j]).symlink_policy
                    == Some(r.symlink_policy)
        }),
        ({
            let r = spec_resolve(ls);
            r.builtin_exclude_set != spec_preset(r.profile).builtin_exclude_set ==> exists|j: int|
                0 <= j < ls.len() && last_profile_idx(ls) <= j && (#[trigger] ls[j]).builtin_exclude_set
                    == Some(r.builtin_exclude_set)
        }),
        ({
            let r = spec_resolve(ls);
            r.when_missing == WhenMissingWorktreeinclude::Blank && spec_preset(r.profile).when_missing
                == WhenMissingWorktreeinclude::AllIgnored ==> exists|j: int|
                0 <= j < ls.len() && last_profile_idx(ls) <= j && (#[trigger] ls[j]).when_missing
                    == Some(WhenMissingWorktreeinclude::Blank)
        }),
{
    let r = spec_resolve(ls);
    let p = last_profile_idx(ls);
    lemma_last_idx(col_when_missing(ls));
    lemma_last_idx(col_semantics(ls));
    lemma_last_idx(col_symlink_policy(ls));
    lemma_last_idx(col_builtin_exclude_set(ls));
    if r.when_missing != spec_preset(r.profile).when_missing {
        let j = last_idx(col_when_missing(ls));
        assert(col_when_missing(ls)[j] == ls[j].when_missing);
    }
    if r.semantics != spec_preset(r.profile).semantics {
        let j = last_idx(col_semantics(ls));
        assert(col_semantics(ls)[j] == ls[j].semantics);
    }
    if r.symlink_policy != spec_preset(r.profile).symlink_policy {
        let j = last_idx(col_symlink_policy(ls));
        assert(col_symlink_policy(ls)[j] == ls[j].symlink_policy);
    }
    if r.builtin_exclude_set != spec_preset(r.profile).builtin_exclude_set {
        let j = last_idx(col_builtin_exclude_set(ls));
        assert(col_builtin_exclude_set(ls)[j] == ls[j].builtin_exclude_set);
    }
}


/// P7: precedence of the standard layer stack. With `r` the resolution of
/// `precedence_order(defaults, user, project, env, cli)`:
///
/// - a knob, profile or copy strategy the CLI sets explicitly is the result;
/// - a CLI profile with no explicit CLI knob gives that profile's preset,
///   whatever the environment, project or user layers say;
/// - when the CLI sets neither the knob nor a profile, an explicit
///   environment knob is the result;
/// - CLI `replace_extra = true` makes the CLI's extras the only extras.
pub proof fn theorem_precedence<'a>(
    defaults: LayerView<'a>,
    user: Option<LayerView<'a>>,
    project: Seq<LayerView<'a>>,
    env: LayerView<'a>,
    cli: LayerView<'a>,
)
    ensures
        ({
            let order = precedence_order(defaults, user, project, env, cli);
            let r = spec_resolve(knobs_of(order));
            let c = cli.knobs;
            let e = env.knobs;
            &&& c.profile is Some ==> r.profile == c.profile->0
            &&& c.when_missing is Some ==> r.when_missing == c.when_missing->0
            &&& c.semantics is Some ==> r.semantics == c.semantics->0
            &&& c.symlink_policy is Some ==> r.symlink_policy == c.symlink_policy->0
            &&& c.builtin_exclude_set is Some ==> r.builtin_exclude_set == c.builtin_exclude_set->0
            &&& c.copy_strategy is Some ==> r.copy_strategy == c.copy_strategy->0
            &&& c.profile is Some && c.when_missing is None ==> r.when_missing == spec_preset(
                c.profile->0,
            ).when_missing
            &&& c.profile is Some && c.semantics is None ==> r.semantics == spec_preset(
                c.profile->0,
            ).semantics
            &&& c.profile is Some && c.symlink_policy is None ==> r.symlink_policy == spec_preset(
                c.profile->0,
            ).symlink_policy
            &&& c.profile is Some && c.builtin_exclude_set is None ==> r.builtin_exclude_set
                == spec_preset(c.profile->0).builtin_exclude_set
            &&& c.profile is None && c.when_missing is None && e.when_missing is Some
                ==> r.when_missing == e.when_missing->0
            &&& c.profile is None && c.semantics is None && e.semantics is Some ==> r.semantics
                == e.semantics->0
            &&& c.profile is None && c.symlink_policy is None && e.symlink_policy is Some
                ==> r.symlink_policy == e.symlink_policy->0
            &&& c.profile is None && c.builtin_exclude_set is None && e.builtin_exclude_set is Some
                ==> r.builtin_exclude_set == e.builtin_exclude_set->0
            &&& c.replace_extra == Some(true) ==> spec_extras(knobs_of(order), extras_of(order))
                == cli.extras@
        }),
{
    let order = precedence_order(defaults, user, project, env, cli);
    let ls = knobs_of(order);
    let xs = extras_of(order);
    let n = order.len() as int;
    assert(order[n - 1] == cli);
    assert(order[n - 2] == env);
    assert(ls[n - 1] == cli.knobs);
    assert(ls[n - 2] == env.knobs);
    assert(xs[n - 1] == cli.extras@);
    theorem_knob_last_writer(ls, n - 1);
    if cli.knobs.profile is None {
        theorem_knob_last_writer(ls, n - 2);
    }
    if cli.knobs.copy_strategy is Some {
        assert(col_copy_strategy(ls)[n - 1] == cli.knobs.copy_strategy);
        lemma_knob_last_writer(col_copy_strategy(ls), 0, DEFAULT_COPY_STRATEGY, n - 1);
    }
    if cli.knobs.replace_extra == Some(true) {
        theorem_extras_closed_form(ls, xs, n - 1);
        assert(xs.subrange(n - 1, n) =~= seq![cli.extras@]);
        seq![cli.extras@].lemma_flatten_one_element();
    }
}

// ---------------------------------------------------------------------------
// Neutral layers

/// A layer that sets nothing: no knob, no replacement, and no extras.
/// `ConfigLayer::default()` is such a layer.
pub open spec fn is_neutral(l: LayerView) -> bool {
    &&& l.knobs.profile is None
    &&& l.knobs.when_missing is None
    &&& l.knobs.semantics is None
    &&& l.knobs.symlink_policy is None
    &&& l.knobs.builtin_exclude_set is None
    &&& l.knobs.copy_strategy is None
    &&& l.knobs.replace_extra != Some(true)
    &&& l.extras@.len() == 0
}

/// Position of index `k` of `x + y` within `x + [inserted] + y`, where
/// `i == x.len()` (-1, meaning "none", stays -1).
pub open spec fn shifted(k: int, i: int) -> int {
    if k >= i {
        k + 1
    } else {
        k
    }
}

proof fn lemma_last_idx_insert_none<T>(x: Seq<Option<T>>, y: Seq<Option<T>>)
    ensures
        last_idx(x + seq![None] + y) == shifted(last_idx(x + y), x.len() as int),
        last_idx(x + y) >= 0 ==> (x + seq![None] + y)[shifted(last_idx(x + y), x.len() as int)]
            == (x + y)[last_idx(x + y)],
{
    let s = x + y;
    let t = x + seq![None] + y;
    let i = x.len() as int;
    lemma_last_idx(s);
    let j = last_idx(s);
    assert forall|k: int| 0 <= k < t.len() implies #[trigger] t[k] == (if k < i {
        s[k]
    } else if k == i {
        None
    } else {
        s[k - 1]
    }) by {}
    if j >= 0 {
        let j2 = shifted(j, i);
        assert(t[j2] == s[j]);
        assert forall|k: int| j2 < k < t.len() implies t[k] is None by {
            if k < i {
                assert(t[k] == s[k]);
            } else if k > i {
                assert(t[k] == s[k - 1]);
            }
        }
        lemma_last_idx_unique(t, j2);
    } else {
        assert forall|k: int| 0 <= k < t.len() implies t[k] is None by {
            if k < i {
                assert(t[k] == s[k]);
            } else if k > i {
                assert(t[k] == s[k - 1]);
            }
        }
        lemma_last_idx_none(t);
    }
}

proof fn lemma_last_true_insert_false(x: Seq<bool>, y: Seq<bool>)
    ensures
        last_true(x + seq![false] + y) == shifted(last_true(x + y), x.len() as int),
{
    let s = x + y;
    let t = x + seq![false] + y;
    let i = x.len() as int;
    lemma_last_true(s);
    lemma_last_true(t);
    let j = last_true(s);
    let j2 = last_true(t);
    assert forall|k: int| 0 <= k < t.len() implies #[trigger] t[k] == (if k < i {
        s[k]
    } else if k == i {
        false
    } else {
        s[k - 1]
    }) by {}
    // `j2` is a `true` of `t`, so it is not `i`, and maps back to a `true`
    // of `s`; `j`, shifted, is a `true` of `t`.
    if j2 >= 0 {
        assert(j2 != i);
        if j2 < i {
            assert(s[j2]);
        } else {
            assert(s[j2 - 1]);
        }
    }
    if j >= 0 {
        assert(t[shifted(j, i)]);
    }
}

proof fn lemma_knob_value_insert_none<T>(
    x: Seq<Option<T>>,
    y: Seq<Option<T>>,
    from: int,
    from2: int,
    fallback: T,
)
    requires
        last_idx(x + y) >= 0 ==> ((last_idx(x + y) >= from) == (shifted(
            last_idx(x + y),
            x.len() as int,
        ) >= from2)),
    ensures
        knob_value(x + seq![None] + y, from2, fallback) == knob_value(x + y, from, fallback),
{
    lemma_last_idx_insert_none(x, y);
    lemma_last_idx(x + y);
}

proof fn lemma_flatten_insert_empty(x: Seq<Seq<String>>, y: Seq<Seq<String>>)
    ensures
        (x + seq![Seq::<String>::empty()] + y).flatten() == (x + y).flatten(),
{
    vstd::seq_lib::lemma_flatten_concat(x, seq![Seq::<String>::empty()]);
    vstd::seq_lib::lemma_flatten_concat(x + seq![Seq::<String>::empty()], y);
    vstd::seq_lib::lemma_flatten_concat(x, y);
    seq![Seq::<String>::empty()].lemma_flatten_one_element();
    assert(x.flatten() + Seq::<String>::empty() =~= x.flatten());
}

/// P5: inserting a neutral layer anywhere changes neither the resolved knobs
/// nor the merged extras.
pub proof fn theorem_neutral_layer(a: Seq<LayerView>, b: Seq<LayerView>, n: LayerView)
    requires
        is_neutral(n),
    ensures
        spec_resolve(knobs_of(a + seq![n] + b)) == spec_resolve(knobs_of(a + b)),
        spec_extras(knobs_of(a + seq![n] + b), extras_of(a + seq![n] + b)) == spec_extras(
            knobs_of(a + b),
            extras_of(a + b),
        ),
{
    let i = a.len() as int;
    let s = knobs_of(a + b);
    let t = knobs_of(a + seq![n] + b);
    let ka = knobs_of(a);
    let kb = knobs_of(b);
    assert(s =~= ka + kb);
    assert(t =~= ka + seq![n.knobs] + kb);
    assert(col_profile(s) =~= col_profile(ka) + col_profile(kb));
    assert(col_profile(t) =~= col_profile(ka) + seq![None] + col_profile(kb));
    assert(col_when_missing(s) =~= col_when_missing(ka) + col_when_missing(kb));
    assert(col_when_missing(t) =~= col_when_missing(ka) + seq![None] + col_when_missing(kb));
    assert(col_semantics(s) =~= col_semantics(ka) + col_semantics(kb));
    assert(col_semantics(t) =~= col_semantics(ka) + seq![None] + col_semantics(kb));
    assert(col_symlink_policy(s) =~= col_symlink_policy(ka) + col_symlink_policy(kb));
    assert(col_symlink_policy(t) =~= col_symlink_policy(ka) + seq![None] + col_symlink_policy(kb));
    assert(col_builtin_exclude_set(s) =~= col_builtin_exclude_set(ka) + col_builtin_exclude_set(kb));
    assert(col_builtin_exclude_set(t) =~= col_builtin_exclude_set(ka) + seq![None]
        + col_builtin_exclude_set(kb));
    assert(col_copy_strategy(s) =~= col_copy_strategy(ka) + col_copy_strategy(kb));
    assert(col_copy_strategy(t) =~= col_copy_strategy(ka) + seq![None] + col_copy_strategy(kb));
    assert(col_replace(s) =~= col_replace(ka) + col_replace(kb));
    assert(col_replace(t) =~= col_replace(ka) + seq![false] + col_replace(kb));
    lemma_last_idx_insert_none(col_profile(ka), col_profile(kb));
    lemma_knob_value_insert_none(col_profile(ka), col_profile(kb), 0, 0, DEFAULT_PROFILE);
    let p = last_profile_idx(s);
    let p2 = last_profile_idx(t);
    assert(p2 == shifted(p, i));
    let pre = spec_preset(spec_resolve(s).profile);
    lemma_last_idx(col_when_missing(s));
    lemma_knob_value_insert_none(col_when_missing(ka), col_when_missing(kb), p, p2, pre.when_missing);
    lemma_knob_value_insert_none(col_semantics(ka), col_semantics(kb), p, p2, pre.semantics);
    lemma_knob_value_insert_none(
        col_symlink_policy(ka),
        col_symlink_policy(kb),
        p,
        p2,
        pre.symlink_policy,
    );
    lemma_knob_value_insert_none(
        col_builtin_exclude_set(ka),
        col_builtin_exclude_set(kb),
        p,
        p2,
        pre.builtin_exclude_set,
    );
    lemma_knob_value_insert_none(
        col_copy_strategy(ka),
        col_copy_strategy(kb),
        0,
        0,
        DEFAULT_COPY_STRATEGY,
    );
    // Extras.
    let xs = extras_of(a + b);
    let ys = extras_of(a + seq![n] + b);
    let xa = extras_of(a);
    let xb = extras_of(b);
    let e = Seq::<String>::empty();
    assert(n.extras@ =~= e);
    assert(xs =~= xa + xb);
    assert(ys =~= xa + seq![e] + xb);
    lemma_last_true_insert_false(col_replace(ka), col_replace(kb));
    lemma_last_true(col_replace(s));
    let j = last_true(col_replace(s));
    let st = extras_start(s);
    if j >= i {
        assert(ys.subrange(j + 1, ys.len() as int) =~= xs.subrange(j, xs.len() as int));
    } else {
        assert(st <= i);
        assert(ys.subrange(st, ys.len() as int) =~= xa.subrange(st, i) + seq![e] + xb);
        assert(xs.subrange(st, xs.len() as int) =~= xa.subrange(st, i) + xb);
        lemma_flatten_insert_empty(xa.subrange(st, i), xb);
    }
}

/// P5: layers that set nothing resolve to the defaults
/// (`resolve()` with only default layers equals `ResolvedPolicy::default()`).
pub proof fn theorem_neutral_layers_resolve_to_default(ls: Seq<LayerView>)
    requires
        forall|k: int| 0 <= k < ls.len() ==> is_neutral(#[trigger] ls[k]),
    ensures
        spec_resolve(knobs_of(ls)) == spec_resolve(Seq::empty()),
        spec_extras(knobs_of(ls), extras_of(ls)) == Seq::<String>::empty(),
    decreases ls.len(),
{
    if ls.len() > 0 {
        let a = ls.drop_last();
        assert forall|k: int| 0 <= k < a.len() implies is_neutral(#[trigger] a[k]) by {
            assert(a[k] == ls[k]);
        }
        theorem_neutral_layers_resolve_to_default(a);
        theorem_neutral_layer(a, Seq::empty(), ls.last());
        assert(a + seq![ls.last()] + Seq::<LayerView>::empty() =~= ls);
        assert(a + Seq::<LayerView>::empty() =~= a);
    } else {
        theorem_default_resolution();
        assert(extras_of(ls) =~= Seq::<Seq<String>>::empty());
        assert(knobs_of(ls) =~= Seq::<KnobLayer>::empty());
    }
}

} // verus!
