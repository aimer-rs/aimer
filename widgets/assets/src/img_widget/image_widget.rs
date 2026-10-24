use std::cell::{Cell, UnsafeCell};
use std::path::PathBuf;

use aimer_attribute::Dimension;
use aimer_macro::{EventElement, Rebuildable};
use aimer_style::BoxFit;
use aimer_widget::base::{BuildContext, Color, Colors, ResolvedSize, Size, Vec2d};
use aimer_widget::{
    AnyElement, Drawable, Element, LayoutCache, LayoutElement, VisitorElement, Widget,
};

use crate::ImageResult::Success;
use crate::img_widget::source::ImageSource;
use crate::{ImageProvider, ImageResult};

#[derive(Clone, Copy, Debug, PartialEq)]
struct ImagePaintGeometry {
    pos: Vec2d,
    size: ResolvedSize,
    use_cover: bool,
}

fn image_paint_geometry(
    target: ResolvedSize,
    intrinsic: Option<(u32, u32)>,
    fit: BoxFit,
    scale_factor: f32,
) -> Option<ImagePaintGeometry> {
    let (iw, ih) = intrinsic?;
    if iw == 0 || ih == 0 {
        return None;
    }

    let target_w = target.width.max(0.0);
    let target_h = target.height.max(0.0);
    let iw = iw as f32;
    let ih = ih as f32;
    let scale_x = target_w / iw;
    let scale_y = target_h / ih;
    let (final_w, final_h, use_cover) = match fit {
        BoxFit::Contain | BoxFit::ScaleDown | BoxFit::None => {
            let mut scale = scale_x.min(scale_y);
            if let BoxFit::ScaleDown = fit {
                scale = scale.min(1.0);
            }
            (iw * scale * scale_factor, ih * scale * scale_factor, false)
        }
        BoxFit::FitWidth => {
            let final_w = target_w * scale_factor;
            (final_w, final_w * (ih / iw), false)
        }
        BoxFit::FitHeight => {
            let final_h = target_h * scale_factor;
            (final_h * (iw / ih), final_h, false)
        }
        BoxFit::Cover => {
            let scale = scale_x.max(scale_y);
            (iw * scale * scale_factor, ih * scale * scale_factor, true)
        }
        BoxFit::Fill => {
            let scale = scale_x.min(scale_y);
            (iw * scale * scale_factor, ih * scale * scale_factor, false)
        }
    };

    Some(ImagePaintGeometry {
        pos: Vec2d {
            x: (target_w - final_w) * 0.5,
            y: (target_h - final_h) * 0.5,
        },
        size: ResolvedSize {
            width: final_w,
            height: final_h,
        },
        use_cover,
    })
}

/// Displays an image read from a file-system path.
///
/// The file is loaded and decoded asynchronously and cached by path. While it
/// is loading, this widget draws no content. If reading or decoding fails, it
/// draws a magenta-and-black error pattern; unlike
/// [`AssetImage`](crate::AssetImage) and [`NetworkImage`](crate::NetworkImage),
/// `Image` does not provide custom fallback builders.
///
/// # Example
///
/// ```
/// use aimer_assets::Image;
/// use aimer_style::BoxFit;
///
/// let image = Image::new("images/photo.png").width(320.0)
///                                           .height(180.0)
///                                           .fit(BoxFit::Cover);
/// ```
#[derive(aimer_macro::PortableWidget)]
#[portable_widget(id = "aimer_assets::Image", schema_only)]
pub struct Image {
    #[portable_skip]
    pub path: PathBuf,
    pub width: Dimension,
    pub height: Dimension,
    #[portable_skip]
    pub fit: BoxFit,
    pub scale: f32,
}

impl Image {
    /// Creates an image that reads from `path`.
    ///
    /// Width and height default to [`Dimension::Auto`], [`BoxFit::None`] is
    /// used, and the drawing scale is `1.0`. The path is interpreted by the
    /// host file system and is not an `aimer.toml` asset key.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            width: Dimension::default(),
            height: Dimension::default(),
            fit: BoxFit::default(),
            scale: 1.0,
        }
    }

    /// Sets the width of the widget's layout box.
    ///
    /// The default is [`Dimension::Auto`].
    pub fn width(mut self, width: impl Into<Dimension>) -> Self {
        self.width = width.into();
        self
    }

    /// Sets the height of the widget's layout box.
    ///
    /// The default is [`Dimension::Auto`].
    pub fn height(mut self, height: impl Into<Dimension>) -> Self {
        self.height = height.into();
        self
    }

    /// Sets how the image is fitted into its layout box.
    ///
    /// The default is [`BoxFit::None`]. Every mode except [`BoxFit::Fill`]
    /// preserves the image's aspect ratio; `Fill` stretches it to the box.
    pub fn fit(mut self, fit: BoxFit) -> Self {
        self.fit = fit;
        self
    }

    /// Multiplies the final painted image size around the center of its layout
    /// box.
    ///
    /// This does not change the widget's layout size. The default is `1.0`;
    /// values are stored without validation, so callers should provide a
    /// finite, non-negative value.
    pub fn scale(mut self, scale: impl Into<f32>) -> Self {
        self.scale = scale.into();
        self
    }
}

impl Widget for Image {
    fn to_element(self, _: &BuildContext) -> AnyElement {
        RawImageWidget {
            source: ImageSource::File(self.path.clone()),
            size: Size {
                width: self.width,
                height: self.height,
            },
            cache: LayoutCache::new(),
            fit: self.fit,
            keep_aspect_ratio: self.fit != BoxFit::Fill,
            loading_element: None,
            error_element: None,
            original_size: Cell::new(None),
            cached_id: UnsafeCell::new(None),
            cached_texture_epoch: Cell::new(0),
            paint_changed: Cell::new(false),
            scale: self.scale,
        }
        .boxed()
    }

    fn debug_name(&self) -> &'static str {
        "Image"
    }
}

#[derive(Rebuildable, EventElement)]
pub struct RawImageWidget<P: ImageProvider> {
    pub source: P,
    pub size: Size,
    pub cache: LayoutCache,
    pub fit: BoxFit,
    pub keep_aspect_ratio: bool,
    pub original_size: Cell<Option<Size>>,
    pub loading_element: Option<AnyElement>,
    pub error_element: Option<AnyElement>,
    pub cached_id: UnsafeCell<Option<ImageResult>>,
    /// Last renderer image-cache generation observed by this retained widget.
    /// A changed generation makes the provider revalidate its cached image ID
    /// before the widget records another draw command.
    pub cached_texture_epoch: Cell<u64>,
    /// Set when the provider result changed after the last local v2 recording,
    /// so the retained node re-records exactly when its pixels can differ.
    pub paint_changed: Cell<bool>,
    pub scale: f32,
}

impl<P: ImageProvider> VisitorElement for RawImageWidget<P> {
    /// Exposes only the element of the current state, so the retained render
    /// tree has a node for what is actually shown. An image that has not
    /// settled yet counts as loading.
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        let state = unsafe { &*self.cached_id.get() };
        let shown = match state {
            None | Some(ImageResult::Loading) => self.loading_element.as_ref(),
            Some(ImageResult::Error(_)) => self.error_element.as_ref(),
            Some(Success(_)) => None,
        };
        if let Some(element) = shown {
            visitor(element.as_ref());
        }
    }

    fn debug_name(&self) -> &'static str {
        "RawImageElement"
    }
}

impl<P: ImageProvider> LayoutElement for RawImageWidget<P> {
    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        let scale_bits = ctx.scale.to_bits();
        if let Some(cached) = self.cache.get_computed(ctx.box_constraint, scale_bits) {
            return cached;
        }

        let result = self.size.resolve(&ctx.parent_size, ctx.scale);

        self.cache
            .set_computed(ctx.box_constraint, scale_bits, result);

        result
    }

    fn content_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.size.resolve(&ctx.parent_size, ctx.scale)
    }

    fn invalidate_layout(&self) {
        self.cache.invalidate();
    }
}

impl<P: ImageProvider> RawImageWidget<P> {
    fn cache_result(&self, result: ImageResult, ctx: &BuildContext) -> ImageResult {
        let was_loading = unsafe { matches!(&*self.cached_id.get(), Some(ImageResult::Loading)) };
        if was_loading && result != ImageResult::Loading {
            // An async provider can finish after an AnimatedSwitcher has
            // stopped animating. Requesting a frame alone is insufficient in
            // that case because retained paint may have no damage to replay.
            aimer_widget::mark_paint_damage_full();
        }
        let previous = unsafe { (*self.cached_id.get()).replace(result.clone()) };
        if previous.as_ref() != Some(&result) {
            self.paint_changed.set(true);
        }
        self.cached_texture_epoch
            .set(ctx.canvas.texture_cache_epoch());
        result
    }

    /// Brings the cached provider result up to date and returns it: polls a
    /// loading image and revalidates a success after the renderer cache moved.
    fn refresh(&self, ctx: &BuildContext) -> ImageResult {
        if let Some(result) = unsafe { &*self.cached_id.get() } {
            let cache_invalidated = match result {
                Success(_) => {
                    let epoch = ctx.canvas.texture_cache_epoch();
                    let invalidated = epoch != self.cached_texture_epoch.get();
                    self.cached_texture_epoch.set(epoch);
                    invalidated
                }
                _ => false,
            };
            if cache_invalidated {
                // Keep the previous result in place: `cache_result` compares
                // against it, so an unchanged texture does not re-record.
                let r = self.source.get_image(ctx);
                self.cache_result(r, ctx)
            } else if result == &ImageResult::Loading {
                let r = self.source.get_image(ctx);
                self.cache_result(r, ctx)
            } else {
                result.clone()
            }
        } else {
            let result = self.source.get_image(ctx);
            self.cache_result(result, ctx)
        }
    }

    /// Records the magenta/black checkerboard shown for a failed image that has
    /// no error element. `target` is in device pixels, the list in logical.
    fn paint_error_checkerboard(
        &self,
        canvas: &aimer_canvas::Canvas,
        target: ResolvedSize,
        scale: f32,
    ) {
        let grid_size = 32.0;
        let rows = (target.height / grid_size).ceil() as i32;
        let cols = (target.width / grid_size).ceil() as i32;
        for row in 0..rows {
            for col in 0..cols {
                let color = if (row + col) % 2 == 0 {
                    Color::Basic(Colors::Magenta)
                } else {
                    Color::Basic(Colors::Black)
                };
                let x = col as f32 * grid_size;
                let y = row as f32 * grid_size;
                let width = grid_size.min(target.width - x);
                let height = grid_size.min(target.height - y);
                if width > 0.0 && height > 0.0 {
                    let (red, green, blue, alpha) = color.to_rgba();
                    canvas.fill_rect(
                        aimer_cupid::draw_cmd_v2::Rect::new(
                            x / scale,
                            y / scale,
                            width / scale,
                            height / scale,
                        ),
                        [red, green, blue, alpha],
                    );
                }
            }
        }
    }

    fn local_v2_image_paint(
        &self,
        ctx: &BuildContext,
    ) -> Option<(
        u32,
        ResolvedSize,
        ImagePaintGeometry,
        Option<std::sync::Arc<aimer_cupid::draw_cmd_v2::ImageResource>>,
    )> {
        let resource = self.source.retained_image_resource(ctx);
        let id = if let Some(resource) = &resource {
            resource.texture_id()
        } else {
            let cached = unsafe { &*self.cached_id.get() }.as_ref()?;
            let Success(id) = cached else {
                return None;
            };
            *id
        };
        let scale = ctx.scale;
        if !scale.is_finite()
            || scale <= 0.0
            || ((self.cached_texture_epoch.get() != ctx.canvas.texture_cache_epoch()
                || !ctx.canvas.is_texture_available(id))
                && resource.is_none())
        {
            return None;
        }

        let target = self.computed_size(ctx);
        if !target.width.is_finite()
            || !target.height.is_finite()
            || target.width < 0.0
            || target.height < 0.0
            || !self.scale.is_finite()
            || self.scale < 0.0
        {
            return None;
        }

        let geometry = if self.keep_aspect_ratio {
            let intrinsic_size = resource
                .as_ref()
                .map(|resource| (resource.intrinsic_width(), resource.intrinsic_height()))
                .or_else(|| ctx.canvas.get_image_size(id));
            image_paint_geometry(target, intrinsic_size, self.fit, self.scale)?
        } else {
            let width = target.width * self.scale;
            let height = target.height * self.scale;
            ImagePaintGeometry {
                pos: Vec2d {
                    x: (target.width - width) * 0.5,
                    y: (target.height - height) * 0.5,
                },
                size: ResolvedSize { width, height },
                use_cover: false,
            }
        };

        if !geometry.pos.x.is_finite()
            || !geometry.pos.y.is_finite()
            || !geometry.size.width.is_finite()
            || !geometry.size.height.is_finite()
            || geometry.size.width < 0.0
            || geometry.size.height < 0.0
        {
            return None;
        }

        Some((id, target, geometry, resource))
    }
}

impl<P: ImageProvider> Drawable for RawImageWidget<P> {
    fn update(&self, ctx: &BuildContext) {
        // The image, or the error checkerboard, is painted by `paint_local_v2`.
        // This traversal advances the provider and visits the retained loading
        // and error elements, which own their own render nodes.
        match self.refresh(ctx) {
            Success(_) => {}
            ImageResult::Loading => {
                if let Some(loading_element) = &self.loading_element {
                    loading_element.update(ctx);
                }
            }
            ImageResult::Error(_) => {
                if let Some(error_element) = &self.error_element {
                    error_element.update(ctx);
                }
            }
        }
    }

    /// Every state has a local list: the image, the error checkerboard, or
    /// nothing while loading (the loading and error elements are retained
    /// children). Provider work is done by `sync_local_v2_state`, not here.
    #[inline]
    fn can_paint_local_v2(&self, _ctx: &BuildContext) -> bool {
        true
    }

    fn sync_local_v2_state(&self, ctx: &BuildContext) -> bool {
        self.sync_paint_geometry(ctx);
        self.refresh(ctx);
        true
    }

    #[inline]
    fn local_v2_paint_needs_recording(&self, _ctx: &BuildContext) -> bool {
        self.paint_changed.get()
    }

    fn retained_v2_paint_outsets(&self, ctx: &BuildContext) -> Option<[f32; 4]> {
        let (_, target, geometry, _) = self.local_v2_image_paint(ctx)?;
        if geometry.use_cover {
            return None;
        }
        let scale = ctx.scale;
        let outsets = [
            (-geometry.pos.x).max(0.0),
            (-geometry.pos.y).max(0.0),
            (geometry.pos.x + geometry.size.width - target.width).max(0.0),
            (geometry.pos.y + geometry.size.height - target.height).max(0.0),
        ]
        .map(|outset| outset / scale);
        outsets
            .iter()
            .any(|outset| *outset > 0.0)
            .then_some(outsets)
    }

    fn paint_local_v2(&self, ctx: &BuildContext) {
        self.paint_changed.set(false);
        let canvas = aimer_canvas::Canvas::of(ctx);
        let scale = ctx.scale;
        if let Some((id, target, geometry, resource)) = self.local_v2_image_paint(ctx) {
            if geometry.use_cover {
                canvas.push_clip(
                    aimer_cupid::draw_cmd_v2::Rect::new(
                        0.0,
                        0.0,
                        target.width / scale,
                        target.height / scale,
                    ),
                    [0.0; 4],
                );
            }
            let destination = aimer_cupid::draw_cmd_v2::Rect::new(
                geometry.pos.x / scale,
                geometry.pos.y / scale,
                geometry.size.width / scale,
                geometry.size.height / scale,
            );
            if let Some(resource) = resource {
                canvas.draw_image_with_resource(destination, resource);
            } else {
                canvas.draw_image(destination, id);
            }
            if geometry.use_cover {
                canvas.pop_clip();
            }
        } else if matches!(
            unsafe { &*self.cached_id.get() },
            Some(ImageResult::Error(_))
        ) && self.error_element.is_none()
            && scale.is_finite()
            && scale > 0.0
        {
            self.paint_error_checkerboard(&canvas, self.computed_size(ctx), scale);
        }
        canvas.finish();
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use super::*;
    use aimer_container::ZeroSizedBox;
    use aimer_cupid::draw_cmd_v2::{DrawCommand, Rect, RenderPaintSource, RenderTree};

    #[derive(Clone, Debug)]
    struct LoadingThenError {
        calls: Rc<Cell<usize>>,
    }

    impl ImageProvider for LoadingThenError {
        fn get_image(&self, _ctx: &BuildContext) -> ImageResult {
            let call = self.calls.get();
            self.calls.set(call + 1);
            if call == 0 {
                ImageResult::Loading
            } else {
                ImageResult::Error("ready".to_owned())
            }
        }
    }

    #[derive(Clone, Debug)]
    struct CountingSuccess {
        calls: Rc<Cell<usize>>,
    }

    impl ImageProvider for CountingSuccess {
        fn get_image(&self, ctx: &BuildContext) -> ImageResult {
            let call = self.calls.get();
            self.calls.set(call + 1);
            ctx.canvas.set_texture_size(7, 1, 1);
            ImageResult::Success(7)
        }
    }

    fn context() -> BuildContext<'static> {
        use aimer_attribute::{BoxConstraint, Vec2d};
        use aimer_canvas::{FrameCanvas, InnerCanvas};
        use aimer_widget::base::WindowHandle;

        static RUNTIME: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
        let runtime = RUNTIME.get_or_init(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("image widget test runtime should build")
        });
        let _guard = runtime.enter();
        let inner = Box::leak(Box::new(InnerCanvas::new()));
        let mut context = BuildContext::new(
            FrameCanvas::new(inner),
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

    fn image_with_source<P: ImageProvider>(source: P) -> RawImageWidget<P> {
        RawImageWidget {
            source,
            size: Size::new(Dimension::Px(16.0), Dimension::Px(16.0)),
            cache: LayoutCache::new(),
            fit: BoxFit::Fill,
            keep_aspect_ratio: false,
            original_size: Cell::new(None),
            loading_element: None,
            error_element: None,
            cached_id: UnsafeCell::new(None),
            cached_texture_epoch: Cell::new(0),
            paint_changed: Cell::new(false),
            scale: 1.0,
        }
    }

    fn loading_then_error_image(calls: Rc<Cell<usize>>) -> RawImageWidget<LoadingThenError> {
        image_with_source(LoadingThenError { calls })
    }

    #[test]
    fn aspect_preserving_geometry_waits_for_intrinsic_dimensions() {
        let target = ResolvedSize {
            width: 1_000.0,
            height: 450.0,
        };

        assert_eq!(
            image_paint_geometry(target, None, BoxFit::Contain, 1.0),
            None,
            "missing image metadata must not turn the layout box into a stretched image"
        );
    }

    #[test]
    fn aspect_preserving_geometry_contains_a_wide_image() {
        let target = ResolvedSize {
            width: 1_000.0,
            height: 450.0,
        };

        let geometry = image_paint_geometry(target, Some((2_170, 1_516)), BoxFit::Contain, 1.0)
            .expect("known image metadata produces paint geometry");

        assert!((geometry.pos.x - 177.9353).abs() < 0.01);
        assert!(geometry.pos.y.abs() < 0.01);
        assert!((geometry.size.width - 644.1293).abs() < 0.01);
        assert!((geometry.size.height - 450.0).abs() < 0.01);
        assert!(!geometry.use_cover);
    }

    #[test]
    fn async_image_completion_invalidates_paint_after_loading_frame() {
        let ctx = context();
        let image = loading_then_error_image(Rc::new(Cell::new(0)));

        aimer_widget::begin_paint_frame(32, 32);
        image.update(&ctx);
        let _ = aimer_widget::take_paint_frame_damage(32, 32);

        aimer_widget::begin_paint_frame(32, 32);
        image.update(&ctx);
        let damage = aimer_widget::take_paint_frame_damage(32, 32);

        assert!(
            damage.is_full(),
            "a loading image becoming drawable must invalidate its retained paint"
        );
    }

    #[test]
    fn image_provider_is_revalidated_after_another_texture_changes() {
        let ctx = context();
        let calls = Rc::new(Cell::new(0));
        let image = image_with_source(CountingSuccess {
            calls: Rc::clone(&calls),
        });

        aimer_widget::begin_paint_frame(32, 32);
        image.update(&ctx);
        let _ = aimer_widget::take_paint_frame_damage(32, 32);
        assert_eq!(calls.get(), 1);

        let _ = ctx.canvas.load_image(&[1, 2, 3, 4], 1, 1);

        aimer_widget::begin_paint_frame(32, 32);
        image.update(&ctx);
        let _ = aimer_widget::take_paint_frame_damage(32, 32);

        assert_eq!(
            calls.get(),
            2,
            "a retained success must be revalidated after the renderer cache changes"
        );
    }

    #[test]
    fn cached_image_records_v2_draw_and_cover_clip_without_redirtying_sibling() {
        use aimer_canvas::Canvas;

        let ctx = context();
        let calls = Rc::new(Cell::new(0));
        let mut image = image_with_source(CountingSuccess {
            calls: calls.clone(),
        });
        image.fit = BoxFit::Cover;
        image.keep_aspect_ratio = true;
        let texture_id = ctx
            .canvas
            .load_image(&[255, 0, 0, 255, 0, 255, 0, 255], 2, 1);
        ctx.canvas.set_texture_size(texture_id, 2, 1);
        image.cache_result(ImageResult::Success(texture_id), &ctx);

        let image_bounds = Rect::new(0.0, 0.0, 16.0, 16.0);
        let tree = RenderTree::new();
        let image_node = tree.add_root(image_bounds).unwrap();
        let sibling_bounds = Rect::new(30.0, 0.0, 8.0, 8.0);
        let sibling_node = tree.add_root(sibling_bounds).unwrap();
        assert!(image.can_paint_local_v2(&ctx));
        let image_context = tree.context(image_node).unwrap();
        ctx.with_local_v2_paint_context(image_context, |ctx| image.paint_local_v2(ctx));
        tree.set_paint_source(image_node, RenderPaintSource::LocalV2)
            .unwrap();

        let sibling_context = tree.context(sibling_node).unwrap();
        let sibling_canvas = Canvas::of(&sibling_context);
        sibling_canvas.fill_rect(Rect::new(0.0, 0.0, 8.0, 8.0), [0, 255, 0, 255]);
        sibling_canvas.finish();
        tree.set_paint_source(sibling_node, RenderPaintSource::LocalV2)
            .unwrap();

        let first_image = tree.draw_list_snapshot(image_node).unwrap();
        assert!(matches!(
            first_image.commands.as_ref(),
            [
                DrawCommand::PushClip { rect, .. },
                DrawCommand::DrawImage { rect: image_rect, texture_id: recorded_id },
                DrawCommand::PopClip,
            ] if *rect == image_bounds
                && *image_rect == Rect::new(-8.0, 0.0, 32.0, 16.0)
                && *recorded_id == texture_id
        ));
        assert_eq!(
            calls.get(),
            0,
            "v2 recording must not request or upload images"
        );

        tree.take_damage();
        tree.invalidate_paint(image_node).unwrap();
        let image_context = tree.context(image_node).unwrap();
        ctx.with_local_v2_paint_context(image_context, |ctx| image.paint_local_v2(ctx));

        assert_eq!(
            tree.draw_list_revision(image_node).unwrap(),
            first_image.revision + 1
        );
        assert_eq!(tree.draw_list_revision(sibling_node).unwrap(), 1);
        assert_eq!(tree.take_damage(), vec![image_bounds]);
    }

    fn record_paint<P: ImageProvider>(
        image: &RawImageWidget<P>,
        ctx: &BuildContext,
    ) -> aimer_cupid::draw_cmd_v2::DrawListSnapshot {
        let tree = RenderTree::new();
        let node = tree.add_root(Rect::new(0.0, 0.0, 16.0, 16.0)).unwrap();
        let node_context = tree.context(node).unwrap();
        ctx.with_local_v2_paint_context(node_context, |ctx| image.paint_local_v2(ctx));
        tree.draw_list_snapshot(node).unwrap()
    }

    #[test]
    fn a_loading_image_paints_locally_without_touching_the_provider() {
        let ctx = context();
        let calls = Rc::new(Cell::new(0));
        let loading = loading_then_error_image(calls.clone());

        assert!(
            loading.can_paint_local_v2(&ctx),
            "a loading image is an empty local list, never a legacy island"
        );
        assert!(record_paint(&loading, &ctx).commands.is_empty());
        assert_eq!(calls.get(), 0, "recording must not poll the provider");
    }

    #[test]
    fn a_stale_image_paints_nothing_until_the_provider_is_revalidated() {
        let ctx = context();
        let stale_calls = Rc::new(Cell::new(0));
        let stale = image_with_source(CountingSuccess {
            calls: stale_calls.clone(),
        });
        let texture_id = ctx.canvas.load_image(&[1, 2, 3, 4], 1, 1);
        stale.cache_result(ImageResult::Success(texture_id), &ctx);
        assert_eq!(record_paint(&stale, &ctx).commands.len(), 1);

        let _new_texture = ctx.canvas.load_image(&[5, 6, 7, 8], 1, 1);
        assert!(stale.can_paint_local_v2(&ctx));
        assert!(record_paint(&stale, &ctx).commands.is_empty());
        assert_eq!(stale_calls.get(), 0, "recording must not poll the provider");

        assert!(stale.sync_local_v2_state(&ctx));
        assert_eq!(stale_calls.get(), 1, "syncing revalidates the stale image");
        assert_eq!(record_paint(&stale, &ctx).commands.len(), 1);
    }

    #[test]
    fn syncing_settles_the_provider_state_and_requests_one_recording() {
        let ctx = context();
        let calls = Rc::new(Cell::new(0));
        let image = loading_then_error_image(calls.clone());

        assert!(image.sync_local_v2_state(&ctx));
        assert!(image.local_v2_paint_needs_recording(&ctx));
        record_paint(&image, &ctx);
        assert!(
            !image.local_v2_paint_needs_recording(&ctx),
            "recording consumes the pending paint change"
        );

        assert!(image.sync_local_v2_state(&ctx));
        assert_eq!(calls.get(), 2);
        assert!(
            image.local_v2_paint_needs_recording(&ctx),
            "loading to error changes what the node must paint"
        );
        record_paint(&image, &ctx);

        assert!(image.sync_local_v2_state(&ctx));
        assert!(
            !image.local_v2_paint_needs_recording(&ctx),
            "an unchanged result must not re-record"
        );
    }

    #[test]
    fn a_failed_image_paints_the_error_checkerboard_locally() {
        let ctx = context();
        let image = loading_then_error_image(Rc::new(Cell::new(0)));
        image.sync_local_v2_state(&ctx);
        image.sync_local_v2_state(&ctx);

        let snapshot = record_paint(&image, &ctx);
        assert!(matches!(
            snapshot.commands.as_ref(),
            [DrawCommand::FillRect { rect, .. }] if *rect == Rect::new(0.0, 0.0, 16.0, 16.0)
        ));
    }

    #[test]
    fn only_the_active_state_element_is_a_retained_child() {
        let ctx = context();
        let calls = Rc::new(Cell::new(0));
        let mut image = loading_then_error_image(calls);
        image.loading_element = Some(ZeroSizedBox.to_element(&ctx));
        image.error_element = Some(ZeroSizedBox.to_element(&ctx));
        let count = |image: &RawImageWidget<LoadingThenError>| {
            let mut children = Vec::new();
            image.visit_children(&mut |child| {
                children.push(child as *const dyn Element as *const ())
            });
            children
        };
        let loading =
            image.loading_element.as_ref().unwrap().as_ref() as *const dyn Element as *const ();
        let error =
            image.error_element.as_ref().unwrap().as_ref() as *const dyn Element as *const ();

        assert_eq!(
            count(&image),
            vec![loading],
            "an unsettled image shows its loading element"
        );
        image.sync_local_v2_state(&ctx);
        assert_eq!(count(&image), vec![loading]);
        image.sync_local_v2_state(&ctx);
        assert_eq!(count(&image), vec![error]);
    }

    #[test]
    fn an_overflowing_image_reports_its_paint_outsets() {
        let ctx = context();
        let mut image = image_with_source(CountingSuccess {
            calls: Rc::new(Cell::new(0)),
        });
        image.scale = 2.0;
        let texture_id = ctx.canvas.load_image(&[1, 2, 3, 4], 1, 1);
        image.cache_result(ImageResult::Success(texture_id), &ctx);

        assert!(image.can_paint_local_v2(&ctx));
        assert_eq!(
            image.retained_v2_paint_outsets(&ctx),
            Some([8.0, 8.0, 8.0, 8.0]),
            "a 2x image overflows its 16px box by half its size on every side"
        );
        let snapshot = record_paint(&image, &ctx);
        assert!(matches!(
            snapshot.commands.as_ref(),
            [DrawCommand::DrawImage { rect, .. }] if *rect == Rect::new(-8.0, -8.0, 32.0, 32.0)
        ));
    }
}
