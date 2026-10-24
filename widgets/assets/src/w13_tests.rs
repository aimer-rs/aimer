use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use crossbeam::channel::{Receiver, unbounded};

use super::{
    AssetCacheConfig, AssetData, AssetErrorKind, AssetLoadOperation, AssetLoadPoll, AssetManager,
    AssetManifest, AssetMetadata, AssetPolicy, AssetProgress, AssetRef, AssetRequest,
    AssetResolver, AssetSource, DecodeProfile, Icon, IconContext, IconDirection, IconSource,
    IconTheme, IconTint, ImageSource, LoadState,
};

#[derive(Debug)]
struct ScriptedOperation {
    polls: VecDeque<AssetLoadPoll>,
    cancelled: bool,
}

impl AssetLoadOperation for ScriptedOperation {
    fn poll(&mut self) -> AssetLoadPoll {
        self.polls
            .pop_front()
            .unwrap_or(AssetLoadPoll::Pending(AssetProgress::unknown()))
    }

    fn cancel(&mut self) {
        self.cancelled = true;
    }
}

#[derive(Clone, Debug)]
struct ScriptedResolver {
    scripts: Receiver<VecDeque<AssetLoadPoll>>,
    starts: Arc<AtomicUsize>,
}

impl ScriptedResolver {
    fn new(scripts: impl IntoIterator<Item = Vec<AssetLoadPoll>>) -> Self {
        let (sender, receiver) = unbounded();
        for script in scripts {
            sender.send(VecDeque::from(script)).unwrap();
        }
        Self {
            scripts: receiver,
            starts: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn starts(&self) -> usize {
        self.starts.load(Ordering::Relaxed)
    }
}

impl AssetResolver for ScriptedResolver {
    fn start(
        &self,
        _request: &AssetRequest,
    ) -> Result<Box<dyn AssetLoadOperation>, super::AssetError> {
        self.starts.fetch_add(1, Ordering::Relaxed);
        let polls = self
            .scripts
            .try_recv()
            .unwrap_or_else(|_| VecDeque::from([AssetLoadPoll::Pending(AssetProgress::unknown())]));
        Ok(Box::new(ScriptedOperation {
            polls,
            cancelled: false,
        }))
    }
}

fn asset_ref() -> AssetRef {
    AssetRef::new(AssetSource::bundled("icons/check.svg")).unwrap()
}

fn ready(bytes: &'static [u8]) -> AssetLoadPoll {
    AssetLoadPoll::Ready(
        AssetData::new(bytes.to_vec()).with_metadata(AssetMetadata::new(
            Some("image/svg+xml"),
            Some(16),
            Some(16),
            false,
        )),
    )
}

#[test]
fn source_identity_is_stable_and_includes_network_request_variants() {
    let mut uppercase = BTreeMap::new();
    uppercase.insert("Accept".to_owned(), "image/avif".to_owned());
    let mut lowercase = BTreeMap::new();
    lowercase.insert("accept".to_owned(), "image/avif".to_owned());

    let first = AssetRef::new(AssetSource::network(
        "https://cdn.example/icon.svg",
        uppercase,
    ))
    .unwrap();
    let second = AssetRef::new(AssetSource::network(
        "https://cdn.example/icon.svg",
        lowercase,
    ))
    .unwrap();
    assert_eq!(first.id(), second.id());

    let changed = AssetRef::new(AssetSource::network(
        "https://cdn.example/icon.svg",
        BTreeMap::from([("accept".to_owned(), "image/png".to_owned())]),
    ))
    .unwrap();
    assert_ne!(first.id(), changed.id());

    let legacy =
        AssetRef::from_image_source(&ImageSource::Asset("icons/check.svg".to_owned())).unwrap();
    assert_eq!(legacy.source(), &AssetSource::bundled("icons/check.svg"));
}

#[test]
fn manifest_rejects_conflicting_identity_and_resolves_registered_sources() {
    let source = asset_ref();
    let mut manifest = AssetManifest::new();
    manifest.register(source.clone()).unwrap();
    assert_eq!(manifest.resolve(source.id()).unwrap(), &source);

    let conflicting = AssetRef::new(AssetSource::File(PathBuf::from("icons/check.svg"))).unwrap();
    let error = manifest
        .register_with_id(source.id().clone(), conflicting)
        .unwrap_err();
    assert_eq!(error.kind(), AssetErrorKind::ManifestConflict);
}

#[test]
fn loading_is_deduplicated_and_publishes_progress_then_ready_cache_state() {
    let resolver = ScriptedResolver::new([vec![
        AssetLoadPoll::Pending(AssetProgress::known(0.25).unwrap()),
        ready(b"svg"),
    ]]);
    let mut manager =
        AssetManager::with_cache_config(resolver.clone(), AssetCacheConfig::new(4, 1024).unwrap());
    let request = AssetRequest::new(asset_ref());
    let first = manager.request(request.clone()).unwrap();
    let second = manager.request(request).unwrap();
    assert_eq!(first, second);
    assert_eq!(resolver.starts(), 1);

    assert!(matches!(
        manager.status(first).unwrap(),
        LoadState::Loading { .. }
    ));
    assert!(matches!(
        manager.poll(first).unwrap(),
        LoadState::Loading { progress, .. } if progress.fraction() == Some(0.25)
    ));
    assert!(matches!(
        manager.poll(first).unwrap(),
        LoadState::Ready { stale: false, .. }
    ));
    assert_eq!(manager.cache_stats().entries(), 1);

    let cached = manager.request(AssetRequest::new(asset_ref())).unwrap();
    assert!(matches!(
        manager.status(cached).unwrap(),
        LoadState::Ready { stale: false, .. }
    ));
    assert_eq!(resolver.starts(), 1);
}

#[test]
fn errors_can_be_retried_and_cancellation_is_visible() {
    let resolver = ScriptedResolver::new([
        vec![AssetLoadPoll::Failed(super::AssetError::resolver(
            "offline",
        ))],
        vec![ready(b"retry")],
        vec![AssetLoadPoll::Pending(AssetProgress::unknown())],
    ]);
    let mut manager = AssetManager::new(resolver.clone());
    let handle = manager.request(AssetRequest::new(asset_ref())).unwrap();
    assert!(matches!(
        manager.poll(handle).unwrap(),
        LoadState::Error { .. }
    ));
    manager.retry(handle).unwrap();
    assert!(matches!(
        manager.poll(handle).unwrap(),
        LoadState::Ready { .. }
    ));
    assert_eq!(resolver.starts(), 2);

    let other = AssetRef::new(AssetSource::bundled("icons/other.svg")).unwrap();
    let cancelled = manager.request(AssetRequest::new(other)).unwrap();
    manager.cancel(cancelled).unwrap();
    assert!(matches!(
        manager.status(cancelled).unwrap(),
        LoadState::Cancelled { .. }
    ));
}

#[test]
fn cache_is_bounded_by_bytes_and_supports_invalidation_and_profile_variants() {
    let resolver = ScriptedResolver::new([
        vec![ready(b"1234")],
        vec![ready(b"5678")],
        vec![ready(b"9")],
    ]);
    let mut manager =
        AssetManager::with_cache_config(resolver, AssetCacheConfig::new(2, 5).unwrap());
    let source = asset_ref();
    let first = manager.request(AssetRequest::new(source.clone())).unwrap();
    manager.poll(first).unwrap();
    let profile = DecodeProfile::new().target_size(32, 32).unwrap();
    let second = manager
        .request(AssetRequest::new(source.clone()).decode_profile(profile))
        .unwrap();
    manager.poll(second).unwrap();
    assert!(manager.cache_stats().bytes() <= 5);
    assert!(manager.cache_stats().entries() <= 2);

    manager.invalidate(source.id());
    assert_eq!(manager.cache_stats().entries(), 0);
}

#[test]
fn policy_rejects_unsafe_sources_and_can_return_stale_data_after_failure() {
    let resolver = ScriptedResolver::new([
        vec![ready(b"old")],
        vec![AssetLoadPoll::Failed(super::AssetError::resolver(
            "offline",
        ))],
    ]);
    let mut manager = AssetManager::new(resolver);
    let old = AssetRequest::new(asset_ref()).version(1);
    let old_handle = manager.request(old).unwrap();
    manager.poll(old_handle).unwrap();

    let stale = AssetRequest::new(asset_ref()).version(2).allow_stale(true);
    let stale_handle = manager.request(stale).unwrap();
    assert!(matches!(
        manager.poll(stale_handle).unwrap(),
        LoadState::Ready {
            stale: true,
            stale_error: Some(error),
            ..
        } if error.kind() == AssetErrorKind::Resolver
    ));

    let unsafe_source = AssetRef::new(AssetSource::bundled("../private/key.pem")).unwrap();
    let error = manager
        .request(AssetRequest::new(unsafe_source))
        .unwrap_err();
    assert_eq!(error.kind(), AssetErrorKind::UnsafePath);

    let network = AssetRef::new(AssetSource::network(
        "https://untrusted.example/icon.svg",
        BTreeMap::new(),
    ))
    .unwrap();
    let network_policy = AssetPolicy::new().allow_network_origin("https://trusted.example");
    let mut network_manager = AssetManager::with_policy(
        ScriptedResolver::new([vec![ready(b"unused")]]),
        network_policy,
    );
    let error = network_manager
        .request(AssetRequest::new(network))
        .unwrap_err();
    assert_eq!(error.kind(), AssetErrorKind::NetworkOriginDenied);

    let strict = AssetPolicy::new().max_response_bytes(2).unwrap();
    let mut strict_manager =
        AssetManager::with_policy(ScriptedResolver::new([vec![ready(b"123")]]), strict);
    let handle = strict_manager
        .request(AssetRequest::new(asset_ref()))
        .unwrap();
    assert!(
        matches!(strict_manager.poll(handle).unwrap(), LoadState::Error { error, .. } if error.kind() == AssetErrorKind::ResourceLimit)
    );
}

#[test]
fn icon_resolution_applies_theme_rtl_size_tint_and_high_contrast() {
    let light = IconSource::glyph("Symbols", 'L');
    let dark = IconSource::glyph("Symbols", 'D');
    let source = IconSource::themed(
        IconSource::directional(light.clone(), IconSource::glyph("Symbols", 'R')),
        dark.clone(),
    );
    let icon = Icon::new(source)
        .size(28.0)
        .unwrap()
        .tint(IconTint::rgba(1, 2, 3, 4))
        .high_contrast_tint(IconTint::rgba(9, 8, 7, 6));

    let resolved = icon.resolve(IconContext::new(IconTheme::Light, IconDirection::Rtl, true));
    assert_eq!(resolved.source(), &IconSource::glyph("Symbols", 'R'));
    assert_eq!(resolved.size(), 28.0);
    assert_eq!(resolved.tint(), Some(IconTint::rgba(9, 8, 7, 6)));

    let resolved = icon.resolve(IconContext::new(IconTheme::Dark, IconDirection::Ltr, false));
    assert_eq!(resolved.source(), &dark);
    assert_eq!(resolved.tint(), Some(IconTint::rgba(1, 2, 3, 4)));
}
