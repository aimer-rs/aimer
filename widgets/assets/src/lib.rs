pub mod img_widget;

mod asset_manager;
mod icon;

pub use asset_manager::{
    AnimationPolicy, AssetCacheConfig, AssetCacheKey, AssetData, AssetError, AssetErrorKind,
    AssetId, AssetIdError, AssetLoadOperation, AssetLoadPoll, AssetManager, AssetManifest,
    AssetMetadata, AssetOperation, AssetPolicy, AssetProgress, AssetRef, AssetRequest,
    AssetResolver, AssetSource, CacheStats, DecodeProfile, LoadHandle, LoadState,
};
pub use icon::{
    Icon, IconContext, IconDirection, IconError, IconSource, IconTheme, IconTint, ResolvedIcon,
};

pub mod font {
    pub use aimer_cupid::font::*;
}

use std::fmt::Debug;
use std::sync::Arc;

use aimer_cupid::draw_cmd_v2::ImageResource;
pub use aimer_svg::SvgAsset;
use aimer_widget::base::BuildContext;
pub use font::{
    FontError, FontFamily, FontRegistration, FontRegistry, FontStyle, FontWeight,
    bundled_monospace_bytes,
};
pub use img_widget::asset_image::AssetImage;
pub use img_widget::image_widget::Image;
pub use img_widget::network_image::NetworkImage;
pub use img_widget::source::ImageSource;

#[derive(Debug, Clone, PartialEq)]
pub enum ImageResult {
    Loading,
    Success(u32),
    Error(String),
}

pub type LoadingResult = Result<u32, &'static str>;

///
/// A trait for providing images based on a given context.
///
/// The `ImageProvider` trait defines a common interface for structures
/// that can return an image representation, identified by a `u32`,
/// based on a provided context (`BuildContext`). The trait requires
/// implementors to be `Clone`.
///
/// # Required Methods
///
/// - `get_image(&self, ctx: &BuildContext) -> LoadingResult`
///
/// This method takes a reference to a `BuildContext` and returns a
/// `LoadingResult` (which is `Result<u32, &'static str>`).
/// The implementation of this method can define how the image is determined
/// from the given context.
///
/// # Example
/// To implement the `ImageProvider` trait:
///
/// ```rust
/// use aimer_assets::ImageResult;
/// use aimer_widget::components::context::BuildContext;
///
/// use self::aimer_assets::ImageProvider;
/// #[derive(Clone, Debug)]
/// struct MyImageProvider;
///
/// impl ImageProvider for MyImageProvider {
///     fn get_image(&self, ctx: &BuildContext) -> ImageResult {
///         // Custom logic to provide an image ID based on the context
///         ImageResult::Success(1) // Example image ID
///     }
/// }
/// ```
///
/// # Notes
/// - The `ImageProvider` trait requires types to implement the `Clone` trait.
/// - The `BuildContext` parameter represents the context used to determine the
///   image and is expected to be provided by the caller.
pub trait ImageProvider: Clone + Debug {
    fn get_image(&self, ctx: &BuildContext) -> ImageResult;

    /// Returns a retained image payload when the provider owns its decoded pixels.
    ///
    /// Providers that do not own pixel data can keep returning `None`; their
    /// image remains eligible for v2 drawing only while the current renderer
    /// still has the texture ID returned by `get_image`.
    #[doc(hidden)]
    fn retained_image_resource(&self, _ctx: &BuildContext) -> Option<Arc<ImageResource>> {
        None
    }
}

#[cfg(test)]
mod public_api_tests {
    use aimer_widget::{AnyElement, AnyWidget, Widget};

    use super::img_widget::image_widget::RawImageWidget;
    use super::{AssetImage, FontFamily, Image, ImageProvider, NetworkImage, SvgAsset};

    #[test]
    fn exposes_font_and_svg_assets_from_one_crate() {
        assert_ne!(FontFamily::SANS_SERIF, FontFamily::MONOSPACE);
        assert_eq!(SvgAsset::new("assets/icon.svg").debug_name(), "SvgAsset");
    }

    #[test]
    fn image_widgets_use_rubick_erased_ownership() {
        fn assert_widget<W: Widget>() {}
        fn assert_asset_fallbacks(image: &AssetImage) {
            let _: &Option<AnyWidget> = &image.error_widget;
            let _: &Option<AnyWidget> = &image.loading_widget;
        }
        fn assert_network_fallbacks(image: &NetworkImage) {
            let _: &Option<AnyWidget> = &image.error_widget;
            let _: &Option<AnyWidget> = &image.loading_widget;
        }
        fn assert_element_fallbacks<P: ImageProvider>(element: &RawImageWidget<P>) {
            let _: &Option<AnyElement> = &element.error_element;
            let _: &Option<AnyElement> = &element.loading_element;
        }

        assert_widget::<Image>();
        assert_widget::<AssetImage>();
        assert_widget::<NetworkImage>();
        assert_asset_fallbacks(&AssetImage::new("asset.png"));
        assert_network_fallbacks(&NetworkImage::new("https://example.com/image.png"));
        let _ = assert_element_fallbacks::<super::ImageSource>;
    }
}

#[cfg(test)]
mod w13_tests;
