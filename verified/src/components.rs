//! Verified kernel: anchored relative-path component splitting.
//!
//! waft never resolves a destination or source pathname as a whole. On Unix
//! it opens each directory component with `openat(O_DIRECTORY | O_NOFOLLOW)`
//! from a root handle, so a renamed or symlinked ancestor cannot redirect the
//! publication. Which byte strings become `openat` names, and which paths are
//! refused before any traversal, is decided here:
//!
//! - [`relative_components`]: the names `relative_components` (production
//!   `src/fs.rs`) walks under a repository root, or why the path is refused.
//! - [`absolute_walk`]: what `open_canonical_directory` does with a canonical
//!   repository root: the names it opens below `/`, in order, and whether it
//!   then stops at a refused component.
//!
//! # Model of `std::path::Path::components` (trusted)
//!
//! Production used to iterate `Path::components()`. The kernel works on the
//! path's bytes, so `std_components` transcribes the documented Unix
//! behaviour of that iterator: a leading `/` yields `RootDir`; otherwise a
//! leading `.` that is the whole path or is followed by `/` yields `CurDir`;
//! the rest is split on `/`, empty and `.` pieces are dropped, `..` yields
//! `ParentDir`, and every other piece is a `Normal` name. The production
//! loops are then mirrored over that component list (`relative_loop`,
//! `absolute_loop`). A differential test in production compares the kernel
//! with `Path::components` on generated inputs.
//!
//! # Theorems
//!
//! - `relative_components` returns exactly `spec_relative_components` and
//!   `absolute_walk` exactly `spec_absolute_walk` (functional correctness
//!   against the mirrored production loops).
//! - `theorem_relative_names_are_safe`: an accepted relative path yields at
//!   least one name, and every name is non-empty, contains no `/`, and is
//!   neither `.` nor `..` — exactly the precondition for a per-component
//!   `openat` walk that cannot leave the root handle by name.
//! - `theorem_relative_classification`: the decision restated on bytes. The
//!   path is refused as unsafe iff it starts with `/`, is `.` or starts with
//!   `./`, or has a `/`-separated segment equal to `..`; otherwise it is
//!   refused as empty iff every segment is empty or `.`; otherwise the names
//!   are exactly the segments that are neither empty nor `.`, in order.
//! - `theorem_relative_round_trip`: for a canonical path (every segment
//!   non-empty and neither `.` nor `..`) the names are exactly its segments,
//!   and joining them with `/` gives back the input byte for byte.
//! - `theorem_absolute_walk`: a path that does not start with `/` is refused
//!   before any traversal; otherwise the walk is refused iff some segment is
//!   `..`, and it opens exactly the segments that are neither empty nor `.`
//!   before the first `..` (all of them when there is none), every one of
//!   them a safe name.
//! - `example_normalisation_quirks`: `a//b`, `a/./b`, `a/b/` and `a/b/.` are
//!   all accepted as `[a, b]` — the silent normalisation `Path::components`
//!   performs. This is current behaviour, kept on purpose and documented
//!   here. Git does not record such paths; only a `RepoRelPath` built with
//!   `from_normalized` or `from_git_bytes` from other input could carry one.
//!
//! # Trust assumptions
//!
//! - `std_components` matches `std::path::Path::components` on Unix (no
//!   path prefixes; `/` is the only separator).
//! - On Unix an `OsStr` is its bytes: production passes
//!   `OsStr::as_bytes`/`str::as_bytes` and turns each returned name back into
//!   an `OsString` with `OsString::from_vec`.

#[allow(unused_imports)]
use vstd::prelude::*;

verus! {

broadcast use vstd::seq_lib::group_seq_properties;

/// The byte `/`.
pub open spec fn slash() -> u8 {
    0x2f
}

/// The byte `.`.
pub open spec fn dot() -> u8 {
    0x2e
}

/// The one-byte name `.`.
pub open spec fn dot_name() -> Seq<u8> {
    seq![dot()]
}

/// The two-byte name `..`.
pub open spec fn dot_dot_name() -> Seq<u8> {
    seq![dot(), dot()]
}

/// The `/`-separated pieces of `path`, empty pieces included (the pieces
/// `str::split('/')` yields).
pub open spec fn split_segments(path: Seq<u8>) -> Seq<Seq<u8>>
    decreases path.len(),
{
    if path.len() == 0 {
        seq![Seq::<u8>::empty()]
    } else {
        let prior = split_segments(path.drop_last());
        if path.last() == slash() {
            prior.push(Seq::<u8>::empty())
        } else {
            prior.update(prior.len() - 1, prior.last().push(path.last()))
        }
    }
}

/// Segments joined back together with `/`.
pub open spec fn join_segments(segments: Seq<Seq<u8>>) -> Seq<u8>
    decreases segments.len(),
{
    if segments.len() <= 1 {
        if segments.len() == 0 {
            Seq::<u8>::empty()
        } else {
            segments[0]
        }
    } else {
        join_segments(segments.drop_last()) + seq![slash()] + segments.last()
    }
}

/// A piece `Path::components` drops: empty (repeated or trailing `/`) or `.`.
pub open spec fn is_skipped(segment: Seq<u8>) -> bool {
    segment.len() == 0 || segment == dot_name()
}

/// A piece `Path::components` keeps as a name.
pub open spec fn is_name(segment: Seq<u8>) -> bool {
    !is_skipped(segment) && segment != dot_dot_name()
}

/// A name that is safe to pass to `openat` as one directory step.
pub open spec fn is_safe_name(name: Seq<u8>) -> bool {
    &&& name.len() > 0
    &&& !name.contains(slash())
    &&& name != dot_name()
    &&& name != dot_dot_name()
}

/// One `std::path::Component` as Unix `Path::components` yields it.
pub ghost enum StdComponent {
    /// `Component::RootDir`.
    RootDir,
    /// `Component::CurDir`.
    CurDir,
    /// `Component::ParentDir`.
    ParentDir,
    /// `Component::Normal(name)`.
    Normal(Seq<u8>),
}

/// The path starts with `/` (Unix `Path::has_root`).
pub open spec fn has_root(path: Seq<u8>) -> bool {
    path.len() > 0 && path[0] == slash()
}

/// The path is `.` or starts with `./` (std's `include_cur_dir` without a
/// root).
pub open spec fn starts_with_cur_dir(path: Seq<u8>) -> bool {
    &&& !has_root(path)
    &&& path.len() > 0
    &&& path[0] == dot()
    &&& (path.len() == 1 || path[1] == slash())
}

/// The components std yields for the `/`-separated pieces after any leading
/// root or `.`: skipped pieces vanish, `..` is `ParentDir`, the rest are
/// names.
pub open spec fn body_components(segments: Seq<Seq<u8>>) -> Seq<StdComponent>
    decreases segments.len(),
{
    if segments.len() == 0 {
        Seq::empty()
    } else {
        let prior = body_components(segments.drop_last());
        let last = segments.last();
        if is_skipped(last) {
            prior
        } else if last == dot_dot_name() {
            prior.push(StdComponent::ParentDir)
        } else {
            prior.push(StdComponent::Normal(last))
        }
    }
}

/// Model of Unix `Path::new(path).components()` (trusted; see the module
/// documentation).
pub open spec fn std_components(path: Seq<u8>) -> Seq<StdComponent> {
    let lead = if has_root(path) {
        seq![StdComponent::RootDir]
    } else if starts_with_cur_dir(path) {
        seq![StdComponent::CurDir]
    } else {
        Seq::<StdComponent>::empty()
    };
    lead + body_components(split_segments(path))
}

/// Why a repository-relative path cannot be walked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RelativeError {
    /// It contains a root, a leading `.`, or a `..` component
    /// ("repository-relative path contains an unsafe component").
    Unsafe,
    /// It has no name components at all ("repository-relative path is
    /// empty").
    Empty,
}

/// Production's `relative_components` loop over std's components: collect
/// `Normal` names, stop with `None` at the first other component.
pub open spec fn relative_loop(components: Seq<StdComponent>) -> Option<Seq<Seq<u8>>>
    decreases components.len(),
{
    if components.len() == 0 {
        Some(Seq::empty())
    } else {
        match relative_loop(components.drop_last()) {
            None => None,
            Some(names) => match components.last() {
                StdComponent::Normal(name) => Some(names.push(name)),
                _ => None,
            },
        }
    }
}

/// What production's `relative_components` returns for `path`: the unsafe
/// error at the first non-name component, the empty error when no name
/// remains, and otherwise the names in order.
pub open spec fn spec_relative_components(path: Seq<u8>) -> Result<Seq<Seq<u8>>, RelativeError> {
    match relative_loop(std_components(path)) {
        None => Err(RelativeError::Unsafe),
        Some(names) => if names.len() == 0 {
            Err(RelativeError::Empty)
        } else {
            Ok(names)
        },
    }
}

/// Production's `open_canonical_directory` loop over std's components:
/// `RootDir` is skipped, each `Normal` name is opened, and any other
/// component stops the walk with an error. Returns the names opened and
/// whether the walk stopped at a refused component.
pub open spec fn absolute_loop(components: Seq<StdComponent>) -> (Seq<Seq<u8>>, bool)
    decreases components.len(),
{
    if components.len() == 0 {
        (Seq::empty(), false)
    } else {
        let (names, refused) = absolute_loop(components.drop_last());
        if refused {
            (names, true)
        } else {
            match components.last() {
                StdComponent::RootDir => (names, false),
                StdComponent::Normal(name) => (names.push(name), false),
                _ => (names, true),
            }
        }
    }
}

/// What `open_canonical_directory` does with `path`: `None` when the path is
/// not absolute (refused before any traversal); otherwise the names opened
/// below `/` and whether it then stops at a refused component.
pub open spec fn spec_absolute_walk(path: Seq<u8>) -> Option<(Seq<Seq<u8>>, bool)> {
    if has_root(path) {
        Some(absolute_loop(std_components(path)))
    } else {
        None
    }
}

/// The pieces of `segments` that are neither empty nor `.`, in order.
pub open spec fn kept_segments(segments: Seq<Seq<u8>>) -> Seq<Seq<u8>>
    decreases segments.len(),
{
    if segments.len() == 0 {
        Seq::empty()
    } else if is_skipped(segments.last()) {
        kept_segments(segments.drop_last())
    } else {
        kept_segments(segments.drop_last()).push(segments.last())
    }
}

/// Some piece is exactly `..`.
pub open spec fn has_dot_dot(segments: Seq<Seq<u8>>) -> bool {
    exists|i: int| 0 <= i < segments.len() && #[trigger] segments[i] == dot_dot_name()
}

/// Scan of the pieces: the names before the first `..`, and whether a `..`
/// occurs. Both production loops reduce to this.
pub open spec fn segment_scan(segments: Seq<Seq<u8>>) -> (Seq<Seq<u8>>, bool)
    decreases segments.len(),
{
    if segments.len() == 0 {
        (Seq::empty(), false)
    } else {
        let (names, found) = segment_scan(segments.drop_last());
        let last = segments.last();
        if found {
            (names, true)
        } else if is_skipped(last) {
            (names, false)
        } else if last == dot_dot_name() {
            (names, true)
        } else {
            (names.push(last), false)
        }
    }
}

/// The byte views of a list of owned names.
pub open spec fn names_view(names: Seq<Vec<u8>>) -> Seq<Seq<u8>> {
    names.map_values(|name: Vec<u8>| name@)
}

// ----------------------------------------------------------------- lemmas

/// `split_segments` always yields at least one piece and no piece contains
/// `/`.
pub proof fn lemma_split_shape(path: Seq<u8>)
    ensures
        split_segments(path).len() >= 1,
        forall|i: int|
            0 <= i < split_segments(path).len() ==> !(#[trigger] split_segments(path)[i]).contains(
                slash(),
            ),
    decreases path.len(),
{
    if path.len() > 0 {
        lemma_split_shape(path.drop_last());
        let prior = split_segments(path.drop_last());
        let s = split_segments(path);
        if path.last() != slash() {
            assert forall|i: int| 0 <= i < s.len() implies !(#[trigger] s[i]).contains(slash()) by {
                if i == s.len() - 1 {
                    let seg = prior.last().push(path.last());
                    assert(s[i] == seg);
                    assert forall|j: int| 0 <= j < seg.len() implies seg[j] != slash() by {
                        if j < prior.last().len() {
                            assert(seg[j] == prior.last()[j]);
                            assert(prior.last() == prior[prior.len() - 1]);
                        }
                    }
                } else {
                    assert(s[i] == prior[i]);
                }
            }
        } else {
            assert forall|i: int| 0 <= i < s.len() implies !(#[trigger] s[i]).contains(slash()) by {
                if i < prior.len() {
                    assert(s[i] == prior[i]);
                }
            }
        }
    }
}

/// Joining the pieces of a path gives back the path.
pub proof fn lemma_join_split(path: Seq<u8>)
    ensures
        join_segments(split_segments(path)) == path,
    decreases path.len(),
{
    if path.len() > 0 {
        let prior = split_segments(path.drop_last());
        lemma_join_split(path.drop_last());
        lemma_split_shape(path.drop_last());
        let s = split_segments(path);
        if path.last() == slash() {
            assert(s.drop_last() =~= prior);
            assert(path =~= path.drop_last() + seq![slash()] + Seq::<u8>::empty());
        } else {
            assert(s.drop_last() =~= prior.drop_last());
            assert(path =~= path.drop_last().push(path.last()));
            if prior.len() == 1 {
                assert(path =~= prior[0].push(path.last()));
            } else {
                let a = join_segments(prior.drop_last()) + seq![slash()];
                assert(join_segments(prior) == a + prior.last());
                assert(s.len() >= 2);
                assert(s.last() == prior.last().push(path.last()));
                assert(join_segments(s) == join_segments(s.drop_last()) + seq![slash()] + s.last());
                assert((a + prior.last()).push(path.last()) =~= a + prior.last().push(path.last()));
            }
        }
    }
}

/// `split_segments(p ++ q)` where `q` has no `/` extends the last piece.
pub proof fn lemma_split_no_slash_suffix(path: Seq<u8>, start: int)
    requires
        0 <= start <= path.len(),
        start == 0 || path[start - 1] == slash(),
        forall|j: int| start <= j < path.len() ==> path[j] != slash(),
    ensures
        split_segments(path) == split_segments(path.subrange(0, start)).drop_last().push(
            path.subrange(start, path.len() as int),
        ),
    decreases path.len() - start,
{
    lemma_split_shape(path);
    if path.len() == start {
        if start == 0 {
            assert(path =~= Seq::<u8>::empty());
            assert(path.subrange(0, 0) =~= Seq::<u8>::empty());
            assert(path.subrange(start, path.len() as int) =~= Seq::<u8>::empty());
        } else {
            let before = path.subrange(0, start - 1);
            assert(path.subrange(0, start) =~= path);
            assert(path.drop_last() =~= before);
            assert(path.subrange(start, path.len() as int) =~= Seq::<u8>::empty());
            assert(split_segments(path) == split_segments(before).push(Seq::<u8>::empty()));
            assert(split_segments(path).drop_last() =~= split_segments(before));
            assert(split_segments(path) =~= split_segments(path).drop_last().push(
                Seq::<u8>::empty(),
            ));
        }
    } else {
        let shorter = path.drop_last();
        lemma_split_no_slash_suffix(shorter, start);
        assert(shorter.subrange(0, start) =~= path.subrange(0, start));
        assert(path.subrange(start, path.len() as int) =~= shorter.subrange(
            start,
            shorter.len() as int,
        ).push(path.last()));
        lemma_split_shape(shorter);
        let prior = split_segments(shorter);
        assert(prior.last() == shorter.subrange(start, shorter.len() as int));
        assert(split_segments(path) == prior.update(prior.len() - 1, prior.last().push(path.last())));
        assert(split_segments(path) =~= prior.drop_last().push(prior.last().push(path.last())));
    }
}

/// The relative loop over a concatenation.
pub proof fn lemma_relative_loop_append(a: Seq<StdComponent>, b: Seq<StdComponent>)
    ensures
        relative_loop(a + b) == match relative_loop(a) {
            None => None,
            Some(na) => match relative_loop(b) {
                None => None,
                Some(nb) => Some(na + nb),
            },
        },
    decreases b.len(),
{
    if b.len() == 0 {
        assert(a + b =~= a);
        match relative_loop(a) {
            None => {},
            Some(na) => {
                assert(na + Seq::<Seq<u8>>::empty() =~= na);
            },
        }
    } else {
        lemma_relative_loop_append(a, b.drop_last());
        assert((a + b).drop_last() =~= a + b.drop_last());
        assert((a + b).last() == b.last());
        match relative_loop(a) {
            None => {},
            Some(na) => {
                match relative_loop(b.drop_last()) {
                    None => {},
                    Some(nb) => {
                        match b.last() {
                            StdComponent::Normal(name) => {
                                assert((na + nb).push(name) =~= na + nb.push(name));
                            },
                            _ => {},
                        }
                    },
                }
            },
        }
    }
}

/// The relative loop over the body components is the segment scan.
pub proof fn lemma_relative_loop_body(segments: Seq<Seq<u8>>)
    ensures
        relative_loop(body_components(segments)) == if segment_scan(segments).1 {
            None
        } else {
            Some(segment_scan(segments).0)
        },
    decreases segments.len(),
{
    if segments.len() > 0 {
        lemma_relative_loop_body(segments.drop_last());
        let prior = body_components(segments.drop_last());
        let last = segments.last();
        if !is_skipped(last) {
            if last == dot_dot_name() {
                assert(body_components(segments) == prior.push(StdComponent::ParentDir));
                assert(body_components(segments).drop_last() =~= prior);
            } else {
                assert(body_components(segments) == prior.push(StdComponent::Normal(last)));
                assert(body_components(segments).drop_last() =~= prior);
            }
        }
    }
}

/// The absolute loop over `RootDir` and the body components is the segment
/// scan.
pub proof fn lemma_absolute_loop_body(segments: Seq<Seq<u8>>)
    ensures
        absolute_loop(seq![StdComponent::RootDir] + body_components(segments)) == segment_scan(
            segments,
        ),
    decreases segments.len(),
{
    let root = seq![StdComponent::RootDir];
    let x = root + body_components(segments);
    if segments.len() == 0 {
        assert(x =~= root);
        assert(root.drop_last() =~= Seq::<StdComponent>::empty());
        assert(absolute_loop(Seq::<StdComponent>::empty()) == (Seq::<Seq<u8>>::empty(), false));
        assert(absolute_loop(root) == (Seq::<Seq<u8>>::empty(), false));
    } else {
        lemma_absolute_loop_body(segments.drop_last());
        let prior = body_components(segments.drop_last());
        let last = segments.last();
        let before = absolute_loop(root + prior);
        assert(before == segment_scan(segments.drop_last()));
        if is_skipped(last) {
            assert(body_components(segments) == prior);
            assert(x == root + prior);
        } else {
            let c = if last == dot_dot_name() {
                StdComponent::ParentDir
            } else {
                StdComponent::Normal(last)
            };
            assert(body_components(segments) == prior.push(c));
            assert(x =~= (root + prior).push(c));
            assert(x.drop_last() =~= root + prior);
            assert(x.last() == c);
            assert(x.len() > 0);
            assert(absolute_loop(x) == {
                let (names, refused) = before;
                if refused {
                    (names, true)
                } else {
                    match c {
                        StdComponent::RootDir => (names, false),
                        StdComponent::Normal(name) => (names.push(name), false),
                        _ => (names, true),
                    }
                }
            });
        }
    }
}

/// The segment scan: `..` is found iff some piece is `..`; without one, the
/// names are the kept pieces; every name is a kept, non-`..` piece.
pub proof fn lemma_segment_scan(segments: Seq<Seq<u8>>)
    ensures
        segment_scan(segments).1 == has_dot_dot(segments),
        !has_dot_dot(segments) ==> segment_scan(segments).0 == kept_segments(segments),
        forall|i: int|
            0 <= i < segment_scan(segments).0.len() ==> is_name(
                #[trigger] segment_scan(segments).0[i],
            ) && segments.contains(segment_scan(segments).0[i]),
    decreases segments.len(),
{
    if segments.len() > 0 {
        let prior = segments.drop_last();
        lemma_segment_scan(prior);
        let last = segments.last();
        assert(segments =~= prior.push(last));
        if has_dot_dot(prior) {
            let i = choose|i: int| 0 <= i < prior.len() && #[trigger] prior[i] == dot_dot_name();
            assert(segments[i] == dot_dot_name());
        }
        if last == dot_dot_name() {
            assert(segments[segments.len() - 1] == dot_dot_name());
        }
        if has_dot_dot(segments) {
            let i = choose|i: int|
                0 <= i < segments.len() && #[trigger] segments[i] == dot_dot_name();
            if i < prior.len() {
                assert(prior[i] == dot_dot_name());
            }
        }
        let names = segment_scan(segments).0;
        assert forall|i: int| 0 <= i < names.len() implies is_name(#[trigger] names[i])
            && segments.contains(names[i]) by {
            let prior_names = segment_scan(prior).0;
            if i < prior_names.len() {
                assert(names[i] == prior_names[i]);
                assert(prior.contains(prior_names[i]));
                let k = choose|k: int| 0 <= k < prior.len() && prior[k] == prior_names[i];
                assert(segments[k] == names[i]);
            } else {
                assert(names[i] == last);
                assert(segments[segments.len() - 1] == last);
            }
        }
    }
}

/// Every kept piece of a canonical list is the list itself.
pub proof fn lemma_kept_all(segments: Seq<Seq<u8>>)
    requires
        forall|i: int| 0 <= i < segments.len() ==> !is_skipped(#[trigger] segments[i]),
    ensures
        kept_segments(segments) == segments,
    decreases segments.len(),
{
    if segments.len() > 0 {
        let prior = segments.drop_last();
        assert forall|i: int| 0 <= i < prior.len() implies !is_skipped(#[trigger] prior[i]) by {
            assert(prior[i] == segments[i]);
        }
        lemma_kept_all(prior);
        assert(!is_skipped(segments[segments.len() - 1]));
        assert(segments =~= prior.push(segments.last()));
    }
}

/// The kept pieces are empty iff every piece is skipped.
pub proof fn lemma_kept_empty(segments: Seq<Seq<u8>>)
    ensures
        (kept_segments(segments).len() == 0) == (forall|i: int|
            0 <= i < segments.len() ==> is_skipped(#[trigger] segments[i])),
    decreases segments.len(),
{
    if segments.len() > 0 {
        let prior = segments.drop_last();
        lemma_kept_empty(prior);
        if kept_segments(segments).len() == 0 {
            assert forall|i: int| 0 <= i < segments.len() implies is_skipped(
                #[trigger] segments[i],
            ) by {
                if i < prior.len() {
                    assert(segments[i] == prior[i]);
                }
            }
        }
        if forall|i: int| 0 <= i < segments.len() ==> is_skipped(#[trigger] segments[i]) {
            assert forall|i: int| 0 <= i < prior.len() implies is_skipped(#[trigger] prior[i]) by {
                assert(prior[i] == segments[i]);
            }
            assert(is_skipped(segments[segments.len() - 1]));
        }
    }
}

/// The relative decision, reduced to the leading bytes and the segment scan.
pub proof fn lemma_relative_reduced(path: Seq<u8>)
    ensures
        spec_relative_components(path) == if has_root(path) || starts_with_cur_dir(path)
            || segment_scan(split_segments(path)).1 {
            Err::<Seq<Seq<u8>>, RelativeError>(RelativeError::Unsafe)
        } else if segment_scan(split_segments(path)).0.len() == 0 {
            Err(RelativeError::Empty)
        } else {
            Ok(segment_scan(split_segments(path)).0)
        },
{
    let segments = split_segments(path);
    let body = body_components(segments);
    lemma_relative_loop_body(segments);
    if has_root(path) {
        let lead = seq![StdComponent::RootDir];
        lemma_relative_loop_append(lead, body);
        assert(lead.drop_last() =~= Seq::<StdComponent>::empty());
    } else if starts_with_cur_dir(path) {
        let lead = seq![StdComponent::CurDir];
        lemma_relative_loop_append(lead, body);
        assert(lead.drop_last() =~= Seq::<StdComponent>::empty());
    } else {
        assert(std_components(path) =~= body);
    }
}

// --------------------------------------------------------------- theorems

/// An accepted relative path yields at least one name, and every name is a
/// safe single `openat` step.
pub proof fn theorem_relative_names_are_safe(path: Seq<u8>)
    ensures
        spec_relative_components(path) matches Ok(names) ==> names.len() > 0 && forall|i: int|
            0 <= i < names.len() ==> is_safe_name(#[trigger] names[i]),
{
    lemma_relative_reduced(path);
    let segments = split_segments(path);
    lemma_segment_scan(segments);
    lemma_split_shape(path);
    if let Ok(names) = spec_relative_components(path) {
        assert forall|i: int| 0 <= i < names.len() implies is_safe_name(#[trigger] names[i]) by {
            assert(is_name(names[i]));
            assert(segments.contains(names[i]));
            let k = choose|k: int| 0 <= k < segments.len() && segments[k] == names[i];
            assert(!segments[k].contains(slash()));
        }
    }
}

/// The relative decision restated on bytes and `/`-separated segments.
pub proof fn theorem_relative_classification(path: Seq<u8>)
    ensures
        (spec_relative_components(path) == Err::<Seq<Seq<u8>>, RelativeError>(
            RelativeError::Unsafe,
        )) == (has_root(path) || starts_with_cur_dir(path) || has_dot_dot(split_segments(path))),
        (spec_relative_components(path) == Err::<Seq<Seq<u8>>, RelativeError>(
            RelativeError::Empty,
        )) == (!(has_root(path) || starts_with_cur_dir(path) || has_dot_dot(split_segments(path)))
            && forall|i: int|
            0 <= i < split_segments(path).len() ==> is_skipped(#[trigger] split_segments(path)[i])),
        spec_relative_components(path) is Ok ==> spec_relative_components(path) == Ok::<
            Seq<Seq<u8>>,
            RelativeError,
        >(kept_segments(split_segments(path))),
{
    let segments = split_segments(path);
    lemma_relative_reduced(path);
    lemma_segment_scan(segments);
    lemma_kept_empty(segments);
}

/// A canonical relative path round-trips: its names are its segments, and
/// joining them with `/` gives back the input.
pub proof fn theorem_relative_round_trip(path: Seq<u8>)
    requires
        forall|i: int|
            0 <= i < split_segments(path).len() ==> is_name(#[trigger] split_segments(path)[i]),
    ensures
        spec_relative_components(path) == Ok::<Seq<Seq<u8>>, RelativeError>(split_segments(path)),
        join_segments(split_segments(path)) == path,
{
    let segments = split_segments(path);
    lemma_split_shape(path);
    lemma_relative_reduced(path);
    lemma_segment_scan(segments);
    lemma_join_split(path);
    assert(!has_dot_dot(segments));
    assert forall|i: int| 0 <= i < segments.len() implies !is_skipped(#[trigger] segments[i]) by {
        assert(is_name(segments[i]));
    }
    lemma_kept_all(segments);
    assert(is_name(segments[0]));
    if has_root(path) {
        lemma_split_first(path, 0);
        assert(path.subrange(0, 0) =~= Seq::<u8>::empty());
    }
    if starts_with_cur_dir(path) {
        lemma_split_first(path, 1);
        assert(path.subrange(0, 1) =~= dot_name());
    }
}

/// A path with a `/` in it has at least two pieces.
pub proof fn lemma_split_len_with_slash(path: Seq<u8>, k: int)
    requires
        0 <= k < path.len(),
        path[k] == slash(),
    ensures
        split_segments(path).len() >= 2,
    decreases path.len(),
{
    lemma_split_shape(path.drop_last());
    if k < path.len() - 1 {
        lemma_split_len_with_slash(path.drop_last(), k);
    }
}

/// The first piece of a path is everything before its first `/`.
pub proof fn lemma_split_first(path: Seq<u8>, k: int)
    requires
        0 <= k <= path.len(),
        k == path.len() || path[k] == slash(),
        forall|j: int| 0 <= j < k ==> path[j] != slash(),
    ensures
        split_segments(path)[0] == path.subrange(0, k),
    decreases path.len(),
{
    lemma_split_shape(path);
    if k == path.len() {
        lemma_split_no_slash_suffix(path, 0);
        assert(path.subrange(0, 0) =~= Seq::<u8>::empty());
        assert(path.subrange(0, k) =~= path);
    } else {
        let prior = path.drop_last();
        lemma_split_shape(prior);
        if k < path.len() - 1 {
            lemma_split_first(prior, k);
            lemma_split_len_with_slash(prior, k);
            assert(prior.subrange(0, k) =~= path.subrange(0, k));
        } else {
            lemma_split_first(prior, k);
            assert(prior.subrange(0, k) =~= path.subrange(0, k));
        }
    }
}

/// When the scan finds a `..`, its names are the kept pieces before the
/// first `..`.
pub proof fn lemma_scan_first_dot_dot(segments: Seq<Seq<u8>>)
    ensures
        segment_scan(segments).1 ==> exists|k: int|
            0 <= k < segments.len() && #[trigger] segments[k] == dot_dot_name() && !has_dot_dot(
                segments.take(k),
            ) && segment_scan(segments).0 == kept_segments(segments.take(k)),
    decreases segments.len(),
{
    if segments.len() > 0 {
        let prior = segments.drop_last();
        lemma_scan_first_dot_dot(prior);
        lemma_segment_scan(prior);
        if segment_scan(prior).1 {
            let k = choose|k: int|
                0 <= k < prior.len() && #[trigger] prior[k] == dot_dot_name() && !has_dot_dot(
                    prior.take(k),
                ) && segment_scan(prior).0 == kept_segments(prior.take(k));
            assert(prior.take(k) =~= segments.take(k));
            assert(segments[k] == prior[k]);
        } else if segments.last() == dot_dot_name() {
            let k = prior.len() as int;
            assert(segments.take(k) =~= prior);
            assert(segments[k] == dot_dot_name());
        }
    }
}

/// The absolute walk restated on bytes and segments: refused before any
/// traversal without a leading `/`; otherwise it opens the pieces that are
/// neither empty nor `.` before the first `..` (all of them when there is
/// none), each a safe name, and is refused iff some piece is `..`.
pub proof fn theorem_absolute_walk(path: Seq<u8>)
    ensures
        spec_absolute_walk(path) is None <==> !has_root(path),
        spec_absolute_walk(path) matches Some((names, refused)) ==> {
            &&& refused == has_dot_dot(split_segments(path))
            &&& !refused ==> names == kept_segments(split_segments(path))
            &&& refused ==> exists|k: int|
                0 <= k < split_segments(path).len() && #[trigger] split_segments(path)[k]
                    == dot_dot_name() && !has_dot_dot(split_segments(path).take(k)) && names
                    == kept_segments(split_segments(path).take(k))
            &&& forall|i: int| 0 <= i < names.len() ==> is_safe_name(#[trigger] names[i])
        },
{
    let segments = split_segments(path);
    lemma_segment_scan(segments);
    lemma_scan_first_dot_dot(segments);
    lemma_split_shape(path);
    if has_root(path) {
        assert(std_components(path) == seq![StdComponent::RootDir] + body_components(segments));
        lemma_absolute_loop_body(segments);
        let names = segment_scan(segments).0;
        assert forall|i: int| 0 <= i < names.len() implies is_safe_name(#[trigger] names[i]) by {
            assert(is_name(names[i]));
            assert(segments.contains(names[i]));
            let k = choose|k: int| 0 <= k < segments.len() && segments[k] == names[i];
            assert(!segments[k].contains(slash()));
        }
    }
}

/// `Path::components` silently normalises repeated `/`, interior `.` and a
/// trailing `/` or `/.`; the relative walk inherits that.
pub proof fn example_normalisation_quirks()
    ensures
        spec_relative_components(seq![0x61u8, 0x2f, 0x2f, 0x62]) == Ok::<
            Seq<Seq<u8>>,
            RelativeError,
        >(seq![seq![0x61u8], seq![0x62u8]]),
        spec_relative_components(seq![0x61u8, 0x2f, 0x2e, 0x2f, 0x62]) == Ok::<
            Seq<Seq<u8>>,
            RelativeError,
        >(seq![seq![0x61u8], seq![0x62u8]]),
        spec_relative_components(seq![0x61u8, 0x2f, 0x62, 0x2f]) == Ok::<
            Seq<Seq<u8>>,
            RelativeError,
        >(seq![seq![0x61u8], seq![0x62u8]]),
        spec_relative_components(seq![0x61u8, 0x2f, 0x62, 0x2f, 0x2e]) == Ok::<
            Seq<Seq<u8>>,
            RelativeError,
        >(seq![seq![0x61u8], seq![0x62u8]]),
        spec_relative_components(seq![0x2eu8, 0x2f, 0x61]) == Err::<Seq<Seq<u8>>, RelativeError>(
            RelativeError::Unsafe,
        ),
        spec_relative_components(seq![0x2eu8, 0x61]) == Ok::<Seq<Seq<u8>>, RelativeError>(
            seq![seq![0x2eu8, 0x61]],
        ),
        spec_relative_components(seq![0x2fu8, 0x2f]) == Err::<Seq<Seq<u8>>, RelativeError>(
            RelativeError::Unsafe,
        ),
        spec_relative_components(Seq::<u8>::empty()) == Err::<Seq<Seq<u8>>, RelativeError>(
            RelativeError::Empty,
        ),
{
    assert(spec_relative_components(seq![0x61u8, 0x2f, 0x2f, 0x62]) == Ok::<
        Seq<Seq<u8>>,
        RelativeError,
    >(seq![seq![0x61u8], seq![0x62u8]])) by (compute);
    assert(spec_relative_components(seq![0x61u8, 0x2f, 0x2e, 0x2f, 0x62]) == Ok::<
        Seq<Seq<u8>>,
        RelativeError,
    >(seq![seq![0x61u8], seq![0x62u8]])) by (compute);
    assert(spec_relative_components(seq![0x61u8, 0x2f, 0x62, 0x2f]) == Ok::<
        Seq<Seq<u8>>,
        RelativeError,
    >(seq![seq![0x61u8], seq![0x62u8]])) by (compute);
    assert(spec_relative_components(seq![0x61u8, 0x2f, 0x62, 0x2f, 0x2e]) == Ok::<
        Seq<Seq<u8>>,
        RelativeError,
    >(seq![seq![0x61u8], seq![0x62u8]])) by (compute);
    assert(spec_relative_components(seq![0x2eu8, 0x2f, 0x61]) == Err::<
        Seq<Seq<u8>>,
        RelativeError,
    >(RelativeError::Unsafe)) by (compute);
    assert(spec_relative_components(seq![0x2eu8, 0x61]) == Ok::<Seq<Seq<u8>>, RelativeError>(
        seq![seq![0x2eu8, 0x61]],
    )) by (compute);
    assert(spec_relative_components(seq![0x2fu8, 0x2f]) == Err::<Seq<Seq<u8>>, RelativeError>(
        RelativeError::Unsafe,
    )) by (compute);
    assert(spec_relative_components(Seq::<u8>::empty()) == Err::<Seq<Seq<u8>>, RelativeError>(
        RelativeError::Empty,
    )) by (compute);
}

// ------------------------------------------------------------------- exec

/// Whether `path[start..end]` is empty or `.`, and whether it is `..`.
fn classify_segment(path: &[u8], start: usize, end: usize) -> (r: (bool, bool))
    requires
        start <= end <= path@.len(),
    ensures
        r.0 == is_skipped(path@.subrange(start as int, end as int)),
        r.1 == (path@.subrange(start as int, end as int) == dot_dot_name()),
{
    let ghost segment = path@.subrange(start as int, end as int);
    let len = end - start;
    let one_dot = len == 1 && path[start] == b'.';
    let dot_dot = len == 2 && path[start] == b'.' && path[start + 1] == b'.';
    proof {
        assert(segment.len() == len);
        if len >= 1 {
            assert(segment[0] == path@[start as int]);
        }
        if len >= 2 {
            assert(segment[1] == path@[start + 1]);
        }
        if one_dot {
            assert(segment =~= dot_name());
        }
        if segment == dot_name() {
            assert(segment[0] == dot());
        }
        if dot_dot {
            assert(segment =~= dot_dot_name());
        }
        if segment == dot_dot_name() {
            assert(segment[0] == dot() && segment[1] == dot());
        }
    }
    (len == 0 || one_dot, dot_dot)
}

/// Scan `path`'s `/`-separated pieces: the names before the first `..`, and
/// whether a `..` occurs.
fn scan_segments(path: &[u8]) -> (r: (Vec<Vec<u8>>, bool))
    ensures
        (names_view(r.0@), r.1) == segment_scan(split_segments(path@)),
{
    let mut names: Vec<Vec<u8>> = Vec::new();
    let mut found = false;
    let mut start: usize = 0;
    let mut i: usize = 0;
    let ghost mut done: Seq<Seq<u8>> = Seq::empty();
    assert(path@.subrange(0, 0) =~= Seq::<u8>::empty());
    while i < path.len()
        invariant
            start <= i <= path@.len(),
            start == 0 || path@[start - 1] == slash(),
            forall|j: int| start <= j < i ==> path@[j] != slash(),
            done == split_segments(path@.subrange(0, start as int)).drop_last(),
            (names_view(names@), found) == segment_scan(done),
        decreases path.len() - i,
    {
        if path[i] == b'/' {
            let (skipped, dot_dot) = classify_segment(path, start, i);
            let ghost segment = path@.subrange(start as int, i as int);
            proof {
                let prefix = path@.subrange(0, i as int);
                assert forall|j: int| start <= j < prefix.len() implies prefix[j] != slash() by {
                    assert(prefix[j] == path@[j]);
                }
                if start > 0 {
                    assert(prefix[start - 1] == path@[start - 1]);
                }
                lemma_split_no_slash_suffix(prefix, start as int);
                assert(prefix.subrange(0, start as int) =~= path@.subrange(0, start as int));
                assert(prefix.subrange(start as int, prefix.len() as int) =~= segment);
                assert(path@.subrange(0, i + 1).drop_last() =~= prefix);
                assert(path@.subrange(0, i + 1).last() == slash());
                assert(split_segments(path@.subrange(0, i + 1)).drop_last() =~= done.push(
                    segment,
                ));
                assert(done.push(segment).drop_last() =~= done);
            }
            if !found {
                if dot_dot {
                    found = true;
                } else if !skipped {
                    let ghost before = names@;
                    names.push(vstd::slice::slice_to_vec(&path[start..i]));
                    proof {
                        assert(names_view(names@) =~= names_view(before).push(segment));
                    }
                }
            }
            proof {
                done = done.push(segment);
            }
            start = i + 1;
        }
        i += 1;
    }
    let (skipped, dot_dot) = classify_segment(path, start, path.len());
    let ghost segment = path@.subrange(start as int, path@.len() as int);
    proof {
        assert(path@.subrange(0, path@.len() as int) =~= path@);
        lemma_split_no_slash_suffix(path@, start as int);
        assert(split_segments(path@) == done.push(segment));
        assert(done.push(segment).drop_last() =~= done);
    }
    if !found {
        if dot_dot {
            found = true;
        } else if !skipped {
            let ghost before = names@;
            names.push(vstd::slice::slice_to_vec(&path[start..path.len()]));
            proof {
                assert(names_view(names@) =~= names_view(before).push(segment));
            }
        }
    }
    (names, found)
}

/// The names production's `relative_components` walks for the
/// repository-relative path with bytes `path`, or why it refuses the path.
pub fn relative_components(path: &[u8]) -> (r: Result<Vec<Vec<u8>>, RelativeError>)
    ensures
        match r {
            Ok(names) => spec_relative_components(path@) == Ok::<Seq<Seq<u8>>, RelativeError>(
                names_view(names@),
            ),
            Err(error) => spec_relative_components(path@) == Err::<Seq<Seq<u8>>, RelativeError>(
                error,
            ),
        },
{
    proof {
        lemma_relative_reduced(path@);
    }
    let rooted = !path.is_empty() && path[0] == b'/';
    let cur_dir = !rooted && !path.is_empty() && path[0] == b'.' && (path.len() == 1 || path[1]
        == b'/');
    let (names, dot_dot) = scan_segments(path);
    if rooted || cur_dir || dot_dot {
        Err(RelativeError::Unsafe)
    } else if names.is_empty() {
        Err(RelativeError::Empty)
    } else {
        Ok(names)
    }
}

/// What production's `open_canonical_directory` does with a root path.
pub struct AbsoluteWalk {
    /// The names to open below `/`, in order.
    pub names: Vec<Vec<u8>>,
    /// Whether the walk then stops at a refused component (`..`).
    pub refused: bool,
}

/// The walk production's `open_canonical_directory` performs for the path
/// with bytes `path`, or `None` when the path is not absolute.
pub fn absolute_walk(path: &[u8]) -> (r: Option<AbsoluteWalk>)
    ensures
        match r {
            None => spec_absolute_walk(path@) is None,
            Some(walk) => spec_absolute_walk(path@) == Some((names_view(walk.names@), walk.refused)),
        },
{
    if path.is_empty() || path[0] != b'/' {
        return None;
    }
    let (names, refused) = scan_segments(path);
    proof {
        let segments = split_segments(path@);
        assert(std_components(path@) == seq![StdComponent::RootDir] + body_components(segments));
        lemma_absolute_loop_body(segments);
    }
    Some(AbsoluteWalk { names, refused })
}

} // verus!
