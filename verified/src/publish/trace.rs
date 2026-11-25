//! The event alphabet shared by every publication protocol, and the safety
//! properties the protocols are proven to keep.
//!
//! Each kernel step machine in [`crate::publish`] records, as a ghost trace,
//! one [`Ev`] for every probe it asks production to run and every
//! namespace-changing syscall it asks production to issue, together with the
//! answer production fed back. The trace is erased from the shipped binary;
//! it exists so that the properties below can be stated about *what the kernel
//! asked for* and proven once for every path through a protocol.
//!
//! The names a publication touches are abstracted to [`Name`]: the
//! destination, the staging name holding this run's prepared file, and the
//! fresh name a destination is moved aside to. File identities are abstracted
//! to [`Ino`] through the [`Identity`] trait.
//!
//! # Safety properties (`safe`)
//!
//! * `proven_unlinks`: every unlink is issued immediately after a probe that
//!   proved the name holds an inode the run is `entitled` to remove — the
//!   run's own staging file, whatever the name holds right after the run's
//!   own fallback hard link from that name succeeded (the extra name of the
//!   publication; the model's A-link axiom is what ties it to the inode the
//!   link published), or the planned destination inode after a read-back
//!   showed it still holds the planned bytes. The destination name itself is
//!   never entitled, so it is never unlinked.
//! * `links_are_fallbacks`: a hard link is only ever made as the fallback of
//!   a `RENAME_NOREPLACE` the filesystem reported as unsupported.
//! * `exchanges_proven`: every atomic exchange of the staging and destination
//!   names is issued immediately after proving what the destination holds —
//!   the verified planned file with its planned bytes (the replacement), or
//!   this run's own prepared file (the undo).
//! * `asides_proven`: the destination is moved aside only after an exchange
//!   was reported unsupported and the destination was freshly re-proved.
//!
//! `lemma_safe_push` is the single induction step every machine uses;
//! `lemma_safe_append_removal` and `lemma_safe_append_noreplace` splice
//! the traces of the two reusable sub-protocols into a parent trace.

#[allow(unused_imports)]
use vstd::prelude::*;

verus! {

/// A file identity: `(st_dev, st_ino)`.
pub type Ino = (u64, u64);

/// A value that identifies a file — in production, the `stat` of a
/// descriptor or a name.
///
/// This is a trust boundary: production's implementation of
/// [`Identity::same_inode`] must compare exactly the `(st_dev, st_ino)` pair
/// that `Identity::ino` abstracts, which is also what every identity probe
/// production runs on the kernel's behalf compares.
pub trait Identity: Copy + Sized {
    /// The `(device, inode)` pair this value identifies.
    spec fn ino(&self) -> Ino;

    /// Whether two identities name the same inode.
    fn same_inode(&self, other: &Self) -> (r: bool)
        ensures
            r == (self.ino() == other.ino()),
    ;
}

/// The view of an optional identity.
pub open spec fn ino_of<I: Identity>(i: Option<I>) -> Option<Ino> {
    match i {
        Some(x) => Some(x.ino()),
        None => None,
    }
}

/// The directory entries a publication acts on, all under one parent
/// directory handle.
pub enum Name {
    /// The destination name the caller asked about.
    Dest,
    /// The fresh `.waft-copy-*` name holding this run's prepared file.
    Staging,
    /// The fresh `.waft-copy-*.displaced` name a destination is moved aside
    /// to where the filesystem cannot exchange two names.
    Displaced,
}

/// How a rename with a `RENAME_*` flag was answered.
pub enum Res {
    /// It succeeded.
    Ok,
    /// The filesystem reported the flag as unsupported.
    Unsupported,
    /// It failed for any other reason.
    Failed,
}

/// One probe or syscall a protocol asked for, with the answer it was given.
pub enum Ev {
    /// Does `name` resolve to `ino`? `holds == false` also covers a probe
    /// that could not be completed.
    Probe { name: Name, ino: Ino, holds: bool },
    /// Does the pinned destination descriptor still read back as exactly the
    /// planned snapshot (bytes and mode)?
    PinnedMatches { holds: bool },
    /// The identity of `name`, read without following a symlink.
    Lstat { name: Name, found: Option<Ino> },
    /// `renameat2(from, Dest, RENAME_NOREPLACE)`.
    RenameNoReplace { from: Name, res: Res },
    /// `linkat(from, Dest)`.
    Link { from: Name, ok: bool },
    /// `unlinkat(name)`.
    Unlink { name: Name, ok: bool },
    /// `renameat2(Staging, Dest, RENAME_EXCHANGE)`.
    Exchange { res: Res },
    /// `renameat(Dest, Displaced)`: unconditional, onto a fresh name.
    RenameAside { ok: bool },
    /// `fchmod(mode)` on the pinned destination descriptor.
    Fchmod { mode: u32, ok: bool },
    /// The pinned destination descriptor read back after a repair.
    ReadBack { content_matches: bool, mode_matches: bool },
    /// `fsync` of the pinned destination descriptor.
    Sync { ok: bool },
    /// The source descriptor was found unchanged since its snapshot.
    SourceUnchanged,
    /// The caller's pre-publication callback (the index-lock recheck)
    /// returned `Ok`.
    BeforePublish,
    /// The destination parent directory handle was revalidated.
    ParentRevalidated,
    /// The destination name was proved to resolve to the pinned destination
    /// descriptor (`holds == false` also covers a proof that failed).
    DestIsPinned { holds: bool },
    /// The staging file was given the source's mode.
    StagingMode,
    /// The staging file was synced.
    StagingSynced,
    /// The staging name was proved to resolve to the prepared staging
    /// descriptor.
    StagingIsPrepared,
}

/// The identities a run may act on.
pub struct Run {
    /// The run's own staging file, as pinned by its staging guard.
    pub own: Option<Ino>,
    /// The planned destination inode the run verified (replacement only).
    pub verified: Option<Ino>,
    /// The run's prepared replacement, as pinned before an exchange.
    pub prepared: Option<Ino>,
}

/// Trigger wrapper: `e` is an unlink.
pub open spec fn is_unlink(e: Ev) -> bool {
    e is Unlink
}

/// Trigger wrapper: `e` is a hard link.
pub open spec fn is_link(e: Ev) -> bool {
    e is Link
}

/// Trigger wrapper: `e` is an exchange.
pub open spec fn is_exchange(e: Ev) -> bool {
    e is Exchange
}

/// Trigger wrapper: `e` moves the destination aside.
pub open spec fn is_aside(e: Ev) -> bool {
    e is RenameAside
}

/// At `j`, the protocol moved `v` under `n` and then observed both that `n`
/// holds `v` and that the pinned descriptor still reads back as the planned
/// snapshot.
pub open spec fn moved_then_verified(t: Seq<Ev>, j: int, n: Name, v: Ino) -> bool {
    &&& 0 <= j
    &&& j + 2 < t.len()
    &&& t[j + 2] == (Ev::PinnedMatches { holds: true })
    &&& match n {
        Name::Staging => t[j] == (Ev::Exchange { res: Res::Ok }) && t[j + 1] == (Ev::Probe {
            name: Name::Staging,
            ino: v,
            holds: true,
        }),
        Name::Displaced => t[j] == (Ev::RenameAside { ok: true }) && t[j + 1] == (Ev::Lstat {
            name: Name::Displaced,
            found: Some(v),
        }),
        Name::Dest => false,
    }
}

/// Somewhere in `t`, `v` was moved under `n` and verified there.
pub open spec fn verified_at(t: Seq<Ev>, n: Name, v: Ino) -> bool {
    exists|j: int| #[trigger] moved_then_verified(t, j, n, v)
}

/// After the events `pre`, may the run remove inode `x` from name `n`?
///
/// Only from a name other than the destination, and only if `x` is the run's
/// own staging file, or `n` was just hard-linked to the destination by the run
/// (so the removal drops what is meant to be a redundant second name — this
/// clause does not constrain `x`; the model's A-link axiom is what makes it
/// the linked inode), or `x` is the verified planned destination inode and
/// was verified after being moved under `n`.
pub open spec fn entitled(pre: Seq<Ev>, n: Name, x: Ino, own: Option<Ino>, v: Option<Ino>) -> bool {
    &&& n != Name::Dest
    &&& {
        ||| (n == Name::Staging && own == Some(x))
        ||| (pre.len() > 0 && pre.last() == (Ev::Link { from: n, ok: true }))
        ||| (v == Some(x) && verified_at(pre, n, x))
    }
}

/// The unlink at `i` was issued immediately after proving that its name
/// holds an inode the run is entitled to remove.
pub open spec fn unlink_proven_at(t: Seq<Ev>, i: int, own: Option<Ino>, v: Option<Ino>) -> bool {
    &&& 1 <= i < t.len()
    &&& t[i] matches Ev::Unlink { name, .. }
    &&& t[i - 1] matches Ev::Probe { name: probed, ino, holds }
    &&& probed == name
    &&& holds
    &&& entitled(t.take(i - 1), name, ino, own, v)
}

/// Every unlink in `t` is proven and entitled.
pub open spec fn proven_unlinks(t: Seq<Ev>, own: Option<Ino>, v: Option<Ino>) -> bool {
    forall|i: int| 0 <= i < t.len() && #[trigger] is_unlink(t[i]) ==> unlink_proven_at(t, i, own, v)
}

/// The link at `i` immediately follows a `RENAME_NOREPLACE` of the same name
/// that the filesystem reported as unsupported.
pub open spec fn link_after_unsupported(t: Seq<Ev>, i: int) -> bool {
    &&& 1 <= i < t.len()
    &&& t[i] matches Ev::Link { from, .. }
    &&& t[i - 1] == (Ev::RenameNoReplace { from, res: Res::Unsupported })
}

/// Every hard link in `t` is a `RENAME_NOREPLACE` fallback.
pub open spec fn links_are_fallbacks(t: Seq<Ev>) -> bool {
    forall|i: int| 0 <= i < t.len() && #[trigger] is_link(t[i]) ==> link_after_unsupported(t, i)
}

/// The exchange at `i` immediately follows a proof of what the destination
/// holds: the verified planned inode with its planned bytes, or the run's own
/// prepared replacement.
pub open spec fn exchange_proven_at(t: Seq<Ev>, i: int, v: Option<Ino>, p: Option<Ino>) -> bool {
    &&& 0 <= i < t.len()
    &&& t[i] is Exchange
    &&& {
        ||| (2 <= i && dest_proved(t[i - 2], v) && t[i - 1] == (Ev::PinnedMatches { holds: true }))
        ||| (1 <= i && dest_proved(t[i - 1], p))
    }
}

/// `e` proved that the destination holds `x`.
pub open spec fn dest_proved(e: Ev, x: Option<Ino>) -> bool {
    x is Some && e == (Ev::Probe { name: Name::Dest, ino: x->Some_0, holds: true })
}

/// Every exchange in `t` is proven.
pub open spec fn exchanges_proven(t: Seq<Ev>, v: Option<Ino>, p: Option<Ino>) -> bool {
    forall|i: int| 0 <= i < t.len() && #[trigger] is_exchange(t[i]) ==> exchange_proven_at(t, i, v, p)
}

/// The move-aside at `i` follows an exchange reported unsupported and a
/// fresh proof that the destination holds the verified planned inode with
/// its planned bytes.
pub open spec fn aside_proven_at(t: Seq<Ev>, i: int, v: Option<Ino>) -> bool {
    &&& 3 <= i < t.len()
    &&& t[i] is RenameAside
    &&& t[i - 3] == (Ev::Exchange { res: Res::Unsupported })
    &&& dest_proved(t[i - 2], v)
    &&& t[i - 1] == (Ev::PinnedMatches { holds: true })
}

/// Every move-aside in `t` is proven.
pub open spec fn asides_proven(t: Seq<Ev>, v: Option<Ino>) -> bool {
    forall|i: int| 0 <= i < t.len() && #[trigger] is_aside(t[i]) ==> aside_proven_at(t, i, v)
}

/// The conjunction of the safety properties, for a run entitled to `r`.
pub open spec fn safe(t: Seq<Ev>, r: Run) -> bool {
    &&& proven_unlinks(t, r.own, r.verified)
    &&& links_are_fallbacks(t)
    &&& exchanges_proven(t, r.verified, r.prepared)
    &&& asides_proven(t, r.verified)
}

/// `t` asks for no namespace change at all.
pub open spec fn no_namespace_change(t: Seq<Ev>) -> bool {
    forall|i: int|
        #![trigger t[i]]
        0 <= i < t.len() ==> !(t[i] is RenameNoReplace || t[i] is Link || t[i] is Unlink
            || t[i] is Exchange || t[i] is RenameAside)
}

/// The trace of a proven removal of `n`, entitled to identity `x`, that
/// ended with `out`.
///
/// This is the whole contract of the removal sub-protocol: nothing is
/// attempted without an identity; the unlink is issued only right after a
/// probe proved `n` still holds `x`; the outcome is `Removed` exactly when
/// that unlink succeeded; and a failure may also be reported without any
/// syscall (production's test-only forced failure).
pub open spec fn removal_trace<E>(t: Seq<Ev>, n: Name, x: Option<Ino>, out: Removal<E>) -> bool {
    match out {
        Removal::Removed => (x matches Some(y) && t == seq![
            Ev::Probe { name: n, ino: y, holds: true },
            Ev::Unlink { name: n, ok: true },
        ]),
        Removal::Left(Leftover::Unchecked) => x is None && t.len() == 0,
        Removal::Left(Leftover::NotHeld) => (x matches Some(y) && t == seq![
            Ev::Probe { name: n, ino: y, holds: false },
        ]),
        Removal::Left(Leftover::RemoveFailed(_)) => t.len() == 0 || (x matches Some(y) && t
            == seq![Ev::Probe { name: n, ino: y, holds: true }, Ev::Unlink { name: n, ok: false }]),
    }
}

/// Why a name a protocol wanted to remove is still on disk.
pub enum Leftover<E> {
    /// No identity was available to prove, so nothing was attempted.
    Unchecked,
    /// The name could not be proved to hold the entitled inode, so it was
    /// left alone.
    NotHeld,
    /// The name was proved, but unlinking it failed (or a test forced a
    /// failure).
    RemoveFailed(E),
}

/// The outcome of a proven removal.
pub enum Removal<E> {
    /// The name held the entitled inode and was unlinked.
    Removed,
    /// The name is still on disk.
    Left(Leftover<E>),
}

/// What a no-clobber publication left under the name it published from.
pub enum Published<E> {
    /// The name is gone: consumed by the rename, or removed after the link.
    Consumed,
    /// The publication stands, but the name it started from is still there.
    LeftBehind(Leftover<E>),
}

/// The removal outcome a no-clobber publication's link cleanup must have had
/// to produce `p`.
pub open spec fn removal_of<E>(p: Published<E>) -> Removal<E> {
    match p {
        Published::Consumed => Removal::Removed,
        Published::LeftBehind(l) => Removal::Left(l),
    }
}

/// The trace of a no-clobber publication of `n` onto the destination,
/// entitled to clean up identity `x`, that ended with `result`.
///
/// `RENAME_NOREPLACE` first; only if that is unsupported, a hard link; only
/// if that succeeded, a proven removal of the extra name.
pub open spec fn noreplace_trace<E>(
    t: Seq<Ev>,
    n: Name,
    x: Option<Ino>,
    result: Result<Published<E>, E>,
) -> bool {
    ||| (t == seq![Ev::RenameNoReplace { from: n, res: Res::Ok }] && result == Ok::<
        Published<E>,
        E,
    >(Published::Consumed))
    ||| (t == seq![Ev::RenameNoReplace { from: n, res: Res::Failed }] && result is Err)
    ||| (t == seq![
        Ev::RenameNoReplace { from: n, res: Res::Unsupported },
        Ev::Link { from: n, ok: false },
    ] && result is Err)
    ||| {
        &&& t.len() >= 2
        &&& t[0] == (Ev::RenameNoReplace { from: n, res: Res::Unsupported })
        &&& t[1] == (Ev::Link { from: n, ok: true })
        &&& result matches Ok(p)
        &&& removal_trace(t.skip(2), n, x, removal_of(p))
    }
}

/// Appending one event keeps every earlier event's justification intact.
proof fn lemma_push_prefix(t: Seq<Ev>, e: Ev, i: int)
    requires
        0 <= i <= t.len(),
    ensures
        t.push(e).take(i) == t.take(i),
{
    assert(t.push(e).take(i) =~= t.take(i));
}

/// A verification observed once stays observed.
pub proof fn lemma_verified_at_push(t: Seq<Ev>, e: Ev, n: Name, x: Ino)
    requires
        verified_at(t, n, x),
    ensures
        verified_at(t.push(e), n, x),
{
    let j = choose|j: int| #[trigger] moved_then_verified(t, j, n, x);
    assert(moved_then_verified(t.push(e), j, n, x));
}

/// A verification observed in `a` is still observed in `a + b`.
pub proof fn lemma_verified_at_append(a: Seq<Ev>, b: Seq<Ev>, n: Name, x: Ino)
    requires
        verified_at(a, n, x),
    ensures
        verified_at(a + b, n, x),
{
    let j = choose|j: int| #[trigger] moved_then_verified(a, j, n, x);
    assert(moved_then_verified(a + b, j, n, x));
}

/// The induction step of every safety proof: appending an event whose own
/// obligation holds keeps the trace safe.
pub proof fn lemma_safe_push(t: Seq<Ev>, e: Ev, r: Run)
    requires
        safe(t, r),
        e is Unlink ==> unlink_proven_at(t.push(e), t.len() as int, r.own, r.verified),
        e is Link ==> link_after_unsupported(t.push(e), t.len() as int),
        e is Exchange ==> exchange_proven_at(t.push(e), t.len() as int, r.verified, r.prepared),
        e is RenameAside ==> aside_proven_at(t.push(e), t.len() as int, r.verified),
    ensures
        safe(t.push(e), r),
{
    let u = t.push(e);
    assert forall|i: int| 0 <= i < u.len() && #[trigger] is_unlink(u[i]) implies unlink_proven_at(
        u,
        i,
        r.own,
        r.verified,
    ) by {
        if i < t.len() {
            assert(is_unlink(t[i]));
            assert(unlink_proven_at(t, i, r.own, r.verified));
            lemma_push_prefix(t, e, i - 1);
        }
    }
    assert forall|i: int| 0 <= i < u.len() && #[trigger] is_link(u[i]) implies link_after_unsupported(
        u,
        i,
    ) by {
        if i < t.len() {
            assert(is_link(t[i]));
        }
    }
    assert forall|i: int| 0 <= i < u.len() && #[trigger] is_exchange(u[i]) implies exchange_proven_at(
        u,
        i,
        r.verified,
        r.prepared,
    ) by {
        if i < t.len() {
            assert(is_exchange(t[i]));
        }
    }
    assert forall|i: int| 0 <= i < u.len() && #[trigger] is_aside(u[i]) implies aside_proven_at(
        u,
        i,
        r.verified,
    ) by {
        if i < t.len() {
            assert(is_aside(t[i]));
        }
    }
}

/// Appending an event that is not a namespace change keeps a trace safe.
pub proof fn lemma_safe_push_observation(t: Seq<Ev>, e: Ev, r: Run)
    requires
        safe(t, r),
        !(e is Unlink || e is Link || e is Exchange || e is RenameAside),
    ensures
        safe(t.push(e), r),
{
    lemma_safe_push(t, e, r);
}

/// Splicing a proven removal of `n` into a safe trace keeps it safe, as long
/// as the run is entitled to remove the removal's identity from `n` at that
/// point.
pub proof fn lemma_safe_append_removal<E>(
    a: Seq<Ev>,
    b: Seq<Ev>,
    n: Name,
    x: Option<Ino>,
    out: Removal<E>,
    r: Run,
)
    requires
        safe(a, r),
        removal_trace(b, n, x, out),
        b.len() == 2 ==> (x matches Some(y) && entitled(a, n, y, r.own, r.verified)),
    ensures
        safe(a + b, r),
        b.len() <= 2,
{
    if b.len() == 0 {
        assert(a + b =~= a);
    } else if b.len() == 1 {
        assert(a + b =~= a.push(b[0]));
        lemma_safe_push(a, b[0], r);
    } else {
        let a1 = a.push(b[0]);
        assert(a + b =~= a1.push(b[1]));
        lemma_safe_push(a, b[0], r);
        lemma_push_prefix(a1, b[1], a.len() as int);
        assert(a1.take(a.len() as int) =~= a);
        lemma_safe_push(a1, b[1], r);
    }
}

/// Splicing a no-clobber publication of `n` (not the destination) into a
/// safe trace keeps it safe: its only unlink removes the extra name its own
/// link has just made redundant.
pub proof fn lemma_safe_append_noreplace<E>(
    a: Seq<Ev>,
    b: Seq<Ev>,
    n: Name,
    x: Option<Ino>,
    result: Result<Published<E>, E>,
    r: Run,
)
    requires
        safe(a, r),
        n != Name::Dest,
        noreplace_trace(b, n, x, result),
    ensures
        safe(a + b, r),
        no_exchange_or_aside(b),
{
    let e0 = Ev::RenameNoReplace { from: n, res: Res::Unsupported };
    if b.len() == 1 {
        assert(a + b =~= a.push(b[0]));
        lemma_safe_push(a, b[0], r);
    } else if b.len() == 2 {
        let a1 = a.push(b[0]);
        assert(a + b =~= a1.push(b[1]));
        lemma_safe_push(a, b[0], r);
        lemma_safe_push(a1, b[1], r);
    } else {
        let a1 = a.push(b[0]);
        let a2 = a1.push(b[1]);
        let c = b.skip(2);
        assert(b =~= seq![b[0], b[1]] + c);
        lemma_safe_push(a, b[0], r);
        lemma_safe_push(a1, b[1], r);
        assert(a2.last() == (Ev::Link { from: n, ok: true }));
        if c.len() == 1 {
            assert(a + b =~= a2.push(c[0]));
            lemma_safe_push(a2, c[0], r);
        } else {
            let a3 = a2.push(c[0]);
            assert(a + b =~= a3.push(c[1]));
            lemma_safe_push(a2, c[0], r);
            assert(a3.take(a2.len() as int) =~= a2);
            lemma_safe_push(a3, c[1], r);
        }
    }
    assert(no_exchange_or_aside(b)) by {
        if b.len() > 2 {
            let c = b.skip(2);
            assert forall|i: int| 0 <= i < b.len() implies !(#[trigger] b[i] is Exchange
                || b[i] is RenameAside) by {
                if i >= 2 {
                    assert(b[i] == c[i - 2]);
                }
            }
        }
    }
}

/// `t` neither exchanges nor moves anything aside.
pub open spec fn no_exchange_or_aside(t: Seq<Ev>) -> bool {
    forall|i: int| 0 <= i < t.len() ==> !(#[trigger] t[i] is Exchange || t[i] is RenameAside)
}

} // verus!
