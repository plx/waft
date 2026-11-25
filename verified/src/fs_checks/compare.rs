//! How two regular files compare: equal, permissions-only, or content
//! differs.
//!
//! The planner uses this verdict to decide whether an existing destination is
//! up to date, needs only a `chmod`, or needs its content replaced. Production
//! `RealFs::compare_files` opens both files stably, then drives a
//! [`StreamComparison`]: [`StreamComparison::begin`] with the two lengths and
//! permission signatures from the opening metadata, then
//! [`StreamComparison::compare_chunks`] with each pair of `read()` results,
//! and finally (after re-checking that neither file changed while it was
//! read) [`MatchedContents::verdict`]. The default
//! `FileSystem::compare_files` (used by in-memory test filesystems) calls
//! [`compare_contents`] on two whole buffers.
//!
//! # Theorems
//!
//! - `begin` returns `None` (production: `ContentDiffers`) exactly when the
//!   recorded lengths differ.
//! - `compare_chunks` reports `Differ` exactly when the two chunks differ (in
//!   length or in any byte), `End` exactly when both are empty and equal, and
//!   otherwise `More`, having appended each chunk to its side's consumed
//!   bytes. Its type invariant keeps the two consumed streams equal, so
//!   `MatchedContents::left` and `MatchedContents::right` are the same bytes:
//!   an `Equal` or `PermissionsDiffer` verdict is only reachable when the
//!   lengths agreed and every pair of chunks read was identical, i.e. both
//!   streams delivered exactly the same bytes.
//! - `MatchedContents::verdict` is `spec_verdict(true, permissions_equal)`:
//!   `Equal` iff the permission signatures are equal, else
//!   `PermissionsDiffer`. With `spec_verdict` this gives the precedence
//!   `ContentDiffers` > `PermissionsDiffer` > `Equal` (`theorem_precedence`).
//! - `theorem_aligned_difference_is_real`: when `Differ` is reported for two
//!   chunks of the same length, the two files' contents really differ,
//!   whatever either stream delivers afterwards.
//! - `example_short_read_quirk` (current behaviour, faithfully kept): chunks
//!   of *different* lengths are reported as `Differ` even when one is a prefix
//!   of the other, so two identical files whose `read()` calls return
//!   different short counts are classified `ContentDiffers`. The error is
//!   conservative (a file that is up to date is treated as a conflict, never
//!   the reverse).
//! - `compare_contents` returns `Equal` iff the buffers are equal and
//!   `ContentDiffers` otherwise; it never reports `PermissionsDiffer` (the
//!   default trait method has no permission information).
//!
//! # Trust assumptions
//!
//! - `read()` returns successive bytes of the file and `0` only at end of
//!   file; production passes exactly `&buffer[..count]` for each read, left
//!   then right, until the kernel says `Differ` or `End`.
//! - The lengths and permission signatures passed to `begin` come from the
//!   metadata of the opened handles, and production converts the verdict to
//!   `FileComparison` variant for variant.

#[allow(unused_imports)]
use vstd::prelude::*;

verus! {

/// How a source file relates to an existing destination file (production
/// `FileComparison`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Comparison {
    /// Byte-identical content and identical permission signatures.
    Equal,
    /// Byte-identical content, differing permission signatures.
    PermissionsDiffer,
    /// Differing content (length or bytes).
    ContentDiffers,
}

/// The verdict for two files whose contents are (`contents_equal`) or are
/// not equal and whose permission signatures are (`permissions_equal`) or are
/// not equal: content first, then permissions.
pub open spec fn spec_verdict(contents_equal: bool, permissions_equal: bool) -> Comparison {
    if !contents_equal {
        Comparison::ContentDiffers
    } else if permissions_equal {
        Comparison::Equal
    } else {
        Comparison::PermissionsDiffer
    }
}

/// Whether a pair of chunks counts as a difference: any difference in
/// length or bytes.
pub open spec fn chunks_differ(left: Seq<u8>, right: Seq<u8>) -> bool {
    left != right
}

/// Precedence: content differences dominate, and `Equal` needs both.
pub proof fn theorem_precedence(contents_equal: bool, permissions_equal: bool)
    ensures
        (spec_verdict(contents_equal, permissions_equal) == Comparison::Equal) == (contents_equal
            && permissions_equal),
        (spec_verdict(contents_equal, permissions_equal) == Comparison::PermissionsDiffer) == (
        contents_equal && !permissions_equal),
        (spec_verdict(contents_equal, permissions_equal) == Comparison::ContentDiffers)
            == !contents_equal,
{
}

/// A streaming comparison in progress: the recorded lengths agreed and every
/// pair of chunks read so far was identical.
pub struct StreamComparison {
    permissions_equal: bool,
    #[allow(dead_code)]
    left: Ghost<Seq<u8>>,
    #[allow(dead_code)]
    right: Ghost<Seq<u8>>,
}

/// Both streams ended after delivering identical bytes.
pub struct MatchedContents {
    permissions_equal: bool,
    #[allow(dead_code)]
    left: Ghost<Seq<u8>>,
    #[allow(dead_code)]
    right: Ghost<Seq<u8>>,
}

/// What one pair of chunks means for the comparison.
pub enum ChunkStep {
    /// The chunks differ (in length or bytes): production reports
    /// `ContentDiffers`.
    Differ,
    /// Identical, non-empty chunks: read the next pair.
    More(StreamComparison),
    /// Both streams are at end of file with identical contents.
    End(MatchedContents),
}

impl StreamComparison {
    #[verifier::type_invariant]
    closed spec fn inv(&self) -> bool {
        self.left@ == self.right@
    }

    /// The bytes the left stream has delivered so far.
    pub closed spec fn left(&self) -> Seq<u8> {
        self.left@
    }

    /// The bytes the right stream has delivered so far.
    pub closed spec fn right(&self) -> Seq<u8> {
        self.right@
    }

    /// Whether the two permission signatures were equal.
    pub closed spec fn permissions_equal(&self) -> bool {
        self.permissions_equal
    }

    /// Start comparing two files with the given recorded lengths and
    /// permission signatures; `None` means their lengths differ.
    pub fn begin(left_len: u64, right_len: u64, left_permissions: u32, right_permissions: u32) -> (r:
        Option<StreamComparison>)
        ensures
            r is None <==> left_len != right_len,
            r matches Some(c) ==> {
                &&& c.left() == Seq::<u8>::empty()
                &&& c.right() == Seq::<u8>::empty()
                &&& c.permissions_equal() == (left_permissions == right_permissions)
            },
    {
        let permissions_equal = left_permissions == right_permissions;
        if left_len != right_len {
            None
        } else {
            Some(
                StreamComparison {
                    permissions_equal,
                    left: Ghost(Seq::empty()),
                    right: Ghost(Seq::empty()),
                },
            )
        }
    }

    /// Compare the next pair of chunks, as returned by one `read()` on each
    /// file.
    pub fn compare_chunks(self, left: &[u8], right: &[u8]) -> (r: ChunkStep)
        ensures
            r is Differ <==> chunks_differ(left@, right@),
            r matches ChunkStep::More(next) ==> {
                &&& left@ == right@
                &&& left@.len() > 0
                &&& next.left() == self.left() + left@
                &&& next.right() == self.right() + right@
                &&& next.permissions_equal() == self.permissions_equal()
            },
            r matches ChunkStep::End(matched) ==> {
                &&& left@ == right@
                &&& left@.len() == 0
                &&& matched.left() == self.left()
                &&& matched.right() == self.right()
                &&& matched.permissions_equal() == self.permissions_equal()
            },
    {
        proof {
            use_type_invariant(&self);
        }
        let same = left == right;
        proof {
            if same {
                assert(left@ =~= right@);
            }
        }
        if !same {
            ChunkStep::Differ
        } else if left.is_empty() {
            ChunkStep::End(
                MatchedContents {
                    permissions_equal: self.permissions_equal,
                    left: self.left,
                    right: self.right,
                },
            )
        } else {
            let ghost left_seen = self.left@ + left@;
            let ghost right_seen = self.right@ + right@;
            ChunkStep::More(
                StreamComparison {
                    permissions_equal: self.permissions_equal,
                    left: Ghost(left_seen),
                    right: Ghost(right_seen),
                },
            )
        }
    }
}

impl MatchedContents {
    #[verifier::type_invariant]
    closed spec fn inv(&self) -> bool {
        self.left@ == self.right@
    }

    /// Everything the left stream delivered.
    pub closed spec fn left(&self) -> Seq<u8> {
        self.left@
    }

    /// Everything the right stream delivered.
    pub closed spec fn right(&self) -> Seq<u8> {
        self.right@
    }

    /// Whether the two permission signatures were equal.
    pub closed spec fn permissions_equal(&self) -> bool {
        self.permissions_equal
    }

    /// The verdict for files whose contents matched.
    pub fn verdict(&self) -> (r: Comparison)
        ensures
            self.left() == self.right(),
            r == spec_verdict(self.left() == self.right(), self.permissions_equal()),
            r != Comparison::ContentDiffers,
    {
        proof {
            use_type_invariant(self);
        }
        if self.permissions_equal {
            Comparison::Equal
        } else {
            Comparison::PermissionsDiffer
        }
    }
}

/// When two equally long chunks differ after identical histories, the files
/// differ, whatever either stream delivers next.
pub proof fn theorem_aligned_difference_is_real(
    seen: Seq<u8>,
    left: Seq<u8>,
    right: Seq<u8>,
    left_rest: Seq<u8>,
    right_rest: Seq<u8>,
)
    requires
        left.len() == right.len(),
        chunks_differ(left, right),
    ensures
        seen + left + left_rest != seen + right + right_rest,
{
    assert(exists|i: int| 0 <= i < left.len() && left[i] != right[i]) by {
        if forall|i: int| 0 <= i < left.len() ==> left[i] == right[i] {
            assert(left =~= right);
        }
    }
    let i = choose|i: int| 0 <= i < left.len() && left[i] != right[i];
    let k = seen.len() + i;
    assert((seen + left + left_rest)[k] == left[i]);
    assert((seen + right + right_rest)[k] == right[i]);
}

/// The chunk-length quirk: a one-byte read of `[7]` against a two-byte read
/// of `[7, 7]` counts as a difference although the left file may continue
/// with another `7`, so identical files read with different short counts
/// compare as `ContentDiffers`.
pub proof fn example_short_read_quirk()
    ensures
        chunks_differ(seq![7u8], seq![7u8, 7u8]),
        seq![7u8] + seq![7u8] == seq![7u8, 7u8],
{
    assert(seq![7u8].len() != seq![7u8, 7u8].len());
    assert(seq![7u8] + seq![7u8] =~= seq![7u8, 7u8]);
}

/// The default `FileSystem::compare_files` verdict for two whole contents.
pub fn compare_contents(left: &[u8], right: &[u8]) -> (r: Comparison)
    ensures
        r == spec_verdict(left@ == right@, true),
        r != Comparison::PermissionsDiffer,
{
    let same = left == right;
    proof {
        if same {
            assert(left@ =~= right@);
        }
    }
    if same {
        Comparison::Equal
    } else {
        Comparison::ContentDiffers
    }
}

} // verus!
