//! Recognition policy for Apple-private glyph data.
//!
//! `hvgl` and `emjc` are implementation details of Apple system fonts, not
//! portable OpenType formats. The Aimer reader deliberately does not inspect
//! their payloads. It records the direct `hvgl`/`emjc` tags here; the bitmap
//! reader separately records an `emjc` graphic type inside a validated `sbix`
//! index. Both signals feed the same explicit fallback decision.

use super::TableRecord;

const HVGL_TAG: super::Tag = super::Tag::from_bytes(*b"hvgl");
const EMJC_TAG: super::Tag = super::Tag::from_bytes(*b"emjc");

const HVGL_FLAG: u8 = 1 << 0;
const EMJC_FLAG: u8 = 1 << 1;

/// Apple-private glyph data advertised by one face.
///
/// This is intentionally a directory-level classification rather than a
/// decoder. A private table is never treated as a public outline or bitmap
/// table merely because its bytes happen to resemble one.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ApplePrivateTables {
    flags: u8,
}

impl ApplePrivateTables {
    /// Classifies private table tags after the SFNT directory has validated
    /// their offsets and lengths.
    pub(super) fn from_records(records: &[TableRecord]) -> Self {
        let mut flags = 0;
        for record in records {
            flags |= match record.tag {
                HVGL_TAG => HVGL_FLAG,
                EMJC_TAG => EMJC_FLAG,
                _ => 0,
            };
        }
        Self { flags }
    }

    #[inline]
    pub(crate) const fn has_hvgl(self) -> bool {
        self.flags & HVGL_FLAG != 0
    }

    #[inline]
    pub(crate) const fn has_emjc(self) -> bool {
        self.flags & EMJC_FLAG != 0
    }

    #[inline]
    pub(crate) const fn is_empty(self) -> bool {
        self.flags == 0
    }

    /// Returns whether the portable rasterizer must decline this face.
    ///
    /// A public outline takes precedence when a font carries both a public
    /// outline and an Apple-private table. Private-only faces are handed to
    /// the compatibility resolver; on a platform without that resolver the
    /// normal missing-glyph contract supplies the advance and no pixels.
    #[inline]
    pub(crate) const fn requires_platform_raster(self, has_standard_outline: bool) -> bool {
        !self.is_empty() && !has_standard_outline
    }

    /// Returns whether the face must be routed as color data for the platform
    /// fallback. `emjc` is a private color-strike format; it is not decoded by
    /// the portable bitmap reader.
    #[inline]
    pub(crate) const fn has_private_color(self) -> bool {
        self.has_emjc()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(tag: [u8; 4]) -> TableRecord {
        TableRecord {
            tag: super::super::Tag::from_bytes(tag),
            offset: 0,
            length: 0,
        }
    }

    #[test]
    fn recognizes_private_tags_without_reading_payloads() {
        let tables = ApplePrivateTables::from_records(&[record(*b"hvgl"), record(*b"emjc")]);

        assert!(tables.has_hvgl());
        assert!(tables.has_emjc());
        assert!(tables.has_private_color());
        assert!(tables.requires_platform_raster(false));
        assert!(!tables.requires_platform_raster(true));
    }

    #[test]
    fn ignores_public_and_unknown_tags() {
        let tables = ApplePrivateTables::from_records(&[
            record(*b"glyf"),
            record(*b"sbix"),
            record(*b"xxxx"),
        ]);

        assert_eq!(tables, ApplePrivateTables::default());
        assert!(!tables.has_private_color());
        assert!(!tables.requires_platform_raster(false));
    }
}
