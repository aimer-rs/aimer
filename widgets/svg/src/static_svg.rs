use std::sync::OnceLock;

use crate::SvgDocument;

/// An embedded SVG whose retained document is shared across widget instances.
///
/// Generated icon libraries can supply a precompiled factory. Other embedded
/// SVGs are parsed on first use. Neither path parses or constructs the scene
/// again after initialization; widget styles never modify the shared document.
///
/// ```
/// use aimer_svg::{StaticSvg, Svg};
/// static MARK: StaticSvg = StaticSvg::new("mark",
///     r#"<svg viewBox="0 0 24 24"><path d="M2 2h20v20z"/></svg>"#);
/// let widget = Svg::new(&MARK).width(48);
/// ```
pub struct StaticSvg {
    name: &'static str,
    source: &'static str,
    factory: Option<fn(&'static str) -> SvgDocument>,
    document: OnceLock<SvgDocument>,
}

impl StaticSvg {
    /// Embeds source for parsing on first use, using `name` in error messages.
    ///
    /// Construction does not validate the source. [`Self::document`] and its
    /// implicit conversion in [`crate::Svg::new`] panic if parsing fails.
    #[inline]
    pub const fn new(name: &'static str, source: &'static str) -> Self {
        Self { name, source, factory: None, document: OnceLock::new() }
    }

    /// Creates a descriptor backed by a build-time validated scene factory.
    ///
    /// The factory must reconstruct the retained document for `source` without
    /// parsing it. It runs at most once after successful initialization.
    #[doc(hidden)]
    pub const fn with_precompiled(
        name: &'static str,
        source: &'static str,
        factory: fn(&'static str) -> SvgDocument,
    ) -> Self {
        Self { name, source, factory: Some(factory), document: OnceLock::new() }
    }

    /// Returns the original embedded SVG without initializing its document.
    #[inline]
    pub const fn source(&self) -> &'static str {
        self.source
    }

    /// Returns a shared retained document, initializing it on first use.
    ///
    /// # Panics
    ///
    /// Panics with the descriptor name if embedded source cannot be parsed,
    /// or if a supplied precompiled factory panics.
    #[inline]
    pub fn document(&self) -> SvgDocument {
        self.document.get_or_init(|| match self.factory {
            Some(factory) => factory(self.source),
            None => SvgDocument::from_svg(self.source)
                .unwrap_or_else(|error| panic!("invalid embedded SVG {}: {error}", self.name)),
        }).clone()
    }
}

impl From<&'static StaticSvg> for SvgDocument {
    #[inline]
    fn from(icon: &'static StaticSvg) -> Self {
        icon.document()
    }
}
