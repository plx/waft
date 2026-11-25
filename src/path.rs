//! Repo-relative path type and normalization.
//!
//! The decisions in [`RepoRelPath::normalize`] (lexical normalization,
//! component-wise containment under the repository root, rejection of the
//! root itself and of anything but plain names remaining) and the text check
//! shared with `RepoRelPath::from_git_bytes` are made by the Verus-verified
//! kernel `waft_verified::path`. This module lowers `std::path` components to
//! the kernel's model and turns its decisions into values and errors.
//!
//! Some steps stay unverified `std` glue here: using the input as given or
//! joining it onto the root, the Windows `to_str` gate in front of the
//! verbatim-prefix rule, collecting the kept components into a `PathBuf`
//! and re-parsing it for the containment decision, and converting each
//! accepted name to UTF-8. The kernel's module documentation lists them with
//! the assumptions that connect its theorems to this code.

use std::fmt;
use std::path::{Component, Path, PathBuf, Prefix};

use waft_verified::path::containment::{Containment, classify_containment};
use waft_verified::path::model::{Comp, PrefixKey};
use waft_verified::path::text::has_forbidden_text_char;

use crate::error::{Error, Result};

/// A normalized, repo-relative path.
///
/// Invariants:
/// - No leading `/` or `\`
/// - No `.` or `..` components
/// - Uses `/` as separator (even on Windows)
/// - Never empty (the repo root is not a valid file path)
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RepoRelPath {
    inner: String,
}

impl RepoRelPath {
    /// Create a `RepoRelPath` from a pre-normalized string.
    ///
    /// This does **not** validate — use [`RepoRelPath::normalize`] for untrusted input.
    pub(crate) fn from_normalized(s: String) -> Self {
        Self { inner: s }
    }

    /// Create a repository-relative path from Git's raw path bytes.
    ///
    /// waft's public output and configuration formats are UTF-8. Reject an
    /// unrepresentable path instead of lossily converting it and risking a
    /// collision with a different destination filename.
    pub(crate) fn from_git_bytes(bytes: &[u8]) -> Result<Self> {
        let value = std::str::from_utf8(bytes).map_err(|_| Error::InvalidPath {
            message: "repository path is not valid UTF-8 and is unsupported".to_string(),
        })?;
        validate_text_path(value)?;
        Ok(Self {
            inner: value.to_string(),
        })
    }

    /// Normalize a path relative to a repo root.
    ///
    /// Accepts absolute or relative paths. Rejects paths that escape the repo
    /// via `..` or resolve to the repo root itself.
    pub fn normalize(path: &Path, repo_root: &Path) -> Result<Self> {
        let abs = if path.is_absolute() {
            path.to_path_buf()
        } else {
            repo_root.join(path)
        };

        // Logical normalization (resolve `.` and `..` without touching the filesystem)
        let normalized = logical_normalize(&without_windows_verbatim_prefix(&abs));

        // Must be strictly under repo_root. The verified kernel compares the
        // components of both normalized paths (what `Path::strip_prefix`
        // compares) and accepts only a non-empty remainder of plain names.
        let repo_normalized = logical_normalize(&without_windows_verbatim_prefix(repo_root));
        let abs_components: Vec<Component<'_>> = normalized.components().collect();
        let root_components: Vec<Component<'_>> = repo_normalized.components().collect();
        let start = match classify_containment(
            &kernel_components(&abs_components),
            &kernel_components(&root_components),
        ) {
            Containment::Outside => {
                return Err(Error::InvalidPath {
                    message: format!(
                        "path {} is outside the repository root {}",
                        path.display(),
                        repo_root.display()
                    ),
                });
            }
            Containment::RepoRoot => {
                return Err(Error::InvalidPath {
                    message: "path resolves to the repository root".to_string(),
                });
            }
            Containment::Inside(start) => start,
        };

        // Convert to forward-slash string. The kernel proved that `start` is
        // in bounds and that every component from it on is a `Normal` name,
        // so `as_os_str` is that name.
        let mut parts = Vec::new();
        for component in &abs_components[start..] {
            let value = component
                .as_os_str()
                .to_str()
                .ok_or_else(|| Error::InvalidPath {
                    message: format!(
                        "path {} is not valid UTF-8 and is unsupported",
                        path.display()
                    ),
                })?;
            parts.push(value.to_string());
        }
        let s = parts.join("/");
        validate_text_path(&s)?;

        Ok(Self { inner: s })
    }

    /// Return the path as a string slice.
    pub fn as_str(&self) -> &str {
        &self.inner
    }

    /// Convert to a `PathBuf` relative to the given root.
    pub fn to_path(&self, root: &Path) -> PathBuf {
        root.join(self.inner.replace('/', std::path::MAIN_SEPARATOR_STR))
    }
}

/// Remove the extended-length prefix returned by Windows canonicalization.
///
/// Windows considers `C:\repo` and `\\?\C:\repo` the same path, but
/// `Path::strip_prefix` does not. Normalizing both spellings at repository
/// boundaries keeps containment checks byte-preserving on Unix while making
/// canonicalized Windows inputs comparable with Git's ordinary paths.
#[cfg(windows)]
pub(crate) fn without_windows_verbatim_prefix(path: &Path) -> PathBuf {
    let Some(path) = path.to_str() else {
        return path.to_path_buf();
    };
    without_verbatim_prefix_text(path)
}

/// Apply the verified verbatim-prefix rule to a UTF-8 path: `\\?\UNC\rest`
/// becomes `\\rest`, any other `\\?\rest` becomes `rest`, and everything else
/// is unchanged. The kernel's offsets skip an ASCII marker, so they are
/// character boundaries.
#[cfg(any(windows, test))]
fn without_verbatim_prefix_text(path: &str) -> PathBuf {
    use waft_verified::path::text::{VerbatimPrefix, classify_verbatim_prefix};

    match classify_verbatim_prefix(path.as_bytes()) {
        VerbatimPrefix::Unc(rest) => PathBuf::from(format!(r"\\{}", &path[rest..])),
        VerbatimPrefix::Plain(rest) => PathBuf::from(&path[rest..]),
        VerbatimPrefix::NotVerbatim => PathBuf::from(path),
    }
}

#[cfg(not(windows))]
pub(crate) fn without_windows_verbatim_prefix(path: &Path) -> PathBuf {
    path.to_path_buf()
}

fn validate_text_path(value: &str) -> Result<()> {
    // Verified to equal `value.contains(['\n', '\r', '\0'])`.
    if has_forbidden_text_char(value) {
        return Err(Error::InvalidPath {
            message: "repository paths containing NUL or newlines are unsupported".to_string(),
        });
    }
    Ok(())
}

impl fmt::Display for RepoRelPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.inner)
    }
}

impl AsRef<str> for RepoRelPath {
    fn as_ref(&self) -> &str {
        &self.inner
    }
}

/// Logically normalize a path by resolving `.` and `..` without filesystem access.
///
/// The verified kernel chooses which components survive (a `..` directly
/// after the root is kept); collecting them through `PathBuf` is unchanged.
fn logical_normalize(path: &Path) -> PathBuf {
    let components: Vec<Component<'_>> = path.components().collect();
    let keep = waft_verified::path::normalize::logical_normalize(&kernel_components(&components));
    keep.into_iter().map(|index| components[index]).collect()
}

/// Lower `std` path components to the verified kernel's model.
fn kernel_components<'a>(components: &[Component<'a>]) -> Vec<Comp<'a>> {
    components.iter().map(kernel_component).collect()
}

/// Lower `path.components()` to the verified kernel's model, one `Comp` per
/// component, with the same conversion `RepoRelPath::normalize` uses. The
/// worktree-mapping kernel compares these with the path model's verified
/// `comp_eq`, so kernel component equality is `Component` equality.
pub(crate) fn kernel_path_components(path: &Path) -> Vec<Comp<'_>> {
    path.components()
        .map(|component| kernel_component(&component))
        .collect()
}

/// Lower one `std` path component. Byte payloads are the components' encoded
/// bytes, so kernel equality is `Component` equality.
fn kernel_component<'a>(component: &Component<'a>) -> Comp<'a> {
    match *component {
        Component::Prefix(prefix) => Comp::Prefix(kernel_prefix(prefix.kind())),
        Component::RootDir => Comp::RootDir,
        Component::CurDir => Comp::CurDir,
        Component::ParentDir => Comp::ParentDir,
        Component::Normal(name) => Comp::Normal(name.as_encoded_bytes()),
    }
}

/// Lower a parsed Windows prefix. `PrefixComponent` equality compares only
/// this parsed value (not the raw prefix text), so the kernel key mirrors it
/// variant for variant.
fn kernel_prefix(prefix: Prefix<'_>) -> PrefixKey<'_> {
    match prefix {
        Prefix::Verbatim(name) => PrefixKey::Verbatim(name.as_encoded_bytes()),
        Prefix::VerbatimUNC(server, share) => {
            PrefixKey::VerbatimUnc(server.as_encoded_bytes(), share.as_encoded_bytes())
        }
        Prefix::VerbatimDisk(drive) => PrefixKey::VerbatimDisk(drive),
        Prefix::DeviceNS(device) => PrefixKey::DeviceNs(device.as_encoded_bytes()),
        Prefix::UNC(server, share) => {
            PrefixKey::Unc(server.as_encoded_bytes(), share.as_encoded_bytes())
        }
        Prefix::Disk(drive) => PrefixKey::Disk(drive),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_simple_relative() {
        let root = Path::new("/repo");
        let p = RepoRelPath::normalize(Path::new("src/main.rs"), root).unwrap();
        assert_eq!(p.as_str(), "src/main.rs");
    }

    #[test]
    fn normalize_absolute_under_root() {
        let root = Path::new("/repo");
        let p = RepoRelPath::normalize(Path::new("/repo/src/lib.rs"), root).unwrap();
        assert_eq!(p.as_str(), "src/lib.rs");
    }

    #[test]
    fn normalize_with_dot_components() {
        let root = Path::new("/repo");
        let p = RepoRelPath::normalize(Path::new("./src/./main.rs"), root).unwrap();
        assert_eq!(p.as_str(), "src/main.rs");
    }

    #[test]
    fn normalize_with_dotdot_staying_inside() {
        let root = Path::new("/repo");
        let p = RepoRelPath::normalize(Path::new("src/sub/../main.rs"), root).unwrap();
        assert_eq!(p.as_str(), "src/main.rs");
    }

    #[test]
    fn reject_path_escaping_root() {
        let root = Path::new("/repo");
        let err = RepoRelPath::normalize(Path::new("../outside"), root).unwrap_err();
        assert!(err.to_string().contains("outside the repository root"));
    }

    #[test]
    fn reject_absolute_outside_root() {
        let root = Path::new("/repo");
        let err = RepoRelPath::normalize(Path::new("/other/file"), root).unwrap_err();
        assert!(err.to_string().contains("outside the repository root"));
    }

    #[test]
    fn reject_repo_root_itself() {
        let root = Path::new("/repo");
        let err = RepoRelPath::normalize(Path::new("."), root).unwrap_err();
        assert!(err.to_string().contains("resolves to the repository root"));
    }

    #[test]
    fn reject_repo_root_absolute() {
        let root = Path::new("/repo");
        let err = RepoRelPath::normalize(Path::new("/repo"), root).unwrap_err();
        assert!(err.to_string().contains("resolves to the repository root"));
    }

    #[test]
    fn to_path_reconstructs_absolute() {
        let root = Path::new("/repo");
        let p = RepoRelPath::normalize(Path::new("src/main.rs"), root).unwrap();
        assert_eq!(p.to_path(root), PathBuf::from("/repo/src/main.rs"));
    }

    #[test]
    fn display_shows_forward_slashes() {
        let p = RepoRelPath::from_normalized("a/b/c".to_string());
        assert_eq!(format!("{p}"), "a/b/c");
    }

    #[test]
    fn ordering_is_lexical() {
        let a = RepoRelPath::from_normalized("a/b".to_string());
        let b = RepoRelPath::from_normalized("a/c".to_string());
        let c = RepoRelPath::from_normalized("b/a".to_string());
        assert!(a < b);
        assert!(b < c);
    }

    #[test]
    fn normalize_deeply_nested_dotdot() {
        let root = Path::new("/repo");
        let p = RepoRelPath::normalize(Path::new("a/b/c/../../d/e"), root).unwrap();
        assert_eq!(p.as_str(), "a/d/e");
    }

    #[test]
    fn reject_dotdot_escaping_via_deep_path() {
        let root = Path::new("/repo");
        let err = RepoRelPath::normalize(Path::new("a/../../outside"), root).unwrap_err();
        assert!(err.to_string().contains("outside the repository root"));
    }

    #[test]
    fn reject_newline_path_that_would_corrupt_line_output() {
        let root = Path::new("/repo");
        let err = RepoRelPath::normalize(Path::new("line\nbreak.env"), root).unwrap_err();
        assert!(err.to_string().contains("newlines"));
    }

    #[cfg(unix)]
    #[test]
    fn reject_non_utf8_path_without_lossy_collision() {
        use std::os::unix::ffi::OsStrExt;

        let root = Path::new("/repo");
        let invalid = std::ffi::OsStr::from_bytes(b"bad-\xff.env");
        let err = RepoRelPath::normalize(Path::new(invalid), root).unwrap_err();
        assert!(err.to_string().contains("not valid UTF-8"));
    }

    #[cfg(windows)]
    #[test]
    fn normalize_accepts_verbatim_path_under_ordinary_root() {
        let root = Path::new(r"C:\repo");
        let path = Path::new(r"\\?\C:\repo\config\.env");
        let normalized = RepoRelPath::normalize(path, root).unwrap();
        assert_eq!(normalized.as_str(), "config/.env");
    }

    #[cfg(windows)]
    #[test]
    fn normalize_accepts_verbatim_unc_path_under_ordinary_root() {
        let root = Path::new(r"\\server\share\repo");
        let path = Path::new(r"\\?\UNC\server\share\repo\.env");
        let normalized = RepoRelPath::normalize(path, root).unwrap();
        assert_eq!(normalized.as_str(), ".env");
    }

    #[test]
    fn reject_escape_above_filesystem_root_instead_of_panicking() {
        // These used to reach `unreachable!` because a `..` directly after
        // the root survives normalization and the normalized root has no
        // name. Every such root lets some path escape this way
        // (`theorem_nameless_root_escapes`), not only `/`.
        for (input, root) in [
            ("../x", "/"),
            ("/../x", "/"),
            ("../x", ""),
            ("..", ""),
            ("/", ""),
            ("/x", "."),
            ("../x", ".."),
            ("../x", "/.."),
            ("../x", "/repo/../.."),
        ] {
            let err = RepoRelPath::normalize(Path::new(input), Path::new(root)).unwrap_err();
            assert_eq!(
                err.to_string(),
                Error::InvalidPath {
                    message: format!("path {input} is outside the repository root {root}"),
                }
                .to_string(),
                "{input:?} under {root:?}"
            );
        }
    }

    #[test]
    fn normalize_under_filesystem_root() {
        let root = Path::new("/");
        let p = RepoRelPath::normalize(Path::new("x/./y"), root).unwrap();
        assert_eq!(p.as_str(), "x/y");
        let p = RepoRelPath::normalize(Path::new("/x/../y"), root).unwrap();
        assert_eq!(p.as_str(), "y");
        let err = RepoRelPath::normalize(Path::new("/"), root).unwrap_err();
        assert!(err.to_string().contains("resolves to the repository root"));
    }

    #[test]
    fn kernel_component_equality_matches_std() {
        let paths = [
            "/",
            "a",
            "./a",
            "../a",
            "/a/./b/../c",
            "a/b",
            "b/a",
            "x/..",
            "./",
            "é/ü",
        ];
        let components: Vec<Component<'_>> = paths
            .iter()
            .flat_map(|p| Path::new(p).components())
            .collect();
        let lowered = kernel_components(&components);
        assert_eq!(lowered.len(), components.len());
        for (a, ka) in components.iter().zip(&lowered) {
            for (b, kb) in components.iter().zip(&lowered) {
                assert_eq!(
                    waft_verified::path::model::comp_eq(ka, kb),
                    a == b,
                    "{a:?} vs {b:?}"
                );
            }
        }
    }

    fn sample_prefixes() -> Vec<Prefix<'static>> {
        use std::ffi::OsStr;

        vec![
            Prefix::Verbatim(OsStr::new("")),
            Prefix::Verbatim(OsStr::new("a")),
            Prefix::Verbatim(OsStr::new("b")),
            Prefix::VerbatimUNC(OsStr::new("a"), OsStr::new("b")),
            Prefix::VerbatimUNC(OsStr::new("a"), OsStr::new("c")),
            Prefix::VerbatimUNC(OsStr::new("ab"), OsStr::new("")),
            Prefix::VerbatimUNC(OsStr::new(""), OsStr::new("ab")),
            Prefix::VerbatimDisk(b'C'),
            Prefix::VerbatimDisk(b'D'),
            Prefix::DeviceNS(OsStr::new("a")),
            Prefix::DeviceNS(OsStr::new("COM1")),
            Prefix::UNC(OsStr::new("a"), OsStr::new("b")),
            Prefix::UNC(OsStr::new("a"), OsStr::new("bc")),
            Prefix::UNC(OsStr::new("ab"), OsStr::new("c")),
            Prefix::UNC(OsStr::new(""), OsStr::new("abc")),
            Prefix::Disk(b'C'),
            Prefix::Disk(b'D'),
        ]
    }

    #[test]
    fn kernel_prefix_equality_matches_std() {
        let prefixes = sample_prefixes();
        for a in &prefixes {
            for b in &prefixes {
                assert_eq!(
                    waft_verified::path::model::prefix_eq(&kernel_prefix(*a), &kernel_prefix(*b)),
                    a == b,
                    "{a:?} vs {b:?}"
                );
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn kernel_prefix_round_trips() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        fn back(key: PrefixKey<'_>) -> Prefix<'_> {
            match key {
                PrefixKey::Verbatim(name) => Prefix::Verbatim(OsStr::from_bytes(name)),
                PrefixKey::VerbatimUnc(server, share) => {
                    Prefix::VerbatimUNC(OsStr::from_bytes(server), OsStr::from_bytes(share))
                }
                PrefixKey::VerbatimDisk(drive) => Prefix::VerbatimDisk(drive),
                PrefixKey::DeviceNs(device) => Prefix::DeviceNS(OsStr::from_bytes(device)),
                PrefixKey::Unc(server, share) => {
                    Prefix::UNC(OsStr::from_bytes(server), OsStr::from_bytes(share))
                }
                PrefixKey::Disk(drive) => Prefix::Disk(drive),
            }
        }

        for prefix in sample_prefixes() {
            assert_eq!(back(kernel_prefix(prefix)), prefix);
        }
    }

    #[test]
    fn text_check_matches_str_contains() {
        for value in [
            "",
            "a",
            "a/b",
            "line\nbreak",
            "cr\rhere",
            "nul\0",
            "\u{85}",
            "\u{2028}",
            "é\n",
            "日本",
            "\u{10FFFF}\r",
            "tab\there",
        ] {
            assert_eq!(
                has_forbidden_text_char(value),
                value.contains(['\n', '\r', '\0']),
                "{value:?}"
            );
        }
    }

    /// The pre-kernel `without_windows_verbatim_prefix` text rule.
    fn legacy_without_verbatim_prefix_text(path: &str) -> PathBuf {
        if let Some(unc) = path.strip_prefix(r"\\?\UNC\") {
            PathBuf::from(format!(r"\\{unc}"))
        } else if let Some(ordinary) = path.strip_prefix(r"\\?\") {
            PathBuf::from(ordinary)
        } else {
            PathBuf::from(path)
        }
    }

    #[test]
    fn verbatim_prefix_rule_matches_legacy() {
        for path in [
            r"\\?\C:\repo",
            r"\\?\UNC\server\share\repo",
            r"\\?\UNC\",
            r"\\?\UNC",
            r"\\?\",
            r"\\?",
            r"\\server\share",
            r"C:\repo",
            r"\\?\Volume{x}\a",
            r"\\?\é",
            r"\\.\COM1",
            "",
            "é",
        ] {
            assert_eq!(
                without_verbatim_prefix_text(path),
                legacy_without_verbatim_prefix_text(path),
                "{path:?}"
            );
        }
    }

    /// A copy of the pre-kernel `RepoRelPath::normalize` (Unix, so no
    /// verbatim stripping). `None` marks inputs that reached `unreachable!`.
    #[cfg(unix)]
    fn legacy_normalize(path: &Path, repo_root: &Path) -> Option<Result<String>> {
        fn legacy_logical_normalize(path: &Path) -> PathBuf {
            let mut components = Vec::new();
            for component in path.components() {
                match component {
                    Component::Prefix(p) => components.push(Component::Prefix(p)),
                    Component::RootDir => {
                        components.retain(|c| matches!(c, Component::Prefix(_)));
                        components.push(Component::RootDir);
                    }
                    Component::CurDir => {}
                    Component::ParentDir => {
                        if let Some(last) = components.last() {
                            if matches!(last, Component::Normal(_)) {
                                components.pop();
                            } else {
                                components.push(Component::ParentDir);
                            }
                        } else {
                            components.push(Component::ParentDir);
                        }
                    }
                    Component::Normal(_) => components.push(component),
                }
            }
            components.iter().collect()
        }

        let abs = if path.is_absolute() {
            path.to_path_buf()
        } else {
            repo_root.join(path)
        };
        let normalized = legacy_logical_normalize(&abs);
        let repo_normalized = legacy_logical_normalize(repo_root);
        let Ok(rel) = normalized.strip_prefix(&repo_normalized) else {
            return Some(Err(Error::InvalidPath {
                message: format!(
                    "path {} is outside the repository root {}",
                    path.display(),
                    repo_root.display()
                ),
            }));
        };
        if rel.as_os_str().is_empty() {
            return Some(Err(Error::InvalidPath {
                message: "path resolves to the repository root".to_string(),
            }));
        }
        let mut parts = Vec::new();
        for component in rel.components() {
            let Component::Normal(os) = component else {
                return None;
            };
            let Some(value) = os.to_str() else {
                return Some(Err(Error::InvalidPath {
                    message: format!(
                        "path {} is not valid UTF-8 and is unsupported",
                        path.display()
                    ),
                }));
            };
            parts.push(value.to_string());
        }
        let s = parts.join("/");
        if s.contains(['\n', '\r', '\0']) {
            return Some(Err(Error::InvalidPath {
                message: "repository paths containing NUL or newlines are unsupported".to_string(),
            }));
        }
        Some(Ok(s))
    }

    #[cfg(unix)]
    fn path_from_segments(absolute: bool, segments: &[&[u8]]) -> PathBuf {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let mut bytes = Vec::new();
        if absolute {
            bytes.push(b'/');
        }
        for (i, segment) in segments.iter().enumerate() {
            if i > 0 {
                bytes.push(b'/');
            }
            bytes.extend_from_slice(segment);
        }
        PathBuf::from(OsStr::from_bytes(&bytes))
    }

    /// Path segments for the differential test, used for both paths and
    /// roots: names, `.`, `..`, empty segments, non-UTF-8 and forbidden
    /// bytes, a backslash, and colon names that are prefixes on Windows.
    #[cfg(unix)]
    const SEGMENTS: &[&[u8]] = &[
        b"repo",
        b"sub",
        b".",
        b"..",
        b"a",
        b"",
        b"\xc3\xa9",
        b"bad\xff",
        b"x\ny",
        b"x\ry",
        b"x\0y",
        b"\\x",
        b"...",
        b"a:b",
        b"C:",
        b"..\xff",
    ];

    #[cfg(unix)]
    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(2048))]

        #[test]
        fn normalize_matches_legacy_except_former_panics(
            path_absolute in proptest::prelude::any::<bool>(),
            path_segments in proptest::collection::vec(0..SEGMENTS.len(), 0..7),
            root_absolute in proptest::prelude::any::<bool>(),
            root_segments in proptest::collection::vec(0..SEGMENTS.len(), 0..4),
        ) {
            let path_parts: Vec<&[u8]> = path_segments.iter().map(|&i| SEGMENTS[i]).collect();
            let root_parts: Vec<&[u8]> = root_segments.iter().map(|&i| SEGMENTS[i]).collect();
            let path = path_from_segments(path_absolute, &path_parts);
            let root = path_from_segments(root_absolute, &root_parts);
            let new = RepoRelPath::normalize(&path, &root)
                .map(|p| p.as_str().to_string())
                .map_err(|e| e.to_string());
            let outside = Error::InvalidPath {
                message: format!(
                    "path {} is outside the repository root {}",
                    path.display(),
                    root.display()
                ),
            };
            match legacy_normalize(&path, &root) {
                Some(Ok(s)) => proptest::prop_assert_eq!(new, Ok(s)),
                Some(Err(e)) => proptest::prop_assert_eq!(new, Err(e.to_string())),
                None => proptest::prop_assert_eq!(new, Err(outside.to_string())),
            }
        }
    }
}
