//! How `(namespace, key)` pairs and quota totals are spelled inside one
//! IndexedDB object store.
//!
//! This is plain string and byte logic with no browser types, so it is compiled
//! and tested on every target even though only the `wasm32` adapter uses it.
#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

use crate::limits::Usage;

/// The record that holds the store's quota totals.
///
/// It starts with a control byte, which no namespace can (see
/// [`is_valid_component`](crate::key::is_valid_component)), so it sorts before,
/// and can never collide with or fall inside the range of, any real record.
pub(crate) const USAGE_KEY: &str = "\u{1}usage";

/// The separator between a namespace and a key in a record's name. NUL cannot
/// appear in either, so the split is unambiguous.
const SEPARATOR: char = '\0';

/// The name of the record holding `key` in `namespace`.
pub(crate) fn record_key(namespace: &str, key: &str) -> String {
    let mut name = String::with_capacity(namespace.len() + 1 + key.len());
    name.push_str(namespace);
    name.push(SEPARATOR);
    name.push_str(key);
    name
}

/// The half-open range `[lower, upper)` of record names in `namespace`.
pub(crate) fn namespace_bounds(namespace: &str) -> (String, String) {
    (
        format!("{namespace}{SEPARATOR}"),
        // One past the separator: every record of this namespace is below it, and
        // every record of another namespace is outside the range, because a
        // longer namespace continues with a byte of at least 0x20.
        format!("{namespace}\u{1}"),
    )
}

/// The key part of a record name from `namespace`, if it is one.
pub(crate) fn key_of<'a>(record_key: &'a str, namespace: &str) -> Option<&'a str> {
    record_key.strip_prefix(namespace)?.strip_prefix(SEPARATOR)
}

const USAGE_BYTES: usize = 16;

/// Totals as two little-endian `u64`s, so a record written by a 64-bit
/// browser build reads the same on a 32-bit one.
pub(crate) fn encode_usage(usage: Usage) -> [u8; USAGE_BYTES] {
    let mut bytes = [0; USAGE_BYTES];
    bytes[..8].copy_from_slice(&(usage.entries as u64).to_le_bytes());
    bytes[8..].copy_from_slice(&(usage.total_bytes as u64).to_le_bytes());
    bytes
}

/// The inverse of [`encode_usage`]; `None` for anything that is not one, or
/// whose totals do not fit this platform's `usize`.
pub(crate) fn decode_usage(bytes: &[u8]) -> Option<Usage> {
    let bytes: &[u8; USAGE_BYTES] = bytes.try_into().ok()?;
    let entries = u64::from_le_bytes(bytes[..8].try_into().ok()?);
    let total_bytes = u64::from_le_bytes(bytes[8..].try_into().ok()?);
    Some(Usage {
        entries: usize::try_from(entries).ok()?,
        total_bytes: usize::try_from(total_bytes).ok()?,
    })
}

#[cfg(test)]
mod tests {
    use std::cmp::Ordering;

    use super::*;

    /// IndexedDB orders string keys by UTF-16 code unit, which differs from
    /// Rust's UTF-8 byte order for characters outside the Basic Multilingual
    /// Plane. The range arithmetic has to hold under the browser's order.
    fn idb_cmp(left: &str, right: &str) -> Ordering {
        left.encode_utf16().cmp(right.encode_utf16())
    }

    fn in_range(name: &str, bounds: &(String, String)) -> bool {
        idb_cmp(&bounds.0, name) != Ordering::Greater && idb_cmp(name, &bounds.1) == Ordering::Less
    }

    #[test]
    fn a_namespace_range_holds_exactly_its_own_records() {
        let namespaces = [
            "a",
            "ab",
            "ab c",
            "a b",
            "a-",
            "a\u{ffff}",
            "a😀",
            "😀",
            "日本語",
            "a~",
            "z",
        ];
        let keys = ["k", "a", "\u{ffff}", "😀", "a b"];

        for namespace in namespaces {
            let bounds = namespace_bounds(namespace);
            for other in namespaces {
                for key in keys {
                    let name = record_key(other, key);
                    assert_eq!(
                        in_range(&name, &bounds),
                        other == namespace,
                        "{name:?} against the range of {namespace:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_usage_record_is_outside_every_namespace_range() {
        for namespace in ["a", "ab", "😀", "\u{20}"] {
            assert!(
                !in_range(USAGE_KEY, &namespace_bounds(namespace)),
                "{namespace:?}"
            );
        }
    }

    #[test]
    fn a_record_name_splits_back_into_its_key() {
        assert_eq!(key_of(&record_key("ns", "key"), "ns"), Some("key"));
        assert_eq!(key_of(&record_key("ns", "key"), "n"), None);
        assert_eq!(key_of(&record_key("ns", "key"), "nsx"), None);
        assert_eq!(key_of(USAGE_KEY, "ns"), None);
    }

    #[test]
    fn usage_round_trips_and_rejects_foreign_records() {
        let usage = Usage {
            entries: 7,
            total_bytes: 123_456,
        };
        assert_eq!(decode_usage(&encode_usage(usage)), Some(usage));
        assert_eq!(decode_usage(&[]), None);
        assert_eq!(decode_usage(&[0; 15]), None);
        assert_eq!(decode_usage(&[0; 17]), None);
    }
}
