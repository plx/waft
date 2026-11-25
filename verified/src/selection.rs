//! Verified kernel: candidate-selection routing, the rule-file consult
//! gates, and empty-selection diagnosis.
//!
//! # What it decides
//!
//! - [`selection_source`]: where the candidate set comes from (the wt-0.39
//!   subtractive walk, the per-path engine, nothing, or every git-ignored
//!   untracked file), from whether a rule file exists under the active
//!   symlink policy, the semantics, and `when_missing`
//!   (`subcommands::select_candidates`).
//! - [`rule_file_consulted`]: whether the per-path engines read a
//!   `.worktreeinclude` at a given location (`explain_path`'s gate and
//!   `root_rule_file_is_consulted`), probing the filesystem lazily through
//!   [`RuleFileProbe`].
//! - [`wt_entry_consulted`]: whether the wt-0.39 walk reads a
//!   `.worktreeinclude` entry, from the walk's (no-follow) entry type.
//! - [`diagnose_empty_selection`]: which configuration cause, if any, to
//!   report for an empty selection, probing lazily through
//!   [`DiagnosisFacts`].
//!
//! # Theorems
//!
//! - `selection_source`: `theorem_when_missing_only_without_rule_file`
//!   (`when_missing` is consulted iff no rule file exists) and
//!   `theorem_rule_file_engages_rules` (a rule file always routes through
//!   the rules, never through the fallback).
//! - `rule_file_consulted` returns `spec_rule_file_consulted` and its
//!   probe log shows the symlink check runs only under the `ignore` policy
//!   and only for a regular file;
//!   `theorem_ignore_policy_never_consults_a_symlink`.
//! - The wt gate and the per-path gate agree except for a symlink whose
//!   target is not a regular file under `follow`/`error`, which only wt
//!   reads (`theorem_wt_gate_differs_only_for_non_regular_symlink_targets`,
//!   `lemma_quirk_wt_reads_symlink_to_non_regular_file`; current
//!   behavior, not fixed: a symlink to `/dev/zero` or a FIFO is read).
//! - `diagnose_empty_selection` returns `spec_diagnosis` with an exact
//!   probe log (the expensive, fallible `follow` walk runs only for an empty
//!   blank selection under the `ignore` policy), and
//!   `theorem_diagnosis_soundness` states the observed facts behind every
//!   reported cause.
//!
//! # Trust assumptions
//!
//! - [`RuleFileProbe`]: `probe_is_file` is `Path::is_file` (follows
//!   symlinks) and `probe_is_symlink` is `symlink_metadata(..)` reporting a
//!   symlink (false when it cannot be read); answers are stable during one
//!   decision (TOCTOU with the later read is out of scope).
//! - [`wt_entry_consulted`] receives walkdir's no-follow entry type, whose
//!   directory, file and symlink tests are mutually exclusive.
//! - [`DiagnosisFacts`]: the root symlink check, `root_rule_file_is_consulted`
//!   and `GitBackend::worktreeinclude_exists_anywhere(.., Follow)` report
//!   what they claim.
//! - Production converts its configuration enums with exhaustive matches.

#[allow(unused_imports)]
use vstd::prelude::*;

verus! {

/// How symlinked `.worktreeinclude` files are treated.
#[derive(Clone, Copy, Debug, PartialEq, Eq, StructuralEq)]
pub enum SymlinkPolicy {
    /// Read the symlink's target as a rule file.
    Follow,
    /// Treat symlinked rule files as absent.
    Ignore,
    /// Reject symlinked rule files (enforced by validation).
    Error,
}

/// The `.worktreeinclude` semantics profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq, StructuralEq)]
pub enum Semantics {
    /// Git per-directory semantics (all applicable rule files).
    Git,
    /// Claude Code 2026-04: only the repository-root rule file.
    Claude202604,
    /// worktrunk 0.39: subtractive literal negations.
    Wt039,
}

/// What to select when no rule file exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq, StructuralEq)]
pub enum WhenMissing {
    /// Select nothing.
    Blank,
    /// Select every git-ignored untracked file.
    AllIgnored,
}

/// Where the candidate set comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, StructuralEq)]
pub enum SelectionSource {
    /// The wt-0.39 subtractive selection.
    WtSubtractive,
    /// The per-path rule engine of the active semantics.
    PerPathEngine,
    /// No candidates.
    Blank,
    /// Every git-ignored untracked file.
    AllIgnored,
}

/// Choose the selection source.
pub fn selection_source(rule_file_exists: bool, semantics: Semantics, when_missing: WhenMissing) -> (s:
    SelectionSource)
    ensures
        rule_file_exists ==> s == (if semantics == Semantics::Wt039 {
            SelectionSource::WtSubtractive
        } else {
            SelectionSource::PerPathEngine
        }),
        !rule_file_exists ==> s == (match when_missing {
            WhenMissing::Blank => SelectionSource::Blank,
            WhenMissing::AllIgnored => SelectionSource::AllIgnored,
        }),
{
    if rule_file_exists {
        match semantics {
            Semantics::Wt039 => SelectionSource::WtSubtractive,
            Semantics::Git | Semantics::Claude202604 => SelectionSource::PerPathEngine,
        }
    } else {
        match when_missing {
            WhenMissing::Blank => SelectionSource::Blank,
            WhenMissing::AllIgnored => SelectionSource::AllIgnored,
        }
    }
}

/// The selection source as a function (the ensures of
/// [`selection_source`] determine it).
pub open spec fn spec_selection_source(rule_file_exists: bool, semantics: Semantics, when_missing: WhenMissing) -> SelectionSource {
    if rule_file_exists {
        if semantics == Semantics::Wt039 {
            SelectionSource::WtSubtractive
        } else {
            SelectionSource::PerPathEngine
        }
    } else {
        match when_missing {
            WhenMissing::Blank => SelectionSource::Blank,
            WhenMissing::AllIgnored => SelectionSource::AllIgnored,
        }
    }
}

/// `when_missing` matters exactly when no rule file exists, and then the
/// semantics does not.
pub proof fn theorem_when_missing_only_without_rule_file(s1: Semantics, s2: Semantics, w1: WhenMissing, w2: WhenMissing)
    ensures
        spec_selection_source(true, s1, w1) == spec_selection_source(true, s1, w2),
        spec_selection_source(false, s1, w1) == spec_selection_source(false, s2, w1),
        w1 != w2 ==> spec_selection_source(false, s1, w1) != spec_selection_source(false, s1, w2),
{
}

/// A rule file always routes selection through the rules (wt's
/// subtractive walk or the per-path engine), and its absence never does.
pub proof fn theorem_rule_file_engages_rules(e: bool, s: Semantics, w: WhenMissing)
    ensures
        e <==> (spec_selection_source(e, s, w) == SelectionSource::WtSubtractive || spec_selection_source(e, s, w)
            == SelectionSource::PerPathEngine),
        spec_selection_source(e, s, w) == SelectionSource::WtSubtractive <==> e && s == Semantics::Wt039,
{
}

// ---------------------------------------------------------------------------
// Rule-file consult gates
// ---------------------------------------------------------------------------

/// A filesystem probe issued by [`rule_file_consulted`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, StructuralEq)]
pub enum GateProbe {
    /// `Path::is_file` (follows symlinks).
    IsFile,
    /// `symlink_metadata` reports a symlink.
    IsSymlink,
}

/// The filesystem facts about one rule-file location, implemented by
/// production.
///
/// Trusted: `probe_is_file` reports whether the path resolves (following
/// symlinks) to a regular file and `probe_is_symlink` whether the path
/// itself is a symlink (false when its metadata cannot be read). Answers
/// are stable during one decision; `probes` is a ghost log of the probes.
pub trait RuleFileProbe {
    /// The path resolves to a regular file.
    spec fn regular_file(&self) -> bool;

    /// The path itself is a symlink.
    spec fn symlink(&self) -> bool;

    /// Probes issued so far, in order.
    spec fn probes(&self) -> Seq<GateProbe>;

    /// Probe whether the path resolves to a regular file.
    fn probe_is_file(&mut self) -> (r: bool)
        ensures
            r == old(self).regular_file(),
            final(self).probes() == old(self).probes().push(GateProbe::IsFile),
            final(self).regular_file() == old(self).regular_file(),
            final(self).symlink() == old(self).symlink(),
    ;

    /// Probe whether the path itself is a symlink.
    fn probe_is_symlink(&mut self) -> (r: bool)
        ensures
            r == old(self).symlink(),
            final(self).probes() == old(self).probes().push(GateProbe::IsSymlink),
            final(self).regular_file() == old(self).regular_file(),
            final(self).symlink() == old(self).symlink(),
    ;
}

/// The per-path engines read a rule file iff it resolves to a regular file
/// and is not a symlink skipped by the `ignore` policy.
pub open spec fn spec_rule_file_consulted(regular_file: bool, symlink: bool, policy: SymlinkPolicy) -> bool {
    regular_file && !(policy == SymlinkPolicy::Ignore && symlink)
}

/// Decide whether the per-path engines read the rule file at a location.
pub fn rule_file_consulted<P: RuleFileProbe>(p: &mut P, policy: SymlinkPolicy) -> (r: bool)
    ensures
        r == spec_rule_file_consulted(old(p).regular_file(), old(p).symlink(), policy),
        final(p).probes() == old(p).probes() + if old(p).regular_file() && policy == SymlinkPolicy::Ignore {
            seq![GateProbe::IsFile, GateProbe::IsSymlink]
        } else {
            seq![GateProbe::IsFile]
        },
{
    let ghost p0 = *p;
    if !p.probe_is_file() {
        assert(p.probes() =~= p0.probes() + seq![GateProbe::IsFile]);
        return false;
    }
    if policy == SymlinkPolicy::Ignore {
        let symlink = p.probe_is_symlink();
        assert(p.probes() =~= p0.probes() + seq![GateProbe::IsFile, GateProbe::IsSymlink]);
        return !symlink;
    }
    assert(p.probes() =~= p0.probes() + seq![GateProbe::IsFile]);
    true
}

/// Under the `ignore` policy no per-path engine reads a symlinked rule
/// file; under the other policies a symlink is read iff its target is a
/// regular file.
pub proof fn theorem_ignore_policy_never_consults_a_symlink(regular_file: bool, symlink: bool, policy: SymlinkPolicy)
    ensures
        policy == SymlinkPolicy::Ignore && symlink ==> !spec_rule_file_consulted(regular_file, symlink, policy),
        policy != SymlinkPolicy::Ignore ==> (spec_rule_file_consulted(regular_file, symlink, policy) == regular_file),
{
}

/// The no-follow type of a walked directory entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, StructuralEq)]
pub enum EntryKind {
    /// A directory.
    Dir,
    /// A regular file.
    File,
    /// A symbolic link (its target is not examined).
    Symlink,
    /// Anything else (FIFO, socket, device).
    Other,
}

/// The wt-0.39 walk reads a `.worktreeinclude` entry iff it is a regular
/// file, or a symlink and the policy is not `ignore` (the symlink's target
/// is never examined).
pub open spec fn spec_wt_entry_consulted(kind: EntryKind, policy: SymlinkPolicy) -> bool {
    kind == EntryKind::File || (kind == EntryKind::Symlink && policy != SymlinkPolicy::Ignore)
}

/// Decide whether the wt-0.39 walk reads a `.worktreeinclude` entry.
pub fn wt_entry_consulted(kind: EntryKind, policy: SymlinkPolicy) -> (r: bool)
    ensures
        r == spec_wt_entry_consulted(kind, policy),
{
    match kind {
        EntryKind::Dir => false,
        EntryKind::Symlink => policy != SymlinkPolicy::Ignore,
        EntryKind::File => true,
        EntryKind::Other => false,
    }
}

/// The filesystem facts the two gates observe are consistent: a regular
/// file entry resolves to a regular file and is no symlink, a symlink entry
/// is a symlink, and any other entry is neither.
pub open spec fn consistent_entry_facts(kind: EntryKind, regular_file: bool, symlink: bool) -> bool {
    &&& kind == EntryKind::File ==> regular_file && !symlink
    &&& kind == EntryKind::Symlink ==> symlink
    &&& (kind == EntryKind::Dir || kind == EntryKind::Other) ==> !regular_file && !symlink
}

/// The wt gate is never stricter than the per-path gate, and the two
/// disagree only for a symlink whose target is not a regular file under the
/// `follow` or `error` policy: wt reads it, the per-path engines skip it.
pub proof fn theorem_wt_gate_differs_only_for_non_regular_symlink_targets(
    kind: EntryKind,
    regular_file: bool,
    symlink: bool,
    policy: SymlinkPolicy,
)
    requires
        consistent_entry_facts(kind, regular_file, symlink),
    ensures
        spec_rule_file_consulted(regular_file, symlink, policy) ==> spec_wt_entry_consulted(kind, policy),
        spec_wt_entry_consulted(kind, policy) != spec_rule_file_consulted(regular_file, symlink, policy)
            ==> kind == EntryKind::Symlink && !regular_file && policy != SymlinkPolicy::Ignore,
{
}

/// Current behavior, deliberately not fixed: under `follow`, the wt walk
/// reads a `.worktreeinclude` symlink whose target is not a regular file
/// (for example `/dev/zero` or a FIFO, which hangs or exhausts memory),
/// while the per-path engines never read it.
pub proof fn lemma_quirk_wt_reads_symlink_to_non_regular_file()
    ensures
        consistent_entry_facts(EntryKind::Symlink, false, true),
        spec_wt_entry_consulted(EntryKind::Symlink, SymlinkPolicy::Follow),
        !spec_rule_file_consulted(false, true, SymlinkPolicy::Follow),
{
}

// ---------------------------------------------------------------------------
// Empty-selection diagnosis
// ---------------------------------------------------------------------------

/// Observed reasons an empty selection may need configuration attention.
#[derive(Clone, Copy, Debug, PartialEq, Eq, StructuralEq)]
pub enum EmptySelectionCause {
    /// No rule file, and the blank fallback selects nothing.
    NoRuleFile,
    /// Root-only semantics, and no root rule file is consulted.
    NoRootRuleFile,
    /// The `ignore` policy skipped a symlinked rule file.
    RuleFileSymlinkIgnored,
    /// The `ignore` policy skipped a nested symlinked rule file that the
    /// root-only semantics would not read anyway.
    NestedRuleFileSymlinkIgnored,
}

/// A probe issued by [`diagnose_empty_selection`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, StructuralEq)]
pub enum DiagnosisProbe {
    /// `symlink_metadata` of the root rule file reports a symlink.
    RootIsSymlink,
    /// `root_rule_file_is_consulted`.
    RootConsulted,
    /// A rule file exists anywhere under the `follow` policy.
    ExistsUnderFollow,
}

/// The facts behind a diagnosis, implemented by production.
///
/// Trusted: each probe reports the fact its name states (the existence
/// walk may fail with an error of type `E`); answers are stable during one
/// diagnosis; `probes` is a ghost log of the probes issued.
pub trait DiagnosisFacts<E> {
    /// The root `.worktreeinclude` is a symlink.
    spec fn root_is_symlink(&self) -> bool;

    /// The root rule file is consulted by the root-only engine.
    spec fn root_consulted(&self) -> bool;

    /// Whether a rule file exists anywhere under the `follow` policy.
    spec fn exists_under_follow(&self) -> Result<bool, E>;

    /// Probes issued so far, in order.
    spec fn probes(&self) -> Seq<DiagnosisProbe>;

    /// Probe whether the root rule file is a symlink.
    fn probe_root_is_symlink(&mut self) -> (r: bool)
        ensures
            r == old(self).root_is_symlink(),
            final(self).probes() == old(self).probes().push(DiagnosisProbe::RootIsSymlink),
            final(self).root_is_symlink() == old(self).root_is_symlink(),
            final(self).root_consulted() == old(self).root_consulted(),
            final(self).exists_under_follow() == old(self).exists_under_follow(),
    ;

    /// Probe whether the root rule file is consulted.
    fn probe_root_consulted(&mut self) -> (r: bool)
        ensures
            r == old(self).root_consulted(),
            final(self).probes() == old(self).probes().push(DiagnosisProbe::RootConsulted),
            final(self).root_is_symlink() == old(self).root_is_symlink(),
            final(self).root_consulted() == old(self).root_consulted(),
            final(self).exists_under_follow() == old(self).exists_under_follow(),
    ;

    /// Walk for a rule file under the `follow` policy.
    fn probe_exists_under_follow(&mut self) -> (r: Result<bool, E>)
        ensures
            r == old(self).exists_under_follow(),
            final(self).probes() == old(self).probes().push(DiagnosisProbe::ExistsUnderFollow),
            final(self).root_is_symlink() == old(self).root_is_symlink(),
            final(self).root_consulted() == old(self).root_consulted(),
            final(self).exists_under_follow() == old(self).exists_under_follow(),
    ;
}

/// The diagnosis as a decision table over the observed facts.
pub open spec fn spec_diagnosis<E>(
    semantics: Semantics,
    policy: SymlinkPolicy,
    when_missing: WhenMissing,
    rule_file_found: bool,
    root_is_symlink: bool,
    root_consulted: bool,
    exists_under_follow: Result<bool, E>,
) -> Result<Option<EmptySelectionCause>, E> {
    let root_only = semantics == Semantics::Claude202604;
    let root_symlink_ignored = policy == SymlinkPolicy::Ignore && root_is_symlink;
    if rule_file_found {
        if root_only && root_symlink_ignored {
            Ok(Some(EmptySelectionCause::RuleFileSymlinkIgnored))
        } else if root_only && !root_consulted {
            Ok(Some(EmptySelectionCause::NoRootRuleFile))
        } else {
            Ok(None)
        }
    } else if when_missing != WhenMissing::Blank {
        Ok(None)
    } else if policy != SymlinkPolicy::Ignore {
        Ok(Some(EmptySelectionCause::NoRuleFile))
    } else {
        match exists_under_follow {
            Err(e) => Err(e),
            Ok(true) => Ok(
                Some(
                    if root_only && !root_symlink_ignored {
                        EmptySelectionCause::NestedRuleFileSymlinkIgnored
                    } else {
                        EmptySelectionCause::RuleFileSymlinkIgnored
                    },
                ),
            ),
            Ok(false) => Ok(Some(EmptySelectionCause::NoRuleFile)),
        }
    }
}

/// The probes a diagnosis issues: the root symlink check only under the
/// `ignore` policy, the root consult check only when a rule file was found
/// under root-only semantics and the root symlink was not skipped, and the
/// `follow` walk only for a blank selection without rule files under the
/// `ignore` policy.
pub open spec fn diagnosis_trace(
    semantics: Semantics,
    policy: SymlinkPolicy,
    when_missing: WhenMissing,
    rule_file_found: bool,
    root_is_symlink: bool,
) -> Seq<DiagnosisProbe> {
    let ignore = policy == SymlinkPolicy::Ignore;
    let root_only = semantics == Semantics::Claude202604;
    (if ignore {
        seq![DiagnosisProbe::RootIsSymlink]
    } else {
        Seq::empty()
    }) + (if rule_file_found && root_only && !(ignore && root_is_symlink) {
        seq![DiagnosisProbe::RootConsulted]
    } else {
        Seq::empty()
    }) + (if !rule_file_found && when_missing == WhenMissing::Blank && ignore {
        seq![DiagnosisProbe::ExistsUnderFollow]
    } else {
        Seq::empty()
    })
}

/// Diagnose an empty selection.
pub fn diagnose_empty_selection<E, F: DiagnosisFacts<E>>(
    f: &mut F,
    semantics: Semantics,
    policy: SymlinkPolicy,
    when_missing: WhenMissing,
    rule_file_found: bool,
) -> (r: Result<Option<EmptySelectionCause>, E>)
    ensures
        r == spec_diagnosis(
            semantics,
            policy,
            when_missing,
            rule_file_found,
            old(f).root_is_symlink(),
            old(f).root_consulted(),
            old(f).exists_under_follow(),
        ),
        final(f).probes() == old(f).probes() + diagnosis_trace(
            semantics,
            policy,
            when_missing,
            rule_file_found,
            old(f).root_is_symlink(),
        ),
{
    let ghost f0 = *f;
    let ghost trace = diagnosis_trace(semantics, policy, when_missing, rule_file_found, f0.root_is_symlink());
    let root_only = semantics == Semantics::Claude202604;
    let root_symlink_ignored = policy == SymlinkPolicy::Ignore && f.probe_root_is_symlink();
    if rule_file_found {
        if root_only {
            if root_symlink_ignored {
                assert(f.probes() =~= f0.probes() + trace);
                return Ok(Some(EmptySelectionCause::RuleFileSymlinkIgnored));
            }
            if !f.probe_root_consulted() {
                assert(f.probes() =~= f0.probes() + trace);
                return Ok(Some(EmptySelectionCause::NoRootRuleFile));
            }
        }
        assert(f.probes() =~= f0.probes() + trace);
        return Ok(None);
    }
    if when_missing != WhenMissing::Blank {
        assert(f.probes() =~= f0.probes() + trace);
        return Ok(None);
    }
    if policy == SymlinkPolicy::Ignore {
        let exists = f.probe_exists_under_follow();
        assert(f.probes() =~= f0.probes() + trace);
        if exists? {
            return Ok(
                Some(
                    if root_only && !root_symlink_ignored {
                        EmptySelectionCause::NestedRuleFileSymlinkIgnored
                    } else {
                        EmptySelectionCause::RuleFileSymlinkIgnored
                    },
                ),
            );
        }
    } else {
        assert(f.probes() =~= f0.probes() + trace);
    }
    Ok(Some(EmptySelectionCause::NoRuleFile))
}

/// Diagnosis soundness: every reported cause is backed by the facts its
/// message states, a symlink is blamed only under the `ignore` policy, an
/// error can only come from the `follow` walk, and nothing is reported when
/// a rule file was found and the semantics reads every rule file.
pub proof fn theorem_diagnosis_soundness<E>(
    semantics: Semantics,
    policy: SymlinkPolicy,
    when_missing: WhenMissing,
    rule_file_found: bool,
    root_is_symlink: bool,
    root_consulted: bool,
    exists_under_follow: Result<bool, E>,
)
    ensures
        ({
            let root_only = semantics == Semantics::Claude202604;
            let ignore = policy == SymlinkPolicy::Ignore;
            match spec_diagnosis(
                semantics,
                policy,
                when_missing,
                rule_file_found,
                root_is_symlink,
                root_consulted,
                exists_under_follow,
            ) {
                Ok(Some(EmptySelectionCause::RuleFileSymlinkIgnored)) => ignore && (root_is_symlink
                    || exists_under_follow == Ok::<bool, E>(true)),
                Ok(Some(EmptySelectionCause::NoRootRuleFile)) => root_only && rule_file_found && !root_consulted
                    && !(ignore && root_is_symlink),
                Ok(Some(EmptySelectionCause::NestedRuleFileSymlinkIgnored)) => root_only && ignore
                    && !rule_file_found && exists_under_follow == Ok::<bool, E>(true) && !root_is_symlink,
                Ok(Some(EmptySelectionCause::NoRuleFile)) => !rule_file_found && when_missing == WhenMissing::Blank
                    && !(ignore && exists_under_follow == Ok::<bool, E>(true)),
                Ok(None) => rule_file_found || when_missing != WhenMissing::Blank,
                Err(_) => !rule_file_found && when_missing == WhenMissing::Blank && ignore
                    && exists_under_follow is Err,
            }
        }),
        rule_file_found && semantics != Semantics::Claude202604 ==> spec_diagnosis(
            semantics,
            policy,
            when_missing,
            rule_file_found,
            root_is_symlink,
            root_consulted,
            exists_under_follow,
        ) == Ok::<Option<EmptySelectionCause>, E>(None),
{
}

} // verus!
