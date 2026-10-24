//! What counts as a safe namespace, key, or secret.

use crate::error::{VaultError, VaultResult};

/// The maximum UTF-8 byte length of a namespace or key component.
///
/// The same bound and the same rules as `aimer_storage`, so a `(namespace, key)`
/// pair that is valid for one crate is valid for the other.
pub(crate) const MAX_COMPONENT_BYTES: usize = 127;

/// The maximum size of one secret.
///
/// The intersection of what every backend can hold: Windows Credential Manager
/// stores at most 2560 bytes (`CRED_MAX_CREDENTIAL_BLOB_SIZE`) per credential,
/// so a secret that fits here fits everywhere and the same application code
/// works on every target. It is room for an API key, a refresh token, or a
/// 256-bit key with its metadata. Larger data belongs encrypted in ordinary
/// storage, under a key kept in the vault.
pub const MAX_SECRET_BYTES: usize = 2560;

/// Whether `value` may be used as a namespace, key, or application id: no path
/// separators, no drive colons, no control bytes, and neither `.` nor `..`.
pub(crate) fn is_valid_component(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_COMPONENT_BYTES
        && value != "."
        && value != ".."
        && value.bytes().all(|byte| {
            byte >= 0x20 && byte != 0x7f && byte != b'/' && byte != b'\\' && byte != b':'
        })
}

pub(crate) fn validate_component(value: &str) -> VaultResult<()> {
    if is_valid_component(value) {
        Ok(())
    } else {
        Err(VaultError::InvalidKey)
    }
}

pub(crate) fn validate(namespace: &str, key: &str) -> VaultResult<()> {
    validate_component(namespace)?;
    validate_component(key)
}

/// Rejects a secret that is empty or over [`MAX_SECRET_BYTES`], before any
/// operation is queued.
pub(crate) fn check_secret(len: usize) -> VaultResult<()> {
    if len == 0 {
        Err(VaultError::EmptySecret)
    } else if len > MAX_SECRET_BYTES {
        Err(VaultError::SecretTooLarge {
            len,
            limit: MAX_SECRET_BYTES,
        })
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn components_are_bounded_and_free_of_path_syntax() {
        assert!(is_valid_component("api-token"));
        assert!(is_valid_component("com.example.app"));
        assert!(is_valid_component(&"k".repeat(MAX_COMPONENT_BYTES)));

        assert!(!is_valid_component(""));
        assert!(!is_valid_component("."));
        assert!(!is_valid_component(".."));
        assert!(!is_valid_component("a/b"));
        assert!(!is_valid_component("a\\b"));
        assert!(!is_valid_component("C:"));
        assert!(!is_valid_component("a\0b"));
        assert!(!is_valid_component("a\nb"));
        assert!(!is_valid_component("a\u{7f}"));
        assert!(!is_valid_component(&"k".repeat(MAX_COMPONENT_BYTES + 1)));
    }

    #[test]
    fn secrets_are_non_empty_and_bounded() {
        assert_eq!(check_secret(0), Err(VaultError::EmptySecret));
        assert_eq!(check_secret(1), Ok(()));
        assert_eq!(check_secret(MAX_SECRET_BYTES), Ok(()));
        assert_eq!(
            check_secret(MAX_SECRET_BYTES + 1),
            Err(VaultError::SecretTooLarge {
                len: MAX_SECRET_BYTES + 1,
                limit: MAX_SECRET_BYTES,
            })
        );
    }
}
