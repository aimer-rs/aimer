//! Typed handles for the files registered under `[assets]` and `[fonts]` in
//! `Aimer.toml`, normally produced by the `aimer::assets!` macro.

use std::fmt::{self, Display, Formatter};
use std::sync::{Arc, Mutex};

use crate::font::{FontError, FontFamily, FontRegistration, FontRegistry, FontStyle, FontWeight};
use crate::img_widget::source::ImageSource;

/// What a bundled file is, judged from its extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AssetKind {
    /// A raster image: png, jpg, jpeg, gif, webp, bmp or ico.
    Image,
    /// A vector image: svg.
    Svg,
    /// A font file: ttf, otf, ttc, woff or woff2.
    Font,
    /// Anything else.
    Other,
}

/// A file registered under `[assets]` in `Aimer.toml`.
///
/// The `path` is the manifest path, relative to the project root, and is the
/// per-platform lookup key used by [`AssetImage`](crate::AssetImage). Values are
/// created by the `aimer::assets!` macro, which checks at compile time that the
/// file exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BundledAsset {
    name: &'static str,
    path: &'static str,
}

impl BundledAsset {
    /// Creates a handle for the asset `name` stored at `path`.
    #[inline]
    pub const fn new(name: &'static str, path: &'static str) -> Self {
        Self { name, path }
    }

    /// The key the asset has in `Aimer.toml`.
    #[inline]
    pub const fn name(&self) -> &'static str {
        self.name
    }

    /// The project-relative path, used verbatim as the asset lookup key.
    #[inline]
    pub const fn path(&self) -> &'static str {
        self.path
    }

    /// The kind of file this is, from its extension and ignoring ASCII case.
    pub const fn kind(&self) -> AssetKind {
        let bytes = self.path.as_bytes();
        let mut dot = bytes.len();
        while dot > 0 {
            dot -= 1;
            if bytes[dot] == b'.' {
                return kind_of_extension(bytes, dot + 1);
            }
            if bytes[dot] == b'/' || bytes[dot] == b'\\' {
                break;
            }
        }
        AssetKind::Other
    }
}

const fn kind_of_extension(path: &[u8], start: usize) -> AssetKind {
    const IMAGE: &[&[u8]] = &[b"png", b"jpg", b"jpeg", b"gif", b"webp", b"bmp", b"ico"];
    const FONT: &[&[u8]] = &[b"ttf", b"otf", b"ttc", b"woff", b"woff2"];

    if extension_is(path, start, &[b"svg"]) {
        AssetKind::Svg
    } else if extension_is(path, start, IMAGE) {
        AssetKind::Image
    } else if extension_is(path, start, FONT) {
        AssetKind::Font
    } else {
        AssetKind::Other
    }
}

const fn extension_is(path: &[u8], start: usize, candidates: &[&[u8]]) -> bool {
    let mut candidate = 0;
    while candidate < candidates.len() {
        let expected = candidates[candidate];
        if path.len() - start == expected.len() {
            let mut index = 0;
            while index < expected.len() && path[start + index].to_ascii_lowercase() == expected[index]
            {
                index += 1;
            }
            if index == expected.len() {
                return true;
            }
        }
        candidate += 1;
    }
    false
}

impl From<BundledAsset> for String {
    #[inline]
    fn from(asset: BundledAsset) -> Self {
        asset.path.to_owned()
    }
}

/// The form `SvgAsset::new` takes its key in.
impl From<BundledAsset> for Arc<str> {
    #[inline]
    fn from(asset: BundledAsset) -> Self {
        Arc::from(asset.path)
    }
}

impl From<BundledAsset> for ImageSource {
    #[inline]
    fn from(asset: BundledAsset) -> Self {
        ImageSource::Asset(asset.path.to_owned())
    }
}

/// Why a bundled font could not be loaded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BundledFontError {
    /// The file could not be read from the platform's asset store.
    Read(String),
    /// The bytes were read but the font registry rejected them.
    Font(FontError),
}

impl Display for BundledFontError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read(message) => formatter.write_str(message),
            Self::Font(error) => Display::fmt(error, formatter),
        }
    }
}

impl std::error::Error for BundledFontError {}

impl From<FontError> for BundledFontError {
    fn from(error: FontError) -> Self {
        Self::Font(error)
    }
}

/// Paths already registered by [`BundledFont`], so registering twice is free.
static REGISTERED: Mutex<Vec<(&'static str, FontWeight, FontStyle, FontFamily)>> =
    Mutex::new(Vec::new());

/// Variants whose load already failed, so the lazy conversion into a
/// [`FontFamily`] reports the failure once instead of re-reading on every build.
#[cfg(not(target_arch = "wasm32"))]
static FAILED: Mutex<Vec<(&'static str, FontWeight, FontStyle)>> = Mutex::new(Vec::new());

/// A font file registered under `[fonts]` in `Aimer.toml`.
///
/// The font is read from the bundled assets, not embedded in the binary. On
/// native targets it loads lazily: converting it into a [`FontFamily`] (for
/// example `TextStyle::new().font_family(Assets::INTER.into())`) reads and
/// registers it the first time and is free afterwards. If the load fails the
/// conversion reports the error on stderr once and yields
/// [`FontFamily::SANS_SERIF`]; call [`register`](Self::register) instead when
/// you need the error. The web cannot read synchronously, so there the font has
/// to be loaded with [`load`](Self::load) before it is used. By default the family is named after the manifest key and
/// the face is registered as regular weight and normal style; chain
/// [`family`](Self::family), [`weight`](Self::weight) and [`style`](Self::style)
/// to register several files as variants of one family.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BundledFont {
    name: &'static str,
    path: &'static str,
    family: &'static str,
    weight: FontWeight,
    style: FontStyle,
}

impl BundledFont {
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
    pub fn register(&self) -> Result<FontFamily, BundledFontError> {
        if let Some(family) = self.registered() {
            return Ok(family);
        }
        let bytes = ImageSource::load_asset_bytes(self.path).map_err(BundledFontError::Read)?;
        self.register_bytes(&bytes)
    }

    /// Like [`register`](Self::register), but reads without blocking on the web.
    ///
    /// On native targets the read still happens synchronously inside the first
    /// poll.
    pub async fn load(&self) -> Result<FontFamily, BundledFontError> {
        if let Some(family) = self.registered() {
            return Ok(family);
        }
        #[cfg(not(target_arch = "wasm32"))]
        let bytes = ImageSource::load_asset_bytes(self.path).map_err(BundledFontError::Read)?;
        #[cfg(target_arch = "wasm32")]
        let bytes = {
            let url = if self.path.starts_with('/') {
                self.path.to_owned()
            } else {
                format!("/{}", self.path)
            };
            ImageSource::fetch_bytes(&url)
                .await
                .map_err(BundledFontError::Read)?
        };
        self.register_bytes(&bytes)
    }

    fn register_bytes(&self, bytes: &[u8]) -> Result<FontFamily, BundledFontError> {
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

/// Loads the font on first use; see [`BundledFont`] for the failure behavior.
#[cfg(not(target_arch = "wasm32"))]
impl From<BundledFont> for FontFamily {
    fn from(font: BundledFont) -> Self {
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
    fn exposes_name_and_path() {
        const ASSET: BundledAsset = BundledAsset::new("logo", "assets/logo.png");
        assert_eq!(ASSET.name(), "logo");
        assert_eq!(ASSET.path(), "assets/logo.png");
    }

    #[test]
    fn kind_comes_from_the_extension_ignoring_case() {
        const fn kind(path: &'static str) -> AssetKind {
            BundledAsset::new("x", path).kind()
        }
        const PNG: AssetKind = kind("assets/a.png");
        assert_eq!(PNG, AssetKind::Image);
        assert_eq!(kind("assets/PHOTO.JpEg"), AssetKind::Image);
        assert_eq!(kind("assets/cat.svg"), AssetKind::Svg);
        assert_eq!(kind("assets/fonts/Inter.ttf"), AssetKind::Font);
        assert_eq!(kind("assets/fonts/Inter.WOFF2"), AssetKind::Font);
        assert_eq!(kind("assets/data.json"), AssetKind::Other);
        assert_eq!(kind("assets/noextension"), AssetKind::Other);
        assert_eq!(kind("assets.png/readme"), AssetKind::Other);
        assert_eq!(kind("assets/trailingdot."), AssetKind::Other);
        assert_eq!(kind(""), AssetKind::Other);
    }

    #[test]
    fn an_asset_converts_into_a_key_and_an_image_source() {
        const ASSET: BundledAsset = BundledAsset::new("logo", "assets/logo.png");

        assert_eq!(String::from(ASSET), "assets/logo.png");
        assert_eq!(
            ImageSource::from(ASSET),
            ImageSource::Asset("assets/logo.png".to_owned())
        );
        assert_eq!(AssetImage::new(ASSET).key, "assets/logo.png");
        assert_eq!(&*Arc::<str>::from(ASSET), "assets/logo.png");
    }

    #[test]
    fn registers_a_font_from_its_bundled_path() {
        let path = temp_file("register.ttf", FONT_BYTES);
        let font = BundledFont::new("bundled_register_test", path);

        assert_eq!(font.registered(), None);
        let family = font.register().expect("the font should register");

        assert_eq!(FontRegistry::family("bundled_register_test"), Some(family));
        assert_eq!(font.registered(), Some(family));
    }

    #[test]
    fn registering_twice_returns_the_same_family() {
        let path = temp_file("twice.ttf", FONT_BYTES);
        let font = BundledFont::new("bundled_twice_test", path);

        assert_eq!(font.register().unwrap(), font.register().unwrap());
    }

    #[test]
    fn variants_of_one_family_share_it() {
        let regular = BundledFont::new("bundled_variant_regular", temp_file("v_regular.ttf", FONT_BYTES))
            .family("bundled_variant_test");
        let bold = BundledFont::new("bundled_variant_bold", temp_file("v_bold.ttf", FONT_BYTES))
            .family("bundled_variant_test")
            .weight(FontWeight::Bold);

        let regular_family = regular.register().unwrap();
        let bold_family = bold.register().unwrap();

        assert_eq!(regular_family, bold_family);
        assert_eq!(FontRegistry::family("bundled_variant_test"), Some(regular_family));
    }

    #[test]
    fn a_missing_file_is_a_read_error() {
        let font = BundledFont::new("bundled_missing_test", "definitely/not/here.ttf");

        assert!(matches!(font.register(), Err(BundledFontError::Read(_))));
        assert_eq!(font.registered(), None);
    }

    #[test]
    fn bytes_that_are_not_a_font_are_a_font_error() {
        let path = temp_file("invalid.ttf", b"not a font");
        let font = BundledFont::new("bundled_invalid_test", path);

        assert_eq!(
            font.register(),
            Err(BundledFontError::Font(FontError::InvalidFont))
        );
        assert_eq!(font.registered(), None);
    }

    #[test]
    fn a_reserved_family_name_is_rejected() {
        let path = temp_file("reserved.ttf", FONT_BYTES);
        let font = BundledFont::new("monospace", path);

        assert_eq!(
            font.register(),
            Err(BundledFontError::Font(FontError::ReservedFamily))
        );
    }

    #[test]
    fn load_registers_like_register() {
        let path = temp_file("load.ttf", FONT_BYTES);
        let font = BundledFont::new("bundled_load_test", path);

        // The native read completes inside the first poll, so a no-op waker is
        // enough to drive the future.
        let family = poll_ready(font.load()).expect("the font should load");
        assert_eq!(FontRegistry::family("bundled_load_test"), Some(family));
    }

    #[test]
    fn converting_into_a_family_loads_the_font_on_first_read() {
        let path = temp_file("lazy.ttf", FONT_BYTES);
        let font = BundledFont::new("bundled_lazy_test", path);
        assert_eq!(font.registered(), None);

        let family = FontFamily::from(font);

        assert_ne!(family, FontFamily::SANS_SERIF);
        assert_eq!(font.registered(), Some(family));
        assert_eq!(FontFamily::from(font), family);
    }

    #[test]
    fn a_font_that_fails_to_load_falls_back_to_sans_serif_and_is_not_retried() {
        let path = temp_file("lazy_bad.ttf", b"not a font");
        let font = BundledFont::new("bundled_lazy_bad_test", path);

        assert_eq!(FontFamily::from(font), FontFamily::SANS_SERIF);

        // Fixing the file does not matter: a failed font is not re-read on every
        // build, which would put disk I/O on the build path.
        std::fs::write(path, FONT_BYTES).unwrap();
        assert_eq!(FontFamily::from(font), FontFamily::SANS_SERIF);
        assert_eq!(font.registered(), None);
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
