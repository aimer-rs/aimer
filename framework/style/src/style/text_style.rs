use aimer_color::prelude::{Color, Colors};
pub use aimer_cupid::font::{FontFamily, FontStyle, FontWeight};

/// Controls the Unicode transformation applied to text before shaping.
///
/// Transformations operate on the source text's Unicode scalar values and may
/// change the number of rendered scalars. Selection and link code must retain
/// the source ranges when a transform is applied.
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextTransform {
    /// Paints the source text without changing its case.
    #[default]
    None,
    /// Converts each character using Unicode uppercase mapping.
    Uppercase,
    /// Converts each character using Unicode lowercase mapping.
    Lowercase,
    /// Uppercases the first cased character of each word.
    Capitalize,
}

/// Defines the distance between adjacent text baselines.
///
/// `Normal` uses the font's natural metrics. `Px` is an absolute logical
/// pixel line box, while `Factor` multiplies the resolved font size. Numeric
/// values must be finite; zero and negative resolved line boxes are invalid
/// during portable lowering.
#[derive(Default, Clone, Copy, Debug, PartialEq)]
pub enum LineHeight {
    /// Uses the natural ascent, descent, and font line gap.
    #[default]
    Normal,
    /// Uses an absolute logical-pixel line box, clamped to the natural glyph
    /// height when it is smaller than the glyphs.
    Px(f32),
    /// Uses a positive multiple of the largest font size on the line, again
    /// clamped to the natural glyph height.
    Factor(f32),
}

impl LineHeight {
    /// Creates an absolute logical-pixel line height.
    #[inline]
    pub const fn px(value: f32) -> Self {
        Self::Px(value)
    }

    /// Creates a line height relative to the resolved font size.
    #[inline]
    pub const fn factor(value: f32) -> Self {
        Self::Factor(value)
    }
}

/// A single glyph shadow painted behind a text run.
///
/// Text shadows affect painting only; their offset and blur do not change the
/// measured advance of the text. Values are logical pixels and are validated
/// when a style crosses the portable encoding seam.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextShadow {
    /// Horizontal shadow offset in logical pixels.
    pub offset_x: f32,
    /// Vertical shadow offset in logical pixels.
    pub offset_y: f32,
    /// Blur radius in logical pixels.
    pub blur: f32,
    /// Shadow color, including opacity.
    pub color: Color,
}

impl Default for TextShadow {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl TextShadow {
    /// Creates a zero-offset, unblurred, semi-transparent black shadow.
    #[inline]
    pub const fn new() -> Self {
        Self {
            offset_x: 0.0,
            offset_y: 0.0,
            blur: 0.0,
            color: Color::Rgba(0, 0, 0, 128),
        }
    }

    /// Sets the horizontal offset in logical pixels.
    #[inline]
    pub const fn offset_x(mut self, offset_x: f32) -> Self {
        self.offset_x = offset_x;
        self
    }

    /// Sets the vertical offset in logical pixels.
    #[inline]
    pub const fn offset_y(mut self, offset_y: f32) -> Self {
        self.offset_y = offset_y;
        self
    }

    /// Sets the blur radius in logical pixels.
    #[inline]
    pub const fn blur(mut self, blur: f32) -> Self {
        self.blur = blur;
        self
    }

    /// Sets the shadow color.
    #[inline]
    pub fn color(mut self, color: impl Into<Color>) -> Self {
        self.color = color.into();
        self
    }
}

/// The set of decoration lines to draw. Behaves like a small bit-set so several
/// lines (e.g. underline + line-through) can be combined without the awkward
/// `Combine(&'static [Self])` slice the old enum used.
#[allow(dead_code)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TextDecorationLine(u8);

impl Default for TextDecorationLine {
    fn default() -> Self {
        Self::NONE
    }
}

#[allow(dead_code)]
impl TextDecorationLine {
    pub const NONE: Self = Self(0);
    pub const UNDERLINE: Self = Self(1 << 0);
    pub const OVERLINE: Self = Self(1 << 1);
    pub const LINE_THROUGH: Self = Self(1 << 2);
    /// Slants the glyphs (synthetic oblique) rather than drawing a line. Kept
    /// in this bit-set so it combines with the real lines (e.g. underline +
    /// italic).
    pub const ITALIC: Self = Self(1 << 3);

    pub const fn bits(self) -> u8 {
        self.0
    }

    /// Reconstructs a decoration bit-set when all bits belong to the stable
    /// portable line vocabulary.
    #[doc(hidden)]
    pub const fn from_bits(bits: u8) -> Option<Self> {
        if bits & !0x0F == 0 {
            Some(Self(bits))
        } else {
            None
        }
    }

    /// True when every line in `other` is present in `self`.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn is_none(self) -> bool {
        self.0 == 0
    }
}

impl core::ops::BitOr for TextDecorationLine {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

/// The stroke style of a decoration line, mirroring the CSS
/// `text-decoration-style`.
#[allow(dead_code)]
#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
pub enum TextDecorationStyle {
    #[default]
    Solid,
    Double,
    Dotted,
    Dashed,
    Wavy,
}

impl TextDecorationStyle {
    /// Stable numeric id handed to the render engine (kept in sync with the
    /// `text_decoration.wgsl` shader's `style` switch).
    pub const fn id(self) -> u32 {
        match self {
            TextDecorationStyle::Solid => 0,
            TextDecorationStyle::Double => 1,
            TextDecorationStyle::Dotted => 2,
            TextDecorationStyle::Dashed => 3,
            TextDecorationStyle::Wavy => 4,
        }
    }
}

/// Full text-decoration description: which lines, their stroke style, an
/// optional dedicated color (falling back to the text color), an optional
/// thickness and a vertical offset. Replaces the old on/off `Underline`-only
/// enum.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextDecoration {
    pub line: TextDecorationLine,
    pub style: TextDecorationStyle,
    /// `None` inherits the text color.
    pub color: Option<Color>,
    /// `None` derives the thickness from the font size (~6%).
    pub thickness: Option<f32>,
    /// Extra vertical offset in logical pixels applied to the line (+ down).
    pub offset: f32,
}

#[allow(dead_code)]
#[allow(non_upper_case_globals)]
impl TextDecoration {
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    pub fn line(mut self, line: TextDecorationLine) -> Self {
        self.line = line;
        self
    }

    #[inline]
    pub fn style(mut self, style: TextDecorationStyle) -> Self {
        self.style = style;
        self
    }

    #[inline]
    pub fn color(mut self, color: impl Into<Color>) -> Self {
        self.color = Some(color.into());
        self
    }

    #[inline]
    pub fn thickness(mut self, thickness: f32) -> Self {
        self.thickness = Some(thickness);
        self
    }

    #[inline]
    pub fn offset(mut self, offset: f32) -> Self {
        self.offset = offset;
        self
    }

    /// No decoration. Kept as an associated constant so existing
    /// `TextDecoration::None` call sites keep working after the enum→struct
    /// change.
    pub const None: Self = Self {
        line: TextDecorationLine::NONE,
        style: TextDecorationStyle::Solid,
        color: None,
        thickness: None,
        offset: 0.0,
    };

    /// A plain solid underline (the previous default decoration). Kept as an
    /// associated constant for backward compatibility with
    /// `TextDecoration::Underline`.
    pub const Underline: Self = Self {
        line: TextDecorationLine::UNDERLINE,
        style: TextDecorationStyle::Solid,
        color: None,
        thickness: None,
        offset: 0.0,
    };

    pub const fn from_parts(line: TextDecorationLine, style: TextDecorationStyle) -> Self {
        Self {
            line,
            style,
            color: None,
            thickness: None,
            offset: 0.0,
        }
    }

    pub const fn with_color(mut self, color: Color) -> Self {
        self.color = Some(color);
        self
    }

    pub const fn with_thickness(mut self, thickness: f32) -> Self {
        self.thickness = Some(thickness);
        self
    }

    pub const fn with_offset(mut self, offset: f32) -> Self {
        self.offset = offset;
        self
    }
}

impl Default for TextDecoration {
    fn default() -> Self {
        Self::None
    }
}

#[allow(dead_code)]
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextAlign {
    #[default]
    TopLeft,
    TopCenter,
    TopRight,
    MidCenter,
    MidLeft,
    MidRight,
    BotLeft,
    BotCenter,
    BotRight,
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextStyle {
    /// Font size in logical pixels.
    pub font_size: u32,
    /// Font family used to shape the run.
    pub font_family: FontFamily,
    /// Font slant used to shape the run.
    pub font_style: FontStyle,
    /// Font weight used to shape the run.
    pub font_weight: FontWeight,
    /// Foreground paint color.
    pub color: Color,
    /// Optional inline background paint; it does not affect text metrics.
    pub background_color: Option<Color>,
    /// Behavior when the text exceeds its available width.
    pub text_overflow: TextOverflow,
    /// Lines painted around the glyph run. Decorations remain part of the
    /// canonical text style rather than being a separate widget property.
    pub text_decoration: TextDecoration,
    /// Unicode transformation applied before shaping.
    pub text_transform: TextTransform,
    /// Additional logical-pixel advance between adjacent rendered graphemes.
    pub letter_spacing: f32,
    /// Additional logical-pixel advance for whitespace word separators.
    pub word_spacing: f32,
    /// Optional glyph shadow. Shadow paint does not affect layout size.
    pub text_shadow: Option<TextShadow>,
}

impl TextStyle {
    pub const DEFAULT_TEXT_COLOR: Color = Color::Basic(Colors::Black);

    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    pub fn font_size(mut self, font_size: u32) -> Self {
        self.font_size = font_size;
        self
    }

    pub const fn font_family(mut self, font_family: FontFamily) -> Self {
        self.font_family = font_family;
        self
    }

    #[inline]
    pub fn font_style(mut self, font_style: FontStyle) -> Self {
        self.font_style = font_style;
        self
    }

    #[inline]
    pub fn font_weight(mut self, font_weight: FontWeight) -> Self {
        self.font_weight = font_weight;
        self
    }

    #[inline]
    pub fn color(mut self, color: impl Into<Color>) -> Self {
        self.color = color.into();
        self
    }

    /// Sets an inherited inline background. It does not add padding or change
    /// text metrics; transparent colors are not painted.
    pub const fn background_color(mut self, color: Color) -> Self {
        self.background_color = Some(color);
        self
    }

    #[inline]
    pub fn text_overflow(mut self, text_overflow: TextOverflow) -> Self {
        self.text_overflow = text_overflow;
        self
    }

    #[inline]
    pub fn text_decoration(mut self, text_decoration: TextDecoration) -> Self {
        self.text_decoration = text_decoration;
        self
    }

    /// Sets the Unicode transformation applied before shaping.
    #[inline]
    pub const fn text_transform(mut self, text_transform: TextTransform) -> Self {
        self.text_transform = text_transform;
        self
    }

    /// Sets the additional advance applied between adjacent glyphs.
    #[inline]
    pub const fn letter_spacing(mut self, letter_spacing: f32) -> Self {
        self.letter_spacing = letter_spacing;
        self
    }

    /// Sets the additional advance applied at word boundaries.
    #[inline]
    pub const fn word_spacing(mut self, word_spacing: f32) -> Self {
        self.word_spacing = word_spacing;
        self
    }

    /// Adds one glyph shadow behind this text style.
    #[inline]
    pub const fn text_shadow(mut self, text_shadow: TextShadow) -> Self {
        self.text_shadow = Some(text_shadow);
        self
    }

    /// Removes the glyph shadow from this text style.
    #[inline]
    pub const fn without_text_shadow(mut self) -> Self {
        self.text_shadow = None;
        self
    }
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            font_size: 13,
            font_family: FontFamily::SANS_SERIF,
            font_style: FontStyle::Normal,
            font_weight: FontWeight::Normal,
            color: Colors::Black.into(),
            background_color: None,
            text_overflow: TextOverflow::Clip,
            text_decoration: TextDecoration::None,
            text_transform: TextTransform::None,
            letter_spacing: 0.0,
            word_spacing: 0.0,
            text_shadow: None,
        }
    }
}

#[allow(dead_code)]
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextOverflow {
    Clip,
    Ellipsis,
    #[default]
    Wrap,
    Value(u32),
}

#[cfg(test)]
mod tests {
    use aimer_color::prelude::Color;

    use super::{
        FontFamily, FontWeight, LineHeight, TextDecorationLine, TextShadow, TextStyle,
        TextTransform,
    };

    #[test]
    fn text_style_selects_a_font_family() {
        let style = TextStyle::new().font_family(FontFamily::MONOSPACE);

        assert_eq!(style.font_family, FontFamily::MONOSPACE);
        assert_eq!(TextStyle::default().font_family, FontFamily::SANS_SERIF);
    }

    // Guards the ITALIC bit: it must combine with real lines (e.g. underline)
    // without colliding, since the text widget reads it via `contains` to decide
    // whether to shear the glyphs.
    #[test]
    fn italic_line_bit_combines() {
        let both = TextDecorationLine::UNDERLINE | TextDecorationLine::ITALIC;
        assert!(both.contains(TextDecorationLine::ITALIC));
        assert!(both.contains(TextDecorationLine::UNDERLINE));
        assert!(!both.contains(TextDecorationLine::LINE_THROUGH));
        // Italic is a distinct bit, not overlapping any decoration line.
        assert_ne!(TextDecorationLine::ITALIC.bits(), 0);
        assert_eq!(
            TextDecorationLine::ITALIC.bits() & TextDecorationLine::UNDERLINE.bits(),
            0
        );
        assert!(!TextDecorationLine::ITALIC.is_none());
    }

    // Guards the weight mapping and the ">= 600 renders bold" contract the
    // text pipeline relies on to trigger faux-bold double-strike.
    #[test]
    fn numeric_weight_and_bold_threshold() {
        assert_eq!(FontWeight::Normal.numeric(), 400);
        assert_eq!(FontWeight::Bold.numeric(), 700);
        assert_eq!(FontWeight::Value(650).numeric(), 650);

        // Normal / light weights stay below the bold threshold.
        assert!(FontWeight::Normal.numeric() < 600);
        assert!(FontWeight::Thin.numeric() < 600);
        assert!(FontWeight::VeryThin.numeric() < 600);
        // Bold and heavier cross it.
        assert!(FontWeight::Bold.numeric() >= 600);
        assert!(FontWeight::Bolder.numeric() >= 600);
    }

    #[test]
    fn text_style_defaults_keep_new_properties_inert() {
        let style = TextStyle::default();

        assert_eq!(style.text_transform, TextTransform::None);
        assert_eq!(style.letter_spacing, 0.0);
        assert_eq!(style.word_spacing, 0.0);
        assert_eq!(style.text_shadow, None);
        assert_eq!(LineHeight::default(), LineHeight::Normal);
    }

    #[test]
    fn text_style_builders_store_the_new_run_values() {
        let shadow = TextShadow::new()
            .offset_x(1.5)
            .offset_y(-2.0)
            .blur(3.0)
            .color(Color::Rgba(10, 20, 30, 40));
        let style = TextStyle::new()
            .text_transform(TextTransform::Uppercase)
            .letter_spacing(0.5)
            .word_spacing(-1.25)
            .text_shadow(shadow);

        assert_eq!(style.text_transform, TextTransform::Uppercase);
        assert_eq!(style.letter_spacing, 0.5);
        assert_eq!(style.word_spacing, -1.25);
        assert_eq!(style.text_shadow, Some(shadow));
        assert_eq!(LineHeight::Px(24.0), LineHeight::Px(24.0));
        assert_eq!(LineHeight::Factor(1.5), LineHeight::Factor(1.5));
    }
}
