use std::collections::HashSet;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use waft_verified::boundary::{self as kernel, BoundaryProbe, BoundaryProbes};
use waft_verified::selection;

use crate::path::RepoRelPath;

/// Whether this platform's filesystems may alias differently-cased names
/// even when `core.ignoreCase` is false (macOS, Windows).
const ALIASING_PLATFORM: bool = cfg!(any(target_os = "macos", windows));

/// Match control filenames according to the native worktree threat model.
///
/// macOS and Windows may alias differently-cased spellings even when
/// `core.ignoreCase` is explicitly false. Discovery, validation, and nested
/// repository boundaries must all make the same conservative decision.
///
/// The decision is the verified `waft_verified::boundary::special_name_matches`
/// over the name's encoded bytes: exact bytes, or ASCII case-insensitive on
/// aliasing platforms. (The kernel proves that an ASCII case-insensitive
/// match of an ASCII name is itself ASCII, so the former `to_str` check
/// never rejected a match.)
pub(crate) fn special_filename_matches(actual: &OsStr, expected: &str) -> bool {
    kernel::special_name_matches(
        actual.as_encoded_bytes(),
        expected.as_bytes(),
        ALIASING_PLATFORM,
    )
}

/// The no-follow type of a walked entry, as kernel data.
///
/// Both `.worktreeinclude` walks classify entries here: the rule-file
/// existence walk (`git_policy::rule_file_counts`) and the wt-0.39 walk
/// (`selection::wt_entry_consulted`), so the two decide over the same
/// classification. `is_dir`, `is_symlink` and `is_file` are mutually
/// exclusive for a no-follow file type, so the order of the tests does not
/// matter.
pub(crate) fn entry_kind(file_type: std::fs::FileType) -> selection::EntryKind {
    if file_type.is_dir() {
        selection::EntryKind::Dir
    } else if file_type.is_symlink() {
        selection::EntryKind::Symlink
    } else if file_type.is_file() {
        selection::EntryKind::File
    } else {
        selection::EntryKind::Other
    }
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
    kernel::boundary_case_folds(ignore_case, ALIASING_PLATFORM)
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
///
/// The verified `waft_verified::boundary::is_git_boundary` decides which of
/// the probes below to ask and in which order (`.git` is a directory, then
/// `.git` is a gitfile whose target exists, then the path is a gitlink),
/// stopping at the first that answers yes. It never probes an entry named
/// `.git` or the walk root itself.
pub(crate) fn is_git_boundary_dir(
    path: &Path,
    depth: usize,
    source_root: &Path,
    gitlinks: &HashSet<String>,
    ignore_case: bool,
) -> bool {
    let mut probes = FsBoundaryProbes {
        path,
        source_root,
        gitlinks,
    };
    kernel::is_git_boundary(
        path.file_name().map(OsStr::as_encoded_bytes),
        depth,
        ignore_case,
        ALIASING_PLATFORM,
        &mut probes,
    )
}

/// Answers the boundary kernel's probes from the filesystem and the gitlink
/// set: the trusted half of the boundary decision.
struct FsBoundaryProbes<'a> {
    path: &'a Path,
    source_root: &'a Path,
    gitlinks: &'a HashSet<String>,
}

impl BoundaryProbes for FsBoundaryProbes<'_> {
    fn probe(&mut self, probe: BoundaryProbe) -> bool {
        match probe {
            BoundaryProbe::DotGitIsDir => self.path.join(".git").is_dir(),
            BoundaryProbe::DotGitFileTargetExists => {
                let dot_git = self.path.join(".git");
                dot_git.is_file()
                    && read_dot_git_pointer(&dot_git).is_some_and(|target| target.exists())
            }
            BoundaryProbe::GitlinkMatches { fold } => {
                RepoRelPath::normalize(self.path, self.source_root).is_ok_and(|rel| {
                    self.gitlinks.iter().any(|gitlink| {
                        crate::git::repo_paths_equivalent(rel.as_str(), gitlink, fold)
                    })
                })
            }
        }
    }
}

/// Resolve a `.git` gitfile's `gitdir:` pointer (parsed by the verified
/// `waft_verified::boundary::gitfile::gitdir_target`) against the file's
/// directory.
fn read_dot_git_pointer(path: &Path) -> Option<PathBuf> {
    let content = std::fs::read_to_string(path).ok()?;
    let candidate = PathBuf::from(kernel::gitfile::gitdir_target(&content)?);
    if candidate.is_absolute() {
        return Some(candidate);
    }
    path.parent().map(|p| p.join(candidate))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn entry_kinds_classify_every_file_type_without_following_links() {
        let temp = tempfile::TempDir::new().unwrap();
        let root = temp.path();
        std::fs::write(root.join("file"), "x").unwrap();
        std::fs::create_dir(root.join("dir")).unwrap();
        std::os::unix::fs::symlink("file", root.join("link")).unwrap();
        std::os::unix::fs::symlink("dir", root.join("dirlink")).unwrap();
        let kind = |path: &Path| entry_kind(std::fs::symlink_metadata(path).unwrap().file_type());
        assert_eq!(kind(root), selection::EntryKind::Dir);
        assert_eq!(kind(&root.join("file")), selection::EntryKind::File);
        assert_eq!(kind(&root.join("dir")), selection::EntryKind::Dir);
        assert_eq!(kind(&root.join("link")), selection::EntryKind::Symlink);
        assert_eq!(kind(&root.join("dirlink")), selection::EntryKind::Symlink);
        assert_eq!(kind(Path::new("/dev/null")), selection::EntryKind::Other);
    }

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

    /// The control-file name rule as written before the verified kernel,
    /// with the platform flag as a parameter; kept as a test oracle.
    fn reference_special_filename_matches(actual: &OsStr, expected: &str, aliasing: bool) -> bool {
        actual == OsStr::new(expected)
            || (aliasing
                && actual
                    .to_str()
                    .is_some_and(|name| name.eq_ignore_ascii_case(expected)))
    }

    #[test]
    fn special_name_kernel_matches_reference_on_both_platform_rules() {
        let mut names: Vec<std::ffi::OsString> = [
            ".git",
            ".GIT",
            ".Git",
            ".gIt",
            ".git ",
            "git",
            ".gi",
            ".gitt",
            "",
            ".worktreeinclude",
            ".WorkTreeInclude",
            ".worktreeinclud",
            ".gitignore",
            ".GITIGNORE",
            ".g\u{130}t",
            ".wor\u{212A}treeinclude",
            "\u{FF0E}git",
        ]
        .iter()
        .map(std::ffi::OsString::from)
        .collect();
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt;
            names.push(std::ffi::OsString::from_vec(vec![b'.', b'G', 0xff, b'T']));
            names.push(std::ffi::OsString::from_vec(vec![b'.', b'g', b'i', 0x80]));
        }
        for expected in [".git", ".worktreeinclude", ".gitignore"] {
            for name in &names {
                for aliasing in [false, true] {
                    assert_eq!(
                        kernel::special_name_matches(
                            name.as_encoded_bytes(),
                            expected.as_bytes(),
                            aliasing
                        ),
                        reference_special_filename_matches(name, expected, aliasing),
                        "{name:?} vs {expected:?} (aliasing {aliasing})"
                    );
                }
                assert_eq!(
                    special_filename_matches(name, expected),
                    reference_special_filename_matches(name, expected, ALIASING_PLATFORM),
                );
            }
        }
    }

    /// Nested clones (`.git` directory) and gitfiles whose target exists are
    /// boundaries below the root; a dangling gitfile is not; the root never is.
    #[test]
    fn nested_repository_markers_are_boundaries() {
        let temp = tempfile::TempDir::new().unwrap();
        let clone = temp.path().join("clone");
        std::fs::create_dir_all(clone.join(".git")).unwrap();
        let linked = temp.path().join("linked");
        std::fs::create_dir(&linked).unwrap();
        std::fs::write(linked.join(".git"), "gitdir: ../store\n").unwrap();
        let dangling = temp.path().join("dangling");
        std::fs::create_dir(&dangling).unwrap();
        std::fs::write(dangling.join(".git"), "gitdir: ../missing\n").unwrap();
        let empty_pointer = temp.path().join("empty-pointer");
        std::fs::create_dir(&empty_pointer).unwrap();
        std::fs::write(empty_pointer.join(".git"), "gitdir:   \n").unwrap();

        let none = gitlinks(&[]);
        assert!(is_git_boundary_dir(&clone, 1, temp.path(), &none, false));
        assert!(!is_git_boundary_dir(&linked, 1, temp.path(), &none, false));
        std::fs::create_dir(temp.path().join("store")).unwrap();
        assert!(is_git_boundary_dir(&linked, 1, temp.path(), &none, false));
        assert!(!is_git_boundary_dir(
            &dangling,
            1,
            temp.path(),
            &none,
            false
        ));
        assert!(!is_git_boundary_dir(
            &empty_pointer,
            1,
            temp.path(),
            &none,
            false
        ));
        assert!(!is_git_boundary_dir(&clone, 0, &clone, &none, false));
        assert!(is_git_boundary_dir(
            &clone.join(".git"),
            0,
            &clone,
            &none,
            false
        ));
    }

    #[test]
    fn boundary_folding_follows_checkout_or_platform() {
        assert!(boundary_case_folds(true));
        assert_eq!(boundary_case_folds(false), ALIASING_PLATFORM);
    }

    /// The gitfile parse as written before the verified kernel; kept as a
    /// test oracle.
    fn reference_gitdir_target(content: &str) -> Option<String> {
        for line in content.lines() {
            if let Some(rest) = line.strip_prefix("gitdir:") {
                let target = rest.trim();
                if target.is_empty() {
                    return None;
                }
                return Some(target.to_string());
            }
        }
        None
    }

    #[test]
    fn gitdir_kernel_matches_reference_parse() {
        let contents = [
            "",
            "gitdir: ../.git/worktrees/x\n",
            "gitdir: ../.git/worktrees/x",
            "gitdir:../rel\r\n",
            "gitdir: /abs/path\r",
            "gitdir: a\r\r\n",
            "gitdir:\n",
            "gitdir:   \t \ngitdir: later\n",
            "first line\ngitdir: second\n",
            " gitdir: indented\n",
            "GITDIR: upper\n",
            "gitdir x\ngitdir: y\n",
            "gitdir: \u{a0}nbsp\u{3000}\n",
            "gitdir:\u{2028}sep\u{2029}\u{85}\n",
            "gitdir: inner  space \u{1680}\n",
            "\n\n\ngitdir: after blanks",
            "gitdir:\r\ngitdir: second\n",
            "xgitdir: no\n",
            "gitdi",
            "gitdir:",
            "gitdir: \u{1F600} emoji \n",
            "\r\ngitdir: crlf-first\r\n",
            "gitdir: a\rb\n",
        ];
        for content in contents {
            assert_eq!(
                kernel::gitfile::gitdir_target(content),
                reference_gitdir_target(content),
                "{content:?}"
            );
        }
    }

    /// Records the probes the boundary kernel asks, answering from a script.
    struct ScriptedProbes {
        answers: Vec<bool>,
        asked: Vec<BoundaryProbe>,
    }

    impl BoundaryProbes for ScriptedProbes {
        fn probe(&mut self, probe: BoundaryProbe) -> bool {
            let answer = self.answers[self.asked.len()];
            self.asked.push(probe);
            answer
        }
    }

    fn run_boundary(
        name: &str,
        depth: usize,
        ignore_case: bool,
        answers: &[bool],
    ) -> (bool, Vec<BoundaryProbe>) {
        let mut probes = ScriptedProbes {
            answers: answers.to_vec(),
            asked: Vec::new(),
        };
        let pruned = kernel::is_git_boundary(
            Some(name.as_bytes()),
            depth,
            ignore_case,
            ALIASING_PLATFORM,
            &mut probes,
        );
        (pruned, probes.asked)
    }

    #[test]
    fn boundary_probes_are_ordered_and_lazy() {
        let fold = boundary_case_folds(false);
        let all = [
            BoundaryProbe::DotGitIsDir,
            BoundaryProbe::DotGitFileTargetExists,
            BoundaryProbe::GitlinkMatches { fold },
        ];
        assert_eq!(run_boundary(".git", 3, false, &[]), (true, vec![]));
        assert_eq!(run_boundary("dir", 0, false, &[]), (false, vec![]));
        assert_eq!(
            run_boundary("dir", 1, false, &[true]),
            (true, all[..1].to_vec())
        );
        assert_eq!(
            run_boundary("dir", 1, false, &[false, true]),
            (true, all[..2].to_vec())
        );
        assert_eq!(
            run_boundary("dir", 1, false, &[false, false, true]),
            (true, all.to_vec())
        );
        assert_eq!(
            run_boundary("dir", 1, false, &[false, false, false]),
            (false, all.to_vec())
        );
        assert_eq!(
            run_boundary("dir", 2, true, &[false, false, false]).1[2],
            BoundaryProbe::GitlinkMatches { fold: true }
        );
    }
}
