//! Stable identifiers for the initial portable Aimer widget schema.

use crate::schema_identity::{EventId, PropertyId, WidgetSchemaId};
use crate::{
    AsyncCallbackSchemaMetadata, CallbackSchemaMetadata, ChildCardinality,
    PortableWidgetSchemaMetadata, PropertySchemaMetadata, PropertyValueKind, ValueSchemaMetadata,
    Version, WidgetSchemaMetadata,
};

/// Canonical identity retained for the Column schema.
pub const WIDGET_COLUMN_NAME: &str = "aimer.widget:aimer_flex::flex::Column";
/// Canonical identity retained for the Row schema.
pub const WIDGET_ROW_NAME: &str = "aimer.widget:aimer_flex::flex::Row";
/// Canonical identity retained for the Container schema.
pub const WIDGET_CONTAINER_NAME: &str = "aimer.widget:aimer_container::single_child::Container";
/// Canonical identity retained for the SizedBox schema.
pub const WIDGET_SIZED_BOX_NAME: &str = "aimer.widget:aimer_container::single_child::SizedBox";
/// Canonical identity retained for the Text schema.
pub const WIDGET_TEXT_NAME: &str = "aimer.widget:aimer_text::Text";
/// Canonical identity retained for the Button schema.
pub const WIDGET_BUTTON_NAME: &str = "aimer.widget:aimer_input::Button";
/// Canonical identity for a portable provider scope.
pub const WIDGET_PROVIDER_NAME: &str = "aimer.widget:aimer_provider::Provider";
/// Canonical identity for a portable animated theme scope.
pub const WIDGET_ANIMATED_THEME_NAME: &str = "aimer.widget:aimer_style::AnimatedTheme";
/// Canonical identity for the transparent gesture recognizer wrapper.
pub const WIDGET_GESTURE_DETECTOR_NAME: &str = "aimer.widget:aimer_input::GestureDetector";
/// Canonical identity for the transparent hover-tracking wrapper.
pub const WIDGET_MOUSE_REGION_NAME: &str = "aimer.widget:aimer_input::MouseRegion";

/// Schema identifier for a vertical flex container.
pub const WIDGET_COLUMN: WidgetSchemaId = WidgetSchemaId::from_canonical_name(WIDGET_COLUMN_NAME);
/// Schema identifier for a horizontal flex container.
pub const WIDGET_ROW: WidgetSchemaId = WidgetSchemaId::from_canonical_name(WIDGET_ROW_NAME);
/// Schema identifier for a decorated single-child container.
pub const WIDGET_CONTAINER: WidgetSchemaId =
    WidgetSchemaId::from_canonical_name(WIDGET_CONTAINER_NAME);
/// Schema identifier for a fixed or automatically sized empty box.
pub const WIDGET_SIZED_BOX: WidgetSchemaId =
    WidgetSchemaId::from_canonical_name(WIDGET_SIZED_BOX_NAME);
/// Schema identifier for plain text.
pub const WIDGET_TEXT: WidgetSchemaId = WidgetSchemaId::from_canonical_name(WIDGET_TEXT_NAME);
/// Schema identifier for a primary-action button.
pub const WIDGET_BUTTON: WidgetSchemaId = WidgetSchemaId::from_canonical_name(WIDGET_BUTTON_NAME);
/// Schema identifier for a portable provider scope.
pub const WIDGET_PROVIDER: WidgetSchemaId =
    WidgetSchemaId::from_canonical_name(WIDGET_PROVIDER_NAME);
/// Schema identifier for a portable animated theme scope.
pub const WIDGET_ANIMATED_THEME: WidgetSchemaId =
    WidgetSchemaId::from_canonical_name(WIDGET_ANIMATED_THEME_NAME);
/// Schema identifier for a transparent gesture recognizer wrapper.
pub const WIDGET_GESTURE_DETECTOR: WidgetSchemaId =
    WidgetSchemaId::from_canonical_name(WIDGET_GESTURE_DETECTOR_NAME);
/// Schema identifier for a transparent hover-tracking wrapper.
pub const WIDGET_MOUSE_REGION: WidgetSchemaId =
    WidgetSchemaId::from_canonical_name(WIDGET_MOUSE_REGION_NAME);

/// The oldest and current schema version supported by the built-in widgets.
///
/// A newer guest may continue to target this version when it only adds
/// omission-based optional properties. Changing a required field, child
/// cardinality, callback contract, or value codec requires a new version.
pub const BUILTIN_WIDGET_SCHEMA_VERSION: Version = Version::new(1, 0);

/// Canonical identity for the versioned `LayoutSpacing` value codec.
pub const LAYOUT_SPACING_VALUE_NAME: &str = "aimer.value:aimer_style::LayoutSpacing";
/// Current `LayoutSpacing` value-codec version.
pub const LAYOUT_SPACING_VALUE_VERSION: Version = Version::new(1, 0);
/// Maximum encoded payload size for one `LayoutSpacing` value.
pub const LAYOUT_SPACING_VALUE_MAXIMUM_ENCODED_BYTES: u32 = 21;

/// Canonical identity for the versioned `Dimension` value codec.
pub const DIMENSION_VALUE_NAME: &str = "aimer.value:aimer_attribute::Dimension";
/// Current `Dimension` value-codec version.
pub const DIMENSION_VALUE_VERSION: Version = Version::new(1, 0);
/// Maximum encoded payload size for one `Dimension` value.
pub const DIMENSION_VALUE_MAXIMUM_ENCODED_BYTES: u32 = 6;

/// Canonical identity for the versioned `BoxDecoration` value codec.
pub const BOX_DECORATION_VALUE_NAME: &str = "aimer.value:aimer_style::BoxDecoration";
/// Current `BoxDecoration` value-codec version.
pub const BOX_DECORATION_VALUE_VERSION: Version = Version::new(1, 0);
/// Maximum encoded payload size declared by the `BoxDecoration` contract.
pub const BOX_DECORATION_VALUE_MAXIMUM_ENCODED_BYTES: u32 = u32::MAX;

/// Canonical identity for the versioned `TextStyle` value codec.
pub const TEXT_STYLE_VALUE_NAME: &str = "aimer.value:aimer_style::TextStyle";
/// Current `TextStyle` value-codec version.
pub const TEXT_STYLE_VALUE_VERSION: Version = Version::new(2, 0);
/// Maximum encoded payload size for one `TextStyle` value.
pub const TEXT_STYLE_VALUE_MAXIMUM_ENCODED_BYTES: u32 = 128;

/// Canonical identity for the versioned `LineHeight` value codec.
pub const LINE_HEIGHT_VALUE_NAME: &str = "aimer.value:aimer_style::LineHeight";
/// Current `LineHeight` value-codec version.
pub const LINE_HEIGHT_VALUE_VERSION: Version = Version::new(1, 0);
/// Maximum encoded payload size for one `LineHeight` value.
pub const LINE_HEIGHT_VALUE_MAXIMUM_ENCODED_BYTES: u32 = 5;

/// Canonical identity for the provider's stable value-type property.
pub const PROPERTY_PROVIDER_TYPE_NAME: &str =
    "aimer.property:aimer_provider::Provider:provider_type";
/// Canonical identity for the provider value-codec version property.
pub const PROPERTY_PROVIDER_SCHEMA_VERSION_NAME: &str =
    "aimer.property:aimer_provider::Provider:provider_schema_version";
/// Canonical identity for the provider snapshot payload property.
pub const PROPERTY_PROVIDER_VALUE_NAME: &str = "aimer.property:aimer_provider::Provider:value";
/// Maximum provider snapshot payload accepted by the permanent host contract.
pub const PROVIDER_VALUE_MAXIMUM_ENCODED_BYTES: u32 = 65_536;

/// Canonical identity for the built-in `ThemeData` provider value codec.
pub const THEME_DATA_VALUE_NAME: &str = "aimer.value:aimer_style::ThemeData";
/// Current `ThemeData` provider value-codec version.
pub const THEME_DATA_VALUE_VERSION: Version = Version::new(1, 0);
/// Maximum encoded payload size for `ThemeData`.
pub const THEME_DATA_VALUE_MAXIMUM_ENCODED_BYTES: u32 = 24;

/// Maximum encoded payload accepted by the portable animated-theme value slot.
pub const THEME_VALUE_MAXIMUM_ENCODED_BYTES: u32 = PROVIDER_VALUE_MAXIMUM_ENCODED_BYTES;

/// Stable provider value-type property identity.
pub const PROPERTY_PROVIDER_TYPE: PropertyId =
    PropertyId::from_canonical_name(PROPERTY_PROVIDER_TYPE_NAME);
/// Stable provider codec-version property identity.
pub const PROPERTY_PROVIDER_SCHEMA_VERSION: PropertyId =
    PropertyId::from_canonical_name(PROPERTY_PROVIDER_SCHEMA_VERSION_NAME);
/// Stable provider snapshot payload property identity.
pub const PROPERTY_PROVIDER_VALUE: PropertyId =
    PropertyId::from_canonical_name(PROPERTY_PROVIDER_VALUE_NAME);

/// Stable animated-theme value-type property identity.
pub const PROPERTY_ANIMATED_THEME_TYPE: PropertyId =
    PropertyId::from_canonical_name(PROPERTY_ANIMATED_THEME_TYPE_NAME);
/// Stable animated-theme codec-version property identity.
pub const PROPERTY_ANIMATED_THEME_SCHEMA_VERSION: PropertyId =
    PropertyId::from_canonical_name(PROPERTY_ANIMATED_THEME_SCHEMA_VERSION_NAME);
/// Stable animated-theme resolved-value property identity.
pub const PROPERTY_ANIMATED_THEME_VALUE: PropertyId =
    PropertyId::from_canonical_name(PROPERTY_ANIMATED_THEME_VALUE_NAME);
/// Stable animated-theme mode property identity.
pub const PROPERTY_ANIMATED_THEME_MODE: PropertyId =
    PropertyId::from_canonical_name(PROPERTY_ANIMATED_THEME_MODE_NAME);
/// Stable animated-theme duration property identity.
pub const PROPERTY_ANIMATED_THEME_DURATION_MILLIS: PropertyId =
    PropertyId::from_canonical_name(PROPERTY_ANIMATED_THEME_DURATION_MILLIS_NAME);
/// Stable animated-theme curve-tag property identity.
pub const PROPERTY_ANIMATED_THEME_CURVE: PropertyId =
    PropertyId::from_canonical_name(PROPERTY_ANIMATED_THEME_CURVE_NAME);
/// Stable animated-theme cubic-bezier x1 property identity.
pub const PROPERTY_ANIMATED_THEME_CURVE_X1: PropertyId =
    PropertyId::from_canonical_name(PROPERTY_ANIMATED_THEME_CURVE_X1_NAME);
/// Stable animated-theme cubic-bezier y1 property identity.
pub const PROPERTY_ANIMATED_THEME_CURVE_Y1: PropertyId =
    PropertyId::from_canonical_name(PROPERTY_ANIMATED_THEME_CURVE_Y1_NAME);
/// Stable animated-theme cubic-bezier x2 property identity.
pub const PROPERTY_ANIMATED_THEME_CURVE_X2: PropertyId =
    PropertyId::from_canonical_name(PROPERTY_ANIMATED_THEME_CURVE_X2_NAME);
/// Stable animated-theme cubic-bezier y2 property identity.
pub const PROPERTY_ANIMATED_THEME_CURVE_Y2: PropertyId =
    PropertyId::from_canonical_name(PROPERTY_ANIMATED_THEME_CURVE_Y2_NAME);

const fn builtin_widget_schema(canonical_name: &'static str) -> WidgetSchemaMetadata<'static> {
    WidgetSchemaMetadata::from_canonical_name(
        canonical_name,
        BUILTIN_WIDGET_SCHEMA_VERSION,
        BUILTIN_WIDGET_SCHEMA_VERSION,
    )
}

/// Optional logical width used by the container schema.
pub const PROPERTY_CONTAINER_WIDTH: PropertyId =
    PropertyId::from_canonical_name(PROPERTY_CONTAINER_WIDTH_NAME);
/// Optional vertical alignment used by the column schema.
pub const PROPERTY_COLUMN_VERTICAL_ALIGNMENT: PropertyId =
    PropertyId::from_canonical_name(PROPERTY_COLUMN_VERTICAL_ALIGNMENT_NAME);
/// Optional horizontal alignment used by the column schema.
pub const PROPERTY_COLUMN_HORIZONTAL_ALIGNMENT: PropertyId =
    PropertyId::from_canonical_name(PROPERTY_COLUMN_HORIZONTAL_ALIGNMENT_NAME);
/// Optional main-axis placement used by the column schema.
pub const PROPERTY_COLUMN_JUSTIFY_CONTENT: PropertyId =
    PropertyId::from_canonical_name(PROPERTY_COLUMN_JUSTIFY_CONTENT_NAME);
/// Optional spacing payload used by the column schema.
pub const PROPERTY_COLUMN_GAPS: PropertyId =
    PropertyId::from_canonical_name(PROPERTY_COLUMN_GAPS_NAME);
/// Optional overflow behavior used by the column schema.
pub const PROPERTY_COLUMN_OVERFLOW: PropertyId =
    PropertyId::from_canonical_name(PROPERTY_COLUMN_OVERFLOW_NAME);
/// Optional vertical alignment used by the row schema.
pub const PROPERTY_ROW_VERTICAL_ALIGNMENT: PropertyId =
    PropertyId::from_canonical_name(PROPERTY_ROW_VERTICAL_ALIGNMENT_NAME);
/// Optional horizontal alignment used by the row schema.
pub const PROPERTY_ROW_HORIZONTAL_ALIGNMENT: PropertyId =
    PropertyId::from_canonical_name(PROPERTY_ROW_HORIZONTAL_ALIGNMENT_NAME);
/// Optional main-axis placement used by the row schema.
pub const PROPERTY_ROW_JUSTIFY_CONTENT: PropertyId =
    PropertyId::from_canonical_name(PROPERTY_ROW_JUSTIFY_CONTENT_NAME);
/// Optional spacing payload used by the row schema.
pub const PROPERTY_ROW_GAPS: PropertyId = PropertyId::from_canonical_name(PROPERTY_ROW_GAPS_NAME);
/// Optional overflow behavior used by the row schema.
pub const PROPERTY_ROW_OVERFLOW: PropertyId =
    PropertyId::from_canonical_name(PROPERTY_ROW_OVERFLOW_NAME);
/// Optional logical height used by the container schema.
pub const PROPERTY_CONTAINER_HEIGHT: PropertyId =
    PropertyId::from_canonical_name(PROPERTY_CONTAINER_HEIGHT_NAME);
/// Optional packed RGBA background used by the container schema.
pub const PROPERTY_CONTAINER_COLOR: PropertyId =
    PropertyId::from_canonical_name(PROPERTY_CONTAINER_COLOR_NAME);
/// Optional versioned spacing payload used by the container schema.
pub const PROPERTY_CONTAINER_PADDING: PropertyId =
    PropertyId::from_canonical_name(PROPERTY_CONTAINER_PADDING_NAME);
/// Optional versioned outer-spacing payload used by the container schema.
pub const PROPERTY_CONTAINER_MARGIN: PropertyId =
    PropertyId::from_canonical_name(PROPERTY_CONTAINER_MARGIN_NAME);
/// Optional versioned decoration payload used by the container schema.
pub const PROPERTY_CONTAINER_BOX_DECORATION: PropertyId =
    PropertyId::from_canonical_name(PROPERTY_CONTAINER_BOX_DECORATION_NAME);
/// Optional logical width used by the sized-box schema.
pub const PROPERTY_SIZED_BOX_WIDTH: PropertyId =
    PropertyId::from_canonical_name(PROPERTY_SIZED_BOX_WIDTH_NAME);
/// Optional logical height used by the sized-box schema.
pub const PROPERTY_SIZED_BOX_HEIGHT: PropertyId =
    PropertyId::from_canonical_name(PROPERTY_SIZED_BOX_HEIGHT_NAME);
/// Required string-table reference used by the text schema.
pub const PROPERTY_TEXT_CONTENT: PropertyId =
    PropertyId::from_canonical_name(PROPERTY_TEXT_CONTENT_NAME);
/// Optional versioned style payload used by the text schema.
pub const PROPERTY_TEXT_STYLE: PropertyId =
    PropertyId::from_canonical_name(PROPERTY_TEXT_STYLE_NAME);
/// Optional text alignment used by the text schema.
pub const PROPERTY_TEXT_ALIGN: PropertyId =
    PropertyId::from_canonical_name(PROPERTY_TEXT_ALIGN_NAME);
/// Optional line-height value used by the text schema.
pub const PROPERTY_TEXT_LINE_HEIGHT: PropertyId =
    PropertyId::from_canonical_name(PROPERTY_TEXT_LINE_HEIGHT_NAME);
/// Optional first-line indentation used by the text schema.
pub const PROPERTY_TEXT_INDENT: PropertyId =
    PropertyId::from_canonical_name(PROPERTY_TEXT_INDENT_NAME);

/// Canonical identity retained for the Container width property.
pub const PROPERTY_CONTAINER_WIDTH_NAME: &str =
    "aimer.property:aimer_container::single_child::Container:width";
/// Canonical identity for the Column vertical-alignment property.
pub const PROPERTY_COLUMN_VERTICAL_ALIGNMENT_NAME: &str =
    "aimer.property:aimer_flex::flex::Column:vertical_alignment";
/// Canonical identity for the Column horizontal-alignment property.
pub const PROPERTY_COLUMN_HORIZONTAL_ALIGNMENT_NAME: &str =
    "aimer.property:aimer_flex::flex::Column:horizontal_alignment";
/// Canonical identity for the Column justify-content property.
pub const PROPERTY_COLUMN_JUSTIFY_CONTENT_NAME: &str =
    "aimer.property:aimer_flex::flex::Column:justify_content";
/// Canonical identity for the Column gaps property.
pub const PROPERTY_COLUMN_GAPS_NAME: &str = "aimer.property:aimer_flex::flex::Column:gaps";
/// Canonical identity for the Column overflow property.
pub const PROPERTY_COLUMN_OVERFLOW_NAME: &str = "aimer.property:aimer_flex::flex::Column:overflow";
/// Canonical identity for the Row vertical-alignment property.
pub const PROPERTY_ROW_VERTICAL_ALIGNMENT_NAME: &str =
    "aimer.property:aimer_flex::flex::Row:vertical_alignment";
/// Canonical identity for the Row horizontal-alignment property.
pub const PROPERTY_ROW_HORIZONTAL_ALIGNMENT_NAME: &str =
    "aimer.property:aimer_flex::flex::Row:horizontal_alignment";
/// Canonical identity for the Row justify-content property.
pub const PROPERTY_ROW_JUSTIFY_CONTENT_NAME: &str =
    "aimer.property:aimer_flex::flex::Row:justify_content";
/// Canonical identity for the Row gaps property.
pub const PROPERTY_ROW_GAPS_NAME: &str = "aimer.property:aimer_flex::flex::Row:gaps";
/// Canonical identity for the Row overflow property.
pub const PROPERTY_ROW_OVERFLOW_NAME: &str = "aimer.property:aimer_flex::flex::Row:overflow";
/// Canonical identity retained for the Container height property.
pub const PROPERTY_CONTAINER_HEIGHT_NAME: &str =
    "aimer.property:aimer_container::single_child::Container:height";
/// Canonical identity retained for the Container color property.
pub const PROPERTY_CONTAINER_COLOR_NAME: &str =
    "aimer.property:aimer_container::single_child::Container:color";
/// Canonical identity retained for the Container padding property.
pub const PROPERTY_CONTAINER_PADDING_NAME: &str =
    "aimer.property:aimer_container::single_child::Container:padding";
/// Canonical identity retained for the Container margin property.
pub const PROPERTY_CONTAINER_MARGIN_NAME: &str =
    "aimer.property:aimer_container::single_child::Container:margin";
/// Canonical identity retained for the Container decoration property.
pub const PROPERTY_CONTAINER_BOX_DECORATION_NAME: &str =
    "aimer.property:aimer_container::single_child::Container:box_decoration";
/// Canonical identity retained for the SizedBox width property.
pub const PROPERTY_SIZED_BOX_WIDTH_NAME: &str =
    "aimer.property:aimer_container::single_child::SizedBox:width";
/// Canonical identity retained for the SizedBox height property.
pub const PROPERTY_SIZED_BOX_HEIGHT_NAME: &str =
    "aimer.property:aimer_container::single_child::SizedBox:height";
/// Canonical identity retained for the Text content property.
pub const PROPERTY_TEXT_CONTENT_NAME: &str = "aimer.property:aimer_text::Text:text";
/// Canonical identity for the optional Text style property.
pub const PROPERTY_TEXT_STYLE_NAME: &str = "aimer.property:aimer_text::Text:text_style";
/// Canonical identity for the optional Text alignment property.
pub const PROPERTY_TEXT_ALIGN_NAME: &str = "aimer.property:aimer_text::Text:text_align";
/// Canonical identity for the optional Text line-height property.
pub const PROPERTY_TEXT_LINE_HEIGHT_NAME: &str = "aimer.property:aimer_text::Text:line_height";
/// Canonical identity for the optional Text indentation property.
pub const PROPERTY_TEXT_INDENT_NAME: &str = "aimer.property:aimer_text::Text:text_indent";

/// Canonical identity for the animated-theme value-type property.
pub const PROPERTY_ANIMATED_THEME_TYPE_NAME: &str =
    "aimer.property:aimer_style::AnimatedTheme:theme_type";
/// Canonical identity for the animated-theme codec-version property.
pub const PROPERTY_ANIMATED_THEME_SCHEMA_VERSION_NAME: &str =
    "aimer.property:aimer_style::AnimatedTheme:theme_schema_version";
/// Canonical identity for the animated-theme resolved-value property.
pub const PROPERTY_ANIMATED_THEME_VALUE_NAME: &str =
    "aimer.property:aimer_style::AnimatedTheme:value";
/// Canonical identity for the animated-theme mode property.
pub const PROPERTY_ANIMATED_THEME_MODE_NAME: &str =
    "aimer.property:aimer_style::AnimatedTheme:mode";
/// Canonical identity for the animated-theme duration property.
pub const PROPERTY_ANIMATED_THEME_DURATION_MILLIS_NAME: &str =
    "aimer.property:aimer_style::AnimatedTheme:duration_millis";
/// Canonical identity for the animated-theme curve-tag property.
pub const PROPERTY_ANIMATED_THEME_CURVE_NAME: &str =
    "aimer.property:aimer_style::AnimatedTheme:curve";
/// Canonical identity for the animated-theme cubic-bezier x1 property.
pub const PROPERTY_ANIMATED_THEME_CURVE_X1_NAME: &str =
    "aimer.property:aimer_style::AnimatedTheme:curve_x1";
/// Canonical identity for the animated-theme cubic-bezier y1 property.
pub const PROPERTY_ANIMATED_THEME_CURVE_Y1_NAME: &str =
    "aimer.property:aimer_style::AnimatedTheme:curve_y1";
/// Canonical identity for the animated-theme cubic-bezier x2 property.
pub const PROPERTY_ANIMATED_THEME_CURVE_X2_NAME: &str =
    "aimer.property:aimer_style::AnimatedTheme:curve_x2";
/// Canonical identity for the animated-theme cubic-bezier y2 property.
pub const PROPERTY_ANIMATED_THEME_CURVE_Y2_NAME: &str =
    "aimer.property:aimer_style::AnimatedTheme:curve_y2";

/// Primary completed-press event emitted by the button schema.
pub const EVENT_BUTTON_PRESS: EventId = EventId::from_canonical_name(EVENT_BUTTON_PRESS_NAME);
/// Recognized long-press event emitted by the button schema.
pub const EVENT_BUTTON_LONG_PRESS: EventId =
    EventId::from_canonical_name(EVENT_BUTTON_LONG_PRESS_NAME);
/// Completed double-press event emitted by the button schema.
pub const EVENT_BUTTON_DOUBLE_PRESS: EventId =
    EventId::from_canonical_name(EVENT_BUTTON_DOUBLE_PRESS_NAME);
/// Completed secondary-button press event emitted by the button schema.
pub const EVENT_BUTTON_RIGHT_PRESS: EventId =
    EventId::from_canonical_name(EVENT_BUTTON_RIGHT_PRESS_NAME);

/// Canonical identity retained for the Button press callback.
pub const EVENT_BUTTON_PRESS_NAME: &str = "aimer.event:aimer_input::Button:on_press";
/// Canonical identity retained for the Button long-press callback.
pub const EVENT_BUTTON_LONG_PRESS_NAME: &str = "aimer.event:aimer_input::Button:on_long_press";
/// Canonical identity retained for the Button double-press callback.
pub const EVENT_BUTTON_DOUBLE_PRESS_NAME: &str = "aimer.event:aimer_input::Button:on_double_press";
/// Canonical identity retained for the Button right-press callback.
pub const EVENT_BUTTON_RIGHT_PRESS_NAME: &str = "aimer.event:aimer_input::Button:on_right_press";

/// Primary completed-press event emitted by the gesture detector schema.
pub const EVENT_GESTURE_DETECTOR_TAP: EventId =
    EventId::from_canonical_name(EVENT_GESTURE_DETECTOR_TAP_NAME);
/// Completed double-press event emitted by the gesture detector schema.
pub const EVENT_GESTURE_DETECTOR_DOUBLE_PRESS: EventId =
    EventId::from_canonical_name(EVENT_GESTURE_DETECTOR_DOUBLE_PRESS_NAME);
/// Recognized long-press event emitted by the gesture detector schema.
pub const EVENT_GESTURE_DETECTOR_LONG_PRESS: EventId =
    EventId::from_canonical_name(EVENT_GESTURE_DETECTOR_LONG_PRESS_NAME);
/// Completed drag event emitted by the gesture detector schema.
pub const EVENT_GESTURE_DETECTOR_DRAG_END: EventId =
    EventId::from_canonical_name(EVENT_GESTURE_DETECTOR_DRAG_END_NAME);
/// Completed secondary-button press event emitted by the gesture detector schema.
pub const EVENT_GESTURE_DETECTOR_RIGHT_TAP: EventId =
    EventId::from_canonical_name(EVENT_GESTURE_DETECTOR_RIGHT_TAP_NAME);
/// Canonical identity for the gesture detector's primary tap callback.
pub const EVENT_GESTURE_DETECTOR_TAP_NAME: &str = "aimer.event:aimer_input::GestureDetector:on_tap";
/// Canonical identity for the gesture detector's double-press callback.
pub const EVENT_GESTURE_DETECTOR_DOUBLE_PRESS_NAME: &str =
    "aimer.event:aimer_input::GestureDetector:on_double_press";
/// Canonical identity for the gesture detector's long-press callback.
pub const EVENT_GESTURE_DETECTOR_LONG_PRESS_NAME: &str =
    "aimer.event:aimer_input::GestureDetector:on_long_press";
/// Canonical identity for the gesture detector's drag-end callback.
pub const EVENT_GESTURE_DETECTOR_DRAG_END_NAME: &str =
    "aimer.event:aimer_input::GestureDetector:on_drag_end";
/// Canonical identity for the gesture detector's secondary-tap callback.
pub const EVENT_GESTURE_DETECTOR_RIGHT_TAP_NAME: &str =
    "aimer.event:aimer_input::GestureDetector:on_right_tap";
/// Hover-enter event emitted by the mouse-region schema.
pub const EVENT_MOUSE_REGION_HOVER_ENTER: EventId =
    EventId::from_canonical_name(EVENT_MOUSE_REGION_HOVER_ENTER_NAME);
/// Hover-exit event emitted by the mouse-region schema.
pub const EVENT_MOUSE_REGION_HOVER_EXIT: EventId =
    EventId::from_canonical_name(EVENT_MOUSE_REGION_HOVER_EXIT_NAME);
/// Canonical identity for the mouse region's hover-enter callback.
pub const EVENT_MOUSE_REGION_HOVER_ENTER_NAME: &str =
    "aimer.event:aimer_input::MouseRegion:on_hover_enter";
/// Canonical identity for the mouse region's hover-exit callback.
pub const EVENT_MOUSE_REGION_HOVER_EXIT_NAME: &str =
    "aimer.event:aimer_input::MouseRegion:on_hover_exit";

/// Canonical identity for the Button's normal decoration property.
pub const PROPERTY_BUTTON_DECORATION_NAME: &str = "aimer.property:aimer_input::Button:decoration";
/// Stable identity for the Button's normal decoration property.
pub const PROPERTY_BUTTON_DECORATION: PropertyId =
    PropertyId::from_canonical_name(PROPERTY_BUTTON_DECORATION_NAME);

const FLEX_COLUMN_PROPERTIES: &[PropertySchemaMetadata<'static>] = &[
    PropertySchemaMetadata::from_canonical_name(
        PROPERTY_COLUMN_VERTICAL_ALIGNMENT_NAME,
        PropertyValueKind::I64,
    )
    .optional(),
    PropertySchemaMetadata::from_canonical_name(
        PROPERTY_COLUMN_HORIZONTAL_ALIGNMENT_NAME,
        PropertyValueKind::I64,
    )
    .optional(),
    PropertySchemaMetadata::from_canonical_name(
        PROPERTY_COLUMN_JUSTIFY_CONTENT_NAME,
        PropertyValueKind::I64,
    )
    .optional(),
    PropertySchemaMetadata::from_canonical_name(
        PROPERTY_COLUMN_GAPS_NAME,
        PropertyValueKind::BlobRef,
    )
    .optional()
    .with_value_schema(ValueSchemaMetadata::from_canonical_name(
        LAYOUT_SPACING_VALUE_NAME,
        LAYOUT_SPACING_VALUE_VERSION,
        LAYOUT_SPACING_VALUE_MAXIMUM_ENCODED_BYTES,
    )),
    PropertySchemaMetadata::from_canonical_name(
        PROPERTY_COLUMN_OVERFLOW_NAME,
        PropertyValueKind::I64,
    )
    .optional(),
];
const FLEX_ROW_PROPERTIES: &[PropertySchemaMetadata<'static>] = &[
    PropertySchemaMetadata::from_canonical_name(
        PROPERTY_ROW_VERTICAL_ALIGNMENT_NAME,
        PropertyValueKind::I64,
    )
    .optional(),
    PropertySchemaMetadata::from_canonical_name(
        PROPERTY_ROW_HORIZONTAL_ALIGNMENT_NAME,
        PropertyValueKind::I64,
    )
    .optional(),
    PropertySchemaMetadata::from_canonical_name(
        PROPERTY_ROW_JUSTIFY_CONTENT_NAME,
        PropertyValueKind::I64,
    )
    .optional(),
    PropertySchemaMetadata::from_canonical_name(PROPERTY_ROW_GAPS_NAME, PropertyValueKind::BlobRef)
        .optional()
        .with_value_schema(ValueSchemaMetadata::from_canonical_name(
            LAYOUT_SPACING_VALUE_NAME,
            LAYOUT_SPACING_VALUE_VERSION,
            LAYOUT_SPACING_VALUE_MAXIMUM_ENCODED_BYTES,
        )),
    PropertySchemaMetadata::from_canonical_name(PROPERTY_ROW_OVERFLOW_NAME, PropertyValueKind::I64)
        .optional(),
];
const CONTAINER_PROPERTIES: &[PropertySchemaMetadata<'static>] = &[
    PropertySchemaMetadata::from_canonical_name(
        PROPERTY_CONTAINER_WIDTH_NAME,
        PropertyValueKind::BlobRef,
    )
    .optional()
    .with_value_schema(ValueSchemaMetadata::from_canonical_name(
        DIMENSION_VALUE_NAME,
        DIMENSION_VALUE_VERSION,
        DIMENSION_VALUE_MAXIMUM_ENCODED_BYTES,
    )),
    PropertySchemaMetadata::from_canonical_name(
        PROPERTY_CONTAINER_HEIGHT_NAME,
        PropertyValueKind::BlobRef,
    )
    .optional()
    .with_value_schema(ValueSchemaMetadata::from_canonical_name(
        DIMENSION_VALUE_NAME,
        DIMENSION_VALUE_VERSION,
        DIMENSION_VALUE_MAXIMUM_ENCODED_BYTES,
    )),
    PropertySchemaMetadata::from_canonical_name(
        PROPERTY_CONTAINER_PADDING_NAME,
        PropertyValueKind::BlobRef,
    )
    .optional()
    .with_value_schema(ValueSchemaMetadata::from_canonical_name(
        LAYOUT_SPACING_VALUE_NAME,
        LAYOUT_SPACING_VALUE_VERSION,
        LAYOUT_SPACING_VALUE_MAXIMUM_ENCODED_BYTES,
    )),
    PropertySchemaMetadata::from_canonical_name(
        PROPERTY_CONTAINER_MARGIN_NAME,
        PropertyValueKind::BlobRef,
    )
    .optional()
    .with_value_schema(ValueSchemaMetadata::from_canonical_name(
        LAYOUT_SPACING_VALUE_NAME,
        LAYOUT_SPACING_VALUE_VERSION,
        LAYOUT_SPACING_VALUE_MAXIMUM_ENCODED_BYTES,
    )),
    PropertySchemaMetadata::from_canonical_name(
        PROPERTY_CONTAINER_BOX_DECORATION_NAME,
        PropertyValueKind::BlobRef,
    )
    .optional()
    .with_value_schema(ValueSchemaMetadata::from_canonical_name(
        BOX_DECORATION_VALUE_NAME,
        BOX_DECORATION_VALUE_VERSION,
        BOX_DECORATION_VALUE_MAXIMUM_ENCODED_BYTES,
    )),
    PropertySchemaMetadata::from_canonical_name(
        PROPERTY_CONTAINER_COLOR_NAME,
        PropertyValueKind::Rgba,
    )
    .optional(),
];
const SIZED_BOX_PROPERTIES: &[PropertySchemaMetadata<'static>] = &[
    PropertySchemaMetadata::from_canonical_name(
        PROPERTY_SIZED_BOX_WIDTH_NAME,
        PropertyValueKind::BlobRef,
    )
    .optional()
    .with_value_schema(ValueSchemaMetadata::from_canonical_name(
        DIMENSION_VALUE_NAME,
        DIMENSION_VALUE_VERSION,
        DIMENSION_VALUE_MAXIMUM_ENCODED_BYTES,
    )),
    PropertySchemaMetadata::from_canonical_name(
        PROPERTY_SIZED_BOX_HEIGHT_NAME,
        PropertyValueKind::BlobRef,
    )
    .optional()
    .with_value_schema(ValueSchemaMetadata::from_canonical_name(
        DIMENSION_VALUE_NAME,
        DIMENSION_VALUE_VERSION,
        DIMENSION_VALUE_MAXIMUM_ENCODED_BYTES,
    )),
];
const TEXT_PROPERTIES: &[PropertySchemaMetadata<'static>] = &[
    PropertySchemaMetadata::from_canonical_name(
        PROPERTY_TEXT_CONTENT_NAME,
        PropertyValueKind::StringRef,
    ),
    PropertySchemaMetadata::from_canonical_name(PROPERTY_TEXT_ALIGN_NAME, PropertyValueKind::I64)
        .optional(),
    PropertySchemaMetadata::from_canonical_name(
        PROPERTY_TEXT_STYLE_NAME,
        PropertyValueKind::BlobRef,
    )
    .optional()
    .with_value_schema(ValueSchemaMetadata::from_canonical_name(
        TEXT_STYLE_VALUE_NAME,
        TEXT_STYLE_VALUE_VERSION,
        TEXT_STYLE_VALUE_MAXIMUM_ENCODED_BYTES,
    )),
    PropertySchemaMetadata::from_canonical_name(
        PROPERTY_TEXT_LINE_HEIGHT_NAME,
        PropertyValueKind::BlobRef,
    )
    .optional()
    .with_value_schema(ValueSchemaMetadata::from_canonical_name(
        LINE_HEIGHT_VALUE_NAME,
        LINE_HEIGHT_VALUE_VERSION,
        LINE_HEIGHT_VALUE_MAXIMUM_ENCODED_BYTES,
    )),
    PropertySchemaMetadata::from_canonical_name(PROPERTY_TEXT_INDENT_NAME, PropertyValueKind::F64)
        .optional(),
];
const PROVIDER_VALUE_SCHEMA: ValueSchemaMetadata<'static> =
    ValueSchemaMetadata::from_canonical_name(
        "aimer.value:aimer_provider::ProviderSnapshot",
        Version::new(1, 0),
        PROVIDER_VALUE_MAXIMUM_ENCODED_BYTES,
    );
const PROVIDER_PROPERTIES: &[PropertySchemaMetadata<'static>] = &[
    PropertySchemaMetadata::from_canonical_name(
        PROPERTY_PROVIDER_TYPE_NAME,
        PropertyValueKind::I64,
    ),
    PropertySchemaMetadata::from_canonical_name(
        PROPERTY_PROVIDER_SCHEMA_VERSION_NAME,
        PropertyValueKind::I64,
    ),
    PropertySchemaMetadata::from_canonical_name(
        PROPERTY_PROVIDER_VALUE_NAME,
        PropertyValueKind::BlobRef,
    )
    .with_value_schema(PROVIDER_VALUE_SCHEMA),
];
const THEME_VALUE_SCHEMA: ValueSchemaMetadata<'static> = ValueSchemaMetadata::from_canonical_name(
    "aimer.value:aimer_style::ThemeValue",
    Version::new(1, 0),
    THEME_VALUE_MAXIMUM_ENCODED_BYTES,
);
const ANIMATED_THEME_PROPERTIES: &[PropertySchemaMetadata<'static>] = &[
    PropertySchemaMetadata::from_canonical_name(
        PROPERTY_ANIMATED_THEME_TYPE_NAME,
        PropertyValueKind::I64,
    ),
    PropertySchemaMetadata::from_canonical_name(
        PROPERTY_ANIMATED_THEME_SCHEMA_VERSION_NAME,
        PropertyValueKind::I64,
    ),
    PropertySchemaMetadata::from_canonical_name(
        PROPERTY_ANIMATED_THEME_VALUE_NAME,
        PropertyValueKind::BlobRef,
    )
    .with_value_schema(THEME_VALUE_SCHEMA),
    PropertySchemaMetadata::from_canonical_name(
        PROPERTY_ANIMATED_THEME_MODE_NAME,
        PropertyValueKind::I64,
    ),
    PropertySchemaMetadata::from_canonical_name(
        PROPERTY_ANIMATED_THEME_DURATION_MILLIS_NAME,
        PropertyValueKind::I64,
    ),
    PropertySchemaMetadata::from_canonical_name(
        PROPERTY_ANIMATED_THEME_CURVE_NAME,
        PropertyValueKind::I64,
    ),
    PropertySchemaMetadata::from_canonical_name(
        PROPERTY_ANIMATED_THEME_CURVE_X1_NAME,
        PropertyValueKind::F64,
    )
    .optional(),
    PropertySchemaMetadata::from_canonical_name(
        PROPERTY_ANIMATED_THEME_CURVE_Y1_NAME,
        PropertyValueKind::F64,
    )
    .optional(),
    PropertySchemaMetadata::from_canonical_name(
        PROPERTY_ANIMATED_THEME_CURVE_X2_NAME,
        PropertyValueKind::F64,
    )
    .optional(),
    PropertySchemaMetadata::from_canonical_name(
        PROPERTY_ANIMATED_THEME_CURVE_Y2_NAME,
        PropertyValueKind::F64,
    )
    .optional(),
];
const BUTTON_CALLBACKS: &[CallbackSchemaMetadata<'static>] = &[
    CallbackSchemaMetadata::from_canonical_name(
        EVENT_BUTTON_PRESS_NAME,
        BUILTIN_WIDGET_SCHEMA_VERSION,
        1,
    )
    .with_async_schema(AsyncCallbackSchemaMetadata::new(
        Version::new(1, 0),
        64,
        4_096,
    )),
    CallbackSchemaMetadata::from_canonical_name(
        EVENT_BUTTON_LONG_PRESS_NAME,
        BUILTIN_WIDGET_SCHEMA_VERSION,
        1,
    )
    .with_async_schema(AsyncCallbackSchemaMetadata::new(
        Version::new(1, 0),
        64,
        4_096,
    )),
    CallbackSchemaMetadata::from_canonical_name(
        EVENT_BUTTON_DOUBLE_PRESS_NAME,
        BUILTIN_WIDGET_SCHEMA_VERSION,
        1,
    )
    .with_async_schema(AsyncCallbackSchemaMetadata::new(
        Version::new(1, 0),
        64,
        4_096,
    )),
    CallbackSchemaMetadata::from_canonical_name(
        EVENT_BUTTON_RIGHT_PRESS_NAME,
        BUILTIN_WIDGET_SCHEMA_VERSION,
        1,
    )
    .with_async_schema(AsyncCallbackSchemaMetadata::new(
        Version::new(1, 0),
        64,
        4_096,
    )),
];
const BUTTON_PROPERTIES: &[PropertySchemaMetadata<'static>] =
    &[PropertySchemaMetadata::from_canonical_name(
        PROPERTY_BUTTON_DECORATION_NAME,
        PropertyValueKind::BlobRef,
    )
    .optional()
    .with_value_schema(ValueSchemaMetadata::from_canonical_name(
        BOX_DECORATION_VALUE_NAME,
        BOX_DECORATION_VALUE_VERSION,
        BOX_DECORATION_VALUE_MAXIMUM_ENCODED_BYTES,
    ))];

const GESTURE_DETECTOR_CALLBACKS: &[CallbackSchemaMetadata<'static>] = &[
    CallbackSchemaMetadata::from_canonical_name(
        EVENT_GESTURE_DETECTOR_TAP_NAME,
        BUILTIN_WIDGET_SCHEMA_VERSION,
        1,
    )
    .with_async_schema(AsyncCallbackSchemaMetadata::new(
        Version::new(1, 0),
        64,
        4_096,
    )),
    CallbackSchemaMetadata::from_canonical_name(
        EVENT_GESTURE_DETECTOR_DOUBLE_PRESS_NAME,
        BUILTIN_WIDGET_SCHEMA_VERSION,
        1,
    )
    .with_async_schema(AsyncCallbackSchemaMetadata::new(
        Version::new(1, 0),
        64,
        4_096,
    )),
    CallbackSchemaMetadata::from_canonical_name(
        EVENT_GESTURE_DETECTOR_LONG_PRESS_NAME,
        BUILTIN_WIDGET_SCHEMA_VERSION,
        1,
    )
    .with_async_schema(AsyncCallbackSchemaMetadata::new(
        Version::new(1, 0),
        64,
        4_096,
    )),
    CallbackSchemaMetadata::from_canonical_name(
        EVENT_GESTURE_DETECTOR_DRAG_END_NAME,
        BUILTIN_WIDGET_SCHEMA_VERSION,
        1,
    )
    .with_async_schema(AsyncCallbackSchemaMetadata::new(
        Version::new(1, 0),
        64,
        4_096,
    )),
    CallbackSchemaMetadata::from_canonical_name(
        EVENT_GESTURE_DETECTOR_RIGHT_TAP_NAME,
        BUILTIN_WIDGET_SCHEMA_VERSION,
        1,
    )
    .with_async_schema(AsyncCallbackSchemaMetadata::new(
        Version::new(1, 0),
        64,
        4_096,
    )),
];
const MOUSE_REGION_CALLBACKS: &[CallbackSchemaMetadata<'static>] = &[
    CallbackSchemaMetadata::from_canonical_name(
        EVENT_MOUSE_REGION_HOVER_ENTER_NAME,
        BUILTIN_WIDGET_SCHEMA_VERSION,
        1,
    )
    .with_async_schema(AsyncCallbackSchemaMetadata::new(
        Version::new(1, 0),
        64,
        4_096,
    )),
    CallbackSchemaMetadata::from_canonical_name(
        EVENT_MOUSE_REGION_HOVER_EXIT_NAME,
        BUILTIN_WIDGET_SCHEMA_VERSION,
        1,
    )
    .with_async_schema(AsyncCallbackSchemaMetadata::new(
        Version::new(1, 0),
        64,
        4_096,
    )),
];

/// Complete portable metadata for every built-in AWIR widget schema.
pub const BUILTIN_PORTABLE_WIDGET_SCHEMAS: [PortableWidgetSchemaMetadata<'static>; 10] = [
    PortableWidgetSchemaMetadata::new(
        builtin_widget_schema(WIDGET_COLUMN_NAME),
        FLEX_COLUMN_PROPERTIES,
        &[],
        ChildCardinality::new(0, u32::MAX),
    ),
    PortableWidgetSchemaMetadata::new(
        builtin_widget_schema(WIDGET_ROW_NAME),
        FLEX_ROW_PROPERTIES,
        &[],
        ChildCardinality::new(0, u32::MAX),
    ),
    PortableWidgetSchemaMetadata::new(
        builtin_widget_schema(WIDGET_CONTAINER_NAME),
        CONTAINER_PROPERTIES,
        &[],
        ChildCardinality::exactly(1),
    ),
    PortableWidgetSchemaMetadata::new(
        builtin_widget_schema(WIDGET_SIZED_BOX_NAME),
        SIZED_BOX_PROPERTIES,
        &[],
        ChildCardinality::none(),
    ),
    PortableWidgetSchemaMetadata::new(
        builtin_widget_schema(WIDGET_TEXT_NAME),
        TEXT_PROPERTIES,
        &[],
        ChildCardinality::none(),
    ),
    PortableWidgetSchemaMetadata::new(
        builtin_widget_schema(WIDGET_BUTTON_NAME),
        BUTTON_PROPERTIES,
        BUTTON_CALLBACKS,
        ChildCardinality::exactly(1),
    ),
    PortableWidgetSchemaMetadata::new(
        builtin_widget_schema(WIDGET_PROVIDER_NAME),
        PROVIDER_PROPERTIES,
        &[],
        ChildCardinality::exactly(1),
    ),
    PortableWidgetSchemaMetadata::new(
        builtin_widget_schema(WIDGET_ANIMATED_THEME_NAME),
        ANIMATED_THEME_PROPERTIES,
        &[],
        ChildCardinality::exactly(1),
    ),
    PortableWidgetSchemaMetadata::new(
        builtin_widget_schema(WIDGET_GESTURE_DETECTOR_NAME),
        &[],
        GESTURE_DETECTOR_CALLBACKS,
        ChildCardinality::exactly(1),
    ),
    PortableWidgetSchemaMetadata::new(
        builtin_widget_schema(WIDGET_MOUSE_REGION_NAME),
        &[],
        MOUSE_REGION_CALLBACKS,
        ChildCardinality::exactly(1),
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stable_schema_hash64;

    #[test]
    fn widget_property_and_event_ids_are_canonical_and_distinct() {
        assert_eq!(
            WIDGET_CONTAINER.value(),
            stable_schema_hash64("aimer.widget:aimer_container::single_child::Container")
        );
        assert_eq!(
            PROPERTY_CONTAINER_WIDTH.value(),
            stable_schema_hash64("aimer.property:aimer_container::single_child::Container:width")
        );
        assert_ne!(PROPERTY_CONTAINER_WIDTH, PROPERTY_SIZED_BOX_WIDTH);
        assert_ne!(PROPERTY_CONTAINER_HEIGHT, PROPERTY_SIZED_BOX_HEIGHT);
        assert_ne!(EVENT_BUTTON_PRESS, EVENT_BUTTON_LONG_PRESS);
        assert_ne!(EVENT_BUTTON_PRESS, EVENT_BUTTON_DOUBLE_PRESS);
        assert_ne!(EVENT_BUTTON_PRESS, EVENT_BUTTON_RIGHT_PRESS);
        for schema in BUILTIN_PORTABLE_WIDGET_SCHEMAS {
            let widget = schema.widget();
            assert_eq!(
                crate::validate_widget_schema_metadata(core::slice::from_ref(&widget)),
                Ok(())
            );
        }
    }
}
