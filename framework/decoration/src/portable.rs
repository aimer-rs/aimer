//! Portable (AWIR) encoding of [`BoxDecoration`].
//!
//! Implements the property traits that let a decoration cross the portable
//! widget boundary as one bounded, versioned blob: decoding is always available,
//! while encoding and validation need the `portable-guest` feature. The wire
//! layout is version one and must stay byte-compatible.

use crate::{
    BorderRadius, BorderSlice, BorderStyle, BoxBorder, BoxDecoration, BoxOutline, BoxShadow,
    ShadowSide,
};
use aimer_attribute::Dimension;
use aimer_color::prelude::Color;
use aimer_widget::portable::__anteros::{
    BOX_DECORATION_VALUE_MAXIMUM_ENCODED_BYTES, BOX_DECORATION_VALUE_NAME,
    BOX_DECORATION_VALUE_VERSION, PropertyId, PropertyValue, ValueSchemaMetadata,
    WidgetDocumentView,
};
#[cfg(feature = "portable-guest")]
use aimer_widget::portable::{
    PortableBuildContext, PortableBuildError, PortableEncodeProperty, PortableWidgetResource,
};
use aimer_widget::portable::{
    PortableMaterializeError, PortableMaterializeProperty, PortableProperty,
    PortablePropertyReflection,
};

// Preserve the version-one codec and canonical identity across the crate move.
const BOX_DECORATION_WIRE_VERSION: u8 = 1;
const BORDER_STYLE_NONE_TAG: u8 = 0;
const BORDER_STYLE_SOLID_TAG: u8 = 1;
const BORDER_STYLE_DASHED_TAG: u8 = 2;
const BORDER_STYLE_DOTTED_TAG: u8 = 3;
const DIMENSION_AUTO_TAG: u8 = 0;
const DIMENSION_PX_TAG: u8 = 1;
const DIMENSION_PERCENT_TAG: u8 = 2;
const SHADOW_SIDE_ALL_TAG: u8 = 0;
const SHADOW_SIDE_TOP_TAG: u8 = 1;
const SHADOW_SIDE_RIGHT_TAG: u8 = 2;
const SHADOW_SIDE_BOTTOM_TAG: u8 = 3;
const SHADOW_SIDE_LEFT_TAG: u8 = 4;
const SHADOW_SIDE_VERTICAL_TAG: u8 = 5;
const SHADOW_SIDE_HORIZONTAL_TAG: u8 = 6;
const SHADOW_SIDE_RANGE_TAG: u8 = 7;
const SHADOW_SIDE_TOP_LEFT_TAG: u8 = 8;
const SHADOW_SIDE_TOP_RIGHT_TAG: u8 = 9;
const SHADOW_SIDE_BOTTOM_RIGHT_TAG: u8 = 10;
const SHADOW_SIDE_BOTTOM_LEFT_TAG: u8 = 11;
const OPTIONAL_NONE_TAG: u8 = 0;
const OPTIONAL_SOME_TAG: u8 = 1;
const BOOLEAN_FALSE_TAG: u8 = 0;
const BOOLEAN_TRUE_TAG: u8 = 1;

impl PortableProperty for BoxDecoration {
    const REFLECTION: PortablePropertyReflection =
        PortablePropertyReflection::custom(ValueSchemaMetadata::from_canonical_name(
            BOX_DECORATION_VALUE_NAME,
            BOX_DECORATION_VALUE_VERSION,
            BOX_DECORATION_VALUE_MAXIMUM_ENCODED_BYTES,
        ));
}

impl PortableMaterializeProperty for BoxDecoration {
    /// Decodes the complete version-one decoration from one bounded AWIR blob.
    fn from_awir(
        document: &WidgetDocumentView<'_>,
        property: PropertyId,
        value: PropertyValue,
    ) -> Result<Self, PortableMaterializeError> {
        let blob = property_blob(document, property, value)?;
        let mut reader = WireReader::new(blob, property);
        reader.require_version(BOX_DECORATION_WIRE_VERSION)?;
        let decoration = Self {
            border: reader.border()?,
            outline: reader.outline()?,
            border_radius: reader.border_radius()?,
            box_shadow: reader.shadows()?,
            background_color: std::cell::Cell::new(reader.optional_color()?),
        };
        reader.finish()?;
        Ok(decoration)
    }
}

#[cfg(feature = "portable-guest")]
impl PortableEncodeProperty for BoxDecoration {
    /// Encodes the complete decoration using the version-one blob contract.
    fn encode_property(
        self,
        context: &mut PortableBuildContext,
    ) -> Result<PropertyValue, PortableBuildError> {
        let shadow_count = checked_shadow_count(self.box_shadow.len())?;
        validate_decoration(&self)?;

        let mut bytes = Vec::new();
        bytes.push(BOX_DECORATION_WIRE_VERSION);
        push_border(&mut bytes, &self.border);
        push_outline(&mut bytes, &self.outline);
        push_border_radius(&mut bytes, &self.border_radius);
        bytes.extend_from_slice(&shadow_count.to_le_bytes());
        for shadow in &self.box_shadow {
            push_shadow(&mut bytes, shadow);
        }
        push_optional_color(&mut bytes, self.background_color.get());
        context.push_owned_blob(bytes)
    }
}

#[cfg(feature = "portable-guest")]
fn checked_shadow_count(length: usize) -> Result<u32, PortableBuildError> {
    u32::try_from(length).map_err(|_| PortableBuildError::LengthOverflow {
        resource: PortableWidgetResource::BlobBytes,
        actual: length,
    })
}

#[cfg(feature = "portable-guest")]
fn validate_decoration(decoration: &BoxDecoration) -> Result<(), PortableBuildError> {
    for slice in [
        &decoration.border.left,
        &decoration.border.right,
        &decoration.border.top,
        &decoration.border.bottom,
        &decoration.outline.left,
        &decoration.outline.right,
        &decoration.outline.top,
        &decoration.outline.bottom,
    ] {
        validate_dimension(slice.stroke)?;
    }
    for dimension in [
        decoration.border_radius.top_left,
        decoration.border_radius.top_right,
        decoration.border_radius.bottom_right,
        decoration.border_radius.bottom_left,
    ] {
        validate_dimension(dimension)?;
    }
    for shadow in &decoration.box_shadow {
        for value in [shadow.offset_x, shadow.offset_y, shadow.blur, shadow.spread] {
            validate_finite(value)?;
        }
        if let ShadowSide::Range(start, end) = shadow.side {
            validate_finite(start)?;
            validate_finite(end)?;
        }
    }
    Ok(())
}

#[cfg(feature = "portable-guest")]
fn validate_dimension(dimension: Dimension) -> Result<(), PortableBuildError> {
    match dimension {
        Dimension::Auto => Ok(()),
        Dimension::Px(value) | Dimension::Percent(value) => validate_finite(value),
    }
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
fn push_border(bytes: &mut Vec<u8>, border: &BoxBorder) {
    push_border_slice(bytes, &border.left);
    push_border_slice(bytes, &border.right);
    push_border_slice(bytes, &border.top);
    push_border_slice(bytes, &border.bottom);
}

#[cfg(feature = "portable-guest")]
fn push_outline(bytes: &mut Vec<u8>, outline: &BoxOutline) {
    push_border_slice(bytes, &outline.left);
    push_border_slice(bytes, &outline.right);
    push_border_slice(bytes, &outline.top);
    push_border_slice(bytes, &outline.bottom);
}

#[cfg(feature = "portable-guest")]
fn push_border_slice(bytes: &mut Vec<u8>, slice: &BorderSlice) {
    bytes.push(match slice.style {
        BorderStyle::None => BORDER_STYLE_NONE_TAG,
        BorderStyle::Solid => BORDER_STYLE_SOLID_TAG,
        BorderStyle::Dashed => BORDER_STYLE_DASHED_TAG,
        BorderStyle::Dotted => BORDER_STYLE_DOTTED_TAG,
    });
    push_dimension(bytes, slice.stroke);
    push_color(bytes, slice.color);
}

#[cfg(feature = "portable-guest")]
fn push_border_radius(bytes: &mut Vec<u8>, radius: &BorderRadius) {
    push_dimension(bytes, radius.top_left);
    push_dimension(bytes, radius.top_right);
    push_dimension(bytes, radius.bottom_right);
    push_dimension(bytes, radius.bottom_left);
}

#[cfg(feature = "portable-guest")]
fn push_dimension(bytes: &mut Vec<u8>, dimension: Dimension) {
    match dimension {
        Dimension::Auto => {
            bytes.push(DIMENSION_AUTO_TAG);
            push_f32(bytes, 0.0);
        }
        Dimension::Px(value) => {
            bytes.push(DIMENSION_PX_TAG);
            push_f32(bytes, value);
        }
        Dimension::Percent(value) => {
            bytes.push(DIMENSION_PERCENT_TAG);
            push_f32(bytes, value);
        }
    }
}

#[cfg(feature = "portable-guest")]
fn push_shadow(bytes: &mut Vec<u8>, shadow: &BoxShadow) {
    push_f32(bytes, shadow.offset_x);
    push_f32(bytes, shadow.offset_y);
    push_f32(bytes, shadow.blur);
    push_f32(bytes, shadow.spread);
    push_color(bytes, shadow.color);
    bytes.push(u8::from(shadow.inset));
    push_shadow_side(bytes, shadow.side);
}

#[cfg(feature = "portable-guest")]
fn push_shadow_side(bytes: &mut Vec<u8>, side: ShadowSide) {
    match side {
        ShadowSide::All => bytes.push(SHADOW_SIDE_ALL_TAG),
        ShadowSide::Top => bytes.push(SHADOW_SIDE_TOP_TAG),
        ShadowSide::Right => bytes.push(SHADOW_SIDE_RIGHT_TAG),
        ShadowSide::Bottom => bytes.push(SHADOW_SIDE_BOTTOM_TAG),
        ShadowSide::Left => bytes.push(SHADOW_SIDE_LEFT_TAG),
        ShadowSide::Vertical => bytes.push(SHADOW_SIDE_VERTICAL_TAG),
        ShadowSide::Horizontal => bytes.push(SHADOW_SIDE_HORIZONTAL_TAG),
        ShadowSide::Range(start, end) => {
            bytes.push(SHADOW_SIDE_RANGE_TAG);
            push_f32(bytes, start);
            push_f32(bytes, end);
        }
        ShadowSide::TopLeft => bytes.push(SHADOW_SIDE_TOP_LEFT_TAG),
        ShadowSide::TopRight => bytes.push(SHADOW_SIDE_TOP_RIGHT_TAG),
        ShadowSide::BottomRight => bytes.push(SHADOW_SIDE_BOTTOM_RIGHT_TAG),
        ShadowSide::BottomLeft => bytes.push(SHADOW_SIDE_BOTTOM_LEFT_TAG),
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

    fn border(&mut self) -> Result<BoxBorder, PortableMaterializeError> {
        Ok(BoxBorder {
            left: self.border_slice()?,
            right: self.border_slice()?,
            top: self.border_slice()?,
            bottom: self.border_slice()?,
        })
    }

    fn outline(&mut self) -> Result<BoxOutline, PortableMaterializeError> {
        Ok(BoxOutline {
            left: self.border_slice()?,
            right: self.border_slice()?,
            top: self.border_slice()?,
            bottom: self.border_slice()?,
        })
    }

    fn border_slice(&mut self) -> Result<BorderSlice, PortableMaterializeError> {
        Ok(BorderSlice {
            style: match self.u8()? {
                BORDER_STYLE_NONE_TAG => BorderStyle::None,
                BORDER_STYLE_SOLID_TAG => BorderStyle::Solid,
                BORDER_STYLE_DASHED_TAG => BorderStyle::Dashed,
                BORDER_STYLE_DOTTED_TAG => BorderStyle::Dotted,
                _ => return Err(self.invalid_value()),
            },
            stroke: self.dimension()?,
            color: self.color()?,
        })
    }

    fn border_radius(&mut self) -> Result<BorderRadius, PortableMaterializeError> {
        Ok(BorderRadius {
            top_left: self.dimension()?,
            top_right: self.dimension()?,
            bottom_right: self.dimension()?,
            bottom_left: self.dimension()?,
        })
    }

    fn dimension(&mut self) -> Result<Dimension, PortableMaterializeError> {
        let tag = self.u8()?;
        let value = self.f32()?;
        match (tag, value) {
            (DIMENSION_AUTO_TAG, 0.0) => Ok(Dimension::Auto),
            (DIMENSION_PX_TAG, value) => Ok(Dimension::Px(value)),
            (DIMENSION_PERCENT_TAG, value) => Ok(Dimension::Percent(value)),
            _ => Err(self.invalid_value()),
        }
    }

    fn shadows(&mut self) -> Result<Vec<BoxShadow>, PortableMaterializeError> {
        const MINIMUM_SHADOW_BYTES: usize = 22;

        let count = usize::try_from(self.u32()?).map_err(|_| self.invalid_value())?;
        if count > self.remaining() / MINIMUM_SHADOW_BYTES {
            return Err(self.invalid_value());
        }
        let mut shadows = Vec::with_capacity(count);
        for _ in 0..count {
            shadows.push(self.shadow()?);
        }
        Ok(shadows)
    }

    fn shadow(&mut self) -> Result<BoxShadow, PortableMaterializeError> {
        Ok(BoxShadow {
            offset_x: self.f32()?,
            offset_y: self.f32()?,
            blur: self.f32()?,
            spread: self.f32()?,
            color: self.color()?,
            inset: self.boolean()?,
            side: self.shadow_side()?,
        })
    }

    fn shadow_side(&mut self) -> Result<ShadowSide, PortableMaterializeError> {
        match self.u8()? {
            SHADOW_SIDE_ALL_TAG => Ok(ShadowSide::All),
            SHADOW_SIDE_TOP_TAG => Ok(ShadowSide::Top),
            SHADOW_SIDE_RIGHT_TAG => Ok(ShadowSide::Right),
            SHADOW_SIDE_BOTTOM_TAG => Ok(ShadowSide::Bottom),
            SHADOW_SIDE_LEFT_TAG => Ok(ShadowSide::Left),
            SHADOW_SIDE_VERTICAL_TAG => Ok(ShadowSide::Vertical),
            SHADOW_SIDE_HORIZONTAL_TAG => Ok(ShadowSide::Horizontal),
            SHADOW_SIDE_RANGE_TAG => Ok(ShadowSide::Range(self.f32()?, self.f32()?)),
            SHADOW_SIDE_TOP_LEFT_TAG => Ok(ShadowSide::TopLeft),
            SHADOW_SIDE_TOP_RIGHT_TAG => Ok(ShadowSide::TopRight),
            SHADOW_SIDE_BOTTOM_RIGHT_TAG => Ok(ShadowSide::BottomRight),
            SHADOW_SIDE_BOTTOM_LEFT_TAG => Ok(ShadowSide::BottomLeft),
            _ => Err(self.invalid_value()),
        }
    }

    fn optional_color(&mut self) -> Result<Option<Color>, PortableMaterializeError> {
        match self.u8()? {
            OPTIONAL_NONE_TAG => Ok(None),
            OPTIONAL_SOME_TAG => self.color().map(Some),
            _ => Err(self.invalid_value()),
        }
    }

    fn color(&mut self) -> Result<Color, PortableMaterializeError> {
        self.u32().map(Color::from_primitive)
    }

    fn boolean(&mut self) -> Result<bool, PortableMaterializeError> {
        match self.u8()? {
            BOOLEAN_FALSE_TAG => Ok(false),
            BOOLEAN_TRUE_TAG => Ok(true),
            _ => Err(self.invalid_value()),
        }
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

    fn finish(self) -> Result<(), PortableMaterializeError> {
        if self.cursor == self.bytes.len() {
            Ok(())
        } else {
            Err(self.invalid_value())
        }
    }

    #[inline]
    fn remaining(&self) -> usize {
        self.bytes.len() - self.cursor
    }

    #[inline]
    const fn invalid_value(&self) -> PortableMaterializeError {
        PortableMaterializeError::InvalidPropertyValue {
            property: self.property,
        }
    }
}
