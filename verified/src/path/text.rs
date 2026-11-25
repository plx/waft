//! Text checks on repository paths, and the Windows verbatim-prefix rule.
//!
//! [`has_forbidden_text_char`] decides whether a path string contains NUL,
//! carriage return or line feed, which would corrupt waft's line-oriented
//! output. It scans the UTF-8 bytes, and `lemma_forbidden_byte_iff_char`
//! proves that scanning bytes is the same as scanning characters: in UTF-8
//! an ASCII byte only ever encodes the ASCII character with that value
//! (every byte of a multi-byte encoding has its high bit set). So the
//! kernel's result is exactly `str::contains(['\n', '\r', '\0'])`.
//!
//! [`classify_verbatim_prefix`] decides how production's Windows-only
//! `without_windows_verbatim_prefix` rewrites an extended-length path:
//! `\\?\UNC\server\share\...` becomes `\\server\share\...`, any other
//! `\\?\rest` becomes `rest`, and everything else is unchanged. The UNC
//! form takes precedence, so a UNC path is never rewritten to a relative
//! `UNC\...` path (`theorem_unc_precedence`).

#[allow(unused_imports)]
use vstd::prelude::*;
#[cfg(verus_keep_ghost)]
use vstd::utf8::*;

verus! {

/// NUL, line feed or carriage return, as a byte.
pub open spec fn is_forbidden_text_byte(b: u8) -> bool {
    b == 0 || b == 10 || b == 13
}

/// NUL, line feed or carriage return.
pub open spec fn is_forbidden_text_char(c: char) -> bool {
    c == '\0' || c == '\n' || c == '\r'
}

/// Some byte of `bytes` is NUL, line feed or carriage return.
pub open spec fn has_forbidden_byte(bytes: Seq<u8>) -> bool {
    exists|i: int| 0 <= i < bytes.len() && is_forbidden_text_byte(#[trigger] bytes[i])
}

/// Some character of `chars` is NUL, line feed or carriage return.
pub open spec fn has_forbidden_char(chars: Seq<char>) -> bool {
    exists|i: int| 0 <= i < chars.len() && is_forbidden_text_char(#[trigger] chars[i])
}

/// The bytes of a multi-byte UTF-8 encoding all have the high bit set.
proof fn lemma_high_bit_bytes(y: u8)
    ensures
        (0xC0u8 | y) >= 0x80u8,
        (0xE0u8 | y) >= 0x80u8,
        (0xF0u8 | y) >= 0x80u8,
        (0x80u8 | y) >= 0x80u8,
{
    assert((0xC0u8 | y) >= 0x80u8) by (bit_vector);
    assert((0xE0u8 | y) >= 0x80u8) by (bit_vector);
    assert((0xF0u8 | y) >= 0x80u8) by (bit_vector);
    assert((0x80u8 | y) >= 0x80u8) by (bit_vector);
}

/// The encoding of one character contains a forbidden byte exactly when the
/// character is forbidden.
proof fn lemma_scalar_forbidden(c: char)
    ensures
        has_forbidden_byte(encode_scalar(c as u32)) <==> is_forbidden_text_char(c),
{
    let v = c as u32;
    let e = encode_scalar(v);
    if has_width_1_encoding(v) {
        assert(v & 0x7F == v) by (bit_vector)
            requires
                v <= 0x7F,
        ;
        assert(e[0] as u32 == v);
        if is_forbidden_text_char(c) {
            assert(is_forbidden_text_byte(e[0]));
        }
        if has_forbidden_byte(e) {
            let i = choose|i: int| 0 <= i < e.len() && is_forbidden_text_byte(#[trigger] e[i]);
            assert(i == 0);
            assert(v == 0 || v == 10 || v == 13);
        }
    } else {
        // Every byte of a 2-, 3- or 4-byte encoding is at least 0x80.
        assert forall|i: int| 0 <= i < e.len() implies #[trigger] e[i] >= 0x80 by {
            lemma_high_bit_bytes(((v >> 6) & 0x1F) as u8);
            lemma_high_bit_bytes(((v >> 12) & 0x0F) as u8);
            lemma_high_bit_bytes(((v >> 18) & 0x7) as u8);
            lemma_high_bit_bytes((v & 0x3F) as u8);
            lemma_high_bit_bytes(((v >> 6) & 0x3F) as u8);
            lemma_high_bit_bytes(((v >> 12) & 0x3F) as u8);
        }
        assert(!is_forbidden_text_char(c)) by {
            assert(v > 0x7F);
        }
    }
}

/// UTF-8 bridge: the encoding of `chars` contains a NUL, line feed or
/// carriage return byte exactly when `chars` contains that character.
pub proof fn lemma_forbidden_byte_iff_char(chars: Seq<char>)
    ensures
        has_forbidden_byte(encode_utf8(chars)) <==> has_forbidden_char(chars),
    decreases chars.len(),
{
    if chars.len() > 0 {
        let c = chars[0];
        let rest = chars.drop_first();
        let e = encode_scalar(c as u32);
        let tail = encode_utf8(rest);
        let all = encode_utf8(chars);
        lemma_forbidden_byte_iff_char(rest);
        lemma_scalar_forbidden(c);
        assert(all == e + tail);
        if has_forbidden_byte(all) {
            let i = choose|i: int| 0 <= i < all.len() && is_forbidden_text_byte(#[trigger] all[i]);
            if i < e.len() {
                assert(e[i] == all[i]);
                assert(has_forbidden_byte(e));
                assert(is_forbidden_text_char(chars[0]));
            } else {
                assert(tail[i - e.len()] == all[i]);
                assert(has_forbidden_byte(tail));
                let j = choose|j: int| 0 <= j < rest.len() && is_forbidden_text_char(#[trigger] rest[j]);
                assert(chars[j + 1] == rest[j]);
            }
        }
        if has_forbidden_char(chars) {
            let j = choose|j: int| 0 <= j < chars.len() && is_forbidden_text_char(#[trigger] chars[j]);
            if j == 0 {
                let i = choose|i: int| 0 <= i < e.len() && is_forbidden_text_byte(#[trigger] e[i]);
                assert(all[i] == e[i]);
            } else {
                assert(rest[j - 1] == chars[j]);
                let i = choose|i: int| 0 <= i < tail.len() && is_forbidden_text_byte(#[trigger] tail[i]);
                assert(all[e.len() + i] == tail[i]);
            }
        }
    }
}

/// Whether `s` contains NUL, line feed or carriage return: exactly
/// `s.contains(['\n', '\r', '\0'])`.
pub fn has_forbidden_text_char(s: &str) -> (r: bool)
    ensures
        r == has_forbidden_char(s@),
        r == (s@.contains('\n') || s@.contains('\r') || s@.contains('\0')),
{
    let bytes = s.as_bytes();
    proof {
        lemma_forbidden_byte_iff_char(s@);
    }
    let mut i: usize = 0;
    let mut found = false;
    while i < bytes.len()
        invariant
            bytes@ == encode_utf8(s@),
            i <= bytes@.len(),
            found == exists|k: int| 0 <= k < i && is_forbidden_text_byte(#[trigger] bytes@[k]),
        decreases bytes.len() - i,
    {
        let b = bytes[i];
        if b == 0 || b == 10 || b == 13 {
            found = true;
        }
        i += 1;
    }
    proof {
        if has_forbidden_char(s@) {
            let j = choose|j: int| 0 <= j < s@.len() && is_forbidden_text_char(#[trigger] s@[j]);
            if s@[j] == '\n' {
                assert(s@.contains('\n'));
            } else if s@[j] == '\r' {
                assert(s@.contains('\r'));
            } else {
                assert(s@.contains('\0'));
            }
        }
        if s@.contains('\n') {
            let j = choose|j: int| 0 <= j < s@.len() && s@[j] == '\n';
            assert(is_forbidden_text_char(s@[j]));
        }
        if s@.contains('\r') {
            let j = choose|j: int| 0 <= j < s@.len() && s@[j] == '\r';
            assert(is_forbidden_text_char(s@[j]));
        }
        if s@.contains('\0') {
            let j = choose|j: int| 0 <= j < s@.len() && s@[j] == '\0';
            assert(is_forbidden_text_char(s@[j]));
        }
    }
    found
}

// ---------------------------------------------------------------------------
// Windows verbatim prefixes
// ---------------------------------------------------------------------------

/// `\\?\` as bytes.
pub open spec fn verbatim_marker() -> Seq<u8> {
    seq![92u8, 92u8, 63u8, 92u8]
}

/// `\\?\UNC\` as bytes.
pub open spec fn verbatim_unc_marker() -> Seq<u8> {
    seq![92u8, 92u8, 63u8, 92u8, 85u8, 78u8, 67u8, 92u8]
}

/// `p` is a prefix of `s`.
pub open spec fn starts_with(s: Seq<u8>, p: Seq<u8>) -> bool {
    p.len() <= s.len() && s.take(p.len() as int) == p
}

/// How to rewrite an extended-length (verbatim) Windows path.
#[derive(Debug)]
pub enum VerbatimPrefix {
    /// `\\?\UNC\` + rest: the result is `\\` followed by the text from this
    /// byte offset (always 8).
    Unc(usize),
    /// `\\?\` + rest (not UNC): the result is the text from this byte offset
    /// (always 4).
    Plain(usize),
    /// Not a verbatim path: unchanged.
    NotVerbatim,
}

/// The rewrite rule as a specification.
pub open spec fn spec_verbatim_prefix(s: Seq<u8>) -> VerbatimPrefix {
    if starts_with(s, verbatim_unc_marker()) {
        VerbatimPrefix::Unc(8)
    } else if starts_with(s, verbatim_marker()) {
        VerbatimPrefix::Plain(4)
    } else {
        VerbatimPrefix::NotVerbatim
    }
}

/// Classify a path's bytes by its verbatim prefix.
pub fn classify_verbatim_prefix(s: &[u8]) -> (r: VerbatimPrefix)
    ensures
        r == spec_verbatim_prefix(s@),
{
    let n = s.len();
    let verbatim = n >= 4 && s[0] == 92 && s[1] == 92 && s[2] == 63 && s[3] == 92;
    proof {
        if verbatim {
            assert(s@.take(4) =~= verbatim_marker());
        } else if n >= 4 {
            assert(s@.take(4)[0] == s@[0] && s@.take(4)[1] == s@[1] && s@.take(4)[2] == s@[2]
                && s@.take(4)[3] == s@[3]);
        }
    }
    if verbatim && n >= 8 && s[4] == 85 && s[5] == 78 && s[6] == 67 && s[7] == 92 {
        assert(s@.take(8) =~= verbatim_unc_marker());
        VerbatimPrefix::Unc(8)
    } else {
        proof {
            if n >= 8 {
                assert(s@.take(8)[0] == s@[0] && s@.take(8)[1] == s@[1] && s@.take(8)[2]
                    == s@[2] && s@.take(8)[3] == s@[3] && s@.take(8)[4] == s@[4] && s@.take(
                    8,
                )[5] == s@[5] && s@.take(8)[6] == s@[6] && s@.take(8)[7] == s@[7]);
            }
        }
        if verbatim {
            VerbatimPrefix::Plain(4)
        } else {
            VerbatimPrefix::NotVerbatim
        }
    }
}

/// The UNC rule wins: a `\\?\UNC\` path is never classified as a plain
/// verbatim path (which would leave a relative `UNC\...` path), and the
/// offsets always lie inside the input and skip exactly an ASCII marker
/// (so they are UTF-8 character boundaries).
pub proof fn theorem_unc_precedence(s: Seq<u8>)
    ensures
        starts_with(s, verbatim_unc_marker()) ==> spec_verbatim_prefix(s) == VerbatimPrefix::Unc(
            8,
        ),
        spec_verbatim_prefix(s) matches VerbatimPrefix::Unc(n) ==> n <= s.len() && s.take(
            n as int,
        ) == verbatim_unc_marker(),
        spec_verbatim_prefix(s) matches VerbatimPrefix::Plain(n) ==> n <= s.len() && s.take(
            n as int,
        ) == verbatim_marker() && !starts_with(s, verbatim_unc_marker()),
        spec_verbatim_prefix(s) is NotVerbatim ==> !starts_with(s, verbatim_marker()),
{
    if starts_with(s, verbatim_unc_marker()) {
        assert(s.take(4) =~= s.take(8).take(4));
        assert(verbatim_unc_marker().take(4) =~= verbatim_marker());
    }
}

} // verus!
