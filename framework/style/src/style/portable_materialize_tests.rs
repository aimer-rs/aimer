#[cfg(feature = "portable-guest")]
use super::{
    FONT_STYLE_NORMAL_TAG, FONT_WEIGHT_NORMAL_TAG, OPTIONAL_NONE_TAG, TEXT_DECORATION_SOLID_TAG,
    TEXT_OVERFLOW_WRAP_TAG,
};
#[cfg(feature = "portable-guest")]
use aimer_widget::portable::__anteros::WidgetProperty;
use aimer_widget::portable::__anteros::{
    ModelLimits, PropertyId, PropertyValue, Version, WidgetDocument, WidgetDocumentView,
    WidgetNode, WidgetSchemaId,
};
#[cfg(feature = "portable-guest")]
use aimer_widget::portable::{
    PortableBuildContext, PortableBuildError, PortableEncodeProperty, PortableLimits,
    PortableWidgetLimits, SourceFingerprint, StableId128,
};
use aimer_widget::portable::{
    PortableMaterializeProperty, PortableProperty, PortablePropertyConversion,
};

use super::super::layout_spacing::{LayoutSpacing, Spacing};
use super::super::text_style::{
    FontFamily, FontStyle, FontWeight, LineHeight, TextDecoration, TextDecorationLine,
    TextDecorationStyle, TextOverflow, TextShadow, TextStyle, TextTransform,
};
use crate::{
    BorderRadius, BorderSlice, BorderStyle, BoxBorder, BoxDecoration, BoxOutline, BoxShadow,
    ShadowSide,
};
use aimer_attribute::Dimension;
use aimer_color::prelude::Color;

const PROPERTY: PropertyId = PropertyId::new(7);
const LIMITS: ModelLimits = ModelLimits::new(4_096, 4, 64, 4_096);

#[cfg(feature = "portable-guest")]
fn guest_context() -> PortableBuildContext {
    PortableBuildContext::new(
        1,
        0,
        PortableWidgetLimits::new(4, 16, 4, 4, 64, 4_096).with_max_blob_bytes(4_096),
        PortableLimits::new(4, 16, 64, 4_096, 4_096),
    )
    .unwrap()
}

#[test]
fn layout_spacing_reflects_its_versioned_blob_schema() {
    let reflection = <LayoutSpacing as PortableProperty>::REFLECTION;
    let schema = reflection.value_schema().unwrap();

    assert_eq!(
        reflection.value_kind(),
        aimer_widget::portable::__anteros::PropertyValueKind::BlobRef
    );
    assert_eq!(
        reflection.conversion(),
        PortablePropertyConversion::CustomValue
    );
    assert_eq!(
        schema.canonical_name(),
        "aimer.value:aimer_style::LayoutSpacing"
    );
    assert_eq!(schema.version(), Version::new(1, 0));
    assert_eq!(schema.maximum_encoded_bytes(), 21);
}

#[test]
fn text_style_reflects_its_versioned_blob_schema() {
    let reflection = <TextStyle as PortableProperty>::REFLECTION;
    let schema = reflection.value_schema().unwrap();

    assert_eq!(
        reflection.value_kind(),
        aimer_widget::portable::__anteros::PropertyValueKind::BlobRef
    );
    assert_eq!(
        schema.canonical_name(),
        "aimer.value:aimer_style::TextStyle"
    );
    assert_eq!(schema.version(), Version::new(2, 0));
    assert_eq!(schema.maximum_encoded_bytes(), 128);
}

#[test]
fn line_height_reflects_its_versioned_blob_schema() {
    let reflection = <LineHeight as PortableProperty>::REFLECTION;
    let schema = reflection.value_schema().unwrap();

    assert_eq!(
        schema.canonical_name(),
        "aimer.value:aimer_style::LineHeight"
    );
    assert_eq!(schema.version(), Version::new(1, 0));
    assert_eq!(schema.maximum_encoded_bytes(), 5);
}

#[cfg(feature = "portable-guest")]
#[test]
fn line_height_guest_encoding_round_trips_each_form() {
    for value in [
        LineHeight::Normal,
        LineHeight::Px(24.0),
        LineHeight::Factor(1.5),
    ] {
        let blob = encode_blob(value);
        assert_eq!(materialize_line_height(&blob), value);
    }
}

#[cfg(feature = "portable-guest")]
#[test]
fn text_style_guest_encoding_round_trips_every_codec_variant() {
    let style = TextStyle::new()
        .font_size(24)
        .font_family(FontFamily::MONOSPACE)
        .font_style(FontStyle::ObliqueDeg(-7))
        .font_weight(FontWeight::Value(650))
        .color(Color::Rgba(10, 20, 30, 40))
        .background_color(Color::Rgba(50, 60, 70, 80))
        .text_overflow(TextOverflow::Value(3))
        .text_transform(TextTransform::Uppercase)
        .letter_spacing(0.75)
        .word_spacing(-1.25)
        .text_shadow(
            TextShadow::new()
                .offset_x(2.0)
                .offset_y(-1.0)
                .blur(3.0)
                .color(Color::Rgba(11, 12, 13, 14)),
        )
        .text_decoration(
            TextDecoration::new()
                .line(TextDecorationLine::UNDERLINE | TextDecorationLine::ITALIC)
                .style(TextDecorationStyle::Wavy)
                .color(Color::Rgba(90, 100, 110, 120))
                .thickness(1.25)
                .offset(-0.5),
        );

    let blob = encode_blob(style);
    assert!(blob.len() <= 128);
    assert_eq!(materialize_text_style(&blob), style);
}

#[cfg(feature = "portable-guest")]
#[test]
fn text_style_version_one_decodes_with_new_values_at_defaults() {
    let mut blob = vec![1_u8];
    blob.extend_from_slice(&19_u32.to_le_bytes());
    blob.extend_from_slice(&FontFamily::SANS_SERIF.raw().to_le_bytes());
    blob.push(FONT_STYLE_NORMAL_TAG);
    blob.push(FONT_WEIGHT_NORMAL_TAG);
    blob.extend_from_slice(&Color::BLACK.as_u32().to_le_bytes());
    blob.push(OPTIONAL_NONE_TAG);
    blob.push(TEXT_OVERFLOW_WRAP_TAG);
    blob.push(TextDecorationLine::NONE.bits());
    blob.push(TEXT_DECORATION_SOLID_TAG);
    blob.push(OPTIONAL_NONE_TAG);
    blob.push(OPTIONAL_NONE_TAG);
    blob.extend_from_slice(&0.0_f32.to_bits().to_le_bytes());

    let style = materialize_text_style(&blob);
    assert_eq!(style.font_size, 19);
    assert_eq!(style.text_overflow, TextOverflow::Wrap);
    assert_eq!(style.text_transform, TextTransform::None);
    assert_eq!(style.letter_spacing, 0.0);
    assert_eq!(style.word_spacing, 0.0);
    assert_eq!(style.text_shadow, None);
}

#[cfg(feature = "portable-guest")]
#[test]
fn text_style_v2_rejects_unknown_transform_non_finite_spacing_and_bad_shadow() {
    let base = TextStyle::default();

    let mut unknown_transform = encode_blob(base);
    unknown_transform[29] = 99;
    assert_invalid_text_style_blob(&unknown_transform);

    let mut non_finite_spacing = encode_blob(TextStyle::default());
    non_finite_spacing[30..34].copy_from_slice(&f32::NAN.to_bits().to_le_bytes());
    assert_invalid_text_style_blob(&non_finite_spacing);

    let mut bad_shadow = encode_blob(TextStyle::new().text_shadow(TextShadow::new()));
    bad_shadow[47..51].copy_from_slice(&(-1.0_f32).to_bits().to_le_bytes());
    assert_invalid_text_style_blob(&bad_shadow);
}

#[cfg(feature = "portable-guest")]
#[test]
fn line_height_rejects_unknown_tags_non_finite_values_and_non_positive_values() {
    let mut non_finite = vec![1, 1];
    non_finite.extend_from_slice(&f32::NAN.to_bits().to_le_bytes());
    for blob in [
        vec![1, 99],
        non_finite,
        vec![1, 1, 0, 0, 0, 0],
        vec![1, 2, 0, 0, 0, 0],
    ] {
        assert_invalid_line_height_blob(&blob);
    }
}

#[test]
fn box_decoration_reflects_an_unrestricted_versioned_blob_schema() {
    let reflection = <BoxDecoration as PortableProperty>::REFLECTION;
    let schema = reflection.value_schema().unwrap();

    assert_eq!(
        reflection.value_kind(),
        aimer_widget::portable::__anteros::PropertyValueKind::BlobRef
    );
    assert_eq!(
        reflection.conversion(),
        PortablePropertyConversion::CustomValue
    );
    assert_eq!(
        schema.canonical_name(),
        "aimer.value:aimer_style::BoxDecoration"
    );
    assert_eq!(schema.version(), Version::new(1, 0));
    assert_eq!(schema.maximum_encoded_bytes(), u32::MAX);
}

#[test]
fn layout_spacing_materializes_every_spacing_variant_from_its_canonical_blob() {
    #[rustfmt::skip]
    let blob = [
        1, // wire version
        1, 12, 0, 0, 0, // top: 12 px
        2, 25, 0, 0, 0, // bottom: 25 percent
        0, 0, 0, 0, 0, // left: none
        1, 7, 0, 0, 0, // right: 7 px
    ];
    let image = widget_document(&blob);
    let document = WidgetDocumentView::decode(&image, LIMITS).unwrap();

    let spacing = LayoutSpacing::from_awir(&document, PROPERTY, PropertyValue::BlobRef(0)).unwrap();

    assert!(
        spacing
            == LayoutSpacing {
                top: Spacing::Px(12),
                bottom: Spacing::Percent(25),
                left: Spacing::None,
                right: Spacing::Px(7),
            }
    );
}

#[cfg(feature = "portable-guest")]
#[test]
fn layout_spacing_guest_encoding_round_trips_its_canonical_blob() {
    let spacing = LayoutSpacing {
        top: Spacing::Px(12),
        bottom: Spacing::Percent(25),
        left: Spacing::None,
        right: Spacing::Px(7),
    };
    let mut context = guest_context();
    let value = spacing.encode_property(&mut context).unwrap();
    assert_eq!(value, PropertyValue::BlobRef(0));

    let node = context
        .push_node(
            WidgetSchemaId::new(1),
            Version::new(1, 0),
            None,
            SourceFingerprint::new(StableId128::from_u128(1)),
            &[WidgetProperty::new(PROPERTY, value)],
            &[],
        )
        .unwrap();
    let graph = context.finish_graph(node).unwrap();

    assert_eq!(
        graph.blob(0),
        Some(
            &[
                1, // wire version
                1, 12, 0, 0, 0, // top: 12 px
                2, 25, 0, 0, 0, // bottom: 25 percent
                0, 0, 0, 0, 0, // left: none
                1, 7, 0, 0, 0, // right: 7 px
            ][..]
        )
    );
    let image = widget_document(graph.blob(0).unwrap());
    let document = WidgetDocumentView::decode(&image, LIMITS).unwrap();
    assert!(
        LayoutSpacing::from_awir(&document, PROPERTY, PropertyValue::BlobRef(0)).unwrap()
            == spacing
    );
}

#[cfg(feature = "portable-guest")]
#[test]
fn box_decoration_guest_encoding_round_trips_empty_and_complete_values() {
    let empty = BoxDecoration::new();
    let empty_blob = encode_blob(empty.clone());
    let mut expected_empty = vec![0_u8; 106];
    expected_empty[0] = 1;
    assert_eq!(empty_blob, expected_empty);
    assert_eq!(materialize_decoration(&empty_blob), empty);

    let complete = BoxDecoration {
        border: BoxBorder {
            left: BorderSlice {
                style: BorderStyle::Solid,
                stroke: Dimension::Px(2.0),
                color: Color::RED,
            },
            right: BorderSlice {
                style: BorderStyle::Dashed,
                stroke: Dimension::Percent(5.0),
                color: Color::GREEN,
            },
            top: BorderSlice {
                style: BorderStyle::Dotted,
                stroke: Dimension::Auto,
                color: Color::BLUE,
            },
            bottom: BorderSlice::default(),
        },
        outline: BoxOutline::default(),
        border_radius: BorderRadius {
            top_left: Dimension::Px(4.0),
            top_right: Dimension::Percent(10.0),
            bottom_right: Dimension::Auto,
            bottom_left: Dimension::Px(8.0),
        },
        box_shadow: vec![
            BoxShadow::new()
                .offset_x(1.0)
                .offset_y(2.0)
                .blur(3.0)
                .spread(4.0)
                .color(Color::BLACK),
            BoxShadow::new()
                .offset_x(-1.0)
                .offset_y(-2.0)
                .blur(5.0)
                .spread(-3.0)
                .color(Color::BLUE)
                .inset(true)
                .side(ShadowSide::Range(0.25, 1.5)),
        ],
        background_color: std::cell::Cell::new(Some(Color::WHITE)),
    };
    let complete_blob = encode_blob(complete.clone());
    assert_eq!(complete_blob, complete_decoration_blob());
    assert_eq!(materialize_decoration(&complete_blob), complete);
}

#[cfg(feature = "portable-guest")]
#[test]
fn box_decoration_guest_encoding_covers_every_shadow_side_tag() {
    let sides = [
        ShadowSide::All,
        ShadowSide::Top,
        ShadowSide::Right,
        ShadowSide::Bottom,
        ShadowSide::Left,
        ShadowSide::Vertical,
        ShadowSide::Horizontal,
        ShadowSide::Range(0.25, 1.5),
        ShadowSide::TopLeft,
        ShadowSide::TopRight,
        ShadowSide::BottomRight,
        ShadowSide::BottomLeft,
    ];

    for (index, side) in sides.into_iter().enumerate() {
        let decoration = BoxDecoration::new().add_shadow(
            BoxShadow::new()
                .offset_x(index as f32)
                .inset(index % 2 == 1)
                .side(side),
        );
        let blob = encode_blob(decoration.clone());
        assert_eq!(blob[126], index as u8);
        assert_eq!(materialize_decoration(&blob), decoration);
    }
}

#[cfg(feature = "portable-guest")]
#[test]
fn box_decoration_guest_encoding_rejects_non_finite_values_before_commit() {
    let mut context = guest_context();
    let value = BoxDecoration::new().border_radius(BorderRadius::new().top_left(f32::NAN));
    assert!(matches!(
        value.encode_property(&mut context),
        Err(PortableBuildError::NonFiniteFloat)
    ));

    let value = BoxDecoration::new().add_shadow(BoxShadow::new().blur(f32::INFINITY));
    assert!(matches!(
        value.encode_property(&mut context),
        Err(PortableBuildError::NonFiniteFloat)
    ));
}

#[test]
fn box_decoration_materializes_the_complete_bounded_blob() {
    let blob = complete_decoration_blob();
    let image = widget_document(&blob);
    let document = WidgetDocumentView::decode(&image, LIMITS).unwrap();

    let decoration =
        BoxDecoration::from_awir(&document, PROPERTY, PropertyValue::BlobRef(0)).unwrap();

    assert_eq!(
        decoration,
        BoxDecoration {
            border: BoxBorder {
                left: BorderSlice {
                    style: BorderStyle::Solid,
                    stroke: Dimension::Px(2.0),
                    color: Color::RED,
                },
                right: BorderSlice {
                    style: BorderStyle::Dashed,
                    stroke: Dimension::Percent(5.0),
                    color: Color::GREEN,
                },
                top: BorderSlice {
                    style: BorderStyle::Dotted,
                    stroke: Dimension::Auto,
                    color: Color::BLUE,
                },
                bottom: BorderSlice::default(),
            },
            outline: BoxOutline::default(),
            border_radius: BorderRadius {
                top_left: Dimension::Px(4.0),
                top_right: Dimension::Percent(10.0),
                bottom_right: Dimension::Auto,
                bottom_left: Dimension::Px(8.0),
            },
            box_shadow: vec![
                BoxShadow::new()
                    .offset_x(1.0)
                    .offset_y(2.0)
                    .blur(3.0)
                    .spread(4.0)
                    .color(Color::BLACK),
                BoxShadow::new()
                    .offset_x(-1.0)
                    .offset_y(-2.0)
                    .blur(5.0)
                    .spread(-3.0)
                    .color(Color::BLUE)
                    .inset(true)
                    .side(ShadowSide::Range(0.25, 1.5)),
            ],
            background_color: std::cell::Cell::new(Some(Color::WHITE)),
        }
    );
}

#[test]
fn portable_style_values_reject_wrong_types_and_non_canonical_blobs() {
    let valid_spacing = [
        1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ];
    let image = widget_document(&valid_spacing);
    let document = WidgetDocumentView::decode(&image, LIMITS).unwrap();
    assert!(matches!(
        LayoutSpacing::from_awir(&document, PROPERTY, PropertyValue::Bool(false)),
        Err(
            aimer_widget::portable::PortableMaterializeError::InvalidPropertyType {
                property: PROPERTY,
            }
        )
    ));

    let invalid_spacing_tag = [
        1, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ];
    assert_invalid_spacing_blob(&invalid_spacing_tag);

    let mut trailing_spacing = valid_spacing.to_vec();
    trailing_spacing.push(0);
    assert_invalid_spacing_blob(&trailing_spacing);

    assert_invalid_spacing_blob(&valid_spacing[..20]);
}

#[test]
fn decoration_rejects_a_shadow_count_not_backed_by_blob_bytes() {
    let mut blob = default_decoration_prefix();
    blob.extend_from_slice(&u32::MAX.to_le_bytes());
    let image = widget_document(&blob);
    let document = WidgetDocumentView::decode(&image, LIMITS).unwrap();

    assert_eq!(
        BoxDecoration::from_awir(&document, PROPERTY, PropertyValue::BlobRef(0)),
        Err(
            aimer_widget::portable::PortableMaterializeError::InvalidPropertyValue {
                property: PROPERTY,
            }
        )
    );
}

#[test]
fn decoration_rejects_truncation_trailing_bytes_unknown_tags_and_non_finite_values() {
    let mut truncated = complete_decoration_blob();
    truncated.pop();
    assert_invalid_decoration_blob(&truncated);

    let mut trailing = complete_decoration_blob();
    trailing.push(0);
    assert_invalid_decoration_blob(&trailing);

    let mut unknown_style = empty_decoration_blob();
    unknown_style[1] = 4;
    assert_invalid_decoration_blob(&unknown_style);

    let mut unknown_dimension = empty_decoration_blob();
    unknown_dimension[2] = 3;
    assert_invalid_decoration_blob(&unknown_dimension);

    let mut unknown_shadow_side = default_decoration_prefix();
    unknown_shadow_side.extend_from_slice(&1_u32.to_le_bytes());
    push_shadow(
        &mut unknown_shadow_side,
        [0.0; 4],
        Color::BLACK,
        false,
        12,
        None,
    );
    assert_invalid_decoration_blob(&unknown_shadow_side);

    let mut noncanonical_boolean = default_decoration_prefix();
    noncanonical_boolean.extend_from_slice(&1_u32.to_le_bytes());
    push_shadow(
        &mut noncanonical_boolean,
        [0.0; 4],
        Color::BLACK,
        false,
        0,
        None,
    );
    noncanonical_boolean[125] = 2;
    assert_invalid_decoration_blob(&noncanonical_boolean);

    let mut noncanonical_option = default_decoration_prefix();
    noncanonical_option.extend_from_slice(&0_u32.to_le_bytes());
    noncanonical_option.push(2);
    assert_invalid_decoration_blob(&noncanonical_option);

    let mut non_finite = complete_decoration_blob();
    non_finite[3..7].copy_from_slice(&f32::NAN.to_bits().to_le_bytes());
    assert_invalid_decoration_blob(&non_finite);
}

fn assert_invalid_spacing_blob(blob: &[u8]) {
    let image = widget_document(blob);
    let document = WidgetDocumentView::decode(&image, LIMITS).unwrap();
    assert!(matches!(
        LayoutSpacing::from_awir(&document, PROPERTY, PropertyValue::BlobRef(0)),
        Err(
            aimer_widget::portable::PortableMaterializeError::InvalidPropertyValue {
                property: PROPERTY,
            }
        )
    ));
}

fn assert_invalid_decoration_blob(blob: &[u8]) {
    let image = widget_document(blob);
    let document = WidgetDocumentView::decode(&image, LIMITS).unwrap();
    assert!(matches!(
        BoxDecoration::from_awir(&document, PROPERTY, PropertyValue::BlobRef(0)),
        Err(
            aimer_widget::portable::PortableMaterializeError::InvalidPropertyValue {
                property: PROPERTY,
            }
        )
    ));
}

#[cfg(feature = "portable-guest")]
fn encode_blob<T: PortableEncodeProperty>(value: T) -> Vec<u8> {
    let mut context = guest_context();
    let property = value.encode_property(&mut context).unwrap();
    let index = match property {
        PropertyValue::BlobRef(index) => index,
        other => panic!("expected a blob property, got {other:?}"),
    };
    let node = context
        .push_node(
            WidgetSchemaId::new(1),
            Version::new(1, 0),
            None,
            SourceFingerprint::new(StableId128::from_u128(2)),
            &[WidgetProperty::new(PROPERTY, property)],
            &[],
        )
        .unwrap();
    let graph = context.finish_graph(node).unwrap();
    graph.blob(index).unwrap().to_vec()
}

#[cfg(feature = "portable-guest")]
fn materialize_decoration(blob: &[u8]) -> BoxDecoration {
    let image = widget_document(blob);
    let document = WidgetDocumentView::decode(&image, LIMITS).unwrap();
    BoxDecoration::from_awir(&document, PROPERTY, PropertyValue::BlobRef(0)).unwrap()
}

#[cfg(feature = "portable-guest")]
fn materialize_text_style(blob: &[u8]) -> TextStyle {
    let image = widget_document(blob);
    let document = WidgetDocumentView::decode(&image, LIMITS).unwrap();
    TextStyle::from_awir(&document, PROPERTY, PropertyValue::BlobRef(0)).unwrap()
}

#[cfg(feature = "portable-guest")]
fn assert_invalid_text_style_blob(blob: &[u8]) {
    let image = widget_document(blob);
    let document = WidgetDocumentView::decode(&image, LIMITS).unwrap();
    assert!(matches!(
        TextStyle::from_awir(&document, PROPERTY, PropertyValue::BlobRef(0)),
        Err(
            aimer_widget::portable::PortableMaterializeError::InvalidPropertyValue {
                property: PROPERTY,
            }
        )
    ));
}

#[cfg(feature = "portable-guest")]
fn materialize_line_height(blob: &[u8]) -> LineHeight {
    let image = widget_document(blob);
    let document = WidgetDocumentView::decode(&image, LIMITS).unwrap();
    LineHeight::from_awir(&document, PROPERTY, PropertyValue::BlobRef(0)).unwrap()
}

#[cfg(feature = "portable-guest")]
fn assert_invalid_line_height_blob(blob: &[u8]) {
    let image = widget_document(blob);
    let document = WidgetDocumentView::decode(&image, LIMITS).unwrap();
    assert!(matches!(
        LineHeight::from_awir(&document, PROPERTY, PropertyValue::BlobRef(0)),
        Err(
            aimer_widget::portable::PortableMaterializeError::InvalidPropertyValue {
                property: PROPERTY,
            }
        )
    ));
}

fn complete_decoration_blob() -> Vec<u8> {
    let mut blob = vec![1];
    push_border_slice(&mut blob, 1, 1, 2.0, Color::RED);
    push_border_slice(&mut blob, 2, 2, 5.0, Color::GREEN);
    push_border_slice(&mut blob, 3, 0, 0.0, Color::BLUE);
    push_border_slice(&mut blob, 0, 0, 0.0, Color::Transparent);
    for _ in 0..4 {
        push_border_slice(&mut blob, 0, 0, 0.0, Color::Transparent);
    }
    push_dimension(&mut blob, 1, 4.0);
    push_dimension(&mut blob, 2, 10.0);
    push_dimension(&mut blob, 0, 0.0);
    push_dimension(&mut blob, 1, 8.0);
    blob.extend_from_slice(&2_u32.to_le_bytes());
    push_shadow(
        &mut blob,
        [1.0, 2.0, 3.0, 4.0],
        Color::BLACK,
        false,
        0,
        None,
    );
    push_shadow(
        &mut blob,
        [-1.0, -2.0, 5.0, -3.0],
        Color::BLUE,
        true,
        7,
        Some((0.25, 1.5)),
    );
    blob.push(1);
    blob.extend_from_slice(&Color::WHITE.as_u32().to_le_bytes());
    blob
}

fn default_decoration_prefix() -> Vec<u8> {
    let mut blob = vec![1];
    for _ in 0..8 {
        push_border_slice(&mut blob, 0, 0, 0.0, Color::Transparent);
    }
    for _ in 0..4 {
        push_dimension(&mut blob, 0, 0.0);
    }
    blob
}

fn empty_decoration_blob() -> Vec<u8> {
    let mut blob = default_decoration_prefix();
    blob.extend_from_slice(&0_u32.to_le_bytes());
    blob.push(0);
    blob
}

fn push_border_slice(blob: &mut Vec<u8>, style: u8, dimension: u8, value: f32, color: Color) {
    blob.push(style);
    push_dimension(blob, dimension, value);
    blob.extend_from_slice(&color.as_u32().to_le_bytes());
}

fn push_dimension(blob: &mut Vec<u8>, tag: u8, value: f32) {
    blob.push(tag);
    blob.extend_from_slice(&value.to_bits().to_le_bytes());
}

fn push_shadow(
    blob: &mut Vec<u8>,
    geometry: [f32; 4],
    color: Color,
    inset: bool,
    side: u8,
    range: Option<(f32, f32)>,
) {
    for value in geometry {
        blob.extend_from_slice(&value.to_bits().to_le_bytes());
    }
    blob.extend_from_slice(&color.as_u32().to_le_bytes());
    blob.push(u8::from(inset));
    blob.push(side);
    if let Some((start, end)) = range {
        blob.extend_from_slice(&start.to_bits().to_le_bytes());
        blob.extend_from_slice(&end.to_bits().to_le_bytes());
    }
}

fn widget_document(blob: &[u8]) -> Vec<u8> {
    let nodes = [WidgetNode::new(WidgetSchemaId::new(1), Version::new(1, 0))];
    WidgetDocument::new(1, 1, 0, &nodes, &[], &[blob])
        .encode(LIMITS)
        .unwrap()
}
