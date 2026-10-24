//! Bounded portable projections for context-menu values.
//!
//! The native menu keeps callbacks, modal handles, and platform placement
//! state outside AWIR. Labels, enabled state, shape, dismissal policy, and the
//! complete visual description are ordinary versioned values, so a guest can
//! describe a menu without smuggling native closures or handles across the
//! boundary.

use aimer_attribute::Dimension;
use aimer_macro::PortableValue;
use aimer_style::{
    BorderRadius, BorderSlice, BorderStyle, BoxBorder, BoxDecoration, BoxOutline, BoxShadow,
    FontFamily, FontStyle, FontWeight, LayoutSpacing, ShadowSide, Spacing, TextDecoration,
    TextDecorationLine, TextDecorationStyle, TextOverflow, TextStyle,
};
use aimer_widget::base::Color;
use aimer_widget::portable::__anteros::{
    PropertyId, PropertyValue, PropertyValueKind, WidgetDocumentView,
};
use aimer_widget::portable::{
    PortableMaterializeError, PortableMaterializeProperty, PortableProperty,
    PortablePropertyConversion, PortablePropertyReflection, PortableValue,
};

#[cfg(feature = "portable-guest")]
use aimer_widget::portable::{PortableBuildContext, PortableBuildError, PortableEncodeProperty};

use crate::item::ContextMenuItem;
use crate::style::ContextMenuStyle;

/// Native menu items with their closures deliberately kept outside the wire
/// value. The wrapper gives the local crate an owned type for the portable
/// property implementation while preserving the public `Vec`-based builder
/// API.
#[derive(Clone, Default)]
pub(crate) struct PortableMenuItems(Vec<ContextMenuItem>);

impl PortableMenuItems {
    pub(crate) fn new(items: Vec<ContextMenuItem>) -> Self {
        Self(items)
    }

    pub(crate) fn push(&mut self, item: ContextMenuItem) {
        self.0.push(item);
    }

    pub(crate) fn len(&self) -> usize {
        self.0.len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub(crate) fn into_vec(self) -> Vec<ContextMenuItem> {
        self.0
    }
}

#[derive(Clone, Debug, PartialEq, PortableValue)]
#[portable_value(
    id = "aimer.value:aimer_ctxmenu::ContextMenuItemList",
    version = "1.0",
    max_encoded_bytes = 8192,
    max_depth = 8,
    max_entries = 256,
    max_string_bytes = 2_048,
    max_value_bytes = 2_048,
    max_reconstruction_work = 1_024
)]
struct PortableContextMenuItemList {
    items: Vec<PortableContextMenuItem>,
}

#[derive(Clone, Debug, PartialEq, PortableValue)]
#[portable_value(
    id = "aimer.value:aimer_ctxmenu::ContextMenuItem",
    version = "1.0",
    max_encoded_bytes = 2_048,
    max_depth = 4,
    max_entries = 8,
    max_string_bytes = 2_048
)]
struct PortableContextMenuItem {
    label: String,
    enabled: bool,
}

#[derive(Clone, Debug, PartialEq, PortableValue)]
#[portable_value(
    id = "aimer.value:aimer_ctxmenu::Dimension",
    version = "1.0",
    max_encoded_bytes = 32,
    max_depth = 4,
    max_entries = 8
)]
enum PortableDimension {
    #[portable_value(tag = 0)]
    Auto,
    #[portable_value(tag = 1)]
    Px(f32),
    #[portable_value(tag = 2)]
    Percent(f32),
}

#[derive(Clone, Debug, PartialEq, PortableValue)]
#[portable_value(
    id = "aimer.value:aimer_ctxmenu::Spacing",
    version = "1.0",
    max_encoded_bytes = 32,
    max_depth = 4,
    max_entries = 8
)]
enum PortableSpacing {
    #[portable_value(tag = 0)]
    None,
    #[portable_value(tag = 1)]
    Px(u32),
    #[portable_value(tag = 2)]
    Percent(u32),
}

#[derive(Clone, Debug, PartialEq, PortableValue)]
#[portable_value(
    id = "aimer.value:aimer_ctxmenu::BorderStyle",
    version = "1.0",
    max_encoded_bytes = 16,
    max_depth = 4,
    max_entries = 4
)]
enum PortableBorderStyle {
    #[portable_value(tag = 0)]
    Solid,
    #[portable_value(tag = 1)]
    Dashed,
    #[portable_value(tag = 2)]
    Dotted,
    #[portable_value(tag = 3)]
    None,
}

#[derive(Clone, Debug, PartialEq, PortableValue)]
#[portable_value(
    id = "aimer.value:aimer_ctxmenu::BorderSlice",
    version = "1.0",
    max_encoded_bytes = 128,
    max_depth = 8,
    max_entries = 16
)]
struct PortableBorderSlice {
    style: PortableBorderStyle,
    stroke: PortableDimension,
    color: u32,
}

#[derive(Clone, Debug, PartialEq, PortableValue)]
#[portable_value(
    id = "aimer.value:aimer_ctxmenu::ShadowSide",
    version = "1.0",
    max_encoded_bytes = 64,
    max_depth = 4,
    max_entries = 8
)]
enum PortableShadowSide {
    #[portable_value(tag = 0)]
    All,
    #[portable_value(tag = 1)]
    Top,
    #[portable_value(tag = 2)]
    Right,
    #[portable_value(tag = 3)]
    Bottom,
    #[portable_value(tag = 4)]
    Left,
    #[portable_value(tag = 5)]
    Vertical,
    #[portable_value(tag = 6)]
    Horizontal,
    #[portable_value(tag = 7)]
    Range(f32, f32),
    #[portable_value(tag = 8)]
    TopLeft,
    #[portable_value(tag = 9)]
    TopRight,
    #[portable_value(tag = 10)]
    BottomRight,
    #[portable_value(tag = 11)]
    BottomLeft,
}

#[derive(Clone, Debug, PartialEq, PortableValue)]
#[portable_value(
    id = "aimer.value:aimer_ctxmenu::BoxShadow",
    version = "1.0",
    max_encoded_bytes = 128,
    max_depth = 8,
    max_entries = 16
)]
struct PortableBoxShadow {
    offset_x: f32,
    offset_y: f32,
    blur: f32,
    spread: f32,
    color: u32,
    inset: bool,
    side: PortableShadowSide,
}

#[derive(Clone, Debug, PartialEq, PortableValue)]
#[portable_value(
    id = "aimer.value:aimer_ctxmenu::BoxDecoration",
    version = "1.0",
    max_encoded_bytes = 4_096,
    max_depth = 12,
    max_entries = 256,
    max_reconstruction_work = 1_024
)]
struct PortableBoxDecoration {
    border: [PortableBorderSlice; 4],
    outline: [PortableBorderSlice; 4],
    border_radius: [PortableDimension; 4],
    box_shadow: Vec<PortableBoxShadow>,
    background_color: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, PortableValue)]
#[portable_value(
    id = "aimer.value:aimer_ctxmenu::FontStyle",
    version = "1.0",
    max_encoded_bytes = 32,
    max_depth = 4,
    max_entries = 8
)]
enum PortableFontStyle {
    #[portable_value(tag = 0)]
    Normal,
    #[portable_value(tag = 1)]
    Italic,
    #[portable_value(tag = 2)]
    Oblique,
    #[portable_value(tag = 3)]
    ObliqueDeg(i32),
}

#[derive(Clone, Debug, PartialEq, PortableValue)]
#[portable_value(
    id = "aimer.value:aimer_ctxmenu::FontWeight",
    version = "1.0",
    max_encoded_bytes = 32,
    max_depth = 4,
    max_entries = 8
)]
enum PortableFontWeight {
    #[portable_value(tag = 0)]
    VeryThin,
    #[portable_value(tag = 1)]
    Thin,
    #[portable_value(tag = 2)]
    Normal,
    #[portable_value(tag = 3)]
    Bold,
    #[portable_value(tag = 4)]
    Bolder,
    #[portable_value(tag = 5)]
    Value(u32),
}

#[derive(Clone, Debug, PartialEq, PortableValue)]
#[portable_value(
    id = "aimer.value:aimer_ctxmenu::TextOverflow",
    version = "1.0",
    max_encoded_bytes = 32,
    max_depth = 4,
    max_entries = 8
)]
enum PortableTextOverflow {
    #[portable_value(tag = 0)]
    Clip,
    #[portable_value(tag = 1)]
    Ellipsis,
    #[portable_value(tag = 2)]
    Wrap,
    #[portable_value(tag = 3)]
    Value(u32),
}

#[derive(Clone, Debug, PartialEq, PortableValue)]
#[portable_value(
    id = "aimer.value:aimer_ctxmenu::TextDecorationStyle",
    version = "1.0",
    max_encoded_bytes = 16,
    max_depth = 4,
    max_entries = 4
)]
enum PortableTextDecorationStyle {
    #[portable_value(tag = 0)]
    Solid,
    #[portable_value(tag = 1)]
    Double,
    #[portable_value(tag = 2)]
    Dotted,
    #[portable_value(tag = 3)]
    Dashed,
    #[portable_value(tag = 4)]
    Wavy,
}

#[derive(Clone, Debug, PartialEq, PortableValue)]
#[portable_value(
    id = "aimer.value:aimer_ctxmenu::TextDecoration",
    version = "1.0",
    max_encoded_bytes = 128,
    max_depth = 8,
    max_entries = 16
)]
struct PortableTextDecoration {
    line: u8,
    style: PortableTextDecorationStyle,
    color: Option<u32>,
    thickness: Option<f32>,
    offset: f32,
}

#[derive(Clone, Debug, PartialEq, PortableValue)]
#[portable_value(
    id = "aimer.value:aimer_ctxmenu::TextStyle",
    version = "1.0",
    max_encoded_bytes = 512,
    max_depth = 12,
    max_entries = 64
)]
struct PortableTextStyle {
    font_size: u32,
    font_family: u64,
    font_style: PortableFontStyle,
    font_weight: PortableFontWeight,
    color: u32,
    background_color: Option<u32>,
    text_overflow: PortableTextOverflow,
    text_decoration: PortableTextDecoration,
}

#[derive(Clone, Debug, PartialEq, PortableValue)]
#[portable_value(
    id = "aimer.value:aimer_ctxmenu::ContextMenuStyle",
    version = "1.0",
    max_encoded_bytes = 8192,
    max_depth = 16,
    max_entries = 512,
    max_string_bytes = 2_048,
    max_value_bytes = 4_096,
    max_reconstruction_work = 2_048
)]
struct PortableContextMenuStyle {
    panel: PortableBoxDecoration,
    padding: [PortableSpacing; 4],
    label: PortableTextStyle,
    disabled_label_color: u32,
    highlight_color: u32,
    separator_color: u32,
    row_height: f32,
    item_padding: f32,
    min_width: f32,
    gap: f32,
    screen_margin: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i64)]
enum ShapeTag {
    Pill = 0,
    List = 1,
}

impl PortableProperty for crate::shape::ContextMenuShape {
    const REFLECTION: PortablePropertyReflection = PortablePropertyReflection::new(
        PropertyValueKind::I64,
        PortablePropertyConversion::SignedInteger {
            minimum: ShapeTag::Pill as i64,
            maximum: ShapeTag::List as i64,
        },
    );
}

impl PortableMaterializeProperty for crate::shape::ContextMenuShape {
    fn from_awir(
        _document: &WidgetDocumentView<'_>,
        property: PropertyId,
        value: PropertyValue,
    ) -> Result<Self, PortableMaterializeError> {
        match value {
            PropertyValue::I64(value) if value == ShapeTag::Pill as i64 => Ok(Self::Pill),
            PropertyValue::I64(value) if value == ShapeTag::List as i64 => Ok(Self::List),
            PropertyValue::I64(_) => {
                Err(PortableMaterializeError::InvalidPropertyValue { property })
            }
            _ => Err(PortableMaterializeError::InvalidPropertyType { property }),
        }
    }
}

#[cfg(feature = "portable-guest")]
impl PortableEncodeProperty for crate::shape::ContextMenuShape {
    fn encode_property(
        self,
        _context: &mut PortableBuildContext,
    ) -> Result<PropertyValue, PortableBuildError> {
        Ok(PropertyValue::I64(match self {
            Self::Pill => ShapeTag::Pill as i64,
            Self::List => ShapeTag::List as i64,
        }))
    }
}

impl PortableProperty for PortableMenuItems {
    const REFLECTION: PortablePropertyReflection =
        PortablePropertyReflection::custom(PortableContextMenuItemList::SCHEMA);
}

impl PortableMaterializeProperty for PortableMenuItems {
    fn from_awir(
        document: &WidgetDocumentView<'_>,
        property: PropertyId,
        value: PropertyValue,
    ) -> Result<Self, PortableMaterializeError> {
        let PropertyValue::BlobRef(index) = value else {
            return Err(PortableMaterializeError::InvalidPropertyType { property });
        };
        let bytes = document
            .blob(index)
            .ok_or(PortableMaterializeError::InvalidPropertyReference { property, index })?;
        let value = PortableContextMenuItemList::decode_value(
            bytes,
            PortableContextMenuItemList::SCHEMA.version(),
        )
        .map_err(|_| PortableMaterializeError::InvalidPropertyValue { property })?;
        Ok(Self::new(
            value
                .items
                .into_iter()
                .map(|item| ContextMenuItem::new(item.label).enabled(item.enabled))
                .collect(),
        ))
    }
}

#[cfg(feature = "portable-guest")]
impl PortableEncodeProperty for PortableMenuItems {
    fn encode_property(
        self,
        context: &mut PortableBuildContext,
    ) -> Result<PropertyValue, PortableBuildError> {
        let value = PortableContextMenuItemList {
            items: self
                .0
                .iter()
                .map(|item| PortableContextMenuItem {
                    label: item.label().to_owned(),
                    enabled: item.is_enabled(),
                })
                .collect(),
        };
        let bytes = value
            .encode_value()
            .map_err(|error| PortableBuildError::ValueCodec {
                rust_type: "Vec<aimer_ctxmenu::ContextMenuItem>",
                message: error.to_string(),
            })?;
        context.push_owned_blob(bytes)
    }
}

impl PortableProperty for ContextMenuStyle {
    const REFLECTION: PortablePropertyReflection =
        PortablePropertyReflection::custom(PortableContextMenuStyle::SCHEMA);
}

impl PortableMaterializeProperty for ContextMenuStyle {
    fn from_awir(
        document: &WidgetDocumentView<'_>,
        property: PropertyId,
        value: PropertyValue,
    ) -> Result<Self, PortableMaterializeError> {
        let PropertyValue::BlobRef(index) = value else {
            return Err(PortableMaterializeError::InvalidPropertyType { property });
        };
        let bytes = document
            .blob(index)
            .ok_or(PortableMaterializeError::InvalidPropertyReference { property, index })?;
        let value = PortableContextMenuStyle::decode_value(
            bytes,
            PortableContextMenuStyle::SCHEMA.version(),
        )
        .map_err(|_| PortableMaterializeError::InvalidPropertyValue { property })?;
        value
            .into_native()
            .map_err(|_| PortableMaterializeError::InvalidPropertyValue { property })
    }
}

#[cfg(feature = "portable-guest")]
impl PortableEncodeProperty for ContextMenuStyle {
    fn encode_property(
        self,
        context: &mut PortableBuildContext,
    ) -> Result<PropertyValue, PortableBuildError> {
        let value = PortableContextMenuStyle::from_native(&self);
        let bytes = value
            .encode_value()
            .map_err(|error| PortableBuildError::ValueCodec {
                rust_type: "aimer_ctxmenu::ContextMenuStyle",
                message: error.to_string(),
            })?;
        context.push_owned_blob(bytes)
    }
}

impl PortableContextMenuItemList {
    #[cfg(test)]
    fn from_items(items: &[ContextMenuItem]) -> Self {
        Self {
            items: items
                .iter()
                .map(|item| PortableContextMenuItem {
                    label: item.label().to_owned(),
                    enabled: item.is_enabled(),
                })
                .collect(),
        }
    }
}

impl PortableContextMenuStyle {
    #[cfg(any(feature = "portable-guest", test))]
    fn from_native(style: &ContextMenuStyle) -> Self {
        Self {
            panel: PortableBoxDecoration::from_native(&style.panel),
            padding: [
                PortableSpacing::from_native(style.padding.top),
                PortableSpacing::from_native(style.padding.bottom),
                PortableSpacing::from_native(style.padding.left),
                PortableSpacing::from_native(style.padding.right),
            ],
            label: PortableTextStyle::from_native(&style.label),
            disabled_label_color: style.disabled_label_color.as_u32(),
            highlight_color: style.highlight_color.as_u32(),
            separator_color: style.separator_color.as_u32(),
            row_height: style.row_height,
            item_padding: style.item_padding,
            min_width: style.min_width,
            gap: style.gap,
            screen_margin: style.screen_margin,
        }
    }

    fn into_native(self) -> Result<ContextMenuStyle, ()> {
        Ok(ContextMenuStyle {
            panel: self.panel.into_native()?,
            padding: LayoutSpacing {
                top: self.padding[0].clone().into_native(),
                bottom: self.padding[1].clone().into_native(),
                left: self.padding[2].clone().into_native(),
                right: self.padding[3].clone().into_native(),
            },
            label: self.label.into_native()?,
            disabled_label_color: Color::from_primitive(self.disabled_label_color),
            highlight_color: Color::from_primitive(self.highlight_color),
            separator_color: Color::from_primitive(self.separator_color),
            row_height: self.row_height,
            item_padding: self.item_padding,
            min_width: self.min_width,
            gap: self.gap,
            screen_margin: self.screen_margin,
        })
    }
}

impl PortableDimension {
    #[cfg(any(feature = "portable-guest", test))]
    fn from_native(value: Dimension) -> Self {
        match value {
            Dimension::Auto => Self::Auto,
            Dimension::Px(value) => Self::Px(value),
            Dimension::Percent(value) => Self::Percent(value),
        }
    }

    fn into_native(self) -> Dimension {
        match self {
            Self::Auto => Dimension::Auto,
            Self::Px(value) => Dimension::Px(value),
            Self::Percent(value) => Dimension::Percent(value),
        }
    }
}

impl PortableSpacing {
    #[cfg(any(feature = "portable-guest", test))]
    fn from_native(value: Spacing) -> Self {
        match value {
            Spacing::None => Self::None,
            Spacing::Px(value) => Self::Px(value),
            Spacing::Percent(value) => Self::Percent(value),
        }
    }

    fn into_native(self) -> Spacing {
        match self {
            Self::None => Spacing::None,
            Self::Px(value) => Spacing::Px(value),
            Self::Percent(value) => Spacing::Percent(value),
        }
    }
}

impl PortableBorderSlice {
    #[cfg(any(feature = "portable-guest", test))]
    fn from_native(value: BorderSlice) -> Self {
        Self {
            style: match value.style {
                BorderStyle::Solid => PortableBorderStyle::Solid,
                BorderStyle::Dashed => PortableBorderStyle::Dashed,
                BorderStyle::Dotted => PortableBorderStyle::Dotted,
                BorderStyle::None => PortableBorderStyle::None,
            },
            stroke: PortableDimension::from_native(value.stroke),
            color: value.color.as_u32(),
        }
    }

    fn into_native(self) -> BorderSlice {
        BorderSlice {
            style: match self.style {
                PortableBorderStyle::Solid => BorderStyle::Solid,
                PortableBorderStyle::Dashed => BorderStyle::Dashed,
                PortableBorderStyle::Dotted => BorderStyle::Dotted,
                PortableBorderStyle::None => BorderStyle::None,
            },
            stroke: self.stroke.into_native(),
            color: Color::from_primitive(self.color),
        }
    }
}

impl PortableShadowSide {
    #[cfg(any(feature = "portable-guest", test))]
    fn from_native(value: ShadowSide) -> Self {
        match value {
            ShadowSide::All => Self::All,
            ShadowSide::Top => Self::Top,
            ShadowSide::Right => Self::Right,
            ShadowSide::Bottom => Self::Bottom,
            ShadowSide::Left => Self::Left,
            ShadowSide::Vertical => Self::Vertical,
            ShadowSide::Horizontal => Self::Horizontal,
            ShadowSide::Range(start, end) => Self::Range(start, end),
            ShadowSide::TopLeft => Self::TopLeft,
            ShadowSide::TopRight => Self::TopRight,
            ShadowSide::BottomRight => Self::BottomRight,
            ShadowSide::BottomLeft => Self::BottomLeft,
        }
    }

    fn into_native(self) -> ShadowSide {
        match self {
            Self::All => ShadowSide::All,
            Self::Top => ShadowSide::Top,
            Self::Right => ShadowSide::Right,
            Self::Bottom => ShadowSide::Bottom,
            Self::Left => ShadowSide::Left,
            Self::Vertical => ShadowSide::Vertical,
            Self::Horizontal => ShadowSide::Horizontal,
            Self::Range(start, end) => ShadowSide::Range(start, end),
            Self::TopLeft => ShadowSide::TopLeft,
            Self::TopRight => ShadowSide::TopRight,
            Self::BottomRight => ShadowSide::BottomRight,
            Self::BottomLeft => ShadowSide::BottomLeft,
        }
    }
}

impl PortableBoxShadow {
    #[cfg(any(feature = "portable-guest", test))]
    fn from_native(value: BoxShadow) -> Self {
        Self {
            offset_x: value.offset_x,
            offset_y: value.offset_y,
            blur: value.blur,
            spread: value.spread,
            color: value.color.as_u32(),
            inset: value.inset,
            side: PortableShadowSide::from_native(value.side),
        }
    }

    fn into_native(self) -> BoxShadow {
        BoxShadow {
            offset_x: self.offset_x,
            offset_y: self.offset_y,
            blur: self.blur,
            spread: self.spread,
            color: Color::from_primitive(self.color),
            inset: self.inset,
            side: self.side.into_native(),
        }
    }
}

impl PortableBoxDecoration {
    #[cfg(any(feature = "portable-guest", test))]
    fn from_native(value: &BoxDecoration) -> Self {
        Self {
            border: [
                PortableBorderSlice::from_native(value.border.left),
                PortableBorderSlice::from_native(value.border.right),
                PortableBorderSlice::from_native(value.border.top),
                PortableBorderSlice::from_native(value.border.bottom),
            ],
            outline: [
                PortableBorderSlice::from_native(value.outline.left),
                PortableBorderSlice::from_native(value.outline.right),
                PortableBorderSlice::from_native(value.outline.top),
                PortableBorderSlice::from_native(value.outline.bottom),
            ],
            border_radius: [
                PortableDimension::from_native(value.border_radius.top_left),
                PortableDimension::from_native(value.border_radius.top_right),
                PortableDimension::from_native(value.border_radius.bottom_right),
                PortableDimension::from_native(value.border_radius.bottom_left),
            ],
            box_shadow: value
                .box_shadow
                .iter()
                .copied()
                .map(PortableBoxShadow::from_native)
                .collect(),
            background_color: value.background_color.get().map(|color| color.as_u32()),
        }
    }

    fn into_native(self) -> Result<BoxDecoration, ()> {
        let panel = BoxDecoration {
            border: BoxBorder {
                left: self.border[0].clone().into_native(),
                right: self.border[1].clone().into_native(),
                top: self.border[2].clone().into_native(),
                bottom: self.border[3].clone().into_native(),
            },
            outline: BoxOutline {
                left: self.outline[0].clone().into_native(),
                right: self.outline[1].clone().into_native(),
                top: self.outline[2].clone().into_native(),
                bottom: self.outline[3].clone().into_native(),
            },
            border_radius: BorderRadius {
                top_left: self.border_radius[0].clone().into_native(),
                top_right: self.border_radius[1].clone().into_native(),
                bottom_right: self.border_radius[2].clone().into_native(),
                bottom_left: self.border_radius[3].clone().into_native(),
            },
            box_shadow: self
                .box_shadow
                .into_iter()
                .map(PortableBoxShadow::into_native)
                .collect(),
            background_color: std::cell::Cell::new(
                self.background_color.map(Color::from_primitive),
            ),
        };
        Ok(panel)
    }
}

impl PortableFontStyle {
    #[cfg(any(feature = "portable-guest", test))]
    fn from_native(value: FontStyle) -> Self {
        match value {
            FontStyle::Normal => Self::Normal,
            FontStyle::Italic => Self::Italic,
            FontStyle::Oblique => Self::Oblique,
            FontStyle::ObliqueDeg(value) => Self::ObliqueDeg(value),
        }
    }

    fn into_native(self) -> FontStyle {
        match self {
            Self::Normal => FontStyle::Normal,
            Self::Italic => FontStyle::Italic,
            Self::Oblique => FontStyle::Oblique,
            Self::ObliqueDeg(value) => FontStyle::ObliqueDeg(value),
        }
    }
}

impl PortableFontWeight {
    #[cfg(any(feature = "portable-guest", test))]
    fn from_native(value: FontWeight) -> Self {
        match value {
            FontWeight::VeryThin => Self::VeryThin,
            FontWeight::Thin => Self::Thin,
            FontWeight::Normal => Self::Normal,
            FontWeight::Bold => Self::Bold,
            FontWeight::Bolder => Self::Bolder,
            FontWeight::Value(value) => Self::Value(value),
        }
    }

    fn into_native(self) -> FontWeight {
        match self {
            Self::VeryThin => FontWeight::VeryThin,
            Self::Thin => FontWeight::Thin,
            Self::Normal => FontWeight::Normal,
            Self::Bold => FontWeight::Bold,
            Self::Bolder => FontWeight::Bolder,
            Self::Value(value) => FontWeight::Value(value),
        }
    }
}

impl PortableTextOverflow {
    #[cfg(any(feature = "portable-guest", test))]
    fn from_native(value: TextOverflow) -> Self {
        match value {
            TextOverflow::Clip => Self::Clip,
            TextOverflow::Ellipsis => Self::Ellipsis,
            TextOverflow::Wrap => Self::Wrap,
            TextOverflow::Value(value) => Self::Value(value),
        }
    }

    fn into_native(self) -> TextOverflow {
        match self {
            Self::Clip => TextOverflow::Clip,
            Self::Ellipsis => TextOverflow::Ellipsis,
            Self::Wrap => TextOverflow::Wrap,
            Self::Value(value) => TextOverflow::Value(value),
        }
    }
}

impl PortableTextDecorationStyle {
    #[cfg(any(feature = "portable-guest", test))]
    fn from_native(value: TextDecorationStyle) -> Self {
        match value {
            TextDecorationStyle::Solid => Self::Solid,
            TextDecorationStyle::Double => Self::Double,
            TextDecorationStyle::Dotted => Self::Dotted,
            TextDecorationStyle::Dashed => Self::Dashed,
            TextDecorationStyle::Wavy => Self::Wavy,
        }
    }

    fn into_native(self) -> TextDecorationStyle {
        match self {
            Self::Solid => TextDecorationStyle::Solid,
            Self::Double => TextDecorationStyle::Double,
            Self::Dotted => TextDecorationStyle::Dotted,
            Self::Dashed => TextDecorationStyle::Dashed,
            Self::Wavy => TextDecorationStyle::Wavy,
        }
    }
}

impl PortableTextDecoration {
    #[cfg(any(feature = "portable-guest", test))]
    fn from_native(value: TextDecoration) -> Self {
        Self {
            line: value.line.bits(),
            style: PortableTextDecorationStyle::from_native(value.style),
            color: value.color.map(|color| color.as_u32()),
            thickness: value.thickness,
            offset: value.offset,
        }
    }

    fn into_native(self) -> Result<TextDecoration, ()> {
        Ok(TextDecoration {
            line: TextDecorationLine::from_bits(self.line).ok_or(())?,
            style: self.style.into_native(),
            color: self.color.map(Color::from_primitive),
            thickness: self.thickness,
            offset: self.offset,
        })
    }
}

impl PortableTextStyle {
    #[cfg(any(feature = "portable-guest", test))]
    fn from_native(value: &TextStyle) -> Self {
        Self {
            font_size: value.font_size,
            font_family: value.font_family.raw(),
            font_style: PortableFontStyle::from_native(value.font_style),
            font_weight: PortableFontWeight::from_native(value.font_weight),
            color: value.color.as_u32(),
            background_color: value.background_color.map(|color| color.as_u32()),
            text_overflow: PortableTextOverflow::from_native(value.text_overflow),
            text_decoration: PortableTextDecoration::from_native(value.text_decoration),
        }
    }

    fn into_native(self) -> Result<TextStyle, ()> {
        Ok(TextStyle {
            font_size: self.font_size,
            font_family: FontFamily::from_raw(self.font_family),
            font_style: self.font_style.into_native(),
            font_weight: self.font_weight.into_native(),
            color: Color::from_primitive(self.color),
            background_color: self.background_color.map(Color::from_primitive),
            text_overflow: self.text_overflow.into_native(),
            text_decoration: self.text_decoration.into_native()?,
            ..TextStyle::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_projection_drops_native_actions_but_keeps_display_state() {
        let items = vec![
            ContextMenuItem::new("Copy").on_select(|| {}),
            ContextMenuItem::new("Paste").enabled(false),
        ];
        let projection = PortableContextMenuItemList::from_items(&items);

        assert_eq!(
            projection.items,
            vec![
                PortableContextMenuItem {
                    label: "Copy".into(),
                    enabled: true,
                },
                PortableContextMenuItem {
                    label: "Paste".into(),
                    enabled: false,
                },
            ]
        );
    }

    #[test]
    fn style_projection_round_trips_the_complete_native_description() {
        let style = ContextMenuStyle::list()
            .panel(
                BoxDecoration::new()
                    .background_color(Color::Rgba(1, 2, 3, 4))
                    .border_radius((1.0, 2.0, 3.0, 4.0)),
            )
            .padding(LayoutSpacing::all(Spacing::Percent(5)))
            .label(
                TextStyle::new()
                    .font_size(17)
                    .font_family(FontFamily::MONOSPACE)
                    .font_style(FontStyle::ObliqueDeg(-4))
                    .font_weight(FontWeight::Value(650))
                    .text_decoration(
                        TextDecoration::new()
                            .line(TextDecorationLine::UNDERLINE)
                            .style(TextDecorationStyle::Wavy)
                            .offset(-0.25),
                    ),
            )
            .disabled_label_color(Color::Rgba(5, 6, 7, 8))
            .highlight_color(Color::Rgba(9, 10, 11, 12))
            .separator_color(Color::Rgba(13, 14, 15, 16))
            .row_height(31.0)
            .item_padding(12.0)
            .min_width(140.0)
            .gap(3.0)
            .screen_margin(6.0);

        let wire = PortableContextMenuStyle::from_native(&style);
        let decoded = PortableContextMenuStyle::decode_value(
            &wire.encode_value().unwrap(),
            PortableContextMenuStyle::SCHEMA.version(),
        )
        .unwrap()
        .into_native()
        .unwrap();

        assert_eq!(decoded.panel, style.panel);
        assert!(decoded.padding == style.padding);
        assert_eq!(decoded.label, style.label);
        assert_eq!(decoded.disabled_label_color, style.disabled_label_color);
        assert_eq!(decoded.highlight_color, style.highlight_color);
        assert_eq!(decoded.separator_color, style.separator_color);
        assert_eq!(decoded.row_height, style.row_height);
        assert_eq!(decoded.item_padding, style.item_padding);
        assert_eq!(decoded.min_width, style.min_width);
        assert_eq!(decoded.gap, style.gap);
        assert_eq!(decoded.screen_margin, style.screen_margin);
    }

    #[test]
    fn projections_reject_trailing_bytes_and_invalid_decoration_bits() {
        let value = PortableContextMenuStyle::from_native(&ContextMenuStyle::default());
        let mut bytes = value.encode_value().unwrap();
        bytes.push(0);
        assert!(
            PortableContextMenuStyle::decode_value(
                &bytes,
                PortableContextMenuStyle::SCHEMA.version()
            )
            .is_err()
        );

        let mut value = value;
        value.label.text_decoration.line = 0x80;
        assert!(value.into_native().is_err());
    }
}
