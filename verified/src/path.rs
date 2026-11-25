//! Verified kernel: repository-relative path normalization and containment.
//!
//! `RepoRelPath::normalize` (production `src/path.rs`) is the final gate that
//! decides whether a user, walker or rule-file path lies strictly inside the
//! repository root, and which repository-relative names it denotes. This
//! kernel makes the gate's normalization, containment and text decisions
//! (listed below). A few steps remain `std` behaviour or unverified glue in
//! production; they are listed under "What production still decides".
//!
//! # What the kernel decides
//!
//! - [`normalize::logical_normalize`]: which components survive lexical
//!   normalization of `.` and `..` (current semantics exactly, including
//!   keeping a `..` directly after the root).
//! - [`containment::classify_containment`]: whether the normalized path is
//!   outside the normalized root, is the root itself, or lies strictly
//!   inside it with only names remaining (the former `strip_prefix`, empty
//!   check, and `Normal`-only loop). Its component-prefix test,
//!   [`containment::component_prefix`], is also the worktree-mapping
//!   kernel's (`crate::worktree_map`), which production feeds through the
//!   same lowering.
//! - [`text::has_forbidden_text_char`]: whether a path string contains NUL,
//!   CR or LF (`validate_text_path`).
//! - [`text::classify_verbatim_prefix`]: how the Windows-only
//!   `without_windows_verbatim_prefix` rewrites `\\?\` and `\\?\UNC\` paths.
//!
//! # What production still decides
//!
//! These steps of `RepoRelPath::normalize` are not kernel decisions:
//!
//! - whether the input is used as given (`Path::is_absolute`) or joined onto
//!   the root (`Path::join`, which on Windows rewrites root-relative and
//!   drive-relative inputs);
//! - on Windows, whether verbatim stripping applies at all: a path that is
//!   not valid UTF-8 (`Path::to_str` fails) keeps its `\\?\` prefix, and
//!   `classify_verbatim_prefix` only decides how a UTF-8 path is rewritten;
//! - the hand-off between the two kernels: the components
//!   `logical_normalize` keeps are collected into a `PathBuf`, and
//!   `classify_containment` is given the components `Path::components`
//!   re-parses from that buffer, not the kept list itself (see the trust
//!   assumptions below);
//! - whether each accepted name is valid UTF-8 (`OsStr::to_str`, an error
//!   otherwise), joining the names with `/`, and building the errors.
//!
//! The exec postconditions (`logical_normalize`, `classify_containment`,
//! `has_forbidden_text_char`, `classify_verbatim_prefix`) hold for exactly
//! the values production passes in. The composite theorems, stated over
//! `spec_repo_relative` (containment of `spec_normalize` outputs), describe
//! production only through the collect/re-parse assumption below, which
//! holds on Unix.
//!
//! # Theorems
//!
//! Normalization (`normalize`):
//! - `logical_normalize` returns strictly increasing, in-bounds indices that
//!   pick out exactly `spec_normalize` of its input.
//! - `theorem_normalize_is_normal_form`: the result has no `.`, the root only
//!   after prefixes, and no `..` directly after a name.
//! - `theorem_fixed_points_are_normal_forms`: normalization leaves a sequence
//!   unchanged exactly when it is in normal form.
//! - `theorem_normalize_idempotent`: normalizing twice is normalizing once.
//! - `theorem_normalize_prefix_first`: normalizing a leading part first does
//!   not change the result.
//! - `theorem_append_names`: a normal form followed by names is normalized.
//! - `theorem_names_last`: for `Path::components`-shaped input (a prefix can
//!   only come first), after the first name in the result only names follow.
//!
//! Containment (`containment`):
//! - `classify_containment` equals `spec_containment` and reports the
//!   remainder's start as the root's length.
//! - `theorem_inside_is_name_extension`: `Inside` means the path is the root
//!   followed by a non-empty list of names (no `.`, `..`, root or prefix).
//! - `theorem_classification_is_exact`: `RepoRoot` iff the sequences are
//!   equal; `Inside` iff the root is a proper component prefix followed only
//!   by names (so success implies a non-empty result).
//! - `theorem_normalized_containment`: normalized root ++ result ==
//!   normalized path.
//! - `theorem_round_trip`: a result joined back onto the root classifies to
//!   the same result (the component form of `normalize(p.to_path(root),
//!   root) == Ok(p)`).
//! - `theorem_escape_needs_nameless_root`: when the normalized root contains a
//!   name, nothing but names can remain under it, so the former
//!   `unreachable!("normalized path should only have Normal components")`
//!   was only reachable for a root whose normalized form has no name (`/`,
//!   `C:\`, `""`, `.`, or only `..`s after those: `..`, `/..`, `/repo/../..`).
//! - `theorem_nameless_root_escapes`: conversely, every such root reaches
//!   that branch (with the path `root/..`), so it is reachable exactly for
//!   nameless roots.
//! - `theorem_escape_is_leading`: when a non-name remains, it is the first
//!   remaining component, so the former loop panicked before converting any
//!   name to UTF-8; classifying the case as Outside changes only the panic.
//! - `example_escape_above_filesystem_root`: `/../x` under `/` (the scout's
//!   reproduction of the panic) is Outside.
//!
//! Text (`text`):
//! - `has_forbidden_text_char` returns exactly
//!   `s.contains(['\n', '\r', '\0'])`, via `lemma_forbidden_byte_iff_char`
//!   (an ASCII byte occurs in UTF-8 only as that ASCII character).
//! - `classify_verbatim_prefix` equals `spec_verbatim_prefix`;
//!   `theorem_unc_precedence`: the UNC form wins and the offsets skip exactly
//!   an ASCII marker inside the input.
//!
//! # Behaviour change
//!
//! The only one: a path whose normalized remainder under the root starts with
//! a non-name (for example `normalize("../x", "/")`, `normalize("../x",
//! "..")`, or any absolute path under the relative root `""` or `.`) used to
//! panic at `unreachable!`; it is now the existing "path ... is outside the
//! repository root ..." error.
//!
//! # Trust assumptions (production glue)
//!
//! - Lowering (`kernel_component` / `kernel_prefix` in `src/path.rs`) is an
//!   exhaustive match that copies each payload's `OsStr::as_encoded_bytes`;
//!   `OsStr` equality is encoded-byte equality, and `PrefixComponent`
//!   equality compares only the parsed `Prefix`, which `model::PrefixKey`
//!   mirrors variant for variant (see `model`). So kernel component equality
//!   is `std::path::Component` equality.
//! - Production collects the kept components into a `PathBuf` and lowers the
//!   *re-parsed* components of that path for the containment decision, as the
//!   old `strip_prefix` call did. On Unix this round trip is the identity on
//!   normal forms, so the composite theorems (`spec_repo_relative`) describe
//!   production exactly. On Windows `PathBuf::push` can rewrite some inputs
//!   (it drops `..` after a verbatim root, and clears the buffer when a name
//!   such as `a:b` or `//server/share` re-parses as a prefix); production
//!   keeps that std behaviour unchanged. `classify_containment`'s own
//!   postcondition still holds for whatever component list std yields, but
//!   the composite theorems do not describe those inputs.
//! - `Path::components` yields a prefix only first, `Normal` names that are
//!   non-empty and never `.` or `..`, and `Component::as_os_str` of a `Normal`
//!   is its name. The remainder production converts to UTF-8 is the list
//!   suffix the kernel accepted, which is what `strip_prefix(..).components()`
//!   yielded before.
//! - `str::as_bytes` is the UTF-8 encoding of the string (vstd's
//!   specification).

pub mod containment;
pub mod model;
pub mod normalize;
pub mod text;
