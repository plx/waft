//! Which `renameat2` errnos mean "this rename flag is not supported here".
//!
//! Publication first tries `RENAME_EXCHANGE` (replacing a destination) or
//! `RENAME_NOREPLACE` (creating one). When the filesystem answers that the
//! flag is unsupported, publication falls back to displacement or to a hard
//! link; any other errno ends the protocol with that error.
//! Production `rename_flag_unsupported` passes the raw errno and the table of
//! raw values for `ENOTSUP`, `EOPNOTSUPP`, `EINVAL` and `ENOSYS`.
//!
//! # Theorems
//!
//! - [`is_listed`] returns exactly `table.contains(errno)`: the fallback is
//!   chosen for an errno iff it is one of the listed values, whatever their
//!   order, duplicates (on Linux `ENOTSUP == EOPNOTSUPP`) or number.
//! - `theorem_unlisted_errno_never_falls_back`: an errno different from every
//!   table entry (for example `EEXIST`, `ENOENT`, `EACCES`, `EXDEV`) is never
//!   classified as unsupported. This only spells out the `false` half of
//!   `is_listed`'s postcondition (it follows from the definition of
//!   `Seq::contains`) and adds no assurance of its own; which errnos belong
//!   in the table is a trust assumption (below).
//!
//! # Trust assumptions
//!
//! - `rustix::io::Errno::raw_os_error` is injective (rustix stores the raw
//!   value, negated on the `linux_raw` backend), so comparing raw values is
//!   comparing `Errno`s, as production did before.
//! - That these four errnos are the right "unsupported" class on every
//!   platform and filesystem is a specification choice, not something the
//!   kernel can prove.

#[allow(unused_imports)]
use vstd::prelude::*;

verus! {

/// Whether `errno` is one of the raw values in `table`.
pub fn is_listed(errno: i32, table: &[i32]) -> (r: bool)
    ensures
        r == table@.contains(errno),
{
    let mut i: usize = 0;
    while i < table.len()
        invariant
            i <= table@.len(),
            forall|j: int| 0 <= j < i ==> table@[j] != errno,
        decreases table.len() - i,
    {
        if table[i] == errno {
            assert(table@[i as int] == errno);
            return true;
        }
        i += 1;
    }
    false
}

/// An errno that differs from every table entry never selects a fallback.
pub proof fn theorem_unlisted_errno_never_falls_back(errno: i32, table: Seq<i32>)
    requires
        forall|j: int| 0 <= j < table.len() ==> table[j] != errno,
    ensures
        !table.contains(errno),
{
}

} // verus!
