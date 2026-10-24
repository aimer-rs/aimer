use sha2::{Digest, Sha256};

const DERIVATION_DOMAIN: &[u8] = b"aimer.stable-id.v1\0";
const CAPABILITY_DERIVATION_DOMAIN: &[u8] = b"aimer.capability-id.v1\0";

/// The semantic domain of a stable application identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdentityKind {
    /// A host-native widget key.
    Widget,
    /// A guest callback binding.
    Callback,
    /// A versioned guest-state entry.
    State,
}

/// A deterministic 128-bit identity shared by native and WebAssembly adapters.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct StableId128([u8; 16]);

impl StableId128 {
    /// Creates an identity from its canonical wire representation.
    ///
    /// This operation does not derive or validate semantic names. Decoders use
    /// it after reading exactly 16 bytes from a trusted canonical boundary.
    #[inline]
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    /// Derives an identity from a stable application namespace and declared name.
    ///
    /// Derivation hashes the `aimer.stable-id.v1` domain, the identity-kind
    /// byte, and both UTF-8 strings prefixed by their little-endian `u64`
    /// lengths with SHA-256. The first 16 digest bytes form the stable ID.
    ///
    /// Namespaces and names are semantic declarations. They must not contain
    /// source locations, compiler hashes, dependency versions, or filesystem
    /// paths because changing any input intentionally changes the identity.
    pub fn derive(namespace: &str, kind: IdentityKind, name: &str) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(DERIVATION_DOMAIN);
        hasher.update([kind.domain_tag()]);
        hasher.update((namespace.len() as u64).to_le_bytes());
        hasher.update(namespace.as_bytes());
        hasher.update((name.len() as u64).to_le_bytes());
        hasher.update(name.as_bytes());

        let digest = hasher.finalize();
        let mut identity = [0_u8; 16];
        identity.copy_from_slice(&digest[..16]);
        Self(identity)
    }

    /// Derives the wire identity of a canonical capability contract name.
    ///
    /// Capability names are complete package-scoped identities such as
    /// `crates.io::aimer_haptics::haptics` or an explicit globally unique ID.
    /// The derivation hashes the `aimer.capability-id.v1` domain followed by
    /// the little-endian `u64` byte length and UTF-8 identity. Package
    /// versions, source paths, compiler hashes, and source locations must not
    /// be included in `canonical_id`.
    #[inline]
    pub fn derive_capability(canonical_id: &str) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(CAPABILITY_DERIVATION_DOMAIN);
        hasher.update((canonical_id.len() as u64).to_le_bytes());
        hasher.update(canonical_id.as_bytes());

        let digest = hasher.finalize();
        let mut identity = [0_u8; 16];
        identity.copy_from_slice(&digest[..16]);
        Self(identity)
    }

    /// Returns the canonical 16-byte identity.
    #[inline]
    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

impl IdentityKind {
    #[inline]
    const fn domain_tag(self) -> u8 {
        match self {
            Self::Widget => 1,
            Self::Callback => 2,
            Self::State => 3,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declared_identities_match_the_version_one_golden_vectors() {
        let widget = StableId128::derive(
            "crates.io::counter_app",
            IdentityKind::Widget,
            "counter_button",
        );
        let callback = StableId128::derive(
            "crates.io::counter_app",
            IdentityKind::Callback,
            "counter_button",
        );

        assert_eq!(
            widget.as_bytes(),
            &[
                0xD3, 0xD7, 0x3F, 0x7C, 0x91, 0xAD, 0x15, 0x1B, 0xF7, 0x4C, 0x7D, 0x72, 0x4A, 0x4E,
                0xDB, 0x71,
            ]
        );
        assert_eq!(
            callback.as_bytes(),
            &[
                0xEA, 0xFF, 0x7C, 0x1D, 0x57, 0x21, 0x0F, 0xCB, 0x93, 0x7F, 0x48, 0x32, 0x78, 0xF0,
                0xDE, 0xE0,
            ]
        );
        assert_ne!(widget, callback);
    }

    #[test]
    fn state_identity_matches_the_version_one_golden_vector() {
        let state = StableId128::derive("crates.io::counter_app", IdentityKind::State, "counter");

        assert_eq!(
            state.as_bytes(),
            &[
                0x27, 0xC4, 0x46, 0x02, 0xF7, 0x47, 0xE7, 0x10, 0x72, 0xE6, 0x50, 0x6E, 0x8D, 0x9B,
                0x2F, 0xEA,
            ]
        );
    }
}
