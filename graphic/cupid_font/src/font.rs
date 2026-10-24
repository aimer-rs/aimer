//! Deterministic font-family registration shared by Aimer styles and Cupid.
//!
//! Register immutable font bytes before `AimerApp::start`. Generic sans-serif
//! and monospace handles are available without an application-owned font when
//! the host has readable system faces; enable the `bundled-fonts` feature when
//! deterministic checked-in faces are preferred. Resolution prefers the
//! requested style, then the nearest numeric weight; normal style is the
//! deterministic fallback when the requested style is not registered. Cupid
//! retains the selected family where it has glyphs and uses its existing
//! Unicode fallback chain only for missing glyphs.
//!
//! Browser wasm targets use Cupid's checked-in generic faces because the
//! browser does not expose a readable system font file to the renderer.
//!
//! On Apple without `apple-core-text`, the portable path intentionally relies
//! on application-registered faces or the opt-in `bundled-fonts` feature rather
//! than system fallback. Apple system files can contain private `hvgl` outline
//! or `emjc` color data that the owned reader does not interpret; registering
//! such a file does not make it portable. Applications targeting that profile
//! should bundle a licensed, readable replacement for every required script,
//! weight, style, and color format.

use std::collections::HashMap;
use std::fmt::{Display, Formatter};
use std::sync::{Arc, OnceLock, RwLock};

#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, Eq)]
pub enum FontStyle {
    #[default]
    Normal,
    Italic,
    Oblique,
    ObliqueDeg(i32),
}

#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, Eq)]
pub enum FontWeight {
    VeryThin,
    Thin,
    #[default]
    Normal,
    Bold,
    Bolder,
    Value(u32),
}

impl FontWeight {
    /// Numeric CSS-style weight (100–900). 400 is normal, 700 is bold.
    pub fn numeric(self) -> u16 {
        match self {
            Self::VeryThin => 100,
            Self::Thin => 300,
            Self::Normal => 400,
            Self::Bold => 700,
            Self::Bolder => 900,
            Self::Value(value) => value.clamp(1, 1000) as u16,
        }
    }
}

/// The written language a run of text belongs to.
///
/// Han is unified: a Chinese and a Japanese face each carry the ideographs
/// their own language writes, and the two sets overlap almost entirely. A run
/// of ideographs alone therefore does not say which face it wants — `你好` is
/// covered by a Japanese face as readily as by a Chinese one, while `你好吗`
/// is not, because `吗` is written only in Chinese. Left to the characters,
/// one word changes typeface when the next one is typed.
///
/// This is what the run cannot tell, supplied by whoever knows it: on iOS the
/// language of the keyboard the text was typed on, elsewhere whatever the
/// caller can say. It is a hint and never overrules the text itself — a run
/// carrying kana is Japanese however it was typed.
///
/// # Examples
///
/// ```
/// use aimer_cupid::font::TextLanguage;
///
/// // A field bound to a Chinese keyboard keeps its Chinese face even when
/// // every character it holds is also written in Japanese.
/// let typed_language = Some(TextLanguage::Chinese);
/// assert_eq!(typed_language, Some(TextLanguage::Chinese));
/// ```
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum TextLanguage {
    /// Chinese, in either script.
    Chinese,
    /// Japanese.
    Japanese,
    /// Korean.
    Korean,
}

impl TextLanguage {
    /// The language an IETF tag names, when it is one of the Han-sharing three.
    ///
    /// Only the primary subtag is read, so `zh-Hans-CN`, `zh_TW` and `zh` all
    /// answer [`Self::Chinese`]; every other language answers `None`, since a
    /// language writing no ideographs has no face to disambiguate.
    ///
    /// # Examples
    ///
    /// ```
    /// use aimer_cupid::font::TextLanguage;
    ///
    /// assert_eq!(TextLanguage::from_tag("zh-Hans"), Some(TextLanguage::Chinese));
    /// assert_eq!(TextLanguage::from_tag("ja-JP"), Some(TextLanguage::Japanese));
    /// assert_eq!(TextLanguage::from_tag("en-US"), None);
    /// ```
    pub fn from_tag(tag: &str) -> Option<Self> {
        let primary = tag.split(['-', '_']).next().unwrap_or_default();
        match primary.to_ascii_lowercase().as_str() {
            "zh" | "yue" | "nan" | "hak" | "wuu" => Some(Self::Chinese),
            "ja" => Some(Self::Japanese),
            "ko" => Some(Self::Korean),
            _ => None,
        }
    }
}

/// A lightweight, process-stable handle to a generic or registered font family.
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub struct FontFamily(u64);

impl FontFamily {
    /// Aimer's generic sans-serif family.
    pub const SANS_SERIF: Self = Self(0);
    /// Aimer's generic monospace family.
    pub const MONOSPACE: Self = Self(1);

    #[doc(hidden)]
    pub const fn raw(self) -> u64 {
        self.0
    }

    /// Reconstructs a family handle received from a portable style codec.
    #[doc(hidden)]
    pub const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }
}

impl Default for FontFamily {
    fn default() -> Self {
        Self::SANS_SERIF
    }
}

/// Returns the optional built-in monospace face.
///
/// This returns the checked-in face only when the `bundled-fonts` feature is
/// enabled. Without that feature it returns an empty slice on every target;
/// the wasm renderer keeps a separate internal copy because browsers do not
/// expose readable system font files.
#[doc(hidden)]
pub const fn bundled_monospace_bytes() -> &'static [u8] {
    #[cfg(feature = "bundled-fonts")]
    {
        include_bytes!("../fonts/JetBrainsMono-Regular.ttf")
    }

    #[cfg(not(feature = "bundled-fonts"))]
    {
        &[]
    }
}

/// Immutable application-owned font bytes and the face metadata used to match
/// them during shaping.
///
/// Provide TTF/OTF/TTC bytes from an application asset or from a checked-in
/// bundled font. The bytes must expose
/// the tables required by the active Aimer reader; Apple-private-only faces
/// such as `hvgl`/`emjc` remain unsupported when `apple-core-text` is disabled.
/// Register one variant for each weight and style the UI needs so fallback does
/// not synthesize a mismatched stroke from an unrelated system face.
///
/// Registration copies the bytes into immutable shared storage. It is intended
/// to happen before application startup; duplicate family/weight/style
/// variants are rejected.
#[derive(Clone, Copy)]
pub struct FontRegistration<'a> {
    pub family: &'a str,
    pub bytes: &'a [u8],
    pub weight: FontWeight,
    pub style: FontStyle,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FontError {
    EmptyFamily,
    InvalidFont,
    ReservedFamily,
    DuplicateVariant {
        family: FontFamily,
        weight: u16,
        style: FontStyle,
    },
    VariantNotFound {
        family: FontFamily,
        weight: u16,
        style: FontStyle,
    },
    HandleCollision,
}

impl Display for FontError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyFamily => formatter.write_str("font family name cannot be empty"),
            Self::InvalidFont => {
                formatter.write_str("font bytes are empty, invalid, or unsupported")
            }
            Self::ReservedFamily => {
                formatter.write_str("generic Aimer font family names are reserved")
            }
            Self::DuplicateVariant { weight, style, .. } => {
                write!(
                    formatter,
                    "font variant {weight}/{style:?} is already registered"
                )
            }
            Self::VariantNotFound { weight, style, .. } => {
                write!(
                    formatter,
                    "font variant {weight}/{style:?} is not registered"
                )
            }
            Self::HandleCollision => formatter.write_str("font family or face handle collision"),
        }
    }
}

impl std::error::Error for FontError {}

#[derive(Clone)]
#[doc(hidden)]
pub struct RegisteredFontFace {
    pub family: FontFamily,
    pub face_id: u32,
    pub bytes: Arc<[u8]>,
    pub weight: u16,
    pub style: FontStyle,
}

#[derive(Default)]
struct RegistryState {
    names: HashMap<String, FontFamily>,
    family_names: HashMap<FontFamily, String>,
    faces: HashMap<FontFamily, Vec<RegisteredFontFace>>,
    face_owners: HashMap<u32, (FontFamily, u16, FontStyle)>,
    revision: u64,
}

fn registry() -> &'static RwLock<RegistryState> {
    static REGISTRY: OnceLock<RwLock<RegistryState>> = OnceLock::new();
    REGISTRY.get_or_init(|| RwLock::new(RegistryState::default()))
}

fn normalize_family(name: &str) -> Result<String, FontError> {
    let normalized = name.trim().to_lowercase();
    if normalized.is_empty() {
        Err(FontError::EmptyFamily)
    } else if matches!(normalized.as_str(), "sans-serif" | "monospace") {
        Err(FontError::ReservedFamily)
    } else {
        Ok(normalized)
    }
}

fn stable_hash(bytes: &[u8]) -> u64 {
    const OFFSET: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x100000001b3;
    bytes.iter().fold(OFFSET, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(PRIME)
    })
}

fn family_handle(name: &str) -> FontFamily {
    FontFamily(stable_hash(name.as_bytes()) | (1 << 63))
}

fn face_id(family: FontFamily, weight: u16, style: FontStyle) -> u32 {
    let key = format!("{}:{weight}:{style:?}", family.raw());
    0x8000_0000 | (stable_hash(key.as_bytes()) as u32 & 0x7fff_ffff)
}

fn style_distance(requested: FontStyle, candidate: FontStyle) -> u8 {
    if requested == candidate {
        0
    } else if candidate == FontStyle::Normal {
        1
    } else if matches!(requested, FontStyle::Oblique | FontStyle::ObliqueDeg(_))
        && matches!(candidate, FontStyle::Oblique | FontStyle::ObliqueDeg(_))
    {
        2
    } else {
        3
    }
}

fn font_bytes_are_valid(bytes: &[u8]) -> bool {
    crate::pipeline::text_pipeline::aimer_font::validate_font(bytes).is_ok()
}

pub struct FontRegistry;

impl FontRegistry {
    /// Validates and registers one immutable family variant.
    ///
    /// Registration is intended to finish before `AimerApp::start`. Registering
    /// the same normalized family, numeric weight, and style twice is rejected.
    pub fn register(registration: FontRegistration<'_>) -> Result<FontFamily, FontError> {
        let family_name = normalize_family(registration.family)?;
        if !font_bytes_are_valid(registration.bytes) {
            return Err(FontError::InvalidFont);
        }

        let family = family_handle(&family_name);
        let weight = registration.weight.numeric();
        let id = face_id(family, weight, registration.style);
        let mut state = registry()
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        if let Some(existing_name) = state.family_names.get(&family)
            && existing_name != &family_name
        {
            return Err(FontError::HandleCollision);
        }
        if let Some(owner) = state.face_owners.get(&id)
            && *owner != (family, weight, registration.style)
        {
            return Err(FontError::HandleCollision);
        }
        if state.faces.get(&family).is_some_and(|faces| {
            faces
                .iter()
                .any(|face| face.weight == weight && face.style == registration.style)
        }) {
            return Err(FontError::DuplicateVariant {
                family,
                weight,
                style: registration.style,
            });
        }

        state.names.insert(family_name.clone(), family);
        state.family_names.insert(family, family_name);
        state
            .face_owners
            .insert(id, (family, weight, registration.style));
        state
            .faces
            .entry(family)
            .or_default()
            .push(RegisteredFontFace {
                family,
                face_id: id,
                bytes: Arc::from(registration.bytes),
                weight,
                style: registration.style,
            });
        state.revision = state.revision.wrapping_add(1);
        Ok(family)
    }

    /// Replaces the bytes of one already-registered family variant.
    ///
    /// The family, numeric weight, style, and face id remain unchanged. A
    /// rasterizer notices the registry revision and invalidates all data
    /// derived from that id before it reads the replacement. This makes a
    /// replacement safe for glyph keys that crossed a worker boundary while
    /// preserving their stable identity.
    pub fn replace(registration: FontRegistration<'_>) -> Result<FontFamily, FontError> {
        let family_name = normalize_family(registration.family)?;
        if !font_bytes_are_valid(registration.bytes) {
            return Err(FontError::InvalidFont);
        }

        let family = family_handle(&family_name);
        let weight = registration.weight.numeric();
        let id = face_id(family, weight, registration.style);
        let mut state = registry()
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        if let Some(existing_name) = state.family_names.get(&family)
            && existing_name != &family_name
        {
            return Err(FontError::HandleCollision);
        }
        if let Some(owner) = state.face_owners.get(&id)
            && *owner != (family, weight, registration.style)
        {
            return Err(FontError::HandleCollision);
        }

        let replaced = state
            .faces
            .get_mut(&family)
            .and_then(|faces| {
                faces
                    .iter_mut()
                    .find(|face| face.weight == weight && face.style == registration.style)
            })
            .map(|face| {
                debug_assert_eq!(face.face_id, id);
                face.bytes = Arc::from(registration.bytes);
            });
        if replaced.is_none() {
            return Err(FontError::VariantNotFound {
                family,
                weight,
                style: registration.style,
            });
        }
        state.revision = state.revision.wrapping_add(1);
        Ok(family)
    }

    /// Removes one registered family variant.
    ///
    /// The deterministic face id is no longer resolvable after removal. The
    /// registry revision still advances when the variant existed, allowing
    /// every live rasterizer to discard cached metrics, shaping data, and
    /// bitmaps for the removed id. Removing the last variant also removes the
    /// family name; registering it again later recreates the same family and
    /// face handles.
    pub fn remove(family: FontFamily, weight: FontWeight, style: FontStyle) -> bool {
        let weight = weight.numeric();
        let id = face_id(family, weight, style);
        let mut state = registry()
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let remove_family = {
            let Some(faces) = state.faces.get_mut(&family) else {
                return false;
            };
            let Some(index) = faces
                .iter()
                .position(|face| face.weight == weight && face.style == style)
            else {
                return false;
            };

            faces.remove(index);
            faces.is_empty()
        };
        state.face_owners.remove(&id);
        if remove_family {
            state.faces.remove(&family);
            if let Some(name) = state.family_names.remove(&family) {
                state.names.remove(&name);
            }
        }
        state.revision = state.revision.wrapping_add(1);
        true
    }

    pub fn family(name: &str) -> Option<FontFamily> {
        let normalized = name.trim().to_lowercase();
        match normalized.as_str() {
            "sans-serif" => Some(FontFamily::SANS_SERIF),
            "monospace" => Some(FontFamily::MONOSPACE),
            _ => registry()
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .names
                .get(&normalized)
                .copied(),
        }
    }

    #[doc(hidden)]
    pub fn resolve(
        family: FontFamily,
        weight: FontWeight,
        style: FontStyle,
    ) -> Option<RegisteredFontFace> {
        let numeric_weight = weight.numeric();
        registry()
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .faces
            .get(&family)?
            .iter()
            .min_by_key(|face| {
                (
                    style_distance(style, face.style),
                    face.weight.abs_diff(numeric_weight),
                    face.weight,
                    face.face_id,
                )
            })
            .cloned()
    }

    #[doc(hidden)]
    pub fn faces() -> Vec<RegisteredFontFace> {
        Self::faces_with_revision().0
    }

    /// Returns the current registration revision.
    ///
    /// Paint and shaping caches include this value because replacing or
    /// removing a registered face changes the glyphs produced for the same
    /// text/style key.
    #[inline]
    pub fn revision() -> u64 {
        registry()
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .revision
    }

    /// Returns a deterministic face snapshot together with its revision.
    pub(crate) fn faces_with_revision() -> (Vec<RegisteredFontFace>, u64) {
        let state = registry()
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut faces: Vec<_> = state.faces.values().flatten().cloned().collect();
        faces.sort_by_key(|face| face.face_id);
        (faces, state.revision)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(feature = "bundled-fonts"))]
    #[test]
    fn bundled_monospace_is_opt_in() {
        assert!(bundled_monospace_bytes().is_empty());
    }

    const TEST_FONT: &[u8] = include_bytes!("../fonts/JetBrainsMono-Regular.ttf");
    const REPLACEMENT_FONT: &[u8] = include_bytes!("../fonts/GoogleSans-Regular.ttf");

    #[test]
    fn checked_in_portable_replacement_fonts_have_owned_outline_data() {
        let assets: [(&str, &[u8]); 3] = [
            (
                "GoogleSans-Regular",
                include_bytes!("../fonts/GoogleSans-Regular.ttf"),
            ),
            (
                "JetBrainsMono-Regular",
                include_bytes!("../fonts/JetBrainsMono-Regular.ttf"),
            ),
            (
                "NotoSansJP-VariableFont_wght",
                include_bytes!("../fonts/NotoSansJP-VariableFont_wght.ttf"),
            ),
        ];

        for (name, bytes) in assets {
            assert!(
                font_bytes_are_valid(bytes),
                "bundled {name} face is not valid"
            );
            let face = crate::pipeline::text_pipeline::aimer_font::SfntFace::from_bytes(bytes, 0)
                .unwrap_or_else(|error| panic!("bundled {name} face is not readable: {error:?}"));
            assert!(
                face.has_standard_outline(),
                "bundled {name} face has no Aimer-readable outline"
            );
        }
    }

    #[test]
    fn nearest_variant_prefers_style_then_weight_deterministically() {
        let family = FontRegistry::register(FontRegistration {
            family: "owned-font-nearest-test",
            bytes: TEST_FONT,
            weight: FontWeight::Normal,
            style: FontStyle::Normal,
        })
        .unwrap();
        FontRegistry::register(FontRegistration {
            family: "owned-font-nearest-test",
            bytes: TEST_FONT,
            weight: FontWeight::Bold,
            style: FontStyle::Normal,
        })
        .unwrap();
        FontRegistry::register(FontRegistration {
            family: "owned-font-nearest-test",
            bytes: TEST_FONT,
            weight: FontWeight::Normal,
            style: FontStyle::Italic,
        })
        .unwrap();

        let exact = FontRegistry::resolve(family, FontWeight::Bold, FontStyle::Normal).unwrap();
        assert_eq!((exact.weight, exact.style), (700, FontStyle::Normal));

        let nearest_weight =
            FontRegistry::resolve(family, FontWeight::Value(600), FontStyle::Normal).unwrap();
        assert_eq!(nearest_weight.weight, 700);

        let exact_style =
            FontRegistry::resolve(family, FontWeight::Bold, FontStyle::Italic).unwrap();
        assert_eq!(
            (exact_style.weight, exact_style.style),
            (400, FontStyle::Italic)
        );

        let normal_style_fallback =
            FontRegistry::resolve(family, FontWeight::Bold, FontStyle::Oblique).unwrap();
        assert_eq!(
            (normal_style_fallback.weight, normal_style_fallback.style),
            (700, FontStyle::Normal)
        );
    }

    #[test]
    fn replacing_a_variant_preserves_its_face_id() {
        let family = FontRegistry::register(FontRegistration {
            family: "owned-font-replace-test",
            bytes: TEST_FONT,
            weight: FontWeight::Normal,
            style: FontStyle::Normal,
        })
        .expect("the original face should register");
        let original = FontRegistry::resolve(family, FontWeight::Normal, FontStyle::Normal)
            .expect("the original face should resolve");

        assert_eq!(
            FontRegistry::replace(FontRegistration {
                family: "owned-font-replace-test",
                bytes: REPLACEMENT_FONT,
                weight: FontWeight::Normal,
                style: FontStyle::Normal,
            }),
            Ok(family)
        );

        let replacement = FontRegistry::resolve(family, FontWeight::Normal, FontStyle::Normal)
            .expect("the replacement face should resolve");
        assert_eq!(replacement.face_id, original.face_id);
        assert_ne!(replacement.bytes.as_ref(), original.bytes.as_ref());

        assert!(FontRegistry::remove(
            family,
            FontWeight::Normal,
            FontStyle::Normal
        ));
    }

    #[test]
    fn removing_the_last_variant_removes_the_family() {
        let family = FontRegistry::register(FontRegistration {
            family: "owned-font-remove-test",
            bytes: TEST_FONT,
            weight: FontWeight::Normal,
            style: FontStyle::Normal,
        })
        .expect("the face should register");

        assert!(FontRegistry::remove(
            family,
            FontWeight::Normal,
            FontStyle::Normal
        ));
        assert!(FontRegistry::resolve(family, FontWeight::Normal, FontStyle::Normal).is_none());
        assert!(FontRegistry::family("owned-font-remove-test").is_none());
        assert!(!FontRegistry::remove(
            family,
            FontWeight::Normal,
            FontStyle::Normal
        ));
    }

    #[test]
    fn replacing_an_unknown_variant_is_rejected_without_mutating_the_family() {
        let family = FontRegistry::register(FontRegistration {
            family: "owned-font-missing-replacement-test",
            bytes: TEST_FONT,
            weight: FontWeight::Normal,
            style: FontStyle::Normal,
        })
        .expect("the face should register");

        let error = FontRegistry::replace(FontRegistration {
            family: "owned-font-missing-replacement-test",
            bytes: REPLACEMENT_FONT,
            weight: FontWeight::Bold,
            style: FontStyle::Normal,
        })
        .expect_err("a replacement must name an existing variant");
        assert_eq!(
            error,
            FontError::VariantNotFound {
                family,
                weight: FontWeight::Bold.numeric(),
                style: FontStyle::Normal,
            }
        );
        assert_eq!(
            FontRegistry::resolve(family, FontWeight::Normal, FontStyle::Normal)
                .expect("the original variant should remain")
                .bytes,
            Arc::from(TEST_FONT)
        );

        assert!(FontRegistry::remove(
            family,
            FontWeight::Normal,
            FontStyle::Normal
        ));
    }

    #[test]
    fn aimer_font_registration_requires_scaling_metrics() {
        let mut directory_only = vec![0_u8; 12];
        directory_only[0..4].copy_from_slice(&0x0001_0000_u32.to_be_bytes());

        let error = FontRegistry::register(FontRegistration {
            family: "owned-font-missing-metrics-test",
            bytes: &directory_only,
            weight: FontWeight::Normal,
            style: FontStyle::Normal,
        })
        .expect_err("a face without head/hhea/maxp metrics must be rejected");

        assert_eq!(error, FontError::InvalidFont);
    }
}
