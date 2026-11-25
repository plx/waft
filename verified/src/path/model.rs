//! Kernel model of `std::path::Component`.
//!
//! Production lowers every `std::path::Component` to a [`Comp`] whose payloads
//! borrow the component's bytes (`OsStr::as_encoded_bytes`). Specifications
//! reason about the ghost view [`CompView`], where byte payloads are
//! `Seq<u8>` and equality is structural.
//!
//! # Equality mirrors std
//!
//! `std::path::Component` derives `PartialEq`, so two components are equal
//! exactly when they are the same variant with equal payloads:
//!
//! - `Normal(&OsStr)`: `OsStr` equality, which is equality of the encoded
//!   bytes (raw bytes on Unix, WTF-8 on Windows; both encodings are
//!   canonical, so byte equality is string equality).
//! - `Prefix(PrefixComponent)`: `PrefixComponent::eq` compares only the
//!   *parsed* `std::path::Prefix` value, not the raw prefix text. For
//!   example `c:` and `C:` both parse to `Prefix::Disk(b'C')` (std
//!   upper-cases the drive letter) and compare equal, and
//!   `\\server\share` and `\\server/share` both parse to
//!   `Prefix::UNC("server", "share")`. [`PrefixKey`] therefore mirrors the
//!   parsed `Prefix` enum variant for variant, never the raw text, so its
//!   structural equality is exactly `PrefixComponent` equality. Prefix
//!   components only occur on Windows.
//!
//! (These facts were checked against the Rust 1.90 standard library sources:
//! `path.rs` derives `PartialEq` on `Component` and `Prefix` and compares
//! `PrefixComponent`s by `parsed`; `sys/path/windows_prefix.rs` upper-cases
//! drive letters and splits UNC names on both separators; `ffi/os_str.rs`
//! compares `OsStr`s by `as_encoded_bytes`.)
//!
//! [`comp_eq`] is proven to decide equality of the views, so the kernel's
//! component comparison agrees with std's on every input production can
//! construct, provided the production conversion is faithful (trusted: it is
//! an exhaustive match that copies each payload's encoded bytes).

#[allow(unused_imports)]
use vstd::prelude::*;

verus! {

/// Kernel mirror of the parsed `std::path::Prefix` value of a Windows prefix
/// component. Byte payloads are the `OsStr::as_encoded_bytes` of the
/// corresponding `Prefix` fields.
#[derive(Clone, Copy, Debug)]
pub enum PrefixKey<'a> {
    /// `\\?\prefix`
    Verbatim(&'a [u8]),
    /// `\\?\UNC\server\share`
    VerbatimUnc(&'a [u8], &'a [u8]),
    /// `\\?\C:` (upper-cased drive letter)
    VerbatimDisk(u8),
    /// `\\.\device`
    DeviceNs(&'a [u8]),
    /// `\\server\share`
    Unc(&'a [u8], &'a [u8]),
    /// `C:` (upper-cased drive letter)
    Disk(u8),
}

/// Ghost view of a [`PrefixKey`].
pub enum PrefixView {
    /// `\\?\prefix`
    Verbatim(Seq<u8>),
    /// `\\?\UNC\server\share`
    VerbatimUnc(Seq<u8>, Seq<u8>),
    /// `\\?\C:`
    VerbatimDisk(u8),
    /// `\\.\device`
    DeviceNs(Seq<u8>),
    /// `\\server\share`
    Unc(Seq<u8>, Seq<u8>),
    /// `C:`
    Disk(u8),
}

/// Kernel mirror of `std::path::Component`.
#[derive(Clone, Copy, Debug)]
pub enum Comp<'a> {
    /// A Windows path prefix such as `C:` or `\\server\share`.
    Prefix(PrefixKey<'a>),
    /// The root directory separator.
    RootDir,
    /// A `.` component (std only yields it at the start of a path, or inside
    /// Windows verbatim paths).
    CurDir,
    /// A `..` component.
    ParentDir,
    /// An ordinary file or directory name (its encoded bytes).
    Normal(&'a [u8]),
}

/// Ghost view of a [`Comp`].
pub enum CompView {
    /// A Windows path prefix.
    Prefix(PrefixView),
    /// The root directory separator.
    RootDir,
    /// A `.` component.
    CurDir,
    /// A `..` component.
    ParentDir,
    /// An ordinary name.
    Normal(Seq<u8>),
}

impl<'a> View for PrefixKey<'a> {
    type V = PrefixView;

    open spec fn view(&self) -> PrefixView {
        match *self {
            PrefixKey::Verbatim(s) => PrefixView::Verbatim(s@),
            PrefixKey::VerbatimUnc(a, b) => PrefixView::VerbatimUnc(a@, b@),
            PrefixKey::VerbatimDisk(d) => PrefixView::VerbatimDisk(d),
            PrefixKey::DeviceNs(s) => PrefixView::DeviceNs(s@),
            PrefixKey::Unc(a, b) => PrefixView::Unc(a@, b@),
            PrefixKey::Disk(d) => PrefixView::Disk(d),
        }
    }
}

impl<'a> View for Comp<'a> {
    type V = CompView;

    open spec fn view(&self) -> CompView {
        match *self {
            Comp::Prefix(p) => CompView::Prefix(p@),
            Comp::RootDir => CompView::RootDir,
            Comp::CurDir => CompView::CurDir,
            Comp::ParentDir => CompView::ParentDir,
            Comp::Normal(s) => CompView::Normal(s@),
        }
    }
}

/// The views of a sequence of kernel components.
pub open spec fn views<'a>(cs: Seq<Comp<'a>>) -> Seq<CompView> {
    cs.map_values(|c: Comp<'a>| c@)
}

/// Byte-slice equality, proven to be equality of the byte sequences.
pub fn bytes_eq(a: &[u8], b: &[u8]) -> (r: bool)
    ensures
        r == (a@ == b@),
{
    let r = a == b;
    if r {
        assert(a@ =~= b@);
    }
    r
}

/// Equality of two prefix keys, proven to be equality of their views.
pub fn prefix_eq(a: &PrefixKey, b: &PrefixKey) -> (r: bool)
    ensures
        r == (a@ == b@),
{
    match (a, b) {
        (PrefixKey::Verbatim(x), PrefixKey::Verbatim(y)) => bytes_eq(x, y),
        (PrefixKey::VerbatimUnc(x1, x2), PrefixKey::VerbatimUnc(y1, y2)) => {
            bytes_eq(x1, y1) && bytes_eq(x2, y2)
        },
        (PrefixKey::VerbatimDisk(x), PrefixKey::VerbatimDisk(y)) => *x == *y,
        (PrefixKey::DeviceNs(x), PrefixKey::DeviceNs(y)) => bytes_eq(x, y),
        (PrefixKey::Unc(x1, x2), PrefixKey::Unc(y1, y2)) => bytes_eq(x1, y1) && bytes_eq(x2, y2),
        (PrefixKey::Disk(x), PrefixKey::Disk(y)) => *x == *y,
        _ => false,
    }
}

/// Equality of two components, proven to be equality of their views (and so,
/// given a faithful conversion, `std::path::Component` equality).
pub fn comp_eq(a: &Comp, b: &Comp) -> (r: bool)
    ensures
        r == (a@ == b@),
{
    match (a, b) {
        (Comp::Prefix(x), Comp::Prefix(y)) => prefix_eq(x, y),
        (Comp::RootDir, Comp::RootDir) => true,
        (Comp::CurDir, Comp::CurDir) => true,
        (Comp::ParentDir, Comp::ParentDir) => true,
        (Comp::Normal(x), Comp::Normal(y)) => bytes_eq(x, y),
        _ => false,
    }
}

/// Whether `c` is a `Normal` component.
pub fn is_normal(c: &Comp) -> (r: bool)
    ensures
        r == (c@ is Normal),
{
    matches!(c, Comp::Normal(_))
}

/// Whether `c` is a `Prefix` component.
pub fn is_prefix(c: &Comp) -> (r: bool)
    ensures
        r == (c@ is Prefix),
{
    matches!(c, Comp::Prefix(_))
}

} // verus!
