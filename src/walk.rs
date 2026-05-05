use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::path::RepoRelPath;

pub(crate) fn is_git_boundary_dir(
    path: &Path,
    depth: usize,
    source_root: &Path,
    gitlinks: &HashSet<String>,
) -> bool {
    if path.file_name().is_some_and(|name| name == ".git") {
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
        && gitlinks.contains(rel.as_str())
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
