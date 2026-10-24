use std::cell::{Cell, UnsafeCell};
use std::collections::HashMap;

use aimer_attribute::Dimension;
use aimer_attribute::size::Size;
use aimer_style::BoxFit;
use aimer_widget::base::BuildContext;
use aimer_widget::{AnyElement, AnyWidget, Element, LayoutCache, Widget};

use crate::img_widget::image_widget::RawImageWidget;
use crate::img_widget::source::ImageSource;

/// Displays an image fetched from a network URL.
///
/// Requests and image decoding are asynchronous, and results are cached by URL.
/// Use [`NetworkImage::loading_widget`] and [`NetworkImage::error_widget`] to
/// replace the default empty loading state and magenta-and-black error pattern.
/// Request headers can be supplied with [`NetworkImage::header`].
///
/// # Example
///
/// ```
/// use std::collections::HashMap;
///
/// use aimer_assets::NetworkImage;
/// use aimer_style::BoxFit;
///
/// let mut headers = HashMap::new();
/// headers.insert("Accept".to_owned(), "image/webp,image/*".to_owned());
///
/// let image = NetworkImage::new("https://example.com/photo.webp").header(headers)
///                                                                .width(320.0)
///                                                                .height(180.0)
///                                                                .fit(BoxFit::Cover);
/// ```
#[derive(aimer_macro::PortableWidget)]
#[portable_widget(id = "aimer_assets::NetworkImage", schema_only)]
pub struct NetworkImage {
    pub url: String,
    pub width: Dimension,
    pub height: Dimension,
    #[portable_skip]
    pub fit: BoxFit,
    #[portable_skip]
    pub header: Option<HashMap<String, String>>,
    #[portable_skip]
    pub error_widget: Option<AnyWidget>,
    #[portable_skip]
    pub loading_widget: Option<AnyWidget>,
    #[portable_skip]
    pub delay: Option<u64>,
    pub scale: f32,
}

impl NetworkImage {
    /// Creates a network image for `url`.
    ///
    /// Width and height default to [`Dimension::Auto`], [`BoxFit::None`] is
    /// used, no request headers or fallback widgets are set, and the
    /// drawing scale is `1.0`. The URL is not requested until the widget is
    /// drawn.
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            width: Dimension::default(),
            height: Dimension::default(),
            fit: BoxFit::default(),
            header: None,
            error_widget: None,
            loading_widget: None,
            delay: None,
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

    /// Sets the complete map of HTTP request headers.
    ///
    /// Calling this builder again replaces the previous map. Invalid header
    /// names or values cause loading to enter the error state rather than
    /// panic.
    pub fn header(mut self, header: HashMap<String, String>) -> Self {
        self.header = Some(header);
        self
    }

    /// Sets the widget drawn when the request or image decoding fails.
    ///
    /// It replaces the built-in magenta-and-black error pattern.
    pub fn error_widget(mut self, error_widget: impl Widget + 'static) -> Self {
        self.error_widget = Some(error_widget.boxed());
        self
    }

    /// Sets the widget drawn while the request and decoding are in progress.
    ///
    /// Without a loading widget, the image draws no content until it is ready.
    pub fn loading_widget(mut self, loading_widget: impl Widget + 'static) -> Self {
        self.loading_widget = Some(loading_widget.boxed());
        self
    }

    /// Stores a requested loading delay in milliseconds.
    ///
    /// The current renderer does not apply this value, so this builder has no
    /// effect on request timing or fallback display yet. The default is no
    /// delay.
    pub fn delay(mut self, delay: u64) -> Self {
        self.delay = Some(delay);
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

impl Widget for NetworkImage {
    #[track_caller]
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let source = match self.header.as_ref() {
            Some(header) => ImageSource::NetworkWithHeaders(self.url.clone(), header.clone()),
            None => ImageSource::Network(self.url.clone()),
        };

        // debug!("creating network image widget with url: {}", self.url);

        RawImageWidget {
            source,
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
        }
        .boxed()
    }

    fn debug_name(&self) -> &'static str {
        "NetworkImage"
    }
}
