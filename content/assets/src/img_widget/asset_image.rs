use std::cell::{Cell, UnsafeCell};

use aimer_attribute::Dimension;
use aimer_attribute::size::Size;
use aimer_style::BoxFit;
use aimer_widget::base::BuildContext;
use aimer_widget::{AnyElement, AnyWidget, Element, LayoutCache, Widget};

use crate::img_widget::gif::{GifController, GifLoops, GifOptions, GifSlot};
use crate::img_widget::image_widget::RawImageWidget;
use crate::img_widget::source::ImageSource;

/// Displays an image bundled with the app and registered under `[assets]` in
/// `aimer.toml`.
///
/// The `key` is the path declared in the manifest (relative to the project
/// root, e.g. `"assets/logo.png"`); it is resolved per platform at runtime
/// through [`ImageSource::Asset`]: from the APK on Android, the app bundle on
/// iOS/macOS, the project directory during desktop development, and via `fetch`
/// from the site root on web.
///
/// Loading and decoding happen asynchronously. Until the image is ready the
/// widget supplied by [`AssetImage::loading_widget`] is drawn, or nothing is
/// drawn when no loading widget is set. A load or decode failure similarly
/// draws the [`AssetImage::error_widget`]; without one, the renderer uses its
/// built-in magenta-and-black error pattern.
///
/// A bundled GIF plays as an animation; see [`AssetImage::gif_controller`] and
/// [`AssetImage::gif_loops`].
pub struct AssetImage {
    pub key: String,
    pub width: Dimension,
    pub height: Dimension,
    pub fit: BoxFit,
    pub error_widget: Option<AnyWidget>,
    pub loading_widget: Option<AnyWidget>,
    pub scale: f32,
    pub animation: GifOptions,
}

impl AssetImage {
    /// Creates an asset image for the registered asset `key`.
    ///
    /// Width and height default to [`Dimension::Auto`], [`BoxFit::None`] is
    /// used, no loading or error widget is installed, and the drawing scale
    /// is `1.0`.
    pub fn new(key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            width: Dimension::default(),
            height: Dimension::default(),
            fit: BoxFit::default(),
            error_widget: None,
            loading_widget: None,
            scale: 1.0,
            animation: GifOptions::new(),
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

    /// Sets the widget drawn when loading or decoding the asset fails.
    ///
    /// It replaces the built-in magenta-and-black error pattern. The fallback
    /// is converted to an element with the same build context as this
    /// image.
    pub fn error_widget(mut self, error_widget: impl Widget + 'static) -> Self {
        self.error_widget = Some(error_widget.boxed());
        self
    }

    /// Sets the widget drawn while the asset is being loaded and decoded.
    ///
    /// Without a loading widget, the image draws no content until it is ready.
    pub fn loading_widget(mut self, loading_widget: impl Widget + 'static) -> Self {
        self.loading_widget = Some(loading_widget.boxed());
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

    /// Attaches a controller that plays, pauses and seeks the image when it is
    /// an animated GIF. It has no effect on other images.
    pub fn gif_controller(mut self, controller: GifController) -> Self {
        self.animation = self.animation.controller(controller);
        self
    }

    /// Sets how many times an animated GIF plays. The default follows the
    /// loop count stored in the file.
    pub fn gif_loops(mut self, loops: GifLoops) -> Self {
        self.animation = self.animation.loops(loops);
        self
    }
}

impl Widget for AssetImage {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        RawImageWidget {
            source: ImageSource::Asset(self.key.clone()),
            size: Size::new(self.width, self.height),
            fit: self.fit,
            keep_aspect_ratio: self.fit != BoxFit::Fill,
            error_element: self.error_widget.map(|w| w.to_element(ctx)),
            loading_element: self.loading_widget.map(|w| w.to_element(ctx)),
            cache: LayoutCache::new(),
            original_size: Cell::new(None),
            cached_id: UnsafeCell::new(None),
            cached_texture_epoch: Cell::new(0),
            paint_changed: Cell::new(false),
            scale: self.scale,
            animation: GifSlot::new(self.animation),
        }
        .boxed()
    }

    fn debug_name(&self) -> &'static str {
        "AssetImage"
    }
}

