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

pub(crate) fn is_git_boundary_dir(
    path: &Path,
    depth: usize,
    source_root: &Path,
    gitlinks: &HashSet<String>,
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
            // A repository boundary is a safety filter, so prefer a
            // conservative case/normalization match even when Git's current
            // config claims the filesystem is case-sensitive.
            crate::git::repo_paths_equivalent(rel.as_str(), gitlink, true)
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
