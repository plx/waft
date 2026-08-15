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
use crate::model::{DestinationExpectation, FileSnapshot, PublishOutcome};
use crate::path::RepoRelPath;

/// How a source file relates to an existing destination file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileComparison {
    /// Byte-identical content and identical relevant permission bits.
    Equal,
    /// Byte-identical content, differing permission bits.
    PermissionsDiffer,
    /// Differing content (length or bytes).
    ContentDiffers,
}

/// Whether this platform can act on a destination that already exists.
///
/// Replacing or repairing an existing destination needs the anchored
/// publication path: an `O_NOFOLLOW` re-open of the planned inode, an atomic
/// name exchange (or an unlink of a proven inode), and `fchmod` on the verified
/// descriptor. Only Unix offers those. Planning consults this so a plan, its
/// `--dry-run` rendering, the executed run, and the exit status all describe
/// the same thing instead of advertising an action the platform cannot perform.
pub(crate) fn overwrite_supported() -> bool {
    cfg!(unix)
}

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
    /// Expected destination state. An existing destination is only touched
    /// when it still matches the snapshot recorded here.
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
    ///
    /// Content equality and permission equality are reported separately so
    /// callers can distinguish "needs a rewrite" from "needs a `chmod`".
    fn compare_files(&self, left: &Path, right: &Path) -> io::Result<FileComparison> {
        if self.read(left)? == self.read(right)? {
            Ok(FileComparison::Equal)
        } else {
            Ok(FileComparison::ContentDiffers)
        }
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
    /// independently mutable absolute pathnames.
    ///
    /// A destination expected to be [`DestinationExpectation::Missing`] is
    /// published with no-clobber semantics. A destination expected to exist is
    /// re-opened and must still match its planning snapshot exactly; only then
    /// is it replaced by atomic exchange or repaired in place.
    ///
    /// `before_publish` runs after content preparation and immediately before
    /// final namespace validation and publication.
    fn copy_file(
        &self,
        request: CopyFileRequest<'_>,
        before_publish: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<PublishOutcome>;
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

    fn compare_files(&self, left: &Path, right: &Path) -> io::Result<FileComparison> {
        let (mut left_file, left_metadata) = open_stable_regular_file(left)?;
        let (mut right_file, right_metadata) = open_stable_regular_file(right)?;

        let permissions_equal =
            permission_signature(&left_metadata) == permission_signature(&right_metadata);
        if left_metadata.len() != right_metadata.len() {
            return Ok(FileComparison::ContentDiffers);
        }

        let mut left_buffer = [0u8; 64 * 1024];
        let mut right_buffer = [0u8; 64 * 1024];
        loop {
            let left_read = left_file.read(&mut left_buffer)?;
            let right_read = right_file.read(&mut right_buffer)?;
            if left_read != right_read || left_buffer[..left_read] != right_buffer[..right_read] {
                return Ok(FileComparison::ContentDiffers);
            }
            if left_read == 0 {
                break;
            }
        }

        ensure_open_file_unchanged(left, &left_file, &left_metadata)?;
        ensure_open_file_unchanged(right, &right_file, &right_metadata)?;
        Ok(if permissions_equal {
            FileComparison::Equal
        } else {
            FileComparison::PermissionsDiffer
        })
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
    ) -> io::Result<PublishOutcome> {
        #[cfg(unix)]
        {
            copy_file_anchored_unix(request, before_publish)
        }

        #[cfg(not(unix))]
        {
            if request.expected_destination.existing_snapshot().is_some() {
                // Windows has no `renameat2`/`renameatx_np` equivalent that can
                // exchange two names atomically while proving the outgoing
                // inode is the one that was planned against.
                return Err(replacement_unsupported_error(request.rel_path));
            }
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

#[cfg(not(unix))]
fn replacement_unsupported_error(rel_path: &RepoRelPath) -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        format!(
            "{rel_path}: replacing an existing destination is not supported on this platform; review and remove the destination file, then rerun"
        ),
    )
}

#[cfg(unix)]
const DIRECTORY_FLAGS: rustix::fs::OFlags = rustix::fs::OFlags::RDONLY
    .union(rustix::fs::OFlags::DIRECTORY)
    .union(rustix::fs::OFlags::NOFOLLOW)
    .union(rustix::fs::OFlags::CLOEXEC);

/// Removes an anchored temporary name unless it has been disarmed.
///
/// Cleanup runs from `Drop`, so an unwinding panic between temp creation and
/// publication cannot leave a `.waft-copy-*` file behind.
#[cfg(unix)]
struct AnchoredTempGuard<'a> {
    parent: &'a OwnedFd,
    name: OsString,
    armed: bool,
}

#[cfg(unix)]
impl<'a> AnchoredTempGuard<'a> {
    fn new(parent: &'a OwnedFd, name: OsString) -> Self {
        Self {
            parent,
            name,
            armed: true,
        }
    }

    fn name(&self) -> &OsStr {
        &self.name
    }

    /// Stop tracking the temporary name; the caller has consumed it.
    fn disarm(&mut self) {
        self.armed = false;
    }

    /// Remove the temporary name now, reporting whether it worked.
    ///
    /// Tracking stops either way, so `Drop` can never unlink this name twice —
    /// and after a successful exchange it must not: the name then holds the
    /// *previous* destination, and a retry that happened to succeed would
    /// delete the very file the caller is about to name in an error. A failure
    /// is handed back rather than retried, because only the caller knows what a
    /// leftover means at that point.
    fn remove_now(&mut self) -> io::Result<()> {
        let removed = if temporary_removal_forced_failure() {
            Err(rustix::io::Errno::IO.into())
        } else {
            unlink_anchored(self.parent, &self.name)
        };
        self.armed = false;
        removed
    }
}

#[cfg(unix)]
impl Drop for AnchoredTempGuard<'_> {
    fn drop(&mut self) {
        if self.armed {
            let _ = unlink_anchored(self.parent, &self.name);
        }
    }
}

#[cfg(unix)]
fn copy_file_anchored_unix(
    request: CopyFileRequest<'_>,
    before_publish: &mut dyn FnMut() -> io::Result<()>,
) -> io::Result<PublishOutcome> {
    let rel_path = request.rel_path;
    let expected_destination = request.expected_destination;
    let source_root = open_canonical_directory(request.source_root)?;
    let destination_root = open_canonical_directory(request.destination_root)?;

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
    if &actual_source != request.expected_source {
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

    // Every operation below goes through the anchored descriptors; this
    // pathname exists only so an error can name a file the caller can find.
    let destination_path = rel_path.to_path(request.destination_root);
    let destination_directory = destination_path
        .parent()
        .unwrap_or(request.destination_root)
        .to_path_buf();

    // An expected-existing destination is pinned to a descriptor and matched
    // against its planning snapshot before anything is prepared. Every later
    // step compares against this descriptor's identity, never a pathname.
    let expected_destination_snapshot = expected_destination.existing_snapshot();
    let mut verified_destination = match expected_destination_snapshot {
        Some(expected) => Some(open_verified_destination(
            &destination_parent,
            &destination_name,
            expected,
        )?),
        None => None,
    };

    if matches!(
        expected_destination,
        DestinationExpectation::RepairPermissions(_)
    ) {
        let expected = expected_destination_snapshot
            .expect("a repair expectation always carries a destination snapshot");
        let destination = verified_destination
            .as_mut()
            .expect("a repair expectation always carries a destination snapshot");
        // A repair changes only the mode, so it is only ever correct when the
        // destination already holds the source's bytes. Planning proved that
        // with a full byte comparison, but the source and destination
        // snapshots are separate reads: re-establish the premise here from the
        // pinned data itself. Both snapshots have just been matched against
        // live descriptors, so agreeing with each other means the two files
        // agree right now.
        if !expected.content_matches(request.expected_source) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "destination content no longer matches the source; refusing to repair permissions",
            ));
        }
        ensure_open_handle_unchanged(&source, &source_metadata)?;
        before_publish()?;
        ensure_same_relative_parent(&destination_root, rel_path, &destination_parent)?;
        ensure_name_refers_to_file(&destination_parent, &destination_name, destination)?;
        ensure_open_snapshot_matches(destination, expected)?;
        before_permission_repair();
        destination.set_permissions(source_metadata.permissions())?;
        // A writer can rewrite this inode in place between the proof above and
        // the `fchmod`, which would leave waft carrying the source's mode onto
        // content that is no longer the source's — and calling it a repair.
        // Re-read the descriptor and require the pinned bytes once more. The
        // mode has legitimately just changed to the source's, so only content
        // is compared.
        //
        // This does not close the window: a writer can always land after
        // whatever check is last, and a repair that reports success may be
        // overwritten a microsecond later. What it bounds is what waft itself
        // does and claims — waft never alters the destination's content, and
        // never reports a repair it did not re-verify.
        if !open_content_matches(destination, expected) {
            return Err(revert_permission_repair(destination, expected));
        }
        // The directory entry is untouched, so only the inode needs syncing.
        rustix::fs::fsync(&*destination)?;
        return Ok(PublishOutcome::PermissionsRepaired);
    }

    let try_reflink = match request.strategy {
        CopyStrategy::SimpleCopy => false,
        CopyStrategy::CowCopy => true,
        CopyStrategy::Auto => cfg!(target_os = "macos"),
    };

    let prepared = if try_reflink {
        try_reflink_anchored(&source, &destination_parent)?
    } else {
        None
    };
    let (temporary_name, mut temporary, needs_stream) = match prepared {
        Some((name, file)) => (name, file, false),
        None => {
            let (name, file) = create_anchored_temp(&destination_parent)?;
            (name, file, true)
        }
    };
    // Everything from here on is guarded: any early return, `?`, or unwinding
    // panic removes the temporary name.
    let mut temporary_guard = AnchoredTempGuard::new(&destination_parent, temporary_name);
    if needs_stream {
        source.seek(SeekFrom::Start(0))?;
        io::copy(&mut source, &mut temporary)?;
    }

    ensure_open_handle_unchanged(&source, &source_metadata)?;
    temporary.set_permissions(source_metadata.permissions())?;
    temporary.sync_all()?;

    // This callback performs the final tracked-index check under Git's
    // cooperative index lock. Revalidate the parent capability after it
    // returns, then publish immediately.
    before_publish()?;
    ensure_same_relative_parent(&destination_root, rel_path, &destination_parent)?;
    ensure_name_refers_to_file(&destination_parent, temporary_guard.name(), &temporary)?;

    let outcome = match verified_destination.as_mut() {
        None => {
            publish_noreplace(
                &destination_parent,
                temporary_guard.name(),
                &destination_name,
            )?;
            temporary_guard.disarm();
            PublishOutcome::Created
        }
        Some(destination) => {
            let expected = expected_destination_snapshot
                .expect("an existing-destination expectation always carries a snapshot");
            replace_verified_destination(
                &destination_parent,
                &destination_directory,
                &mut temporary_guard,
                &destination_name,
                destination,
                expected,
            )?;
            PublishOutcome::Replaced
        }
    };
    rustix::fs::fsync(&destination_parent)?;
    Ok(outcome)
}

/// Open the destination by name under an already-authorized parent handle and
/// require it to still match the state recorded while planning.
#[cfg(unix)]
fn open_verified_destination(
    parent: &OwnedFd,
    name: &OsStr,
    expected: &FileSnapshot,
) -> io::Result<fs::File> {
    let fd = rustix::fs::openat(
        parent,
        name,
        rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::NONBLOCK
            | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )?;
    let mut destination = fs::File::from(fd);
    ensure_open_snapshot_matches(&mut destination, expected)?;
    Ok(destination)
}

/// Require an open regular file to still hold exactly the bytes and mode
/// recorded in `expected`.
///
/// Identity alone is not enough: a writer can truncate and rewrite the same
/// inode in place, so the content fingerprint is re-read from the descriptor.
#[cfg(unix)]
fn ensure_open_snapshot_matches(file: &mut fs::File, expected: &FileSnapshot) -> io::Result<()> {
    let metadata = file.metadata()?;
    if !metadata.file_type().is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "destination is not a regular file",
        ));
    }
    let actual = snapshot_open_regular_file(file, &metadata)?;
    if actual != *expected {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "destination changed after planning; refusing to replace it",
        ));
    }
    Ok(())
}

/// Whether an open regular file still holds exactly the bytes recorded in
/// `expected`, ignoring its mode.
///
/// Used only after waft has already changed something, to decide whether that
/// change can be reported as correct. Anything that prevents proving it — an
/// unreadable descriptor, a file that is no longer regular, a change observed
/// mid-read — is therefore a mismatch rather than an error to propagate.
#[cfg(unix)]
fn open_content_matches(file: &mut fs::File, expected: &FileSnapshot) -> bool {
    let Ok(metadata) = file.metadata() else {
        return false;
    };
    if !metadata.file_type().is_file() {
        return false;
    }
    match snapshot_open_regular_file(file, &metadata) {
        Ok(actual) => actual.content_matches(expected),
        Err(_) => false,
    }
}

/// Put the destination's mode back after a repair whose premise turned out to
/// be false, and describe what happened.
///
/// The restore is best effort: it is the only way back to the state the caller
/// was promised, but a descriptor that cannot be `fchmod`-ed leaves the file
/// carrying the source's mode over content this run never verified. That is the
/// more specific problem, so it is the one reported.
#[cfg(unix)]
fn revert_permission_repair(file: &fs::File, expected: &FileSnapshot) -> io::Error {
    use std::os::unix::fs::PermissionsExt;

    // `expected.permissions` is a full `st_mode`; only the permission bits are
    // meaningful to `fchmod`.
    match file.set_permissions(fs::Permissions::from_mode(expected.permissions & 0o7777)) {
        Ok(()) => io::Error::new(
            io::ErrorKind::Interrupted,
            "destination changed during publication; its content was not touched and its \
             permissions were left as they were found",
        ),
        Err(restore_error) => io::Error::other(format!(
            "destination changed during publication and its original permissions could not be \
             restored ({restore_error}): its content was not touched, but it now carries the \
             source's mode over content this run could not verify"
        )),
    }
}

/// Replace `destination_name` with the prepared temporary, proving that the
/// pathname still resolves to `verified` — with exactly the planned bytes and
/// mode — at the moment of the swap.
///
/// The exchange makes the destination name point at the new content and the
/// temporary name point at the outgoing inode in a single atomic step. If the
/// file that was swapped out is not the one that was planned against, the
/// exchange is undone and this file is reported as a per-file failure rather
/// than a silent clobber.
///
/// `parent_directory` is the pathname of `parent` at planning time, used only
/// to name files a caller may have to deal with by hand.
#[cfg(unix)]
fn replace_verified_destination(
    parent: &OwnedFd,
    parent_directory: &Path,
    temporary_guard: &mut AnchoredTempGuard<'_>,
    destination_name: &OsStr,
    verified: &mut fs::File,
    expected: &FileSnapshot,
) -> io::Result<()> {
    ensure_name_refers_to_file(parent, destination_name, verified)?;
    ensure_open_snapshot_matches(verified, expected)?;
    let verified_stat = rustix::fs::fstat(&*verified)?;

    after_destination_verified();

    match exchange_anchored(parent, temporary_guard.name(), destination_name) {
        Ok(()) => {
            let swapped_out_as_planned =
                swapped_out_matches(parent, temporary_guard.name(), &verified_stat)
                    .unwrap_or(false)
                    && ensure_open_snapshot_matches(verified, expected).is_ok();
            if swapped_out_as_planned {
                // The temporary name now holds the outgoing inode. Removing it
                // is the last step, and its failure is not cosmetic: the
                // replacement itself stands, but the file it replaced is still
                // on disk under a name nobody expects, possibly holding the
                // secrets this run was moving around. Report the file as failed
                // and say exactly what has to be deleted.
                let leftover = parent_directory.join(temporary_guard.name());
                return temporary_guard.remove_now().map_err(|error| {
                    io::Error::new(
                        error.kind(),
                        format!(
                            "{} was replaced with the planned content, but the file it replaced \
                             could not be removed ({error}): it is still on disk as {}, still \
                             holding the previous content; delete it by hand",
                            Path::new(destination_name).display(),
                            leftover.display(),
                        ),
                    )
                });
            }
            // Another writer changed the destination between the identity
            // proof and the exchange. Put both names back and fail this file.
            before_exchange_back();
            match exchange_anchored(parent, temporary_guard.name(), destination_name) {
                Ok(()) => Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "destination changed during publication; it was left untouched",
                )),
                Err(undo_error) => {
                    // The undo failed, so the destination name may still hold
                    // the newly prepared content and the temporary name holds
                    // a file this process never verified. Deleting that file
                    // would destroy another writer's data, so the guard is
                    // released and the situation is described instead.
                    let stranded = temporary_guard.name().to_os_string();
                    temporary_guard.disarm();
                    Err(io::Error::other(format!(
                        "destination changed during publication and the swap could not be undone \
                         ({undo_error}): {} may now hold newly published content and the file \
                         that was there was left as {}; nothing was deleted",
                        Path::new(destination_name).display(),
                        Path::new(&stranded).display(),
                    )))
                }
            }
        }
        Err(error) if rename_flag_unsupported(error) => {
            // Without an atomic exchange the best available sequence is
            // "unlink the inode we proved, then publish with no-clobber". If
            // anything recreates the name in that window, NOREPLACE fails and
            // the file is reported as failed; it is never clobbered.
            ensure_name_refers_to_file(parent, destination_name, verified)?;
            ensure_open_snapshot_matches(verified, expected)?;
            unlink_anchored(parent, destination_name)?;
            after_destination_removed();
            match publish_noreplace(parent, temporary_guard.name(), destination_name) {
                Ok(()) => {
                    temporary_guard.disarm();
                    Ok(())
                }
                Err(publish_error) => {
                    // The verified destination is already unlinked and this
                    // filesystem offers no way to put it back. The prepared
                    // replacement is the only copy of this file's content left
                    // on disk, so keep it and name it rather than letting the
                    // guard delete it on the way out.
                    let stranded = temporary_guard.name().to_os_string();
                    temporary_guard.disarm();
                    Err(io::Error::new(
                        publish_error.kind(),
                        format!(
                            "could not publish over the removed destination ({publish_error}): \
                             this filesystem cannot exchange two names atomically, so {} was \
                             removed and the prepared replacement was left as {}",
                            Path::new(destination_name).display(),
                            Path::new(&stranded).display(),
                        ),
                    ))
                }
            }
        }
        Err(error) => Err(error.into()),
    }
}

/// Atomically swap two names under `parent`.
#[cfg(unix)]
fn exchange_anchored(parent: &OwnedFd, from: &OsStr, to: &OsStr) -> Result<(), rustix::io::Errno> {
    if exchange_forced_unsupported() {
        return Err(rustix::io::Errno::NOTSUP);
    }
    rustix::fs::renameat_with(parent, from, parent, to, rustix::fs::RenameFlags::EXCHANGE)
}

/// After an exchange, check that the temporary name holds the inode that was
/// previously verified at the destination name.
#[cfg(unix)]
fn swapped_out_matches(
    parent: &OwnedFd,
    temporary_name: &OsStr,
    verified: &rustix::fs::Stat,
) -> io::Result<bool> {
    let swapped_out = rustix::fs::openat(
        parent,
        temporary_name,
        rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::NONBLOCK
            | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )?;
    let swapped_out = rustix::fs::fstat(&swapped_out)?;
    Ok(swapped_out.st_dev == verified.st_dev && swapped_out.st_ino == verified.st_ino)
}

/// Errnos that mean "this filesystem cannot exchange two names atomically".
#[cfg(unix)]
fn rename_flag_unsupported(error: rustix::io::Errno) -> bool {
    error == rustix::io::Errno::NOTSUP
        || error == rustix::io::Errno::OPNOTSUPP
        || error == rustix::io::Errno::INVAL
        || error == rustix::io::Errno::NOSYS
}

/// Test-only seams inside the publication window.
///
/// The destructive recovery branches of [`replace_verified_destination`] — the
/// undo of a failed exchange, and the unlink-then-publish fallback used where
/// exchange is unsupported — cannot be provoked from an ordinary test volume:
/// the guards that run before the swap reject any change made from
/// `before_publish`, and every filesystem CI runs on supports exchange. Two
/// more branches are just as unreachable by ordinary means: the instant between
/// verifying a repair's destination and `fchmod`-ing it, which a real writer
/// would have to hit exactly, and an unlink of the swapped-out file that fails
/// after the exchange has already succeeded. These hooks let unit tests reach
/// every one of those branches so they are covered rather than merely argued
/// about.
#[cfg(all(test, unix))]
mod publish_hooks {
    use std::cell::RefCell;

    /// Callbacks and forced failures for one test.
    #[derive(Default)]
    pub(super) struct Hooks {
        /// Runs after the destination identity proof and before the swap.
        pub(super) after_destination_verified: Option<Box<dyn FnMut()>>,
        /// Runs after a mismatch is detected and before the undo swap.
        pub(super) before_exchange_back: Option<Box<dyn FnMut()>>,
        /// Runs in the exchange-less fallback, after the verified destination
        /// has been unlinked and before the replacement is published.
        pub(super) after_destination_removed: Option<Box<dyn FnMut()>>,
        /// Runs after a repair's destination is verified and before its mode is
        /// changed.
        pub(super) before_permission_repair: Option<Box<dyn FnMut()>>,
        /// Makes every exchange report the filesystem as unable to swap names.
        pub(super) exchange_unsupported: bool,
        /// Makes removing a temporary name report an I/O error without
        /// unlinking anything, standing in for a transient failure on the
        /// last step of a replacement.
        pub(super) temporary_removal_fails: bool,
    }

    thread_local! {
        static HOOKS: RefCell<Hooks> = RefCell::new(Hooks::default());
    }

    /// Installs `hooks` for the current thread until the returned guard drops.
    pub(super) fn install(hooks: Hooks) -> HookGuard {
        HOOKS.with(|cell| *cell.borrow_mut() = hooks);
        HookGuard
    }

    pub(super) struct HookGuard;

    impl Drop for HookGuard {
        fn drop(&mut self) {
            HOOKS.with(|cell| *cell.borrow_mut() = Hooks::default());
        }
    }

    pub(super) fn after_destination_verified() {
        run(|hooks| hooks.after_destination_verified.take());
    }

    pub(super) fn before_exchange_back() {
        run(|hooks| hooks.before_exchange_back.take());
    }

    pub(super) fn after_destination_removed() {
        run(|hooks| hooks.after_destination_removed.take());
    }

    pub(super) fn before_permission_repair() {
        run(|hooks| hooks.before_permission_repair.take());
    }

    pub(super) fn exchange_forced_unsupported() -> bool {
        HOOKS.with(|cell| cell.borrow().exchange_unsupported)
    }

    pub(super) fn temporary_removal_forced_failure() -> bool {
        HOOKS.with(|cell| cell.borrow().temporary_removal_fails)
    }

    /// Takes a callback out of the hook set before running it, so a hook can
    /// itself re-enter the publication path without recursing or deadlocking
    /// on the `RefCell`.
    fn run(select: impl FnOnce(&mut Hooks) -> Option<Box<dyn FnMut()>>) {
        let callback = HOOKS.with(|cell| select(&mut cell.borrow_mut()));
        if let Some(mut callback) = callback {
            callback();
        }
    }
}

#[cfg(all(test, unix))]
use publish_hooks::{
    after_destination_removed, after_destination_verified, before_exchange_back,
    before_permission_repair, exchange_forced_unsupported, temporary_removal_forced_failure,
};

#[cfg(all(not(test), unix))]
fn after_destination_verified() {}

#[cfg(all(not(test), unix))]
fn before_exchange_back() {}

#[cfg(all(not(test), unix))]
fn after_destination_removed() {}

#[cfg(all(not(test), unix))]
fn before_permission_repair() {}

#[cfg(all(not(test), unix))]
fn exchange_forced_unsupported() -> bool {
    false
}

#[cfg(all(not(test), unix))]
fn temporary_removal_forced_failure() -> bool {
    false
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
        // NOSYS/INVAL mean the kernel or filesystem does not implement the
        // flag; NOTSUP/OPNOTSUPP are what macOS returns for SMB, NFS, and
        // exFAT destinations. All four are "no RENAME_NOREPLACE here", not
        // "this rename is wrong".
        Err(error) if rename_flag_unsupported(error) => {
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
) -> io::Result<PublishOutcome> {
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
        return Ok(PublishOutcome::Created);
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
        .map(|_| PublishOutcome::Created)
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
    ) -> io::Result<PublishOutcome> {
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

    #[cfg(unix)]
    fn existing_snapshot(path: &Path) -> FileSnapshot {
        RealFs.file_snapshot(path).unwrap()
    }

    #[cfg(unix)]
    #[test]
    fn realfs_overwrite_replaces_a_verified_differing_destination() {
        use std::os::unix::fs::MetadataExt;

        for strategy in [CopyStrategy::SimpleCopy, CopyStrategy::CowCopy] {
            let tmp = TempDir::new().unwrap();
            let (source_root, destination_root) = fixture(&tmp, "file.env", "new\n");
            let dst = destination_root.join("file.env");
            write(&dst, "old\n");
            let before = fs::metadata(&dst).unwrap().ino();

            let outcome = copy(
                &source_root,
                &destination_root,
                "file.env",
                strategy,
                &DestinationExpectation::ReplaceExisting(existing_snapshot(&dst)),
                &mut || Ok(()),
            )
            .unwrap();

            assert_eq!(outcome, PublishOutcome::Replaced);
            assert_eq!(fs::read_to_string(&dst).unwrap(), "new\n");
            assert_ne!(
                fs::metadata(&dst).unwrap().ino(),
                before,
                "a replacement publishes a new inode"
            );
            assert!(no_temporaries_left(&destination_root));
        }
    }

    #[cfg(unix)]
    #[test]
    fn realfs_overwrite_repairs_permissions_without_rewriting_content() {
        use std::os::unix::fs::MetadataExt;

        let tmp = TempDir::new().unwrap();
        let (source_root, destination_root) = fixture(&tmp, "file.env", "same\n");
        let src = source_root.join("file.env");
        let dst = destination_root.join("file.env");
        write(&dst, "same\n");
        fs::set_permissions(&src, fs::Permissions::from_mode(0o644)).unwrap();
        fs::set_permissions(&dst, fs::Permissions::from_mode(0o600)).unwrap();
        let before = fs::metadata(&dst).unwrap().ino();

        let outcome = copy(
            &source_root,
            &destination_root,
            "file.env",
            CopyStrategy::SimpleCopy,
            &DestinationExpectation::RepairPermissions(existing_snapshot(&dst)),
            &mut || Ok(()),
        )
        .unwrap();

        assert_eq!(outcome, PublishOutcome::PermissionsRepaired);
        assert_eq!(
            fs::metadata(&dst).unwrap().permissions().mode() & 0o777,
            0o644
        );
        assert_eq!(fs::read_to_string(&dst).unwrap(), "same\n");
        assert_eq!(fs::metadata(&dst).unwrap().ino(), before);
        assert!(no_temporaries_left(&destination_root));
    }

    #[cfg(unix)]
    #[test]
    fn realfs_overwrite_refuses_a_destination_that_changed_after_planning() {
        let tmp = TempDir::new().unwrap();
        let (source_root, destination_root) = fixture(&tmp, "file.env", "new\n");
        let dst = destination_root.join("file.env");
        write(&dst, "old\n");
        let planned = existing_snapshot(&dst);
        // The destination is edited after the plan was built.
        write(&dst, "edited by someone else\n");

        let error = copy(
            &source_root,
            &destination_root,
            "file.env",
            CopyStrategy::SimpleCopy,
            &DestinationExpectation::ReplaceExisting(planned),
            &mut || Ok(()),
        )
        .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
        assert_eq!(
            fs::read_to_string(&dst).unwrap(),
            "edited by someone else\n"
        );
        assert!(no_temporaries_left(&destination_root));
    }

    /// The interesting window: the destination is rewritten *in place*, keeping
    /// its inode, after verification and while the publication is in flight.
    #[cfg(unix)]
    #[test]
    fn realfs_overwrite_refuses_a_destination_rewritten_during_publication() {
        for strategy in [CopyStrategy::SimpleCopy, CopyStrategy::CowCopy] {
            let tmp = TempDir::new().unwrap();
            let (source_root, destination_root) = fixture(&tmp, "file.env", "new\n");
            let dst = destination_root.join("file.env");
            write(&dst, "old\n");
            let planned = existing_snapshot(&dst);
            let mut rewrite_destination_in_place = || {
                // `write` truncates and rewrites the same inode, so an
                // identity check alone would not notice this.
                fs::write(&dst, "concurrent edit\n")?;
                Ok(())
            };

            let error = copy(
                &source_root,
                &destination_root,
                "file.env",
                strategy,
                &DestinationExpectation::ReplaceExisting(planned),
                &mut rewrite_destination_in_place,
            )
            .unwrap_err();

            assert_eq!(error.kind(), io::ErrorKind::Interrupted);
            assert_eq!(fs::read_to_string(&dst).unwrap(), "concurrent edit\n");
            assert!(no_temporaries_left(&destination_root));
        }
    }

    #[cfg(unix)]
    #[test]
    fn realfs_permission_repair_refuses_a_destination_rewritten_during_publication() {
        let tmp = TempDir::new().unwrap();
        let (source_root, destination_root) = fixture(&tmp, "file.env", "same\n");
        let src = source_root.join("file.env");
        let dst = destination_root.join("file.env");
        write(&dst, "same\n");
        fs::set_permissions(&src, fs::Permissions::from_mode(0o644)).unwrap();
        fs::set_permissions(&dst, fs::Permissions::from_mode(0o600)).unwrap();
        let planned = existing_snapshot(&dst);
        let mut rewrite_destination_in_place = || {
            fs::write(&dst, "concurrent edit\n")?;
            Ok(())
        };

        let error = copy(
            &source_root,
            &destination_root,
            "file.env",
            CopyStrategy::SimpleCopy,
            &DestinationExpectation::RepairPermissions(planned),
            &mut rewrite_destination_in_place,
        )
        .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
        assert_eq!(fs::read_to_string(&dst).unwrap(), "concurrent edit\n");
        assert_eq!(
            fs::metadata(&dst).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    /// A repair may only ever change the mode, so it must refuse to run when
    /// the bytes it was told are equal are not: the byte comparison that
    /// classified this file and the snapshot pinned into the plan are separate
    /// reads, and a writer can land between them.
    #[cfg(unix)]
    #[test]
    fn realfs_permission_repair_refuses_a_destination_whose_content_is_not_the_source() {
        let tmp = TempDir::new().unwrap();
        let (source_root, destination_root) = fixture(&tmp, "file.env", "source\n");
        let src = source_root.join("file.env");
        let dst = destination_root.join("file.env");
        write(&dst, "not the source at all\n");
        fs::set_permissions(&src, fs::Permissions::from_mode(0o644)).unwrap();
        fs::set_permissions(&dst, fs::Permissions::from_mode(0o600)).unwrap();

        let error = copy(
            &source_root,
            &destination_root,
            "file.env",
            CopyStrategy::SimpleCopy,
            // A repair expectation that the destination still matches exactly,
            // but whose premise — equal content — does not hold.
            &DestinationExpectation::RepairPermissions(existing_snapshot(&dst)),
            &mut || Ok(()),
        )
        .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
        assert_eq!(fs::read_to_string(&dst).unwrap(), "not the source at all\n");
        assert_eq!(
            fs::metadata(&dst).unwrap().permissions().mode() & 0o777,
            0o600,
            "a refused repair must not change the mode either"
        );
    }

    /// The last window a repair has: the destination is rewritten in place
    /// after it has been verified and before the `fchmod` lands. Waft cannot
    /// prevent that — a writer can always land after whatever check is last —
    /// but it must not carry the source's mode onto content it never proved,
    /// and it must not call that a repair.
    #[cfg(unix)]
    #[test]
    fn realfs_permission_repair_reverts_when_the_destination_changes_before_the_chmod() {
        use std::io::Write as _;

        let tmp = TempDir::new().unwrap();
        let (source_root, destination_root) = fixture(&tmp, "file.env", "same\n");
        let src = source_root.join("file.env");
        let dst = destination_root.join("file.env");
        write(&dst, "same\n");
        fs::set_permissions(&src, fs::Permissions::from_mode(0o644)).unwrap();
        fs::set_permissions(&dst, fs::Permissions::from_mode(0o600)).unwrap();
        let planned = existing_snapshot(&dst);

        let rewritten = dst.clone();
        let _hooks = publish_hooks::install(publish_hooks::Hooks {
            before_permission_repair: Some(Box::new(move || {
                // Another writer overwrites the same inode in place with the
                // same number of bytes, leaving the mode alone: identity,
                // length, and mode all still match the snapshot, so only the
                // content fingerprint can tell that this is not the file the
                // repair was planned for.
                let mut file = fs::OpenOptions::new().write(true).open(&rewritten).unwrap();
                file.write_all(b"diff\n").unwrap();
            })),
            ..publish_hooks::Hooks::default()
        });

        let error = copy(
            &source_root,
            &destination_root,
            "file.env",
            CopyStrategy::SimpleCopy,
            &DestinationExpectation::RepairPermissions(planned),
            &mut || Ok(()),
        )
        .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
        assert_eq!(
            fs::read_to_string(&dst).unwrap(),
            "diff\n",
            "a repair must never write the destination's content"
        );
        assert_eq!(
            fs::metadata(&dst).unwrap().permissions().mode() & 0o777,
            0o600,
            "the mode must be put back to the one that was verified"
        );
        assert!(no_temporaries_left(&destination_root));
    }

    /// Swap the destination name onto a different inode, imitating another
    /// writer publishing its own file there.
    #[cfg(unix)]
    fn substitute_destination(destination_root: &Path, rel: &str, content: &str) {
        let interloper = destination_root.join("interloper");
        write(&interloper, content);
        fs::rename(&interloper, destination_root.join(rel)).unwrap();
    }

    #[cfg(unix)]
    fn temporaries(directory: &Path) -> Vec<std::path::PathBuf> {
        let mut found: Vec<_> = fs::read_dir(directory)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with(".waft-copy-")
            })
            .collect();
        found.sort();
        found
    }

    /// The window the identity proof cannot cover: the destination is
    /// substituted after every guard has passed and before the swap.
    #[cfg(unix)]
    #[test]
    fn realfs_overwrite_undoes_the_swap_when_the_destination_changed_at_the_last_instant() {
        for strategy in [CopyStrategy::SimpleCopy, CopyStrategy::CowCopy] {
            let tmp = TempDir::new().unwrap();
            let (source_root, destination_root) = fixture(&tmp, "file.env", "new\n");
            let dst = destination_root.join("file.env");
            write(&dst, "old\n");
            let planned = existing_snapshot(&dst);

            let root = destination_root.clone();
            let _hooks = publish_hooks::install(publish_hooks::Hooks {
                after_destination_verified: Some(Box::new(move || {
                    substitute_destination(&root, "file.env", "someone else's file\n");
                })),
                ..publish_hooks::Hooks::default()
            });

            let error = copy(
                &source_root,
                &destination_root,
                "file.env",
                strategy,
                &DestinationExpectation::ReplaceExisting(planned),
                &mut || Ok(()),
            )
            .unwrap_err();

            assert_eq!(error.kind(), io::ErrorKind::Interrupted);
            assert_eq!(
                fs::read_to_string(&dst).unwrap(),
                "someone else's file\n",
                "the swap must be undone, leaving the other writer's file in place"
            );
            assert!(no_temporaries_left(&destination_root));
        }
    }

    /// If the undo itself fails, the file left under the temporary name belongs
    /// to another writer. It must survive, and the error must say where it is.
    #[cfg(unix)]
    #[test]
    fn realfs_overwrite_keeps_the_stranded_file_when_the_swap_cannot_be_undone() {
        let tmp = TempDir::new().unwrap();
        let (source_root, destination_root) = fixture(&tmp, "file.env", "new\n");
        let dst = destination_root.join("file.env");
        write(&dst, "old\n");
        let planned = existing_snapshot(&dst);

        let substitute_root = destination_root.clone();
        let removed_dst = dst.clone();
        let _hooks = publish_hooks::install(publish_hooks::Hooks {
            after_destination_verified: Some(Box::new(move || {
                substitute_destination(&substitute_root, "file.env", "someone else's file\n");
            })),
            // Removing the destination name makes the undo swap fail: an
            // exchange needs both names to exist.
            before_exchange_back: Some(Box::new(move || {
                fs::remove_file(&removed_dst).unwrap();
            })),
            ..publish_hooks::Hooks::default()
        });

        let error = copy(
            &source_root,
            &destination_root,
            "file.env",
            CopyStrategy::SimpleCopy,
            &DestinationExpectation::ReplaceExisting(planned),
            &mut || Ok(()),
        )
        .unwrap_err();

        let stranded = temporaries(&destination_root);
        assert_eq!(
            stranded.len(),
            1,
            "the unverified file must not be deleted on the way out"
        );
        assert_eq!(
            fs::read_to_string(&stranded[0]).unwrap(),
            "someone else's file\n"
        );
        let message = error.to_string();
        let name = stranded[0].file_name().unwrap().to_string_lossy();
        assert!(
            message.contains(&*name),
            "the error must name the stranded file, got: {message}"
        );
    }

    /// The exchange succeeded, so the destination genuinely holds the new
    /// content — but the file it replaced could not be unlinked afterwards.
    /// Reporting that as a plain success would leave the previous content,
    /// secrets and all, under a `.waft-copy-*` name nothing will ever clean up.
    #[cfg(unix)]
    #[test]
    fn realfs_overwrite_reports_a_swapped_out_file_it_could_not_remove() {
        let tmp = TempDir::new().unwrap();
        let (source_root, destination_root) = fixture(&tmp, "file.env", "new\n");
        let dst = destination_root.join("file.env");
        write(&dst, "old\n");

        let _hooks = publish_hooks::install(publish_hooks::Hooks {
            // Stands in for a transient unlink failure on the last step.
            temporary_removal_fails: true,
            ..publish_hooks::Hooks::default()
        });

        let error = copy(
            &source_root,
            &destination_root,
            "file.env",
            CopyStrategy::SimpleCopy,
            &DestinationExpectation::ReplaceExisting(existing_snapshot(&dst)),
            &mut || Ok(()),
        )
        .unwrap_err();

        assert_eq!(
            fs::read_to_string(&dst).unwrap(),
            "new\n",
            "the replacement itself succeeded and must stand"
        );
        let leftover = temporaries(&destination_root);
        assert_eq!(
            leftover.len(),
            1,
            "the file that was replaced is still on disk"
        );
        assert_eq!(fs::read_to_string(&leftover[0]).unwrap(), "old\n");
        let message = error.to_string();
        assert!(
            message.contains(&*leftover[0].to_string_lossy()),
            "the error must name the full path of the file to delete, got: {message}"
        );
    }

    /// Filesystems without an atomic exchange (macOS SMB/NFS/exFAT, some
    /// overlay setups) take the unlink-then-publish path instead.
    #[cfg(unix)]
    #[test]
    fn realfs_overwrite_replaces_without_an_atomic_exchange() {
        for strategy in [CopyStrategy::SimpleCopy, CopyStrategy::CowCopy] {
            let tmp = TempDir::new().unwrap();
            let (source_root, destination_root) = fixture(&tmp, "file.env", "new\n");
            let dst = destination_root.join("file.env");
            write(&dst, "old\n");

            let _hooks = publish_hooks::install(publish_hooks::Hooks {
                exchange_unsupported: true,
                ..publish_hooks::Hooks::default()
            });

            let outcome = copy(
                &source_root,
                &destination_root,
                "file.env",
                strategy,
                &DestinationExpectation::ReplaceExisting(existing_snapshot(&dst)),
                &mut || Ok(()),
            )
            .unwrap();

            assert_eq!(outcome, PublishOutcome::Replaced);
            assert_eq!(fs::read_to_string(&dst).unwrap(), "new\n");
            assert!(no_temporaries_left(&destination_root));
        }
    }

    /// The inherent gap in the exchange-less fallback: the destination is
    /// already unlinked when the name is taken by someone else. The no-clobber
    /// publish refuses, and the prepared replacement must be kept rather than
    /// deleted — it is the only copy of the new content left.
    #[cfg(unix)]
    #[test]
    fn realfs_overwrite_without_exchange_keeps_the_replacement_when_the_name_reappears() {
        let tmp = TempDir::new().unwrap();
        let (source_root, destination_root) = fixture(&tmp, "file.env", "new\n");
        let dst = destination_root.join("file.env");
        write(&dst, "old\n");
        let planned = existing_snapshot(&dst);

        let reappear = dst.clone();
        let _hooks = publish_hooks::install(publish_hooks::Hooks {
            exchange_unsupported: true,
            after_destination_removed: Some(Box::new(move || {
                write(&reappear, "someone else's file\n");
            })),
            ..publish_hooks::Hooks::default()
        });

        let error = copy(
            &source_root,
            &destination_root,
            "file.env",
            CopyStrategy::SimpleCopy,
            &DestinationExpectation::ReplaceExisting(planned),
            &mut || Ok(()),
        )
        .unwrap_err();

        assert_eq!(
            fs::read_to_string(&dst).unwrap(),
            "someone else's file\n",
            "the file that took the name must never be clobbered"
        );
        let kept = temporaries(&destination_root);
        assert_eq!(kept.len(), 1, "the prepared replacement must be kept");
        assert_eq!(fs::read_to_string(&kept[0]).unwrap(), "new\n");
        let message = error.to_string();
        assert!(
            message.contains(&*kept[0].file_name().unwrap().to_string_lossy()),
            "the error must name the kept replacement, got: {message}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn realfs_removes_anchored_temp_when_publication_unwinds() {
        for strategy in [CopyStrategy::SimpleCopy, CopyStrategy::CowCopy] {
            let tmp = TempDir::new().unwrap();
            let (source_root, destination_root) = fixture(&tmp, "file.env", "source\n");

            let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _ = copy(
                    &source_root,
                    &destination_root,
                    "file.env",
                    strategy,
                    &DestinationExpectation::Missing,
                    &mut || panic!("tracked-state recheck exploded"),
                );
            }));

            assert!(panicked.is_err());
            assert!(!destination_root.join("file.env").exists());
            assert!(
                no_temporaries_left(&destination_root),
                "an unwinding panic must not leave a .waft-copy-* file behind"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn rename_flag_fallback_covers_every_unsupported_errno() {
        // These are the errnos a kernel or filesystem uses to say "this rename
        // flag is not implemented here" — notably macOS SMB/NFS/exFAT, which
        // answer ENOTSUP/EOPNOTSUPP rather than ENOSYS/EINVAL.
        for errno in [
            rustix::io::Errno::NOSYS,
            rustix::io::Errno::INVAL,
            rustix::io::Errno::NOTSUP,
            rustix::io::Errno::OPNOTSUPP,
        ] {
            assert!(rename_flag_unsupported(errno), "{errno:?}");
        }
        for errno in [
            rustix::io::Errno::EXIST,
            rustix::io::Errno::NOENT,
            rustix::io::Errno::ACCESS,
            rustix::io::Errno::XDEV,
        ] {
            assert!(!rename_flag_unsupported(errno), "{errno:?}");
        }
    }

    fn no_temporaries_left(directory: &Path) -> bool {
        fs::read_dir(directory).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".waft-copy-")
        })
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
    fn realfs_file_comparison_separates_permission_differences() {
        let tmp = TempDir::new().unwrap();
        let left = tmp.path().join("left");
        let right = tmp.path().join("right");
        write(&left, "same\n");
        write(&right, "same\n");
        fs::set_permissions(&left, fs::Permissions::from_mode(0o755)).unwrap();
        fs::set_permissions(&right, fs::Permissions::from_mode(0o644)).unwrap();

        assert_eq!(
            RealFs.compare_files(&left, &right).unwrap(),
            FileComparison::PermissionsDiffer
        );
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
