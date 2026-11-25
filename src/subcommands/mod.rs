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

use waft_verified::policy::vocab;
use waft_verified::selection::{self, DiagnosisFacts, SelectionSource};

use crate::config::{ResolvedPolicy, SymlinkPolicy, WorktreeincludeSemantics};
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
    let rule_file_exists =
        git.worktreeinclude_exists_anywhere(source_root, policy.symlink_policy)?;
    // The routing is the verified `selection::selection_source` kernel. The
    // wt-0.39 engine is too unusual for the per-path
    // `list_worktreeinclude_candidates` shape (it's purely subtractive on top
    // of the all-ignored set), so it has a dedicated source; when_missing is
    // consulted only when no rule file exists.
    let source = selection::selection_source(
        rule_file_exists,
        vocab::WorktreeincludeSemantics::from(policy.semantics),
        vocab::WhenMissingWorktreeinclude::from(policy.when_missing),
    );
    let candidates = match source {
        SelectionSource::WtSubtractive => crate::worktreeinclude_engine::wt_collect_candidates(
            source_root,
            git,
            policy.symlink_policy,
        )?,
        SelectionSource::PerPathEngine => git.list_worktreeinclude_candidates(
            source_root,
            policy.semantics,
            policy.symlink_policy,
        )?,
        SelectionSource::Blank => Vec::new(),
        SelectionSource::AllIgnored => git.list_ignored_untracked(source_root)?,
    };
    Ok((candidates, rule_file_exists))
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

/// Observed reasons an empty selection may need configuration attention.
/// These are hints, not predictions that a different policy will validate or
/// select files. Diagnosis never opens a rule file skipped by the policy.
pub(crate) use waft_verified::selection::EmptySelectionCause;

/// Diagnose an empty selection. The decision and the order and laziness of
/// its probes are the verified `selection::diagnose_empty_selection` kernel.
fn diagnose_empty_selection(
    git: &dyn GitBackend,
    source_root: &Path,
    policy: &ResolvedPolicy,
    rule_file_found: bool,
) -> Result<Option<EmptySelectionCause>> {
    let mut facts = DiagnosisProbes {
        git,
        source_root,
        symlink_policy: policy.symlink_policy,
    };
    selection::diagnose_empty_selection(
        &mut facts,
        vocab::WorktreeincludeSemantics::from(policy.semantics),
        vocab::SymlinkPolicy::from(policy.symlink_policy),
        vocab::WhenMissingWorktreeinclude::from(policy.when_missing),
        rule_file_found,
    )
}

/// The facts behind a diagnosis (the kernel's [`DiagnosisFacts`] trust
/// boundary).
struct DiagnosisProbes<'a> {
    git: &'a dyn GitBackend,
    source_root: &'a Path,
    symlink_policy: SymlinkPolicy,
}

impl DiagnosisFacts<Error> for DiagnosisProbes<'_> {
    fn probe_root_is_symlink(&mut self) -> bool {
        std::fs::symlink_metadata(self.source_root.join(".worktreeinclude"))
            .is_ok_and(|metadata| metadata.file_type().is_symlink())
    }

    fn probe_root_consulted(&mut self) -> bool {
        crate::worktreeinclude::root_rule_file_is_consulted(self.source_root, self.symlink_policy)
    }

    /// This existing metadata-only walk counts symlink entries without
    /// opening their targets. It observes what Ignore omitted; Follow is not
    /// validated.
    fn probe_exists_under_follow(&mut self) -> Result<bool> {
        self.git
            .worktreeinclude_exists_anywhere(self.source_root, SymlinkPolicy::Follow)
    }
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
            "note: no .worktreeinclude is consulted in the repository root; {} semantics ignore nested rule files (inspect them before trying --worktreeinclude-semantics git)",
            policy.semantics.as_str()
        ),
        EmptySelectionCause::RuleFileSymlinkIgnored => eprintln!(
            "note: the active symlink policy skips a .worktreeinclude symlink; inspect its target before trying --worktreeinclude-symlink-policy follow, then run waft validate with that policy"
        ),
        EmptySelectionCause::NestedRuleFileSymlinkIgnored => eprintln!(
            "note: the active symlink policy skips a nested .worktreeinclude symlink, and {} semantics ignore nested rule files; inspect the targets before trying --worktreeinclude-semantics git --worktreeinclude-symlink-policy follow, then run waft validate with those settings",
            policy.semantics.as_str()
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
