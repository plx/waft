#[cfg(target_os = "macos")]
use std::fs;

use tempfile::TempDir;

#[cfg(target_os = "macos")]
#[test]
fn reflink_copy_directory_input_spike() {
    let tmp = TempDir::new().unwrap();
    let src = tmp.path().join("src");
    let dst = tmp.path().join("dst");
    fs::create_dir_all(src.join("nested")).unwrap();
    fs::write(src.join("a.conf"), "a\n").unwrap();
    fs::write(src.join("nested/b.conf"), "b\n").unwrap();

    let result = reflink_copy::reflink(&src, &dst);
    if result.is_ok() {
        assert_eq!(fs::read_to_string(dst.join("a.conf")).unwrap(), "a\n");
        assert_eq!(
            fs::read_to_string(dst.join("nested/b.conf")).unwrap(),
            "b\n"
        );
    } else {
        assert!(!dst.exists());
    }
}

#[cfg(not(target_os = "macos"))]
#[test]
fn reflink_copy_directory_input_spike_is_macos_only() {
    let _ = TempDir::new().unwrap();
}
