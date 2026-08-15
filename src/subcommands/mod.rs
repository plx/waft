//! Subcommand argument types and top-level handlers.
//!
//! Each submodule owns the `clap::Args` struct and the `run_*` entry point
//! for one subcommand. The handlers share the same early stages — context
//! resolution and validation — then diverge based on which pipeline stage the
//! subcommand needs to reach (see `docs/architecture.md`).

mod copy;
mod info;
mod list;
mod validate;

use std::path::Path;

use crate::config::{
    ResolvedPolicy, SymlinkPolicy, WhenMissingWorktreeinclude, WorktreeincludeSemantics,
};
use crate::error::{Error, Result};
use crate::git::{GitBackend, IgnoreCheckRecord};
use crate::model::WorktreeincludeStatus;
use crate::path::RepoRelPath;

pub(crate) use copy::run_copy_with_context;
pub use copy::{CopyArgs, run_copy};
pub(crate) use info::run_info_with_context;
pub use info::{InfoArgs, run_info};
pub(crate) use list::run_list_with_context;
pub use list::{ListArgs, run_list};
pub(crate) use validate::run_validate_with_context;
pub use validate::{ValidateArgs, run_validate};

/// Select candidate paths in `source_root` according to the active policy.
///
/// Mirrors the contract documented in the worktreeinclude config schema:
///
/// - If at least one `.worktreeinclude` file exists anywhere in the repo,
///   selection runs through the `.worktreeinclude` matcher.
/// - Otherwise, `policy.when_missing` decides:
///   - `blank`: no candidates,
///   - `all-ignored`: every git-ignored untracked file is a candidate.
///
/// The returned set is only the profile selection stage. Call
/// [`eligible_records`] to apply exclusions, Git-ignore membership, and
/// physical source-type checks.
///
/// The returned flag is the gate above: whether a `.worktreeinclude` existed
/// anywhere in the repo under the active symlink policy. It says nothing
/// about whether the active *semantics* read that file — see
/// [`diagnose_empty_selection`], which refines it.
pub(crate) fn select_candidates(
    git: &dyn GitBackend,
    source_root: &Path,
    policy: &ResolvedPolicy,
) -> Result<(Vec<RepoRelPath>, bool)> {
    if git.worktreeinclude_exists_anywhere(source_root, policy.symlink_policy)? {
        // The wt-0.39 engine is too unusual for the per-path
        // `list_worktreeinclude_candidates` shape (it's purely subtractive
        // on top of the all-ignored set); route it through a dedicated
        // helper. when_missing is not consulted here because a rule file
        // exists; explicit-selection mode is engaged.
        if policy.semantics == WorktreeincludeSemantics::Wt039 {
            let candidates = crate::worktreeinclude_engine::wt_collect_candidates(
                source_root,
                git,
                policy.symlink_policy,
            )?;
            return Ok((candidates, true));
        }
        let candidates = git.list_worktreeinclude_candidates(
            source_root,
            policy.semantics,
            policy.symlink_policy,
        )?;
        Ok((candidates, true))
    } else {
        let candidates = match policy.when_missing {
            WhenMissingWorktreeinclude::Blank => Vec::new(),
            WhenMissingWorktreeinclude::AllIgnored => git.list_ignored_untracked(source_root)?,
        };
        Ok((candidates, false))
    }
}

/// Result of the shared eligibility pass.
pub(crate) struct Eligibility {
    /// Paths that satisfy the complete eligibility contract.
    pub records: Vec<IgnoreCheckRecord>,
    /// Why an empty `records` is a configuration artifact rather than an
    /// empty repository, when it is one. Always `None` when `records` is
    /// non-empty.
    pub empty_selection_cause: Option<EmptySelectionCause>,
}

/// An empty selection that the configuration, not the repository, explains.
///
/// Each variant is a claim printed to the user, so each must be true on its
/// own terms — "no rule file" is not interchangeable with "a rule file the
/// active configuration never reads".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EmptySelectionCause {
    /// No `.worktreeinclude` exists anywhere in the repo, and the effective
    /// `when_missing` selects nothing without one.
    NoRuleFile,
    /// A `.worktreeinclude` exists, but not at the repository root, and the
    /// active semantics read the root file only.
    NoRootRuleFile,
    /// Every `.worktreeinclude` the active configuration would have read is a
    /// symlink, which the active symlink policy skips — repo-wide, or just at
    /// the root when the semantics read the root file only.
    RuleFileSymlinkIgnored,
}

/// Explain an empty selection, or decline to.
///
/// `rule_file_found` is the coarse repo-wide answer [`select_candidates`]
/// gates `when_missing` on. Turning it into something worth printing takes
/// two refinements it cannot make on its own:
///
/// - It is symlink-policy-relative. Under `SymlinkPolicy::Ignore` a symlinked
///   rule file reads as absent, and telling a user that a file they can see
///   does not exist is worse than saying nothing.
/// - It is semantics-agnostic. `claude-2026-04` reads the root file only, so
///   a rule file that exists exclusively in a subdirectory is found here and
///   still never consulted — the one case where a rule file exists and the
///   empty result is nonetheless a configuration mistake.
///
/// `when_missing` gates only the two "no rule file was consulted" variants.
/// [`EmptySelectionCause::NoRootRuleFile`] ignores it because a rule file was
/// found, so `when_missing` never applied to this run at all.
///
/// The two refinements compose: a symlinked root file plus any regular nested
/// one satisfies the repo-wide gate and fails the root-only check, and only
/// the symlink policy explains that. Reporting the absence of a root file
/// there would be false, and its remedy — Git semantics — would leave the
/// root symlink just as skipped, so the run would stay empty.
fn diagnose_empty_selection(
    git: &dyn GitBackend,
    source_root: &Path,
    policy: &ResolvedPolicy,
    rule_file_found: bool,
) -> Result<Option<EmptySelectionCause>> {
    if rule_file_found {
        let root_only = policy.semantics == WorktreeincludeSemantics::Claude202604;
        if root_only
            && !crate::worktreeinclude::root_rule_file_is_consulted(
                source_root,
                policy.symlink_policy,
            )
        {
            // Re-ask the root predicate while following symlinks, mirroring
            // the repo-wide re-ask below. A yes means the root file exists and
            // the policy is what hid it — and, because the predicate is the
            // engine's own existence gate, that the named remedy restores it.
            if policy.symlink_policy == SymlinkPolicy::Ignore
                && crate::worktreeinclude::root_rule_file_is_consulted(
                    source_root,
                    SymlinkPolicy::Follow,
                )
            {
                return Ok(Some(EmptySelectionCause::RuleFileSymlinkIgnored));
            }
            return Ok(Some(EmptySelectionCause::NoRootRuleFile));
        }
        // A consulted rule file that selects nothing is a legitimate
        // configuration, not a missing one.
        return Ok(None);
    }

    if policy.when_missing != WhenMissingWorktreeinclude::Blank {
        // An absent rule file still selects every git-ignored untracked file,
        // so an empty result means the repository has nothing to copy.
        return Ok(None);
    }

    // Nothing was found under the active symlink policy. Re-ask while
    // following symlinks: if that finds one, the policy is what hid it, and
    // the honest note names the policy rather than denying the file exists.
    // Only reached on an already-empty selection, so the second walk is off
    // the common path.
    if policy.symlink_policy == SymlinkPolicy::Ignore
        && git.worktreeinclude_exists_anywhere(source_root, SymlinkPolicy::Follow)?
    {
        return Ok(Some(EmptySelectionCause::RuleFileSymlinkIgnored));
    }

    Ok(Some(EmptySelectionCause::NoRuleFile))
}

/// Evaluate the complete, command-independent eligibility contract.
///
/// A returned path is selected by the configured worktreeinclude/fallback
/// semantics, survives policy exclusions, is Git-ignored and untracked, and
/// is a physical regular file (not a symlink). Keeping this pass shared makes
/// `list`, `info`, and `copy` agree on the meaning of "eligible".
pub(crate) fn eligible_records(
    git: &dyn GitBackend,
    source_root: &Path,
    policy: &ResolvedPolicy,
    core_ignore_case: bool,
) -> Result<Eligibility> {
    let (mut candidates, rule_file_found) = select_candidates(git, source_root, policy)?;
    candidates.sort();
    candidates.dedup();

    crate::policy_filter::filter_paths_with_case(
        &mut candidates,
        policy,
        source_root,
        crate::policy_filter::effective_case_insensitive(core_ignore_case),
    )?;
    if candidates.is_empty() {
        return Ok(Eligibility {
            records: Vec::new(),
            empty_selection_cause: diagnose_empty_selection(
                git,
                source_root,
                policy,
                rule_file_found,
            )?,
        });
    }

    let mut eligible = Vec::new();
    for record in git.check_ignore(source_root, &candidates)? {
        if !record.ignored {
            continue;
        }

        let source = record.path.to_path(source_root);
        let metadata = match std::fs::symlink_metadata(&source) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(source_error) => {
                return Err(Error::Io {
                    context: format!("reading source metadata for {}", source.display()),
                    source: source_error,
                });
            }
        };
        if metadata.file_type().is_file() {
            eligible.push(record);
        }
    }

    eligible.sort_by(|a, b| a.path.cmp(&b.path));
    eligible.dedup_by(|a, b| a.path == b.path);
    let empty_selection_cause = if eligible.is_empty() {
        diagnose_empty_selection(git, source_root, policy, rule_file_found)?
    } else {
        None
    };
    Ok(Eligibility {
        records: eligible,
        empty_selection_cause,
    })
}

/// Print the diagnosis from [`diagnose_empty_selection`], if there is one.
///
/// An empty result is otherwise indistinguishable from "nothing to do", and
/// the cases below are the ones where the configuration, not the repository,
/// is the answer. Each variant names the specific thing the user can change.
///
/// The note goes to stderr; `list` and `info` write machine-readable data to
/// stdout and it must stay parseable.
pub(crate) fn note_empty_selection(policy: &ResolvedPolicy, pass: &Eligibility, quiet: bool) {
    if quiet {
        return;
    }
    let Some(cause) = pass.empty_selection_cause else {
        return;
    };
    match cause {
        EmptySelectionCause::NoRuleFile => eprintln!(
            "note: no .worktreeinclude found; {} selects nothing without one (see waft validate)",
            blank_selection_subject(policy)
        ),
        EmptySelectionCause::NoRootRuleFile => eprintln!(
            "note: no .worktreeinclude in the repository root; {} semantics ignore nested rule files (use --worktreeinclude-semantics git to read them)",
            policy.semantics.as_str()
        ),
        EmptySelectionCause::RuleFileSymlinkIgnored => eprintln!(
            "note: .worktreeinclude is a symlink and the active symlink policy skips it; pass --worktreeinclude-symlink-policy follow to use it"
        ),
    }
}

/// Name whatever is responsible for selecting nothing without a rule file.
///
/// The profile is only a fair answer when the profile's own preset is what
/// blanks the selection. Under `--compat-profile wt
/// --when-missing-worktreeinclude blank` the explicit knob overrode a profile
/// that otherwise selects every git-ignored untracked file, and blaming `wt`
/// would send the user to change the one setting that is not at fault.
fn blank_selection_subject(policy: &ResolvedPolicy) -> String {
    if policy.profile.selects_without_rule_file() {
        "--when-missing-worktreeinclude blank".to_string()
    } else {
        format!("the {} profile", policy.profile.as_str())
    }
}

/// Render a per-path explanation without contradicting the canonical
/// selection result. Wt 0.39 is set-based: glob negations shown by the
/// diagnostic Git matcher are deliberately non-operative.
pub(crate) fn format_worktreeinclude_status(
    status: &WorktreeincludeStatus,
    selected: bool,
    semantics: WorktreeincludeSemantics,
) -> String {
    match status {
        WorktreeincludeStatus::Included {
            file,
            line,
            pattern,
        } => format!("included ({}:{}: {})", file.display(), line, pattern),
        WorktreeincludeStatus::ExcludedByNegation {
            file,
            line,
            pattern,
        } if selected && semantics == WorktreeincludeSemantics::Wt039 => format!(
            "selected (effective wt-0.39 policy; rule does not subtract this path: {}:{}: {})",
            file.display(),
            line,
            pattern
        ),
        WorktreeincludeStatus::ExcludedByNegation {
            file,
            line,
            pattern,
        } => format!("excluded ({}:{}: {})", file.display(), line, pattern),
        WorktreeincludeStatus::NoMatch if selected => "selected (effective policy)".to_string(),
        WorktreeincludeStatus::NoMatch => "no match".to_string(),
    }
}
