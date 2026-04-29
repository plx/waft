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
//! rooted at `source_root`. The filter is applied AFTER `check_ignore` so
//! list/copy already saw the eligible set; this lets us drop a strict
//! subset rather than re-deriving the union with the worktreeinclude
//! matcher.

use std::path::Path;

use ignore::gitignore::{Gitignore, GitignoreBuilder};

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

/// Build the exclusion matcher implied by the active policy.
///
/// Returns `Ok(None)` when neither the builtin set nor `extra_excludes`
/// contribute any pattern, i.e. nothing would ever be filtered.
pub fn build_excluder(policy: &ResolvedPolicy, source_root: &Path) -> Result<Option<Gitignore>> {
    let mut builder = GitignoreBuilder::new(source_root);
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
    let matcher = match build_excluder(policy, source_root)? {
        Some(m) => m,
        None => return Ok(()),
    };
    candidates.retain(|rel| !path_excluded(&matcher, rel, source_root));
    Ok(())
}

fn path_excluded(matcher: &Gitignore, rel: &RepoRelPath, source_root: &Path) -> bool {
    let abs = rel.to_path(source_root);
    let is_dir = abs.is_dir();
    matcher
        .matched_path_or_any_parents(rel.as_str(), is_dir)
        .is_ignore()
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
}
