use aimer_widget::portable::__anteros::{
    LAYOUT_SPACING_VALUE_MAXIMUM_ENCODED_BYTES, LAYOUT_SPACING_VALUE_NAME,
    LAYOUT_SPACING_VALUE_VERSION, LINE_HEIGHT_VALUE_MAXIMUM_ENCODED_BYTES, LINE_HEIGHT_VALUE_NAME,
    LINE_HEIGHT_VALUE_VERSION, PropertyId, PropertyValue, TEXT_STYLE_VALUE_MAXIMUM_ENCODED_BYTES,
    TEXT_STYLE_VALUE_NAME, TEXT_STYLE_VALUE_VERSION, ValueSchemaMetadata, WidgetDocumentView,
};
#[cfg(feature = "portable-guest")]
use aimer_widget::portable::{PortableBuildContext, PortableBuildError, PortableEncodeProperty};
use aimer_widget::portable::{
    PortableMaterializeError, PortableMaterializeProperty, PortableProperty,
    PortablePropertyConversion, PortablePropertyReflection,
};

use super::layout_spacing::{LayoutSpacing, Spacing};
use super::text_style::{
    FontFamily, FontStyle, FontWeight, LineHeight, TextAlign, TextDecoration, TextDecorationLine,
    TextDecorationStyle, TextOverflow, TextShadow, TextStyle, TextTransform,
};
use aimer_color::prelude::Color;

impl PortableProperty for TextAlign {
    const REFLECTION: PortablePropertyReflection = PortablePropertyReflection::new(
        aimer_widget::portable::__anteros::PropertyValueKind::I64,
        PortablePropertyConversion::SignedInteger {
            minimum: 0,
            maximum: 8,
        },
    );
}

impl PortableMaterializeProperty for TextAlign {
    fn from_awir(
        _document: &WidgetDocumentView<'_>,
        property: PropertyId,
        value: PropertyValue,
    ) -> Result<Self, PortableMaterializeError> {
        let PropertyValue::I64(value) = value else {
            return Err(PortableMaterializeError::InvalidPropertyType { property });
        };
        match value {
            0 => Ok(Self::TopLeft),
            1 => Ok(Self::TopCenter),
            2 => Ok(Self::TopRight),
            3 => Ok(Self::MidCenter),
            4 => Ok(Self::MidLeft),
            5 => Ok(Self::MidRight),
            6 => Ok(Self::BotLeft),
            7 => Ok(Self::BotCenter),
            8 => Ok(Self::BotRight),
            _ => Err(PortableMaterializeError::InvalidPropertyValue { property }),
        }
    }
}

#[cfg(feature = "portable-guest")]
impl PortableEncodeProperty for TextAlign {
    fn encode_property(
        self,
        _context: &mut PortableBuildContext,
    ) -> Result<PropertyValue, PortableBuildError> {
        Ok(PropertyValue::I64(match self {
            Self::TopLeft => 0,
            Self::TopCenter => 1,
            Self::TopRight => 2,
            Self::MidCenter => 3,
            Self::MidLeft => 4,
            Self::MidRight => 5,
            Self::BotLeft => 6,
            Self::BotCenter => 7,
            Self::BotRight => 8,
        }))
    }
}

// These bytes are part of the value codec contracts. A decoder must reject a
// different version until an explicit migration path is added.
const LAYOUT_SPACING_WIRE_VERSION: u8 = 1;
const TEXT_STYLE_WIRE_VERSION: u8 = 2;
const LINE_HEIGHT_WIRE_VERSION: u8 = 1;

// Canonical enum tags. Do not renumber an existing tag; append a new tag and
// advance the value codec version when the old meaning cannot be preserved.
const SPACING_NONE_TAG: u8 = 0;
const SPACING_PX_TAG: u8 = 1;
const SPACING_PERCENT_TAG: u8 = 2;
const OPTIONAL_NONE_TAG: u8 = 0;
const OPTIONAL_SOME_TAG: u8 = 1;
const FONT_STYLE_NORMAL_TAG: u8 = 0;
const FONT_STYLE_ITALIC_TAG: u8 = 1;
const FONT_STYLE_OBLIQUE_TAG: u8 = 2;
const FONT_STYLE_OBLIQUE_DEG_TAG: u8 = 3;
const FONT_WEIGHT_VERY_THIN_TAG: u8 = 0;
const FONT_WEIGHT_THIN_TAG: u8 = 1;
const FONT_WEIGHT_NORMAL_TAG: u8 = 2;
const FONT_WEIGHT_BOLD_TAG: u8 = 3;
const FONT_WEIGHT_BOLDER_TAG: u8 = 4;
const FONT_WEIGHT_VALUE_TAG: u8 = 5;
const TEXT_OVERFLOW_CLIP_TAG: u8 = 0;
const TEXT_OVERFLOW_ELLIPSIS_TAG: u8 = 1;
const TEXT_OVERFLOW_WRAP_TAG: u8 = 2;
const TEXT_OVERFLOW_VALUE_TAG: u8 = 3;
const TEXT_DECORATION_SOLID_TAG: u8 = 0;
const TEXT_DECORATION_DOUBLE_TAG: u8 = 1;
const TEXT_DECORATION_DOTTED_TAG: u8 = 2;
const TEXT_DECORATION_DASHED_TAG: u8 = 3;
const TEXT_DECORATION_WAVY_TAG: u8 = 4;
const TEXT_TRANSFORM_NONE_TAG: u8 = 0;
const TEXT_TRANSFORM_UPPERCASE_TAG: u8 = 1;
const TEXT_TRANSFORM_LOWERCASE_TAG: u8 = 2;
const TEXT_TRANSFORM_CAPITALIZE_TAG: u8 = 3;
const LINE_HEIGHT_NORMAL_TAG: u8 = 0;
const LINE_HEIGHT_PX_TAG: u8 = 1;
const LINE_HEIGHT_FACTOR_TAG: u8 = 2;

impl PortableProperty for LayoutSpacing {
    const REFLECTION: PortablePropertyReflection =
        PortablePropertyReflection::custom(ValueSchemaMetadata::from_canonical_name(
            LAYOUT_SPACING_VALUE_NAME,
            LAYOUT_SPACING_VALUE_VERSION,
            LAYOUT_SPACING_VALUE_MAXIMUM_ENCODED_BYTES,
        ));
}

impl PortableMaterializeProperty for LayoutSpacing {
    /// Decodes four canonical spacing values in top, bottom, left, right order.
    fn from_awir(
        document: &WidgetDocumentView<'_>,
        property: PropertyId,
        value: PropertyValue,
    ) -> Result<Self, PortableMaterializeError> {
        let blob = property_blob(document, property, value)?;
        let mut reader = WireReader::new(blob, property);
        reader.require_version(LAYOUT_SPACING_WIRE_VERSION)?;
        let spacing = Self {
            top: reader.spacing()?,
            bottom: reader.spacing()?,
            left: reader.spacing()?,
            right: reader.spacing()?,
        };
        reader.finish()?;
        Ok(spacing)
    }
}

#[cfg(feature = "portable-guest")]
impl PortableEncodeProperty for LayoutSpacing {
    /// Encodes four stable spacing records in top, bottom, left, right order.
    fn encode_property(
        self,
        context: &mut PortableBuildContext,
    ) -> Result<PropertyValue, PortableBuildError> {
        let mut bytes = Vec::with_capacity(LAYOUT_SPACING_VALUE_MAXIMUM_ENCODED_BYTES as usize);
        bytes.push(LAYOUT_SPACING_WIRE_VERSION);
        push_spacing(&mut bytes, self.top);
        push_spacing(&mut bytes, self.bottom);
        push_spacing(&mut bytes, self.left);
        push_spacing(&mut bytes, self.right);
        context.push_owned_blob(bytes)
    }
}

#[cfg(feature = "portable-guest")]
fn push_spacing(bytes: &mut Vec<u8>, spacing: Spacing) {
    match spacing {
        Spacing::None => {
            bytes.push(SPACING_NONE_TAG);
            bytes.extend_from_slice(&0_u32.to_le_bytes());
        }
        Spacing::Px(value) => {
            bytes.push(SPACING_PX_TAG);
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        Spacing::Percent(value) => {
            bytes.push(SPACING_PERCENT_TAG);
            bytes.extend_from_slice(&value.to_le_bytes());
        }
    }
}

impl PortableProperty for TextStyle {
    const REFLECTION: PortablePropertyReflection =
        PortablePropertyReflection::custom(ValueSchemaMetadata::from_canonical_name(
            TEXT_STYLE_VALUE_NAME,
            TEXT_STYLE_VALUE_VERSION,
            TEXT_STYLE_VALUE_MAXIMUM_ENCODED_BYTES,
        ));
}

impl PortableMaterializeProperty for TextStyle {
    /// Decodes the version-one or version-two text style from one bounded AWIR
    /// blob. Version one remains accepted with the new properties at defaults.
    fn from_awir(
        document: &WidgetDocumentView<'_>,
        property: PropertyId,
        value: PropertyValue,
    ) -> Result<Self, PortableMaterializeError> {
        let blob = property_blob(document, property, value)?;
        let mut reader = WireReader::new(blob, property);
        let version = reader.u8()?;
        if version != 1 && version != TEXT_STYLE_WIRE_VERSION {
            return Err(reader.invalid_value());
        }
        let style = TextStyle {
            font_size: reader.u32()?,
            font_family: FontFamily::from_raw(reader.u64()?),
            font_style: reader.font_style()?,
            font_weight: reader.font_weight()?,
            color: reader.color()?,
            background_color: reader.optional_color()?,
            text_overflow: reader.text_overflow()?,
            text_decoration: reader.text_decoration()?,
            text_transform: if version == 1 {
                TextTransform::None
            } else {
                reader.text_transform()?
            },
            letter_spacing: if version == 1 { 0.0 } else { reader.f32()? },
            word_spacing: if version == 1 { 0.0 } else { reader.f32()? },
            text_shadow: if version == 1 {
                None
            } else {
                reader.optional_text_shadow()?
            },
        };
        reader.finish()?;
        Ok(style)
    }
}

#[cfg(feature = "portable-guest")]
impl PortableEncodeProperty for TextStyle {
    /// Encodes all text metrics, paint, overflow, and decoration fields in a
    /// bounded, versioned payload.
    fn encode_property(
        self,
        context: &mut PortableBuildContext,
    ) -> Result<PropertyValue, PortableBuildError> {
        validate_text_style(&self)?;

        let mut bytes = Vec::with_capacity(TEXT_STYLE_VALUE_MAXIMUM_ENCODED_BYTES as usize);
        bytes.push(TEXT_STYLE_WIRE_VERSION);
        bytes.extend_from_slice(&self.font_size.to_le_bytes());
        bytes.extend_from_slice(&self.font_family.raw().to_le_bytes());
        push_font_style(&mut bytes, self.font_style);
        push_font_weight(&mut bytes, self.font_weight);
        push_color(&mut bytes, self.color);
        push_optional_color(&mut bytes, self.background_color);
        push_text_overflow(&mut bytes, self.text_overflow);
        bytes.push(self.text_decoration.line.bits());
        push_text_decoration_style(&mut bytes, self.text_decoration.style);
        push_optional_color(&mut bytes, self.text_decoration.color);
        push_optional_f32(&mut bytes, self.text_decoration.thickness);
        push_f32(&mut bytes, self.text_decoration.offset);
        push_text_transform(&mut bytes, self.text_transform);
        push_f32(&mut bytes, self.letter_spacing);
        push_f32(&mut bytes, self.word_spacing);
        push_optional_text_shadow(&mut bytes, self.text_shadow);
        context.push_owned_blob(bytes)
    }
}

#[cfg(feature = "portable-guest")]
fn validate_text_style(style: &TextStyle) -> Result<(), PortableBuildError> {
    if let Some(thickness) = style.text_decoration.thickness {
        validate_finite(thickness)?;
    }
    validate_finite(style.text_decoration.offset)?;
    validate_finite(style.letter_spacing)?;
    validate_finite(style.word_spacing)?;
    if let Some(shadow) = style.text_shadow {
        validate_finite(shadow.offset_x)?;
        validate_finite(shadow.offset_y)?;
        validate_finite(shadow.blur)?;
        if shadow.blur < 0.0 {
            return Err(PortableBuildError::InvalidPropertyValue {
                rust_type: "TextStyle::text_shadow.blur",
            });
        }
    }
    Ok(())
}

impl PortableProperty for LineHeight {
    const REFLECTION: PortablePropertyReflection =
        PortablePropertyReflection::custom(ValueSchemaMetadata::from_canonical_name(
            LINE_HEIGHT_VALUE_NAME,
            LINE_HEIGHT_VALUE_VERSION,
            LINE_HEIGHT_VALUE_MAXIMUM_ENCODED_BYTES,
        ));
}

impl PortableMaterializeProperty for LineHeight {
    fn from_awir(
        document: &WidgetDocumentView<'_>,
        property: PropertyId,
        value: PropertyValue,
    ) -> Result<Self, PortableMaterializeError> {
        let blob = property_blob(document, property, value)?;
        let mut reader = WireReader::new(blob, property);
        reader.require_version(LINE_HEIGHT_WIRE_VERSION)?;
        let line_height = match reader.u8()? {
            LINE_HEIGHT_NORMAL_TAG => Self::Normal,
            LINE_HEIGHT_PX_TAG => {
                let value = reader.f32()?;
                if value <= 0.0 {
                    return Err(reader.invalid_value());
                }
                Self::Px(value)
            }
            LINE_HEIGHT_FACTOR_TAG => {
                let value = reader.f32()?;
                if value <= 0.0 {
                    return Err(reader.invalid_value());
                }
                Self::Factor(value)
            }
            _ => return Err(reader.invalid_value()),
        };
        reader.finish()?;
        Ok(line_height)
    }
}

#[cfg(feature = "portable-guest")]
impl PortableEncodeProperty for LineHeight {
    fn encode_property(
        self,
        context: &mut PortableBuildContext,
    ) -> Result<PropertyValue, PortableBuildError> {
        let mut bytes = Vec::with_capacity(LINE_HEIGHT_VALUE_MAXIMUM_ENCODED_BYTES as usize);
        bytes.push(LINE_HEIGHT_WIRE_VERSION);
        match self {
            Self::Normal => bytes.push(LINE_HEIGHT_NORMAL_TAG),
            Self::Px(value) => {
                validate_positive_finite(value)?;
                bytes.push(LINE_HEIGHT_PX_TAG);
                push_f32(&mut bytes, value);
            }
            Self::Factor(value) => {
                validate_positive_finite(value)?;
                bytes.push(LINE_HEIGHT_FACTOR_TAG);
                push_f32(&mut bytes, value);
            }
        }
        context.push_owned_blob(bytes)
    }
}

#[cfg(feature = "portable-guest")]
fn validate_positive_finite(value: f32) -> Result<(), PortableBuildError> {
    validate_finite(value)?;
    if value > 0.0 {
        Ok(())
    } else {
        Err(PortableBuildError::InvalidPropertyValue {
            rust_type: "aimer_style::LineHeight",
        })
    }
}

#[cfg(feature = "portable-guest")]
fn push_text_transform(bytes: &mut Vec<u8>, transform: TextTransform) {
    bytes.push(match transform {
        TextTransform::None => TEXT_TRANSFORM_NONE_TAG,
        TextTransform::Uppercase => TEXT_TRANSFORM_UPPERCASE_TAG,
        TextTransform::Lowercase => TEXT_TRANSFORM_LOWERCASE_TAG,
        TextTransform::Capitalize => TEXT_TRANSFORM_CAPITALIZE_TAG,
    });
}

#[cfg(feature = "portable-guest")]
fn push_optional_text_shadow(bytes: &mut Vec<u8>, shadow: Option<TextShadow>) {
    match shadow {
        None => bytes.push(OPTIONAL_NONE_TAG),
        Some(shadow) => {
            bytes.push(OPTIONAL_SOME_TAG);
            push_f32(bytes, shadow.offset_x);
            push_f32(bytes, shadow.offset_y);
            push_f32(bytes, shadow.blur);
            push_color(bytes, shadow.color);
        }
    }
}

#[cfg(feature = "portable-guest")]
fn push_font_style(bytes: &mut Vec<u8>, style: FontStyle) {
    match style {
        FontStyle::Normal => bytes.push(FONT_STYLE_NORMAL_TAG),
        FontStyle::Italic => bytes.push(FONT_STYLE_ITALIC_TAG),
        FontStyle::Oblique => bytes.push(FONT_STYLE_OBLIQUE_TAG),
        FontStyle::ObliqueDeg(degrees) => {
            bytes.push(FONT_STYLE_OBLIQUE_DEG_TAG);
            bytes.extend_from_slice(&degrees.to_le_bytes());
        }
    }
}

#[cfg(feature = "portable-guest")]
fn push_font_weight(bytes: &mut Vec<u8>, weight: FontWeight) {
    match weight {
        FontWeight::VeryThin => bytes.push(FONT_WEIGHT_VERY_THIN_TAG),
        FontWeight::Thin => bytes.push(FONT_WEIGHT_THIN_TAG),
        FontWeight::Normal => bytes.push(FONT_WEIGHT_NORMAL_TAG),
        FontWeight::Bold => bytes.push(FONT_WEIGHT_BOLD_TAG),
        FontWeight::Bolder => bytes.push(FONT_WEIGHT_BOLDER_TAG),
        FontWeight::Value(value) => {
            bytes.push(FONT_WEIGHT_VALUE_TAG);
            bytes.extend_from_slice(&value.to_le_bytes());
        }
    }
}

#[cfg(feature = "portable-guest")]
fn push_text_overflow(bytes: &mut Vec<u8>, overflow: TextOverflow) {
    match overflow {
        TextOverflow::Clip => bytes.push(TEXT_OVERFLOW_CLIP_TAG),
        TextOverflow::Ellipsis => bytes.push(TEXT_OVERFLOW_ELLIPSIS_TAG),
        TextOverflow::Wrap => bytes.push(TEXT_OVERFLOW_WRAP_TAG),
        TextOverflow::Value(value) => {
            bytes.push(TEXT_OVERFLOW_VALUE_TAG);
            bytes.extend_from_slice(&value.to_le_bytes());
        }
    }
}

#[cfg(feature = "portable-guest")]
fn push_text_decoration_style(bytes: &mut Vec<u8>, style: TextDecorationStyle) {
    bytes.push(match style {
        TextDecorationStyle::Solid => TEXT_DECORATION_SOLID_TAG,
        TextDecorationStyle::Double => TEXT_DECORATION_DOUBLE_TAG,
        TextDecorationStyle::Dotted => TEXT_DECORATION_DOTTED_TAG,
        TextDecorationStyle::Dashed => TEXT_DECORATION_DASHED_TAG,
        TextDecorationStyle::Wavy => TEXT_DECORATION_WAVY_TAG,
    });
}

#[cfg(feature = "portable-guest")]
#[inline]
fn validate_finite(value: f32) -> Result<(), PortableBuildError> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(PortableBuildError::NonFiniteFloat)
    }
}

#[cfg(feature = "portable-guest")]
fn push_optional_color(bytes: &mut Vec<u8>, color: Option<Color>) {
    match color {
        None => bytes.push(OPTIONAL_NONE_TAG),
        Some(color) => {
            bytes.push(OPTIONAL_SOME_TAG);
            push_color(bytes, color);
        }
    }
}

#[cfg(feature = "portable-guest")]
fn push_optional_f32(bytes: &mut Vec<u8>, value: Option<f32>) {
    match value {
        None => bytes.push(OPTIONAL_NONE_TAG),
        Some(value) => {
            bytes.push(OPTIONAL_SOME_TAG);
            push_f32(bytes, value);
        }
    }
}

#[cfg(feature = "portable-guest")]
#[inline]
fn push_color(bytes: &mut Vec<u8>, color: Color) {
    // This custom style codec uses the stable packed Color representation that
    // its host decoder accepts; it is intentionally separate from inline RGBA.
    bytes.extend_from_slice(&color.as_u32().to_le_bytes());
}

#[cfg(feature = "portable-guest")]
#[inline]
fn push_f32(bytes: &mut Vec<u8>, value: f32) {
    bytes.extend_from_slice(&value.to_bits().to_le_bytes());
}

fn property_blob<'a>(
    document: &WidgetDocumentView<'a>,
    property: PropertyId,
    value: PropertyValue,
) -> Result<&'a [u8], PortableMaterializeError> {
    let PropertyValue::BlobRef(index) = value else {
        return Err(PortableMaterializeError::InvalidPropertyType { property });
    };
    document
        .blob(index)
        .ok_or(PortableMaterializeError::InvalidPropertyReference { property, index })
}

struct WireReader<'a> {
    bytes: &'a [u8],
    cursor: usize,
    property: PropertyId,
}

impl<'a> WireReader<'a> {
    #[inline]
    const fn new(bytes: &'a [u8], property: PropertyId) -> Self {
        Self {
            bytes,
            cursor: 0,
            property,
        }
    }

    fn require_version(&mut self, expected: u8) -> Result<(), PortableMaterializeError> {
        if self.u8()? == expected {
            Ok(())
        } else {
            Err(self.invalid_value())
        }
    }

    fn spacing(&mut self) -> Result<Spacing, PortableMaterializeError> {
        let tag = self.u8()?;
        let value = self.u32()?;
        match (tag, value) {
            (SPACING_NONE_TAG, 0) => Ok(Spacing::None),
            (SPACING_PX_TAG, value) => Ok(Spacing::Px(value)),
            (SPACING_PERCENT_TAG, value) => Ok(Spacing::Percent(value)),
            _ => Err(self.invalid_value()),
        }
    }

    fn font_style(&mut self) -> Result<FontStyle, PortableMaterializeError> {
        match self.u8()? {
            FONT_STYLE_NORMAL_TAG => Ok(FontStyle::Normal),
            FONT_STYLE_ITALIC_TAG => Ok(FontStyle::Italic),
            FONT_STYLE_OBLIQUE_TAG => Ok(FontStyle::Oblique),
            FONT_STYLE_OBLIQUE_DEG_TAG => Ok(FontStyle::ObliqueDeg(self.i32()?)),
            _ => Err(self.invalid_value()),
        }
    }

    fn font_weight(&mut self) -> Result<FontWeight, PortableMaterializeError> {
        match self.u8()? {
            FONT_WEIGHT_VERY_THIN_TAG => Ok(FontWeight::VeryThin),
            FONT_WEIGHT_THIN_TAG => Ok(FontWeight::Thin),
            FONT_WEIGHT_NORMAL_TAG => Ok(FontWeight::Normal),
            FONT_WEIGHT_BOLD_TAG => Ok(FontWeight::Bold),
            FONT_WEIGHT_BOLDER_TAG => Ok(FontWeight::Bolder),
            FONT_WEIGHT_VALUE_TAG => Ok(FontWeight::Value(self.u32()?)),
            _ => Err(self.invalid_value()),
        }
    }

    fn text_overflow(&mut self) -> Result<TextOverflow, PortableMaterializeError> {
        match self.u8()? {
            TEXT_OVERFLOW_CLIP_TAG => Ok(TextOverflow::Clip),
            TEXT_OVERFLOW_ELLIPSIS_TAG => Ok(TextOverflow::Ellipsis),
            TEXT_OVERFLOW_WRAP_TAG => Ok(TextOverflow::Wrap),
            TEXT_OVERFLOW_VALUE_TAG => Ok(TextOverflow::Value(self.u32()?)),
            _ => Err(self.invalid_value()),
        }
    }

    fn text_transform(&mut self) -> Result<TextTransform, PortableMaterializeError> {
        match self.u8()? {
            TEXT_TRANSFORM_NONE_TAG => Ok(TextTransform::None),
            TEXT_TRANSFORM_UPPERCASE_TAG => Ok(TextTransform::Uppercase),
            TEXT_TRANSFORM_LOWERCASE_TAG => Ok(TextTransform::Lowercase),
            TEXT_TRANSFORM_CAPITALIZE_TAG => Ok(TextTransform::Capitalize),
            _ => Err(self.invalid_value()),
        }
    }

    fn text_decoration(&mut self) -> Result<TextDecoration, PortableMaterializeError> {
        let line = TextDecorationLine::from_bits(self.u8()?).ok_or_else(|| self.invalid_value())?;
        let style = match self.u8()? {
            TEXT_DECORATION_SOLID_TAG => TextDecorationStyle::Solid,
            TEXT_DECORATION_DOUBLE_TAG => TextDecorationStyle::Double,
            TEXT_DECORATION_DOTTED_TAG => TextDecorationStyle::Dotted,
            TEXT_DECORATION_DASHED_TAG => TextDecorationStyle::Dashed,
            TEXT_DECORATION_WAVY_TAG => TextDecorationStyle::Wavy,
            _ => return Err(self.invalid_value()),
        };
        Ok(TextDecoration {
            line,
            style,
            color: self.optional_color()?,
            thickness: self.optional_f32()?,
            offset: self.f32()?,
        })
    }

    fn optional_color(&mut self) -> Result<Option<Color>, PortableMaterializeError> {
        match self.u8()? {
            OPTIONAL_NONE_TAG => Ok(None),
            OPTIONAL_SOME_TAG => self.color().map(Some),
            _ => Err(self.invalid_value()),
        }
    }

    fn optional_f32(&mut self) -> Result<Option<f32>, PortableMaterializeError> {
        match self.u8()? {
            OPTIONAL_NONE_TAG => Ok(None),
            OPTIONAL_SOME_TAG => self.f32().map(Some),
            _ => Err(self.invalid_value()),
        }
    }

    fn optional_text_shadow(&mut self) -> Result<Option<TextShadow>, PortableMaterializeError> {
        match self.u8()? {
            OPTIONAL_NONE_TAG => Ok(None),
            OPTIONAL_SOME_TAG => {
                let offset_x = self.f32()?;
                let offset_y = self.f32()?;
                let blur = self.f32()?;
                if blur < 0.0 {
                    return Err(self.invalid_value());
                }
                Ok(Some(TextShadow {
                    offset_x,
                    offset_y,
                    blur,
                    color: self.color()?,
                }))
            }
            _ => Err(self.invalid_value()),
        }
    }

    fn color(&mut self) -> Result<Color, PortableMaterializeError> {
        self.u32().map(Color::from_primitive)
    }

    fn f32(&mut self) -> Result<f32, PortableMaterializeError> {
        let value = f32::from_bits(self.u32()?);
        if value.is_finite() {
            Ok(value)
        } else {
            Err(self.invalid_value())
        }
    }

    fn u8(&mut self) -> Result<u8, PortableMaterializeError> {
        let value = *self
            .bytes
            .get(self.cursor)
            .ok_or_else(|| self.invalid_value())?;
        self.cursor += 1;
        Ok(value)
    }

    fn u32(&mut self) -> Result<u32, PortableMaterializeError> {
        let end = self
            .cursor
            .checked_add(4)
            .ok_or_else(|| self.invalid_value())?;
        let bytes = self
            .bytes
            .get(self.cursor..end)
            .ok_or_else(|| self.invalid_value())?;
        self.cursor = end;
        Ok(u32::from_le_bytes(
            bytes.try_into().expect("four bytes were checked"),
        ))
    }

    fn i32(&mut self) -> Result<i32, PortableMaterializeError> {
        Ok(self.u32()? as i32)
    }

    fn u64(&mut self) -> Result<u64, PortableMaterializeError> {
        let end = self
            .cursor
            .checked_add(8)
            .ok_or_else(|| self.invalid_value())?;
        let bytes = self
            .bytes
            .get(self.cursor..end)
            .ok_or_else(|| self.invalid_value())?;
        self.cursor = end;
        Ok(u64::from_le_bytes(
            bytes.try_into().expect("eight bytes were checked"),
        ))
    }

    fn finish(self) -> Result<(), PortableMaterializeError> {
        if self.cursor == self.bytes.len() {
            Ok(())
        } else {
            Err(self.invalid_value())
        }
    }

    #[inline]
    const fn invalid_value(&self) -> PortableMaterializeError {
        PortableMaterializeError::InvalidPropertyValue {
            property: self.property,
        }
    }
}

#[cfg(test)]
#[path = "portable_materialize_tests.rs"]
mod tests;
