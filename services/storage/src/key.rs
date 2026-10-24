//! What counts as a safe namespace or key.

use crate::error::{StorageError, StorageResult};
use crate::limits::StorageLimits;

/// Whether `value` may be used as a namespace or a key.
///
/// The rules are the intersection of what every adapter can spell: no path
/// separators, no drive colons, no control bytes, and neither `.` nor `..`.
/// The control-byte ban also lets the IndexedDB adapter use NUL as a separator
/// that no component can contain.
pub(crate) fn is_valid_component(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= StorageLimits::MAX_COMPONENT_BYTES
        && value != "."
        && value != ".."
        && value.bytes().all(|byte| {
            byte >= 0x20 && byte != 0x7f && byte != b'/' && byte != b'\\' && byte != b':'
        })
}

pub(crate) fn validate_namespace(namespace: &str) -> StorageResult<()> {
    if is_valid_component(namespace) {
        Ok(())
    } else {
        Err(StorageError::InvalidKey)
    }
}

pub(crate) fn validate(namespace: &str, key: &str) -> StorageResult<()> {
    validate_namespace(namespace)?;
    validate_namespace(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn components_are_bounded_and_free_of_path_syntax() {
        assert!(is_valid_component("display-mode"));
        assert!(is_valid_component("com.example.app"));
        assert!(is_valid_component(
            &"k".repeat(StorageLimits::MAX_COMPONENT_BYTES)
        ));

        assert!(!is_valid_component(""));
        assert!(!is_valid_component("."));
        assert!(!is_valid_component(".."));
        assert!(!is_valid_component("a/b"));
        assert!(!is_valid_component("a\\b"));
        assert!(!is_valid_component("C:"));
        assert!(!is_valid_component("a\0b"));
        assert!(!is_valid_component("a\nb"));
        assert!(!is_valid_component("a\u{7f}"));
        assert!(!is_valid_component(
            &"k".repeat(StorageLimits::MAX_COMPONENT_BYTES + 1)
        ));
    }
}
