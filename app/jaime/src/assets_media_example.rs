//! Jaime's bounded asset lifecycle and optional-media fallback example.
//!
//! W17 owns central showcase registration and workspace dependency wiring. The
//! page remains a deterministic capability/fallback example on native and web
//! targets.

use std::collections::BTreeMap;

use aimer::{
    AnyElement, AssetData, AssetLoadOperation, AssetLoadPoll, AssetManager, AssetMetadata,
    AssetProgress, AssetRef, AssetRequest, AssetResolver, AssetSource, BuildContext, Column,
    Container, Icon, IconContext, IconDirection, IconSource, IconTheme, IconTint, LoadHandle,
    LoadState, Text, Widget,
};

use aimer::media::{
    CapabilitySet, MediaElement as ModelMediaElement, MediaId as ModelMediaId,
    MediaSource as ModelMediaSource,
};

#[derive(Clone, Copy, Debug)]
struct ExampleResolver;

struct ExampleOperation {
    data: Option<AssetData>,
    cancelled: bool,
}

impl AssetLoadOperation for ExampleOperation {
    fn poll(&mut self) -> AssetLoadPoll {
        if self.cancelled {
            return AssetLoadPoll::Cancelled;
        }
        self.data
            .take()
            .map(AssetLoadPoll::Ready)
            .unwrap_or(AssetLoadPoll::Pending(AssetProgress::known(1.0).unwrap()))
    }

    fn cancel(&mut self) {
        self.cancelled = true;
    }
}

impl AssetResolver for ExampleResolver {
    fn start(
        &self,
        _request: &AssetRequest,
    ) -> Result<Box<dyn AssetLoadOperation>, aimer::AssetError> {
        let data = AssetData::new(b"<svg viewBox='0 0 16 16'/ >".to_vec()).with_metadata(
            AssetMetadata::new(Some("image/svg+xml"), Some(16), Some(16), false),
        );
        Ok(Box::new(ExampleOperation {
            data: Some(data),
            cancelled: false,
        }))
    }
}

/// A small public snapshot used by the example test without requiring a renderer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssetsMediaSnapshot {
    asset_is_ready: bool,
    cache_entries: usize,
    icon_size: u32,
    media_is_unsupported: bool,
}

impl AssetsMediaSnapshot {
    /// Returns whether the demo asset reached a ready state.
    pub const fn asset_is_ready(&self) -> bool {
        self.asset_is_ready
    }

    /// Returns the bounded cache entry count.
    pub const fn cache_entries(&self) -> usize {
        self.cache_entries
    }

    /// Returns the resolved icon size rounded to a logical pixel.
    pub const fn icon_size(&self) -> u32 {
        self.icon_size
    }

    /// Returns whether the optional media fallback is explicit.
    pub const fn media_is_unsupported(&self) -> bool {
        self.media_is_unsupported
    }
}

/// A deterministic assets/media showcase page.
pub struct AssetsMediaExample {
    manager: AssetManager<ExampleResolver>,
    handle: LoadHandle,
    icon: Icon,
    media: ModelMediaElement,
}

impl AssetsMediaExample {
    /// Creates a preload/cache, icon, and unsupported-media demonstration.
    pub fn new() -> Self {
        let asset = AssetRef::new(AssetSource::network(
            "https://cdn.example.test/icons/check.svg",
            BTreeMap::from([("accept".to_owned(), "image/svg+xml".to_owned())]),
        ))
        .expect("the example source has a stable identity");
        let mut manager = AssetManager::with_cache_config(
            ExampleResolver,
            aimer::AssetCacheConfig::new(4, 4 * 1024).expect("positive example cache limits"),
        );
        manager
            .register(asset.clone())
            .expect("the example manifest entry is unique");
        let handle = manager
            .preload(AssetRequest::new(asset))
            .expect("the example source passes the default policy");
        manager
            .poll(handle)
            .expect("the example resolver is deterministic");

        let icon = Icon::new(IconSource::glyph("Symbols", '✓'))
            .size(24.0)
            .expect("the example icon size is finite")
            .tint(IconTint::rgba(0, 0, 0, 255));

        let mut media = ModelMediaElement::video(
            ModelMediaId::new(13),
            ModelMediaSource::url("https://cdn.example.test/video.mp4"),
            CapabilitySet::new(),
        )
        .expect("the example media source is non-empty");
        let _ = media.load();

        Self {
            manager,
            handle,
            icon,
            media,
        }
    }

    /// Builds the model-only state asserted by the integration test.
    pub fn snapshot(&self) -> AssetsMediaSnapshot {
        let asset_is_ready = matches!(
            self.manager
                .status(self.handle),
            Ok(LoadState::Ready { .. })
        );
        let icon = self
            .icon
            .resolve(IconContext::new(IconTheme::Dark, IconDirection::Ltr, false));
        AssetsMediaSnapshot {
            asset_is_ready,
            cache_entries: self
                .manager
                .cache_stats()
                .entries(),
            icon_size: icon.size().round() as u32,
            media_is_unsupported: matches!(
                self.media.state(),
                aimer::media::MediaState::Unsupported { .. }
            ),
        }
    }
}

impl Default for AssetsMediaExample {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for AssetsMediaExample {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let snapshot = self.snapshot();
        let icon = self
            .icon
            .resolve(IconContext::new(IconTheme::Dark, IconDirection::Ltr, false));
        Container::new()
            .child(
                Column::new().children([
                    Text::new("Assets and optional media")
                        .wrapped()
                        .boxed(),
                    Text::new(format!(
                        "Preload/cache: ready={}, entries={}",
                        snapshot.asset_is_ready(),
                        snapshot.cache_entries()
                    ))
                    .wrapped()
                    .boxed(),
                    Text::new(format!(
                        "Icon: glyph at {}px, tint={:?}",
                        icon.size(),
                        icon.tint()
                            .map(IconTint::channels)
                    ))
                    .wrapped()
                    .boxed(),
                    Text::new(format!(
                        "Optional video capability: {}",
                        if snapshot.media_is_unsupported() {
                            "unsupported on this adapter"
                        } else {
                            "available"
                        }
                    ))
                    .wrapped()
                    .boxed(),
                ]),
            )
            .to_element(ctx)
    }

    fn debug_name(&self) -> &'static str {
        "AssetsMediaExample"
    }
}

impl aimer::PortableWidget for AssetsMediaExample {}

/// Builds the assets/media showcase without starting an application.
pub fn assets_media_example() -> impl Widget {
    AssetsMediaExample::new()
}
