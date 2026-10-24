use std::fmt;

const FNV_OFFSET_BASIS: u128 = 0x6c62_272e_07bb_0142_62b8_2175_6295_c58d;
const FNV_PRIME: u128 = 0x0000_0000_0100_0000_0000_0000_0000_013b;

/// A stable 128-bit identity used by portable Aimer metadata.
///
/// Unlike [`std::any::TypeId`], this value is independent of a process, target,
/// and compiler invocation. Generated code should derive IDs from canonical
/// package, module, type, or slot paths and keep the domain string unchanged.
#[doc(hidden)]
#[derive(Clone, Copy, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct StableId128([u8; 16]);

impl StableId128 {
    /// The all-zero identity.
    pub const ZERO: Self = Self([0; 16]);

    /// Creates an identity from its canonical little-endian bytes.
    #[inline]
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    /// Returns the canonical little-endian bytes.
    #[inline]
    pub const fn to_bytes(self) -> [u8; 16] {
        self.0
    }

    /// Creates an identity from a numeric value.
    #[inline]
    pub const fn from_u128(value: u128) -> Self {
        Self(value.to_le_bytes())
    }

    /// Returns this identity as a numeric value.
    #[inline]
    pub const fn to_u128(self) -> u128 {
        u128::from_le_bytes(self.0)
    }

    /// Deterministically derives an identity from a domain and canonical path.
    ///
    /// Length framing makes `(domain, path)` unambiguous. Domains should name
    /// the generated identity class, for example `"aimer.package.v1"` or
    /// `"aimer.type.v1"`.
    #[inline]
    pub const fn from_path(domain: &str, path: &str) -> Self {
        let mut hasher = StableHasher::new();
        hasher.write_str(domain);
        hasher.write_str(path);
        hasher.finish()
    }

    /// Deterministically derives an identity from framed path segments.
    #[inline]
    pub const fn from_segments(domain: &str, segments: &[&str]) -> Self {
        let mut hasher = StableHasher::new();
        hasher.write_str(domain);
        let mut index = 0;
        while index < segments.len() {
            hasher.write_str(segments[index]);
            index += 1;
        }
        hasher.finish()
    }
}

impl fmt::Debug for StableId128 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "StableId128({self})")
    }
}

impl fmt::Display for StableId128 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0.iter().rev() {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

/// A stable identity assigned to a reflected Rust type.
#[doc(hidden)]
pub type StableTypeId = StableId128;

/// A stable identity assigned to one retained state slot.
#[doc(hidden)]
pub type StableSlotId = StableId128;

/// A stable fingerprint of one portable schema.
#[doc(hidden)]
pub type StableSchemaId = StableId128;

pub(crate) struct StableHasher {
    state: u128,
}

impl StableHasher {
    #[inline]
    pub(crate) const fn new() -> Self {
        Self { state: FNV_OFFSET_BASIS }
    }

    #[inline]
    pub(crate) const fn write_byte(&mut self, byte: u8) {
        self.state ^= byte as u128;
        self.state = self.state.wrapping_mul(FNV_PRIME);
    }

    pub(crate) const fn write_bytes(&mut self, bytes: &[u8]) {
        let mut index = 0;
        while index < bytes.len() {
            self.write_byte(bytes[index]);
            index += 1;
        }
    }

    pub(crate) const fn write_u64(&mut self, value: u64) {
        self.write_bytes(&value.to_le_bytes());
    }

    pub(crate) const fn write_str(&mut self, value: &str) {
        self.write_u64(value.len() as u64);
        self.write_bytes(value.as_bytes());
    }

    #[inline]
    pub(crate) const fn finish(self) -> StableId128 {
        StableId128::from_u128(self.state)
    }
}

#[cfg(test)]
mod tests {
    use super::StableId128;

    #[test]
    fn identity_is_const_stable_and_domain_separated() {
        const FIRST: StableId128 = StableId128::from_path("package", "app::Counter");
        const SECOND: StableId128 = StableId128::from_path("package", "app::Counter");
        const OTHER: StableId128 = StableId128::from_path("module", "app::Counter");

        assert_eq!(FIRST, SECOND);
        assert_ne!(FIRST, OTHER);
        assert_eq!(StableId128::from_bytes(FIRST.to_bytes()), FIRST);
        assert_eq!(StableId128::from_u128(FIRST.to_u128()), FIRST);
        assert_ne!(
            StableId128::from_segments("type", &["ab", "c"]),
            StableId128::from_segments("type", &["a", "bc"])
        );
    }
}
