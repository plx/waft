use std::io;
use std::path::Path;

#[cfg(target_os = "macos")]
use std::ffi::CString;
#[cfg(target_os = "macos")]
use std::os::unix::ffi::OsStrExt;

#[cfg(target_os = "macos")]
const CLONE_NOFOLLOW: u32 = 0x0001;

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn clonefile(
        src: *const std::os::raw::c_char,
        dst: *const std::os::raw::c_char,
        flags: u32,
    ) -> std::os::raw::c_int;
}

/// Clone a directory tree with macOS `clonefile(2)`.
///
/// The destination must not exist. Callers must preflight the source tree
/// before invoking this primitive; `clonefile` recursively reproduces the
/// physical directory tree exactly.
#[cfg(target_os = "macos")]
pub(crate) fn clonefile_dir(src: &Path, dst: &Path) -> io::Result<()> {
    let src = CString::new(src.as_os_str().as_bytes()).map_err(|_| {
        io::Error::new(io::ErrorKind::InvalidInput, "source path contains NUL byte")
    })?;
    let dst = CString::new(dst.as_os_str().as_bytes()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "destination path contains NUL byte",
        )
    })?;

    // SAFETY: `src` and `dst` are valid NUL-terminated C strings for the
    // duration of the call. `CLONE_NOFOLLOW` prevents following a symlink at
    // the top-level source or destination path.
    let rc = unsafe { clonefile(src.as_ptr(), dst.as_ptr(), CLONE_NOFOLLOW) };
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Directory clonefile is only available on macOS.
#[cfg(not(target_os = "macos"))]
pub(crate) fn clonefile_dir(_src: &Path, _dst: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "clonefile is only available on macOS",
    ))
}
