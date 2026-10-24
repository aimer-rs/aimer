//! A secret held in memory.

use std::fmt;

use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

/// Bytes that must not leak: wiped when dropped, and never printed.
///
/// `SecretBytes` is deliberately not `Clone` and its [`Debug`](fmt::Debug)
/// output is redacted, so a secret cannot be duplicated or logged by accident;
/// reading it is an explicit [`expose`](SecretBytes::expose). Equality compares
/// every byte regardless of where the first difference is, so comparing a
/// token does not reveal how much of a guess was right.
///
/// This is hygiene, not a guarantee. Wiping on drop shrinks how long a secret
/// sits in freed memory, but it cannot reach copies the platform made while
/// handing the bytes over, nor ones the caller made by copying out of
/// [`expose`](SecretBytes::expose).
///
/// # Examples
///
/// ```
/// use aimer_vault::SecretBytes;
///
/// let token = SecretBytes::from_slice(b"s3cr3t");
/// assert_eq!(token.expose(), b"s3cr3t");
/// assert_eq!(format!("{token:?}"), "SecretBytes(<redacted, 6 bytes>)");
/// ```
pub struct SecretBytes {
    // `Zeroizing` wipes the buffer on drop with writes the optimizer may not elide.
    bytes: Zeroizing<Vec<u8>>,
}

impl SecretBytes {
    /// Takes ownership of `bytes` without copying them.
    ///
    /// Prefer this over [`from_slice`](Self::from_slice) when the caller already
    /// owns a buffer: it leaves no second copy behind.
    #[inline]
    pub fn new(bytes: Vec<u8>) -> Self {
        Self {
            bytes: Zeroizing::new(bytes),
        }
    }

    /// Copies `bytes` into a new secret.
    #[inline]
    pub fn from_slice(bytes: &[u8]) -> Self {
        Self::new(bytes.to_vec())
    }

    /// Returns the secret's bytes.
    #[inline]
    pub fn expose(&self) -> &[u8] {
        &self.bytes
    }

    /// Returns the secret's byte length.
    #[inline]
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Returns whether the secret has no bytes.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

impl From<Vec<u8>> for SecretBytes {
    #[inline]
    fn from(bytes: Vec<u8>) -> Self {
        Self::new(bytes)
    }
}

impl From<&[u8]> for SecretBytes {
    #[inline]
    fn from(bytes: &[u8]) -> Self {
        Self::from_slice(bytes)
    }
}

impl PartialEq for SecretBytes {
    fn eq(&self, other: &Self) -> bool {
        self.bytes.as_slice().ct_eq(other.bytes.as_slice()).into()
    }
}

impl Eq for SecretBytes {}

impl fmt::Debug for SecretBytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SecretBytes(<redacted, {} bytes>)", self.bytes.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_never_shows_the_bytes() {
        let secret = SecretBytes::from_slice(b"hunter2");
        let printed = format!("{secret:?}");
        assert!(!printed.contains("hunter2"));
        assert_eq!(printed, "SecretBytes(<redacted, 7 bytes>)");
    }

    #[test]
    fn equality_compares_length_and_content() {
        let secret = SecretBytes::from_slice(b"abc");
        assert_eq!(secret, SecretBytes::from_slice(b"abc"));
        assert_ne!(secret, SecretBytes::from_slice(b"abd"));
        assert_ne!(secret, SecretBytes::from_slice(b"ab"));
        assert_ne!(secret, SecretBytes::from_slice(b"abcd"));
    }

    #[test]
    fn new_keeps_the_callers_allocation() {
        let bytes = vec![1, 2, 3];
        let pointer = bytes.as_ptr();
        let secret = SecretBytes::new(bytes);
        assert_eq!(secret.expose().as_ptr(), pointer);
        assert_eq!(secret.len(), 3);
        assert!(!secret.is_empty());
    }
}
