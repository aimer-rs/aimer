//! How values are laid out on disk, and the blocking primitives that touch it.
//!
//! Everything here is synchronous and runs on a Venus offload worker.
//!
//! ```text
//! <root>/<hex(namespace)>/<hex(key)>        one file per value
//! <root>/<hex(namespace)>/.aimer-tmp-<pid>-<n>   a write in flight
//! ```
//!
//! Components are spelled as lowercase hex rather than escaped: it is the one
//! encoding that never collides on a case-insensitive filesystem, and it keeps
//! every byte a component may contain out of the path.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::error::{StorageError, StorageIoKind, StorageOperation, StorageResult};
use crate::key::is_valid_component;
use crate::limits::Usage;

/// Prefix of the file a write is staged in before it is renamed into place.
pub(super) const TEMP_PREFIX: &str = ".aimer-tmp-";

const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";

pub(super) fn native_error(operation: StorageOperation, error: io::Error) -> StorageError {
    match error.kind() {
        io::ErrorKind::PermissionDenied => StorageError::PermissionDenied { operation },
        io::ErrorKind::InvalidData | io::ErrorKind::UnexpectedEof => {
            StorageError::Corrupt { operation }
        }
        io::ErrorKind::StorageFull | io::ErrorKind::QuotaExceeded => StorageError::Io {
            operation,
            kind: StorageIoKind::StorageFull,
        },
        io::ErrorKind::ReadOnlyFilesystem => StorageError::Io {
            operation,
            kind: StorageIoKind::ReadOnly,
        },
        _ => StorageError::Io {
            operation,
            kind: StorageIoKind::Other,
        },
    }
}

pub(super) fn encode_component(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len() * 2);
    for byte in value.bytes() {
        encoded.push(HEX_DIGITS[(byte >> 4) as usize] as char);
        encoded.push(HEX_DIGITS[(byte & 0x0f) as usize] as char);
    }
    encoded
}

fn decode_hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        // Uppercase is not something this adapter writes, and accepting it would
        // let two file names decode to one key.
        _ => None,
    }
}

pub(super) fn decode_component(value: &str, operation: StorageOperation) -> StorageResult<String> {
    let corrupt = || StorageError::Corrupt { operation };
    if value.is_empty() || !value.len().is_multiple_of(2) {
        return Err(corrupt());
    }
    let mut bytes = Vec::with_capacity(value.len() / 2);
    // The length is even, so there is no remainder.
    let (pairs, _) = value.as_bytes().as_chunks::<2>();
    for &[high, low] in pairs {
        let high = decode_hex(high).ok_or_else(corrupt)?;
        let low = decode_hex(low).ok_or_else(corrupt)?;
        bytes.push((high << 4) | low);
    }
    let decoded = String::from_utf8(bytes).map_err(|_| corrupt())?;
    if is_valid_component(&decoded) {
        Ok(decoded)
    } else {
        Err(corrupt())
    }
}

pub(super) fn namespace_path(root: &Path, namespace: &str) -> PathBuf {
    root.join(encode_component(namespace))
}

pub(super) fn value_path(root: &Path, namespace: &str, key: &str) -> PathBuf {
    namespace_path(root, namespace).join(encode_component(key))
}

/// The size of the regular file at `path`, or `None` when nothing is there.
pub(super) fn stat_value(path: &Path, operation: StorageOperation) -> StorageResult<Option<usize>> {
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => usize::try_from(metadata.len())
            .map(Some)
            .map_err(|_| StorageError::Corrupt { operation }),
        Ok(_) => Err(StorageError::Corrupt { operation }),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            // Unix reports a path through a regular file as `NotADirectory`.
            // Windows reports it as plain "not found", which would let a
            // batch plan around a namespace that is really a file in the way.
            #[cfg(windows)]
            if path
                .parent()
                .and_then(|parent| fs::metadata(parent).ok())
                .is_some_and(|parent| !parent.is_dir())
            {
                return Err(StorageError::Corrupt { operation });
            }
            Ok(None)
        }
        Err(error) => Err(native_error(operation, error)),
    }
}

/// Walks the whole store once and totals what is in it.
///
/// This is the only full scan. It runs the first time a quota decision is
/// needed, and the totals are then maintained incrementally.
pub(super) fn scan_disk(root: &Path, operation: StorageOperation) -> StorageResult<Usage> {
    let namespaces = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Usage::default()),
        Err(error) => return Err(native_error(operation, error)),
    };

    let corrupt = || StorageError::Corrupt { operation };
    let mut usage = Usage::default();
    for namespace_entry in namespaces {
        let namespace_entry = namespace_entry.map_err(|error| native_error(operation, error))?;
        // The batch journal sits beside the namespaces and is not one of them.
        if namespace_entry
            .file_name()
            .to_str()
            .is_some_and(super::journal::is_journal_name)
        {
            continue;
        }
        let file_type = namespace_entry
            .file_type()
            .map_err(|error| native_error(operation, error))?;
        if !file_type.is_dir() {
            return Err(corrupt());
        }
        let encoded_namespace = namespace_entry
            .file_name()
            .into_string()
            .map_err(|_| corrupt())?;
        decode_component(&encoded_namespace, operation)?;
        for value_entry in
            fs::read_dir(namespace_entry.path()).map_err(|error| native_error(operation, error))?
        {
            let value_entry = value_entry.map_err(|error| native_error(operation, error))?;
            let name = value_entry
                .file_name()
                .into_string()
                .map_err(|_| corrupt())?;
            if name.starts_with(TEMP_PREFIX) {
                continue;
            }
            decode_component(&name, operation)?;
            let size = stat_value(&value_entry.path(), operation)?.ok_or_else(corrupt)?;
            usage.entries = usage.entries.checked_add(1).ok_or_else(corrupt)?;
            usage.total_bytes = usage.total_bytes.checked_add(size).ok_or_else(corrupt)?;
        }
    }
    Ok(usage)
}

/// Every `(key, size)` in a namespace, sorted by key.
pub(super) fn namespace_entries(
    root: &Path,
    namespace: &str,
    operation: StorageOperation,
) -> StorageResult<Vec<(String, usize)>> {
    let values = match fs::read_dir(namespace_path(root, namespace)) {
        Ok(values) => values,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(native_error(operation, error)),
    };
    let corrupt = || StorageError::Corrupt { operation };
    let mut entries = Vec::new();
    for value_entry in values {
        let value_entry = value_entry.map_err(|error| native_error(operation, error))?;
        let name = value_entry
            .file_name()
            .into_string()
            .map_err(|_| corrupt())?;
        if name.starts_with(TEMP_PREFIX) {
            continue;
        }
        let key = decode_component(&name, operation)?;
        let size = stat_value(&value_entry.path(), operation)?.ok_or_else(corrupt)?;
        entries.push((key, size));
    }
    entries.sort_unstable_by(|left, right| left.0.cmp(&right.0));
    Ok(entries)
}

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(1);

/// Replaces `path` with `value` so a reader sees the old value or the new one,
/// never a torn write, and the new one survives a crash once this returns.
pub(super) fn write_atomically(path: &Path, value: &[u8]) -> io::Result<()> {
    write_via_temp(path, value, true)
}

/// [`write_atomically`] without the syncs: a reader still never sees a torn
/// value, but nothing is promised after a crash. For callers that hold the same
/// bytes durably elsewhere, like a committed batch journal.
pub(super) fn write_unsynced(path: &Path, value: &[u8]) -> io::Result<()> {
    write_via_temp(path, value, false)
}

fn write_via_temp(path: &Path, value: &[u8], durable: bool) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "storage path has no parent"))?;
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(
        "{TEMP_PREFIX}{}-{}",
        std::process::id(),
        NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        file.write_all(value)?;
        if durable {
            file.sync_all()?;
        }
        fs::rename(&temporary, path)?;
        // Without this the rename itself can be lost in a crash. Not every
        // platform can open a directory, and the value is already in place.
        if durable && let Ok(directory) = File::open(parent) {
            let _ = directory.sync_all();
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn components_round_trip_through_their_file_name() {
        let operation = StorageOperation::Read;
        for original in ["a", "display-mode", "naïve.key", "日本語", "a b"] {
            let encoded = encode_component(original);
            assert!(
                encoded
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            );
            assert_eq!(
                decode_component(&encoded, operation).as_deref(),
                Ok(original)
            );
        }
    }

    #[test]
    fn a_longest_component_fits_in_a_file_name() {
        let longest = "k".repeat(crate::StorageLimits::MAX_COMPONENT_BYTES);
        assert!(encode_component(&longest).len() <= 255);
    }

    #[test]
    fn foreign_file_names_are_corrupt() {
        let operation = StorageOperation::List;
        for name in ["", "6", "zz", "6A", ".DS_Store", "2f"] {
            assert_eq!(
                decode_component(name, operation),
                Err(StorageError::Corrupt { operation }),
                "{name:?}"
            );
        }
    }
}
