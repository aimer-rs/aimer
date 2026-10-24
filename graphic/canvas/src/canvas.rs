use std::cell::{Cell, RefCell};
use std::rc::Rc;

use aimer_attribute::position::Vec2d;
use aimer_attribute::size::ResolvedSize;
use aimer_color::prelude::Color;
pub use aimer_cupid::canvas::TextMetrics;
pub use aimer_cupid::draw_cmd::{
    RETAINED_LAYER_MAX_BYTES, RETAINED_LAYER_MAX_DIMENSION, RETAINED_LAYER_MAX_TILES_PER_FRAME,
    RETAINED_LAYER_TILE_SIZE, RetainedDrawList, RetainedLayerContent, RetainedLayerPadding,
    next_retained_layer_id,
};
pub use aimer_cupid::font::TextLanguage;
pub use aimer_cupid::font::{FontFamily, FontStyle};
pub use aimer_cupid::text_pipeline::TextOverflowMode;
pub use aimer_cupid::text_pipeline::text_layout::TextHorizontalAlign;
pub use aimer_cupid::text_pipeline::text_layout::TextInteractionLayout;
pub use aimer_cupid::utilities::Mat3;
/// The look of a shader transition between an element's two children; see
/// [`Canvas::morph_children`].
pub use aimer_cupid::draw_cmd_v2::{MorphParams, MorphStyle};
/// A rectangle in the canvas's own coordinates, as [`Canvas`] draws and clips with.
pub use aimer_cupid::utilities::Rect;
/// The renderer's colour, which [`Canvas`] paints with.
///
/// It is not [`aimer_color::Color`], the colour widgets are styled with: that one
/// carries theme and alpha semantics, this one is the eight-bit-per-channel value
/// the draw commands hold. Convert at the point of drawing, for example from a
/// widget colour's packed `0xAARRGGBB` form with
/// `RenderColor::rgba8(r, g, b, a)`.
pub use aimer_cupid::utilities::Color as RenderColor;

use crate::material::MaterialDrawRequest;
mod native_impl;

pub trait CanvasRendering: Clone {
    fn begin_frame(&self);
    fn translate(&self, pos: Vec2d);
    fn scale(&self, sx: f32, sy: f32);
    fn rotate(&self, radians: f32);
    fn save(&self);
    fn restore(&self);

    fn get_image_size(&self, image_id: u32) -> Option<(u32, u32)>;
    /// Returns the generation of renderer-side image-cache changes.
    ///
    /// Image widgets use this cheap generation check to avoid probing the
    /// cache on every draw. Backends without automatic image eviction may keep
    /// the default value.
    fn texture_cache_epoch(&self) -> u64 {
        0
    }
    /// Returns whether an image ID can still be used by the canvas.
    ///
    /// The default derives availability from the retained intrinsic-size
    /// metadata. Native backends override it when their renderer can evict
    /// textures independently of the draw list.
    fn is_texture_available(&self, image_id: u32) -> bool {
        self.get_image_size(image_id).is_some()
    }
    fn measure_text(&self, text: &str, font_size: f32) -> f32;
    fn measure_text_styled(
        &self,
        text: &str,
        font_size: f32,
        font_family: FontFamily,
        font_style: FontStyle,
        font_weight: u16,
    ) -> f32 {
        let _ = (font_family, font_style, font_weight);
        self.measure_text(text, font_size)
    }
    /// Builds the source-aware Aimer layout used by caret and selection
    /// consumers.
    #[allow(clippy::too_many_arguments)]
    fn layout_text_styled(
        &self,
        text: &str,
        font_size: f32,
        max_width: f32,
        font_family: FontFamily,
        font_style: FontStyle,
        font_weight: u16,
    ) -> TextInteractionLayout;
    fn measure_text_metrics(&self, text: &str, font_size: f32, max_width: f32) -> TextMetrics;
    fn measure_text_metrics_styled(
        &self,
        text: &str,
        font_size: f32,
        max_width: f32,
        font_family: FontFamily,
        font_style: FontStyle,
        font_weight: u16,
    ) -> TextMetrics {
        let _ = (font_family, font_style, font_weight);
        self.measure_text_metrics(text, font_size, max_width)
    }
    /// Measures the rendered width of each line after wrapping.
    #[allow(clippy::too_many_arguments)]
    fn measure_text_line_widths_styled(
        &self,
        text: &str,
        font_size: f32,
        max_width: f32,
        font_family: FontFamily,
        font_style: FontStyle,
        font_weight: u16,
    ) -> Vec<f32> {
        vec![
            self.measure_text_metrics_styled(
                text,
                font_size,
                max_width,
                font_family,
                font_style,
                font_weight,
            )
            .width,
        ]
    }

    /// Declares the written language of subsequent text draws and
    /// measurements.
    ///
    /// Han is unified: `你好` is drawn by a Japanese face as readily as by a
    /// Chinese one, so a run of ideographs alone cannot say which it wants and
    /// keeps whichever the platform prefers — until a character only one
    /// language writes joins it and the whole word changes typeface. A widget
    /// that knows the language, such as a text field bound to a keyboard,
    /// declares it here. Default is a no-op for backends that don't support
    /// it.
    fn set_text_language(&self, _language: Option<TextLanguage>) {}
    /// The language declared by
    /// [`set_text_language`](CanvasRendering::set_text_language), so a caller
    /// that declares one for a measuring pass can put back the one it found.
    fn text_language(&self) -> Option<TextLanguage> {
        None
    }
    fn load_image(&self, bytes: &[u8], width: u32, height: u32) -> u32;
    fn load_image_with_id(&self, image_id: u32, bytes: &[u8], width: u32, height: u32);
    fn remove_texture(&self, image_id: u32);
    fn set_texture_size(&self, image_id: u32, width: u32, height: u32);
    fn get_transform_translation(&self) -> (f32, f32) {
        (0.0, 0.0)
    }
    fn get_transform(&self) -> Mat3 {
        Mat3::identity()
    }
}

use aimer_cupid::canvas::CupidCanvas;

pub type InnerCanvas = CupidCanvas;

#[allow(dead_code)]
// #[derive(Clone)]
#[derive(Clone)]
pub struct FrameCanvas<'a> {
    inner: &'a CupidCanvas,
}

impl<'a> FrameCanvas<'a> {
    #[allow(dead_code)]
    #[inline]
    /// Provides low level control to FrameCanvas
    ///
    /// # Safety
    /// This function is marked as `unsafe` because it directly returns a
    /// reference to an internal `FrameCanvas` object. The caller need to write
    /// platform-specific code for making platform-specific operations.
    ///
    /// # Returns
    /// * `&'a FrameCanvas` - A reference to the internal `FrameCanvas` object.
    ///
    /// # Example
    /// ```rust ignore
    /// let canvas = my_object.get_canvas();
    /// // Ensure no mutable operations on `my_object` while using `canvas`.
    /// ```
    unsafe fn get_canvas(&'a self) -> &'a CupidCanvas {
        self.inner
    }

    #[allow(dead_code)]
    #[inline]
    pub fn new(canvas: &'a CupidCanvas) -> Self {
        Self { inner: canvas }
    }

    pub fn get_inner_canvas(&self) -> &CupidCanvas {
        self.inner
    }

    /// Creates a short-lived canvas for recording a static subtree in local
    /// coordinates while sharing the parent's text, font, and texture state.
    #[inline]
    pub fn fork_for_recording(&self) -> InnerCanvas {
        self.inner.fork_for_recording()
    }
}

impl<'a> FrameCanvas<'a> {
    /// Prepares the canvas for a new frame, clearing any previous draw
    /// commands.
    #[allow(dead_code)]
    #[inline]
    pub fn begin_frame(&self) {
        CanvasRendering::begin_frame(self.inner);
    }

    /// Translates the canvas origin by the given vector.
    #[allow(dead_code)]
    #[inline]
    pub fn translate(&self, pos: Vec2d) {
        CanvasRendering::translate(self.inner, pos);
    }

    /// Scales the canvas by the given factors.
    #[allow(dead_code)]
    #[inline]
    pub fn scale(&self, sx: f32, sy: f32) {
        CanvasRendering::scale(self.inner, sx, sy);
    }

    /// Rotates the canvas by the given angle in radians.
    #[allow(dead_code)]
    #[inline]
    pub fn rotate(&self, radians: f32) {
        CanvasRendering::rotate(self.inner, radians);
    }

    /// Saves the entire state of the canvas by pushing the current state onto a
    /// stack.
    #[allow(dead_code)]
    #[inline]
    pub fn save(&self) {
        CanvasRendering::save(self.inner);
    }

    /// Restores the most recently saved canvas state from the stack.
    #[allow(dead_code)]
    #[inline]
    pub fn restore(&self) {
        CanvasRendering::restore(self.inner);
    }

    #[allow(dead_code)]
    #[inline]
    pub fn get_image_size(&self, image_id: u32) -> Option<(u32, u32)> {
        CanvasRendering::get_image_size(self.inner, image_id)
    }

    /// Returns the generation of renderer-side image-cache changes.
    #[inline]
    pub fn texture_cache_epoch(&self) -> u64 {
        CanvasRendering::texture_cache_epoch(self.inner)
    }

    /// Returns whether an image ID can still be used by the canvas.
    #[inline]
    pub fn is_texture_available(&self, image_id: u32) -> bool {
        CanvasRendering::is_texture_available(self.inner, image_id)
    }

    #[allow(dead_code)]
    #[inline]
    pub fn load_image(&self, bytes: &[u8], width: u32, height: u32) -> u32 {
        CanvasRendering::load_image(self.inner, bytes, width, height)
    }

    /// Loads an image from the specified path with a predefined image ID.
    #[allow(dead_code)]
    #[inline]
    pub fn load_image_with_id(&self, image_id: u32, bytes: &[u8], width: u32, height: u32) {
        CanvasRendering::load_image_with_id(self.inner, image_id, bytes, width, height)
    }

    /// Releases the GPU texture and its cached size metadata.
    pub fn remove_texture(&self, image_id: u32) {
        CanvasRendering::remove_texture(self.inner, image_id);
    }

    /// Sets the intrinsic size of a texture. This is useful for preserving
    /// metadata across frames.
    pub fn set_texture_size(&self, image_id: u32, width: u32, height: u32) {
        CanvasRendering::set_texture_size(self.inner, image_id, width, height);
    }

    /// Measures the approximate width of text at the given font size.
    #[allow(dead_code)]
    #[inline]
    pub fn measure_text(&self, text: &str, font_size: f32) -> f32 {
        CanvasRendering::measure_text(self.inner, text, font_size)
    }

    #[inline]
    pub fn measure_text_styled(
        &self,
        text: &str,
        font_size: f32,
        font_family: FontFamily,
        font_style: FontStyle,
        font_weight: u16,
    ) -> f32 {
        CanvasRendering::measure_text_styled(
            self.inner,
            text,
            font_size,
            font_family,
            font_style,
            font_weight,
        )
    }

    /// Builds source-aware Aimer geometry for a selectable text participant.
    #[allow(clippy::too_many_arguments)]
    pub fn layout_text_styled(
        &self,
        text: &str,
        font_size: f32,
        max_width: f32,
        font_family: FontFamily,
        font_style: FontStyle,
        font_weight: u16,
    ) -> TextInteractionLayout {
        CanvasRendering::layout_text_styled(
            self.inner,
            text,
            font_size,
            max_width,
            font_family,
            font_style,
            font_weight,
        )
    }

    /// Builds shared source-aware geometry for a styled text run.
    ///
    /// The native canvas retains width-independent interaction geometry so
    /// rich paragraphs can re-wrap without rebuilding every span's clusters.
    #[allow(clippy::too_many_arguments)]
    pub fn layout_text_styled_shared(
        &self,
        text: &str,
        font_size: f32,
        max_width: f32,
        font_family: FontFamily,
        font_style: FontStyle,
        font_weight: u16,
    ) -> Rc<TextInteractionLayout> {
        self.inner.layout_text_styled_shared(
            text,
            font_size,
            max_width,
            font_family,
            font_style,
            font_weight,
        )
    }

    /// Measures paragraph text metrics at the given font size and optional max
    /// width.
    #[allow(dead_code)]
    #[inline]
    pub fn measure_text_metrics(&self, text: &str, font_size: f32, max_width: f32) -> TextMetrics {
        CanvasRendering::measure_text_metrics(self.inner, text, font_size, max_width)
    }

    #[inline]
    pub fn measure_text_metrics_styled(
        &self,
        text: &str,
        font_size: f32,
        max_width: f32,
        font_family: FontFamily,
        font_style: FontStyle,
        font_weight: u16,
    ) -> TextMetrics {
        CanvasRendering::measure_text_metrics_styled(
            self.inner,
            text,
            font_size,
            max_width,
            font_family,
            font_style,
            font_weight,
        )
    }

    /// Measures the rendered width of each line after applying styled text
    /// wrapping.
    #[inline]
    #[allow(clippy::too_many_arguments)]
    pub fn measure_text_line_widths_styled(
        &self,
        text: &str,
        font_size: f32,
        max_width: f32,
        font_family: FontFamily,
        font_style: FontStyle,
        font_weight: u16,
    ) -> Vec<f32> {
        CanvasRendering::measure_text_line_widths_styled(
            self.inner,
            text,
            font_size,
            max_width,
            font_family,
            font_style,
            font_weight,
        )
    }

    /// Declares the written language of subsequent text draws and
    /// measurements — see
    /// [`CanvasRendering::set_text_language`](CanvasRendering::set_text_language).
    #[allow(dead_code)]
    #[inline]
    pub fn set_text_language(&self, language: Option<TextLanguage>) {
        CanvasRendering::set_text_language(self.inner, language);
    }

    /// The language declared by [`set_text_language`](Self::set_text_language).
    #[allow(dead_code)]
    #[inline]
    pub fn text_language(&self) -> Option<TextLanguage> {
        CanvasRendering::text_language(self.inner)
    }

    /// Returns the current transform's translation (tx, ty) in physical pixels.
    #[allow(dead_code)]
    #[inline]
    pub fn get_transform_translation(&self) -> (f32, f32) {
        CanvasRendering::get_transform_translation(self.inner)
    }

    /// Returns the current local-to-physical canvas transform.
    #[inline]
    pub fn get_transform(&self) -> Mat3 {
        CanvasRendering::get_transform(self.inner)
    }
}

/// A short-lived recorder for one element's v2 retained draw list.
///
/// Commands are staged locally and committed by [`Canvas::finish`],
/// [`Canvas::try_finish`], or automatically when the canvas is dropped.
/// Dropping during panic unwinding abandons the staged commands and leaves the
/// retained list dirty, avoiding a second panic from `Drop`. A commit error
/// during an ordinary drop panics.
pub struct Canvas {
    writer: Option<aimer_cupid::draw_cmd_v2::DrawListWriter>,
    commands: RefCell<Vec<aimer_cupid::draw_cmd_v2::DrawCommand>>,
    background: Cell<Color>,
    corner_radii: Cell<Option<[f32; 4]>>,
}

/// Resolves the active element-local command list used by [`Canvas::of`].
#[doc(hidden)]
pub trait CanvasContext {
    /// Returns the retained paint context active for this framework context.
    fn current_canvas_context(&self) -> aimer_cupid::draw_cmd_v2::CurrentBuildContext;

    /// Returns the declared solid surface behind this recorder, if known.
    #[doc(hidden)]
    #[inline]
    fn paint_background(&self) -> Color {
        Color::Transparent
    }
}

impl CanvasContext for aimer_cupid::draw_cmd_v2::CurrentBuildContext {
    #[inline]
    fn current_canvas_context(&self) -> aimer_cupid::draw_cmd_v2::CurrentBuildContext {
        self.clone()
    }
}

impl Canvas {
    /// Opens a recorder for the active element in `context`.
    ///
    /// # Panics
    ///
    /// Panics if the context has no active element-local recorder or if its
    /// retained list cannot be opened.
    pub fn of(context: &impl CanvasContext) -> Self {
        let background = context.paint_background();
        let context = context.current_canvas_context();
        let writer = context.begin_recording().unwrap_or_else(|error| {
            panic!("Canvas::of could not open the active element list: {error:?}")
        });
        Self {
            writer: Some(writer),
            commands: RefCell::new(Vec::new()),
            background: Cell::new(background),
            corner_radii: Cell::new(None),
        }
    }

    /// Returns the declared solid background underneath subsequent decoration layers.
    /// Transparent means that no solid background is known.
    #[inline]
    pub fn background_color(&self) -> Color {
        self.background.get()
    }

    /// Updates surface metadata after a decoration paints its background.
    /// Does not record any drawing commands or sample backdrop pixels.
    #[doc(hidden)]
    #[inline]
    pub fn add_background_color(&self, color: Option<Color>) {
        self.background
            .set(crate::composite_background(color, self.background.get()));
    }

    /// Returns the corner radii declared for the decorated box so far.
    ///
    /// Radii are logical pixels in top-left, top-right, bottom-right,
    /// bottom-left order. A decoration that follows a shape declaration, such
    /// as `BorderRadius`, adopts it; the default is square corners. Use
    /// [`Self::declared_corner_radii`] to tell a declared square from none.
    #[inline]
    pub fn corner_radii(&self) -> [f32; 4] {
        self.corner_radii.get().unwrap_or([0.0; 4])
    }

    /// Returns the declared corner radii, or `None` if no layer declared a shape.
    ///
    /// A declared `[0.0; 4]` is a deliberate square and differs from `None`,
    /// which lets a widget apply its own default shape.
    #[inline]
    pub fn declared_corner_radii(&self) -> Option<[f32; 4]> {
        self.corner_radii.get()
    }

    /// Declares the outer corner radii of the decorated box for later layers.
    ///
    /// The nearest declaration wins. Does not record any drawing commands.
    /// Values are stored as given; consumers sanitize them for their own use.
    #[inline]
    pub fn set_corner_radii(&self, radii: [f32; 4]) {
        self.corner_radii.set(Some(radii));
    }

    /// Adds a simple rectangle to this element's local paint list.
    #[inline]
    pub fn fill_rect(&self, rect: aimer_cupid::utilities::Rect, color: [u8; 4]) {
        self.fill_rect_styled(
            rect,
            aimer_cupid::utilities::Color::rgba8(color[0], color[1], color[2], color[3]),
            [0.0; 4],
            [0.0; 4],
            aimer_cupid::utilities::Color::transparent(),
            [0.0; 4],
            aimer_cupid::utilities::Color::transparent(),
        );
    }

    /// Adds a solid local rectangle from widget geometry and a framework color.
    #[inline]
    pub fn fill_color_rect(&self, pos: Vec2d, size: ResolvedSize, color: Color) {
        let rgba = color.as_u32();
        self.fill_rect(
            aimer_cupid::utilities::Rect::new(pos.x, pos.y, size.width, size.height),
            [
                ((rgba >> 16) & 0xff) as u8,
                ((rgba >> 8) & 0xff) as u8,
                (rgba & 0xff) as u8,
                ((rgba >> 24) & 0xff) as u8,
            ],
        );
    }

    /// Adds a rectangle with per-corner radii and per-side border and outline.
    #[allow(clippy::too_many_arguments)]
    pub fn fill_rect_styled(
        &self,
        rect: aimer_cupid::utilities::Rect,
        color: aimer_cupid::utilities::Color,
        border_radius: [f32; 4],
        border_width: [f32; 4],
        border_color: aimer_cupid::utilities::Color,
        outline_width: [f32; 4],
        outline_color: aimer_cupid::utilities::Color,
    ) {
        self.commands
            .borrow_mut()
            .push(aimer_cupid::draw_cmd_v2::DrawCommand::FillRect {
                rect,
                color,
                border_radius,
                border_width,
                border_color,
                outline_width,
                outline_color,
            });
    }

    /// Adds a rectangle with one border width on all sides.
    #[inline]
    pub fn fill_rect_with_border(
        &self,
        rect: aimer_cupid::utilities::Rect,
        color: aimer_cupid::utilities::Color,
        border_radius: [f32; 4],
        border_width: f32,
        border_color: aimer_cupid::utilities::Color,
    ) {
        self.fill_rect_styled(
            rect,
            color,
            border_radius,
            [border_width; 4],
            border_color,
            [0.0; 4],
            aimer_cupid::utilities::Color::transparent(),
        );
    }

    /// Adds one text run to this element's local paint list.
    pub fn draw_text(
        &self,
        text: impl Into<String>,
        origin: [f32; 2],
        font_size: f32,
        color: [u8; 4],
    ) {
        self.draw_text_styled(
            std::sync::Arc::<str>::from(text.into()),
            aimer_cupid::utilities::Vec2d::new(origin[0], origin[1]),
            font_size,
            aimer_cupid::utilities::Color::rgba8(color[0], color[1], color[2], color[3]),
            None,
            None,
            aimer_cupid::text_pipeline::TextOverflowMode::Clip,
            aimer_cupid::text_pipeline::text_layout::TextHorizontalAlign::Left,
            aimer_cupid::font::FontFamily::SANS_SERIF,
            aimer_cupid::font::FontStyle::Normal,
            400,
            None,
            true,
        );
    }

    /// Adds one text run with explicit family, style, overflow, and bounds.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_text_styled(
        &self,
        text: impl Into<std::sync::Arc<str>>,
        position: aimer_cupid::utilities::Vec2d,
        font_size: f32,
        color: aimer_cupid::utilities::Color,
        bounds_width: Option<f32>,
        bounds_height: Option<f32>,
        overflow: aimer_cupid::text_pipeline::TextOverflowMode,
        horizontal_align: aimer_cupid::text_pipeline::text_layout::TextHorizontalAlign,
        font_family: aimer_cupid::font::FontFamily,
        font_style: aimer_cupid::font::FontStyle,
        font_weight: u16,
        shadow: Option<aimer_cupid::text_pipeline::TextShadowRequest>,
        draw_glyphs: bool,
    ) {
        self.commands
            .borrow_mut()
            .push(aimer_cupid::draw_cmd_v2::DrawCommand::DrawText {
                position,
                text: text.into(),
                font_size,
                color,
                bounds_width,
                bounds_height,
                overflow,
                horizontal_align,
                font_family,
                font_style,
                font_weight,
                shadow,
                draw_glyphs,
            });
    }

    /// Adds a rich text run with per-span styles.
    pub fn draw_rich_text(
        &self,
        position: aimer_cupid::utilities::Vec2d,
        spans: Vec<aimer_cupid::draw_cmd_v2::RichTextSegment>,
        font_size: f32,
        color: aimer_cupid::utilities::Color,
        bounds_width: Option<f32>,
        bounds_height: Option<f32>,
        overflow: aimer_cupid::text_pipeline::TextOverflowMode,
    ) {
        self.commands
            .borrow_mut()
            .push(aimer_cupid::draw_cmd_v2::DrawCommand::DrawRichText {
                position,
                spans,
                font_size,
                color,
                bounds_width,
                bounds_height,
                overflow,
            });
    }

    /// Adds a text-decoration line.
    pub fn draw_text_decoration(
        &self,
        rect: aimer_cupid::utilities::Rect,
        color: aimer_cupid::utilities::Color,
        style: u32,
        thickness: f32,
        period: f32,
    ) {
        self.commands.borrow_mut().push(
            aimer_cupid::draw_cmd_v2::DrawCommand::DrawTextDecoration {
                rect,
                color,
                style,
                thickness,
                period,
            },
        );
    }

    /// Adds a draw of an image resource that has already been uploaded.
    #[inline]
    pub fn draw_image(&self, rect: aimer_cupid::utilities::Rect, texture_id: u32) {
        self.commands
            .borrow_mut()
            .push(aimer_cupid::draw_cmd_v2::DrawCommand::DrawImage { rect, texture_id });
    }

    /// Adds a draw that retains the image pixels for renderer-side upload.
    #[inline]
    pub fn draw_image_with_resource(
        &self,
        rect: aimer_cupid::utilities::Rect,
        resource: std::sync::Arc<aimer_cupid::draw_cmd_v2::ImageResource>,
    ) {
        self.commands
            .borrow_mut()
            .push(aimer_cupid::draw_cmd_v2::DrawCommand::DrawImageWithResource { rect, resource });
    }

    /// Adds a draw of an SVG scene.
    pub fn draw_svg(
        &self,
        scene: std::sync::Arc<aimer_cupid::svg::SvgScene>,
        destination: aimer_cupid::utilities::Rect,
        overrides: std::sync::Arc<[aimer_cupid::svg::SvgNodeStyleOverride]>,
    ) {
        self.commands
            .borrow_mut()
            .push(aimer_cupid::draw_cmd_v2::DrawCommand::Svg {
                scene,
                destination,
                overrides,
            });
    }

    /// Adds a custom-pipeline request to this element's retained command list.
    #[inline]
    pub fn draw_custom(
        &self,
        pipeline_name: impl Into<std::sync::Arc<str>>,
        data: std::sync::Arc<[u8]>,
    ) {
        self.push_command(aimer_cupid::draw_cmd_v2::DrawCommand::DrawCustom {
            pipeline_name: pipeline_name.into(),
            data,
        });
    }

    /// Declares that this element's commands show only where its descendants
    /// have coverage, and that the descendants are not drawn themselves.
    ///
    /// Call it before any other command so it heads the list. This records the
    /// marker [`COVERAGE_MASK_PIPELINE_NAME`], which makes the renderer draw the
    /// element's commands into one offscreen target and its descendants into
    /// another, then keep the first only where the second has alpha. An element
    /// with no descendants shows nothing.
    ///
    /// [`COVERAGE_MASK_PIPELINE_NAME`]: aimer_cupid::draw_cmd_v2::COVERAGE_MASK_PIPELINE_NAME
    #[inline]
    pub fn mask_by_descendants(&self) {
        self.draw_custom(
            aimer_cupid::draw_cmd_v2::COVERAGE_MASK_PIPELINE_NAME,
            std::sync::Arc::<[u8]>::from(Vec::new()),
        );
    }

    /// Declares that this element's commands are only an alpha mask, and that
    /// its descendants are drawn through it.
    ///
    /// Call it before any other command so it heads the list. This records the
    /// marker [`FADE_MASK_PIPELINE_NAME`], which makes the renderer draw the
    /// element's commands into one offscreen target and its descendants into
    /// another, then scale the descendants by the first target's alpha. The
    /// element's own colors are never shown. An element with no descendants shows
    /// nothing.
    ///
    /// [`FADE_MASK_PIPELINE_NAME`]: aimer_cupid::draw_cmd_v2::FADE_MASK_PIPELINE_NAME
    #[inline]
    pub fn fade_descendants(&self) {
        self.draw_custom(
            aimer_cupid::draw_cmd_v2::FADE_MASK_PIPELINE_NAME,
            std::sync::Arc::<[u8]>::from(Vec::new()),
        );
    }

    /// Declares that this element's first two children are the outgoing and
    /// incoming pictures of a shader transition described by `params`.
    ///
    /// Call it before any other command so it heads the list. This records the
    /// marker [`MORPH_PIPELINE_NAME`], which makes the renderer draw each child
    /// subtree into an offscreen target and blend the two with a shader instead
    /// of painting them on top of each other. The marker draws nothing itself,
    /// and an element that does not have exactly two children paints as an
    /// ordinary one, so record it only while the transition runs.
    ///
    /// [`MORPH_PIPELINE_NAME`]: aimer_cupid::draw_cmd_v2::MORPH_PIPELINE_NAME
    #[inline]
    pub fn morph_children(&self, params: &aimer_cupid::draw_cmd_v2::MorphParams) {
        self.draw_custom(
            aimer_cupid::draw_cmd_v2::MORPH_PIPELINE_NAME,
            std::sync::Arc::<[u8]>::from(params.encode().as_slice()),
        );
    }

    /// Adds a retained Glass/Liquid material request.
    #[inline]
    pub fn draw_material(&self, request: MaterialDrawRequest) {
        self.draw_custom(
            crate::material::MATERIAL_PIPELINE_NAME,
            std::sync::Arc::<[u8]>::from(request.encode()),
        );
    }

    /// Adds a shadow rectangle.
    pub fn draw_shadow_rect(
        &self,
        rect: aimer_cupid::utilities::Rect,
        shadow_color: aimer_cupid::utilities::Color,
        shadow_params: [f32; 4],
        border_radius: [f32; 4],
        inset: bool,
        side_params: [f32; 3],
    ) {
        self.commands
            .borrow_mut()
            .push(aimer_cupid::draw_cmd_v2::DrawCommand::DrawShadowRect {
                rect,
                shadow_color,
                shadow_params,
                border_radius,
                inset,
                side_params,
            });
    }

    /// Adds an explicit local command. State scopes are checked by `finish`.
    #[inline]
    pub fn push_command(&self, command: aimer_cupid::draw_cmd_v2::DrawCommand) {
        self.commands.borrow_mut().push(command);
    }

    /// Begins a clip scope for subsequent commands in this element.
    #[inline]
    pub fn push_clip(&self, rect: aimer_cupid::utilities::Rect, border_radius: [f32; 4]) {
        self.commands
            .borrow_mut()
            .push(aimer_cupid::draw_cmd_v2::DrawCommand::PushClip {
                rect,
                border_radius,
            });
    }

    /// Ends the most recent element-local clip scope.
    #[inline]
    pub fn pop_clip(&self) {
        self.commands
            .borrow_mut()
            .push(aimer_cupid::draw_cmd_v2::DrawCommand::PopClip);
    }

    /// Begins a transform scope for subsequent commands in this element.
    #[inline]
    pub fn push_transform(&self, matrix: aimer_cupid::utilities::Mat3) {
        self.commands
            .borrow_mut()
            .push(aimer_cupid::draw_cmd_v2::DrawCommand::PushTransform { matrix });
    }

    /// Ends the most recent element-local transform scope.
    #[inline]
    pub fn pop_transform(&self) {
        self.commands
            .borrow_mut()
            .push(aimer_cupid::draw_cmd_v2::DrawCommand::PopTransform);
    }

    /// Sets alpha for subsequent local commands until `restore_alpha`.
    #[inline]
    pub fn set_alpha(&self, alpha: f32) {
        self.commands
            .borrow_mut()
            .push(aimer_cupid::draw_cmd_v2::DrawCommand::SetAlpha { alpha });
    }

    /// Restores default alpha for subsequent local commands.
    #[inline]
    pub fn restore_alpha(&self) {
        self.commands
            .borrow_mut()
            .push(aimer_cupid::draw_cmd_v2::DrawCommand::RestoreAlpha);
    }

    /// Sets italic styling for subsequent plain text commands.
    #[inline]
    pub fn set_italic(&self, italic: bool) {
        self.commands
            .borrow_mut()
            .push(aimer_cupid::draw_cmd_v2::DrawCommand::SetItalic { italic });
    }

    /// Sets the language hint for subsequent text commands.
    #[inline]
    pub fn set_text_language(&self, language: Option<aimer_cupid::font::TextLanguage>) {
        self.commands
            .borrow_mut()
            .push(aimer_cupid::draw_cmd_v2::DrawCommand::SetTextLanguage { language });
    }

    /// Sets the current local transform.
    #[inline]
    pub fn set_transform(&self, matrix: aimer_cupid::utilities::Mat3) {
        self.commands
            .borrow_mut()
            .push(aimer_cupid::draw_cmd_v2::DrawCommand::SetTransform { matrix });
    }

    /// Commits this element's local commands and returns their new revision.
    ///
    /// # Panics
    ///
    /// Panics if the commands have unbalanced state scopes or the retained
    /// element no longer exists.
    pub fn finish(self) -> u64 {
        self.try_finish().unwrap_or_else(|error| {
            panic!("Canvas::finish could not commit the element list: {error:?}")
        })
    }

    /// Commits this element's local commands and returns their new revision.
    ///
    /// Returns an error if the commands have unbalanced state scopes or the
    /// retained element no longer exists. If this is not called, dropping the
    /// canvas attempts the same commit automatically.
    pub fn try_finish(mut self) -> Result<u64, aimer_cupid::draw_cmd_v2::RenderTreeError> {
        self.commit_pending()
            .expect("Canvas recording was already finished")
    }

    fn commit_pending(&mut self) -> Option<Result<u64, aimer_cupid::draw_cmd_v2::RenderTreeError>> {
        let writer = self.writer.take()?;
        let commands = std::mem::take(self.commands.get_mut());
        Some(writer.commit(commands))
    }
}

impl Drop for Canvas {
    fn drop(&mut self) {
        if std::thread::panicking() {
            // Dropping the writer marks its previous list dirty so recording
            // can be retried after the paint panic is handled.
            drop(self.writer.take());
            return;
        }

        if let Some(Err(error)) = self.commit_pending() {
            panic!("Canvas dropped with an invalid element list: {error:?}");
        }
    }
}
