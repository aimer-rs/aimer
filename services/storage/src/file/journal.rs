//! The batch journal: the one file whose existence means "a batch committed".
//!
//! Values live one per file, and no filesystem renames several files
//! atomically. So a batch is first written, whole, into a single journal file:
//! staged beside it, synced, then renamed to [`JOURNAL`]. That rename is the
//! commit point. Before it, a crash leaves the store as it was; after it, a
//! crash leaves a journal that [`recover`](super::commit::recover) replays, and
//! replaying is idempotent, so it may be interrupted and repeated.
//!
//! ```text
//! "AIMERBAT" version:u8 count:u32le  op*  "END!"
//! op = 1 ns_len:u16le ns key_len:u16le key value_len:u32le value   (write)
//!    | 2 ns_len:u16le ns key_len:u16le key                          (remove)
//! ```
//!
//! The format is private to this crate and only ever read back by the same
//! version, but it is bounds-checked throughout: a damaged journal is reported
//! as corruption, never trusted.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

use crate::batch::FinalOp;
use crate::error::{StorageError, StorageOperation, StorageResult};
use crate::key::validate;

/// The committed journal, in the store root.
pub(super) const JOURNAL: &str = ".aimer-batch";
/// Where a journal is staged before its rename.
pub(super) const JOURNAL_STAGING: &str = ".aimer-batch-staging";

const MAGIC: &[u8; 8] = b"AIMERBAT";
const VERSION: u8 = 1;
const TRAILER: &[u8; 4] = b"END!";
const KIND_WRITE: u8 = 1;
const KIND_REMOVE: u8 = 2;

/// Whether a root-level name belongs to the journal rather than a namespace.
pub(super) fn is_journal_name(name: &str) -> bool {
    name == JOURNAL || name == JOURNAL_STAGING
}

/// One decoded journal operation.
#[derive(Debug, Eq, PartialEq)]
pub(super) struct JournalOp {
    pub(super) namespace: String,
    pub(super) key: String,
    /// The bytes to store, or `None` to remove.
    pub(super) value: Option<Vec<u8>>,
}

pub(super) fn encode(ops: &[FinalOp<'_>]) -> Vec<u8> {
    let payload: usize = ops
        .iter()
        .map(|op| 16 + op.namespace.len() + op.key.len() + op.value.map_or(0, <[u8]>::len))
        .sum();
    let mut bytes = Vec::with_capacity(MAGIC.len() + 5 + payload + TRAILER.len());
    bytes.extend_from_slice(MAGIC);
    bytes.push(VERSION);
    bytes.extend_from_slice(&(ops.len() as u32).to_le_bytes());
    for op in ops {
        bytes.push(if op.value.is_some() {
            KIND_WRITE
        } else {
            KIND_REMOVE
        });
        // Components are at most `MAX_COMPONENT_BYTES`, far below `u16::MAX`.
        bytes.extend_from_slice(&(op.namespace.len() as u16).to_le_bytes());
        bytes.extend_from_slice(op.namespace.as_bytes());
        bytes.extend_from_slice(&(op.key.len() as u16).to_le_bytes());
        bytes.extend_from_slice(op.key.as_bytes());
        if let Some(value) = op.value {
            bytes.extend_from_slice(&(value.len() as u32).to_le_bytes());
            bytes.extend_from_slice(value);
        }
    }
    bytes.extend_from_slice(TRAILER);
    bytes
}

/// A cursor that never reads past the end.
struct Reader<'a> {
    bytes: &'a [u8],
}

impl<'a> Reader<'a> {
    fn take(&mut self, len: usize) -> Option<&'a [u8]> {
        let (head, rest) = self.bytes.split_at_checked(len)?;
        self.bytes = rest;
        Some(head)
    }

    fn u8(&mut self) -> Option<u8> {
        self.take(1).map(|bytes| bytes[0])
    }

    fn u16(&mut self) -> Option<usize> {
        Some(usize::from(u16::from_le_bytes(
            self.take(2)?.try_into().ok()?,
        )))
    }

    fn u32(&mut self) -> Option<usize> {
        usize::try_from(u32::from_le_bytes(self.take(4)?.try_into().ok()?)).ok()
    }

    fn string(&mut self) -> Option<String> {
        let len = self.u16()?;
        String::from_utf8(self.take(len)?.to_vec()).ok()
    }
}

pub(super) fn decode(bytes: &[u8], operation: StorageOperation) -> StorageResult<Vec<JournalOp>> {
    let corrupt = || StorageError::Corrupt { operation };
    let mut reader = Reader { bytes };
    if reader.take(MAGIC.len()) != Some(MAGIC) || reader.u8() != Some(VERSION) {
        return Err(corrupt());
    }
    let count = reader.u32().ok_or_else(corrupt)?;
    // Every op takes at least 5 bytes, so a count the remaining bytes cannot
    // hold is rejected before it can reserve memory.
    if count > reader.bytes.len() / 5 {
        return Err(corrupt());
    }
    let mut ops = Vec::with_capacity(count);
    for _ in 0..count {
        let kind = reader.u8().ok_or_else(corrupt)?;
        let namespace = reader.string().ok_or_else(corrupt)?;
        let key = reader.string().ok_or_else(corrupt)?;
        // A journal naming a path outside the store is as bad as a torn one.
        validate(&namespace, &key).map_err(|_| corrupt())?;
        let value = match kind {
            KIND_WRITE => {
                let len = reader.u32().ok_or_else(corrupt)?;
                Some(reader.take(len).ok_or_else(corrupt)?.to_vec())
            }
            KIND_REMOVE => None,
            _ => return Err(corrupt()),
        };
        ops.push(JournalOp {
            namespace,
            key,
            value,
        });
    }
    if reader.take(TRAILER.len()) != Some(TRAILER) || !reader.bytes.is_empty() {
        return Err(corrupt());
    }
    Ok(ops)
}

/// Syncs a directory so a rename or removal inside it survives a crash.
/// Not every platform can open one, and the entry is already in place.
pub(super) fn sync_directory(directory: &Path) {
    if let Ok(directory) = File::open(directory) {
        let _ = directory.sync_all();
    }
}

/// Makes `bytes` the committed journal. When this returns `Ok`, the batch has
/// committed: it survives a crash and will be completed by recovery.
pub(super) fn commit(root: &Path, bytes: &[u8]) -> io::Result<()> {
    fs::create_dir_all(root)?;
    let staging = root.join(JOURNAL_STAGING);
    let result = (|| {
        // `create_new` fails on a leftover staging file instead of appending to
        // it; a stale one is removed first, since only this lane writes here.
        let _ = fs::remove_file(&staging);
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&staging)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&staging, root.join(JOURNAL))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&staging);
        return result;
    }
    sync_directory(root);
    Ok(())
}

/// The committed journal's bytes, or `None` when no batch is pending.
pub(super) fn read(root: &Path, operation: StorageOperation) -> StorageResult<Option<Vec<u8>>> {
    match fs::read(root.join(JOURNAL)) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(super::layout::native_error(operation, error)),
    }
}

/// Retires the journal once its batch is fully applied and durable.
pub(super) fn remove(root: &Path) -> io::Result<()> {
    match fs::remove_file(root.join(JOURNAL)) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => {
            sync_directory(root);
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OP: StorageOperation = StorageOperation::Write;

    fn ops() -> Vec<FinalOp<'static>> {
        vec![
            FinalOp {
                namespace: "settings",
                key: "theme",
                value: Some(b"dark"),
            },
            FinalOp {
                namespace: "drafts",
                key: "old",
                value: None,
            },
            FinalOp {
                namespace: "日本語",
                key: "k",
                value: Some(b""),
            },
        ]
    }

    #[test]
    fn a_journal_round_trips() {
        let decoded = decode(&encode(&ops()), OP).unwrap();
        assert_eq!(
            decoded,
            [
                JournalOp {
                    namespace: "settings".into(),
                    key: "theme".into(),
                    value: Some(b"dark".to_vec())
                },
                JournalOp {
                    namespace: "drafts".into(),
                    key: "old".into(),
                    value: None
                },
                JournalOp {
                    namespace: "日本語".into(),
                    key: "k".into(),
                    value: Some(Vec::new())
                },
            ]
        );
    }

    #[test]
    fn an_empty_journal_round_trips() {
        assert_eq!(decode(&encode(&[]), OP).unwrap(), []);
    }

    #[test]
    fn every_truncation_is_corruption() {
        let bytes = encode(&ops());
        for len in 0..bytes.len() {
            assert_eq!(
                decode(&bytes[..len], OP),
                Err(StorageError::Corrupt { operation: OP }),
                "truncated to {len} of {} bytes",
                bytes.len()
            );
        }
    }

    #[test]
    fn trailing_garbage_and_bad_headers_are_corruption() {
        let corrupt = Err(StorageError::Corrupt { operation: OP });
        let good = encode(&ops());

        let mut extra = good.clone();
        extra.push(0);
        assert_eq!(decode(&extra, OP), corrupt);

        let mut wrong_magic = good.clone();
        wrong_magic[0] ^= 0xff;
        assert_eq!(decode(&wrong_magic, OP), corrupt);

        let mut wrong_version = good.clone();
        wrong_version[MAGIC.len()] = VERSION + 1;
        assert_eq!(decode(&wrong_version, OP), corrupt);

        let mut wrong_kind = good;
        wrong_kind[MAGIC.len() + 5] = 9;
        assert_eq!(decode(&wrong_kind, OP), corrupt);
    }

    #[test]
    fn a_huge_claimed_count_is_rejected_without_reserving_memory() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(MAGIC);
        bytes.push(VERSION);
        bytes.extend_from_slice(&u32::MAX.to_le_bytes());
        bytes.extend_from_slice(TRAILER);
        assert_eq!(
            decode(&bytes, OP),
            Err(StorageError::Corrupt { operation: OP })
        );
    }

    #[test]
    fn a_journal_cannot_name_a_path_outside_the_store() {
        let escaping = [FinalOp {
            namespace: "ns",
            key: "../x",
            value: None,
        }];
        assert_eq!(
            decode(&encode(&escaping), OP),
            Err(StorageError::Corrupt { operation: OP })
        );
    }
}
