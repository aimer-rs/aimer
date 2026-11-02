pub mod raw_text;
pub mod selectable_text;

use std::cell::Cell;

use aimer_macro::PortableWidget;
use aimer_style::{LineHeight, TextAlign, TextOverflow, TextShadow, TextStyle, TextTransform};
use aimer_widget::base::BuildContext;
#[cfg(feature = "portable-guest")]
use aimer_widget::portable::{PortableBuildContext, PortableBuildError, SourceFingerprint};
use aimer_widget::{AnyElement, Element, LayoutCache, Widget};

use crate::selection::selectable::SelectionScope;
use crate::text::raw_text::RawTextWidget;
use crate::text::selectable_text::RawSelectableText;
use crate::text_source::TextSource;

/// The highlight color a selectable `Text` falls back to when it somehow sits
/// outside a region; regions always supply their own.
const DEFAULT_SELECTION_COLOR: aimer_widget::base::Color =
    aimer_widget::base::Color::Rgba(51, 153, 255, 96);

/// Displays a single run of styled text.
///
/// Text uses [`TextStyle::default`] and [`TextAlign::default`] unless replaced.
/// Overflow behavior comes from the active style; use [`Text::wrapped`] or
/// [`Text::ellipsis`] for the common modes. Unlike [`crate::RichText`], this
/// widget does not provide spans, links, or selection. Run-level properties
/// such as transformation, spacing, decoration, and glyph shadow belong to
/// [`TextStyle`]; the forwarding builders below only update that canonical
/// style. Line height and first-line indentation belong to the paragraph and
/// affect layout, wrapping, hit geometry, and painting together.
///
/// # Example
///
/// ```
/// use aimer_style::{TextAlign, TextStyle};
/// use aimer_text::Text;
///
/// let title = Text::new("Aimer")
///                 .text_align(TextAlign::MidCenter)
///                 .text_style(TextStyle::default())
///                 .wrapped();
/// ```
#[allow(dead_code)]
#[derive(PortableWidget)]
#[portable_widget(
    id = "aimer_text::Text",
    validate = validate_portable_text,
    materializer = materialize_portable_text
)]
pub struct Text {
    text: TextSource,
    #[portable_optional]
    text_align: TextAlign,
    #[portable_optional]
    text_style: TextStyle,
    #[portable_optional]
    line_height: LineHeight,
    #[portable_optional]
    text_indent: f32,
}

impl Text {
    /// Creates text containing `text` with default style and alignment.
    ///
    /// Passing a `&'static str` literal stores it directly with no
    /// allocation; passing a `String` or `Rc<str>` behaves as before. See
    /// [`TextSource`] for details.
    #[inline]
    pub fn new(text: impl Into<TextSource>) -> Self {
        Self {
            text: text.into(),
            text_align: TextAlign::default(),
            text_style: TextStyle::default(),
            line_height: LineHeight::default(),
            text_indent: 0.0,
        }
    }

    /// Replaces the displayed string while preserving style and alignment.
    #[inline]
    pub fn text(mut self, text: impl Into<TextSource>) -> Self {
        self.text = text.into();
        self
    }

    /// Sets how laid-out text is aligned within its available width.
    #[inline]
    pub fn text_align(mut self, text_align: TextAlign) -> Self {
        self.text_align = text_align;
        self
    }

    /// Replaces the complete style used for shaping, layout, and painting.
    ///
    /// This includes font attributes, color, decoration, and overflow behavior.
    #[inline]
    pub fn text_style(mut self, text_style: TextStyle) -> Self {
        self.text_style = text_style;
        self
    }

    /// Sets the Unicode transformation applied before shaping.
    #[inline]
    pub fn text_transform(mut self, text_transform: TextTransform) -> Self {
        self.text_style.text_transform = text_transform;
        self
    }

    /// Sets the additional advance between adjacent rendered graphemes.
    #[inline]
    pub fn letter_spacing(mut self, letter_spacing: f32) -> Self {
        self.text_style.letter_spacing = letter_spacing;
        self
    }

    /// Sets the additional advance at whitespace boundaries.
    #[inline]
    pub fn word_spacing(mut self, word_spacing: f32) -> Self {
        self.text_style.word_spacing = word_spacing;
        self
    }

    /// Adds one glyph shadow behind the text.
    #[inline]
    pub fn text_shadow(mut self, text_shadow: TextShadow) -> Self {
        self.text_style.text_shadow = Some(text_shadow);
        self
    }

    /// Removes the glyph shadow from the text.
    #[inline]
    pub fn without_text_shadow(mut self) -> Self {
        self.text_style.text_shadow = None;
        self
    }

    /// Sets the distance between adjacent paragraph baselines.
    #[inline]
    pub fn line_height(mut self, line_height: LineHeight) -> Self {
        self.line_height = line_height;
        self
    }

    /// Sets the first-line paragraph indent in logical pixels.
    ///
    /// A negative value is a hanging indent: the first line starts before the
    /// paragraph's normal origin while subsequent lines use the normal origin.
    #[inline]
    pub fn text_indent(mut self, text_indent: f32) -> Self {
        self.text_indent = text_indent;
        self
    }
    /// Sets overflow behavior on the current style.
    ///
    /// Prefer configuring [`TextStyle::text_overflow`] before passing the style
    /// to [`Text::text_style`].
    #[deprecated(note = "set TextStyle::text_overflow and pass it to Text::text_style")]
    #[inline]
    pub fn text_overflow(mut self, text_overflow: TextOverflow) -> Self {
        self.text_style.text_overflow = text_overflow;
        self
    }

    /// Configures text to wrap onto additional lines when width is constrained.
    #[allow(deprecated)]
    #[inline]
    pub fn wrapped(self) -> Self {
        self.text_overflow(TextOverflow::Wrap)
    }

    /// Configures overflowing text to be truncated with an ellipsis.
    #[allow(deprecated)]
    #[inline]
    pub fn ellipsis(self) -> Self {
        self.text_overflow(TextOverflow::Ellipsis)
    }
}

#[cfg(feature = "portable-guest")]
fn validate_portable_text(
    text: &Text,
    ctx: &PortableBuildContext,
    source: SourceFingerprint,
) -> Result<(), PortableBuildError> {
    let _ = (text, ctx, source);
    Ok(())
}

fn materialize_portable_text(
    document: &aimer_widget::portable::__anteros::WidgetDocumentView<'_>,
    node: aimer_widget::portable::__anteros::WidgetNodeView<'_>,
    children: Vec<aimer_widget::AnyWidget>,
) -> Result<aimer_widget::AnyWidget, aimer_widget::portable::PortableMaterializeError> {
    if !children.is_empty() {
        return Err(
            aimer_widget::portable::PortableMaterializeError::InvalidChildCount {
                expected: 0,
                actual: children.len(),
            },
        );
    }
    let property = aimer_widget::portable::__anteros::PropertyId::from_canonical_name(
        "aimer.property:aimer_text::Text:text",
    );
    let text: TextSource =
        aimer_widget::portable::required_materialized_property(document, &node, property)?;
    let alignment_property = aimer_widget::portable::__anteros::PropertyId::from_canonical_name(
        "aimer.property:aimer_text::Text:text_align",
    );
    let alignment = aimer_widget::portable::optional_materialized_property::<TextAlign>(
        document,
        &node,
        alignment_property,
    )?;
    let style_property = aimer_widget::portable::__anteros::PropertyId::from_canonical_name(
        "aimer.property:aimer_text::Text:text_style",
    );
    let style = aimer_widget::portable::optional_materialized_property::<TextStyle>(
        document,
        &node,
        style_property,
    )?;
    let line_height_property = aimer_widget::portable::__anteros::PropertyId::from_canonical_name(
        "aimer.property:aimer_text::Text:line_height",
    );
    let line_height = aimer_widget::portable::optional_materialized_property::<LineHeight>(
        document,
        &node,
        line_height_property,
    )?;
    let text_indent_property = aimer_widget::portable::__anteros::PropertyId::from_canonical_name(
        "aimer.property:aimer_text::Text:text_indent",
    );
    let text_indent = aimer_widget::portable::optional_materialized_property::<f32>(
        document,
        &node,
        text_indent_property,
    )?;
    let mut widget = Text::new(text);
    if let Some(alignment) = alignment {
        widget = widget.text_align(alignment);
    }
    if let Some(style) = style {
        widget = widget.text_style(style);
    }
    if let Some(line_height) = line_height {
        widget = widget.line_height(line_height);
    }
    if let Some(text_indent) = text_indent {
        widget = widget.text_indent(text_indent);
    }
    Ok(widget.boxed())
}

impl Widget for Text {
    /// Emits the paragraph-backed selectable element inside a
    /// [`SelectionArea`](crate::SelectionArea) and the plain fast path
    /// everywhere else.
    ///
    /// The lookup is a single `TypeId` probe, so a tree without a region pays
    /// nothing for selection.
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        if ctx.get_state::<SelectionScope>().is_some() {
            return RawSelectableText::new(
                ctx,
                self.text,
                self.text_style,
                self.text_align,
                self.line_height,
                self.text_indent,
                DEFAULT_SELECTION_COLOR,
            )
            .boxed();
        }
        RawTextWidget {
            text: self.text,
            text_style: self.text_style,
            text_align: self.text_align,
            line_height: self.line_height,
            text_indent: self.text_indent,
            cache: LayoutCache::new(),
            _typeface: Cell::new(None),
        }
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    #[cfg(feature = "portable-guest")]
    use aimer_anteros::{
        PROPERTY_TEXT_ALIGN, PROPERTY_TEXT_CONTENT, PROPERTY_TEXT_INDENT,
        PROPERTY_TEXT_LINE_HEIGHT, PROPERTY_TEXT_STYLE, PropertyValue, Version, WIDGET_TEXT,
        WidgetDocumentView, WidgetProperty,
    };
    use aimer_attribute::{ResolvedSize, Vec2d};
    use aimer_canvas::{FrameCanvas, InnerCanvas};
    #[cfg(feature = "portable-guest")]
    use aimer_color::prelude::Colors;
    use aimer_cupid::draw_cmd_v2::{DrawCommand, Rect, RenderPaintSource, RenderTree};
    #[cfg(feature = "portable-guest")]
    use aimer_style::{
        FontFamily, FontStyle, FontWeight, TextAlign, TextDecoration, TextDecorationLine,
        TextDecorationStyle, TextOverflow,
    };
    use aimer_style::{LineHeight, TextAlign, TextStyle, TextTransform};
    #[cfg(feature = "portable-guest")]
    use aimer_widget::PortableWidget;
    use aimer_widget::base::{BuildContext, WindowHandle};
    #[cfg(feature = "portable-guest")]
    use aimer_widget::portable::{
        PortableBuildContext, PortableBuildError, PortableLimits, PortableWidgetLimits,
        PortableWidgetResource, SourceFingerprint, StableId128,
    };
    use aimer_widget::{Drawable, LayoutCache, LayoutElement, Rebuildable, Widget};

    use super::{RawTextWidget, Text};
    use crate::selection::selectable::{SelectionCoordinator, SelectionScope};
    use crate::selection::session::SelectionSession;
    use crate::text_source::TextSource;

    /// The debug name of the non-selectable fast path.
    const RAW_TEXT_NAME: &str = "RawTextWidget";

    #[test]
    fn text_builder_keeps_paragraph_values_with_the_widget() {
        let text = Text::new("Aimer")
            .text_style(
                TextStyle::new()
                    .text_transform(TextTransform::Uppercase)
                    .letter_spacing(0.5),
            )
            .line_height(LineHeight::Px(24.0))
            .text_indent(-8.0);

        assert_eq!(text.text_style.text_transform, TextTransform::Uppercase);
        assert_eq!(text.text_style.letter_spacing, 0.5);
        assert_eq!(text.line_height, LineHeight::Px(24.0));
        assert_eq!(text.text_indent, -8.0);
    }

    fn context<'a>(
        canvas: FrameCanvas<'a>,
        runtime: &'a tokio::runtime::Runtime,
    ) -> BuildContext<'a> {
        context_with_scale(canvas, runtime, 1.0)
    }

    fn context_with_scale<'a>(
        canvas: FrameCanvas<'a>,
        runtime: &'a tokio::runtime::Runtime,
        scale: f32,
    ) -> BuildContext<'a> {
        BuildContext::new(
            canvas,
            ResolvedSize {
                width: 200.0 * scale,
                height: 100.0 * scale,
            },
            scale,
            Vec2d::default(),
            Vec2d::default(),
            WindowHandle::headless(
                winit::dpi::PhysicalSize::new((200.0 * scale) as u32, (100.0 * scale) as u32),
                f64::from(scale),
            ),
            runtime.handle().clone(),
        )
    }

    #[cfg(feature = "portable-guest")]
    fn portable_source(value: u8) -> SourceFingerprint {
        SourceFingerprint::new(StableId128::from_bytes([value; 16]))
    }

    #[cfg(feature = "portable-guest")]
    fn portable_limits() -> PortableWidgetLimits {
        PortableWidgetLimits::new(4, 4, 4, 4, 64, 1_024).with_max_blob_bytes(128)
    }

    #[cfg(feature = "portable-guest")]
    fn portable_context(limits: PortableWidgetLimits) -> PortableBuildContext {
        PortableBuildContext::new(7, 11, limits, PortableLimits::new(4, 8, 64, 128, 1_024)).unwrap()
    }

    #[cfg(feature = "portable-guest")]
    fn assert_exact_portable_text(text: Text, expected: &str) {
        let mut ctx = portable_context(portable_limits());
        let node = text.to_portable_node(&mut ctx, portable_source(1)).unwrap();
        assert_eq!(node.index(), 0);
        let document = ctx.finish_document(node).unwrap();
        let bytes = document.encode().unwrap();
        let view = WidgetDocumentView::decode(&bytes, document.model_limits()).unwrap();

        assert_eq!(view.generation_id(), 7);
        assert_eq!(view.document_revision(), 11);
        assert_eq!(view.root_node(), 0);
        assert_eq!(view.node_count(), 1);
        assert_eq!(view.string(0), Some(expected));
        let node = view.node(0).unwrap();
        assert_eq!(node.widget_type(), WIDGET_TEXT);
        assert_eq!(node.widget_schema(), Version::new(1, 0));
        assert_eq!(node.children().count(), 0);
        assert_eq!(
            node.properties().collect::<Vec<_>>(),
            vec![WidgetProperty::new(
                PROPERTY_TEXT_CONTENT,
                PropertyValue::StringRef(0),
            )]
        );
    }

    #[cfg(feature = "portable-guest")]
    fn assert_limit(error: PortableBuildError, resource: PortableWidgetResource) {
        match error {
            PortableBuildError::LimitExceeded {
                resource: actual, ..
            } => {
                assert_eq!(actual, resource)
            }
            PortableBuildError::PropertyEncoding { cause, .. } => assert_limit(*cause, resource),
            other => panic!("expected {resource:?} limit, got {other:?}"),
        }
    }

    #[cfg(feature = "portable-guest")]
    #[test]
    fn static_and_owned_formatted_text_lower_to_exact_text_ir() {
        assert_exact_portable_text(Text::new("static"), "static");
        assert_exact_portable_text(Text::new(format!("formatted {}", 7)), "formatted 7");
    }

    #[cfg(feature = "portable-guest")]
    #[test]
    fn text_lowering_enforces_string_property_and_key_limits() {
        let mut ctx = portable_context(portable_limits().with_max_string_bytes(4));
        assert_limit(
            Text::new("12345")
                .to_portable_node(&mut ctx, portable_source(1))
                .unwrap_err(),
            PortableWidgetResource::StringBytes,
        );

        let mut ctx = portable_context(portable_limits().with_max_properties(0));
        assert_limit(
            Text::new("text")
                .to_portable_node(&mut ctx, portable_source(2))
                .unwrap_err(),
            PortableWidgetResource::Properties,
        );

        let mut ctx = portable_context(portable_limits().with_max_keys(0));
        assert_limit(
            Text::new("text")
                .to_portable_node(&mut ctx, portable_source(3))
                .unwrap_err(),
            PortableWidgetResource::Keys,
        );
    }

    #[cfg(feature = "portable-guest")]
    #[test]
    fn text_alignment_lowers_as_an_optional_property() {
        let mut context = portable_context(portable_limits());
        let root = Text::new("text")
            .text_align(TextAlign::MidCenter)
            .to_portable_node(&mut context, portable_source(8))
            .unwrap();
        let document = context.finish_document(root).unwrap();
        let bytes = document.encode().unwrap();
        let view = WidgetDocumentView::decode(&bytes, document.model_limits()).unwrap();
        let node = view.node(root.index()).unwrap();
        assert_eq!(
            node.properties().collect::<Vec<_>>(),
            vec![
                WidgetProperty::new(PROPERTY_TEXT_CONTENT, PropertyValue::StringRef(0)),
                WidgetProperty::new(PROPERTY_TEXT_ALIGN, PropertyValue::I64(3)).optional(),
            ]
        );
        super::materialize_portable_text(&view, node, Vec::new()).unwrap();
    }

    #[cfg(feature = "portable-guest")]
    #[test]
    fn text_paragraph_properties_lower_as_optional_values() {
        let mut context = portable_context(portable_limits());
        let root = Text::new("text")
            .line_height(LineHeight::Px(24.0))
            .text_indent(-8.0)
            .to_portable_node(&mut context, portable_source(9))
            .unwrap();
        let document = context.finish_document(root).unwrap();
        let bytes = document.encode().unwrap();
        let view = WidgetDocumentView::decode(&bytes, document.model_limits()).unwrap();
        let node = view.node(root.index()).unwrap();
        let properties = node.properties().collect::<Vec<_>>();

        assert!(properties.iter().any(|property| {
            property.property_id() == PROPERTY_TEXT_LINE_HEIGHT
                && matches!(property.value(), PropertyValue::BlobRef(_))
        }));
        assert!(properties.iter().any(|property| {
            property.property_id() == PROPERTY_TEXT_INDENT
                && property.value() == PropertyValue::F64(-8.0)
        }));
        assert!(
            !properties
                .iter()
                .any(|property| property.property_id() == PROPERTY_TEXT_STYLE)
        );
        assert!(
            !properties
                .iter()
                .any(|property| property.property_id() == PROPERTY_TEXT_ALIGN)
        );
        super::materialize_portable_text(&view, node, Vec::new()).unwrap();
    }

    #[cfg(feature = "portable-guest")]
    #[test]
    fn every_text_style_field_lowers_and_materializes() {
        let style = TextStyle::new()
            .font_size(24)
            .font_family(FontFamily::MONOSPACE)
            .font_style(FontStyle::ObliqueDeg(-7))
            .font_weight(FontWeight::Value(650))
            .color(Colors::White)
            .background_color(Colors::Black.into())
            .text_overflow(TextOverflow::Ellipsis)
            .text_decoration(
                TextDecoration::new()
                    .line(TextDecorationLine::UNDERLINE | TextDecorationLine::ITALIC)
                    .style(TextDecorationStyle::Wavy)
                    .color(Colors::White)
                    .thickness(1.25)
                    .offset(-0.5),
            );
        let mut context = portable_context(portable_limits());
        let root = Text::new("styled")
            .text_style(style)
            .to_portable_node(&mut context, portable_source(4))
            .unwrap();
        let document = context.finish_document(root).unwrap();
        let bytes = document.encode().unwrap();
        let view = WidgetDocumentView::decode(&bytes, document.model_limits()).unwrap();
        let node = view.node(0).unwrap();

        assert_eq!(
            node.properties().collect::<Vec<_>>(),
            vec![
                WidgetProperty::new(PROPERTY_TEXT_CONTENT, PropertyValue::StringRef(0)),
                WidgetProperty::new(PROPERTY_TEXT_STYLE, PropertyValue::BlobRef(0)).optional(),
            ]
        );
        super::materialize_portable_text(&view, node, Vec::new()).unwrap();
    }

    #[test]
    fn text_outside_a_region_stays_on_the_non_selectable_fast_path() {
        let inner = InnerCanvas::new();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let ctx = context(FrameCanvas::new(&inner), &runtime);

        let element = Text::new("plain").to_element(&ctx);

        assert_eq!(element.debug_name(), RAW_TEXT_NAME);
    }

    #[test]
    fn plain_text_records_a_local_v2_run_and_repaints_without_dirtifying_its_sibling() {
        let inner = InnerCanvas::new();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let ctx = context_with_scale(FrameCanvas::new(&inner), &runtime, 2.0);
        let scale = ctx.scale;
        let raw_text = |text| RawTextWidget {
            text: TextSource::Static(text),
            text_style: TextStyle::default(),
            text_align: TextAlign::TopLeft,
            line_height: LineHeight::Normal,
            text_indent: 0.0,
            cache: LayoutCache::new(),
            _typeface: Cell::new(None),
        };
        let original = raw_text("before");
        let sibling = raw_text("sibling");
        let original_size = original.computed_size(&ctx);
        let sibling_size = sibling.computed_size(&ctx);
        let original_bounds = Rect::new(
            0.0,
            0.0,
            original_size.width / scale,
            original_size.height / scale,
        );
        let tree = RenderTree::new();
        let text_node = tree.add_root(original_bounds).unwrap();
        let sibling_node = tree
            .add_root(Rect::new(
                original_bounds.width + 20.0,
                0.0,
                sibling_size.width / scale,
                sibling_size.height / scale,
            ))
            .unwrap();

        let local_context = tree.context(text_node).unwrap();
        assert!(original.can_paint_local_v2(&ctx));
        ctx.with_local_v2_paint_context(local_context, |ctx| original.paint_local_v2(ctx));
        tree.set_paint_source(text_node, RenderPaintSource::LocalV2)
            .unwrap();
        let sibling_context = tree.context(sibling_node).unwrap();
        assert!(sibling.can_paint_local_v2(&ctx));
        ctx.with_local_v2_paint_context(sibling_context, |ctx| sibling.paint_local_v2(ctx));
        tree.set_paint_source(sibling_node, RenderPaintSource::LocalV2)
            .unwrap();

        let first_recording = tree.draw_list_snapshot(text_node).unwrap();
        assert_eq!(first_recording.revision, 1);
        assert_eq!(
            tree.paint_source(text_node).unwrap(),
            RenderPaintSource::LocalV2
        );
        assert!(matches!(
            first_recording.commands.as_ref(),
            [DrawCommand::DrawText { text, font_size, .. }]
                if text.as_ref() == "before" && *font_size == 13.0
        ));

        tree.take_damage();
        tree.invalidate_paint(text_node).unwrap();
        let updated = raw_text("after");
        let updated_context = tree.context(text_node).unwrap();
        assert!(updated.can_paint_local_v2(&ctx));
        ctx.with_local_v2_paint_context(updated_context, |ctx| updated.paint_local_v2(ctx));

        let updated_recording = tree.draw_list_snapshot(text_node).unwrap();
        assert_eq!(updated_recording.revision, 2);
        assert!(matches!(
            updated_recording.commands.as_ref(),
            [DrawCommand::DrawText { text, .. }] if text.as_ref() == "after"
        ));
        assert_eq!(tree.draw_list_revision(sibling_node).unwrap(), 1);
        assert_eq!(tree.take_damage(), vec![original_bounds]);
    }

    #[test]
    fn text_inside_a_region_becomes_selectable() {
        let inner = InnerCanvas::new();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let ctx = context(FrameCanvas::new(&inner), &runtime);
        let session = SelectionSession::new(
            ctx.window.clone(),
            Rc::new(SelectionCoordinator::default()),
            super::DEFAULT_SELECTION_COLOR,
        );

        let element = ctx.with_state(SelectionScope(Rc::clone(&session)), |ctx| {
            Text::new("selectable").to_element(ctx)
        });

        assert_eq!(element.debug_name(), "SelectableText");
        session.select_all();
        assert_eq!(session.selected_text(), "selectable");
    }

    #[test]
    fn toggling_a_region_around_a_text_swaps_the_element_without_panicking() {
        let inner = InnerCanvas::new();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let ctx = context(FrameCanvas::new(&inner), &runtime);
        let session = SelectionSession::new(
            ctx.window.clone(),
            Rc::new(SelectionCoordinator::default()),
            super::DEFAULT_SELECTION_COLOR,
        );

        let inside = ctx.with_state(SelectionScope(Rc::clone(&session)), |ctx| {
            Text::new("toggled").to_element(ctx)
        });
        let outside = Text::new("toggled").to_element(&ctx);
        outside.adopt_runtime_state_from(inside.as_ref());

        assert_eq!(outside.debug_name(), RAW_TEXT_NAME);
    }
}
