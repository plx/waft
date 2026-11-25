//! Verified kernel: source/destination worktree resolution verdicts.
//!
//! # What it decides
//!
//! `waft` resolves every command against a *context*: the source worktree it
//! reads from and, for `copy`, the destination worktree it writes into. The
//! production function `context::resolve_context` asks Git for the worktree
//! topology and the top-level directory of each path involved, interns every
//! path it learns into an identity key (equal keys exactly when the paths
//! are equal under `std::path::Path` equality), and hands those observations
//! to this kernel in two stages:
//!
//! 1. [`select_main`] picks the main worktree record (the first record
//!    flagged as main) or reports that there is none, or that it is bare
//!    while the command is `copy`.
//! 2. [`resolve_roots`] picks the source and destination roots from the
//!    observed top-level directories and applies the safety requirements in
//!    a fixed precedence order, returning either the resolved roots or the
//!    highest-precedence violated requirement.
//!
//! The paths travel through the kernel as an opaque payload `P` next to
//! their key, so the roots production stores are exactly the values Git
//! returned (byte for byte) and the kernel never has to name a path it was
//! not given.
//!
//! The command-line policy inputs (default command, user-config source, and
//! the CLI `replace_extra_excludes` field) live in the [`cli_policy`]
//! submodule.
//!
//! # Theorems
//!
//! - [`select_main`]'s postcondition: the selected record is the *first*
//!   record flagged as main; "no main worktree" is reported exactly when no
//!   record is flagged as main; "bare main" is reported exactly when the
//!   command is `copy` and that first main record is bare
//!   (`lemma_first_main_exists` and `lemma_first_main_unique`: when some
//!   record is flagged as main, exactly one is the first).
//! - [`resolve_roots`] equals `spec_resolve_roots`: the roots are chosen
//!   by `chosen_source`/`chosen_dest`, and the error is the first
//!   violated requirement in `first_violation`'s precedence order.
//! - `theorem_reported_error_is_highest_precedence`: the reported error is
//!   exactly the violated requirement of lowest `precedence` rank, and the
//!   roots resolve exactly when no requirement is violated.
//! - `theorem_copy_dest_is_main_unreachable`: the "copy destination must
//!   be a linked worktree" requirement is never the reported one, for every
//!   input: whenever it is violated, "copy source must be main" or "source
//!   equals destination" is violated first. Production keeps the branch.
//! - `theorem_resolved_dest_is_distinct_family_member`: for every command,
//!   a resolved destination differs from the source and is a member of the
//!   worktree family.
//! - `theorem_copy_resolves_main_to_linked`: a resolved `copy` always has
//!   a destination, its source is the main worktree, and its destination is
//!   a family member other than the main worktree.
//! - `theorem_copy_context_is_safe`: composed with [`select_main`], a
//!   resolved `copy` reads from the first main record, which is not bare,
//!   and writes into a different record of the same `git worktree list`.
//! - `theorem_non_copy_has_no_copy_errors`: `list`, `info` and `validate`
//!   never report a copy-only error.
//! - `theorem_auto_detect_from_linked_worktree` and
//!   `theorem_auto_detect_from_main_worktree`: with no `--source` or
//!   `--dest`, running in a linked worktree of the family resolves to
//!   main → that worktree for every command; running in the main worktree
//!   resolves `copy` to "copy requires a destination" and every other command
//!   to the main worktree with no destination.
//!
//! # Trust assumptions
//!
//! - Production assigns keys so that two keys are equal exactly when the two
//!   paths are equal under `Path` equality (component-wise; an equivalence
//!   relation), which is the comparison the code made before the kernel.
//! - The records are those `git worktree list` returned, in order, and
//!   `main.key` is the key of the record [`select_main`] selected.
//! - The toplevel observations are the results of `show_toplevel` on the
//!   arguments the [`Toplevels`] variant names (production queries Git and
//!   propagates its errors itself, in the same order as before).

#[allow(unused_imports)]
use vstd::prelude::*;

pub mod cli_policy;

verus! {

// ---------------------------------------------------------------------------
// Command and worktree records
// ---------------------------------------------------------------------------
/// The command a context is resolved for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, StructuralEq)]
pub enum CommandKind {
    /// `copy`: requires a destination and writes into it.
    Copy,
    /// `list`.
    List,
    /// `info`.
    Info,
    /// `validate`.
    Validate,
}

/// One `git worktree list` record, reduced to what the decision reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, StructuralEq)]
pub struct WorktreeRecord {
    /// Identity key of the record's path.
    pub key: usize,
    /// Whether Git reports this record as the main worktree.
    pub is_main: bool,
    /// Whether the worktree is bare (has no working tree).
    pub is_bare: bool,
}

/// The outcome of choosing the main worktree record.
#[derive(Clone, Copy, Debug, PartialEq, Eq, StructuralEq)]
pub enum MainSelection {
    /// The index of the main worktree record.
    Main(usize),
    /// No record is flagged as main.
    NoMain,
    /// The command is `copy` and the main worktree is bare.
    BareMainCopy,
}

/// `records[i]` is the first record flagged as main.
pub open spec fn is_first_main(records: Seq<WorktreeRecord>, i: int) -> bool {
    &&& 0 <= i < records.len()
    &&& records[i].is_main
    &&& forall|j: int| 0 <= j < i ==> !(#[trigger] records[j]).is_main
}

/// Some record is flagged as main.
pub open spec fn has_main(records: Seq<WorktreeRecord>) -> bool {
    exists|i: int| 0 <= i < records.len() && (#[trigger] records[i]).is_main
}

/// At most one record is the first main record.
pub proof fn lemma_first_main_unique(records: Seq<WorktreeRecord>, i: int, j: int)
    requires
        is_first_main(records, i),
        is_first_main(records, j),
    ensures
        i == j,
{
    if i < j {
        assert(!records[i].is_main);
    } else if j < i {
        assert(!records[j].is_main);
    }
}

/// If any record is flagged as main, a first one exists.
pub proof fn lemma_first_main_exists(records: Seq<WorktreeRecord>)
    requires
        has_main(records),
    ensures
        exists|i: int| is_first_main(records, i),
    decreases records.len(),
{
    if records[0].is_main {
        assert(is_first_main(records, 0));
    } else {
        let rest = records.drop_first();
        let k = choose|k: int| 0 <= k < records.len() && (#[trigger] records[k]).is_main;
        assert(rest[k - 1] == records[k]);
        assert(has_main(rest));
        lemma_first_main_exists(rest);
        let i = choose|i: int| is_first_main(rest, i);
        assert forall|j: int| 0 <= j < i + 1 implies !(#[trigger] records[j]).is_main by {
            if j > 0 {
                assert(records[j] == rest[j - 1]);
            }
        }
        assert(is_first_main(records, i + 1));
    }
}

/// Choose the main worktree record (`Iterator::find` on `is_main`), then
/// refuse a bare main worktree for `copy`.
pub fn select_main(command: CommandKind, records: &[WorktreeRecord]) -> (r: MainSelection)
    ensures
        r is NoMain <==> !has_main(records@),
        r matches MainSelection::Main(i) ==> is_first_main(records@, i as int) && !(command
            == CommandKind::Copy && records@[i as int].is_bare),
        r is BareMainCopy <==> (command == CommandKind::Copy && exists|i: int|
            #[trigger] is_first_main(records@, i) && records@[i].is_bare),
{
    let mut i: usize = 0;
    while i < records.len()
        invariant
            i <= records@.len(),
            forall|j: int| 0 <= j < i ==> !(#[trigger] records@[j]).is_main,
        decreases records.len() - i,
    {
        if records[i].is_main {
            proof {
                assert(is_first_main(records@, i as int));
                assert forall|k: int| #[trigger] is_first_main(records@, k) implies k == i by {
                    lemma_first_main_unique(records@, k, i as int);
                }
            }
            if records[i].is_bare && command == CommandKind::Copy {
                return MainSelection::BareMainCopy;
            }
            return MainSelection::Main(i);
        }
        i += 1;
    }
    MainSelection::NoMain
}

// ---------------------------------------------------------------------------
// Root selection and requirements
// ---------------------------------------------------------------------------
/// A path production observed, with its identity key.
pub struct Observed<P> {
    /// Identity key (equal keys exactly for equal paths).
    pub key: usize,
    /// The path itself, carried untouched.
    pub path: P,
}

/// The top-level directories production asked Git for, by which arguments
/// were given.
#[allow(inconsistent_fields)]
pub enum Toplevels<P> {
    /// `--source` was given: its toplevel, and `--dest`'s when given.
    Explicit {
        /// Toplevel of `--source`.
        source: Observed<P>,
        /// Toplevel of `--dest`, when given.
        dest: Option<Observed<P>>,
    },
    /// Only `--dest` was given: its toplevel.
    DestOnly {
        /// Toplevel of `--dest`.
        dest: Observed<P>,
    },
    /// Neither was given: the toplevel of the working directory.
    FromCwd {
        /// Toplevel of the working directory.
        cwd: Observed<P>,
    },
}

/// A safety requirement on the chosen roots, named by the error reported
/// when it is violated.
#[derive(Clone, Copy, Debug, PartialEq, Eq, StructuralEq)]
pub enum RootError {
    /// `copy` needs a destination.
    CopyNeedsDest,
    /// `copy` must read from the main worktree.
    CopySourceNotMain,
    /// Source and destination must differ.
    SameSourceAndDest,
    /// `copy` must not write into the main worktree.
    CopyDestIsMain,
    /// The destination must belong to the worktree family.
    NotInFamily,
}

/// The kernel's verdict on the roots.
#[allow(inconsistent_fields)]
pub enum RootsVerdict<P> {
    /// The roots resolved.
    Resolved {
        /// The source root.
        source: Observed<P>,
        /// The destination root, if any.
        dest: Option<Observed<P>>,
    },
    /// See [`RootError::CopyNeedsDest`].
    CopyNeedsDest,
    /// See [`RootError::CopySourceNotMain`]; carries the chosen source.
    CopySourceNotMain {
        /// The chosen source root.
        source: Observed<P>,
    },
    /// See [`RootError::SameSourceAndDest`]; carries the chosen source.
    SameSourceAndDest {
        /// The chosen source root.
        source: Observed<P>,
    },
    /// See [`RootError::CopyDestIsMain`] (never returned; see
    /// `theorem_copy_dest_is_main_unreachable`).
    CopyDestIsMain,
    /// See [`RootError::NotInFamily`]; carries both chosen roots.
    NotInFamily {
        /// The chosen source root.
        source: Observed<P>,
        /// The chosen destination root.
        dest: Observed<P>,
    },
}

/// The source root: `--source`'s toplevel when given; otherwise the main
/// worktree, except that running in the main worktree itself keeps the
/// working directory's own observation (Path-equal to main).
pub open spec fn chosen_source<P>(main: Observed<P>, tops: Toplevels<P>) -> Observed<P> {
    match tops {
        Toplevels::Explicit { source, .. } => source,
        Toplevels::DestOnly { .. } => main,
        Toplevels::FromCwd { cwd } => if cwd.key == main.key {
            cwd
        } else {
            main
        },
    }
}

/// The destination root: `--dest`'s toplevel when given; with neither
/// argument, the working directory's worktree unless it is the main one.
pub open spec fn chosen_dest<P>(main: Observed<P>, tops: Toplevels<P>) -> Option<Observed<P>> {
    match tops {
        Toplevels::Explicit { dest, .. } => dest,
        Toplevels::DestOnly { dest } => Some(dest),
        Toplevels::FromCwd { cwd } => if cwd.key == main.key {
            None
        } else {
            Some(cwd)
        },
    }
}

/// The key of an optional observation.
pub open spec fn key_of<P>(o: Option<Observed<P>>) -> Option<usize> {
    match o {
        Some(x) => Some(x.key),
        None => None,
    }
}

/// Some worktree record has this key.
pub open spec fn in_family(records: Seq<WorktreeRecord>, key: usize) -> bool {
    exists|j: int| 0 <= j < records.len() && (#[trigger] records[j]).key == key
}

/// Whether requirement `e` is violated by the chosen roots.
pub open spec fn violates(
    e: RootError,
    command: CommandKind,
    source: usize,
    dest: Option<usize>,
    main: usize,
    records: Seq<WorktreeRecord>,
) -> bool {
    match e {
        RootError::CopyNeedsDest => command == CommandKind::Copy && dest is None,
        RootError::CopySourceNotMain => command == CommandKind::Copy && source != main,
        RootError::SameSourceAndDest => dest == Some(source),
        RootError::CopyDestIsMain => command == CommandKind::Copy && dest == Some(main),
        RootError::NotInFamily => dest matches Some(d) && !in_family(records, d),
    }
}

/// Precedence rank: when several requirements are violated, the one with the
/// lowest rank is reported (the order the checks have always run in).
pub open spec fn precedence(e: RootError) -> int {
    match e {
        RootError::CopyNeedsDest => 0,
        RootError::CopySourceNotMain => 1,
        RootError::SameSourceAndDest => 2,
        RootError::CopyDestIsMain => 3,
        RootError::NotInFamily => 4,
    }
}

/// The first violated requirement, in precedence order.
pub open spec fn first_violation(
    command: CommandKind,
    source: usize,
    dest: Option<usize>,
    main: usize,
    records: Seq<WorktreeRecord>,
) -> Option<RootError> {
    if violates(RootError::CopyNeedsDest, command, source, dest, main, records) {
        Some(RootError::CopyNeedsDest)
    } else if violates(RootError::CopySourceNotMain, command, source, dest, main, records) {
        Some(RootError::CopySourceNotMain)
    } else if violates(RootError::SameSourceAndDest, command, source, dest, main, records) {
        Some(RootError::SameSourceAndDest)
    } else if violates(RootError::CopyDestIsMain, command, source, dest, main, records) {
        Some(RootError::CopyDestIsMain)
    } else if violates(RootError::NotInFamily, command, source, dest, main, records) {
        Some(RootError::NotInFamily)
    } else {
        None
    }
}

/// The full verdict: roots from `chosen_source`/`chosen_dest`, error
/// from `first_violation`.
pub open spec fn spec_resolve_roots<P>(
    command: CommandKind,
    records: Seq<WorktreeRecord>,
    main: Observed<P>,
    tops: Toplevels<P>,
) -> RootsVerdict<P> {
    let s = chosen_source(main, tops);
    let d = chosen_dest(main, tops);
    match first_violation(command, s.key, key_of(d), main.key, records) {
        None => RootsVerdict::Resolved { source: s, dest: d },
        Some(RootError::CopyNeedsDest) => RootsVerdict::CopyNeedsDest,
        Some(RootError::CopySourceNotMain) => RootsVerdict::CopySourceNotMain { source: s },
        Some(RootError::SameSourceAndDest) => RootsVerdict::SameSourceAndDest { source: s },
        Some(RootError::CopyDestIsMain) => RootsVerdict::CopyDestIsMain,
        Some(RootError::NotInFamily) => RootsVerdict::NotInFamily { source: s, dest: d->Some_0 },
    }
}

/// Whether some record has `key` (`known_worktrees.iter().any(..)`).
fn family_contains(records: &[WorktreeRecord], key: usize) -> (r: bool)
    ensures
        r == in_family(records@, key),
{
    let mut j: usize = 0;
    while j < records.len()
        invariant
            j <= records@.len(),
            forall|k: int| 0 <= k < j ==> (#[trigger] records@[k]).key != key,
        decreases records.len() - j,
    {
        if records[j].key == key {
            return true;
        }
        j += 1;
    }
    false
}

/// Choose the source and destination roots and check them, in the order
/// production always has.
pub fn resolve_roots<P>(
    command: CommandKind,
    records: &[WorktreeRecord],
    main: Observed<P>,
    tops: Toplevels<P>,
) -> (r: RootsVerdict<P>)
    ensures
        r == spec_resolve_roots(command, records@, main, tops),
{
    let is_copy = command == CommandKind::Copy;
    let main_key = main.key;
    let ghost s = chosen_source(main, tops);
    let ghost d = chosen_dest(main, tops);
    let (source, dest) = match tops {
        Toplevels::Explicit { source, dest } => (source, dest),
        Toplevels::DestOnly { dest } => (main, Some(dest)),
        Toplevels::FromCwd { cwd } => {
            if cwd.key == main_key {
                (cwd, None)
            } else {
                (main, Some(cwd))
            }
        },
    };
    assert(source == s && dest == d);
    if is_copy && dest.is_none() {
        return RootsVerdict::CopyNeedsDest;
    }
    if is_copy && source.key != main_key {
        return RootsVerdict::CopySourceNotMain { source };
    }
    match dest {
        None => RootsVerdict::Resolved { source, dest: None },
        Some(dest) => {
            if source.key == dest.key {
                return RootsVerdict::SameSourceAndDest { source };
            }
            if is_copy && dest.key == main_key {
                return RootsVerdict::CopyDestIsMain;
            }
            if !family_contains(records, dest.key) {
                return RootsVerdict::NotInFamily { source, dest };
            }
            RootsVerdict::Resolved { source, dest: Some(dest) }
        },
    }
}

// ---------------------------------------------------------------------------
// Theorems
// ---------------------------------------------------------------------------
/// The reported error is the violated requirement of lowest precedence rank,
/// and nothing is reported exactly when no requirement is violated.
pub proof fn theorem_reported_error_is_highest_precedence(
    command: CommandKind,
    source: usize,
    dest: Option<usize>,
    main: usize,
    records: Seq<WorktreeRecord>,
)
    ensures
        first_violation(command, source, dest, main, records) is None <==> forall|e: RootError|
            !#[trigger] violates(e, command, source, dest, main, records),
        forall|e: RootError|
            first_violation(command, source, dest, main, records) == Some(e) <==> {
                &&& #[trigger] violates(e, command, source, dest, main, records)
                &&& forall|f: RootError|
                    precedence(f) < precedence(e) ==> !#[trigger] violates(
                        f,
                        command,
                        source,
                        dest,
                        main,
                        records,
                    )
            },
{
    let fv = first_violation(command, source, dest, main, records);
    assert forall|e: RootError| fv is None implies !#[trigger] violates(
        e,
        command,
        source,
        dest,
        main,
        records,
    ) by {
        match e {
            RootError::CopyNeedsDest => {},
            RootError::CopySourceNotMain => {},
            RootError::SameSourceAndDest => {},
            RootError::CopyDestIsMain => {},
            RootError::NotInFamily => {},
        }
    }
    assert forall|e: RootError|
        fv == Some(e) <==> {
            &&& #[trigger] violates(e, command, source, dest, main, records)
            &&& forall|f: RootError|
                precedence(f) < precedence(e) ==> !#[trigger] violates(
                    f,
                    command,
                    source,
                    dest,
                    main,
                    records,
                )
        } by {
        if fv == Some(e) {
            assert forall|f: RootError| precedence(f) < precedence(e) implies !#[trigger] violates(
                f,
                command,
                source,
                dest,
                main,
                records,
            ) by {
                match f {
                    RootError::CopyNeedsDest => {},
                    RootError::CopySourceNotMain => {},
                    RootError::SameSourceAndDest => {},
                    RootError::CopyDestIsMain => {},
                    RootError::NotInFamily => {},
                }
            }
        }
        if violates(e, command, source, dest, main, records) && (forall|f: RootError|
            precedence(f) < precedence(e) ==> !#[trigger] violates(
                f,
                command,
                source,
                dest,
                main,
                records,
            )) {
            assert(precedence(RootError::CopyNeedsDest) == 0);
            assert(precedence(RootError::CopySourceNotMain) == 1);
            assert(precedence(RootError::SameSourceAndDest) == 2);
            assert(precedence(RootError::CopyDestIsMain) == 3);
            match e {
                RootError::CopyNeedsDest => {},
                RootError::CopySourceNotMain => {
                    assert(!violates(RootError::CopyNeedsDest, command, source, dest, main, records));
                },
                RootError::SameSourceAndDest => {
                    assert(!violates(RootError::CopyNeedsDest, command, source, dest, main, records));
                    assert(!violates(
                        RootError::CopySourceNotMain,
                        command,
                        source,
                        dest,
                        main,
                        records,
                    ));
                },
                RootError::CopyDestIsMain => {
                    assert(!violates(RootError::CopyNeedsDest, command, source, dest, main, records));
                    assert(!violates(
                        RootError::CopySourceNotMain,
                        command,
                        source,
                        dest,
                        main,
                        records,
                    ));
                    assert(!violates(
                        RootError::SameSourceAndDest,
                        command,
                        source,
                        dest,
                        main,
                        records,
                    ));
                },
                RootError::NotInFamily => {
                    assert(!violates(RootError::CopyNeedsDest, command, source, dest, main, records));
                    assert(!violates(
                        RootError::CopySourceNotMain,
                        command,
                        source,
                        dest,
                        main,
                        records,
                    ));
                    assert(!violates(
                        RootError::SameSourceAndDest,
                        command,
                        source,
                        dest,
                        main,
                        records,
                    ));
                    assert(!violates(
                        RootError::CopyDestIsMain,
                        command,
                        source,
                        dest,
                        main,
                        records,
                    ));
                },
            }
        }
    }
}

/// "Copy destination must be a linked worktree" is never the reported error:
/// whenever it is violated, an earlier requirement is violated too.
pub proof fn theorem_copy_dest_is_main_unreachable<P>(
    command: CommandKind,
    records: Seq<WorktreeRecord>,
    main: Observed<P>,
    tops: Toplevels<P>,
)
    ensures
        forall|source: usize, dest: Option<usize>|
            #[trigger] first_violation(command, source, dest, main.key, records) != Some(
                RootError::CopyDestIsMain,
            ),
        !(spec_resolve_roots(command, records, main, tops) is CopyDestIsMain),
{
}

/// For every command, a resolved destination differs from the source and
/// belongs to the worktree family.
pub proof fn theorem_resolved_dest_is_distinct_family_member<P>(
    command: CommandKind,
    records: Seq<WorktreeRecord>,
    main: Observed<P>,
    tops: Toplevels<P>,
)
    ensures
        spec_resolve_roots(command, records, main, tops) matches RootsVerdict::Resolved {
            source,
            dest: Some(dest),
        } ==> dest.key != source.key && in_family(records, dest.key),
{
}

/// A resolved `copy` has a destination; its source is the main worktree and
/// its destination is a family member other than the main worktree.
pub proof fn theorem_copy_resolves_main_to_linked<P>(
    records: Seq<WorktreeRecord>,
    main: Observed<P>,
    tops: Toplevels<P>,
)
    ensures
        spec_resolve_roots(CommandKind::Copy, records, main, tops) matches RootsVerdict::Resolved {
            source,
            dest,
        } ==> {
            &&& source.key == main.key
            &&& dest matches Some(d)
            &&& d.key != main.key
            &&& in_family(records, d.key)
        },
{
}

/// End to end with [`select_main`]: a resolved `copy` reads from the first
/// record flagged as main, which is not bare, and writes into a different
/// record of the same worktree list.
pub proof fn theorem_copy_context_is_safe<P>(
    records: Seq<WorktreeRecord>,
    i: int,
    main: Observed<P>,
    tops: Toplevels<P>,
)
    requires
        is_first_main(records, i),
        !records[i].is_bare,
        main.key == records[i].key,
    ensures
        spec_resolve_roots(CommandKind::Copy, records, main, tops) matches RootsVerdict::Resolved {
            source,
            dest,
        } ==> {
            &&& source.key == records[i].key
            &&& records[i].is_main
            &&& !records[i].is_bare
            &&& dest matches Some(d)
            &&& exists|j: int|
                0 <= j < records.len() && j != i && (#[trigger] records[j]).key == d.key
        },
{
    theorem_copy_resolves_main_to_linked(records, main, tops);
    match spec_resolve_roots(CommandKind::Copy, records, main, tops) {
        RootsVerdict::Resolved { dest: Some(d), .. } => {
            let j = choose|j: int| 0 <= j < records.len() && (#[trigger] records[j]).key == d.key;
            assert(j != i);
        },
        _ => {},
    }
}

/// `list`, `info` and `validate` never report a copy-only error.
pub proof fn theorem_non_copy_has_no_copy_errors<P>(
    command: CommandKind,
    records: Seq<WorktreeRecord>,
    main: Observed<P>,
    tops: Toplevels<P>,
)
    requires
        command != CommandKind::Copy,
    ensures
        !(spec_resolve_roots(command, records, main, tops) is CopyNeedsDest),
        !(spec_resolve_roots(command, records, main, tops) is CopySourceNotMain),
        !(spec_resolve_roots(command, records, main, tops) is CopyDestIsMain),
{
}

/// With no `--source`/`--dest`, running inside a linked worktree of the
/// family resolves to "main → that worktree" for every command (in
/// particular, bare `waft` in a linked worktree copies into it).
pub proof fn theorem_auto_detect_from_linked_worktree<P>(
    command: CommandKind,
    records: Seq<WorktreeRecord>,
    main: Observed<P>,
    cwd: Observed<P>,
)
    requires
        cwd.key != main.key,
        in_family(records, cwd.key),
    ensures
        spec_resolve_roots(command, records, main, Toplevels::FromCwd { cwd }) == (
        RootsVerdict::Resolved { source: main, dest: Some(cwd) }),
{
}

/// With no `--source`/`--dest`, running inside the main worktree makes
/// `copy` ask for a destination and resolves every other command to the main
/// worktree (as observed from the working directory) with no destination.
pub proof fn theorem_auto_detect_from_main_worktree<P>(
    command: CommandKind,
    records: Seq<WorktreeRecord>,
    main: Observed<P>,
    cwd: Observed<P>,
)
    requires
        cwd.key == main.key,
    ensures
        spec_resolve_roots(command, records, main, Toplevels::FromCwd { cwd }) == (if command
            == CommandKind::Copy {
            RootsVerdict::CopyNeedsDest
        } else {
            RootsVerdict::Resolved { source: cwd, dest: None }
        }),
{
}

} // verus!
