//! Filesystem abstraction for testability.

use std::fs;
use std::io::{self, Read};
use std::path::Path;

#[cfg(unix)]
use std::ffi::{OsStr, OsString};
#[cfg(unix)]
use std::io::{Seek, SeekFrom};
#[cfg(unix)]
use std::os::fd::OwnedFd;
#[cfg(unix)]
use std::path::Component;

use crate::config::CopyStrategy;
use crate::model::{DestinationExpectation, FileSnapshot};
use crate::path::RepoRelPath;

/// Immutable inputs for one conditional file publication.
#[derive(Debug, Clone, Copy)]
pub struct CopyFileRequest<'a> {
    /// Canonical source worktree root.
    pub source_root: &'a Path,
    /// Canonical destination worktree root.
    pub destination_root: &'a Path,
    /// Validated repository-relative file path.
    pub rel_path: &'a RepoRelPath,
    /// Requested copy mechanism.
    pub strategy: CopyStrategy,
    /// Exact source state captured while planning.
    pub expected_source: &'a FileSnapshot,
    /// Expected destination state. Existing destinations are rejected.
    pub expected_destination: &'a DestinationExpectation,
}

/// Abstraction over filesystem operations needed by the planner and executor.
pub trait FileSystem {
    /// Check if a path exists.
    fn exists(&self, path: &Path) -> bool;

    /// Check if a path is a regular file.
    fn is_file(&self, path: &Path) -> bool;

    /// Check if a path is a directory.
    fn is_dir(&self, path: &Path) -> bool;

    /// Check if a path is a symlink (without following it).
    fn is_symlink(&self, path: &Path) -> bool;

    /// Read the entire contents of a file.
    fn read(&self, path: &Path) -> io::Result<Vec<u8>>;

    /// Compare two regular files without requiring implementations to retain
    /// their complete contents in memory.
    fn files_equal(&self, left: &Path, right: &Path) -> io::Result<bool> {
        Ok(self.read(left)? == self.read(right)?)
    }

    /// Capture a bounded-memory snapshot used to detect destination changes
    /// between planning and execution.
    fn file_snapshot(&self, path: &Path) -> io::Result<FileSnapshot> {
        let data = self.read(path)?;
        Ok(FileSnapshot::new(
            data.len() as u64,
            fingerprint_bytes(&data),
            0,
            None,
        ))
    }

    /// Check whether any component of the path (excluding the final component)
    /// is a symlink.
    fn parent_has_symlink(&self, path: &Path) -> bool;

    /// Copy `src` to `dst` using the given [`CopyStrategy`].
    ///
    /// Implementations receive repository roots and a validated relative path
    /// so they can anchor traversal to directory handles rather than resolving
    /// independently mutable absolute pathnames. The destination is published
    /// only if it is still missing; replacement of existing paths is rejected.
    /// `before_publish` runs after content preparation and immediately before
    /// final namespace validation and no-clobber publication.
    fn copy_file(
        &self,
        request: CopyFileRequest<'_>,
        before_publish: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<()>;
}

/// Real filesystem implementation.
#[derive(Debug, Default)]
pub struct RealFs;

impl FileSystem for RealFs {
    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn is_file(&self, path: &Path) -> bool {
        path.is_file()
    }

    fn is_dir(&self, path: &Path) -> bool {
        path.is_dir()
    }

    fn is_symlink(&self, path: &Path) -> bool {
        fs::symlink_metadata(path)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false)
    }

    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        fs::read(path)
    }

    fn files_equal(&self, left: &Path, right: &Path) -> io::Result<bool> {
        let (mut left_file, left_metadata) = open_stable_regular_file(left)?;
        let (mut right_file, right_metadata) = open_stable_regular_file(right)?;

        if left_metadata.len() != right_metadata.len()
            || permission_signature(&left_metadata) != permission_signature(&right_metadata)
        {
            return Ok(false);
        }

        let mut left_buffer = [0u8; 64 * 1024];
        let mut right_buffer = [0u8; 64 * 1024];
        loop {
            let left_read = left_file.read(&mut left_buffer)?;
            let right_read = right_file.read(&mut right_buffer)?;
            if left_read != right_read || left_buffer[..left_read] != right_buffer[..right_read] {
                return Ok(false);
            }
            if left_read == 0 {
                break;
            }
        }

        ensure_open_file_unchanged(left, &left_file, &left_metadata)?;
        ensure_open_file_unchanged(right, &right_file, &right_metadata)?;
        Ok(true)
    }

    fn file_snapshot(&self, path: &Path) -> io::Result<FileSnapshot> {
        snapshot_regular_file(path)
    }

    fn parent_has_symlink(&self, path: &Path) -> bool {
        let mut current = path.to_path_buf();
        // Walk up from the file's parent, checking each component
        while let Some(parent) = current.parent() {
            if parent == current {
                break;
            }
            if fs::symlink_metadata(parent)
                .map(|m| m.file_type().is_symlink())
                .unwrap_or(false)
            {
                return true;
            }
            current = parent.to_path_buf();
        }
        false
    }

    fn copy_file(
        &self,
        request: CopyFileRequest<'_>,
        before_publish: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<()> {
        if matches!(
            request.expected_destination,
            DestinationExpectation::ExistingUntracked(_)
        ) {
            return Err(overwrite_disabled_error(request.rel_path));
        }

        #[cfg(unix)]
        {
            copy_file_anchored_unix(
                request.source_root,
                request.destination_root,
                request.rel_path,
                request.strategy,
                request.expected_source,
                before_publish,
            )
        }

        #[cfg(not(unix))]
        {
            copy_file_path_fallback(
                &request.rel_path.to_path(request.source_root),
                &request.rel_path.to_path(request.destination_root),
                request.strategy,
                request.expected_source,
                before_publish,
            )
        }
    }
}

fn fingerprint_bytes(bytes: &[u8]) -> u64 {
    let mut fingerprint = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        fingerprint ^= u64::from(*byte);
        fingerprint = fingerprint.wrapping_mul(0x0000_0100_0000_01b3);
    }
    fingerprint
}

fn fingerprint_reader(reader: &mut impl Read) -> io::Result<u64> {
    let mut fingerprint = 0xcbf2_9ce4_8422_2325u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            return Ok(fingerprint);
        }
        for byte in &buffer[..count] {
            fingerprint ^= u64::from(*byte);
            fingerprint = fingerprint.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
}

fn snapshot_regular_file(path: &Path) -> io::Result<FileSnapshot> {
    let (mut file, metadata) = open_stable_regular_file(path)?;
    let content_fingerprint = fingerprint_reader(&mut file)?;
    ensure_open_file_unchanged(path, &file, &metadata)?;
    Ok(FileSnapshot::new(
        metadata.len(),
        content_fingerprint,
        permission_signature(&metadata),
        file_identity(&metadata),
    ))
}

#[cfg(unix)]
fn snapshot_open_regular_file(
    file: &mut fs::File,
    metadata: &fs::Metadata,
) -> io::Result<FileSnapshot> {
    file.seek(SeekFrom::Start(0))?;
    let content_fingerprint = fingerprint_reader(file)?;
    ensure_open_handle_unchanged(file, metadata)?;
    file.seek(SeekFrom::Start(0))?;
    Ok(FileSnapshot::new(
        metadata.len(),
        content_fingerprint,
        permission_signature(metadata),
        file_identity(metadata),
    ))
}

fn open_stable_regular_file(path: &Path) -> io::Result<(fs::File, fs::Metadata)> {
    let path_metadata = fs::symlink_metadata(path)?;
    if !path_metadata.file_type().is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "path is not a regular file",
        ));
    }

    let file = open_regular_candidate(path)?;
    let file_metadata = file.metadata()?;
    let current_path_metadata = fs::symlink_metadata(path)?;
    if !current_path_metadata.file_type().is_file()
        || !same_file(&path_metadata, &file_metadata)
        || !same_file(&file_metadata, &current_path_metadata)
    {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "file changed while it was being opened",
        ));
    }
    Ok((file, file_metadata))
}

fn open_regular_candidate(path: &Path) -> io::Result<fs::File> {
    #[cfg(unix)]
    {
        let fd = rustix::fs::open(
            path,
            rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::NONBLOCK
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )?;
        Ok(fs::File::from(fd))
    }

    #[cfg(not(unix))]
    {
        fs::File::open(path)
    }
}

fn ensure_open_file_unchanged(
    path: &Path,
    file: &fs::File,
    initial: &fs::Metadata,
) -> io::Result<()> {
    let handle_metadata = file.metadata()?;
    let path_metadata = fs::symlink_metadata(path)?;
    if !path_metadata.file_type().is_file()
        || !same_file(initial, &handle_metadata)
        || !same_file(&handle_metadata, &path_metadata)
        || !same_file_state(initial, &handle_metadata)
        || !same_file_state(&handle_metadata, &path_metadata)
    {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "file changed while it was being read",
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn same_file(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    left.dev() == right.dev() && left.ino() == right.ino()
}

#[cfg(not(unix))]
fn same_file(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    same_file_state(left, right)
}

fn same_file_state(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    left.len() == right.len()
        && left.modified().ok() == right.modified().ok()
        && change_time_signature(left) == change_time_signature(right)
        && permission_signature(left) == permission_signature(right)
}

#[cfg(unix)]
fn change_time_signature(metadata: &fs::Metadata) -> Option<(i64, i64)> {
    use std::os::unix::fs::MetadataExt;
    Some((metadata.ctime(), metadata.ctime_nsec()))
}

#[cfg(not(unix))]
fn change_time_signature(_metadata: &fs::Metadata) -> Option<(i64, i64)> {
    None
}

#[cfg(unix)]
fn permission_signature(metadata: &fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode()
}

#[cfg(not(unix))]
fn permission_signature(metadata: &fs::Metadata) -> u32 {
    u32::from(metadata.permissions().readonly())
}

#[cfg(unix)]
fn file_identity(metadata: &fs::Metadata) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    Some((metadata.dev(), metadata.ino()))
}

#[cfg(not(unix))]
fn file_identity(_metadata: &fs::Metadata) -> Option<(u64, u64)> {
    None
}

fn overwrite_disabled_error(rel_path: &RepoRelPath) -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!(
            "{rel_path}: replacing an existing destination is disabled because it cannot be made race-safe; review and remove the destination file, then rerun"
        ),
    )
}

#[cfg(unix)]
const DIRECTORY_FLAGS: rustix::fs::OFlags = rustix::fs::OFlags::RDONLY
    .union(rustix::fs::OFlags::DIRECTORY)
    .union(rustix::fs::OFlags::NOFOLLOW)
    .union(rustix::fs::OFlags::CLOEXEC);

#[cfg(unix)]
fn copy_file_anchored_unix(
    source_root: &Path,
    destination_root: &Path,
    rel_path: &RepoRelPath,
    strategy: CopyStrategy,
    expected_source: &FileSnapshot,
    before_publish: &mut dyn FnMut() -> io::Result<()>,
) -> io::Result<()> {
    let source_root = open_canonical_directory(source_root)?;
    let destination_root = open_canonical_directory(destination_root)?;

    // Open the source through a no-follow component walk. From this point on,
    // renames of source path components cannot redirect the bytes being read.
    let (source_parent, source_name) = open_relative_parent(&source_root, rel_path, false)?;
    let source_fd = rustix::fs::openat(
        &source_parent,
        &source_name,
        rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::NONBLOCK
            | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )?;
    let mut source = fs::File::from(source_fd);
    let source_metadata = source.metadata()?;
    if !source_metadata.file_type().is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "source is not a regular file",
        ));
    }
    let actual_source = snapshot_open_regular_file(&mut source, &source_metadata)?;
    if &actual_source != expected_source {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "source changed after planning",
        ));
    }

    // Destination parents are created and reopened one component at a time
    // relative to an already-open directory handle. Symlinks are never
    // traversed, including if another process races directory creation.
    let (destination_parent, destination_name) =
        open_relative_parent(&destination_root, rel_path, true)?;

    let try_reflink = match strategy {
        CopyStrategy::SimpleCopy => false,
        CopyStrategy::CowCopy => true,
        CopyStrategy::Auto => cfg!(target_os = "macos"),
    };

    let prepared = if try_reflink {
        try_reflink_anchored(&source, &destination_parent)?
    } else {
        None
    };
    let (temporary_name, temporary) = match prepared {
        Some(prepared) => prepared,
        None => {
            source.seek(SeekFrom::Start(0))?;
            let (name, mut file) = create_anchored_temp(&destination_parent)?;
            if let Err(error) = io::copy(&mut source, &mut file) {
                let _ = unlink_anchored(&destination_parent, &name);
                return Err(error);
            }
            (name, file)
        }
    };

    let mut published = false;
    let result = (|| {
        ensure_open_handle_unchanged(&source, &source_metadata)?;
        temporary.set_permissions(source_metadata.permissions())?;
        temporary.sync_all()?;

        // This callback performs the final tracked-index check. Revalidate the
        // parent capability after it returns, then issue NOREPLACE immediately.
        before_publish()?;
        ensure_same_relative_parent(&destination_root, rel_path, &destination_parent)?;
        ensure_name_refers_to_file(&destination_parent, &temporary_name, &temporary)?;
        publish_noreplace(&destination_parent, &temporary_name, &destination_name)?;
        published = true;
        rustix::fs::fsync(&destination_parent)?;
        Ok(())
    })();

    if !published {
        let _ = unlink_anchored(&destination_parent, &temporary_name);
    }
    result
}

#[cfg(unix)]
fn open_canonical_directory(path: &Path) -> io::Result<OwnedFd> {
    if !path.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "repository root did not resolve to an absolute path",
        ));
    }

    let mut current = rustix::fs::open(Path::new("/"), DIRECTORY_FLAGS, rustix::fs::Mode::empty())?;
    for component in path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(name) => {
                current =
                    rustix::fs::openat(&current, name, DIRECTORY_FLAGS, rustix::fs::Mode::empty())?;
            }
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "canonical repository root contains an invalid component",
                ));
            }
        }
    }
    Ok(current)
}

#[cfg(unix)]
fn relative_components(rel_path: &RepoRelPath) -> io::Result<Vec<OsString>> {
    let mut result = Vec::new();
    for component in Path::new(rel_path.as_str()).components() {
        match component {
            Component::Normal(name) => result.push(name.to_os_string()),
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "repository-relative path contains an unsafe component",
                ));
            }
        }
    }
    if result.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "repository-relative path is empty",
        ));
    }
    Ok(result)
}

#[cfg(unix)]
fn open_relative_parent(
    root: &OwnedFd,
    rel_path: &RepoRelPath,
    create: bool,
) -> io::Result<(OwnedFd, OsString)> {
    let mut components = relative_components(rel_path)?;
    let final_name = components
        .pop()
        .expect("relative_components rejects empty paths");
    let mut current = rustix::fs::openat(root, c".", DIRECTORY_FLAGS, rustix::fs::Mode::empty())?;

    for component in components {
        match rustix::fs::openat(
            &current,
            &component,
            DIRECTORY_FLAGS,
            rustix::fs::Mode::empty(),
        ) {
            Ok(next) => current = next,
            Err(error) if create && error == rustix::io::Errno::NOENT => {
                let created = match rustix::fs::mkdirat(
                    &current,
                    &component,
                    rustix::fs::Mode::from_raw_mode(0o777),
                ) {
                    Ok(()) => true,
                    Err(error) if error == rustix::io::Errno::EXIST => false,
                    Err(error) => return Err(error.into()),
                };
                let next = rustix::fs::openat(
                    &current,
                    &component,
                    DIRECTORY_FLAGS,
                    rustix::fs::Mode::empty(),
                )?;
                if created {
                    rustix::fs::fsync(&next)?;
                    rustix::fs::fsync(&current)?;
                }
                current = next;
            }
            Err(error) => return Err(error.into()),
        }
    }
    Ok((current, final_name))
}

#[cfg(unix)]
fn ensure_same_relative_parent(
    root: &OwnedFd,
    rel_path: &RepoRelPath,
    expected: &OwnedFd,
) -> io::Result<()> {
    let (current, _) = open_relative_parent(root, rel_path, false)?;
    let expected = rustix::fs::fstat(expected)?;
    let current = rustix::fs::fstat(&current)?;
    if expected.st_dev == current.st_dev && expected.st_ino == current.st_ino {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "destination parent changed before publication",
        ))
    }
}

#[cfg(unix)]
fn ensure_open_handle_unchanged(file: &fs::File, initial: &fs::Metadata) -> io::Result<()> {
    let current = file.metadata()?;
    if current.file_type().is_file()
        && same_file(initial, &current)
        && same_file_state(initial, &current)
    {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "source changed while it was being copied",
        ))
    }
}

#[cfg(unix)]
fn next_temporary_name() -> io::Result<OsString> {
    let mut random = [0_u8; 16];
    getrandom::fill(&mut random)
        .map_err(|error| io::Error::other(format!("failed to generate temporary name: {error}")))?;
    let mut encoded = String::with_capacity(32);
    for byte in random {
        use std::fmt::Write as _;
        write!(&mut encoded, "{byte:02x}").expect("writing to a String cannot fail");
    }
    Ok(OsString::from(format!(".waft-copy-{encoded}")))
}

#[cfg(unix)]
fn create_anchored_temp(parent: &OwnedFd) -> io::Result<(OsString, fs::File)> {
    for _ in 0..128 {
        let name = next_temporary_name()?;
        match rustix::fs::openat(
            parent,
            &name,
            rustix::fs::OFlags::RDWR
                | rustix::fs::OFlags::CREATE
                | rustix::fs::OFlags::EXCL
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::from_raw_mode(0o600),
        ) {
            Ok(fd) => return Ok((name, fs::File::from(fd))),
            Err(error) if error == rustix::io::Errno::EXIST => {}
            Err(error) => return Err(error.into()),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate a unique temporary filename",
    ))
}

#[cfg(target_os = "linux")]
fn try_reflink_anchored(
    source: &fs::File,
    parent: &OwnedFd,
) -> io::Result<Option<(OsString, fs::File)>> {
    let (name, temporary) = create_anchored_temp(parent)?;
    match rustix::fs::ioctl_ficlone(&temporary, source) {
        Ok(()) => Ok(Some((name, temporary))),
        Err(_) => {
            let _ = unlink_anchored(parent, &name);
            Ok(None)
        }
    }
}

#[cfg(target_os = "macos")]
fn try_reflink_anchored(
    source: &fs::File,
    parent: &OwnedFd,
) -> io::Result<Option<(OsString, fs::File)>> {
    for _ in 0..128 {
        let name = next_temporary_name()?;
        match rustix::fs::fclonefileat(
            source,
            parent,
            &name,
            rustix::fs::CloneFlags::NOFOLLOW | rustix::fs::CloneFlags::NOOWNERCOPY,
        ) {
            Ok(()) => {
                let fd = match rustix::fs::openat(
                    parent,
                    &name,
                    rustix::fs::OFlags::RDONLY
                        | rustix::fs::OFlags::NOFOLLOW
                        | rustix::fs::OFlags::NONBLOCK
                        | rustix::fs::OFlags::CLOEXEC,
                    rustix::fs::Mode::empty(),
                ) {
                    Ok(fd) => fd,
                    Err(error) => {
                        let _ = unlink_anchored(parent, &name);
                        return Err(error.into());
                    }
                };
                return Ok(Some((name, fs::File::from(fd))));
            }
            Err(error) if error == rustix::io::Errno::EXIST => {}
            Err(_) => {
                let _ = unlink_anchored(parent, &name);
                return Ok(None);
            }
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate a unique reflink temporary filename",
    ))
}

#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
fn try_reflink_anchored(
    _source: &fs::File,
    _parent: &OwnedFd,
) -> io::Result<Option<(OsString, fs::File)>> {
    Ok(None)
}

#[cfg(unix)]
fn ensure_name_refers_to_file(
    parent: &OwnedFd,
    name: &OsStr,
    expected: &fs::File,
) -> io::Result<()> {
    let named = rustix::fs::openat(
        parent,
        name,
        rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::NONBLOCK
            | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )?;
    let expected = rustix::fs::fstat(expected)?;
    let named = rustix::fs::fstat(&named)?;
    if expected.st_dev == named.st_dev && expected.st_ino == named.st_ino {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "temporary file changed before publication",
        ))
    }
}

#[cfg(unix)]
fn publish_noreplace(parent: &OwnedFd, temporary: &OsStr, destination: &OsStr) -> io::Result<()> {
    match rustix::fs::renameat_with(
        parent,
        temporary,
        parent,
        destination,
        rustix::fs::RenameFlags::NOREPLACE,
    ) {
        Ok(()) => Ok(()),
        Err(error) if error == rustix::io::Errno::NOSYS || error == rustix::io::Errno::INVAL => {
            // A hard link is also an atomic no-replace publication for a
            // regular file. The temporary name is then removed.
            rustix::fs::linkat(
                parent,
                temporary,
                parent,
                destination,
                rustix::fs::AtFlags::empty(),
            )?;
            rustix::fs::unlinkat(parent, temporary, rustix::fs::AtFlags::empty())?;
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

#[cfg(unix)]
fn unlink_anchored(parent: &OwnedFd, name: &OsStr) -> io::Result<()> {
    rustix::fs::unlinkat(parent, name, rustix::fs::AtFlags::empty()).map_err(Into::into)
}

#[cfg(not(unix))]
fn copy_file_path_fallback(
    src: &Path,
    dst: &Path,
    strategy: CopyStrategy,
    expected_source: &FileSnapshot,
    before_publish: &mut dyn FnMut() -> io::Result<()>,
) -> io::Result<()> {
    if &snapshot_regular_file(src)? != expected_source {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "source changed after planning",
        ));
    }
    let parent = dst.parent().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "path has no parent directory")
    })?;
    create_dir_all_without_symlinks(parent)?;
    ensure_directory_without_symlink(parent)?;

    let try_reflink = match strategy {
        CopyStrategy::SimpleCopy => false,
        CopyStrategy::CowCopy => true,
        CopyStrategy::Auto => false,
    };
    if try_reflink && let Some((tmp_path, source_permissions)) = try_reflink_to_temp(src, parent)? {
        fs::set_permissions(&tmp_path, source_permissions)?;
        fs::File::open(&tmp_path)?.sync_all()?;
        before_publish()?;
        tmp_path.persist_noclobber(dst).map_err(|e| e.error)?;
        return Ok(());
    }

    let (mut source, source_metadata) = open_stable_regular_file(src)?;
    let mut temporary = tempfile::Builder::new()
        .prefix(".waft-copy-")
        .tempfile_in(parent)?;
    io::copy(&mut source, temporary.as_file_mut())?;
    ensure_open_file_unchanged(src, &source, &source_metadata)?;
    temporary
        .as_file()
        .set_permissions(source_metadata.permissions())?;
    temporary.as_file().sync_all()?;
    before_publish()?;
    temporary
        .persist_noclobber(dst)
        .map(|_| ())
        .map_err(|error| error.error)
}

#[cfg(not(unix))]
fn ensure_directory_without_symlink(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_dir() {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "destination parent is not a real directory",
        ))
    }
}

#[cfg(not(unix))]
fn create_dir_all_without_symlinks(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_dir() => return Ok(()),
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "destination directory path contains a non-directory",
            ));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    if let Some(parent) = path.parent()
        && parent != path
        && !parent.as_os_str().is_empty()
    {
        create_dir_all_without_symlinks(parent)?;
    }
    match fs::create_dir(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            ensure_directory_without_symlink(path)
        }
        Err(error) => Err(error),
    }
}

#[cfg(not(unix))]
fn try_reflink_to_temp(
    src: &Path,
    parent: &Path,
) -> io::Result<Option<(tempfile::TempPath, fs::Permissions)>> {
    let (source_file, source_metadata) = open_stable_regular_file(src)?;
    let tmp = tempfile::Builder::new()
        .prefix(".waft-copy-")
        .tempfile_in(parent)?;
    let tmp_path = tmp.into_temp_path();
    fs::remove_file(&tmp_path)?;
    match reflink_copy::reflink(src, &tmp_path) {
        Ok(()) => {
            ensure_open_file_unchanged(src, &source_file, &source_metadata)?;
            Ok(Some((tmp_path, source_metadata.permissions())))
        }
        Err(_) => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use tempfile::TempDir;

    fn write(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    fn fixture(
        tmp: &TempDir,
        rel: &str,
        content: &str,
    ) -> (std::path::PathBuf, std::path::PathBuf) {
        let source_root = tmp.path().join("source");
        let destination_root = tmp.path().join("destination");
        fs::create_dir_all(&source_root).unwrap();
        fs::create_dir_all(&destination_root).unwrap();
        write(&source_root.join(rel), content);
        (
            fs::canonicalize(source_root).unwrap(),
            fs::canonicalize(destination_root).unwrap(),
        )
    }

    fn copy(
        source_root: &Path,
        destination_root: &Path,
        rel: &str,
        strategy: CopyStrategy,
        expected: &DestinationExpectation,
        before_publish: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<()> {
        let expected_source = RealFs.file_snapshot(&source_root.join(rel))?;
        let rel_path = RepoRelPath::from_normalized(rel.to_string());
        RealFs.copy_file(
            CopyFileRequest {
                source_root,
                destination_root,
                rel_path: &rel_path,
                strategy,
                expected_source: &expected_source,
                expected_destination: expected,
            },
            before_publish,
        )
    }

    #[test]
    fn realfs_missing_copy_never_clobbers_path_that_appeared() {
        let tmp = TempDir::new().unwrap();
        let (source_root, destination_root) = fixture(&tmp, "file.env", "source\n");
        let dst = destination_root.join("file.env");
        write(&dst, "appeared\n");

        let error = copy(
            &source_root,
            &destination_root,
            "file.env",
            CopyStrategy::SimpleCopy,
            &DestinationExpectation::Missing,
            &mut || Ok(()),
        )
        .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read_to_string(dst).unwrap(), "appeared\n");
    }

    #[test]
    fn realfs_overwrite_is_always_rejected_before_callback() {
        use std::cell::Cell;

        let tmp = TempDir::new().unwrap();
        let (source_root, destination_root) = fixture(&tmp, "file.env", "new\n");
        let dst = destination_root.join("file.env");
        write(&dst, "old\n");
        let snapshot = RealFs.file_snapshot(&dst).unwrap();
        let callback_called = Cell::new(false);

        let error = copy(
            &source_root,
            &destination_root,
            "file.env",
            CopyStrategy::SimpleCopy,
            &DestinationExpectation::ExistingUntracked(snapshot),
            &mut || {
                callback_called.set(true);
                Ok(())
            },
        )
        .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert!(!callback_called.get());
        assert_eq!(fs::read_to_string(dst).unwrap(), "old\n");
    }

    #[test]
    fn realfs_callback_failure_prevents_publication_and_cleans_temp() {
        use std::cell::Cell;

        let tmp = TempDir::new().unwrap();
        let (source_root, destination_root) = fixture(&tmp, "file.env", "source\n");

        for strategy in [CopyStrategy::SimpleCopy, CopyStrategy::CowCopy] {
            let dst = destination_root.join("file.env");
            let callback_calls = Cell::new(0);
            let mut reject_publish = || {
                callback_calls.set(callback_calls.get() + 1);
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "tracked-state recheck failed",
                ))
            };

            let error = copy(
                &source_root,
                &destination_root,
                "file.env",
                strategy,
                &DestinationExpectation::Missing,
                &mut reject_publish,
            )
            .unwrap_err();

            assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
            assert_eq!(callback_calls.get(), 1);
            assert!(!dst.exists());
            assert!(fs::read_dir(&destination_root).unwrap().all(|entry| {
                !entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".waft-copy-")
            }));
        }
    }

    #[test]
    fn realfs_destination_appearing_in_callback_is_not_clobbered() {
        let tmp = TempDir::new().unwrap();
        let (source_root, destination_root) = fixture(&tmp, "file.env", "new\n");
        let dst = destination_root.join("file.env");
        let mut mutate_destination = || {
            write(&dst, "concurrent\n");
            Ok(())
        };

        let error = copy(
            &source_root,
            &destination_root,
            "file.env",
            CopyStrategy::SimpleCopy,
            &DestinationExpectation::Missing,
            &mut mutate_destination,
        )
        .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read_to_string(dst).unwrap(), "concurrent\n");
    }

    #[cfg(unix)]
    #[test]
    fn realfs_streaming_copy_preserves_unix_permissions() {
        let tmp = TempDir::new().unwrap();
        let (source_root, destination_root) = fixture(&tmp, "bin/script", "#!/bin/sh\n");
        let src = source_root.join("bin/script");
        let dst = destination_root.join("bin/script");
        fs::set_permissions(&src, fs::Permissions::from_mode(0o751)).unwrap();

        copy(
            &source_root,
            &destination_root,
            "bin/script",
            CopyStrategy::SimpleCopy,
            &DestinationExpectation::Missing,
            &mut || Ok(()),
        )
        .unwrap();

        assert_eq!(
            fs::metadata(dst).unwrap().permissions().mode() & 0o777,
            0o751
        );
    }

    #[cfg(unix)]
    #[test]
    fn realfs_file_equality_includes_unix_permissions() {
        let tmp = TempDir::new().unwrap();
        let left = tmp.path().join("left");
        let right = tmp.path().join("right");
        write(&left, "same\n");
        write(&right, "same\n");
        fs::set_permissions(&left, fs::Permissions::from_mode(0o755)).unwrap();
        fs::set_permissions(&right, fs::Permissions::from_mode(0o644)).unwrap();

        assert!(!RealFs.files_equal(&left, &right).unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn realfs_refuses_static_symlink_destination_component() {
        let tmp = TempDir::new().unwrap();
        let (source_root, destination_root) = fixture(&tmp, "nested/file.env", "source\n");
        let outside = tmp.path().join("outside");
        fs::create_dir(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, destination_root.join("nested")).unwrap();

        let error = copy(
            &source_root,
            &destination_root,
            "nested/file.env",
            CopyStrategy::SimpleCopy,
            &DestinationExpectation::Missing,
            &mut || Ok(()),
        )
        .unwrap_err();

        assert!(!outside.join("file.env").exists());
        assert!(matches!(
            error.raw_os_error(),
            Some(code) if code == rustix::io::Errno::LOOP.raw_os_error()
                || code == rustix::io::Errno::NOTDIR.raw_os_error()
        ));
    }

    #[cfg(unix)]
    #[test]
    fn realfs_rejects_ancestor_rename_and_symlink_before_publication() {
        for strategy in [CopyStrategy::SimpleCopy, CopyStrategy::CowCopy] {
            let tmp = TempDir::new().unwrap();
            let (source_root, destination_root) = fixture(&tmp, "a/nested/file.env", "source\n");
            fs::create_dir_all(destination_root.join("a/nested")).unwrap();
            let moved = tmp.path().join("moved-a");
            let attacker = tmp.path().join("attacker");
            fs::create_dir_all(attacker.join("nested")).unwrap();
            let mut replace_ancestor = || {
                fs::rename(destination_root.join("a"), &moved)?;
                std::os::unix::fs::symlink(&attacker, destination_root.join("a"))?;
                Ok(())
            };

            let error = copy(
                &source_root,
                &destination_root,
                "a/nested/file.env",
                strategy,
                &DestinationExpectation::Missing,
                &mut replace_ancestor,
            )
            .unwrap_err();

            assert!(!attacker.join("nested/file.env").exists());
            assert!(!moved.join("nested/file.env").exists());
            assert!(
                error.kind() == io::ErrorKind::Interrupted
                    || matches!(
                        error.raw_os_error(),
                        Some(code) if code == rustix::io::Errno::LOOP.raw_os_error()
                            || code == rustix::io::Errno::NOTDIR.raw_os_error()
                    )
            );
            assert!(fs::read_dir(moved.join("nested")).unwrap().all(|entry| {
                !entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".waft-copy-")
            }));
        }
    }

    #[cfg(unix)]
    #[test]
    fn anchored_source_handle_is_not_redirected_by_ancestor_replacement() {
        let tmp = TempDir::new().unwrap();
        let source_root = tmp.path().join("source");
        let original = source_root.join("a");
        let moved = tmp.path().join("moved-source");
        let attacker = tmp.path().join("attacker-source");
        write(&original.join("file.env"), "original\n");
        write(&attacker.join("file.env"), "attacker\n");

        let root = open_canonical_directory(&fs::canonicalize(&source_root).unwrap()).unwrap();
        let rel = RepoRelPath::from_normalized("a/file.env".to_string());
        let (parent, name) = open_relative_parent(&root, &rel, false).unwrap();
        let fd = rustix::fs::openat(
            &parent,
            &name,
            rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )
        .unwrap();
        let mut opened = fs::File::from(fd);

        fs::rename(&original, &moved).unwrap();
        std::os::unix::fs::symlink(&attacker, &original).unwrap();
        let mut content = String::new();
        opened.read_to_string(&mut content).unwrap();

        assert_eq!(content, "original\n");
    }

    #[cfg(unix)]
    #[test]
    fn realfs_rejects_source_ancestor_replaced_after_planning() {
        use std::cell::Cell;

        let tmp = TempDir::new().unwrap();
        let (source_root, destination_root) = fixture(&tmp, "a/file.env", "planned-source\n");
        let rel = RepoRelPath::from_normalized("a/file.env".to_string());
        let expected_source = RealFs
            .file_snapshot(&source_root.join("a/file.env"))
            .unwrap();
        let moved = tmp.path().join("moved-source-a");
        fs::rename(source_root.join("a"), &moved).unwrap();
        write(&source_root.join("a/file.env"), "replacement\n");
        let callback_called = Cell::new(false);

        let error = RealFs
            .copy_file(
                CopyFileRequest {
                    source_root: &source_root,
                    destination_root: &destination_root,
                    rel_path: &rel,
                    strategy: CopyStrategy::SimpleCopy,
                    expected_source: &expected_source,
                    expected_destination: &DestinationExpectation::Missing,
                },
                &mut || {
                    callback_called.set(true);
                    Ok(())
                },
            )
            .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
        assert!(!callback_called.get());
        assert!(!destination_root.join("a/file.env").exists());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn realfs_rejects_source_replaced_by_fifo_without_blocking() {
        use std::cell::Cell;

        let tmp = TempDir::new().unwrap();
        let (source_root, destination_root) = fixture(&tmp, "a/file.env", "planned-source\n");
        let rel = RepoRelPath::from_normalized("a/file.env".to_string());
        let expected_source = RealFs
            .file_snapshot(&source_root.join("a/file.env"))
            .unwrap();
        fs::remove_file(source_root.join("a/file.env")).unwrap();
        let root = open_canonical_directory(&source_root).unwrap();
        let (parent, name) = open_relative_parent(&root, &rel, false).unwrap();
        rustix::fs::mkfifoat(&parent, &name, rustix::fs::Mode::from_raw_mode(0o600)).unwrap();
        let callback_called = Cell::new(false);

        let error = RealFs
            .copy_file(
                CopyFileRequest {
                    source_root: &source_root,
                    destination_root: &destination_root,
                    rel_path: &rel,
                    strategy: CopyStrategy::SimpleCopy,
                    expected_source: &expected_source,
                    expected_destination: &DestinationExpectation::Missing,
                },
                &mut || {
                    callback_called.set(true);
                    Ok(())
                },
            )
            .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert!(!callback_called.get());
        assert!(!destination_root.join("a/file.env").exists());
    }
}
