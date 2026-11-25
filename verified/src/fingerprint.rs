//! Verified kernel: content fingerprints.
//!
//! waft records a 64-bit FNV-1a fingerprint of every file it plans against
//! (`FileSnapshot::content_fingerprint`) and compares it again immediately
//! before acting, so a file rewritten between planning and execution is not
//! replaced, repaired or copied from. Production computes the fingerprint in
//! two shapes: over a whole in-memory buffer (`fingerprint_bytes`, used by the
//! default `FileSystem::file_snapshot`) and over a stream of `read()` chunks
//! (`fingerprint_reader`, used by the real filesystem). Both now run this
//! kernel.
//!
//! # What the kernel decides
//!
//! - [`fingerprint`]: the FNV-1a 64-bit hash of a byte slice.
//! - [`Fingerprinter`]: the same hash computed incrementally over chunks;
//!   it carries the (ghost) bytes it has consumed.
//!
//! # Theorems
//!
//! - `fingerprint` returns `spec_fingerprint(bytes)`, the FNV-1a fold from
//!   [`FNV_OFFSET`] (xor each byte in, then multiply by [`FNV_PRIME`] modulo
//!   2^64). Overflow is impossible by construction (wrapping arithmetic is the
//!   specification).
//! - `Fingerprinter::finish` returns `spec_fingerprint` of the concatenation
//!   of every chunk passed to `update`, in order, so the streamed fingerprint
//!   of a file read in any sequence of short reads equals the whole-buffer
//!   fingerprint of the same bytes. This rests on `lemma_fold_concat`
//!   (`fold(h, a ++ b) == fold(fold(h, a), b)`) and is restated over an
//!   arbitrary list of chunks as `theorem_chunking_invariance`.
//! - `example_known_answers`: the empty input hashes to the offset basis, and
//!   `"a"` and `"foobar"` hash to the published FNV-1a 64 test vectors
//!   (`0xaf63dc4c8601ec8c`, `0x85944171f73967e8`), checked by evaluation.
//! - `lemma_step_injective_in_state` / `lemma_fold_injective_in_state`:
//!   folding the same bytes from two different states never meets, because
//!   [`FNV_PRIME`] is odd and so has the inverse [`FNV_PRIME_INVERSE`]
//!   modulo 2^64 (`lemma_prime_inverse`).
//! - `theorem_single_byte_change_detected`: two inputs of the same length
//!   that differ in exactly one byte always have different fingerprints. A
//!   same-length, in-place rewrite of one byte is therefore always caught by
//!   the snapshot comparison, whatever the rest of the file holds. (Arbitrary
//!   multi-byte collisions remain possible: FNV-1a is not cryptographic.)
//!
//! # Trust assumptions
//!
//! - Production passes `&buffer[..count]` for each successful `read()` that
//!   returned `count > 0`, in order, and calls `finish` at end of file, so the
//!   consumed bytes are the bytes `read()` delivered.
//! - Fingerprint equality does not imply byte equality (64-bit,
//!   non-cryptographic): equal fingerprints are a concurrency guard, as
//!   `ASSURANCE.md` states, not a proof of equal content.

#[allow(unused_imports)]
use vstd::prelude::*;

verus! {

/// FNV-1a 64-bit offset basis: the fingerprint of the empty input.
pub const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;

/// FNV-1a 64-bit prime.
pub const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// The multiplicative inverse of [`FNV_PRIME`] modulo 2^64 (used only in
/// proofs).
pub const FNV_PRIME_INVERSE: u64 = 0xce96_5057_aff6_957b;

/// One FNV-1a step: xor the byte into the state, then multiply by the prime
/// modulo 2^64.
pub open spec fn fnv_step(state: u64, byte: u8) -> u64 {
    (state ^ (byte as u64)).wrapping_mul(FNV_PRIME)
}

/// FNV-1a folded over `bytes` from `state`, first byte first.
pub open spec fn fnv_fold(state: u64, bytes: Seq<u8>) -> u64
    decreases bytes.len(),
{
    if bytes.len() == 0 {
        state
    } else {
        fnv_step(fnv_fold(state, bytes.drop_last()), bytes.last())
    }
}

/// The FNV-1a 64-bit fingerprint of `bytes`.
pub open spec fn spec_fingerprint(bytes: Seq<u8>) -> u64 {
    fnv_fold(FNV_OFFSET, bytes)
}

/// Fold a list of chunks one after another, as a streaming reader does.
pub open spec fn fold_chunks(state: u64, chunks: Seq<Seq<u8>>) -> u64
    decreases chunks.len(),
{
    if chunks.len() == 0 {
        state
    } else {
        fnv_fold(fold_chunks(state, chunks.drop_last()), chunks.last())
    }
}

/// Folding `a ++ b` is folding `a`, then continuing with `b`.
pub proof fn lemma_fold_concat(state: u64, a: Seq<u8>, b: Seq<u8>)
    ensures
        fnv_fold(state, a + b) == fnv_fold(fnv_fold(state, a), b),
    decreases b.len(),
{
    if b.len() == 0 {
        assert(a + b =~= a);
    } else {
        assert((a + b).drop_last() =~= a + b.drop_last());
        assert((a + b).last() == b.last());
        lemma_fold_concat(state, a, b.drop_last());
    }
}

/// Chunking invariance: folding any list of chunks one by one gives the
/// fingerprint state of their concatenation.
pub proof fn theorem_chunking_invariance(state: u64, chunks: Seq<Seq<u8>>)
    ensures
        fold_chunks(state, chunks) == fnv_fold(state, chunks.flatten()),
    decreases chunks.len(),
{
    chunks.lemma_flatten_and_flatten_alt_are_equivalent();
    if chunks.len() == 0 {
    } else {
        theorem_chunking_invariance(state, chunks.drop_last());
        chunks.drop_last().lemma_flatten_and_flatten_alt_are_equivalent();
        lemma_fold_concat(state, chunks.drop_last().flatten_alt(), chunks.last());
    }
}

/// Known-answer checks against the published FNV-1a 64 test vectors.
pub proof fn example_known_answers()
    ensures
        spec_fingerprint(Seq::<u8>::empty()) == FNV_OFFSET,
        spec_fingerprint(seq![0x61u8]) == 0xaf63_dc4c_8601_ec8cu64,
        spec_fingerprint(seq![0x66u8, 0x6f, 0x6f, 0x62, 0x61, 0x72])
            == 0x8594_4171_f739_67e8u64,
{
    assert(spec_fingerprint(seq![0x61u8]) == 0xaf63_dc4c_8601_ec8cu64) by (compute_only);
    assert(spec_fingerprint(seq![0x66u8, 0x6f, 0x6f, 0x62, 0x61, 0x72])
        == 0x8594_4171_f739_67e8u64) by (compute_only);
}

/// [`FNV_PRIME_INVERSE`] undoes multiplication by [`FNV_PRIME`] modulo 2^64.
pub proof fn lemma_prime_inverse(x: u64)
    ensures
        x.wrapping_mul(FNV_PRIME).wrapping_mul(FNV_PRIME_INVERSE) == x,
{
    let m: int = 0x1_0000_0000_0000_0000;
    let p: int = FNV_PRIME as int;
    let q: int = FNV_PRIME_INVERSE as int;
    let k: int = 0x00ce_9650_590f;
    assert(0x0000_0100_0000_01b3int * 0xce96_5057_aff6_957bint
        == 0x1_0000_0000_0000_0000int * 0x00ce_9650_590fint + 1) by (compute_only);
    let y: int = (x as int * p) % m;
    assert(x.wrapping_mul(FNV_PRIME) as int == y);
    vstd::arithmetic::div_mod::lemma_mul_mod_noop_left(x as int * p, q, m);
    vstd::arithmetic::mul::lemma_mul_is_associative(x as int, p, q);
    vstd::arithmetic::mul::lemma_mul_is_distributive_add(x as int, m * k, 1);
    vstd::arithmetic::mul::lemma_mul_is_associative(x as int, m, k);
    vstd::arithmetic::mul::lemma_mul_is_commutative(x as int, m);
    vstd::arithmetic::mul::lemma_mul_is_associative(m, x as int, k);
    vstd::arithmetic::div_mod::lemma_mod_multiples_vanish(x as int * k, x as int, m);
    vstd::arithmetic::div_mod::lemma_small_mod(x as nat, m as nat);
    assert((y * q) % m == x as int);
}

/// One step is injective in the state: different states stay different after
/// the same byte.
pub proof fn lemma_step_injective_in_state(s1: u64, s2: u64, byte: u8)
    requires
        fnv_step(s1, byte) == fnv_step(s2, byte),
    ensures
        s1 == s2,
{
    lemma_prime_inverse(s1 ^ (byte as u64));
    lemma_prime_inverse(s2 ^ (byte as u64));
    let b = byte as u64;
    assert(s1 ^ b == s2 ^ b);
    assert((s1 ^ b) ^ b == s1) by (bit_vector);
    assert((s2 ^ b) ^ b == s2) by (bit_vector);
}

/// One step is injective in the byte: from the same state, different bytes
/// give different states.
pub proof fn lemma_step_injective_in_byte(state: u64, b1: u8, b2: u8)
    requires
        fnv_step(state, b1) == fnv_step(state, b2),
    ensures
        b1 == b2,
{
    lemma_prime_inverse(state ^ (b1 as u64));
    lemma_prime_inverse(state ^ (b2 as u64));
    let x1 = b1 as u64;
    let x2 = b2 as u64;
    assert(state ^ x1 == state ^ x2);
    assert((state ^ x1) ^ state == x1) by (bit_vector);
    assert((state ^ x2) ^ state == x2) by (bit_vector);
}

/// Folding the same bytes from two states agrees only if the states agree.
pub proof fn lemma_fold_injective_in_state(s1: u64, s2: u64, bytes: Seq<u8>)
    requires
        fnv_fold(s1, bytes) == fnv_fold(s2, bytes),
    ensures
        s1 == s2,
    decreases bytes.len(),
{
    if bytes.len() > 0 {
        lemma_step_injective_in_state(
            fnv_fold(s1, bytes.drop_last()),
            fnv_fold(s2, bytes.drop_last()),
            bytes.last(),
        );
        lemma_fold_injective_in_state(s1, s2, bytes.drop_last());
    }
}

/// Two inputs of the same length that differ in exactly one byte never share
/// a fingerprint.
pub proof fn theorem_single_byte_change_detected(a: Seq<u8>, b: Seq<u8>, i: int)
    requires
        a.len() == b.len(),
        0 <= i < a.len(),
        a[i] != b[i],
        forall|j: int| 0 <= j < a.len() && j != i ==> a[j] == b[j],
    ensures
        spec_fingerprint(a) != spec_fingerprint(b),
{
    let n = a.len() as int;
    let prefix_a = a.subrange(0, i);
    let prefix_b = b.subrange(0, i);
    let suffix_a = a.subrange(i + 1, n);
    let suffix_b = b.subrange(i + 1, n);
    assert(prefix_a =~= prefix_b);
    assert(suffix_a =~= suffix_b);
    assert(a =~= prefix_a.push(a[i]) + suffix_a);
    assert(b =~= prefix_b.push(b[i]) + suffix_b);
    lemma_fold_concat(FNV_OFFSET, prefix_a.push(a[i]), suffix_a);
    lemma_fold_concat(FNV_OFFSET, prefix_b.push(b[i]), suffix_b);
    assert(prefix_a.push(a[i]).drop_last() =~= prefix_a);
    assert(prefix_b.push(b[i]).drop_last() =~= prefix_b);
    let h = fnv_fold(FNV_OFFSET, prefix_a);
    if spec_fingerprint(a) == spec_fingerprint(b) {
        lemma_fold_injective_in_state(fnv_step(h, a[i]), fnv_step(h, b[i]), suffix_a);
        lemma_step_injective_in_byte(h, a[i], b[i]);
    }
}

/// Continue an FNV-1a fold from `state` over `chunk`.
pub fn extend(state: u64, chunk: &[u8]) -> (r: u64)
    ensures
        r == fnv_fold(state, chunk@),
{
    let mut h = state;
    let mut i: usize = 0;
    while i < chunk.len()
        invariant
            i <= chunk@.len(),
            h == fnv_fold(state, chunk@.subrange(0, i as int)),
        decreases chunk.len() - i,
    {
        assert(chunk@.subrange(0, i + 1).drop_last() =~= chunk@.subrange(0, i as int));
        h = (h ^ (chunk[i] as u64)).wrapping_mul(FNV_PRIME);
        i += 1;
    }
    assert(chunk@.subrange(0, chunk@.len() as int) =~= chunk@);
    h
}

/// The FNV-1a 64-bit fingerprint of `bytes`.
pub fn fingerprint(bytes: &[u8]) -> (r: u64)
    ensures
        r == spec_fingerprint(bytes@),
{
    extend(FNV_OFFSET, bytes)
}

/// An FNV-1a fingerprint computed incrementally over successive chunks.
pub struct Fingerprinter {
    state: u64,
    #[allow(dead_code)]
    consumed: Ghost<Seq<u8>>,
}

impl Fingerprinter {
    #[verifier::type_invariant]
    closed spec fn inv(&self) -> bool {
        self.state == spec_fingerprint(self.consumed@)
    }

    /// Every byte passed to [`Fingerprinter::update`] so far, in order.
    pub closed spec fn consumed(&self) -> Seq<u8> {
        self.consumed@
    }

    /// A fingerprinter that has consumed nothing.
    pub fn new() -> (r: Fingerprinter)
        ensures
            r.consumed() == Seq::<u8>::empty(),
    {
        Fingerprinter { state: FNV_OFFSET, consumed: Ghost(Seq::empty()) }
    }

    /// Consume the next chunk.
    pub fn update(&mut self, chunk: &[u8])
        ensures
            final(self).consumed() == old(self).consumed() + chunk@,
    {
        proof {
            use_type_invariant(&*self);
            lemma_fold_concat(FNV_OFFSET, self.consumed@, chunk@);
        }
        let next = extend(self.state, chunk);
        let ghost consumed = self.consumed@ + chunk@;
        *self = Fingerprinter { state: next, consumed: Ghost(consumed) };
    }

    /// The fingerprint of everything consumed so far.
    pub fn finish(&self) -> (r: u64)
        ensures
            r == spec_fingerprint(self.consumed()),
    {
        proof {
            use_type_invariant(self);
        }
        self.state
    }
}

impl Default for Fingerprinter {
    fn default() -> (r: Fingerprinter)
        ensures
            r.consumed() == Seq::<u8>::empty(),
    {
        Fingerprinter::new()
    }
}

} // verus!
