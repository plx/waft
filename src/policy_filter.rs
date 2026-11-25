//! Post-selection exclusion filter driven by [`ResolvedPolicy`].
//!
//! Selection (worktreeinclude or all-ignored) produces a candidate set; this
//! module drops paths that the active policy says should never be copied.
//! Two contributing knobs:
//!
//! - `policy.builtin_exclude_set`: a curated, named pattern set that ships
//!   with waft. Currently `tooling-v1` is the only non-empty set; it covers
//!   well-known tool-state directories (`.conductor/`, `.claude/`,
//!   `.worktrees/`, common VCS dirs, etc.) so worktrunk-equivalent behavior
//!   can be obtained without users hand-curating a list.
//! - `policy.extra_excludes`: arbitrary user-supplied glob patterns that
//!   layer on top of the builtin set.
//!
//! Both contribute to a single [`ignore::gitignore::Gitignore`] matcher
//! rooted at `source_root`. The filter narrows the selection candidate set;
//! Git ignore/tracked membership is still checked by the command's canonical
//! eligibility pass.

use std::path::{Component, Path};

use ignore::Match;
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use waft_verified::exclusion::{self, LevelMatchOracle};
use waft_verified::rules::precedence::MatchKind;

use crate::config::{BuiltinExcludeSet, ResolvedPolicy};
use crate::error::{Error, Result};
use crate::path::RepoRelPath;

/// Gitignore-style patterns shipped with the `tooling-v1` set.
///
/// Each pattern is interpreted as if it were a line in a `.gitignore` at
/// `source_root`. Directory patterns (trailing `/`) match anything under
/// that directory; explicit children would also work but the directory
/// form is shorter and matches the matrix doc's listing.
pub const TOOLING_V1_PATTERNS: &[&str] = &[
    ".conductor/",
    ".claude/",
    ".worktrees/",
    ".git/",
    ".jj/",
    ".hg/",
    ".svn/",
    ".bzr/",
    ".pijul/",
    ".sl/",
    ".entire/",
    ".pi/",
];

/// Whether exclusion matching must be conservative about case aliases.
///
/// A repository may explicitly set `core.ignoreCase=false` even though its
/// macOS or Windows worktree still aliases differently-cased names. Built-in
/// safety exclusions must follow the filesystem threat model, not trust that
/// configuration bit alone.
///
/// This is intentionally *not* [`crate::git::case_folding_applies`], which
/// decides whether two repository paths are the same path. The two answers
/// can differ — on a case-sensitive volume configured `core.ignoreCase=false`,
/// `Vendor/` is genuinely not the `vendor/` submodule, yet a `vendor/**`
/// exclusion still withholds it — because the failure modes are not
/// symmetric. Treating two distinct paths as one *drops a legitimate
/// candidate*, so path comparison must be exact; failing to exclude a file
/// *copies a secret*, so exclusion takes `core_ignore_case` as a lower bound
/// and stays conservative where the platform may alias names.
pub fn effective_case_insensitive(core_ignore_case: bool) -> bool {
    exclusion::exclusion_folds_case(core_ignore_case, cfg!(any(target_os = "macos", windows)))
}

/// Build the exclusion matcher implied by the active policy.
///
/// Returns `Ok(None)` when neither the builtin set nor `extra_excludes`
/// contribute any pattern, i.e. nothing would ever be filtered.
pub fn build_excluder(policy: &ResolvedPolicy, source_root: &Path) -> Result<Option<Gitignore>> {
    build_excluder_with_case(policy, source_root, false)
}

/// Build the exclusion matcher, honoring the repository's
/// `core.ignoreCase` setting.
pub fn build_excluder_with_case(
    policy: &ResolvedPolicy,
    source_root: &Path,
    case_insensitive: bool,
) -> Result<Option<Gitignore>> {
    let mut builder = GitignoreBuilder::new(source_root);
    if case_insensitive {
        builder.case_insensitive(true).map_err(|e| Error::Config {
            message: format!("failed to enable case-insensitive excludes: {e}"),
        })?;
    }
    let mut any = false;

    let builtin_patterns = match policy.builtin_exclude_set {
        BuiltinExcludeSet::None => &[][..],
        BuiltinExcludeSet::ToolingV1 => TOOLING_V1_PATTERNS,
    };
    for pat in builtin_patterns {
        builder.add_line(None, pat).map_err(|e| Error::Config {
            message: format!("invalid builtin exclude pattern {pat:?}: {e}"),
        })?;
        any = true;
    }
    for pat in &policy.extra_excludes {
        builder.add_line(None, pat).map_err(|e| Error::Config {
            message: format!("invalid extra exclude pattern {pat:?}: {e}"),
        })?;
        any = true;
    }

    if !any {
        return Ok(None);
    }
    let matcher = builder.build().map_err(|e| Error::Config {
        message: format!("failed to compile exclude patterns: {e}"),
    })?;
    Ok(Some(matcher))
}

/// Apply the policy-driven exclusion filter to `candidates`.
///
/// Mutates in place by retaining only entries whose path does not match any
/// configured exclusion pattern (including ancestor directory matches —
/// e.g. a candidate `foo/bar.txt` is dropped when the matcher includes
/// `foo/`).
pub fn filter_paths(
    candidates: &mut Vec<RepoRelPath>,
    policy: &ResolvedPolicy,
    source_root: &Path,
) -> Result<()> {
    filter_paths_with_case(candidates, policy, source_root, false)
}

/// Apply exclusions using the repository's effective case-sensitivity.
pub fn filter_paths_with_case(
    candidates: &mut Vec<RepoRelPath>,
    policy: &ResolvedPolicy,
    source_root: &Path,
    case_insensitive: bool,
) -> Result<()> {
    let matcher = match build_excluder_with_case(policy, source_root, case_insensitive)? {
        Some(m) => m,
        None => return Ok(()),
    };
    candidates.retain(|rel| !path_excluded(&matcher, rel, source_root));
    Ok(())
}

/// Whether the exclusion filter drops `rel`. The walk over the path and its
/// parents and the decision are the verified `exclusion::path_excluded`
/// kernel; this function supplies the matcher's per-level answers.
fn path_excluded(matcher: &Gitignore, rel: &RepoRelPath, source_root: &Path) -> bool {
    let abs = rel.to_path(source_root);
    let is_dir = abs.is_dir();
    let mut levels = ExclusionLevels::new(matcher, Path::new(rel.as_str()), is_dir);
    exclusion::path_excluded(&mut levels)
}

/// The exclusion matcher's answers for one candidate, level by level: the
/// path itself, then each parent, ending with the empty path. This is the
/// walk `Gitignore::matched_path_or_any_parents` performs on the candidate
/// after stripping the matcher root from it once.
///
/// `matched` strips the root from each level separately, so asking it level
/// by level is the same walk only when that strip leaves every level
/// unchanged (`root_strip_is_identity`). It does for the CLI, which
/// passes a relative candidate and a canonical, absolute source root. For
/// any other root (a library caller may pass a relative one, which `ignore`
/// strips byte-wise from the whole path), the oracle reports one level: the
/// verdict of `matched_path_or_any_parents` itself, so the kernel decides
/// exactly as that call did.
struct ExclusionLevels<'a> {
    matcher: &'a Gitignore,
    levels: Vec<&'a Path>,
    is_dir: bool,
    /// Whether `levels` holds only the candidate, answered with the
    /// matcher's own parent walk.
    whole_walk: bool,
}

impl<'a> ExclusionLevels<'a> {
    fn new(matcher: &'a Gitignore, path: &'a Path, is_dir: bool) -> Self {
        if !root_strip_is_identity(matcher.path(), path) {
            return ExclusionLevels {
                matcher,
                levels: vec![path],
                is_dir,
                whole_walk: true,
            };
        }
        let mut levels = vec![path];
        let mut current = path;
        while let Some(parent) = current.parent() {
            levels.push(parent);
            current = parent;
        }
        ExclusionLevels {
            matcher,
            levels,
            is_dir,
            whole_walk: false,
        }
    }
}

/// Whether `ignore`'s prefix strip leaves `path` and each of its parents
/// unchanged under a matcher rooted at `root`.
///
/// `ignore` 0.4.33 strips a leading `./` component, and then strips the
/// matcher root when the candidate begins with it (byte-wise on Unix,
/// component-wise elsewhere). An absolute root never begins a path that has
/// no root, and the parents of such a path have none either. A path whose
/// first component is `.` is excluded too. Answering `false` is always
/// safe: the caller then uses `matched_path_or_any_parents` itself.
fn root_strip_is_identity(root: &Path, path: &Path) -> bool {
    root.is_absolute()
        && !path.has_root()
        && !matches!(path.components().next(), Some(Component::CurDir))
}

impl LevelMatchOracle for ExclusionLevels<'_> {
    fn levels(&self) -> usize {
        self.levels.len()
    }

    fn query_level(&mut self, l: usize) -> Option<MatchKind> {
        let matched = if self.whole_walk {
            self.matcher
                .matched_path_or_any_parents(self.levels[l], self.is_dir)
        } else {
            // Only the path itself may be a file; every parent is a directory.
            let is_dir = if l == 0 { self.is_dir } else { true };
            self.matcher.matched(self.levels[l], is_dir)
        };
        match matched {
            Match::None => None,
            Match::Ignore(_) => Some(MatchKind::Ignore),
            Match::Whitelist(_) => Some(MatchKind::Whitelist),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{BuiltinExcludeSet, CompatProfile, ConfigLayer, ResolvedPolicy};

    fn rel(s: &str) -> RepoRelPath {
        RepoRelPath::from_normalized(s.to_string())
    }

    fn policy_with(profile: Option<CompatProfile>, builtin: BuiltinExcludeSet) -> ResolvedPolicy {
        let mut layer = ConfigLayer {
            builtin_exclude_set: Some(builtin),
            ..ConfigLayer::default()
        };
        if let Some(p) = profile {
            layer.profile = Some(p);
        }
        ResolvedPolicy::from_layers([&layer])
    }

    #[test]
    fn no_builtin_no_extras_yields_no_matcher() {
        let policy = ResolvedPolicy::default();
        let tmp = tempfile::TempDir::new().unwrap();
        assert!(build_excluder(&policy, tmp.path()).unwrap().is_none());
    }

    #[test]
    fn tooling_v1_drops_conductor_subtree() {
        let policy = policy_with(None, BuiltinExcludeSet::ToolingV1);
        let tmp = tempfile::TempDir::new().unwrap();
        let mut candidates = vec![
            rel(".conductor/state/dev.key"),
            rel(".env"),
            rel("src/main.rs"),
        ];
        filter_paths(&mut candidates, &policy, tmp.path()).unwrap();
        let names: Vec<_> = candidates.iter().map(|p| p.as_str()).collect();
        assert!(!names.contains(&".conductor/state/dev.key"));
        assert!(names.contains(&".env"));
        assert!(names.contains(&"src/main.rs"));
    }

    #[test]
    fn extra_excludes_apply_on_top_of_builtin() {
        let mut policy = policy_with(None, BuiltinExcludeSet::ToolingV1);
        policy.extra_excludes.push("**/*.tmp".to_string());
        let tmp = tempfile::TempDir::new().unwrap();
        let mut candidates = vec![rel(".conductor/x.key"), rel("foo/bar.tmp"), rel("keep.me")];
        filter_paths(&mut candidates, &policy, tmp.path()).unwrap();
        let names: Vec<_> = candidates.iter().map(|p| p.as_str()).collect();
        assert_eq!(names, vec!["keep.me"]);
    }

    #[test]
    fn extras_only_no_builtin_still_filters() {
        let mut policy = ResolvedPolicy::default();
        policy.extra_excludes.push("logs/".to_string());
        let tmp = tempfile::TempDir::new().unwrap();
        let mut candidates = vec![rel("logs/today.log"), rel("notes.md")];
        filter_paths(&mut candidates, &policy, tmp.path()).unwrap();
        let names: Vec<_> = candidates.iter().map(|p| p.as_str()).collect();
        assert_eq!(names, vec!["notes.md"]);
    }

    #[test]
    fn invalid_extra_pattern_returns_error() {
        let mut policy = ResolvedPolicy::default();
        // Trailing backslash is malformed.
        policy.extra_excludes.push("\\".to_string());
        let tmp = tempfile::TempDir::new().unwrap();
        let mut candidates = vec![rel("a")];
        let err = filter_paths(&mut candidates, &policy, tmp.path()).unwrap_err();
        assert!(err.to_string().contains("exclude pattern"));
    }

    #[test]
    fn tooling_excludes_follow_core_ignore_case() {
        let policy = policy_with(None, BuiltinExcludeSet::ToolingV1);
        let tmp = tempfile::TempDir::new().unwrap();
        let mut candidates = vec![rel(".Conductor/state/key"), rel(".env")];
        filter_paths_with_case(&mut candidates, &policy, tmp.path(), true).unwrap();
        assert_eq!(candidates, vec![rel(".env")]);
    }

    #[test]
    fn level_walk_matches_ignore_parent_walk() {
        // Under an absolute root the kernel walks the levels itself and asks
        // `matched` per level; under any other root it gets the parent
        // walk's verdict as one level. Either way it must agree with
        // `matched_path_or_any_parents` everywhere, as the filter did
        // before the kernel existed.
        let tmp = tempfile::TempDir::new().unwrap();
        let roots = [
            tmp.path(),
            Path::new("src"),
            Path::new("su"),
            Path::new("a/b"),
            Path::new("./src"),
            Path::new("."),
            Path::new(""),
        ];
        let pattern_sets: &[&[&str]] = &[
            TOOLING_V1_PATTERNS,
            &[".claude/", "!.claude/settings.json"],
            &["a/", "!a/b/"],
            &["*", "!keep"],
            &["*.tmp", "!x/"],
            &["/"],
            &["**/node_modules/", "!**/node_modules/keep/**"],
            &["logs/", "!logs/*.md", "logs/secret.md"],
            &["/Root.ENV", "sub/*/deep"],
            &["src"],
            &["sub", "!b"],
            &["main.rs", "!src/"],
            &["/b/", "c"],
        ];
        let paths = [
            ".claude/settings.json",
            ".Claude/x",
            "src/.git/config",
            "a/b/c",
            "a/c",
            "keep",
            "x/keep",
            "x/a.tmp",
            "y/a.tmp",
            "node_modules/keep/a",
            "node_modules/drop/a",
            "logs/readme.md",
            "logs/secret.md",
            "logs/a/b",
            "root.env",
            "sub/q/deep",
            "sub/q/deep/file",
            "src/main.rs",
            "src/src/main.rs",
            "sub/x",
            "su/b",
            "a/b/a/b/c",
        ];
        for root in roots {
            for patterns in pattern_sets {
                for case_insensitive in [false, true] {
                    let mut builder = GitignoreBuilder::new(root);
                    if case_insensitive {
                        builder.case_insensitive(true).unwrap();
                    }
                    for pattern in *patterns {
                        builder.add_line(None, pattern).unwrap();
                    }
                    let matcher = builder.build().unwrap();
                    for path in paths {
                        for is_dir in [false, true] {
                            let expected = matcher
                                .matched_path_or_any_parents(path, is_dir)
                                .is_ignore();
                            let mut levels =
                                ExclusionLevels::new(&matcher, Path::new(path), is_dir);
                            assert_eq!(levels.whole_walk, !root.is_absolute());
                            assert_eq!(
                                exclusion::path_excluded(&mut levels),
                                expected,
                                "root {root:?}, patterns {patterns:?}, path {path:?}, \
                                 is_dir {is_dir}, case_insensitive {case_insensitive}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn relative_source_root_keeps_the_parent_walk_answer() {
        // `ignore` strips a relative matcher root from the whole candidate
        // once, byte-wise; the filter keeps that answer for library callers
        // that pass a relative source root.
        let cases = [("src", "src", "src/main.rs"), ("su", "sub", "sub/x")];
        for (root, extra, path) in cases {
            let mut policy = ResolvedPolicy::default();
            policy.extra_excludes.push(extra.to_string());
            let mut candidates = vec![rel(path)];
            filter_paths(&mut candidates, &policy, Path::new(root)).unwrap();
            assert_eq!(
                candidates,
                vec![rel(path)],
                "root {root:?}, extra {extra:?}"
            );
        }
    }

    #[test]
    fn negated_extra_reincludes_guarded_path() {
        // Pins current behavior (exclusion::
        // lemma_quirk_negated_extra_reincludes_guarded_path): extras follow
        // the built-ins, and the path's own negated match decides first.
        let mut policy = policy_with(None, BuiltinExcludeSet::ToolingV1);
        policy
            .extra_excludes
            .push("!.claude/settings.json".to_string());
        let tmp = tempfile::TempDir::new().unwrap();
        let mut candidates = vec![rel(".claude/settings.json"), rel(".claude/other.json")];
        filter_paths(&mut candidates, &policy, tmp.path()).unwrap();
        assert_eq!(candidates, vec![rel(".claude/settings.json")]);
    }

    #[test]
    fn native_case_alias_platforms_are_conservative() {
        assert_eq!(
            effective_case_insensitive(false),
            cfg!(any(target_os = "macos", windows))
        );
        assert!(effective_case_insensitive(true));
    }
}
