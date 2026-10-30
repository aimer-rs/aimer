//! Typed handles for the files registered under `[assets]` and `[fonts]` in
//! `Aimer.toml`, normally produced by the `aimer::assets!` macro.

use std::fmt::{self, Display, Formatter};
use std::sync::{Arc, Mutex};

use crate::font::{FontError, FontFamily, FontRegistration, FontRegistry, FontStyle, FontWeight};
use crate::img_widget::source::ImageSource;

/// A file registered under `[assets]` or `[fonts]` in `Aimer.toml`.
///
/// The `aimer::assets` macro generates one value of a `Bundle*` type per entry,
/// chosen from the file's extension: [`BundleSvg`], [`BundleImage`],
/// [`BundleFont`], [`BundleDocuments`] or [`BundleOther`]. Each widget accepts
/// the kind it can show, so an SVG handed to an image widget does not compile.
/// This trait is what they share, for code that works with any bundled file.
pub trait BundleAsset: Copy + fmt::Debug + 'static {
    /// The key the file has in `Aimer.toml`.
    fn name(&self) -> &'static str;

    /// The project-relative path, used verbatim as the asset lookup key.
    fn path(&self) -> &'static str;
}

/// Declares a bundle type that is only a name and a path.
macro_rules! bundle_type {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub struct $name {
            name: &'static str,
            path: &'static str,
        }

        impl $name {
            /// Creates a handle for the file `name` stored at `path`.
            #[inline]
            pub const fn new(name: &'static str, path: &'static str) -> Self {
                Self { name, path }
            }

            /// The key the file has in `Aimer.toml`.
            #[inline]
            pub const fn name(&self) -> &'static str {
                self.name
            }

            /// The project-relative path, used verbatim as the asset lookup key.
            #[inline]
            pub const fn path(&self) -> &'static str {
                self.path
            }
        }

        impl BundleAsset for $name {
            #[inline]
            fn name(&self) -> &'static str {
                self.name
            }

            #[inline]
            fn path(&self) -> &'static str {
                self.path
            }
        }
    };
}

bundle_type! {
    /// An SVG registered under `[assets]`: `svg`.
    ///
    /// Accepted by `SvgAsset::new`.
    BundleSvg
}

bundle_type! {
    /// A raster image registered under `[assets]`: `png`, `jpg`, `jpeg`, `gif`,
    /// `webp`, `bmp` or `ico`.
    ///
    /// Accepted by `AssetImage::new`, and convertible into an [`ImageSource`].
    BundleImage
}

bundle_type! {
    /// A text or data document registered under `[assets]`: `md`, `markdown`,
    /// `txt`, `json`, `toml`, `yaml`, `yml`, `csv`, `xml`, `html`, `htm` or `pdf`.
    BundleDocuments
}

bundle_type! {
    /// Any other file registered under `[assets]`.
    BundleOther
}

impl From<BundleImage> for String {
    #[inline]
    fn from(image: BundleImage) -> Self {
        image.path.to_owned()
    }
}

impl From<BundleImage> for ImageSource {
    #[inline]
    fn from(image: BundleImage) -> Self {
        ImageSource::Asset(image.path.to_owned())
    }
}

/// The form `SvgAsset::new` takes its key in.
impl From<BundleSvg> for Arc<str> {
    #[inline]
    fn from(svg: BundleSvg) -> Self {
        Arc::from(svg.path)
    }
}

/// Why a bundled font could not be loaded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BundleFontError {
    /// The file could not be read from the platform's asset store.
    Read(String),
    /// The bytes were read but the font registry rejected them.
    Font(FontError),
}

impl Display for BundleFontError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read(message) => formatter.write_str(message),
            Self::Font(error) => Display::fmt(error, formatter),
        }
    }
}

impl std::error::Error for BundleFontError {}

impl From<FontError> for BundleFontError {
    fn from(error: FontError) -> Self {
        Self::Font(error)
    }
}

/// Paths already registered by [`BundleFont`], so registering twice is free.
static REGISTERED: Mutex<Vec<(&'static str, FontWeight, FontStyle, FontFamily)>> =
    Mutex::new(Vec::new());

/// Variants whose load already failed, so the lazy conversion into a
/// [`FontFamily`] reports the failure once instead of re-reading on every build.
#[cfg(not(target_arch = "wasm32"))]
static FAILED: Mutex<Vec<(&'static str, FontWeight, FontStyle)>> = Mutex::new(Vec::new());

/// A font registered under `[fonts]` in `Aimer.toml`, or under `[assets]` with a
/// font extension: `ttf`, `otf`, `ttc`, `woff` or `woff2`.
///
/// The font is read from the bundled assets, not embedded in the binary. On
/// native targets it loads lazily: converting it into a [`FontFamily`] (for
/// example `TextStyle::new().font_family(Assets::INTER.into())`) reads and
/// registers it the first time and is free afterwards. If the load fails the
/// conversion reports the error on stderr once and yields
/// [`FontFamily::SANS_SERIF`]; call [`register`](Self::register) instead when
/// you need the error. The web cannot read synchronously, so there the font has
/// to be loaded with [`load`](Self::load) before it is used; until then the
/// conversion yields the default sans-serif. By default the family is named
/// after the manifest key and the face is registered as regular weight and
/// normal style; chain
/// [`family`](Self::family), [`weight`](Self::weight) and [`style`](Self::style)
/// to register several files as variants of one family.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BundleFont {
    name: &'static str,
    path: &'static str,
    family: &'static str,
    weight: FontWeight,
    style: FontStyle,
}

impl BundleFont {
    /// Creates a handle for the font `name` stored at `path`.
    #[inline]
    pub const fn new(name: &'static str, path: &'static str) -> Self {
        Self {
            name,
            path,
            family: name,
            weight: FontWeight::Normal,
            style: FontStyle::Normal,
        }
    }

    /// The key the font has in `Aimer.toml`.
    #[inline]
    pub const fn name(&self) -> &'static str {
        self.name
    }

    /// The project-relative path, used verbatim as the asset lookup key.
    #[inline]
    pub const fn path(&self) -> &'static str {
        self.path
    }

    /// Registers the face under the family `name` instead of the manifest key.
    #[inline]
    pub const fn family(mut self, name: &'static str) -> Self {
        self.family = name;
        self
    }

    /// Sets the weight the face is registered with.
    #[inline]
    pub const fn weight(mut self, weight: FontWeight) -> Self {
        self.weight = weight;
        self
    }

    /// Sets the style the face is registered with.
    #[inline]
    pub const fn style(mut self, style: FontStyle) -> Self {
        self.style = style;
        self
    }

    /// The family this font is registered under, once it has been loaded.
    pub fn registered(&self) -> Option<FontFamily> {
        REGISTERED
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .find(|(path, weight, style, _)| {
                *path == self.path && *weight == self.weight && *style == self.style
            })
            .map(|entry| entry.3)
    }

    /// Reads the font from the bundled assets and registers it, blocking the
    /// calling thread for the read.
    ///
    /// Registering a font that is already registered returns the existing
    /// family. Not available on the web, where assets can only be fetched
    /// asynchronously; use [`load`](Self::load) there.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn register(&self) -> Result<FontFamily, BundleFontError> {
        if let Some(family) = self.registered() {
            return Ok(family);
        }
        let bytes = ImageSource::load_asset_bytes(self.path).map_err(BundleFontError::Read)?;
        self.register_bytes(&bytes)
    }

    /// Like [`register`](Self::register), but reads without blocking on the web.
    ///
    /// On native targets the read still happens synchronously inside the first
    /// poll.
    pub async fn load(&self) -> Result<FontFamily, BundleFontError> {
        if let Some(family) = self.registered() {
            return Ok(family);
        }
        #[cfg(not(target_arch = "wasm32"))]
        let bytes = ImageSource::load_asset_bytes(self.path).map_err(BundleFontError::Read)?;
        #[cfg(target_arch = "wasm32")]
        let bytes = {
            let url = if self.path.starts_with('/') {
                self.path.to_owned()
            } else {
                format!("/{}", self.path)
            };
            ImageSource::fetch_bytes(&url)
                .await
                .map_err(BundleFontError::Read)?
        };
        self.register_bytes(&bytes)
    }

    fn register_bytes(&self, bytes: &[u8]) -> Result<FontFamily, BundleFontError> {
        let family = match FontRegistry::register(FontRegistration {
            family: self.family,
            bytes,
            weight: self.weight,
            style: self.style,
        }) {
            Ok(family) => family,
            // Someone registered the same variant by hand; share it.
            Err(FontError::DuplicateVariant { family, .. }) => family,
            Err(error) => return Err(error.into()),
        };
        let mut registered = REGISTERED
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !registered
            .iter()
            .any(|(path, weight, style, _)| {
                *path == self.path && *weight == self.weight && *style == self.style
            })
        {
            registered.push((self.path, self.weight, self.style, family));
        }
        Ok(family)
    }
}

impl BundleAsset for BundleFont {
    #[inline]
    fn name(&self) -> &'static str {
        self.name
    }

    #[inline]
    fn path(&self) -> &'static str {
        self.path
    }
}

/// Loads the font on first use; see [`BundleFont`] for the failure behavior.
#[cfg(not(target_arch = "wasm32"))]
impl From<BundleFont> for FontFamily {
    fn from(font: BundleFont) -> Self {
        if let Some(family) = font.registered() {
            return family;
        }
        let variant = (font.path, font.weight, font.style);
        let mut failed = FAILED
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if failed.contains(&variant) {
            return FontFamily::SANS_SERIF;
        }
        // The lock is held across the read so concurrent first reads of one
        // font load it once; a font is read at most once per process.
        match font.register() {
            Ok(family) => family,
            Err(error) => {
                eprintln!(
                    "aimer: bundled font `{}` ({}) failed to load: {error}",
                    font.name, font.path
                );
                failed.push(variant);
                FontFamily::SANS_SERIF
            }
        }
    }
}

/// The web cannot read a file while a widget is being built, so a font that has
/// not been [loaded](BundleFont::load) yet is the default sans-serif. The
/// conversion exists so that code shared with native targets compiles.
#[cfg(target_arch = "wasm32")]
impl From<BundleFont> for FontFamily {
    fn from(font: BundleFont) -> Self {
        font.registered().unwrap_or(FontFamily::SANS_SERIF)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::AssetImage;

    const FONT_BYTES: &[u8] = include_bytes!("../../../graphic/cupid_font/fonts/JetBrainsMono-Regular.ttf");

    /// Writes `bytes` to a per-process temp file and returns its absolute path
    /// as the `'static` string the const-friendly handles require.
    fn temp_file(name: &str, bytes: &[u8]) -> &'static str {
        let path: PathBuf =
            std::env::temp_dir().join(format!("aimer_bundled_{}_{name}", std::process::id()));
        std::fs::write(&path, bytes).unwrap();
        Box::leak(path.to_string_lossy().into_owned().into_boxed_str())
    }

    #[test]
    fn every_bundle_type_exposes_its_name_and_path() {
        const IMAGE: BundleImage = BundleImage::new("logo", "assets/logo.png");
        assert_eq!(IMAGE.name(), "logo");
        assert_eq!(IMAGE.path(), "assets/logo.png");

        // The trait gives the same answers to code that is generic over the kind.
        fn describe(asset: impl BundleAsset) -> (&'static str, &'static str) {
            (asset.name(), asset.path())
        }
        assert_eq!(describe(IMAGE), ("logo", "assets/logo.png"));
        assert_eq!(describe(BundleSvg::new("icon", "a.svg")), ("icon", "a.svg"));
        assert_eq!(describe(BundleDocuments::new("doc", "a.md")), ("doc", "a.md"));
        assert_eq!(describe(BundleOther::new("blob", "a.bin")), ("blob", "a.bin"));
        assert_eq!(describe(BundleFont::new("mono", "a.ttf")), ("mono", "a.ttf"));
    }

    #[test]
    fn an_image_converts_into_a_key_and_an_image_source() {
        const IMAGE: BundleImage = BundleImage::new("logo", "assets/logo.png");

        assert_eq!(String::from(IMAGE), "assets/logo.png");
        assert_eq!(
            ImageSource::from(IMAGE),
            ImageSource::Asset("assets/logo.png".to_owned())
        );
        assert_eq!(AssetImage::new(IMAGE).key, "assets/logo.png");
    }

    #[test]
    fn an_svg_converts_into_the_key_svg_asset_takes() {
        const SVG: BundleSvg = BundleSvg::new("icon", "assets/icon.svg");

        assert_eq!(&*Arc::<str>::from(SVG), "assets/icon.svg");
        // Accepted by the widget, which is the point of the conversion.
        let _ = crate::SvgAsset::new(SVG);
    }

    #[test]
    fn registers_a_font_from_its_bundled_path() {
        let path = temp_file("register.ttf", FONT_BYTES);
        let font = BundleFont::new("bundled_register_test", path);

        assert_eq!(font.registered(), None);
        let family = font.register().expect("the font should register");

        assert_eq!(FontRegistry::family("bundled_register_test"), Some(family));
        assert_eq!(font.registered(), Some(family));
    }

    #[test]
    fn registering_twice_returns_the_same_family() {
        let path = temp_file("twice.ttf", FONT_BYTES);
        let font = BundleFont::new("bundled_twice_test", path);

        assert_eq!(font.register().unwrap(), font.register().unwrap());
    }

    #[test]
    fn variants_of_one_family_share_it() {
        let regular = BundleFont::new("bundled_variant_regular", temp_file("v_regular.ttf", FONT_BYTES))
            .family("bundled_variant_test");
        let bold = BundleFont::new("bundled_variant_bold", temp_file("v_bold.ttf", FONT_BYTES))
            .family("bundled_variant_test")
            .weight(FontWeight::Bold);

        let regular_family = regular.register().unwrap();
        let bold_family = bold.register().unwrap();

        assert_eq!(regular_family, bold_family);
        assert_eq!(FontRegistry::family("bundled_variant_test"), Some(regular_family));
    }

    #[test]
    fn a_missing_file_is_a_read_error() {
        let font = BundleFont::new("bundled_missing_test", "definitely/not/here.ttf");

        assert!(matches!(font.register(), Err(BundleFontError::Read(_))));
        assert_eq!(font.registered(), None);
    }

    #[test]
    fn bytes_that_are_not_a_font_are_a_font_error() {
        let path = temp_file("invalid.ttf", b"not a font");
        let font = BundleFont::new("bundled_invalid_test", path);

        assert_eq!(
            font.register(),
            Err(BundleFontError::Font(FontError::InvalidFont))
        );
        assert_eq!(font.registered(), None);
    }

    #[test]
    fn a_reserved_family_name_is_rejected() {
        let path = temp_file("reserved.ttf", FONT_BYTES);
        let font = BundleFont::new("monospace", path);

        assert_eq!(
            font.register(),
            Err(BundleFontError::Font(FontError::ReservedFamily))
        );
    }

    #[test]
    fn load_registers_like_register() {
        let path = temp_file("load.ttf", FONT_BYTES);
        let font = BundleFont::new("bundled_load_test", path);

        // The native read completes inside the first poll, so a no-op waker is
        // enough to drive the future.
        let family = poll_ready(font.load()).expect("the font should load");
        assert_eq!(FontRegistry::family("bundled_load_test"), Some(family));
    }

    #[test]
    fn converting_into_a_family_loads_the_font_on_first_read() {
        let path = temp_file("lazy.ttf", FONT_BYTES);
        let font = BundleFont::new("bundled_lazy_test", path);
        assert_eq!(font.registered(), None);

        let family = FontFamily::from(font);

        assert_ne!(family, FontFamily::SANS_SERIF);
        assert_eq!(font.registered(), Some(family));
        assert_eq!(FontFamily::from(font), family);
    }

    #[test]
    fn a_font_that_fails_to_load_falls_back_to_sans_serif_and_is_not_retried() {
        let path = temp_file("lazy_bad.ttf", b"not a font");
        let font = BundleFont::new("bundled_lazy_bad_test", path);

        assert_eq!(FontFamily::from(font), FontFamily::SANS_SERIF);

        // Fixing the file does not matter: a failed font is not re-read on every
        // build, which would put disk I/O on the build path.
        std::fs::write(path, FONT_BYTES).unwrap();
        assert_eq!(FontFamily::from(font), FontFamily::SANS_SERIF);
        assert_eq!(font.registered(), None);
    }

    #[test]
    fn a_text_style_accepts_a_bundle_font_and_loads_it() {
        use aimer_style::TextStyle;

        let path = temp_file("style.ttf", FONT_BYTES);
        let font = BundleFont::new("bundled_style_test", path);
        assert_eq!(font.registered(), None);

        let style = TextStyle::new().font_family(font);

        assert_ne!(style.font_family, FontFamily::SANS_SERIF);
        assert_eq!(font.registered(), Some(style.font_family));
        // A plain family still works.
        assert_eq!(
            TextStyle::new().font_family(FontFamily::MONOSPACE).font_family,
            FontFamily::MONOSPACE
        );
    }

    fn poll_ready<T>(future: impl std::future::Future<Output = T>) -> T {
        use std::pin::pin;
        use std::task::{Context, Poll, Waker};

        let mut future = pin!(future);
        match future.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
            Poll::Ready(value) => value,
            Poll::Pending => panic!("a native font load must not suspend"),
        }
    }
}
