mod cache;
pub mod copy_button;
mod custom;
mod document;
mod markdown_theme;
mod renderer;
mod syntax;

// Arborium's debug Tree-sitter runtime references C's `stderr`, which is not
// provided by the `wasm32-unknown-unknown` target. Its optional diagnostics
// accept a null stream, so provide the missing WASM-side storage here. The
// default `synoptic` highlighter is pure Rust and needs none of this.
#[cfg(all(target_arch = "wasm32", feature = "arborium"))]
#[unsafe(no_mangle)]
static mut stderr: usize = 0;

use std::cell::RefCell;
use std::rc::Rc;

use aimer_container::Container;
use aimer_rubick::{ShareRef, Shared};
use aimer_scroll::{ScrollAxis, Scrollable};
use aimer_style::LayoutSpacing;
use aimer_widget::base::BuildContext;
use aimer_widget::{AnyElement, AnyWidget, Key, Widget};
use cache::LruCache;
pub use custom::{
    BlockRule, BlockSyntax, CustomBlock, CustomBlockBuilder, CustomBlockData, CustomBlockInput,
    CustomInline, CustomInlineBuilder, CustomInlineData, InlineRule, InlineSyntax,
};
pub use document::{Alignment, Block, Document, Inline, ListItem, MarkdownError, TableRow};
pub use markdown_theme::MarkdownTheme;
pub use renderer::{ImageResolver, LinkHandler, MarkdownImage, default_image_resolver};
pub use syntax::{CaptureSpan, highlight};
#[cfg(feature = "syntect")]
pub use syntax::{add_syntax_from_dir, add_syntax_from_file, add_syntax_from_str};

/// Documents kept parsed. A chat transcript rebuilds every message in the same
/// order, so a cache smaller than the transcript evicts each entry just before
/// it is needed again and every rebuild re-parses everything.
const DOCUMENT_CACHE_CAPACITY: usize = 256;

aimer_tls::thread_local! {
    static DOCUMENT_CACHE: RefCell<DocumentCache> = RefCell::new(DocumentCache::new(DOCUMENT_CACHE_CAPACITY));
}

struct DocumentCache {
    entries: LruCache<ShareRef<str>, Shared<Result<Document, MarkdownError>>>,
}

impl DocumentCache {
    fn new(capacity: usize) -> Self {
        Self {
            entries: LruCache::new(capacity),
        }
    }

    fn parse(&mut self, source: ShareRef<str>) -> Shared<Result<Document, MarkdownError>> {
        self.entries.get_or_insert_with(source, |source| {
            Shared::new(Document::parse_shared(source.clone()))
        })
    }
}

fn parse_document(source: ShareRef<str>) -> Shared<Result<Document, MarkdownError>> {
    DOCUMENT_CACHE.with(|cache| cache.borrow_mut().parse(source))
}

fn open_web_link_with<E>(
    target: &str,
    opener: impl FnOnce(&str) -> Result<(), E>,
) -> Option<Result<(), E>> {
    if !target.starts_with("https://") && !target.starts_with("http://") {
        return None;
    }
    Some(opener(target))
}

fn open_web_link(target: Rc<str>) {
    if let Some(Err(error)) = open_web_link_with(&target, webbrowser::open) {
        eprintln!("Failed to open Markdown link '{target}': {error}");
    }
}

/// A scrollable Markdown document rendered with native Aimer widgets.
///
/// Create an empty viewer with [`MarkdownViewer::new`], then provide source
/// with [`MarkdownViewer::markdown`]. Fenced code blocks display their language
/// in a header and provide a copy control that writes the complete code source
/// to the platform clipboard. Unlabelled fences keep the header without showing
/// a language name. The internal viewport follows normal [`Scrollable`]
/// reconciliation by default; use [`MarkdownViewer::key`] when it needs an
/// application-defined identity.
#[derive(Clone)]
pub struct MarkdownViewer {
    source: Option<Shared<str>>,
    theme: MarkdownTheme,
    link_handler: Option<LinkHandler>,
    image_resolver: ImageResolver,
    custom_blocks: Vec<(BlockRule, CustomBlockBuilder)>,
    custom_inlines: Vec<(InlineRule, CustomInlineBuilder)>,
    typed_custom_blocks: Vec<(BlockRule, custom::TypedCustomBlockBuilder)>,
    typed_custom_inlines: Vec<(InlineRule, custom::TypedCustomInlineBuilder)>,
    padding: LayoutSpacing,
    scrollable: bool,
    key: Option<Key>,
}

impl Default for MarkdownViewer {
    fn default() -> Self {
        Self::new()
    }
}

impl MarkdownViewer {
    /// Creates an empty viewer with the default theme and image resolver.
    ///
    /// Activated HTTP and HTTPS links open in the system web browser by
    /// default.
    pub fn new() -> Self {
        Self {
            source: None,
            theme: MarkdownTheme::default(),
            link_handler: Some(Rc::new(open_web_link)),
            image_resolver: Rc::new(default_image_resolver),
            custom_blocks: Vec::new(),
            custom_inlines: Vec::new(),
            typed_custom_blocks: Vec::new(),
            typed_custom_inlines: Vec::new(),
            padding: Default::default(),
            key: None,
            scrollable: true,
        }
    }

    pub fn padding(mut self, padding: LayoutSpacing) -> Self {
        self.padding = padding;
        self
    }

    /// Sets the Markdown source rendered by this viewer.
    pub fn markdown(mut self, source: impl Into<Shared<str>>) -> Self {
        self.source = Some(source.into());
        self
    }

    /// Sets whether the viewer should be scrollable.
    pub fn scrollable(mut self, scrollable: bool) -> Self {
        self.scrollable = scrollable;
        self
    }

    /// Replaces the complete visual theme.
    pub fn theme(mut self, theme: MarkdownTheme) -> Self {
        self.theme = theme;
        self
    }

    /// Replaces the default browser opener with a custom link handler.
    ///
    /// The handler receives every activated target, including `#footnote-*`
    /// references.
    pub fn on_link(mut self, handler: impl Fn(Rc<str>) + 'static) -> Self {
        self.link_handler = Some(Rc::new(handler));
        self
    }

    /// Resolves each Markdown image into an arbitrary Aimer widget.
    pub fn image_resolver(
        mut self,
        resolver: impl Fn(&MarkdownImage) -> AnyWidget + 'static,
    ) -> Self {
        self.image_resolver = Rc::new(resolver);
        self
    }

    /// Registers a paired custom block and its widget builder.
    #[inline]
    pub fn custom_block(
        mut self,
        rule: impl Into<BlockRule>,
        builder: impl Fn(&CustomBlockData) -> AnyWidget + 'static,
    ) -> Self {
        self.custom_blocks.push((rule.into(), Rc::new(builder)));
        self
    }

    /// Registers a paired custom inline value and its widget builder.
    #[inline]
    pub fn custom_inline(
        mut self,
        rule: impl Into<InlineRule>,
        builder: impl Fn(&CustomInlineData) -> AnyWidget + 'static,
    ) -> Self {
        self.custom_inlines.push((rule.into(), Rc::new(builder)));
        self
    }

    /// Registers a strongly typed custom block.
    #[inline]
    pub fn typed_block<T: CustomBlock>(mut self) -> Self {
        let rule = BlockRule::new(
            T::NAME,
            BlockSyntax::Paired {
                opening: T::OPENING,
                closing: T::CLOSING,
            },
        );
        let builder = Rc::new(|data: &CustomBlockData, ctx: &BuildContext| {
            match T::parse(CustomBlockInput {
                raw: &data.text,
                content: &data.content,
            }) {
                Ok(props) => T::build(&props, ctx),
                Err(error) => {
                    aimer_text::Text::new(format!("custom block '{}': {error}", T::NAME)).boxed()
                }
            }
        });
        self.typed_custom_blocks.push((rule, builder));
        self
    }

    /// Registers a strongly typed custom inline element.
    #[inline]
    pub fn typed_inline<T: CustomInline>(mut self) -> Self {
        let rule = InlineRule::new(
            T::NAME,
            InlineSyntax::Paired {
                opening: T::OPENING,
                closing: T::CLOSING,
            },
        );
        let builder =
            Rc::new(
                |data: &CustomInlineData, ctx: &BuildContext| match T::parse(&data.text) {
                    Ok(props) => T::build(&props, ctx),
                    Err(error) => {
                        aimer_text::Text::new(format!("custom inline '{}': {error}", T::NAME))
                            .boxed()
                    }
                },
            );
        self.typed_custom_inlines.push((rule, builder));
        self
    }

    /// Adds an explicit identity for the internal scrollable viewport.
    ///
    /// Without an explicit key, the viewer uses the scrollable's normal
    /// call-site identity and preserves its live position across rebuilds.
    /// An explicit key additionally opts into scroll-offset persistence across
    /// a full teardown.
    pub fn key(mut self, key: Key) -> Self {
        self.key = Some(key);
        self
    }
}

impl Widget for MarkdownViewer {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let block_rules = self
            .custom_blocks
            .iter()
            .map(|(rule, _)| rule.clone())
            .collect::<Vec<BlockRule>>();
        let inline_rules = self
            .custom_inlines
            .iter()
            .map(|(rule, _)| rule.clone())
            .collect::<Vec<_>>();
        let typed_block_rules = self
            .typed_custom_blocks
            .iter()
            .map(|(rule, _)| rule.clone())
            .collect::<Vec<_>>();
        let typed_inline_rules = self
            .typed_custom_inlines
            .iter()
            .map(|(rule, _)| rule.clone())
            .collect::<Vec<_>>();
        let block_rules = block_rules
            .into_iter()
            .chain(typed_block_rules)
            .collect::<Vec<_>>();
        let inline_rules = inline_rules
            .into_iter()
            .chain(typed_inline_rules)
            .collect::<Vec<_>>();
        let source = self.source.unwrap_or_default().as_share_ref();
        let document = if block_rules.is_empty() && inline_rules.is_empty() {
            parse_document(source)
        } else {
            Shared::new(Document::parse_shared_with_rules(
                source.clone(),
                &block_rules,
                &inline_rules,
            ))
        };
        let content = match document.as_ref() {
            Ok(document) => renderer::render_document_with_context(
                document,
                &self.theme,
                self.link_handler.as_ref(),
                &self.image_resolver,
                &self.custom_blocks,
                &self.custom_inlines,
                &self.typed_custom_blocks,
                &self.typed_custom_inlines,
                Some(ctx),
            ),
            Err(error) => aimer_text::Text::new(error.to_string())
                .text_style(self.theme.body)
                .boxed(),
        };

        if self.scrollable {
            // The viewer is rebuilt when an inherited theme changes. Let the
            // scrollable keep its call-site identity so those rebuilds retain
            // the live offset; a unique key would reset it on every theme tick.
            let mut scrollable = Scrollable::new().axis(ScrollAxis::Vertical);
            if let Some(key) = self.key {
                scrollable = scrollable.key(key);
            }
            scrollable
                .child(Container::new().padding(self.padding).child(content))
                .to_element(ctx)
        } else {
            Container::new()
                .padding(self.padding)
                .child(content)
                .to_element(ctx)
        }
    }

    fn debug_name(&self) -> &'static str {
        "MarkdownViewer"
    }

    fn text_content(&self) -> Option<&str> {
        Some(self.source.as_deref().unwrap_or(""))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn document_cache_reuses_unchanged_markdown() {
        let mut cache = DocumentCache::new(2);
        let source: Shared<str> = Shared::from("# Cached");

        let first = cache.parse(source.as_share_ref());
        let second = cache.parse(Shared::<str>::from("# Cached").as_share_ref());

        assert!(Shared::ptr_eq(&first, &second));
    }

    #[test]
    fn document_cache_parses_updated_markdown() {
        let mut cache = DocumentCache::new(2);

        let first = cache.parse(Shared::<str>::from("# Before").as_share_ref());
        let second = cache.parse(Shared::<str>::from("# After").as_share_ref());

        assert!(!Shared::ptr_eq(&first, &second));
        assert_ne!(first.as_ref(), second.as_ref());
    }

    #[test]
    fn document_cache_reuses_parse_errors() {
        let mut cache = DocumentCache::new(2);

        let first = cache.parse(Shared::<str>::from("<div>unsupported</div>").as_share_ref());
        let second = cache.parse(Shared::<str>::from("<div>unsupported</div>").as_share_ref());

        assert!(first.is_err());
        assert!(Shared::ptr_eq(&first, &second));
    }

    #[test]
    fn document_cache_holds_a_long_transcript() {
        // A chat transcript is walked in the same order on every rebuild, which
        // evicts every entry before it is reused once it outgrows the cache.
        let sources: Vec<Shared<str>> = (0..64)
            .map(|index| Shared::from(format!("# Reply {index}")))
            .collect();
        let mut cache = DocumentCache::new(DOCUMENT_CACHE_CAPACITY);
        let first: Vec<_> = sources
            .iter()
            .map(|source| cache.parse(source.as_share_ref()))
            .collect();

        for (source, parsed) in sources.iter().zip(&first) {
            assert!(Shared::ptr_eq(parsed, &cache.parse(source.as_share_ref())));
        }
    }

    #[test]
    fn document_cache_evicts_the_least_recently_used_source() {
        let mut cache = DocumentCache::new(2);
        let first = cache.parse(Shared::<str>::from("First").as_share_ref());
        cache.parse(Shared::<str>::from("Second").as_share_ref());
        cache.parse(Shared::<str>::from("Third").as_share_ref());

        let reparsed = cache.parse(Shared::<str>::from("First").as_share_ref());

        assert!(!Shared::ptr_eq(&first, &reparsed));
    }

    #[test]
    fn viewer_does_not_force_a_new_scroll_identity_by_default() {
        assert!(MarkdownViewer::new().key.is_none());
    }

    #[test]
    fn viewer_keeps_an_explicit_scroll_identity() {
        let key = Key::from("markdown-viewer");
        assert_eq!(MarkdownViewer::new().key(key.clone()).key, Some(key));
    }

    #[test]
    fn web_links_are_forwarded_to_the_browser() {
        let opened = RefCell::new(Vec::new());

        let handled = open_web_link_with("https://aimer.dev/docs", |url| {
            opened.borrow_mut().push(url.to_owned());
            Ok::<(), ()>(())
        });

        assert!(matches!(handled, Some(Ok(()))));
        assert_eq!(opened.into_inner(), ["https://aimer.dev/docs"]);
    }

    #[test]
    fn document_anchors_are_not_forwarded_to_the_browser() {
        let opened = RefCell::new(Vec::new());

        let handled = open_web_link_with("#footnote-guide", |url| {
            opened.borrow_mut().push(url.to_owned());
            Ok::<(), ()>(())
        });

        assert!(handled.is_none());
        assert!(opened.into_inner().is_empty());
    }

    #[test]
    fn viewer_registers_typed_custom_rules() {
        struct Alert;

        impl CustomBlock for Alert {
            const NAME: &'static str = "alert";
            const OPENING: &'static str = ":::alert";

            type Props = String;

            fn parse(input: CustomBlockInput<'_>) -> Result<Self::Props, MarkdownError> {
                Ok(input.raw.to_owned())
            }

            fn build(_props: &Self::Props, _ctx: &BuildContext) -> AnyWidget {
                aimer_text::Text::new("alert").boxed()
            }
        }

        struct Mention;

        impl CustomInline for Mention {
            const NAME: &'static str = "mention";
            const OPENING: &'static str = "@{";
            const CLOSING: &'static str = "}";

            type Props = String;

            fn parse(raw: &str) -> Result<Self::Props, MarkdownError> {
                Ok(raw.to_owned())
            }

            fn build(_props: &Self::Props, _ctx: &BuildContext) -> AnyWidget {
                aimer_text::Text::new("mention").boxed()
            }
        }

        let _viewer = MarkdownViewer::new()
            .typed_block::<Alert>()
            .typed_inline::<Mention>();
    }

}
