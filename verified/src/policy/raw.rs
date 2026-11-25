//! TOML layer acceptance: after serde has produced the raw strings of a
//! `.waft.toml` or user config file, decide whether the file is accepted,
//! which error is reported first, and the knob values of the layer.
//!
//! # What it decides
//!
//! [`layer_from_raw`] (called by production `parse_toml`) applies the
//! version gate, checks the string-valued fields in the fixed order
//! `compat.profile`, `worktreeinclude.when_missing`,
//! `worktreeinclude.semantics`, `worktreeinclude.symlink_policy`,
//! `exclude.builtin_set`, `copy.strategy`, and maps each field to its knob.
//!
//! # Theorems (postconditions of `layer_from_raw`)
//!
//! - Version gate: a present `version` other than 1 is `UnsupportedVersion`
//!   with that number, whatever else the file contains, and
//!   `UnsupportedVersion` is returned for nothing else.
//! - Acceptance: `Ok` exactly when the version is absent or 1 and every
//!   present field is an accepted spelling (per [`super::vocab`]).
//! - Error precedence: when the version is fine, the first bad field `f`
//!   (`first_bad_field`: present, not accepted, every field before it absent
//!   or accepted) is reported as `UnknownValue(f, s)` with `s` its raw
//!   value; conversely every `UnknownValue(f, s)` is that report.
//! - Field independence: an accepted layer holds each field's `spec_parse`
//!   (`None` when absent) and `replace_extra` unchanged, so a file with no
//!   recognized fields yields the empty layer.
//! - Determinism: `theorem_first_bad_field` shows that a file with a bad
//!   field has exactly one first bad field, so the postconditions above fix
//!   the result (variant and payload) for every input.
//!
//! # Trust
//!
//! serde/toml deserialization (unknown-key rejection, types, the `u32`
//! range of `version`), the flattening of absent sections to absent fields,
//! `exclude.extra` (copied as is), and the error-message text stay in
//! production.

#[allow(unused_imports)]
use vstd::prelude::*;

use super::merge::KnobLayer;
use super::vocab::{
    BuiltinExcludeSet, CompatProfile, CopyStrategy, SymlinkPolicy, WhenMissingWorktreeinclude,
    WorktreeincludeSemantics,
};

verus! {

/// A string-valued knob of a config file, in the order errors are checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawField {
    /// `compat.profile`.
    CompatProfile,
    /// `worktreeinclude.when_missing`.
    WhenMissing,
    /// `worktreeinclude.semantics`.
    Semantics,
    /// `worktreeinclude.symlink_policy`.
    SymlinkPolicy,
    /// `exclude.builtin_set`.
    BuiltinExcludeSet,
    /// `copy.strategy`.
    CopyStrategy,
}

impl RawField {
    /// Position in the error-checking order.
    pub open spec fn rank(self) -> int {
        match self {
            RawField::CompatProfile => 0,
            RawField::WhenMissing => 1,
            RawField::Semantics => 2,
            RawField::SymlinkPolicy => 3,
            RawField::BuiltinExcludeSet => 4,
            RawField::CopyStrategy => 5,
        }
    }
}

/// Why a config file is rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawError<'a> {
    /// `version` is present and is not 1.
    UnsupportedVersion(u32),
    /// The field's value is not an accepted spelling.
    UnknownValue(RawField, &'a str),
}

/// The scalar values serde extracted from a config file (`None` = absent).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawKnobs<'a> {
    /// `version`.
    pub version: Option<u32>,
    /// `compat.profile`.
    pub profile: Option<&'a str>,
    /// `worktreeinclude.when_missing`.
    pub when_missing: Option<&'a str>,
    /// `worktreeinclude.semantics`.
    pub semantics: Option<&'a str>,
    /// `worktreeinclude.symlink_policy`.
    pub symlink_policy: Option<&'a str>,
    /// `exclude.builtin_set`.
    pub builtin_exclude_set: Option<&'a str>,
    /// `copy.strategy`.
    pub copy_strategy: Option<&'a str>,
    /// `exclude.replace_extra`.
    pub replace_extra: Option<bool>,
}

/// The raw value of `field`.
pub open spec fn field_value<'a>(raw: RawKnobs<'a>, field: RawField) -> Option<&'a str> {
    match field {
        RawField::CompatProfile => raw.profile,
        RawField::WhenMissing => raw.when_missing,
        RawField::Semantics => raw.semantics,
        RawField::SymlinkPolicy => raw.symlink_policy,
        RawField::BuiltinExcludeSet => raw.builtin_exclude_set,
        RawField::CopyStrategy => raw.copy_strategy,
    }
}

/// Whether `s` is an accepted spelling for `field`.
pub open spec fn accepted(field: RawField, s: Seq<char>) -> bool {
    match field {
        RawField::CompatProfile => CompatProfile::spec_parse(s) is Some,
        RawField::WhenMissing => WhenMissingWorktreeinclude::spec_parse(s) is Some,
        RawField::Semantics => WorktreeincludeSemantics::spec_parse(s) is Some,
        RawField::SymlinkPolicy => SymlinkPolicy::spec_parse(s) is Some,
        RawField::BuiltinExcludeSet => BuiltinExcludeSet::spec_parse(s) is Some,
        RawField::CopyStrategy => CopyStrategy::spec_parse(s) is Some,
    }
}

/// Whether `field` is absent or accepted.
pub open spec fn field_ok(raw: RawKnobs, field: RawField) -> bool {
    match field_value(raw, field) {
        Some(s) => accepted(field, s@),
        None => true,
    }
}

/// Whether `version` is present and not 1.
pub open spec fn version_bad(raw: RawKnobs) -> bool {
    match raw.version {
        Some(v) => v != 1,
        None => false,
    }
}

/// Whether `field` is the first bad field: it is present with a spelling
/// that is not accepted, and every field before it in `RawField::rank`
/// order is absent or accepted.
pub open spec fn first_bad_field(raw: RawKnobs, field: RawField) -> bool {
    &&& !field_ok(raw, field)
    &&& forall|g: RawField| g.rank() < field.rank() ==> #[trigger] field_ok(raw, g)
}

/// At most one field is the first bad field.
pub proof fn lemma_first_bad_field_unique(raw: RawKnobs, f: RawField, g: RawField)
    requires
        first_bad_field(raw, f),
        first_bad_field(raw, g),
    ensures
        f == g,
{
    // Neither can come before the other, since the earlier one would then be
    // accepted; equal ranks mean equal fields.
    assert(!(f.rank() < g.rank()));
    assert(!(g.rank() < f.rank()));
    match f {
        RawField::CompatProfile => {},
        RawField::WhenMissing => {},
        RawField::Semantics => {},
        RawField::SymlinkPolicy => {},
        RawField::BuiltinExcludeSet => {},
        RawField::CopyStrategy => {},
    }
}

/// Every file has either no bad field or exactly one first bad field. With
/// the postconditions of [`layer_from_raw`] (a bad version decides the
/// result; otherwise all fields fine gives the parsed layer and a first bad
/// field gives `UnknownValue` naming it), this shows those postconditions
/// determine the result for every input.
pub proof fn theorem_first_bad_field(raw: RawKnobs)
    ensures
        (forall|f: RawField| #[trigger] field_ok(raw, f)) || (exists|f: RawField|
            #[trigger] first_bad_field(raw, f)),
        forall|f: RawField, g: RawField|
            #[trigger] first_bad_field(raw, f) && #[trigger] first_bad_field(raw, g) ==> f == g,
{
    assert forall|f: RawField, g: RawField|
        #[trigger] first_bad_field(raw, f) && #[trigger] first_bad_field(raw, g) implies f == g by {
        lemma_first_bad_field_unique(raw, f, g);
    }
    if !field_ok(raw, RawField::CompatProfile) {
        assert(first_bad_field(raw, RawField::CompatProfile));
    } else if !field_ok(raw, RawField::WhenMissing) {
        assert(first_bad_field(raw, RawField::WhenMissing));
    } else if !field_ok(raw, RawField::Semantics) {
        assert(first_bad_field(raw, RawField::Semantics));
    } else if !field_ok(raw, RawField::SymlinkPolicy) {
        assert(first_bad_field(raw, RawField::SymlinkPolicy));
    } else if !field_ok(raw, RawField::BuiltinExcludeSet) {
        assert(first_bad_field(raw, RawField::BuiltinExcludeSet));
    } else if !field_ok(raw, RawField::CopyStrategy) {
        assert(first_bad_field(raw, RawField::CopyStrategy));
    } else {
        assert forall|f: RawField| #[trigger] field_ok(raw, f) by {
            match f {
                RawField::CompatProfile => {},
                RawField::WhenMissing => {},
                RawField::Semantics => {},
                RawField::SymlinkPolicy => {},
                RawField::BuiltinExcludeSet => {},
                RawField::CopyStrategy => {},
            }
        }
    }
}

/// Parses an optional value with `parse` semantics lifted to `Option`.
pub open spec fn lift<T>(o: Option<&str>, parse: spec_fn(Seq<char>) -> Option<T>) -> Option<T> {
    match o {
        Some(s) => parse(s@),
        None => None,
    }
}

/// Decides a config file's layer from its raw values.
///
/// Proven: the version gate comes first (`version` present and not 1 is
/// `UnsupportedVersion` with that number whatever else the file contains,
/// and `UnsupportedVersion` is returned only then); otherwise the file is
/// accepted exactly when every present field has an accepted spelling; when
/// the version is fine, the first bad field in `RawField::rank` order is
/// always reported as `UnknownValue` together with its raw value, and every
/// `UnknownValue` is that one; an accepted layer holds each field's parse
/// (`None` when absent) and `replace_extra` unchanged. With
/// `theorem_first_bad_field` these postconditions determine the result for
/// every input.
pub fn layer_from_raw<'a>(raw: RawKnobs<'a>) -> (r: Result<KnobLayer, RawError<'a>>)
    ensures
        version_bad(raw) ==> r == Err::<KnobLayer, RawError>(
            RawError::UnsupportedVersion(raw.version->0),
        ),
        r matches Err(RawError::UnsupportedVersion(v)) ==> {
            &&& version_bad(raw)
            &&& raw.version == Some(v)
        },
        r is Ok <==> !version_bad(raw) && forall|f: RawField| #[trigger] field_ok(raw, f),
        forall|f: RawField|
            !version_bad(raw) && #[trigger] first_bad_field(raw, f) ==> r == Err::<KnobLayer, RawError>(
                RawError::UnknownValue(f, field_value(raw, f)->0),
            ),
        r matches Err(RawError::UnknownValue(f, s)) ==> {
            &&& !version_bad(raw)
            &&& field_value(raw, f) == Some(s)
            &&& !accepted(f, s@)
            &&& forall|g: RawField| g.rank() < f.rank() ==> #[trigger] field_ok(raw, g)
        },
        r matches Ok(l) ==> {
            &&& l.profile == lift(raw.profile, |s: Seq<char>| CompatProfile::spec_parse(s))
            &&& l.when_missing == lift(
                raw.when_missing,
                |s: Seq<char>| WhenMissingWorktreeinclude::spec_parse(s),
            )
            &&& l.semantics == lift(
                raw.semantics,
                |s: Seq<char>| WorktreeincludeSemantics::spec_parse(s),
            )
            &&& l.symlink_policy == lift(
                raw.symlink_policy,
                |s: Seq<char>| SymlinkPolicy::spec_parse(s),
            )
            &&& l.builtin_exclude_set == lift(
                raw.builtin_exclude_set,
                |s: Seq<char>| BuiltinExcludeSet::spec_parse(s),
            )
            &&& l.copy_strategy == lift(raw.copy_strategy, |s: Seq<char>| CopyStrategy::spec_parse(s))
            &&& l.replace_extra == raw.replace_extra
        },
{
    match raw.version {
        Some(v) if v != 1 => return Err(RawError::UnsupportedVersion(v)),
        Some(_) | None => {},
    }
    let profile = match raw.profile {
        Some(s) => match CompatProfile::parse(s) {
            Some(x) => Some(x),
            None => {
                proof {
                    assert(!field_ok(raw, RawField::CompatProfile));
                }
                return Err(RawError::UnknownValue(RawField::CompatProfile, s));
            },
        },
        None => None,
    };
    assert(field_ok(raw, RawField::CompatProfile));
    let when_missing = match raw.when_missing {
        Some(s) => match WhenMissingWorktreeinclude::parse(s) {
            Some(x) => Some(x),
            None => {
                proof {
                    assert(!field_ok(raw, RawField::WhenMissing));
                    assert forall|g: RawField| g.rank() < RawField::WhenMissing.rank() implies #[trigger] field_ok(raw, g) by {
                        assert(g == RawField::CompatProfile);
                    }
                }
                return Err(RawError::UnknownValue(RawField::WhenMissing, s));
            },
        },
        None => None,
    };
    assert(field_ok(raw, RawField::WhenMissing));
    let semantics = match raw.semantics {
        Some(s) => match WorktreeincludeSemantics::parse(s) {
            Some(x) => Some(x),
            None => {
                proof {
                    assert(!field_ok(raw, RawField::Semantics));
                    assert forall|g: RawField| g.rank() < RawField::Semantics.rank() implies #[trigger] field_ok(raw, g) by {
                        assert(g == RawField::CompatProfile || g == RawField::WhenMissing);
                    }
                }
                return Err(RawError::UnknownValue(RawField::Semantics, s));
            },
        },
        None => None,
    };
    assert(field_ok(raw, RawField::Semantics));
    let symlink_policy = match raw.symlink_policy {
        Some(s) => match SymlinkPolicy::parse(s) {
            Some(x) => Some(x),
            None => {
                proof {
                    assert(!field_ok(raw, RawField::SymlinkPolicy));
                    assert forall|g: RawField| g.rank() < RawField::SymlinkPolicy.rank() implies #[trigger] field_ok(raw, g) by {
                        assert(g == RawField::CompatProfile || g == RawField::WhenMissing || g
                            == RawField::Semantics);
                    }
                }
                return Err(RawError::UnknownValue(RawField::SymlinkPolicy, s));
            },
        },
        None => None,
    };
    assert(field_ok(raw, RawField::SymlinkPolicy));
    let builtin_exclude_set = match raw.builtin_exclude_set {
        Some(s) => match BuiltinExcludeSet::parse(s) {
            Some(x) => Some(x),
            None => {
                proof {
                    assert(!field_ok(raw, RawField::BuiltinExcludeSet));
                    assert forall|g: RawField| g.rank() < RawField::BuiltinExcludeSet.rank() implies #[trigger] field_ok(raw, g) by {
                        assert(g == RawField::CompatProfile || g == RawField::WhenMissing || g
                            == RawField::Semantics || g == RawField::SymlinkPolicy);
                    }
                }
                return Err(RawError::UnknownValue(RawField::BuiltinExcludeSet, s));
            },
        },
        None => None,
    };
    assert(field_ok(raw, RawField::BuiltinExcludeSet));
    let copy_strategy = match raw.copy_strategy {
        Some(s) => match CopyStrategy::parse(s) {
            Some(x) => Some(x),
            None => {
                proof {
                    assert(!field_ok(raw, RawField::CopyStrategy));
                    assert forall|g: RawField| g.rank() < RawField::CopyStrategy.rank() implies #[trigger] field_ok(raw, g) by {
                        assert(g == RawField::CompatProfile || g == RawField::WhenMissing || g
                            == RawField::Semantics || g == RawField::SymlinkPolicy || g
                            == RawField::BuiltinExcludeSet);
                    }
                }
                return Err(RawError::UnknownValue(RawField::CopyStrategy, s));
            },
        },
        None => None,
    };
    assert(field_ok(raw, RawField::CopyStrategy));
    proof {
        assert forall|f: RawField| #[trigger] field_ok(raw, f) by {
            match f {
                RawField::CompatProfile => {},
                RawField::WhenMissing => {},
                RawField::Semantics => {},
                RawField::SymlinkPolicy => {},
                RawField::BuiltinExcludeSet => {},
                RawField::CopyStrategy => {},
            }
        }
    }
    Ok(
        KnobLayer {
            profile,
            when_missing,
            semantics,
            symlink_policy,
            builtin_exclude_set,
            copy_strategy,
            replace_extra: raw.replace_extra,
        },
    )
}

} // verus!
