//! An abstract filesystem model of the names a publication acts on, and the
//! filesystem-level safety theorems over every admissible interleaving of
//! waft's steps with arbitrary steps of other processes.
//!
//! # The model
//!
//! `Fs` maps each of the three names a publication touches (the destination,
//! the staging name, the displaced name; all under one directory handle) to
//! the inode it holds; an absent key is a vacant name. A *schedule* for a trace
//! `t` (the ghost trace a kernel machine records, see
//! [`crate::publish::trace`]) is the sequence `pre` of namespaces seen right
//! before each of waft's steps. Between `effect(pre[i], t[i])` and `pre[i + 1]`
//! the environment may change the namespace **arbitrarily** — rename, link,
//! unlink, replace any of the three names — subject only to the axioms below.
//!
//! # Trusted definitions
//!
//! These spec functions are the trust base of this layer. They are ghost
//! definitions, not assumptions the verifier is told to accept, and state POSIX
//! semantics as waft relies on them:
//!
//! * `effect`: a successful `renameat2(RENAME_NOREPLACE)` moves the
//!   source's inode onto the vacant destination; a successful `linkat` adds the
//!   destination as a second name; a successful `unlinkat` removes the name; a
//!   successful `RENAME_EXCHANGE` swaps the staging and destination entries; a
//!   successful move aside renames the destination onto the displaced name.
//!   Probes, read-backs, `fchmod`, `fsync`, and every failed syscall change no
//!   name.
//! * `consistent`: an answer production fed back is possible in the
//!   namespace it ran against — a probe that said "holds" saw that inode, a
//!   successful no-clobber rename or link found the destination vacant, a
//!   successful exchange found both names present, and so on.
//!
//! # Schedules
//!
//! `consistent_run`: every answer production fed back is consistent with the
//! namespace it ran against. Beyond that, an admissible schedule
//! (`admissible`) satisfies two axioms:
//!
//! * **A-window** (`window_quiet`): nothing else acts on the namespace between
//!   a successful probe and an unlink that immediately follows it. POSIX has no
//!   conditional unlink, so that two-syscall gap between proving a name and
//!   unlinking it is irreducible; this axiom states it rather than hiding it.
//!   Every other gap — between a probe and the read-back or exchange after it,
//!   between an exchange and the probe that follows, and so on — is open to the
//!   environment.
//! * **A-fresh** (`aside_fresh`): the displaced name is vacant whenever waft
//!   moves the destination onto it — it is a fresh 128-bit random name nobody
//!   else has a reason to create.
//!
//! `theorem_only_own_or_verified_lost` additionally assumes **A-link**
//! (`link_window_quiet`): nothing else acts between a successful fallback
//! hard link and the probe of the extra name right after it. Production
//! defends that window (a re-pointed name is left alone and reported), so
//! this axiom is only needed for the stronger conclusion, not for safety.
//!
//! # Theorems
//!
//! * `theorem_no_foreign_loss` (**T1**): for every safe trace and every
//!   admissible schedule, any inode that one of waft's steps makes vanish from
//!   the namespace is one the step's immediately preceding probe proved under
//!   the name it unlinked, and that the run was `entitled` to remove there:
//!   its own staging file, the verified planned destination after the
//!   read-back that followed moving it, or an inode proved under a name right
//!   after waft's own fallback hard link from that name succeeded. In the last
//!   case T1 does not say which inode that is — only T1′, with A-link, ties it
//!   to the inode the link published at the destination, which then only
//!   vanishes if another process removed it from the destination in between:
//!   the hard-link fallback cannot do better than POSIX lets it.
//! * `theorem_only_own_or_verified_lost` (**T1′**): with A-link as well,
//!   the only inodes waft ever makes vanish are its own staging file and the
//!   verified planned destination; a third writer's file never vanishes
//!   through a waft step.
//! * `theorem_dest_never_clobbered` (**T2**): whatever the destination name
//!   holds right before any of waft's steps is still in the namespace right
//!   after it — a waft step can move it aside or swap it out, never drop it.
//!   This needs neither axiom: only `consistent_run` and `safe`.
//! * `theorem_noreplace_keeps_dest` (**NR5**): a no-clobber publication
//!   never changes an occupied destination entry at all (again from
//!   `consistent_run` and `safe` alone).
//! * `theorem_repair_keeps_namespace` (**T3**): a repair changes no name. The
//!   repair kernel has no namespace-changing step at all, so this states a
//!   property of its event alphabet rather than of any decision it makes.
//!
//! Every finished kernel record ensures `safe` of its trace (or
//! `no_namespace_change` for a repair), so each theorem applies to every run
//! the kernel can produce.

#[allow(unused_imports)]
use vstd::prelude::*;

#[allow(unused_imports)]
use crate::publish::trace::{Ev, Ino, Name, Res, Run};
#[cfg(verus_keep_ghost)]
use crate::publish::trace::{
    entitled, is_unlink, no_exchange_or_aside, no_namespace_change, safe, unlink_proven_at,
};

verus! {

/// The namespace one publication can see: which inode each of its names
/// holds.
pub struct Fs {
    /// Name to inode; an absent key is a vacant name.
    pub at: Map<Name, Ino>,
}

/// The namespace change of one of waft's steps (trusted POSIX semantics).
pub open spec fn effect(fs: Fs, e: Ev) -> Fs {
    match e {
        Ev::RenameNoReplace { from, res: Res::Ok } => Fs {
            at: fs.at.insert(Name::Dest, fs.at[from]).remove(from),
        },
        Ev::Link { from, ok: true } => Fs { at: fs.at.insert(Name::Dest, fs.at[from]) },
        Ev::Unlink { name, ok: true } => Fs { at: fs.at.remove(name) },
        Ev::Exchange { res: Res::Ok } => Fs {
            at: fs.at.insert(Name::Dest, fs.at[Name::Staging]).insert(
                Name::Staging,
                fs.at[Name::Dest],
            ),
        },
        Ev::RenameAside { ok: true } => Fs {
            at: fs.at.insert(Name::Displaced, fs.at[Name::Dest]).remove(Name::Dest),
        },
        _ => fs,
    }
}

/// The answer recorded in `e` is possible in the namespace it ran against
/// (trusted POSIX semantics).
pub open spec fn consistent(fs: Fs, e: Ev) -> bool {
    match e {
        Ev::Probe { name, ino, holds: true } => fs.at.contains_key(name) && fs.at[name] == ino,
        Ev::Lstat { name, found: Some(x) } => fs.at.contains_key(name) && fs.at[name] == x,
        Ev::RenameNoReplace { from, res: Res::Ok } => fs.at.contains_key(from)
            && !fs.at.contains_key(Name::Dest),
        Ev::Link { from, ok: true } => fs.at.contains_key(from) && !fs.at.contains_key(Name::Dest),
        Ev::Unlink { name, ok: true } => fs.at.contains_key(name),
        Ev::Exchange { res: Res::Ok } => fs.at.contains_key(Name::Staging) && fs.at.contains_key(
            Name::Dest,
        ),
        Ev::RenameAside { ok: true } => fs.at.contains_key(Name::Dest),
        _ => true,
    }
}

/// Trigger wrapper: the step at `i` is a successful probe.
pub open spec fn probe_held(e: Ev) -> bool {
    e matches Ev::Probe { holds, .. } && holds
}

/// `pre` is a schedule for `t` in which every recorded answer is consistent
/// with the namespace it ran against.
pub open spec fn consistent_run(t: Seq<Ev>, pre: Seq<Fs>) -> bool {
    &&& pre.len() == t.len()
    &&& forall|i: int| 0 <= i < t.len() ==> consistent(#[trigger] pre[i], t[i])
}

/// A-window: nothing else acts on the namespace between a successful probe
/// and an unlink issued immediately after it.
pub open spec fn window_quiet(t: Seq<Ev>, pre: Seq<Fs>) -> bool {
    forall|i: int|
        0 <= i && i + 1 < t.len() && #[trigger] probe_held(t[i]) && t[i + 1] is Unlink ==> pre[i
            + 1] == effect(pre[i], t[i])
}

/// A-fresh: the displaced name is vacant whenever the destination is moved
/// onto it.
pub open spec fn aside_fresh(t: Seq<Ev>, pre: Seq<Fs>) -> bool {
    forall|i: int|
        0 <= i < t.len() && #[trigger] t[i] is RenameAside ==> !pre[i].at.contains_key(
            Name::Displaced,
        )
}

/// `pre` is an admissible schedule for `t`: every recorded answer is
/// consistent with the namespace it ran against, and the axioms A-window and
/// A-fresh hold. The environment is otherwise unconstrained.
pub open spec fn admissible(t: Seq<Ev>, pre: Seq<Fs>) -> bool {
    &&& consistent_run(t, pre)
    &&& window_quiet(t, pre)
    &&& aside_fresh(t, pre)
}

/// Some name in `fs` holds inode `x`.
pub open spec fn present(fs: Fs, x: Ino) -> bool {
    exists|n: Name| #[trigger] fs.at.contains_key(n) && fs.at[n] == x
}

/// The step at `i` made `x` vanish from the namespace.
pub open spec fn lost_at(t: Seq<Ev>, pre: Seq<Fs>, i: int, x: Ino) -> bool {
    present(pre[i], x) && !present(effect(pre[i], t[i]), x)
}

/// The step at `i` unlinked a name it had just proved to hold `x`, and the
/// run was entitled to remove `x` from that name.
pub open spec fn removed_as_entitled(t: Seq<Ev>, i: int, x: Ino, r: Run) -> bool {
    &&& 1 <= i < t.len()
    &&& t[i] matches Ev::Unlink { name, ok }
    &&& ok
    &&& t[i - 1] == (Ev::Probe { name, ino: x, holds: true })
    &&& entitled(t.take(i - 1), name, x, r.own, r.verified)
}

proof fn lemma_present_witness(fs: Fs, n: Name, x: Ino)
    requires
        fs.at.contains_key(n),
        fs.at[n] == x,
    ensures
        present(fs, x),
{
}

/// Every step other than a successful unlink keeps every present inode
/// present.
proof fn lemma_step_keeps(t: Seq<Ev>, pre: Seq<Fs>, i: int, x: Ino)
    requires
        admissible(t, pre),
        0 <= i < t.len(),
        present(pre[i], x),
        !(t[i] matches Ev::Unlink { ok, .. } && ok),
    ensures
        present(effect(pre[i], t[i]), x),
{
    let fs = pre[i];
    let e = t[i];
    let post = effect(fs, e);
    assert(consistent(pre[i], t[i]));
    let n = choose|n: Name| #[trigger] fs.at.contains_key(n) && fs.at[n] == x;
    match e {
        Ev::RenameNoReplace { from, res: Res::Ok } => {
            if n == from {
                lemma_present_witness(post, Name::Dest, x);
            } else {
                assert(n != Name::Dest);
                lemma_present_witness(post, n, x);
            }
        },
        Ev::Link { from, ok: true } => {
            assert(n != Name::Dest);
            lemma_present_witness(post, n, x);
        },
        Ev::Exchange { res: Res::Ok } => {
            if n == Name::Dest {
                lemma_present_witness(post, Name::Staging, x);
            } else if n == Name::Staging {
                lemma_present_witness(post, Name::Dest, x);
            } else {
                lemma_present_witness(post, n, x);
            }
        },
        Ev::RenameAside { ok: true } => {
            assert(!fs.at.contains_key(Name::Displaced));
            if n == Name::Dest {
                lemma_present_witness(post, Name::Displaced, x);
            } else {
                assert(n == Name::Staging);
                lemma_present_witness(post, n, x);
            }
        },
        _ => {
            assert(post == fs);
            lemma_present_witness(post, n, x);
        },
    }
}

/// **T1**: any inode one of waft's steps makes vanish from the namespace was
/// proved by the probe right before that step, under the name the step
/// unlinked, and was the run's to remove.
pub proof fn theorem_no_foreign_loss(t: Seq<Ev>, pre: Seq<Fs>, r: Run)
    requires
        safe(t, r),
        admissible(t, pre),
    ensures
        forall|i: int, x: Ino|
            0 <= i < t.len() && #[trigger] lost_at(t, pre, i, x) ==> removed_as_entitled(t, i, x, r),
{
    assert forall|i: int, x: Ino| 0 <= i < t.len() && #[trigger] lost_at(t, pre, i, x) implies removed_as_entitled(
        t,
        i,
        x,
        r,
    ) by {
        if !(t[i] matches Ev::Unlink { ok, .. } && ok) {
            lemma_step_keeps(t, pre, i, x);
        } else {
            let name = t[i]->Unlink_name;
            assert(is_unlink(t[i]));
            assert(unlink_proven_at(t, i, r.own, r.verified));
            let y = t[i - 1]->Probe_ino;
            assert(probe_held(t[i - 1]));
            assert(consistent(pre[i - 1], t[i - 1]));
            assert(pre[i] == effect(pre[i - 1], t[i - 1]));
            assert(pre[i] == pre[i - 1]);
            assert(pre[i].at[name] == y);
            // Only `name` changed, so a lost inode is the one `name` held.
            let fs = pre[i];
            let post = effect(fs, t[i]);
            let n = choose|n: Name| #[trigger] fs.at.contains_key(n) && fs.at[n] == x;
            if n != name {
                lemma_present_witness(post, n, x);
            }
            assert(x == y);
        }
    }
}

/// Trigger wrapper: `e` is a successful hard link.
pub open spec fn link_held(e: Ev) -> bool {
    e matches Ev::Link { ok, .. } && ok
}

/// A-link: nothing else acts on the namespace between a successful fallback
/// hard link and the waft step right after it.
pub open spec fn link_window_quiet(t: Seq<Ev>, pre: Seq<Fs>) -> bool {
    forall|i: int|
        0 <= i && i + 1 < t.len() && #[trigger] link_held(t[i]) ==> pre[i + 1] == effect(
            pre[i],
            t[i],
        )
}

/// **T1′**: with A-link as well, an inode one of waft's steps makes vanish is
/// the run's own staging file or the verified planned destination — never
/// anything else.
pub proof fn theorem_only_own_or_verified_lost(t: Seq<Ev>, pre: Seq<Fs>, r: Run)
    requires
        safe(t, r),
        admissible(t, pre),
        link_window_quiet(t, pre),
    ensures
        forall|i: int, x: Ino|
            0 <= i < t.len() && #[trigger] lost_at(t, pre, i, x) ==> r.own == Some(x)
                || r.verified == Some(x),
{
    theorem_no_foreign_loss(t, pre, r);
    assert forall|i: int, x: Ino| 0 <= i < t.len() && #[trigger] lost_at(t, pre, i, x) implies r.own
        == Some(x) || r.verified == Some(x) by {
        assert(removed_as_entitled(t, i, x, r));
        let name = t[i]->Unlink_name;
        let before = t.take(i - 1);
        if r.own != Some(x) && r.verified != Some(x) {
            // Only the link clause is left: the step two before is this
            // name's own successful hard link onto the destination.
            assert(before.len() > 0 && before.last() == (Ev::Link { from: name, ok: true }));
            let j = i - 2;
            assert(t[j] == before.last());
            assert(link_held(t[j]));
            assert(consistent(pre[j], t[j]));
            assert(pre[j + 1] == effect(pre[j], t[j]));
            assert(probe_held(t[i - 1]));
            assert(consistent(pre[i - 1], t[i - 1]));
            assert(pre[i] == effect(pre[i - 1], t[i - 1]));
            // The link put the inode the name held onto the destination, the
            // probe saw `x` under the name, and nothing moved in between: the
            // destination still holds `x` when the name is unlinked.
            assert(pre[i].at[Name::Dest] == x);
            lemma_present_witness(effect(pre[i], t[i]), Name::Dest, x);
        }
    }
}

/// Every step other than a successful unlink keeps whatever the destination
/// held somewhere in the namespace — from consistency alone: no rename or
/// link onto an occupied destination succeeds, and an exchange or a move
/// aside carries the destination's inode to another name.
proof fn lemma_dest_kept(t: Seq<Ev>, pre: Seq<Fs>, i: int)
    requires
        consistent_run(t, pre),
        0 <= i < t.len(),
        pre[i].at.contains_key(Name::Dest),
        !(t[i] matches Ev::Unlink { ok, .. } && ok),
    ensures
        present(effect(pre[i], t[i]), pre[i].at[Name::Dest]),
{
    let fs = pre[i];
    let x = fs.at[Name::Dest];
    let post = effect(fs, t[i]);
    assert(consistent(pre[i], t[i]));
    match t[i] {
        Ev::Exchange { res: Res::Ok } => {
            lemma_present_witness(post, Name::Staging, x);
        },
        Ev::RenameAside { ok: true } => {
            lemma_present_witness(post, Name::Displaced, x);
        },
        _ => {
            assert(post == fs);
            lemma_present_witness(post, Name::Dest, x);
        },
    }
}

/// **T2**: whatever the destination holds right before any of waft's steps is
/// still in the namespace right after it. Needs neither A-window nor
/// A-fresh: only that every answer was consistent.
pub proof fn theorem_dest_never_clobbered(t: Seq<Ev>, pre: Seq<Fs>, r: Run)
    requires
        safe(t, r),
        consistent_run(t, pre),
    ensures
        forall|i: int|
            0 <= i < t.len() && (#[trigger] pre[i]).at.contains_key(Name::Dest) ==> present(
                effect(pre[i], t[i]),
                pre[i].at[Name::Dest],
            ),
{
    assert forall|i: int| 0 <= i < t.len() && (#[trigger] pre[i]).at.contains_key(Name::Dest) implies present(
        effect(pre[i], t[i]),
        pre[i].at[Name::Dest],
    ) by {
        let x = pre[i].at[Name::Dest];
        lemma_present_witness(pre[i], Name::Dest, x);
        if t[i] matches Ev::Unlink { ok, .. } && ok {
            assert(is_unlink(t[i]));
            assert(unlink_proven_at(t, i, r.own, r.verified));
            let name = t[i]->Unlink_name;
            assert(name != Name::Dest);
            lemma_present_witness(effect(pre[i], t[i]), Name::Dest, x);
        } else {
            lemma_dest_kept(t, pre, i);
        }
    }
}

/// **NR5**: a trace with no exchange and no move-aside — every no-clobber
/// publication — never changes an occupied destination entry. Like T2, this
/// needs only consistent answers.
pub proof fn theorem_noreplace_keeps_dest(t: Seq<Ev>, pre: Seq<Fs>, r: Run)
    requires
        safe(t, r),
        no_exchange_or_aside(t),
        consistent_run(t, pre),
    ensures
        forall|i: int|
            0 <= i < t.len() && (#[trigger] pre[i]).at.contains_key(Name::Dest) ==> {
                &&& effect(pre[i], t[i]).at.contains_key(Name::Dest)
                &&& effect(pre[i], t[i]).at[Name::Dest] == pre[i].at[Name::Dest]
            },
{
    assert forall|i: int| 0 <= i < t.len() && (#[trigger] pre[i]).at.contains_key(Name::Dest) implies {
        &&& effect(pre[i], t[i]).at.contains_key(Name::Dest)
        &&& effect(pre[i], t[i]).at[Name::Dest] == pre[i].at[Name::Dest]
    } by {
        assert(consistent(pre[i], t[i]));
        assert(!(t[i] is Exchange || t[i] is RenameAside));
        if t[i] matches Ev::Unlink { ok, .. } && ok {
            assert(is_unlink(t[i]));
            assert(unlink_proven_at(t, i, r.own, r.verified));
        }
    }
}

/// **T3**: a trace that asks for no namespace change — every repair — leaves
/// every name exactly as the environment left it.
pub proof fn theorem_repair_keeps_namespace(t: Seq<Ev>, pre: Seq<Fs>)
    requires
        no_namespace_change(t),
        pre.len() == t.len(),
    ensures
        forall|i: int| 0 <= i < t.len() ==> effect(#[trigger] pre[i], t[i]) == pre[i],
{
    assert forall|i: int| 0 <= i < t.len() implies effect(#[trigger] pre[i], t[i]) == pre[i] by {
        assert(!(t[i] is RenameNoReplace || t[i] is Link || t[i] is Unlink || t[i] is Exchange
            || t[i] is RenameAside));
    }
}

/// Non-vacuity: the hypotheses of T1 are satisfiable by a real replacement —
/// the exchange path that removes the verified planned inode `v` — and T1's
/// conclusion is then exercised, since that removal does make `v` vanish.
pub proof fn lemma_model_witness()
    ensures
        ({
            let v: Ino = (1, 1);
            let p: Ino = (1, 2);
            let r = Run { own: Some(p), verified: Some(v), prepared: Some(p) };
            let t = seq![
                Ev::Probe { name: Name::Dest, ino: v, holds: true },
                Ev::PinnedMatches { holds: true },
                Ev::Exchange { res: Res::Ok },
                Ev::Probe { name: Name::Staging, ino: v, holds: true },
                Ev::PinnedMatches { holds: true },
                Ev::Probe { name: Name::Staging, ino: v, holds: true },
                Ev::Unlink { name: Name::Staging, ok: true },
            ];
            let before = Fs { at: Map::empty().insert(Name::Dest, v).insert(Name::Staging, p) };
            let after = Fs { at: Map::empty().insert(Name::Dest, p).insert(Name::Staging, v) };
            let pre = seq![before, before, before, after, after, after, after];
            &&& safe(t, r)
            &&& admissible(t, pre)
            &&& lost_at(t, pre, 6, v)
        }),
{
    let v: Ino = (1, 1);
    let p: Ino = (1, 2);
    let r = Run { own: Some(p), verified: Some(v), prepared: Some(p) };
    let t = seq![
        Ev::Probe { name: Name::Dest, ino: v, holds: true },
        Ev::PinnedMatches { holds: true },
        Ev::Exchange { res: Res::Ok },
        Ev::Probe { name: Name::Staging, ino: v, holds: true },
        Ev::PinnedMatches { holds: true },
        Ev::Probe { name: Name::Staging, ino: v, holds: true },
        Ev::Unlink { name: Name::Staging, ok: true },
    ];
    let before = Fs { at: Map::empty().insert(Name::Dest, v).insert(Name::Staging, p) };
    let after = Fs { at: Map::empty().insert(Name::Dest, p).insert(Name::Staging, v) };
    let pre = seq![before, before, before, after, after, after, after];
    assert(effect(before, t[2]).at =~= after.at);
    assert(crate::publish::trace::moved_then_verified(t.take(5), 2, Name::Staging, v));
    assert(unlink_proven_at(t, 6, r.own, r.verified));
    assert forall|i: int| 0 <= i < t.len() && #[trigger] is_unlink(t[i]) implies unlink_proven_at(
        t,
        i,
        r.own,
        r.verified,
    ) by {
        assert(i == 6);
    }
    assert forall|i: int|
        0 <= i < t.len() && #[trigger] crate::publish::trace::is_exchange(
            t[i],
        ) implies crate::publish::trace::exchange_proven_at(t, i, r.verified, r.prepared) by {
        assert(i == 2);
    }
    assert forall|i: int|
        0 <= i < t.len() && #[trigger] crate::publish::trace::is_link(t[i]) implies false by {}
    assert forall|i: int|
        0 <= i < t.len() && #[trigger] crate::publish::trace::is_aside(t[i]) implies false by {}
    assert forall|i: int| 0 <= i < t.len() implies consistent(#[trigger] pre[i], t[i]) by {}
    assert forall|i: int|
        0 <= i && i + 1 < t.len() && #[trigger] probe_held(t[i]) && t[i + 1] is Unlink implies pre[i
            + 1] == effect(pre[i], t[i]) by {
        assert(i == 5);
    }
    lemma_present_witness(pre[6], Name::Staging, v);
    let gone = effect(pre[6], t[6]);
    assert(gone.at =~= Map::empty().insert(Name::Dest, p));
    assert(!present(gone, v));
}

} // verus!
