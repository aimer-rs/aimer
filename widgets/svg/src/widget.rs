use std::cell::{Cell, RefCell, UnsafeCell};
use std::sync::Arc;

use aimer_attribute::{Bounds, Dimension, ResolvedSize};
use aimer_cupid::svg::{
    SvgAspectMode, SvgFillRule, SvgFitPolicy, SvgGeometry, SvgNode, SvgNodeId,
    SvgNodeStyleOverride, SvgPathCommand, SvgScene, SvgTransform, SvgViewport,
};
use aimer_events::element::ElementEvent;
use aimer_events::pointer::{PointerInfo, PointerSource};
use aimer_events::window::request_animation_frame;
use aimer_utils::callback::{Callback, CallbackExecutor};
use aimer_widget::InteractionBounds;
use aimer_widget::base::BuildContext;
use aimer_widget::portable::__anteros::{PropertyId, WidgetDocumentView, WidgetNodeView};
use aimer_widget::portable::{
    PortableMaterializeError, optional_materialized_property, required_materialized_property,
};
use aimer_widget::{
    AnyElement, AnyWidget, Drawable, Element, EventElement, EventResult, LayoutElement, PointerKey,
    Rebuildable, VisitorElement, Widget,
};

use crate::source::load_source;
#[cfg(not(target_arch = "wasm32"))]
use crate::source::report_load_result;
use crate::{SvgDocument, SvgError, SvgLoadState, SvgLoader, SvgSelector, SvgSource, SvgStyle};

pub type SvgCallback = Callback<SvgHit, ()>;

#[derive(Clone, Debug)]
pub struct SvgNodeMetadata {
    pub svg_id: Option<Arc<str>>,
    pub classes: Arc<[Arc<str>]>,
    pub element: aimer_cupid::svg::SvgElementKind,
}

#[derive(Clone, Debug)]
pub struct SvgHit {
    pub node_id: SvgNodeId,
    pub metadata: SvgNodeMetadata,
}

#[derive(Clone)]
struct StyleRule {
    selector: SvgSelector,
    style: SvgStyle,
}

#[derive(Clone)]
struct CallbackRule {
    selector: SvgSelector,
    callback: SvgCallback,
}

#[derive(Clone)]
/// Renders an already parsed [`SvgDocument`].
///
/// With no explicit dimensions, the widget uses the document's intrinsic SVG
/// viewport. Setting only [`Svg::width`] or [`Svg::height`] preserves the
/// viewport aspect ratio; setting both uses the requested dimensions.
/// Selector-based style overrides and pointer interaction are optional and
/// empty by default.
///
/// Styles target `#id`, `.class`, or element-name selectors. A [`SvgStyle`]
/// changes only the properties it contains: fill and stroke colors can be
/// replaced or removed, opacity is overridden, and a transform replaces the
/// selected node's rendered transform. The non-`try_` builders silently ignore
/// invalid selectors; their `try_` counterparts return
/// [`SvgError::InvalidSelector`].
///
/// # Example
///
/// ```
/// use aimer_svg::{Svg, SvgColor, SvgDocument, SvgError, SvgStyle};
///
/// # fn example() -> Result<(), SvgError> {
/// let document = SvgDocument::from_svg(
///     br#"<svg viewBox="0 0 24 24"><path id="mark" d="M2 2h20v20z"/></svg>"#,
/// )?;
/// let svg = Svg::new(document).width(48.0)
///                             .style("#mark",
///                                    SvgStyle::new().fill(SvgColor::rgba8(0, 128, 255, 255)));
/// # Ok(())
/// # }
/// ```
#[derive(aimer_widget::PortableWidget)]
#[portable_widget(id = "aimer_svg::Svg", materializer = materialize_portable_svg)]
pub struct Svg {
    #[portable_skip]
    document: SvgDocument,
    source: String,
    width: Option<Dimension>,
    height: Option<Dimension>,
    #[portable_skip]
    styles: Vec<StyleRule>,
    #[portable_skip]
    hover_styles: Vec<StyleRule>,
    #[portable_skip]
    pressed_styles: Vec<StyleRule>,
    #[portable_skip]
    callbacks: Vec<CallbackRule>,
    #[portable_skip]
    paint_bounded: bool,
}

impl Svg {
    /// Creates a widget for a parsed document or an embedded [`crate::StaticSvg`].
    ///
    /// The intrinsic viewport determines its size, and no style or callback
    /// rules are installed.
    pub fn new(document: impl Into<SvgDocument>) -> Self {
        let document = document.into();
        let source = document.source().to_owned();
        Self {
            document,
            source,
            width: None,
            height: None,
            styles: Vec::new(),
            hover_styles: Vec::new(),
            pressed_styles: Vec::new(),
            callbacks: Vec::new(),
            paint_bounded: false,
        }
    }

    /// Sets the rendered width.
    ///
    /// When height is not set, it is derived from the document viewport to
    /// preserve the intrinsic aspect ratio.
    pub fn width(mut self, width: impl Into<Dimension>) -> Self {
        self.width = Some(width.into());
        self
    }

    /// Sets the rendered height.
    ///
    /// When width is not set, it is derived from the document viewport to
    /// preserve the intrinsic aspect ratio.
    pub fn height(mut self, height: impl Into<Dimension>) -> Self {
        self.height = Some(height.into());
        self
    }

    /// Declares that this SVG stays inside its layout rectangle while painted.
    ///
    /// Without this, a document that cannot be proven to stay inside its
    /// viewport is clipped to its layout rectangle, as SVG does at the root.
    /// Declaring it skips that clip. Use it only for assets whose paths,
    /// strokes, filters, and style transforms do not overflow the widget
    /// bounds, since anything that does would otherwise be cut off.
    #[inline]
    pub fn bounded(mut self) -> Self {
        self.paint_bounded = true;
        self
    }

    /// Adds a persistent style override for nodes matching `selector`.
    ///
    /// Supported selectors are `#id`, `.class`, and element names. An invalid
    /// selector is ignored; use [`Svg::try_style`] to receive an error instead.
    /// Rules are retained in insertion order and only properties set in `style`
    /// override the SVG document.
    pub fn style(mut self, selector: impl AsRef<str>, style: impl Into<SvgStyle>) -> Self {
        if let Ok(selector) = selector.as_ref().parse() {
            self.styles.push(StyleRule {
                selector,
                style: style.into(),
            });
        }
        self
    }

    /// Adds a persistent style override, returning an error for an invalid
    /// selector.
    ///
    /// See [`Svg::style`] for matching and override semantics.
    pub fn try_style(
        mut self,
        selector: impl AsRef<str>,
        style: SvgStyle,
    ) -> Result<Self, SvgError> {
        let selector = selector.as_ref().parse()?;
        self.styles.push(StyleRule { selector, style });
        Ok(self)
    }

    /// Adds a style applied while a matching painted node is under the pointer.
    ///
    /// An invalid selector is ignored; use [`Svg::try_hover_style`] to receive
    /// an error. Only properties present in `style` are overridden.
    pub fn hover_style(mut self, selector: impl AsRef<str>, style: SvgStyle) -> Self {
        if let Ok(selector) = selector.as_ref().parse() {
            self.hover_styles.push(StyleRule { selector, style });
        }
        self
    }

    /// Adds a hover style, returning an error for an invalid selector.
    ///
    /// See [`Svg::hover_style`] for interaction and override semantics.
    pub fn try_hover_style(
        mut self,
        selector: impl AsRef<str>,
        style: SvgStyle,
    ) -> Result<Self, SvgError> {
        let selector = selector.as_ref().parse()?;
        self.hover_styles.push(StyleRule { selector, style });
        Ok(self)
    }

    /// Adds a style applied while a matching painted node is pressed.
    ///
    /// An invalid selector is ignored; use [`Svg::try_pressed_style`] to
    /// receive an error. The pressed state ends when the pointer is
    /// released or cancelled.
    pub fn pressed_style(mut self, selector: impl AsRef<str>, style: SvgStyle) -> Self {
        if let Ok(selector) = selector.as_ref().parse() {
            self.pressed_styles.push(StyleRule { selector, style });
        }
        self
    }

    /// Adds a pressed style, returning an error for an invalid selector.
    ///
    /// See [`Svg::pressed_style`] for interaction and override semantics.
    pub fn try_pressed_style(
        mut self,
        selector: impl AsRef<str>,
        style: SvgStyle,
    ) -> Result<Self, SvgError> {
        let selector = selector.as_ref().parse()?;
        self.pressed_styles.push(StyleRule { selector, style });
        Ok(self)
    }

    /// Registers a callback for a press completed on a matching painted node.
    ///
    /// The callback receives [`SvgHit`] metadata and runs only when pointer
    /// down and pointer up hit the same node. An invalid selector is
    /// ignored; use [`Svg::try_on_path_press`] to receive an error.
    pub fn on_path_press(
        mut self,
        selector: impl AsRef<str>,
        callback: impl Into<SvgCallback>,
    ) -> Self {
        if let Ok(selector) = selector.as_ref().parse() {
            self.callbacks.push(CallbackRule {
                selector,
                callback: callback.into(),
            });
        }
        self
    }

    /// Registers a press callback, returning an error for an invalid selector.
    ///
    /// See [`Svg::on_path_press`] for hit-testing and press lifecycle
    /// semantics.
    pub fn try_on_path_press(
        mut self,
        selector: impl AsRef<str>,
        callback: impl Into<SvgCallback>,
    ) -> Result<Self, SvgError> {
        let selector = selector.as_ref().parse()?;
        self.callbacks.push(CallbackRule {
            selector,
            callback: callback.into(),
        });
        Ok(self)
    }
}

const PROPERTY_SVG_SOURCE: PropertyId =
    PropertyId::from_canonical_name("aimer.property:aimer_svg::Svg:source");
const PROPERTY_SVG_WIDTH: PropertyId =
    PropertyId::from_canonical_name("aimer.property:aimer_svg::Svg:width");
const PROPERTY_SVG_HEIGHT: PropertyId =
    PropertyId::from_canonical_name("aimer.property:aimer_svg::Svg:height");

fn materialize_portable_svg(
    document: &WidgetDocumentView<'_>,
    node: WidgetNodeView<'_>,
    children: Vec<AnyWidget>,
) -> Result<AnyWidget, PortableMaterializeError> {
    if !children.is_empty() {
        return Err(PortableMaterializeError::InvalidChildCount {
            expected: 0,
            actual: children.len(),
        });
    }
    let source = required_materialized_property::<String>(document, &node, PROPERTY_SVG_SOURCE)?;
    let svg_document = SvgDocument::from_svg(source).map_err(|_| {
        PortableMaterializeError::InvalidPropertyValue {
            property: PROPERTY_SVG_SOURCE,
        }
    })?;
    let width = optional_materialized_property::<Dimension>(document, &node, PROPERTY_SVG_WIDTH)?;
    let height = optional_materialized_property::<Dimension>(document, &node, PROPERTY_SVG_HEIGHT)?;
    let mut widget = Svg::new(svg_document);
    if let Some(width) = width {
        widget = widget.width(width);
    }
    if let Some(height) = height {
        widget = widget.height(height);
    }
    Ok(widget.boxed())
}

impl Widget for Svg {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        let paint_bounded = self.paint_bounded
            || (document_paint_is_bounded(&self.document)
                && style_rules_are_bounded(&self.styles)
                && style_rules_are_bounded(&self.hover_styles)
                && style_rules_are_bounded(&self.pressed_styles));
        RawSvg {
            document: self.document,
            width: self.width,
            height: self.height,
            styles: self.styles,
            hover_styles: self.hover_styles,
            pressed_styles: self.pressed_styles,
            callbacks: self.callbacks,
            bounds: InteractionBounds::new(),
            hovered: Cell::new(None),
            interaction: RefCell::new(SvgInteraction::default()),
            paint_bounded,
        }
        .boxed()
    }
}

/// Displays an SVG bundled with the app and registered under `[assets]` in
/// `aimer.toml`.
///
/// The asset key follows the same platform lookup rules as `AssetImage`:
/// Android's asset manager, app-bundle resources on Apple platforms, the
/// project directory during desktop development, and a root-relative request on
/// web. Loading and parsing run asynchronously. While loading, the configured
/// [`SvgAsset::loading_widget`] is active; after an I/O or parse error, the
/// configured [`SvgAsset::error_widget`] is active. Without the corresponding
/// fallback, the widget has no active child for that phase.
///
/// Once loaded, sizing, selector styles, transforms, colors, and callbacks have
/// the same semantics as [`Svg`].
///
/// # Example
///
/// ```
/// use aimer_svg::{SvgAsset, SvgColor, SvgStyle};
///
/// let icon =
///     SvgAsset::new("assets/icon.svg").width(32.0)
///                                     .height(32.0)
///                                     .style(".accent",
///                                            SvgStyle::new().fill(SvgColor::rgba8(0, 128, 255,
///                                                                                 255)));
/// ```
#[derive(aimer_widget::PortableWidget)]
#[portable_widget(id = "aimer_svg::SvgAsset", schema_only)]
pub struct SvgAsset {
    #[portable_skip]
    key: Arc<str>,
    width: Option<Dimension>,
    height: Option<Dimension>,
    #[portable_skip]
    styles: Vec<StyleRule>,
    #[portable_skip]
    hover_styles: Vec<StyleRule>,
    #[portable_skip]
    pressed_styles: Vec<StyleRule>,
    #[portable_skip]
    callbacks: Vec<CallbackRule>,
    #[portable_skip]
    loading_widget: Option<AnyWidget>,
    #[portable_skip]
    error_widget: Option<AnyWidget>,
}

impl SvgAsset {
    /// Creates a widget for the registered SVG asset `key`.
    ///
    /// The intrinsic SVG viewport determines the loaded widget's size. No
    /// style, callback, loading, or error widgets are set by default.
    pub fn new(key: impl Into<Arc<str>>) -> Self {
        Self {
            key: key.into(),
            width: None,
            height: None,
            styles: Vec::new(),
            hover_styles: Vec::new(),
            pressed_styles: Vec::new(),
            callbacks: Vec::new(),
            loading_widget: None,
            error_widget: None,
        }
    }

    /// Sets the rendered width after the asset loads.
    ///
    /// When height is not set, it is derived from the SVG viewport to preserve
    /// the intrinsic aspect ratio.
    pub fn width(mut self, width: impl Into<Dimension>) -> Self {
        self.width = Some(width.into());
        self
    }

    /// Sets the rendered height after the asset loads.
    ///
    /// When width is not set, it is derived from the SVG viewport to preserve
    /// the intrinsic aspect ratio.
    pub fn height(mut self, height: impl Into<Dimension>) -> Self {
        self.height = Some(height.into());
        self
    }

    /// Adds a persistent style override for nodes matching `selector`.
    ///
    /// Supported selectors are `#id`, `.class`, and element names. An invalid
    /// selector is ignored; use [`SvgAsset::try_style`] to receive an error
    /// instead. Only properties set in [`SvgStyle`] override the loaded
    /// document.
    pub fn style(mut self, selector: impl AsRef<str>, style: SvgStyle) -> Self {
        if let Ok(selector) = selector.as_ref().parse() {
            self.styles.push(StyleRule { selector, style });
        }
        self
    }

    /// Adds a persistent style override, returning an error for an invalid
    /// selector.
    ///
    /// See [`SvgAsset::style`] for matching and override semantics.
    pub fn try_style(
        mut self,
        selector: impl AsRef<str>,
        style: SvgStyle,
    ) -> Result<Self, SvgError> {
        let selector = selector.as_ref().parse()?;
        self.styles.push(StyleRule { selector, style });
        Ok(self)
    }

    /// Adds a style applied while a matching painted node is under the pointer.
    ///
    /// An invalid selector is ignored; use [`SvgAsset::try_hover_style`] to
    /// receive an error.
    pub fn hover_style(mut self, selector: impl AsRef<str>, style: SvgStyle) -> Self {
        if let Ok(selector) = selector.as_ref().parse() {
            self.hover_styles.push(StyleRule { selector, style });
        }
        self
    }

    /// Adds a hover style, returning an error for an invalid selector.
    ///
    /// See [`SvgAsset::hover_style`] for interaction semantics.
    pub fn try_hover_style(
        mut self,
        selector: impl AsRef<str>,
        style: SvgStyle,
    ) -> Result<Self, SvgError> {
        let selector = selector.as_ref().parse()?;
        self.hover_styles.push(StyleRule { selector, style });
        Ok(self)
    }

    /// Adds a style applied while a matching painted node is pressed.
    ///
    /// An invalid selector is ignored; use [`SvgAsset::try_pressed_style`] to
    /// receive an error. The pressed state ends on pointer release or
    /// cancellation.
    pub fn pressed_style(mut self, selector: impl AsRef<str>, style: SvgStyle) -> Self {
        if let Ok(selector) = selector.as_ref().parse() {
            self.pressed_styles.push(StyleRule { selector, style });
        }
        self
    }

    /// Adds a pressed style, returning an error for an invalid selector.
    ///
    /// See [`SvgAsset::pressed_style`] for interaction semantics.
    pub fn try_pressed_style(
        mut self,
        selector: impl AsRef<str>,
        style: SvgStyle,
    ) -> Result<Self, SvgError> {
        let selector = selector.as_ref().parse()?;
        self.pressed_styles.push(StyleRule { selector, style });
        Ok(self)
    }

    /// Registers a callback for a press completed on a matching painted node.
    ///
    /// The callback receives [`SvgHit`] metadata and runs only when pointer
    /// down and pointer up hit the same node. An invalid selector is
    /// ignored; use [`SvgAsset::try_on_path_press`] to receive an error.
    pub fn on_path_press(
        mut self,
        selector: impl AsRef<str>,
        callback: impl Into<SvgCallback>,
    ) -> Self {
        if let Ok(selector) = selector.as_ref().parse() {
            self.callbacks.push(CallbackRule {
                selector,
                callback: callback.into(),
            });
        }
        self
    }

    /// Registers a press callback, returning an error for an invalid selector.
    ///
    /// See [`SvgAsset::on_path_press`] for hit-testing and press lifecycle
    /// semantics.
    pub fn try_on_path_press(
        mut self,
        selector: impl AsRef<str>,
        callback: impl Into<SvgCallback>,
    ) -> Result<Self, SvgError> {
        let selector = selector.as_ref().parse()?;
        self.callbacks.push(CallbackRule {
            selector,
            callback: callback.into(),
        });
        Ok(self)
    }

    /// Sets the child active while the asset is being read and parsed.
    ///
    /// Without one, the asset widget has no active child during loading. The
    /// child is built with the same build context as the asset widget.
    pub fn loading_widget(mut self, loading_widget: impl Widget + 'static) -> Self {
        self.loading_widget = Some(loading_widget.boxed());
        self
    }

    /// Sets the child active after asset loading or SVG parsing fails.
    ///
    /// The underlying error text is not passed to the child. Without an error
    /// widget, the asset widget has no active child after failure.
    pub fn error_widget(mut self, error_widget: impl Widget + 'static) -> Self {
        self.error_widget = Some(error_widget.boxed());
        self
    }
}

impl Widget for SvgAsset {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let loader = SvgLoader::new(SvgSource::Asset(self.key.clone()));
        let (source, updates) = loader.background_parts();
        let window = ctx.window.clone();
        // An asset that has loaded before is ready already, and loading it again
        // would only read the file once more for every time the widget is made.
        let initial = loader.state();
        let needs_load = matches!(initial, SvgLoadState::Loading);

        #[cfg(not(target_arch = "wasm32"))]
        match aimer_venus::Venus::current() {
            _ if !needs_load => {}
            Some(venus) => {
                let source = source.clone();
                venus.spawn(async move {
                    let _ = updates.send(SvgLoadState::Loading);
                    let _ = updates.send(load_source(&source).await);
                    window.request_redraw();
                });
            }
            None => {
                let state = report_load_result(
                    &source,
                    SvgLoadState::Error(Arc::from("Venus runtime is unavailable")),
                );
                let _ = updates.send(state);
                window.request_redraw();
            }
        }

        #[cfg(target_arch = "wasm32")]
        if needs_load {
            wasm_bindgen_futures::spawn_local(async move {
                let _ = updates.send(SvgLoadState::Loading);
                let _ = updates.send(load_source(&source).await);
                window.request_redraw();
            });
        }

        let asset = RawSvgAsset {
            loader: RefCell::new(loader),
            phase: Cell::new(SvgAssetPhase::Loading),
            width: self.width,
            height: self.height,
            styles: self.styles.clone(),
            hover_styles: self.hover_styles.clone(),
            pressed_styles: self.pressed_styles.clone(),
            callbacks: self.callbacks.clone(),
            loading_element: self.loading_widget.map(|widget| widget.to_element(ctx)),
            error_element: self.error_widget.map(|widget| widget.to_element(ctx)),
            svg_element: UnsafeCell::new(None),
        };
        // An asset that loaded before is ready as it is made. Installing its icon
        // here, in the element that is still being built, is not a change of a
        // tree anyone has seen; waiting for the first layout to do it announced
        // one for every icon an animation made again, on every frame.
        if let SvgLoadState::Ready(document) = initial {
            asset.install(document, ctx);
        }
        asset.boxed()
    }

    fn debug_name(&self) -> &'static str {
        "SvgAsset"
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum SvgAssetPhase {
    Loading,
    Ready,
    Error,
}

struct RawSvgAsset {
    loader: RefCell<SvgLoader>,
    phase: Cell<SvgAssetPhase>,
    width: Option<Dimension>,
    height: Option<Dimension>,
    styles: Vec<StyleRule>,
    hover_styles: Vec<StyleRule>,
    pressed_styles: Vec<StyleRule>,
    callbacks: Vec<CallbackRule>,
    loading_element: Option<AnyElement>,
    error_element: Option<AnyElement>,
    svg_element: UnsafeCell<Option<AnyElement>>,
}

impl RawSvgAsset {
    /// Builds the icon from a loaded `document` and makes it the active child.
    ///
    /// The caller says whether the change is one the rest of the tree has to be
    /// told of.
    fn install(&self, document: SvgDocument, ctx: &BuildContext) {
        let source = document.source().to_owned();
        let svg = Svg {
            document,
            source,
            width: self.width,
            height: self.height,
            styles: self.styles.clone(),
            hover_styles: self.hover_styles.clone(),
            pressed_styles: self.pressed_styles.clone(),
            callbacks: self.callbacks.clone(),
            paint_bounded: false,
        };
        // Rendering and element-tree access are single-threaded. The
        // background task only publishes an owned immutable state through
        // the loader mailbox; this thread installs the resulting element.
        unsafe {
            *self.svg_element.get() = Some(svg.to_element(ctx));
        }
        self.phase.set(SvgAssetPhase::Ready);
    }

    fn refresh(&self, ctx: &BuildContext) {
        if self.phase.get() != SvgAssetPhase::Loading {
            return;
        }
        match self.loader.borrow().state() {
            SvgLoadState::Loading => {}
            SvgLoadState::Ready(document) => {
                self.install(document, ctx);
                aimer_widget::notify_element_tree_changed();
                request_animation_frame();
            }
            SvgLoadState::Error(_) => {
                self.phase.set(SvgAssetPhase::Error);
                aimer_widget::notify_element_tree_changed();
            }
        }
    }

    fn active_element(&self) -> Option<&dyn Element> {
        match self.phase.get() {
            SvgAssetPhase::Loading => self.loading_element.as_deref(),
            SvgAssetPhase::Ready => {
                // The element is initialized before the phase changes to Ready and
                // is thereafter only read on the render thread.
                unsafe { (&*self.svg_element.get()).as_deref() }
            }
            SvgAssetPhase::Error => self.error_element.as_deref(),
        }
    }
}

impl VisitorElement for RawSvgAsset {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        if let Some(element) = self.active_element() {
            visitor(element);
        }
    }

    fn debug_name(&self) -> &'static str {
        "SvgAssetElement"
    }
}

impl LayoutElement for RawSvgAsset {
    fn layout(&self, ctx: &BuildContext) -> ResolvedSize {
        self.refresh(ctx);
        self.active_element()
            .map(|element| element.layout(ctx))
            .unwrap_or_default()
    }

    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.refresh(ctx);
        self.active_element()
            .map(|element| element.computed_size(ctx))
            .unwrap_or_default()
    }

    fn content_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.refresh(ctx);
        self.active_element()
            .map(|element| element.content_size(ctx))
            .unwrap_or_default()
    }

    fn invalidate_layout(&self) {
        if let Some(element) = self.active_element() {
            element.invalidate_layout();
        }
    }
}

impl Drawable for RawSvgAsset {
    fn update(&self, ctx: &BuildContext) {
        self.refresh(ctx);
        if let Some(element) = self.active_element() {
            element.update(ctx);
        }
    }

    fn can_paint_local_v2(&self, ctx: &BuildContext) -> bool {
        self.phase.get() == SvgAssetPhase::Ready
            && self
                .active_element()
                .is_some_and(|element| element.can_paint_local_v2(ctx))
    }

    fn paint_local_v2(&self, ctx: &BuildContext) {
        let canvas = aimer_canvas::Canvas::of(ctx);
        canvas.finish();
    }
}

impl EventElement for RawSvgAsset {
    fn on_event(&self, event: &ElementEvent) -> EventResult {
        self.active_element()
            .map_or_else(EventResult::ignored, |element| element.on_event(event))
    }

    fn event_children<'a>(&'a self, _visitor: &mut dyn FnMut(&'a dyn Element)) {}
}

impl Rebuildable for RawSvgAsset {
    fn option_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    /// Carries the (possibly already loaded) loader across a rebuild so a frame
    /// of an ancestor's animation does not reset the SVG back to `Loading` and
    /// re-read the asset. `phase` is left fresh — `refresh` re-evaluates the
    /// carried loader on the next draw and builds the element from it.
    fn adopt_runtime_state_from(&self, old: &dyn Element) {
        let Some(old) = old
            .option_any()
            .and_then(|value| value.downcast_ref::<Self>())
        else {
            return;
        };
        *self.loader.borrow_mut() = old.loader.borrow().clone();
    }
}

pub struct RawSvg {
    document: SvgDocument,
    width: Option<Dimension>,
    height: Option<Dimension>,
    styles: Vec<StyleRule>,
    hover_styles: Vec<StyleRule>,
    pressed_styles: Vec<StyleRule>,
    callbacks: Vec<CallbackRule>,
    bounds: InteractionBounds,
    hovered: Cell<Option<SvgNodeId>>,
    interaction: RefCell<SvgInteraction>,
    paint_bounded: bool,
}

fn style_rules_are_bounded(rules: &[StyleRule]) -> bool {
    rules.iter().all(|rule| rule.style.transform.is_none())
}

/// Checks the static SVG scene once, at element construction, so a bounded
/// animation can invalidate only the transformed widget box. Bézier control
/// points conservatively bound their curves; stroke padding covers caps, joins,
/// and miter expansion. A document that cannot be proven inside its viewport
/// stays on the full-damage/live-paint path.
fn document_paint_is_bounded(document: &SvgDocument) -> bool {
    if matches!(
        document.fit_policy(),
        SvgFitPolicy::PreserveAspectRatio(preserve) if preserve.mode == SvgAspectMode::Slice
    ) {
        // A slice fit can map an in-viewport path outside the destination when
        // both widget dimensions are supplied. The widget currently captures
        // the ancestor clip, not an implicit SVG-destination clip, so keep the
        // conservative live path for this policy.
        return false;
    }

    let scene = document.scene();
    let Ok(compensation) = document.fit_compensation(scene.viewport.width, scene.viewport.height)
    else {
        return false;
    };
    let viewport = scene.viewport;
    let epsilon = f32::EPSILON * viewport.width.max(viewport.height).max(1.0) * 16.0;

    for node in scene.nodes.iter().filter(|node| {
        node.visible && !node.is_definition && node.opacity > 0.0 && node.geometry.is_some()
    }) {
        let Some(geometry) = scene.geometry(node) else {
            return false;
        };
        let transform = compensation.mul(node.transform);
        if !transform.is_finite() {
            return false;
        }
        let transform_scale = transform
            .sx
            .hypot(transform.ky)
            .hypot(transform.kx.hypot(transform.sy));
        if !transform_scale.is_finite() {
            return false;
        }
        let stroke_padding = node
            .stroke
            .as_ref()
            .map(|stroke| {
                stroke.width.abs() * stroke.miter_limit.abs().max(1.0) * 0.5 * transform_scale
            })
            .unwrap_or(0.0);
        if !stroke_padding.is_finite() {
            return false;
        }

        let mut min_x = f32::INFINITY;
        let mut min_y = f32::INFINITY;
        let mut max_x = f32::NEG_INFINITY;
        let mut max_y = f32::NEG_INFINITY;
        let mut has_point = false;
        let mut include = |x: f32, y: f32| {
            let (x, y) = transform.transform_point(x, y);
            if !x.is_finite() || !y.is_finite() {
                return false;
            }
            has_point = true;
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
            true
        };

        for command in geometry.commands.iter() {
            let valid = match *command {
                SvgPathCommand::MoveTo { x, y } | SvgPathCommand::LineTo { x, y } => include(x, y),
                SvgPathCommand::QuadraticTo {
                    control_x,
                    control_y,
                    x,
                    y,
                } => include(control_x, control_y) && include(x, y),
                SvgPathCommand::CubicTo {
                    control1_x,
                    control1_y,
                    control2_x,
                    control2_y,
                    x,
                    y,
                } => {
                    include(control1_x, control1_y)
                        && include(control2_x, control2_y)
                        && include(x, y)
                }
                SvgPathCommand::Close => true,
            };
            if !valid {
                return false;
            }
        }

        if has_point
            && (min_x - stroke_padding < -epsilon
                || min_y - stroke_padding < -epsilon
                || max_x + stroke_padding > viewport.width + epsilon
                || max_y + stroke_padding > viewport.height + epsilon)
        {
            return false;
        }
    }

    true
}

impl RawSvg {
    fn resolved_size(&self, ctx: &BuildContext) -> ResolvedSize {
        let viewport = self.document.scene().viewport;
        let width = self
            .width
            .map(|value| value.resolve(ctx.parent_size.width, ctx.scale));
        let height = self
            .height
            .map(|value| value.resolve(ctx.parent_size.height, ctx.scale));
        let (width, height) = resolved_svg_size(viewport, width, height);
        ResolvedSize {
            width: width.clamp(ctx.box_constraint.min_width, ctx.box_constraint.max_width),
            height: height.clamp(ctx.box_constraint.min_height, ctx.box_constraint.max_height),
        }
    }

    fn active_rules(&self) -> Vec<(SvgSelector, SvgStyle)> {
        let mut rules = self
            .styles
            .iter()
            .map(|rule| (rule.selector.clone(), rule.style))
            .collect::<Vec<_>>();
        if let Some(hovered) = self.hovered.get()
            && let Some(node) = self.document.scene().node(hovered)
        {
            rules.extend(
                self.hover_styles
                    .iter()
                    .filter(|rule| rule.selector.matches(node))
                    .map(|rule| (rule.selector.clone(), rule.style)),
            );
        }
        if let Some(pressed) = self.interaction.borrow().pressed
            && let Some(node) = self.document.scene().node(pressed)
        {
            rules.extend(
                self.pressed_styles
                    .iter()
                    .filter(|rule| rule.selector.matches(node))
                    .map(|rule| (rule.selector.clone(), rule.style)),
            );
        }
        rules
    }

    fn overrides_for_size(&self, width: f32, height: f32) -> Vec<SvgNodeStyleOverride> {
        let mut overrides = overrides_for_rules(self.document.scene(), &self.active_rules());
        let Ok(compensation) = self.document.fit_compensation(width, height) else {
            return overrides;
        };
        for node in self
            .document
            .scene()
            .nodes
            .iter()
            .filter(|node| !node.is_definition && node.geometry.is_some())
        {
            if let Some(override_) = overrides
                .iter_mut()
                .find(|override_| override_.node_id == node.node_id)
            {
                let transform = override_.transform.unwrap_or(node.transform);
                override_.transform = Some(compensation.mul(transform));
            } else {
                overrides.push(SvgNodeStyleOverride {
                    node_id: node.node_id,
                    fill: None,
                    stroke: None,
                    opacity: None,
                    transform: Some(compensation.mul(node.transform)),
                });
            }
        }
        overrides
    }

    fn hit_at(&self, x: f32, y: f32) -> Option<SvgHit> {
        let bounds = self.bounds.get_bounds()?;
        hit_test_scene(
            self.document.scene(),
            bounds,
            x,
            y,
            &self.overrides_for_size(bounds.width, bounds.height),
        )
    }

    /// Whether a press on `node_id` means anything to this SVG: a press callback
    /// or a pressed style matches it. Hover styles follow the pointer, not a press.
    fn listens_on(&self, node_id: SvgNodeId) -> bool {
        let Some(node) = self.document.scene().node(node_id) else {
            return false;
        };
        self.callbacks.iter().any(|rule| rule.selector.matches(node))
            || self.pressed_styles.iter().any(|rule| rule.selector.matches(node))
    }

    fn set_hovered(&self, hovered: Option<SvgNodeId>) {
        if self.hovered.replace(hovered) != hovered && !self.hover_styles.is_empty() {
            request_animation_frame();
        }
    }

    fn clear_pressed(&self) -> bool {
        self.interaction.borrow_mut().cancel()
    }

    fn execute_callbacks(&self, hit: SvgHit) {
        let Some(node) = self.document.scene().node(hit.node_id) else {
            return;
        };
        for rule in self
            .callbacks
            .iter()
            .filter(|rule| rule.selector.matches(node))
        {
            rule.callback.execute(hit.clone());
        }
    }

    #[inline]
    fn save_bounds(&self, ctx: &BuildContext, size: ResolvedSize) {
        let (x, y) = ctx.canvas.get_transform_translation();
        self.bounds.save(ctx.scale, x, y, size.width, size.height);
    }
}

impl VisitorElement for RawSvg {
    fn debug_name(&self) -> &'static str {
        "Svg"
    }
}

impl Rebuildable for RawSvg {}

impl LayoutElement for RawSvg {
    fn is_layout_stable(&self) -> bool {
        true
    }

    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.resolved_size(ctx)
    }

    fn layout(&self, ctx: &BuildContext) -> ResolvedSize {
        let size = self.resolved_size(ctx);
        self.save_bounds(ctx, size);
        size
    }

    fn pos_start_end(&self) -> Option<(aimer_attribute::Vec2d, aimer_attribute::Vec2d)> {
        self.bounds.pos_start_end()
    }
}

impl Drawable for RawSvg {
    /// Hit-tested against the rectangle this element reports as its own size,
    /// which can be larger than the content box its render node is laid out as.
    #[inline]
    fn retained_v2_interaction_size(&self, ctx: &BuildContext) -> Option<ResolvedSize> {
        Some(self.resolved_size(ctx))
    }

    #[inline]
    fn adopt_retained_v2_interaction_source(&self, source: aimer_widget::InteractionSource) {
        self.bounds.adopt(source);
    }

    #[inline]
    fn retained_v2_interaction_disagreement(
        &self,
    ) -> Option<aimer_widget::InteractionDisagreement> {
        self.bounds.disagreement("Svg")
    }

    fn update(&self, ctx: &BuildContext) {
        // The document is painted by `paint_local_v2`.
        let size = self.resolved_size(ctx);
        self.save_bounds(ctx, size);
    }

    fn sync_paint_geometry(&self, ctx: &BuildContext) {
        self.save_bounds(ctx, self.resolved_size(ctx));
    }

    fn can_paint_local_v2(&self, ctx: &BuildContext) -> bool {
        let size = self.resolved_size(ctx);
        ctx.scale.is_finite()
            && ctx.scale > 0.0
            && size.width.is_finite()
            && size.height.is_finite()
            && size.width >= 0.0
            && size.height >= 0.0
    }

    fn paint_local_v2(&self, ctx: &BuildContext) {
        let size = self.resolved_size(ctx);
        let scale = ctx.scale;
        let destination =
            aimer_cupid::draw_cmd_v2::Rect::new(0.0, 0.0, size.width / scale, size.height / scale);
        let canvas = aimer_canvas::Canvas::of(ctx);
        if !self.paint_bounded {
            canvas.push_clip(destination, [0.0; 4]);
        }
        canvas.draw_svg(
            self.document.scene().clone(),
            destination,
            self.overrides_for_size(size.width, size.height).into(),
        );
        if !self.paint_bounded {
            canvas.pop_clip();
        }
        canvas.finish();
    }
}

impl EventElement for RawSvg {
    fn on_event(&self, event: &ElementEvent) -> EventResult {
        match event {
            ElementEvent::PointerMove(PointerInfo {
                pos,
                source: PointerSource::Mouse,
                ..
            }) => {
                let previous = self.hovered.get();
                self.set_hovered(self.hit_at(pos.x, pos.y).map(|hit| hit.node_id));
                if previous != self.hovered.get() && !self.hover_styles.is_empty() {
                    EventResult::redraw()
                } else {
                    EventResult::ignored()
                }
            }
            ElementEvent::PointerExited(PointerSource::Mouse, _) => {
                let hovered = self
                    .hovered
                    .get()
                    .is_some_and(|_| !self.hover_styles.is_empty());
                self.set_hovered(None);
                let pressed = self.clear_pressed() && !self.pressed_styles.is_empty();
                let result = if hovered || pressed {
                    EventResult::redraw()
                } else {
                    EventResult::ignored()
                };
                svg_pointer_capture_effect(result, event, false)
            }
            ElementEvent::PointerExited(_, _) => {
                let pressed = self.clear_pressed() && !self.pressed_styles.is_empty();
                let result = if pressed {
                    EventResult::redraw()
                } else {
                    EventResult::ignored()
                };
                svg_pointer_capture_effect(result, event, false)
            }
            ElementEvent::PointerDown(pointer) => {
                // Children are offered an event before their parent, so taking a
                // press nothing here listens for would starve the control the
                // icon sits in — a button or a tab bar — wherever the icon is drawn.
                let hit = self
                    .hit_at(pointer.x(), pointer.y())
                    .filter(|hit| self.listens_on(hit.node_id));
                let changed = self
                    .interaction
                    .borrow_mut()
                    .pointer_down(hit.as_ref().map(|hit| hit.node_id));
                let captured = hit.is_some();
                let result = if captured {
                    EventResult::consumed()
                } else {
                    EventResult::ignored()
                };
                svg_pointer_capture_effect(
                    if changed && !self.pressed_styles.is_empty() {
                        result.with_redraw()
                    } else {
                        result
                    },
                    event,
                    captured,
                )
            }
            ElementEvent::PointerUp(pointer) => {
                let hit = self.hit_at(pointer.x(), pointer.y());
                let had_pressed = self.interaction.borrow().pressed.is_some();
                let pressed = self
                    .interaction
                    .borrow_mut()
                    .pointer_up(hit.as_ref().map(|hit| hit.node_id));
                let result = if pressed.is_some()
                    && let Some(hit) = hit
                {
                    self.execute_callbacks(hit);
                    request_animation_frame();
                    EventResult::consumed()
                } else {
                    EventResult::ignored()
                };
                let result = if had_pressed && !self.pressed_styles.is_empty() {
                    result.with_redraw()
                } else {
                    result
                };
                svg_pointer_capture_effect(result, event, false)
            }
            ElementEvent::Cancel => {
                if self.clear_pressed() && !self.pressed_styles.is_empty() {
                    EventResult::redraw()
                } else {
                    EventResult::ignored()
                }
            }
            _ => EventResult::ignored(),
        }
    }
}

pub(crate) fn svg_pointer_capture_effect(
    result: EventResult,
    event: &ElementEvent,
    capture_on_down: bool,
) -> EventResult {
    match event {
        ElementEvent::PointerDown(pointer) if capture_on_down => {
            result.with_pointer_capture(PointerKey::new(pointer.source, pointer.id))
        }
        ElementEvent::PointerUp(pointer) => {
            result.with_pointer_release(PointerKey::new(pointer.source, pointer.id))
        }
        ElementEvent::PointerExited(source, id) => {
            result.with_pointer_release(PointerKey::new(*source, *id))
        }
        _ => result,
    }
}

#[derive(Default)]
pub(crate) struct SvgInteraction {
    pressed: Option<SvgNodeId>,
}

impl SvgInteraction {
    pub(crate) fn pointer_down(&mut self, hit: Option<SvgNodeId>) -> bool {
        let changed = self.pressed != hit;
        self.pressed = hit;
        changed
    }

    pub(crate) fn pointer_up(&mut self, hit: Option<SvgNodeId>) -> Option<SvgNodeId> {
        let pressed = self.pressed.take();
        if pressed == hit { pressed } else { None }
    }

    fn cancel(&mut self) -> bool {
        self.pressed.take().is_some()
    }
}

pub(crate) fn resolved_svg_size(
    viewport: SvgViewport,
    width: Option<f32>,
    height: Option<f32>,
) -> (f32, f32) {
    let ratio = viewport.width / viewport.height.max(f32::EPSILON);
    match (width, height) {
        (Some(width), Some(height)) => (width, height),
        (Some(width), None) => (width, width / ratio.max(f32::EPSILON)),
        (None, Some(height)) => (height * ratio, height),
        (None, None) => (viewport.width, viewport.height),
    }
}

pub(crate) fn overrides_for_rules(
    scene: &SvgScene,
    rules: &[(SvgSelector, SvgStyle)],
) -> Vec<SvgNodeStyleOverride> {
    scene
        .nodes
        .iter()
        .filter_map(|node| {
            let mut result = SvgNodeStyleOverride {
                node_id: node.node_id,
                fill: None,
                stroke: None,
                opacity: None,
                transform: None,
            };
            let mut ancestors = Vec::new();
            let mut current = Some(node.node_id);
            while let Some(node_id) = current {
                ancestors.push(node_id);
                current = scene.node(node_id).and_then(|node| node.parent);
            }
            ancestors.reverse();
            let mut matched = false;
            let mut group_opacity = 1.0;
            let mut group_transform = SvgTransform::default();
            for ancestor_id in ancestors {
                let Some(ancestor) = scene.node(ancestor_id) else {
                    continue;
                };
                let is_group_ancestor = ancestor.node_id != node.node_id
                    && ancestor.element == aimer_cupid::svg::SvgElementKind::Group;
                for (_, style) in rules
                    .iter()
                    .filter(|(selector, _)| selector.matches(ancestor))
                {
                    matched = true;
                    if is_group_ancestor {
                        if style.fill.is_some() {
                            result.fill = style.fill;
                        }
                        if style.stroke.is_some() {
                            result.stroke = style.stroke;
                        }
                        if let Some(opacity) = style.opacity {
                            group_opacity *= opacity;
                        }
                        if let Some(transform) = style.transform {
                            group_transform = group_transform.mul(transform);
                        }
                    } else {
                        if style.fill.is_some() {
                            result.fill = style.fill;
                        }
                        if style.stroke.is_some() {
                            result.stroke = style.stroke;
                        }
                        if style.opacity.is_some() {
                            result.opacity = style.opacity;
                        }
                        if style.transform.is_some() {
                            result.transform = style.transform;
                        }
                    }
                }
            }
            if group_opacity != 1.0 {
                result.opacity = Some(result.opacity.unwrap_or(node.opacity) * group_opacity);
            }
            if group_transform != SvgTransform::default() {
                result.transform =
                    Some(group_transform.mul(result.transform.unwrap_or(node.transform)));
            }
            matched.then_some(result)
        })
        .collect()
}

pub(crate) fn hit_test_scene(
    scene: &SvgScene,
    bounds: Bounds,
    x: f32,
    y: f32,
    overrides: &[SvgNodeStyleOverride],
) -> Option<SvgHit> {
    if bounds.width <= 0.0
        || bounds.height <= 0.0
        || x < bounds.x
        || y < bounds.y
        || x > bounds.x + bounds.width
        || y > bounds.y + bounds.height
    {
        return None;
    }
    let scene_point = (
        (x - bounds.x) * scene.viewport.width / bounds.width,
        (y - bounds.y) * scene.viewport.height / bounds.height,
    );
    for node in scene
        .nodes
        .iter()
        .rev()
        .filter(|node| node.visible && !node.is_definition && node.geometry.is_some())
    {
        let node_override = overrides.iter().find(|value| value.node_id == node.node_id);
        if node_override
            .and_then(|value| value.opacity)
            .unwrap_or(node.opacity)
            <= 0.0
        {
            continue;
        }
        let transform = node_override
            .and_then(|value| value.transform)
            .unwrap_or(node.transform);
        let Some(inverse) = transform.inverse() else {
            continue;
        };
        let point = inverse.transform_point(scene_point.0, scene_point.1);
        let Some(geometry) = scene.geometry(node) else {
            continue;
        };
        if hits_geometry(node, node_override, geometry, point) {
            return Some(SvgHit {
                node_id: node.node_id,
                metadata: SvgNodeMetadata {
                    svg_id: node.svg_id.clone(),
                    classes: node.classes.clone(),
                    element: node.element,
                },
            });
        }
    }
    None
}

fn hits_geometry(
    node: &SvgNode,
    node_override: Option<&SvgNodeStyleOverride>,
    geometry: &SvgGeometry,
    point: (f32, f32),
) -> bool {
    let contours = flatten_geometry(geometry);
    let fill_visible = match node_override.map(|value| value.fill) {
        Some(Some(None)) => false,
        Some(Some(Some(_))) => true,
        Some(None) | None => node.fill.is_some(),
    };
    let fill_rule = node
        .fill
        .as_ref()
        .map(|fill| fill.rule)
        .unwrap_or(SvgFillRule::NonZero);
    if fill_visible && contains_point(&contours, point, fill_rule) {
        return true;
    }
    let stroke_visible = match node_override.map(|value| value.stroke) {
        Some(Some(None)) => false,
        Some(Some(Some(_))) => true,
        Some(None) | None => node.stroke.is_some(),
    };
    if stroke_visible && let Some(stroke) = &node.stroke {
        let threshold = stroke.width * 0.5;
        return contours.iter().any(|contour| {
            contour
                .windows(2)
                .any(|segment| point_segment_distance(point, segment[0], segment[1]) <= threshold)
        });
    }
    false
}

fn flatten_geometry(geometry: &SvgGeometry) -> Vec<Vec<(f32, f32)>> {
    let mut contours = Vec::new();
    let mut contour = Vec::new();
    let mut current = (0.0, 0.0);
    for command in geometry.commands.iter().copied() {
        match command {
            SvgPathCommand::MoveTo { x, y } => {
                if !contour.is_empty() {
                    contours.push(std::mem::take(&mut contour));
                }
                current = (x, y);
                contour.push(current);
            }
            SvgPathCommand::LineTo { x, y } => {
                current = (x, y);
                contour.push(current);
            }
            SvgPathCommand::QuadraticTo {
                control_x,
                control_y,
                x,
                y,
            } => {
                let start = current;
                for step in 1..=16 {
                    let t = step as f32 / 16.0;
                    let inverse = 1.0 - t;
                    contour.push((
                        inverse * inverse * start.0 + 2.0 * inverse * t * control_x + t * t * x,
                        inverse * inverse * start.1 + 2.0 * inverse * t * control_y + t * t * y,
                    ));
                }
                current = (x, y);
            }
            SvgPathCommand::CubicTo {
                control1_x,
                control1_y,
                control2_x,
                control2_y,
                x,
                y,
            } => {
                let start = current;
                for step in 1..=24 {
                    let t = step as f32 / 24.0;
                    let inverse = 1.0 - t;
                    contour.push((
                        inverse.powi(3) * start.0
                            + 3.0 * inverse * inverse * t * control1_x
                            + 3.0 * inverse * t * t * control2_x
                            + t.powi(3) * x,
                        inverse.powi(3) * start.1
                            + 3.0 * inverse * inverse * t * control1_y
                            + 3.0 * inverse * t * t * control2_y
                            + t.powi(3) * y,
                    ));
                }
                current = (x, y);
            }
            SvgPathCommand::Close => {
                if contour.first() != contour.last()
                    && let Some(first) = contour.first().copied()
                {
                    contour.push(first);
                }
            }
        }
    }
    if !contour.is_empty() {
        contours.push(contour);
    }
    contours
}

fn contains_point(contours: &[Vec<(f32, f32)>], point: (f32, f32), rule: SvgFillRule) -> bool {
    let mut winding = 0_i32;
    let mut crossings = 0_u32;
    for contour in contours {
        for segment in contour.windows(2) {
            let (a, b) = (segment[0], segment[1]);
            if (a.1 > point.1) != (b.1 > point.1) {
                let x = a.0 + (point.1 - a.1) * (b.0 - a.0) / (b.1 - a.1);
                if x > point.0 {
                    crossings += 1;
                }
            }
            if a.1 <= point.1 {
                if b.1 > point.1 && cross(a, b, point) > 0.0 {
                    winding += 1;
                }
            } else if b.1 <= point.1 && cross(a, b, point) < 0.0 {
                winding -= 1;
            }
        }
    }
    match rule {
        SvgFillRule::EvenOdd => crossings % 2 == 1,
        SvgFillRule::NonZero => winding != 0,
    }
}

fn cross(a: (f32, f32), b: (f32, f32), point: (f32, f32)) -> f32 {
    (b.0 - a.0) * (point.1 - a.1) - (point.0 - a.0) * (b.1 - a.1)
}

fn point_segment_distance(point: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let segment = (b.0 - a.0, b.1 - a.1);
    let length_squared = segment.0 * segment.0 + segment.1 * segment.1;
    if length_squared <= f32::EPSILON {
        return (point.0 - a.0).hypot(point.1 - a.1);
    }
    let t = (((point.0 - a.0) * segment.0 + (point.1 - a.1) * segment.1) / length_squared)
        .clamp(0.0, 1.0);
    (point.0 - (a.0 + t * segment.0)).hypot(point.1 - (a.1 + t * segment.1))
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell, UnsafeCell};
    use std::sync::Arc;
    #[cfg(not(target_arch = "wasm32"))]
    use std::sync::OnceLock;

    #[cfg(not(target_arch = "wasm32"))]
    use aimer_attribute::{BoxConstraint, Vec2d};
    #[cfg(not(target_arch = "wasm32"))]
    use aimer_widget::base::{BuildContext, ResolvedSize, WindowHandle};
    use aimer_widget::{Element, Rebuildable, Widget};

    use super::{RawSvgAsset, Svg, SvgAssetPhase};
    use crate::{SvgDocument, SvgLoadState, SvgLoader, SvgSource};

    #[cfg(not(target_arch = "wasm32"))]
    fn context() -> BuildContext<'static> {
        let inner = Box::leak(Box::new(aimer_canvas::InnerCanvas::new()));
        static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
        let runtime = RUNTIME.get_or_init(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("SVG widget test runtime should build")
        });
        let _guard = runtime.enter();
        let mut context = BuildContext::new(
            aimer_canvas::FrameCanvas::new(inner),
            ResolvedSize {
                width: 32.0,
                height: 32.0,
            },
            1.0,
            Vec2d::default(),
            Vec2d::default(),
            WindowHandle::headless(Default::default(), 1.0),
            tokio::runtime::Handle::current(),
        );
        context.box_constraint = BoxConstraint {
            min_width: 0.0,
            min_height: 0.0,
            max_width: 32.0,
            max_height: 32.0,
        };
        context
    }

    fn asset(loader: SvgLoader) -> RawSvgAsset {
        RawSvgAsset {
            loader: RefCell::new(loader),
            phase: Cell::new(SvgAssetPhase::Loading),
            width: None,
            height: None,
            styles: Vec::new(),
            hover_styles: Vec::new(),
            pressed_styles: Vec::new(),
            callbacks: Vec::new(),
            loading_element: None,
            error_element: None,
            svg_element: UnsafeCell::new(None),
        }
    }

    // A tab bar that reveals itself makes its icons again on every frame. An asset
    // that has loaded before is ready the moment it is made, so there is nothing
    // for its first layout to announce: telling the framework the tree changed
    // made every pass after it walk the whole page.
    #[tokio::test]
    async fn an_asset_that_loaded_before_is_ready_when_made_and_announces_no_change() {
        use crate::source::finish_load;
        use crate::SvgAsset;

        let venus = aimer_venus::Venus::new();
        venus.install();
        let key = "assets/tests/ready-when-made.svg";
        let document = SvgDocument::from_svg(
            br#"<svg width="2" height="2" xmlns="http://www.w3.org/2000/svg"><path d="M0 0h1v1z"/></svg>"#,
        )
        .unwrap();
        finish_load(&SvgSource::Asset(Arc::from(key)), SvgLoadState::Ready(document));
        let ctx = context();

        let element = SvgAsset::new(key).to_element(&ctx);

        let mut children = 0;
        element.visit_children(&mut |_| children += 1);
        assert_eq!(children, 1, "the loaded icon is the element's child as soon as it is made");
        let before = aimer_widget::element_tree_generation();
        element.layout(&ctx);
        assert_eq!(
            aimer_widget::element_tree_generation(),
            before,
            "the first layout of a ready asset announced a change of the tree"
        );
        aimer_venus::Venus::uninstall();
    }

    // An ancestor animation rebuilds its child every frame and hands runtime
    // state over via `carry_element_state`. The rebuilt `SvgAsset` element must
    // adopt the old one's loader so it renders immediately instead of restarting
    // the async load every frame.
    #[tokio::test]
    async fn rebuild_carries_the_loaded_loader() {
        let venus = aimer_venus::Venus::new();
        venus.install();
        let source = br#"<svg width="2" height="2" xmlns="http://www.w3.org/2000/svg"><path d="M0 0h1v1z"/></svg>"#;

        let loaded = SvgLoader::new(SvgSource::Memory(Arc::from(source.as_slice())));
        loaded.load().await;
        let old = asset(loaded);

        let fresh = SvgLoader::new(SvgSource::Memory(Arc::from(source.as_slice())));
        let new = asset(fresh);
        assert!(matches!(new.loader.borrow().state(), SvgLoadState::Loading));

        new.adopt_runtime_state_from(&old as &dyn Element);

        assert!(matches!(
            new.loader.borrow().state(),
            SvgLoadState::Ready(_)
        ));
        aimer_venus::Venus::uninstall();
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn recorded_commands(
        element: &dyn Element,
        context: &BuildContext,
    ) -> aimer_cupid::draw_cmd_v2::DrawListSnapshot {
        use aimer_cupid::draw_cmd_v2::{Rect, RenderTree};

        let tree = RenderTree::new();
        let node = tree.add_root(Rect::new(0.0, 0.0, 32.0, 32.0)).unwrap();
        let node_context = tree.context(node).unwrap();
        context.with_local_v2_paint_context(node_context, |ctx| element.paint_local_v2(ctx));
        tree.draw_list_snapshot(node).unwrap()
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn svg_with_geometry_outside_the_viewport_is_clipped_to_its_box() {
        use aimer_cupid::draw_cmd_v2::{DrawCommand, Rect};

        let document = SvgDocument::from_svg(
            br#"<svg width="32" height="32" xmlns="http://www.w3.org/2000/svg"><path d="M-1 0h2v2H-1z"/></svg>"#,
        )
        .unwrap();
        let context = context();
        let element = Svg::new(document).to_element(&context);

        // SVG overflow is hidden at the root, so the widget stays inside its
        // box and never needs the legacy live path.
        assert!(element.can_paint_local_v2(&context));
        let snapshot = recorded_commands(element.as_ref(), &context);
        assert!(matches!(
            snapshot.commands.as_ref(),
            [
                DrawCommand::PushClip { rect, .. },
                DrawCommand::Svg { .. },
                DrawCommand::PopClip,
            ] if *rect == Rect::new(0.0, 0.0, 32.0, 32.0)
        ));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_provably_bounded_svg_is_recorded_without_a_clip() {
        use aimer_cupid::draw_cmd_v2::DrawCommand;

        let document = SvgDocument::from_svg(
            br#"<svg width="32" height="32" xmlns="http://www.w3.org/2000/svg"><path d="M0 0h32v32H0z"/></svg>"#,
        )
        .unwrap();
        let context = context();
        let element = Svg::new(document).to_element(&context);

        let snapshot = recorded_commands(element.as_ref(), &context);
        assert!(matches!(
            snapshot.commands.as_ref(),
            [DrawCommand::Svg { .. }]
        ));
    }
}
