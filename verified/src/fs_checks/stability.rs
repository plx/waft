//! Whether observations of a file describe one file that did not change.
//!
//! waft reads a source or destination through an open handle and, around the
//! read, compares the metadata seen through the handle with what `lstat` sees
//! at the path. These predicates are the premise of "the file was not
//! replaced or rewritten while it was being opened, read, or copied":
//!
//! - [`opened_stably`] (`open_stable_regular_file`): the path still names a
//!   regular file, and the path before the open, the opened handle, and the
//!   path after the open all name the same file.
//! - [`unchanged_through_path`] (`ensure_open_file_unchanged`): additionally
//!   the file's state (length, modification and change times, permission
//!   signature) is the same in the initial observation, the handle now, and
//!   the path now.
//! - [`unchanged_handle`] (`ensure_open_handle_unchanged`): the handle still
//!   shows a regular file with the initial identity and state.
//!
//! "Same file" is `same_file`: equal `(st_dev, st_ino)` on Unix; where the
//! platform reports no identity it falls back to `same_state`, as
//! production's non-Unix `same_file` always did.
//!
//! # Theorems
//!
//! - Each exec predicate returns exactly its production condition
//!   (`spec_opened_stably`, `spec_unchanged_through_path`,
//!   `spec_unchanged_handle`).
//! - `lemma_agreement_is_an_equivalence`: `same_state` and `same_file`
//!   on Unix-shaped observations are reflexive, symmetric and transitive.
//! - `theorem_opened_stably_unix`: with inode identities, an accepted open
//!   means the three observations name one inode (so the check order is
//!   irrelevant) and the path is a regular file.
//! - `theorem_unchanged_through_path_unix`: with inode identities, an
//!   accepted read means all three observations agree on the inode and on
//!   length, both timestamps and the permission signature; in particular the
//!   path now names the initially observed inode in the initially observed
//!   state.
//! - `theorem_unchanged_handle_unix`: the handle still shows the initial
//!   inode and state, as a regular file.
//! - `theorem_without_identity`: where no identity is reported, every
//!   predicate degenerates to agreement of state.
//!
//! # Trust assumptions
//!
//! - Production converts each `fs::Metadata` field for field: `is_file` from
//!   the file type, the identity from `(dev, ino)` on Unix (always
//!   [`Identity::Inode`] there, always [`Identity::Unavailable`] elsewhere),
//!   `len`, the permission signature, the change time as the raw
//!   `(ctime, ctime_nsec)` pair on Unix, and the modification time as its
//!   signed distance from the Unix epoch. These encodings are injective, so
//!   kernel equality is exactly the equality production compared before
//!   (`Option<SystemTime>`, `Option<(i64, i64)>`, `u32`, `u64`).
//! - The operating system updates `ctime` on every data or metadata change;
//!   that is what makes "unchanged state" imply "unchanged bytes". This is
//!   an assumption about the OS, listed in `VERIFICATION.md`.

#[allow(unused_imports)]
use vstd::prelude::*;

verus! {

/// A timestamp as the platform reported it, compared only for equality.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stamp {
    /// Not reported (`Metadata::modified` failed, or no change time off
    /// Unix).
    Unavailable,
    /// A `SystemTime`, as its distance from the Unix epoch and the side of
    /// the epoch it lies on.
    Since {
        /// The time lies before the epoch.
        before_epoch: bool,
        /// Whole seconds of the distance.
        whole_secs: u64,
        /// Nanoseconds of the distance beyond `whole_secs`.
        subsec_nanos: u32,
    },
    /// A raw `(seconds, nanoseconds)` pair from `stat`.
    Raw {
        /// Seconds.
        secs: i64,
        /// Nanoseconds.
        nanos: i64,
    },
}

/// Which file an observation is of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Identity {
    /// `(st_dev, st_ino)`.
    Inode {
        /// Device.
        dev: u64,
        /// Inode number.
        ino: u64,
    },
    /// The platform reports no file identity.
    Unavailable,
}

/// One `stat` of a file, through its path or through an open handle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Observation {
    /// The file is a regular file.
    pub is_file: bool,
    /// Which file it is.
    pub identity: Identity,
    /// Length in bytes.
    pub len: u64,
    /// Last modification time.
    pub modified: Stamp,
    /// Last status change time.
    pub changed: Stamp,
    /// Permission signature (the full `st_mode` on Unix).
    pub permissions: u32,
}

/// The two observations show the same length, timestamps and permission
/// signature (production `same_file_state`).
pub open spec fn same_state(a: Observation, b: Observation) -> bool {
    &&& a.len == b.len
    &&& a.modified == b.modified
    &&& a.changed == b.changed
    &&& a.permissions == b.permissions
}

/// The two observations are of the same file (production `same_file`).
pub open spec fn same_file(a: Observation, b: Observation) -> bool {
    match (a.identity, b.identity) {
        (Identity::Inode { .. }, Identity::Inode { .. }) => a.identity == b.identity,
        _ => same_state(a, b),
    }
}

/// Both observations carry an inode identity (always so on Unix).
pub open spec fn has_inode(o: Observation) -> bool {
    o.identity is Inode
}

/// Production's condition after opening a path: the path still names a
/// regular file, and before, handle and after name the same file.
pub open spec fn spec_opened_stably(
    before: Observation,
    handle: Observation,
    after: Observation,
) -> bool {
    &&& after.is_file
    &&& same_file(before, handle)
    &&& same_file(handle, after)
}

/// Production's condition after reading through a handle: the path still
/// names a regular file, and the initial observation, the handle now and the
/// path now are the same file in the same state.
pub open spec fn spec_unchanged_through_path(
    initial: Observation,
    handle: Observation,
    path: Observation,
) -> bool {
    &&& path.is_file
    &&& same_file(initial, handle)
    &&& same_file(handle, path)
    &&& same_state(initial, handle)
    &&& same_state(handle, path)
}

/// Production's condition on a handle alone: it still shows a regular file
/// that is the initially observed file in the same state.
pub open spec fn spec_unchanged_handle(initial: Observation, current: Observation) -> bool {
    &&& current.is_file
    &&& same_file(initial, current)
    &&& same_state(initial, current)
}

/// Agreement of state and of identity are equivalence relations.
pub proof fn lemma_agreement_is_an_equivalence(a: Observation, b: Observation, c: Observation)
    ensures
        same_state(a, a),
        same_state(a, b) ==> same_state(b, a),
        same_state(a, b) && same_state(b, c) ==> same_state(a, c),
        has_inode(a) ==> same_file(a, a),
        has_inode(a) && has_inode(b) && same_file(a, b) ==> same_file(b, a),
        has_inode(a) && has_inode(b) && has_inode(c) && same_file(a, b) && same_file(b, c)
            ==> same_file(a, c),
{
}

/// With inode identities, an accepted open means one inode throughout.
pub proof fn theorem_opened_stably_unix(before: Observation, handle: Observation, after: Observation)
    requires
        has_inode(before),
        has_inode(handle),
        has_inode(after),
    ensures
        spec_opened_stably(before, handle, after) == (after.is_file && before.identity
            == handle.identity && handle.identity == after.identity),
        spec_opened_stably(before, handle, after) ==> before.identity == after.identity,
{
}

/// With inode identities, an accepted read means one inode in one state
/// throughout; in particular the path now shows the initial inode and state.
pub proof fn theorem_unchanged_through_path_unix(
    initial: Observation,
    handle: Observation,
    path: Observation,
)
    requires
        has_inode(initial),
        has_inode(handle),
        has_inode(path),
    ensures
        spec_unchanged_through_path(initial, handle, path) == (path.is_file && initial.identity
            == handle.identity && handle.identity == path.identity && same_state(initial, handle)
            && same_state(handle, path)),
        spec_unchanged_through_path(initial, handle, path) ==> initial.identity == path.identity
            && same_state(initial, path),
{
}

/// With inode identities, an accepted handle check means the initial inode in
/// the initial state.
pub proof fn theorem_unchanged_handle_unix(initial: Observation, current: Observation)
    requires
        has_inode(initial),
        has_inode(current),
    ensures
        spec_unchanged_handle(initial, current) == (current.is_file && initial.identity
            == current.identity && same_state(initial, current)),
{
}

/// Without identities, every check is agreement of state.
pub proof fn theorem_without_identity(a: Observation, b: Observation, c: Observation)
    requires
        a.identity is Unavailable,
        b.identity is Unavailable,
        c.identity is Unavailable,
    ensures
        spec_opened_stably(a, b, c) == (c.is_file && same_state(a, b) && same_state(b, c)),
        spec_unchanged_through_path(a, b, c) == (c.is_file && same_state(a, b) && same_state(
            b,
            c,
        )),
        spec_unchanged_handle(a, b) == (b.is_file && same_state(a, b)),
{
}

fn same_stamp(a: &Stamp, b: &Stamp) -> (r: bool)
    ensures
        r == (*a == *b),
{
    match (a, b) {
        (Stamp::Unavailable, Stamp::Unavailable) => true,
        (
            Stamp::Since { before_epoch: b1, whole_secs: s1, subsec_nanos: n1 },
            Stamp::Since { before_epoch: b2, whole_secs: s2, subsec_nanos: n2 },
        ) => *b1 == *b2 && *s1 == *s2 && *n1 == *n2,
        (Stamp::Raw { secs: s1, nanos: n1 }, Stamp::Raw { secs: s2, nanos: n2 }) => *s1 == *s2
            && *n1 == *n2,
        _ => false,
    }
}

fn same_state_exec(a: &Observation, b: &Observation) -> (r: bool)
    ensures
        r == same_state(*a, *b),
{
    a.len == b.len && same_stamp(&a.modified, &b.modified) && same_stamp(&a.changed, &b.changed)
        && a.permissions == b.permissions
}

fn same_file_exec(a: &Observation, b: &Observation) -> (r: bool)
    ensures
        r == same_file(*a, *b),
{
    match (a.identity, b.identity) {
        (Identity::Inode { dev: d1, ino: i1 }, Identity::Inode { dev: d2, ino: i2 }) => d1 == d2
            && i1 == i2,
        _ => same_state_exec(a, b),
    }
}

/// `open_stable_regular_file`'s check after the open.
pub fn opened_stably(before: &Observation, handle: &Observation, after: &Observation) -> (r: bool)
    ensures
        r == spec_opened_stably(*before, *handle, *after),
{
    after.is_file && same_file_exec(before, handle) && same_file_exec(handle, after)
}

/// `ensure_open_file_unchanged`'s check after a read.
pub fn unchanged_through_path(
    initial: &Observation,
    handle: &Observation,
    path: &Observation,
) -> (r: bool)
    ensures
        r == spec_unchanged_through_path(*initial, *handle, *path),
{
    path.is_file && same_file_exec(initial, handle) && same_file_exec(handle, path)
        && same_state_exec(initial, handle) && same_state_exec(handle, path)
}

/// `ensure_open_handle_unchanged`'s check.
pub fn unchanged_handle(initial: &Observation, current: &Observation) -> (r: bool)
    ensures
        r == spec_unchanged_handle(*initial, *current),
{
    current.is_file && same_file_exec(initial, current) && same_state_exec(initial, current)
}

} // verus!
