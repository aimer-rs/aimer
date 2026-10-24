//! Small, allocation-free Unicode script classifier used by the owned text
//! pipeline.
//!
//! The renderer only needs a script identity to keep shaping runs apart. It
//! does not need a complete Unicode property database here: the Aimer layout
//! dispatcher owns the detailed script checks and returns a checked fallback
//! result for scripts outside its supported shaping slices. Keeping this
//! classifier local removes a font-engine dependency from the hot run-building
//! path while retaining the important invariant that, for example, Khmer and
//! Latin are never shaped as one run.

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum Script {
    Common,
    Inherited,
    Unknown,
    Latin,
    Greek,
    Cyrillic,
    Armenian,
    Hebrew,
    Arabic,
    Devanagari,
    Bengali,
    Gurmukhi,
    Gujarati,
    Oriya,
    Tamil,
    Telugu,
    Kannada,
    Malayalam,
    Sinhala,
    Thai,
    Lao,
    Khmer,
    Myanmar,
    Ethiopic,
    Georgian,
    Tibetan,
    Han,
    Hiragana,
    Katakana,
    Hangul,
    Mongolian,
    Cherokee,
    Yi,
    Other,
}

impl Script {
    /// Classifies one Unicode scalar into the run identity needed by Cupid.
    ///
    /// Common and inherited characters deliberately remain distinct from a
    /// writing script. Callers can therefore attach spaces and combining marks
    /// to the surrounding run without splitting it, while unsupported scripts
    /// still form stable boundaries instead of being accidentally combined with
    /// a supported script.
    #[inline]
    pub(crate) fn for_codepoint(codepoint: char) -> Self {
        let value = codepoint as u32;
        match value {
            0x0300..=0x036F
            | 0x1AB0..=0x1AFF
            | 0x1DC0..=0x1DFF
            | 0x20D0..=0x20FF
            | 0xFE00..=0xFE0F
            | 0xE0100..=0xE01EF
            | 0x200C..=0x200D => Self::Inherited,
            0x0000..=0x0040
            | 0x005B..=0x0060
            | 0x007B..=0x00BF
            | 0x2000..=0x206F
            | 0x2070..=0x209F
            | 0x20A0..=0x20CF
            | 0x2100..=0x214F
            | 0x2150..=0x218F
            | 0x2190..=0x21FF
            | 0x2200..=0x22FF
            | 0x2300..=0x23FF
            | 0x2400..=0x243F
            | 0x2440..=0x245F
            | 0x2460..=0x24FF
            | 0x2500..=0x257F
            | 0x2580..=0x259F
            | 0x25A0..=0x25FF
            | 0x2600..=0x27BF
            | 0x27C0..=0x27EF
            | 0x27F0..=0x27FF
            | 0x2800..=0x28FF
            | 0x2900..=0x297F
            | 0x2980..=0x29FF
            | 0x2A00..=0x2AFF
            | 0x2B00..=0x2BFF
            | 0x2E80..=0x2FFF
            | 0x3000..=0x303F
            | 0xFE10..=0xFE1F
            | 0xFE30..=0xFE4F
            | 0xFF00..=0xFF65
            | 0x1F000..=0x1FAFF => Self::Common,
            0x0041..=0x005A | 0x0061..=0x007A | 0x00C0..=0x02AF | 0x1E00..=0x1EFF => Self::Latin,
            0x0370..=0x03FF | 0x1F00..=0x1FFF => Self::Greek,
            0x0400..=0x052F | 0x2DE0..=0x2DFF | 0xA640..=0xA69F => Self::Cyrillic,
            0x0530..=0x058F => Self::Armenian,
            0x0590..=0x05FF | 0xFB1D..=0xFB4F => Self::Hebrew,
            0x0600..=0x06FF
            | 0x0750..=0x077F
            | 0x08A0..=0x08FF
            | 0xFB50..=0xFDFF
            | 0xFE70..=0xFEFF => Self::Arabic,
            0x0900..=0x097F | 0xA8E0..=0xA8FF => Self::Devanagari,
            0x0980..=0x09FF => Self::Bengali,
            0x0A00..=0x0A7F => Self::Gurmukhi,
            0x0A80..=0x0AFF => Self::Gujarati,
            0x0B00..=0x0B7F => Self::Oriya,
            0x0B80..=0x0BFF => Self::Tamil,
            0x0C00..=0x0C7F => Self::Telugu,
            0x0C80..=0x0CFF => Self::Kannada,
            0x0D00..=0x0D7F => Self::Malayalam,
            0x0D80..=0x0DFF => Self::Sinhala,
            0x0E00..=0x0E7F => Self::Thai,
            0x0E80..=0x0EFF => Self::Lao,
            0x0F00..=0x0FFF => Self::Tibetan,
            0x1000..=0x109F | 0xAA60..=0xAA7F => Self::Myanmar,
            0x1100..=0x11FF | 0x3130..=0x318F | 0xA960..=0xA97F | 0xAC00..=0xD7FF => Self::Hangul,
            0x1200..=0x137F | 0x1380..=0x139F => Self::Ethiopic,
            0x10A0..=0x10FF | 0x2D00..=0x2D2F => Self::Georgian,
            0x1780..=0x17FF => Self::Khmer,
            0x1800..=0x18AF => Self::Mongolian,
            0x13A0..=0x13FF | 0xAB70..=0xABBF => Self::Cherokee,
            0x3040..=0x309F | 0x1B000..=0x1B0FF => Self::Hiragana,
            0x30A0..=0x30FF | 0x31F0..=0x31FF | 0xFF66..=0xFF9F => Self::Katakana,
            0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0x20000..=0x3FFFF => Self::Han,
            0xA000..=0xA4CF => Self::Yi,
            _ if codepoint.is_ascii_digit() => Self::Common,
            _ => Self::Other,
        }
    }
}
