use super::*;

/// The fingerprint production computed before the verified kernel replaced it.
fn reference_fingerprint(bytes: &[u8]) -> u64 {
    let mut fingerprint = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        fingerprint ^= u64::from(*byte);
        fingerprint = fingerprint.wrapping_mul(0x0000_0100_0000_01b3);
    }
    fingerprint
}

/// A reader that hands out its bytes in the given (short) chunk sizes.
struct ChunkedReader<'a> {
    data: &'a [u8],
    sizes: &'a [usize],
    next: usize,
}

impl Read for ChunkedReader<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let size = self.sizes[self.next % self.sizes.len()]
            .min(buffer.len())
            .min(self.data.len());
        self.next += 1;
        buffer[..size].copy_from_slice(&self.data[..size]);
        self.data = &self.data[size..];
        Ok(size)
    }
}

fn sample_bytes(len: usize) -> Vec<u8> {
    (0..len)
        .map(|i| (i.wrapping_mul(131).wrapping_add(i / 7)) as u8)
        .collect()
}

#[test]
fn fingerprint_matches_published_test_vectors() {
    assert_eq!(fingerprint_bytes(b""), 0xcbf2_9ce4_8422_2325);
    assert_eq!(fingerprint_bytes(b"a"), 0xaf63_dc4c_8601_ec8c);
    assert_eq!(fingerprint_bytes(b"foobar"), 0x8594_4171_f739_67e8);
}

#[test]
fn fingerprint_matches_the_replaced_implementation() {
    for len in [0, 1, 2, 7, 64, 1000, 70_000] {
        let data = sample_bytes(len);
        assert_eq!(fingerprint_bytes(&data), reference_fingerprint(&data));
    }
}

#[test]
fn streamed_fingerprint_is_independent_of_read_sizes() {
    let data = sample_bytes(150_000);
    for sizes in [
        &[1usize][..],
        &[3, 1, 4, 1, 5, 9, 2, 6][..],
        &[65_536][..],
        &[70_000, 1][..],
    ] {
        let mut reader = ChunkedReader {
            data: &data,
            sizes,
            next: 0,
        };
        assert_eq!(
            fingerprint_reader(&mut reader).unwrap(),
            reference_fingerprint(&data),
            "{sizes:?}"
        );
    }
}

/// `relative_components` as production computed it with `Path::components`.
#[cfg(unix)]
fn reference_relative(path: &str) -> Result<Vec<Vec<u8>>, &'static str> {
    use std::os::unix::ffi::OsStrExt;
    use std::path::Component;

    let mut result = Vec::new();
    for component in Path::new(path).components() {
        match component {
            Component::Normal(name) => result.push(name.as_bytes().to_vec()),
            _ => return Err("unsafe"),
        }
    }
    if result.is_empty() {
        return Err("empty");
    }
    Ok(result)
}

/// `open_canonical_directory`'s walk as production computed it with
/// `Path::components`: `None` when not absolute, otherwise the names opened
/// and whether the walk stopped at an invalid component.
#[cfg(unix)]
fn reference_absolute(path: &str) -> Option<(Vec<Vec<u8>>, bool)> {
    use std::os::unix::ffi::OsStrExt;
    use std::path::Component;

    let path = Path::new(path);
    if !path.is_absolute() {
        return None;
    }
    let mut names = Vec::new();
    for component in path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(name) => names.push(name.as_bytes().to_vec()),
            _ => return Some((names, true)),
        }
    }
    Some((names, false))
}

/// Every string over `alphabet` up to `max_len` characters.
#[cfg(unix)]
fn all_strings(alphabet: &[&str], max_len: usize) -> Vec<String> {
    let mut all = vec![String::new()];
    let mut frontier = vec![String::new()];
    for _ in 0..max_len {
        let mut next = Vec::new();
        for prefix in &frontier {
            for piece in alphabet {
                next.push(format!("{prefix}{piece}"));
            }
        }
        all.extend(next.iter().cloned());
        frontier = next;
    }
    all
}

#[cfg(unix)]
#[test]
fn relative_components_kernel_matches_path_components() {
    use waft_verified::components::RelativeError;

    let mut inputs = all_strings(&["a", ".", "/", "\\"], 7);
    inputs.extend(all_strings(&["é", ".", "/"], 4));
    for input in inputs {
        let kernel = waft_verified::components::relative_components(input.as_bytes());
        let kernel = match kernel {
            Ok(names) => Ok(names),
            Err(RelativeError::Unsafe) => Err("unsafe"),
            Err(RelativeError::Empty) => Err("empty"),
        };
        assert_eq!(kernel, reference_relative(&input), "{input:?}");
    }
}

#[cfg(unix)]
#[test]
fn absolute_walk_kernel_matches_path_components() {
    let mut inputs = all_strings(&["a", ".", "/", "\\"], 7);
    inputs.extend(all_strings(&["é", ".", "/"], 4));
    for input in inputs {
        let kernel = waft_verified::components::absolute_walk(input.as_bytes())
            .map(|walk| (walk.names, walk.refused));
        assert_eq!(kernel, reference_absolute(&input), "{input:?}");
    }
}

#[cfg(unix)]
#[test]
fn relative_components_keeps_its_error_messages() {
    let cases = [
        ("", "repository-relative path is empty"),
        (
            "a/..",
            "repository-relative path contains an unsafe component",
        ),
        (
            "/a",
            "repository-relative path contains an unsafe component",
        ),
        (
            "./a",
            "repository-relative path contains an unsafe component",
        ),
    ];
    for (input, message) in cases {
        let error =
            relative_components(&RepoRelPath::from_normalized(input.to_string())).expect_err(input);
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(error.to_string(), message);
    }
    assert_eq!(
        relative_components(&RepoRelPath::from_normalized("a//b/./c/".to_string())).unwrap(),
        vec![
            OsString::from("a"),
            OsString::from("b"),
            OsString::from("c")
        ]
    );
}

#[cfg(unix)]
#[test]
fn open_canonical_directory_keeps_its_error_messages() {
    let error = open_canonical_directory(Path::new("relative/root")).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    assert_eq!(
        error.to_string(),
        "repository root did not resolve to an absolute path"
    );

    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let error = open_canonical_directory(&root.join("..").join("x")).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    assert_eq!(
        error.to_string(),
        "canonical repository root contains an invalid component"
    );
    assert!(open_canonical_directory(&root).is_ok());
}

#[test]
fn file_comparison_conversion_is_variant_for_variant() {
    use waft_verified::fs_checks::compare::Comparison;

    for production in [
        FileComparison::Equal,
        FileComparison::PermissionsDiffer,
        FileComparison::ContentDiffers,
    ] {
        let kernel = match production {
            FileComparison::Equal => Comparison::Equal,
            FileComparison::PermissionsDiffer => Comparison::PermissionsDiffer,
            FileComparison::ContentDiffers => Comparison::ContentDiffers,
        };
        assert_eq!(FileComparison::from_kernel(kernel), production);
    }
}

#[test]
fn stream_comparison_reports_differing_chunk_lengths() {
    use waft_verified::fs_checks::compare::{ChunkStep, StreamComparison};

    assert!(StreamComparison::begin(1, 2, 0, 0).is_none());
    // The documented quirk: identical bytes split differently still differ.
    let comparison = StreamComparison::begin(2, 2, 0o644, 0o644).unwrap();
    assert!(matches!(
        comparison.compare_chunks(&[7], &[7, 7]),
        ChunkStep::Differ
    ));
    let comparison = StreamComparison::begin(2, 2, 0o644, 0o755).unwrap();
    let ChunkStep::More(comparison) = comparison.compare_chunks(&[7, 7], &[7, 7]) else {
        panic!("identical chunks continue");
    };
    let ChunkStep::End(matched) = comparison.compare_chunks(&[], &[]) else {
        panic!("empty chunks end the comparison");
    };
    assert_eq!(
        FileComparison::from_kernel(matched.verdict()),
        FileComparison::PermissionsDiffer
    );
}

#[cfg(unix)]
#[test]
fn realfs_compare_files_classifies_lengths_bytes_and_modes() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempfile::TempDir::new().unwrap();
    let file = |name: &str, content: &[u8], mode: u32| {
        let path = tmp.path().join(name);
        fs::write(&path, content).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
        path
    };
    let big = sample_bytes(200_000);
    let mut big_tail = big.clone();
    *big_tail.last_mut().unwrap() ^= 1;
    let cases = [
        (&b""[..], 0o644, &b""[..], 0o644, FileComparison::Equal),
        (
            &b"same\n"[..],
            0o644,
            &b"same\n"[..],
            0o644,
            FileComparison::Equal,
        ),
        (
            &b"same\n"[..],
            0o644,
            &b"same\n"[..],
            0o600,
            FileComparison::PermissionsDiffer,
        ),
        (
            &b"left\n"[..],
            0o644,
            &b"rght\n"[..],
            0o644,
            FileComparison::ContentDiffers,
        ),
        (
            &b"short"[..],
            0o644,
            &b"longer"[..],
            0o600,
            FileComparison::ContentDiffers,
        ),
        (&big[..], 0o644, &big[..], 0o644, FileComparison::Equal),
        (
            &big[..],
            0o644,
            &big_tail[..],
            0o644,
            FileComparison::ContentDiffers,
        ),
    ];
    for (index, (left, left_mode, right, right_mode, expected)) in cases.into_iter().enumerate() {
        let left = file(&format!("left-{index}"), left, left_mode);
        let right = file(&format!("right-{index}"), right, right_mode);
        assert_eq!(
            RealFs.compare_files(&left, &right).unwrap(),
            expected,
            "case {index}"
        );
    }
}

#[test]
fn modified_stamps_are_equal_exactly_when_times_are() {
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    let mut times = vec![None, Some(UNIX_EPOCH)];
    for offset in [
        Duration::from_nanos(1),
        Duration::from_millis(1500),
        Duration::from_secs(1),
        Duration::new(1 << 33, 999_999_999),
    ] {
        times.push(UNIX_EPOCH.checked_add(offset));
        times.push(UNIX_EPOCH.checked_sub(offset));
    }
    times.push(Some(SystemTime::now()));
    for left in &times {
        for right in &times {
            assert_eq!(
                modified_stamp(*left) == modified_stamp(*right),
                left == right,
                "{left:?} {right:?}"
            );
        }
    }
}

#[cfg(unix)]
#[test]
fn stability_kernel_matches_the_replaced_predicates() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    use waft_verified::fs_checks::stability;

    fn same_file(left: &fs::Metadata, right: &fs::Metadata) -> bool {
        left.dev() == right.dev() && left.ino() == right.ino()
    }
    fn same_file_state(left: &fs::Metadata, right: &fs::Metadata) -> bool {
        left.len() == right.len()
            && left.modified().ok() == right.modified().ok()
            && change_time_signature(left) == change_time_signature(right)
            && permission_signature(left) == permission_signature(right)
    }

    let tmp = tempfile::TempDir::new().unwrap();
    let path = tmp.path().join("file");
    let other = tmp.path().join("other");
    let link = tmp.path().join("link");
    fs::write(&path, b"one\n").unwrap();
    fs::write(&other, b"one\n").unwrap();
    std::os::unix::fs::symlink(&path, &link).unwrap();
    let file = fs::File::open(&path).unwrap();

    let mut observed = vec![
        fs::symlink_metadata(&path).unwrap(),
        file.metadata().unwrap(),
        fs::symlink_metadata(&other).unwrap(),
        fs::symlink_metadata(&link).unwrap(),
        fs::symlink_metadata(tmp.path()).unwrap(),
    ];
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    observed.push(file.metadata().unwrap());
    fs::write(&path, b"two\n").unwrap();
    observed.push(fs::symlink_metadata(&path).unwrap());
    fs::write(&path, b"three\n").unwrap();
    observed.push(file.metadata().unwrap());

    for a in &observed {
        let oa = observation(a);
        assert_eq!(
            oa.identity,
            stability::Identity::Inode {
                dev: a.dev(),
                ino: a.ino()
            }
        );
        for b in &observed {
            let ob = observation(b);
            assert_eq!(
                stability::unchanged_handle(&oa, &ob),
                b.file_type().is_file() && same_file(a, b) && same_file_state(a, b)
            );
            for c in &observed {
                let oc = observation(c);
                assert_eq!(
                    stability::opened_stably(&oa, &ob, &oc),
                    c.file_type().is_file() && same_file(a, b) && same_file(b, c)
                );
                assert_eq!(
                    stability::unchanged_through_path(&oa, &ob, &oc),
                    c.file_type().is_file()
                        && same_file(a, b)
                        && same_file(b, c)
                        && same_file_state(a, b)
                        && same_file_state(b, c)
                );
            }
        }
    }
}

#[cfg(unix)]
#[test]
fn rename_flag_table_holds_exactly_the_unsupported_errnos() {
    let expected = [
        rustix::io::Errno::NOTSUP,
        rustix::io::Errno::OPNOTSUPP,
        rustix::io::Errno::INVAL,
        rustix::io::Errno::NOSYS,
    ];
    for raw in RENAME_FLAG_UNSUPPORTED {
        assert!(expected.contains(&rustix::io::Errno::from_raw_os_error(raw)));
    }
    for errno in expected {
        assert!(RENAME_FLAG_UNSUPPORTED.contains(&errno.raw_os_error()));
    }
}

#[test]
fn reflink_strategy_table_is_unchanged() {
    assert!(!attempts_reflink(CopyStrategy::SimpleCopy));
    assert!(attempts_reflink(CopyStrategy::CowCopy));
    assert_eq!(
        attempts_reflink(CopyStrategy::Auto),
        cfg!(target_os = "macos")
    );
}

#[test]
fn temporary_name_budget_grants_exactly_128_attempts() {
    let mut attempts = waft_verified::fs_checks::steps::TempNameAttempts::new();
    let mut granted = 0;
    while attempts.try_another() {
        granted += 1;
        assert!(granted <= 128);
    }
    assert_eq!(granted, 128);
    assert!(!attempts.try_another());
}

#[test]
fn mkdir_answers_decide_reopen_and_sync() {
    use waft_verified::fs_checks::steps::{MkdirAnswer, after_mkdir};

    assert_eq!(after_mkdir::<i32>(MkdirAnswer::Created), Ok(true));
    assert_eq!(after_mkdir::<i32>(MkdirAnswer::AlreadyExists), Ok(false));
    assert_eq!(after_mkdir(MkdirAnswer::Failed(5)), Err(5));
}
