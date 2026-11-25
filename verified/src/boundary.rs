//! Verified kernel: repository-boundary decisions for the source walkers.
//!
//! # What it decides
//!
//! - Whether a directory entry is one of waft's control files (`.git`,
//!   `.worktreeinclude`, `.gitignore`): an exact byte match, or on platforms
//!   whose filesystems may alias letter case (macOS, Windows) an ASCII
//!   case-insensitive match ([`special_name_matches`]).
//! - Whether a gitlink comparison folds case ([`boundary_case_folds`]).
//! - Whether a walker must not descend into a directory
//!   ([`is_git_boundary`]): the entry is itself named `.git`; or, below the
//!   walk root, its `.git` is a directory, or a gitfile whose target exists,
//!   or its repository path is a registered gitlink. The filesystem and
//!   gitlink questions are asked through the [`BoundaryProbes`] oracle, in
//!   that order, and only until one answers yes.
//! - Which target a `.git` gitfile names ([`gitfile`] submodule).
//!
//! # Theorems
//!
//! - [`special_name_matches`] equals `special_name_spec`.
//!   `lemma_special_name_reflexive`: a name always matches itself.
//!   `lemma_special_name_exact_off_aliasing_platforms`: off aliasing
//!   platforms a match is byte equality.
//!   `lemma_special_name_monotone`: every match without aliasing is also
//!   a match with it (the aliasing rule only adds matches).
//!   `lemma_ascii_ci_match_is_ascii`: an ASCII case-insensitive match of an
//!   ASCII name is itself ASCII, hence valid UTF-8; so the byte-level kernel
//!   agrees with the former `OsStr::to_str` + `str::eq_ignore_ascii_case`
//!   test, which needed the name to be Unicode first.
//! - [`boundary_case_folds`]'s postcondition: folds exactly when the
//!   checkout folds case or the platform aliases case, so it is never less
//!   conservative than the checkout's own setting.
//! - [`is_git_boundary`]'s postcondition, and `theorem_boundary_decision`:
//!   an entry named `.git` is pruned with no probe; otherwise the walk root
//!   (depth 0) is kept with no probe; otherwise the probes issued are a
//!   prefix of `probe_plan` that stops right after the first "yes" (or
//!   after the plan), and the directory is pruned exactly when some issued
//!   probe answered yes. The gitlink probe, when issued, folds case exactly
//!   as [`boundary_case_folds`] decides, so on aliasing platforms it always
//!   folds.
//!
//! # Trust assumptions
//!
//! - Production passes `OsStr::as_encoded_bytes` of the entry's file name
//!   (the raw bytes on Unix, WTF-8 on Windows, whose equality is `OsStr`
//!   equality) and the native `aliasing_platform` flag.
//! - The [`BoundaryProbes`] implementation answers each probe from the
//!   filesystem and the gitlink set at the time it is asked (the answers are
//!   trusted; the kernel only decides which probes to ask, in which order,
//!   and how to combine the answers).
//! - The walker supplies the entry's depth below the walk root.

#[allow(unused_imports)]
use vstd::prelude::*;

pub mod gitfile;

verus! {

// ---------------------------------------------------------------------------
// Control-file names
// ---------------------------------------------------------------------------
/// ASCII lowercase of one byte (`u8::to_ascii_lowercase`).
pub open spec fn ascii_lower(b: u8) -> u8 {
    if b'A' <= b <= b'Z' {
        (b + 32) as u8
    } else {
        b
    }
}

/// ASCII case-insensitive equality (`<[u8]>::eq_ignore_ascii_case`).
pub open spec fn ascii_ci_eq(a: Seq<u8>, b: Seq<u8>) -> bool {
    &&& a.len() == b.len()
    &&& forall|i: int| 0 <= i < a.len() ==> #[trigger] ascii_lower(a[i]) == ascii_lower(b[i])
}

/// Every byte is ASCII.
pub open spec fn is_ascii_bytes(s: Seq<u8>) -> bool {
    forall|i: int| 0 <= i < s.len() ==> #[trigger] s[i] < 0x80
}

/// The control-file name rule: exact bytes, or ASCII case-insensitive on a
/// platform whose filesystems may alias case.
pub open spec fn special_name_spec(actual: Seq<u8>, expected: Seq<u8>, aliasing_platform: bool) -> bool {
    actual == expected || (aliasing_platform && ascii_ci_eq(actual, expected))
}

fn ascii_lower_exec(b: u8) -> (r: u8)
    ensures
        r == ascii_lower(b),
{
    match b {
        b'A'..=b'Z' => b + 32,
        _ => b,
    }
}

/// Whether the entry name `actual` is the control file `expected`.
pub fn special_name_matches(actual: &[u8], expected: &[u8], aliasing_platform: bool) -> (r: bool)
    ensures
        r == special_name_spec(actual@, expected@, aliasing_platform),
{
    if actual == expected {
        assert(actual@ =~= expected@);
        return true;
    }
    if !aliasing_platform || actual.len() != expected.len() {
        return false;
    }
    let mut i: usize = 0;
    while i < actual.len()
        invariant
            actual@.len() == expected@.len(),
            i <= actual@.len(),
            forall|k: int| 0 <= k < i ==> #[trigger] ascii_lower(actual@[k]) == ascii_lower(
                expected@[k],
            ),
        decreases actual.len() - i,
    {
        if ascii_lower_exec(actual[i]) != ascii_lower_exec(expected[i]) {
            return false;
        }
        i += 1;
    }
    true
}

/// A name always matches itself, on every platform.
pub proof fn lemma_special_name_reflexive(name: Seq<u8>, aliasing_platform: bool)
    ensures
        special_name_spec(name, name, aliasing_platform),
{
}

/// Off aliasing platforms a control-file match is byte equality.
pub proof fn lemma_special_name_exact_off_aliasing_platforms(actual: Seq<u8>, expected: Seq<u8>)
    ensures
        special_name_spec(actual, expected, false) <==> actual == expected,
{
}

/// The aliasing rule only adds matches.
pub proof fn lemma_special_name_monotone(actual: Seq<u8>, expected: Seq<u8>)
    ensures
        special_name_spec(actual, expected, false) ==> special_name_spec(actual, expected, true),
{
}

/// An ASCII case-insensitive match of an ASCII name is all ASCII (so it is
/// valid UTF-8, and the former `to_str` gate never rejected a match).
pub proof fn lemma_ascii_ci_match_is_ascii(actual: Seq<u8>, expected: Seq<u8>)
    requires
        is_ascii_bytes(expected),
        ascii_ci_eq(actual, expected),
    ensures
        is_ascii_bytes(actual),
{
    assert forall|i: int| 0 <= i < actual.len() implies #[trigger] actual[i] < 0x80 by {
        assert(ascii_lower(actual[i]) == ascii_lower(expected[i]));
        assert(expected[i] < 0x80);
    }
}

/// Whether a repository-boundary (gitlink) comparison folds case: when the
/// checkout folds case, or the platform may alias case regardless.
pub fn boundary_case_folds(ignore_case: bool, aliasing_platform: bool) -> (r: bool)
    ensures
        r == (ignore_case || aliasing_platform),
{
    ignore_case || aliasing_platform
}

// ---------------------------------------------------------------------------
// Boundary decision
// ---------------------------------------------------------------------------
/// A question the boundary decision may ask about a directory.
#[derive(Clone, Copy, Debug, PartialEq, Eq, StructuralEq)]
pub enum BoundaryProbe {
    /// Is `<dir>/.git` a directory?
    DotGitIsDir,
    /// Is `<dir>/.git` a file whose `gitdir:` pointer names an existing path?
    DotGitFileTargetExists,
    /// Is the directory's repository path a registered gitlink, comparing
    /// with case folding when `fold` is set?
    GitlinkMatches {
        /// Whether to compare with case folding.
        fold: bool,
    },
}

/// The production side of the boundary decision: answers probes.
///
/// `answered` is a ghost log of every probe asked so far with the answer
/// given; it exists only for the specification.
pub trait BoundaryProbes {
    /// Every probe answered so far, oldest first, with its answer.
    spec fn answered(&self) -> Seq<(BoundaryProbe, bool)>;

    /// Answer one probe. Trusted: production answers from the filesystem
    /// and the gitlink set; the kernel assumes only that the call is logged.
    fn probe(&mut self, probe: BoundaryProbe) -> (answer: bool)
        ensures
            final(self).answered() == old(self).answered().push((probe, answer)),
    ;
}

/// `.git` as bytes.
pub open spec fn dot_git() -> Seq<u8> {
    seq![b'.', b'g', b'i', b't']
}

/// The entry is named `.git` (under the control-file name rule).
pub open spec fn named_dot_git(name: Option<Seq<u8>>, aliasing_platform: bool) -> bool {
    name matches Some(n) && special_name_spec(n, dot_git(), aliasing_platform)
}

/// The probes, in the order they are asked below the walk root.
pub open spec fn probe_plan(ignore_case: bool, aliasing_platform: bool) -> Seq<BoundaryProbe> {
    seq![
        BoundaryProbe::DotGitIsDir,
        BoundaryProbe::DotGitFileTargetExists,
        BoundaryProbe::GitlinkMatches { fold: ignore_case || aliasing_platform },
    ]
}

/// `issued` asks the probes of `plan` in order, stopping right after the
/// first "yes" or at the end of the plan (lazy disjunction).
pub open spec fn lazy_prefix(issued: Seq<(BoundaryProbe, bool)>, plan: Seq<BoundaryProbe>) -> bool {
    &&& 0 < issued.len() <= plan.len()
    &&& forall|k: int| 0 <= k < issued.len() ==> (#[trigger] issued[k]).0 == plan[k]
    &&& forall|k: int| 0 <= k < issued.len() - 1 ==> !(#[trigger] issued[k]).1
    &&& issued.len() < plan.len() ==> issued.last().1
}

/// Some issued probe answered yes.
pub open spec fn any_yes(issued: Seq<(BoundaryProbe, bool)>) -> bool {
    exists|k: int| 0 <= k < issued.len() && (#[trigger] issued[k]).1
}

/// The decision, given the probes issued (`issued`) and their answers.
pub open spec fn boundary_spec(
    name: Option<Seq<u8>>,
    depth: usize,
    ignore_case: bool,
    aliasing_platform: bool,
    issued: Seq<(BoundaryProbe, bool)>,
    r: bool,
) -> bool {
    if named_dot_git(name, aliasing_platform) {
        issued.len() == 0 && r
    } else if depth == 0 {
        issued.len() == 0 && !r
    } else {
        lazy_prefix(issued, probe_plan(ignore_case, aliasing_platform)) && r == any_yes(issued)
    }
}

/// The view of an optional name.
pub open spec fn name_view(name: Option<&[u8]>) -> Option<Seq<u8>> {
    match name {
        Some(n) => Some(n@),
        None => None,
    }
}

/// Whether a walker must not descend into the directory whose file name is
/// `name`, at `depth` below the walk root.
pub fn is_git_boundary<O: BoundaryProbes>(
    name: Option<&[u8]>,
    depth: usize,
    ignore_case: bool,
    aliasing_platform: bool,
    probes: &mut O,
) -> (r: bool)
    ensures
        final(probes).answered().len() >= old(probes).answered().len(),
        final(probes).answered().subrange(0, old(probes).answered().len() as int) == old(
            probes,
        ).answered(),
        boundary_spec(
            name_view(name),
            depth,
            ignore_case,
            aliasing_platform,
            final(probes).answered().subrange(
                old(probes).answered().len() as int,
                final(probes).answered().len() as int,
            ),
            r,
        ),
{
    let ghost start = probes.answered();
    let ghost n = start.len() as int;
    let ghost plan = probe_plan(ignore_case, aliasing_platform);
    let dot_git_bytes: [u8; 4] = [b'.', b'g', b'i', b't'];
    assert(dot_git_bytes@ =~= dot_git());
    let named = match name {
        Some(n) => special_name_matches(n, dot_git_bytes.as_slice(), aliasing_platform),
        None => false,
    };
    if named {
        assert(probes.answered().subrange(n, n) =~= Seq::<(BoundaryProbe, bool)>::empty());
        assert(probes.answered().subrange(0, n) =~= start);
        return true;
    }
    if depth == 0 {
        assert(probes.answered().subrange(n, n) =~= Seq::<(BoundaryProbe, bool)>::empty());
        assert(probes.answered().subrange(0, n) =~= start);
        return false;
    }
    let is_dir = probes.probe(BoundaryProbe::DotGitIsDir);
    let ghost after1 = probes.answered();
    assert(after1.subrange(n, n + 1) =~= seq![(BoundaryProbe::DotGitIsDir, is_dir)]);
    assert(after1.subrange(0, n) =~= start);
    if is_dir {
        assert(any_yes(after1.subrange(n, n + 1))) by {
            assert(after1.subrange(n, n + 1)[0].1);
        }
        return true;
    }
    let pointer = probes.probe(BoundaryProbe::DotGitFileTargetExists);
    let ghost after2 = probes.answered();
    assert(after2.subrange(n, n + 2) =~= seq![
        (BoundaryProbe::DotGitIsDir, is_dir),
        (BoundaryProbe::DotGitFileTargetExists, pointer),
    ]);
    assert(after2.subrange(0, n) =~= start);
    if pointer {
        assert(any_yes(after2.subrange(n, n + 2))) by {
            assert(after2.subrange(n, n + 2)[1].1);
        }
        return true;
    }
    let fold = boundary_case_folds(ignore_case, aliasing_platform);
    let gitlink = probes.probe(BoundaryProbe::GitlinkMatches { fold });
    let ghost after3 = probes.answered();
    let ghost issued = after3.subrange(n, n + 3);
    assert(issued =~= seq![
        (BoundaryProbe::DotGitIsDir, is_dir),
        (BoundaryProbe::DotGitFileTargetExists, pointer),
        (BoundaryProbe::GitlinkMatches { fold }, gitlink),
    ]);
    assert(after3.subrange(0, n) =~= start);
    assert(any_yes(issued) == gitlink) by {
        if gitlink {
            assert(issued[2].1);
        }
        if any_yes(issued) {
            let k = choose|k: int| 0 <= k < issued.len() && (#[trigger] issued[k]).1;
            assert(k == 2);
        }
    }
    gitlink
}

/// The boundary decision in closed form: pruned exactly when named `.git`,
/// or below the root with some issued probe answering yes; no probe is ever
/// asked for an entry named `.git` or for the walk root; probes follow
/// `probe_plan` and stop at the first yes; and the gitlink probe folds
/// case whenever the checkout or the platform does.
pub proof fn theorem_boundary_decision(
    name: Option<Seq<u8>>,
    depth: usize,
    ignore_case: bool,
    aliasing_platform: bool,
    issued: Seq<(BoundaryProbe, bool)>,
    r: bool,
)
    requires
        boundary_spec(name, depth, ignore_case, aliasing_platform, issued, r),
    ensures
        r == (named_dot_git(name, aliasing_platform) || (depth > 0 && any_yes(issued))),
        (named_dot_git(name, aliasing_platform) || depth == 0) ==> issued.len() == 0,
        forall|k: int|
            0 <= k < issued.len() ==> (#[trigger] issued[k]).0 == probe_plan(
                ignore_case,
                aliasing_platform,
            )[k],
        forall|k: int| 0 <= k < issued.len() - 1 ==> !(#[trigger] issued[k]).1,
        forall|k: int|
            0 <= k < issued.len() && (#[trigger] issued[k]).0 is GitlinkMatches ==> issued[k].0
                == (BoundaryProbe::GitlinkMatches { fold: ignore_case || aliasing_platform }),
        aliasing_platform ==> forall|k: int|
            0 <= k < issued.len() && (#[trigger] issued[k]).0 is GitlinkMatches
                ==> issued[k].0 == (BoundaryProbe::GitlinkMatches { fold: true }),
{
    let plan = probe_plan(ignore_case, aliasing_platform);
    if !named_dot_git(name, aliasing_platform) && depth != 0 {
        assert forall|k: int|
            0 <= k < issued.len() && (#[trigger] issued[k]).0 is GitlinkMatches implies issued[k].0
                == (BoundaryProbe::GitlinkMatches { fold: ignore_case || aliasing_platform }) by {
            assert(issued[k].0 == plan[k]);
        }
    }
}

} // verus!
