//! Where encrypted secrets live on disk.
// Only the Android adapter uses this, but it is plain logic, so it is compiled and
// tested on every target.
#![cfg_attr(not(target_os = "android"), allow(dead_code))]


use std::fs::{self, File};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

use crate::error::{VaultError, VaultResult};

/// The directory under the root that holds every vault of an application.
const DIRECTORY: &str = "aimer-vault";

/// The paths of one application's secrets.
///
/// Names are lowercase hex of the UTF-8 bytes, so any valid namespace or key is
/// a safe file name on any filesystem, and two names differ on disk exactly when
/// they differ as strings. 127 bytes is the longest component whose encoding
/// (254 characters) still fits a 255-byte file name.
pub(crate) struct Layout {
    app: PathBuf,
}

impl Layout {
    pub(crate) fn new(root: PathBuf, app_id: &str) -> Self {
        Self {
            app: root.join(DIRECTORY).join(hex(app_id)),
        }
    }

    pub(crate) fn namespace(&self, namespace: &str) -> PathBuf {
        self.app.join(hex(namespace))
    }

    pub(crate) fn secret(&self, namespace: &str, key: &str) -> PathBuf {
        self.namespace(namespace).join(hex(key))
    }
}

fn hex(text: &str) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(text.len() * 2);
    for byte in text.bytes() {
        encoded.push(char::from(DIGITS[usize::from(byte >> 4)]));
        encoded.push(char::from(DIGITS[usize::from(byte & 0xf)]));
    }
    encoded
}

fn io_error(_: std::io::Error) -> VaultError {
    VaultError::Failed
}

pub(crate) fn read(path: &Path) -> VaultResult<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(io_error(error)),
    }
}

/// Replaces the file atomically: a reader sees the old secret or the new one,
/// never a torn write, and a crash leaves at worst a stray `.tmp` file.
pub(crate) fn write(path: &Path, bytes: &[u8]) -> VaultResult<()> {
    let parent = path.parent().expect("a secret path has a namespace directory");
    fs::create_dir_all(parent).map_err(io_error)?;
    let temporary = path.with_extension("tmp");
    let mut file = File::create(&temporary).map_err(io_error)?;
    let written = file
        .write_all(bytes)
        .and_then(|()| file.sync_all())
        .and_then(|()| fs::rename(&temporary, path));
    if let Err(error) = written {
        let _ = fs::remove_file(&temporary);
        return Err(io_error(error));
    }
    Ok(())
}

/// Removes one file and reports whether it existed.
pub(crate) fn remove(path: &Path) -> VaultResult<bool> {
    match fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(io_error(error)),
    }
}

/// Removes a namespace directory and everything in it; an absent one is fine.
pub(crate) fn remove_namespace(path: &Path) -> VaultResult<()> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io_error(error)),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(1);
            Self(std::env::temp_dir().join(format!(
                "aimer-vault-files-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            )))
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn names_are_hex_and_distinct() {
        assert_eq!(hex("ab"), "6162");
        assert_eq!(hex("é"), "c3a9");
        assert_ne!(hex("a.b"), hex("a_b"));
        assert_eq!(hex(&"k".repeat(127)).len(), 254);
    }

    #[test]
    fn a_secret_lives_under_its_application_and_namespace() {
        let layout = Layout::new(PathBuf::from("/root"), "app");
        assert_eq!(
            layout.secret("ns", "key"),
            Path::new("/root/aimer-vault").join(hex("app")).join(hex("ns")).join(hex("key"))
        );
        assert!(layout.secret("ns", "key").starts_with(layout.namespace("ns")));
        assert!(!layout.secret("ns2", "key").starts_with(layout.namespace("ns")));
    }

    #[test]
    fn writes_replace_and_leave_no_temporary_file() {
        let scratch = Scratch::new();
        let path = Layout::new(scratch.0.clone(), "app").secret("ns", "key");

        assert_eq!(read(&path).unwrap(), None);
        write(&path, b"one").unwrap();
        write(&path, b"two").unwrap();
        assert_eq!(read(&path).unwrap().as_deref(), Some(&b"two"[..]));
        assert!(!path.with_extension("tmp").exists());
    }

    #[test]
    fn remove_reports_existence_and_clear_empties_a_namespace() {
        let scratch = Scratch::new();
        let layout = Layout::new(scratch.0.clone(), "app");
        write(&layout.secret("ns", "a"), b"1").unwrap();
        write(&layout.secret("ns", "b"), b"2").unwrap();
        write(&layout.secret("other", "a"), b"3").unwrap();

        assert!(remove(&layout.secret("ns", "a")).unwrap());
        assert!(!remove(&layout.secret("ns", "a")).unwrap());

        remove_namespace(&layout.namespace("ns")).unwrap();
        assert_eq!(read(&layout.secret("ns", "b")).unwrap(), None);
        assert!(read(&layout.secret("other", "a")).unwrap().is_some());
        // Clearing what is already gone is not an error.
        remove_namespace(&layout.namespace("ns")).unwrap();
    }
}
