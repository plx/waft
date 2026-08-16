use std::collections::HashSet;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use crate::path::RepoRelPath;

/// Match control filenames according to the native worktree threat model.
///
/// macOS and Windows may alias differently-cased spellings even when
/// `core.ignoreCase` is explicitly false. Discovery, validation, and nested
/// repository boundaries must all make the same conservative decision.
pub(crate) fn special_filename_matches(actual: &OsStr, expected: &str) -> bool {
    if actual == OsStr::new(expected) {
        return true;
    }
    cfg!(any(target_os = "macos", windows))
        && actual
            .to_str()
            .is_some_and(|name| name.eq_ignore_ascii_case(expected))
}

/// Decide how a repository-boundary comparison treats letter case.
///
/// Nested-repository and config-discovery boundaries are safety exclusions:
/// a false negative walks into another repository and can select or trust
/// its files, while a false positive merely leaves one directory unscanned.
/// The comparison therefore folds whenever the checkout folds case or the
/// platform's filesystems may alias case even under `core.ignoreCase =
/// false` (macOS, Windows) — the on-disk spelling cannot be trusted to
/// match the index gitlink there. Mirrors [`special_filename_matches`] and
/// the exclusion-pattern rule in `policy_filter`.
pub(crate) fn boundary_case_folds(ignore_case: bool) -> bool {
    ignore_case || cfg!(any(target_os = "macos", windows))
}

/// Return true when `path` is a directory the source walk must not enter.
///
/// `ignore_case` is the checkout's own case sensitivity (`core.ignoreCase`,
/// via [`crate::git::case_folding_applies`]). The gitlink comparison applies
/// it through [`boundary_case_folds`]: on a case-insensitive checkout —
/// or any platform that may alias case regardless of the key — `Vendor/`
/// and the `vendor/` submodule are treated as the same directory, while a
/// case-sensitive checkout on a non-aliasing platform keeps them distinct
/// so a legitimate source subtree is not silently excluded.
pub(crate) fn is_git_boundary_dir(
    path: &Path,
    depth: usize,
    source_root: &Path,
    gitlinks: &HashSet<String>,
    ignore_case: bool,
) -> bool {
    if path
        .file_name()
        .is_some_and(|name| special_filename_matches(name, ".git"))
    {
        return true;
    }
    if depth == 0 {
        return false;
    }
    let dot_git = path.join(".git");
    if dot_git.is_dir() {
        return true;
    }
    if dot_git.is_file()
        && let Some(target) = read_dot_git_pointer(&dot_git)
        && target.exists()
    {
        return true;
    }
    if let Ok(rel) = RepoRelPath::normalize(path, source_root)
        && gitlinks.iter().any(|gitlink| {
            crate::git::repo_paths_equivalent(
                rel.as_str(),
                gitlink,
                boundary_case_folds(ignore_case),
            )
        })
    {
        return true;
    }
    false
}

fn read_dot_git_pointer(path: &Path) -> Option<PathBuf> {
    let content = std::fs::read_to_string(path).ok()?;
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("gitdir:") {
            let target = rest.trim();
            if target.is_empty() {
                return None;
            }
            let candidate = PathBuf::from(target);
            if candidate.is_absolute() {
                return Some(candidate);
            }
            return path.parent().map(|p| p.join(candidate));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gitlinks(paths: &[&str]) -> HashSet<String> {
        paths.iter().map(|path| (*path).to_string()).collect()
    }

    /// A case-sensitive checkout on a non-aliasing platform holds `Vendor/`
    /// and the `vendor/` submodule as two different directories; only the
    /// submodule is a boundary.
    #[test]
    #[cfg(not(any(target_os = "macos", windows)))]
    fn gitlink_boundary_requires_exact_spelling_when_case_is_significant() {
        let temp = tempfile::TempDir::new().unwrap();
        let vendor = temp.path().join("Vendor");
        std::fs::create_dir(&vendor).unwrap();

        assert!(!is_git_boundary_dir(
            &vendor,
            1,
            temp.path(),
            &gitlinks(&["vendor"]),
            false
        ));
        assert!(is_git_boundary_dir(
            &vendor,
            1,
            temp.path(),
            &gitlinks(&["Vendor"]),
            false
        ));
    }

    /// On platforms whose filesystems may alias case, the gitlink comparison
    /// stays folded even for a case-sensitive checkout: the on-disk spelling
    /// cannot be trusted to match the index's, and descending into a
    /// registered submodule is the failure that must not happen.
    #[test]
    #[cfg(any(target_os = "macos", windows))]
    fn gitlink_boundary_stays_conservative_on_aliasing_platforms() {
        let temp = tempfile::TempDir::new().unwrap();
        let vendor = temp.path().join("Vendor");
        std::fs::create_dir(&vendor).unwrap();

        assert!(is_git_boundary_dir(
            &vendor,
            1,
            temp.path(),
            &gitlinks(&["vendor"]),
            false
        ));
        assert!(is_git_boundary_dir(
            &vendor,
            1,
            temp.path(),
            &gitlinks(&["Vendor"]),
            false
        ));
    }

    /// A case-insensitive checkout resolves both spellings to the submodule.
    #[test]
    fn gitlink_boundary_folds_case_when_the_checkout_does() {
        let temp = tempfile::TempDir::new().unwrap();
        let vendor = temp.path().join("Vendor");
        std::fs::create_dir(&vendor).unwrap();

        assert!(is_git_boundary_dir(
            &vendor,
            1,
            temp.path(),
            &gitlinks(&["vendor"]),
            true
        ));
    }

    /// Unrelated directories are never boundaries under either policy.
    #[test]
    fn unrelated_directories_are_not_boundaries() {
        let temp = tempfile::TempDir::new().unwrap();
        let config = temp.path().join("config");
        std::fs::create_dir(&config).unwrap();

        for ignore_case in [false, true] {
            assert!(!is_git_boundary_dir(
                &config,
                1,
                temp.path(),
                &gitlinks(&["vendor"]),
                ignore_case
            ));
        }
    }
}
