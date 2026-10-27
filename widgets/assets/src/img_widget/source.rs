use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::Hash;
#[cfg(not(target_arch = "wasm32"))]
use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use aimer_cupid::draw_cmd_v2::ImageResource;
use aimer_utils::error;
#[cfg(not(target_arch = "wasm32"))]
use aimer_venus::Venus;
use aimer_widget::base::{BuildContext, WindowHandle};
use crossbeam::channel::{Receiver, Sender, TryRecvError, unbounded};
use once_cell::sync::Lazy;

use crate::ImageResult::Success;
use crate::{ImageProvider, ImageResult};

/// Decoded pixels are much larger than the small `Loaded(texture_id, ...)`
/// records. Keep a generous working set, then release cold decoded entries;
/// the GPU cache has its own byte budget and protects textures referenced by
/// the current draw list.
const DECODED_CACHE_BUDGET_BYTES: usize = 64 * 1024 * 1024;
const DECODED_CACHE_MAX_ENTRIES: usize = 512;
const DECODED_CACHE_IDLE_ACCESSES: u64 = 128;
static DECODED_CACHE_ACCESS_CLOCK: AtomicU64 = AtomicU64::new(0);

#[inline]
fn next_cache_access() -> u64 {
    DECODED_CACHE_ACCESS_CLOCK
        .fetch_add(1, Ordering::Relaxed)
        .wrapping_add(1)
}

#[inline]
fn touch_cache_key<K: Clone + Eq + Hash>(access: &mut HashMap<K, u64>, key: &K) -> u64 {
    let tick = next_cache_access();
    access.insert(key.clone(), tick);
    tick
}

#[inline]
fn remove_cache_key<K: Eq + Hash>(access: &mut HashMap<K, u64>, key: &K) {
    access.remove(key);
}

#[derive(Clone, Debug)]
enum ImageCacheState {
    Loading,
    Ready(Vec<u8>, u32, u32, u32, u32),
    Loaded(u32, u32, u32, Arc<ImageResource>),
    Error(String),
}

enum ImageCacheUpdate {
    File {
        path: PathBuf,
        state: ImageCacheState,
    },
    Network {
        url: String,
        state: ImageCacheState,
    },
    #[cfg(not(target_arch = "wasm32"))]
    Asset {
        key: String,
        state: ImageCacheState,
    },
}

struct ImageCacheMailbox {
    sender: Sender<ImageCacheUpdate>,
    receiver: Receiver<ImageCacheUpdate>,
}

impl ImageCacheMailbox {
    fn new() -> Self {
        let (sender, receiver) = unbounded();
        Self { sender, receiver }
    }
}

static IMAGE_CACHE_MAILBOX: Lazy<ImageCacheMailbox> = Lazy::new(ImageCacheMailbox::new);

/// Image caches are owned by the thread that owns the render `BuildContext`.
/// Background decoders never access these maps directly; they publish completed
/// states through `IMAGE_CACHE_MAILBOX`, and the render thread drains it before
/// looking up an image. `BuildContext` contains the non-`Send` canvas/widget
/// state, so image lookup is already confined to that render thread.
#[derive(Default)]
struct ImageCaches {
    file: HashMap<PathBuf, ImageCacheState>,
    file_access: HashMap<PathBuf, u64>,
    network: HashMap<String, ImageCacheState>,
    network_access: HashMap<String, u64>,
    #[cfg(not(target_arch = "wasm32"))]
    asset: HashMap<String, ImageCacheState>,
    #[cfg(not(target_arch = "wasm32"))]
    asset_access: HashMap<String, u64>,
}

aimer_tls::thread_local! {
    static IMAGE_CACHES: RefCell<ImageCaches> = RefCell::new(ImageCaches::default());
}

enum CacheLookup {
    StartLoad,
    Loading,
    Ready {
        bytes: Vec<u8>,
        upload_width: u32,
        upload_height: u32,
        width: u32,
        height: u32,
    },
    Loaded {
        id: u32,
        width: u32,
        height: u32,
    },
    Error(String),
}

/// Evicts only cold entries. `is_protected` marks entries that must stay
/// regardless of age: work still in flight, and decoded images the retained
/// render tree is still painting. Age alone cannot identify the latter, because
/// the retained paint path reads a settled image without touching it, so a
/// stable visible image looks idle while other images are looked up. If the
/// active working set itself exceeds the budget, it is retained until entries
/// become cold; correctness and visible-content residency take precedence over
/// a hard byte cap.
fn prune_decoded_cache<K, S>(
    cache: &mut HashMap<K, S>,
    access: &mut HashMap<K, u64>,
    now: u64,
    decoded_bytes: impl Fn(&S) -> usize,
    is_protected: impl Fn(&S) -> bool,
) where
    K: Clone + Eq + Hash,
{
    let mut total_bytes = cache.values().map(&decoded_bytes).sum::<usize>();
    let mut candidates = cache
        .iter()
        .filter_map(|(key, state)| {
            if is_protected(state) {
                return None;
            }
            Some((
                key.clone(),
                access.get(key).copied().unwrap_or(0),
                decoded_bytes(state),
            ))
        })
        .collect::<Vec<_>>();
    candidates.sort_unstable_by_key(|(_, last_access, _)| *last_access);

    for (key, last_access, bytes) in candidates {
        let idle = now.saturating_sub(last_access);
        if idle < DECODED_CACHE_IDLE_ACCESSES {
            continue;
        }
        if bytes > 0 || cache.len() > DECODED_CACHE_MAX_ENTRIES {
            if cache.remove(&key).is_some() {
                total_bytes = total_bytes.saturating_sub(bytes);
                remove_cache_key(access, &key);
            }
        }
        if total_bytes <= DECODED_CACHE_BUDGET_BYTES && cache.len() <= DECODED_CACHE_MAX_ENTRIES {
            // Do not scan the remaining cold tail once both bounds are met;
            // keeping it would only make a later visible hit pay a decode.
            break;
        }
    }

    access.retain(|key, _| cache.contains_key(key));
}

#[inline]
fn image_decoded_bytes(state: &ImageCacheState) -> usize {
    match state {
        ImageCacheState::Ready(bytes, ..) => bytes.capacity(),
        ImageCacheState::Loaded(_, _, _, resource) => resource.rgba().len(),
        ImageCacheState::Loading | ImageCacheState::Error(..) => 0,
    }
}

fn new_retained_image_resource(
    upload_width: u32,
    upload_height: u32,
    intrinsic_width: u32,
    intrinsic_height: u32,
    rgba: Vec<u8>,
) -> Arc<ImageResource> {
    let texture_id =
        aimer_cupid::draw_cmd::DrawList::image_texture_id(&rgba, upload_width, upload_height);
    let revision =
        aimer_cupid::draw_cmd::DrawList::image_content_revision(&rgba, upload_width, upload_height);
    Arc::new(
        ImageResource::rgba8_with_intrinsic_size(
            texture_id,
            revision,
            upload_width,
            upload_height,
            intrinsic_width,
            intrinsic_height,
            Arc::from(rgba),
        )
        .expect("decoded RGBA image dimensions match its pixel payload"),
    )
}

fn materialize_retained_resource(state: &mut ImageCacheState) -> Option<Arc<ImageResource>> {
    let current = std::mem::replace(state, ImageCacheState::Loading);
    match current {
        ImageCacheState::Ready(bytes, upload_width, upload_height, width, height) => {
            let resource =
                new_retained_image_resource(upload_width, upload_height, width, height, bytes);
            let id = resource.texture_id();
            *state = ImageCacheState::Loaded(id, width, height, resource.clone());
            Some(resource)
        }
        ImageCacheState::Loaded(id, width, height, resource) => {
            *state = ImageCacheState::Loaded(id, width, height, resource.clone());
            Some(resource)
        }
        other => {
            *state = other;
            None
        }
    }
}

fn cached_retained_resource<K: Clone + Eq + Hash>(
    cache: &mut HashMap<K, ImageCacheState>,
    access: &mut HashMap<K, u64>,
    key: &K,
) -> Option<Arc<ImageResource>> {
    if let Some(resource) = cache.get(key).and_then(loaded_retained_resource) {
        return Some(resource);
    }
    if !cache.contains_key(key) {
        return None;
    }
    let now = touch_cache_key(access, key);
    let resource = cache.get_mut(key).and_then(materialize_retained_resource);
    prune_image_cache(cache, access, now);
    resource
}

fn loaded_retained_resource(state: &ImageCacheState) -> Option<Arc<ImageResource>> {
    match state {
        ImageCacheState::Loaded(_, _, _, resource) => Some(resource.clone()),
        ImageCacheState::Loading | ImageCacheState::Ready(..) | ImageCacheState::Error(..) => None,
    }
}

/// Loading entries are in-flight work. A loaded entry is protected while a
/// handle other than the cache's own exists: the retained render tree clones the
/// resource into the draw list it paints, and dropping the cache's copy of a
/// resource that is on screen forces the next re-created widget to fetch it
/// again and paint nothing until the fetch lands.
#[inline]
fn image_is_protected(state: &ImageCacheState) -> bool {
    match state {
        ImageCacheState::Loading => true,
        ImageCacheState::Loaded(_, _, _, resource) => Arc::strong_count(resource) > 1,
        ImageCacheState::Ready(..) | ImageCacheState::Error(..) => false,
    }
}

#[inline]
fn prune_image_cache<K: Clone + Eq + Hash>(
    cache: &mut HashMap<K, ImageCacheState>,
    access: &mut HashMap<K, u64>,
    now: u64,
) {
    prune_decoded_cache(cache, access, now, image_decoded_bytes, image_is_protected);
}

fn lookup_cache<K: Clone + Eq + Hash>(
    cache: &mut HashMap<K, ImageCacheState>,
    access: &mut HashMap<K, u64>,
    key: &K,
) -> CacheLookup {
    if !cache.contains_key(key) {
        let now = touch_cache_key(access, key);
        cache.insert(key.clone(), ImageCacheState::Loading);
        prune_image_cache(cache, access, now);
        return CacheLookup::StartLoad;
    }

    let now = touch_cache_key(access, key);
    prune_image_cache(cache, access, now);
    if matches!(cache.get(key), Some(ImageCacheState::Ready(..))) {
        let state = std::mem::replace(
            cache.get_mut(key).expect("cache entry was touched above"),
            ImageCacheState::Loading,
        );
        return match state {
            ImageCacheState::Ready(bytes, upload_width, upload_height, width, height) => {
                CacheLookup::Ready {
                    bytes,
                    upload_width,
                    upload_height,
                    width,
                    height,
                }
            }
            _ => unreachable!("cache entry changed while it was borrowed"),
        };
    }

    match cache.get(key).expect("cache entry was touched above") {
        ImageCacheState::Loaded(id, width, height, _) => CacheLookup::Loaded {
            id: *id,
            width: *width,
            height: *height,
        },
        ImageCacheState::Loading => CacheLookup::Loading,
        ImageCacheState::Error(error) => CacheLookup::Error(error.clone()),
        ImageCacheState::Ready(..) => unreachable!("ready cache entry was handled above"),
    }
}

#[inline]
fn set_cache_state<K: Clone + Eq + Hash>(
    cache: &mut HashMap<K, ImageCacheState>,
    access: &mut HashMap<K, u64>,
    key: K,
    state: ImageCacheState,
) {
    let now = touch_cache_key(access, &key);
    cache.insert(key, state);
    prune_image_cache(cache, access, now);
}

#[inline]
#[cfg(test)]
fn remove_cache_entry<K: Eq + Hash>(
    cache: &mut HashMap<K, ImageCacheState>,
    access: &mut HashMap<K, u64>,
    key: &K,
) {
    cache.remove(key);
    remove_cache_key(access, key);
}

impl ImageCaches {
    fn apply_update(&mut self, update: ImageCacheUpdate) {
        match update {
            ImageCacheUpdate::File { path, state } => {
                set_cache_state(&mut self.file, &mut self.file_access, path, state);
            }
            ImageCacheUpdate::Network { url, state } => {
                set_cache_state(&mut self.network, &mut self.network_access, url, state);
            }
            #[cfg(not(target_arch = "wasm32"))]
            ImageCacheUpdate::Asset { key, state } => {
                set_cache_state(&mut self.asset, &mut self.asset_access, key, state);
            }
        }
    }

    fn lookup_file(&mut self, path: &PathBuf) -> CacheLookup {
        lookup_cache(&mut self.file, &mut self.file_access, path)
    }

    fn retained_file_resource(&mut self, path: &PathBuf) -> Option<Arc<ImageResource>> {
        cached_retained_resource(&mut self.file, &mut self.file_access, path)
    }

    #[cfg(test)]
    fn remove_file(&mut self, path: &PathBuf) {
        remove_cache_entry(&mut self.file, &mut self.file_access, path);
    }

    fn set_file_loaded(
        &mut self,
        path: PathBuf,
        id: u32,
        width: u32,
        height: u32,
        resource: Arc<ImageResource>,
    ) {
        set_cache_state(
            &mut self.file,
            &mut self.file_access,
            path,
            ImageCacheState::Loaded(id, width, height, resource),
        );
    }

    fn lookup_network(&mut self, url: &String) -> CacheLookup {
        lookup_cache(&mut self.network, &mut self.network_access, url)
    }

    fn retained_network_resource(&mut self, url: &String) -> Option<Arc<ImageResource>> {
        cached_retained_resource(&mut self.network, &mut self.network_access, url)
    }

    fn set_network_loaded(
        &mut self,
        url: String,
        id: u32,
        width: u32,
        height: u32,
        resource: Arc<ImageResource>,
    ) {
        set_cache_state(
            &mut self.network,
            &mut self.network_access,
            url,
            ImageCacheState::Loaded(id, width, height, resource),
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn lookup_asset(&mut self, key: &String) -> CacheLookup {
        lookup_cache(&mut self.asset, &mut self.asset_access, key)
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn retained_asset_resource(&mut self, key: &String) -> Option<Arc<ImageResource>> {
        cached_retained_resource(&mut self.asset, &mut self.asset_access, key)
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn set_asset_loaded(
        &mut self,
        key: String,
        id: u32,
        width: u32,
        height: u32,
        resource: Arc<ImageResource>,
    ) {
        set_cache_state(
            &mut self.asset,
            &mut self.asset_access,
            key,
            ImageCacheState::Loaded(id, width, height, resource),
        );
    }
}

fn drain_cache_updates() {
    IMAGE_CACHES.with(|caches| {
        let mut caches = caches.borrow_mut();
        loop {
            match IMAGE_CACHE_MAILBOX.receiver.try_recv() {
                Ok(update) => caches.apply_update(update),
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            }
        }
    });
}

#[inline]
fn send_cache_update(update: ImageCacheUpdate) {
    if IMAGE_CACHE_MAILBOX.sender.send(update).is_err() {
        error!("Image cache update channel is disconnected");
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn schedule_native_image_load<F, U>(work: F, update: U, window: WindowHandle) -> bool
where
    F: FnOnce() -> ImageCacheState + Send + 'static,
    U: FnOnce(ImageCacheState) -> ImageCacheUpdate + Send + 'static,
{
    let Some(venus) = Venus::current() else {
        return false;
    };
    let worker_runtime = venus.clone();

    venus.spawn(async move {
        let state = worker_runtime.spawn_blocking(work).await;
        send_cache_update(update(state));
        window.request_animation_frame();
    });
    true
}

#[cfg(not(target_arch = "wasm32"))]
fn venus_unavailable() -> String {
    "Venus runtime is unavailable".to_owned()
}

#[allow(dead_code)]
const BROWSER_IMAGE_MAX_DIMENSION: u32 = 2048;
#[allow(dead_code)]
fn constrained_browser_image_size(width: u32, height: u32) -> (u32, u32) {
    if width <= BROWSER_IMAGE_MAX_DIMENSION && height <= BROWSER_IMAGE_MAX_DIMENSION {
        return (width, height);
    }

    if width >= height {
        (
            BROWSER_IMAGE_MAX_DIMENSION,
            ((height as u64 * BROWSER_IMAGE_MAX_DIMENSION as u64) / width as u64).max(1) as u32,
        )
    } else {
        (
            ((width as u64 * BROWSER_IMAGE_MAX_DIMENSION as u64) / height as u64).max(1) as u32,
            BROWSER_IMAGE_MAX_DIMENSION,
        )
    }
}
#[allow(dead_code)]
fn image_mime_type(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
        "image/png"
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "image/jpeg"
    } else if bytes.starts_with(&[0x47, 0x49, 0x46]) {
        "image/gif"
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        "image/webp"
    } else {
        "image/png"
    }
}

///
/// Represents the source of an image, which can either be identified by an ID,
/// a file path, or a URL.
///
/// # Variants
///
/// * `Id(u32)` - Specifies the image source using a unique numerical
///   identifier.
///   - `u32`: The unique identifier for the image.
///
/// * `File(String)` - Specifies the image source using a file path.
///   - `String`: The file path to the image as a UTF-8 encoded string.
///
/// * `Network(String)` - Specifies the image source using a URL.
///   - `String`: The URL of the image.
///
/// # Traits Derived
///
/// The `ImageSource` enum derives the following traits:
///
/// * `Clone` - Enables producing a copy of an `ImageSource`.
/// * `Debug` - Facilitates formatting and debugging output.
/// * `PartialEq` - Allows comparison of `ImageSource` instances for equality.
///
/// # Example
/// ```rust ignore
/// use your_crate::ImageSource;
///
/// let img_by_id = ImageSource::Id(123);
/// let img_by_file = ImageSource::File(String::from("path/to/file.png"));
/// let img_by_url = ImageSource::Network(String::from("https://example.com/image.png"));
///
/// match img_by_id {
///     ImageSource::Id(id) => println!("Image ID: {}", id),
///     ImageSource::File(path) => println!("Image Path: {}", path),
///     ImageSource::Network(url) => println!("Image URL: {}", url),
/// }
/// ```
#[derive(Clone, Debug, PartialEq)]
pub enum ImageSource {
    Id(u32),
    /// A bundled asset registered under `[assets]` in `aimer.toml`. The string
    /// is the path declared there (relative to the project root) and is used
    /// verbatim as the per-platform lookup key.
    Asset(String),
    File(PathBuf),
    Network(String),
    NetworkWithHeaders(String, HashMap<String, String>),
}

impl ImageProvider for ImageSource {
    fn get_image(&self, ctx: &BuildContext) -> ImageResult {
        match self {
            ImageSource::Id(id) => Success(*id),
            ImageSource::Asset(key) => Self::load_asset_image(ctx, key),
            ImageSource::File(path) => Self::load_image(ctx, path),
            ImageSource::Network(url) => Self::load_network_image(ctx, url),
            ImageSource::NetworkWithHeaders(url, headers) => {
                Self::load_network_image_with_headers(ctx, url, headers)
            }
        }
    }

    fn retained_image_resource(&self, _ctx: &BuildContext) -> Option<Arc<ImageResource>> {
        let cached = IMAGE_CACHES.with(|caches| {
            let caches = caches.borrow();
            match self {
                ImageSource::Id(_) => None,
                ImageSource::Asset(key) => {
                    #[cfg(not(target_arch = "wasm32"))]
                    {
                        caches.asset.get(key).and_then(loaded_retained_resource)
                    }
                    #[cfg(target_arch = "wasm32")]
                    {
                        let url = if key.starts_with('/') {
                            key.clone()
                        } else {
                            format!("/{key}")
                        };
                        caches.network.get(&url).and_then(loaded_retained_resource)
                    }
                }
                ImageSource::File(path) => caches.file.get(path).and_then(loaded_retained_resource),
                ImageSource::Network(url) | ImageSource::NetworkWithHeaders(url, _) => {
                    caches.network.get(url).and_then(loaded_retained_resource)
                }
            }
        });
        if cached.is_some() || matches!(self, ImageSource::Id(_)) {
            return cached;
        }

        drain_cache_updates();
        match self {
            ImageSource::Id(_) => None,
            ImageSource::Asset(key) => {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    IMAGE_CACHES.with(|caches| caches.borrow_mut().retained_asset_resource(key))
                }
                #[cfg(target_arch = "wasm32")]
                {
                    let url = if key.starts_with('/') {
                        key.clone()
                    } else {
                        format!("/{key}")
                    };
                    IMAGE_CACHES.with(|caches| caches.borrow_mut().retained_network_resource(&url))
                }
            }
            ImageSource::File(path) => {
                IMAGE_CACHES.with(|caches| caches.borrow_mut().retained_file_resource(path))
            }
            ImageSource::Network(url) | ImageSource::NetworkWithHeaders(url, _) => {
                IMAGE_CACHES.with(|caches| caches.borrow_mut().retained_network_resource(url))
            }
        }
    }
}

impl ImageSource {
    pub fn load_image(ctx: &BuildContext, path: &PathBuf) -> ImageResult {
        drain_cache_updates();

        match IMAGE_CACHES.with(|caches| caches.borrow_mut().lookup_file(path)) {
            CacheLookup::Loaded { id, width, height } => {
                ctx.canvas.set_texture_size(id, width, height);
                Success(id)
            }
            CacheLookup::Ready {
                bytes,
                upload_width,
                upload_height,
                width,
                height,
            } => {
                let resource =
                    new_retained_image_resource(upload_width, upload_height, width, height, bytes);
                let id = resource.texture_id();
                ctx.canvas.set_texture_size(id, width, height);
                IMAGE_CACHES.with(|caches| {
                    caches
                        .borrow_mut()
                        .set_file_loaded(path.clone(), id, width, height, resource)
                });
                Success(id)
            }
            CacheLookup::Loading => ImageResult::Loading,
            CacheLookup::Error(error) => ImageResult::Error(error),
            CacheLookup::StartLoad => {
                // Cache miss: decode off the render thread so scrolling a large
                // image into view does not block the frame for hundreds of
                // milliseconds.
                #[cfg(not(target_arch = "wasm32"))]
                {
                    let task_path = path.clone();
                    let update_path = path.clone();
                    let error_path = path.clone();
                    let window = ctx.window.clone();
                    let error_window = window.clone();
                    if !schedule_native_image_load(
                        move || match image::open(&task_path) {
                            Ok(image) => {
                                let rgba = image.to_rgba8();
                                let (width, height) = (rgba.width(), rgba.height());
                                ImageCacheState::Ready(
                                    rgba.into_raw(),
                                    width,
                                    height,
                                    width,
                                    height,
                                )
                            }
                            Err(_) => ImageCacheState::Error("Failed to load image".into()),
                        },
                        move |state| ImageCacheUpdate::File {
                            path: update_path,
                            state,
                        },
                        window,
                    ) {
                        let error = venus_unavailable();
                        send_cache_update(ImageCacheUpdate::File {
                            path: error_path,
                            state: ImageCacheState::Error(error.clone()),
                        });
                        error_window.request_animation_frame();
                        return ImageResult::Error(error);
                    }
                }

                // wasm: fetch and decode asynchronously via the browser's
                // native decoder (much faster than the Rust `image` crate).
                #[cfg(target_arch = "wasm32")]
                {
                    let url = path.to_string_lossy().to_string();
                    let path_buf = path.clone();
                    let window = ctx.window.clone();
                    wasm_bindgen_futures::spawn_local(async move {
                        let state = match Self::fetch_bytes(&url).await {
                            Ok(bytes) => match Self::decode_image_browser(&bytes).await {
                                Ok((rgba, upload_w, upload_h, w, h)) => {
                                    ImageCacheState::Ready(rgba, upload_w, upload_h, w, h)
                                }
                                Err(error) => ImageCacheState::Error(error),
                            },
                            Err(error) => ImageCacheState::Error(error),
                        };
                        send_cache_update(ImageCacheUpdate::File {
                            path: path_buf,
                            state,
                        });
                        window.request_animation_frame();
                    });
                }

                ImageResult::Loading
            }
        }
    }

    pub fn load_network_image(ctx: &BuildContext, url: &str) -> ImageResult {
        Self::load_network_image_with_headers(ctx, url, &HashMap::new())
    }

    /// Load a bundled asset by its registered key.
    ///
    /// On native targets the bytes are read from the platform's asset store
    /// (Android `AssetManager`, or the app bundle / project dir on desktop &
    /// iOS/macOS), decoded off the UI thread, retained as RGBA pixels and
    /// uploaded by the renderer on first draw.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn load_asset_image(ctx: &BuildContext, key: &str) -> ImageResult {
        let key_owned = key.to_string();
        drain_cache_updates();

        match IMAGE_CACHES.with(|caches| caches.borrow_mut().lookup_asset(&key_owned)) {
            CacheLookup::Loaded { id, width, height } => {
                ctx.canvas.set_texture_size(id, width, height);
                Success(id)
            }
            CacheLookup::Ready {
                bytes,
                upload_width,
                upload_height,
                width,
                height,
            } => {
                let resource =
                    new_retained_image_resource(upload_width, upload_height, width, height, bytes);
                let id = resource.texture_id();
                ctx.canvas.set_texture_size(id, width, height);
                IMAGE_CACHES.with(|caches| {
                    caches.borrow_mut().set_asset_loaded(
                        key_owned.clone(),
                        id,
                        width,
                        height,
                        resource,
                    )
                });
                Success(id)
            }
            CacheLookup::Loading => ImageResult::Loading,
            CacheLookup::Error(error) => ImageResult::Error(error),
            CacheLookup::StartLoad => {
                // Cache miss: read + decode the asset off the render thread so
                // scrolling it into view does not block the frame.
                let task_key = key_owned.clone();
                let update_key = key_owned.clone();
                let error_key = key_owned;
                let window = ctx.window.clone();
                let error_window = window.clone();
                if !schedule_native_image_load(
                    move || match Self::load_asset_bytes(&task_key) {
                        Ok(bytes) => match image::load_from_memory(&bytes) {
                            Ok(image) => {
                                let rgba = image.to_rgba8();
                                let (width, height) = (rgba.width(), rgba.height());
                                ImageCacheState::Ready(
                                    rgba.into_raw(),
                                    width,
                                    height,
                                    width,
                                    height,
                                )
                            }
                            Err(_) => ImageCacheState::Error(format!(
                                "Failed to decode asset image '{task_key}'"
                            )),
                        },
                        Err(error) => ImageCacheState::Error(error),
                    },
                    move |state| ImageCacheUpdate::Asset {
                        key: update_key,
                        state,
                    },
                    window,
                ) {
                    let error = venus_unavailable();
                    send_cache_update(ImageCacheUpdate::Asset {
                        key: error_key,
                        state: ImageCacheState::Error(error.clone()),
                    });
                    error_window.request_animation_frame();
                    return ImageResult::Error(error);
                }
                ImageResult::Loading
            }
        }
    }

    /// Load a bundled asset on web.
    ///
    /// Assets are served from the site root (Vite `public/`), so they are
    /// fetched asynchronously through the same machinery as network images.
    #[cfg(target_arch = "wasm32")]
    pub fn load_asset_image(ctx: &BuildContext, key: &str) -> ImageResult {
        let url = if key.starts_with('/') {
            key.to_string()
        } else {
            format!("/{key}")
        };
        Self::load_network_image(ctx, &url)
    }

    /// Read the raw bytes of a bundled asset from the platform's asset store.
    #[cfg(not(target_arch = "wasm32"))]
    fn load_asset_bytes(key: &str) -> Result<Vec<u8>, String> {
        #[cfg(target_os = "android")]
        {
            use std::ffi::CString;
            use std::io::Read;

            let app = aimer_events::android_app::get_android_app()
                .ok_or("Android app handle not available")?;
            let manager = app.asset_manager();
            let cstr = CString::new(key).map_err(|e| format!("invalid asset key '{key}': {e}"))?;
            let mut asset = manager
                .open(&cstr)
                .ok_or_else(|| format!("asset '{key}' not found in APK"))?;
            let mut buffer = Vec::new();
            asset
                .read_to_end(&mut buffer)
                .map_err(|e| format!("failed to read asset '{key}': {e}"))?;
            Ok(buffer)
        }
        #[cfg(not(target_os = "android"))]
        {
            for path in Self::asset_candidate_paths(key) {
                if let Ok(bytes) = std::fs::read(&path) {
                    return Ok(bytes);
                }
            }
            Err(format!("asset '{key}' not found"))
        }
    }

    /// Candidate filesystem locations for a bundled asset on desktop and
    /// iOS/macOS, tried in order: the project directory (dev runs), then the
    /// app bundle's resource directory (packaged apps).
    #[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
    fn asset_candidate_paths(key: &str) -> Vec<PathBuf> {
        let mut paths = vec![PathBuf::from(key)];
        if let Ok(exe) = std::env::current_exe()
            && let Some(exe_dir) = exe.parent()
        {
            // macOS: <App>.app/Contents/MacOS/<exe> -> <App>.app/Contents/Resources
            if let Some(contents) = exe_dir.parent() {
                paths.push(contents.join("Resources").join(key));
            }
            // iOS: <App>.app/<exe> -> <App>.app/<key>
            paths.push(exe_dir.join(key));
        }
        paths
    }

    pub fn load_network_image_with_headers(
        ctx: &BuildContext,
        url: &str,
        headers: &HashMap<String, String>,
    ) -> ImageResult {
        drain_cache_updates();
        let url_owned = url.to_string();

        match IMAGE_CACHES.with(|caches| caches.borrow_mut().lookup_network(&url_owned)) {
            CacheLookup::Loaded { id, width, height } => {
                ctx.canvas.set_texture_size(id, width, height);
                Success(id)
            }
            CacheLookup::Ready {
                bytes,
                upload_width,
                upload_height,
                width,
                height,
            } => {
                let resource =
                    new_retained_image_resource(upload_width, upload_height, width, height, bytes);
                let id = resource.texture_id();
                ctx.canvas.set_texture_size(id, width, height);
                IMAGE_CACHES.with(|caches| {
                    caches.borrow_mut().set_network_loaded(
                        url_owned.clone(),
                        id,
                        width,
                        height,
                        resource,
                    )
                });
                Success(id)
            }
            CacheLookup::Loading => ImageResult::Loading,
            CacheLookup::Error(error) => ImageResult::Error(error),
            CacheLookup::StartLoad => {
                let headers = headers.clone();
                let window = ctx.window.clone();

                #[cfg(not(target_arch = "wasm32"))]
                {
                    let task_url = url_owned.clone();
                    let update_url = url_owned.clone();
                    let error_url = url_owned;
                    let error_window = window.clone();
                    if !schedule_native_image_load(
                        move || match Self::fetch_full_image_with_headers(&task_url, &headers) {
                            Ok(state) => state,
                            Err(error) => {
                                error!("Error to fetch network image : {}", error);
                                ImageCacheState::Error(error)
                            }
                        },
                        move |state| ImageCacheUpdate::Network {
                            url: update_url,
                            state,
                        },
                        window,
                    ) {
                        let error = venus_unavailable();
                        send_cache_update(ImageCacheUpdate::Network {
                            url: error_url,
                            state: ImageCacheState::Error(error.clone()),
                        });
                        error_window.request_animation_frame();
                        return ImageResult::Error(error);
                    }
                }

                #[cfg(target_arch = "wasm32")]
                {
                    wasm_bindgen_futures::spawn_local(async move {
                        let state = match Self::fetch_full_image_with_headers(&url_owned, &headers)
                            .await
                        {
                            Ok(state) => state,
                            Err(error) => {
                                error!("Failed to fetch network image ({}): {}", url_owned, error);
                                ImageCacheState::Error(error)
                            }
                        };
                        send_cache_update(ImageCacheUpdate::Network {
                            url: url_owned,
                            state,
                        });
                        window.request_animation_frame();
                    });
                }

                ImageResult::Loading
            }
        }
    }

    #[cfg(target_arch = "wasm32")]
    #[allow(dead_code)]
    async fn fetch_full_image(url: &str, window: WindowHandle) -> Result<(), String> {
        let state = Self::fetch_full_image_with_headers(url, &HashMap::new()).await?;
        send_cache_update(ImageCacheUpdate::Network {
            url: url.to_string(),
            state,
        });
        window.request_animation_frame();
        Ok(())
    }

    #[cfg(target_arch = "wasm32")]
    async fn fetch_full_image_with_headers(
        url: &str,
        maps: &HashMap<String, String>,
    ) -> Result<ImageCacheState, String> {
        let bytes = if maps.is_empty() {
            Self::fetch_bytes(url).await?
        } else {
            Self::fetch_bytes_with_headers(url, maps).await?
        };

        let (rgba, upload_width, upload_height, width, height) =
            Self::decode_image_browser(&bytes).await?;

        Ok(ImageCacheState::Ready(
            rgba,
            upload_width,
            upload_height,
            width,
            height,
        ))
    }

    /// Fetch raw bytes from a URL using the browser's `fetch` API.
    #[cfg(target_arch = "wasm32")]
    async fn fetch_bytes(url: &str) -> Result<Vec<u8>, String> {
        use wasm_bindgen::JsCast;

        let web_window = web_sys::window().ok_or("No window found")?;
        let resp_value = wasm_bindgen_futures::JsFuture::from(web_window.fetch_with_str(url))
            .await
            .map_err(|e| format!("{:?}", e))?;
        let resp: web_sys::Response = resp_value.dyn_into().map_err(|e| format!("{:?}", e))?;
        if !resp.ok() {
            return Err(format!("HTTP error: {}", resp.status()));
        }
        let buf = wasm_bindgen_futures::JsFuture::from(
            resp.array_buffer().map_err(|e| format!("{:?}", e))?,
        )
        .await
        .map_err(|e| format!("{:?}", e))?;
        Ok(js_sys::Uint8Array::new(&buf).to_vec())
    }

    /// Fetch raw bytes with custom headers.
    #[cfg(target_arch = "wasm32")]
    async fn fetch_bytes_with_headers(
        url: &str,
        headers: &HashMap<String, String>,
    ) -> Result<Vec<u8>, String> {
        use wasm_bindgen::JsCast;
        use wasm_bindgen::prelude::*;
        use web_sys::Headers;

        let js_headers = Headers::new().map_err(|e| format!("{:?}", e))?;
        for (key, value) in headers {
            js_headers
                .append(key, value)
                .map_err(|e| format!("{:?}", e))?;
        }
        let web_window = web_sys::window().ok_or("No window found")?;
        let request_init = web_sys::RequestInit::new();
        request_init.set_method("GET");
        request_init.set_headers(&JsValue::from(js_headers));

        let resp_value = wasm_bindgen_futures::JsFuture::from(
            web_window.fetch_with_str_and_init(url, &request_init),
        )
        .await
        .map_err(|e| format!("{:?}", e))?;
        let resp: web_sys::Response = resp_value.dyn_into().map_err(|e| format!("{:?}", e))?;
        if !resp.ok() {
            return Err(format!("HTTP error: {}", resp.status()));
        }
        let buf = wasm_bindgen_futures::JsFuture::from(
            resp.array_buffer().map_err(|e| format!("{:?}", e))?,
        )
        .await
        .map_err(|e| format!("{:?}", e))?;
        Ok(js_sys::Uint8Array::new(&buf).to_vec())
    }

    /// Decode raw image bytes (PNG/JPEG/WebP/GIF) to RGBA using the browser's
    /// native decoder via `createImageBitmap` + `OffscreenCanvas`.
    #[cfg(target_arch = "wasm32")]
    async fn decode_image_browser(bytes: &[u8]) -> Result<(Vec<u8>, u32, u32, u32, u32), String> {
        use wasm_bindgen::JsCast;

        let blob_parts = js_sys::Array::new();
        blob_parts.push(&js_sys::Uint8Array::from(bytes));
        let blob_opts = web_sys::BlobPropertyBag::new();
        blob_opts.set_type(image_mime_type(bytes));
        let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&blob_parts, &blob_opts)
            .map_err(|e| format!("Blob creation failed: {:?}", e))?;

        // createImageBitmap asks the browser to decode asynchronously, rather
        // than completing HtmlImageElement decoding on the event loop.
        let window = web_sys::window().ok_or("No window found")?;
        let bitmap = wasm_bindgen_futures::JsFuture::from(
            window
                .create_image_bitmap_with_blob(&blob)
                .map_err(|e| format!("Image bitmap creation failed: {:?}", e))?,
        )
        .await
        .map_err(|e| format!("Image decode failed: {:?}", e))?
        .dyn_into::<web_sys::ImageBitmap>()
        .map_err(|e| format!("Image bitmap cast failed: {:?}", e))?;
        let w = bitmap.width();
        let h = bitmap.height();
        let (upload_width, upload_height) = constrained_browser_image_size(w, h);

        let bitmap = if (upload_width, upload_height) == (w, h) {
            bitmap
        } else {
            let options = web_sys::ImageBitmapOptions::new();
            options.set_resize_width(upload_width);
            options.set_resize_height(upload_height);
            options.set_resize_quality(web_sys::ResizeQuality::High);
            let resized = wasm_bindgen_futures::JsFuture::from(
                window
                    .create_image_bitmap_with_image_bitmap_and_image_bitmap_options(
                        &bitmap, &options,
                    )
                    .map_err(|e| format!("Image bitmap resize failed: {:?}", e))?,
            )
            .await
            .map_err(|e| format!("Image bitmap resize failed: {:?}", e))?
            .dyn_into::<web_sys::ImageBitmap>()
            .map_err(|e| format!("Resized image bitmap cast failed: {:?}", e))?;
            bitmap.close();
            resized
        };

        // Let the browser scale before pixel readback. This avoids copying the
        // full decoded image into WASM only to resize it again on the render path.
        let canvas = web_sys::OffscreenCanvas::new(upload_width, upload_height)
            .map_err(|e| format!("OffscreenCanvas creation failed: {:?}", e))?;
        let ctx = canvas
            .get_context("2d")
            .map_err(|e| format!("get_context failed: {:?}", e))?
            .ok_or("No 2d context")?
            .dyn_into::<web_sys::OffscreenCanvasRenderingContext2d>()
            .map_err(|e| format!("Context cast failed: {:?}", e))?;
        ctx.draw_image_with_image_bitmap(&bitmap, 0.0, 0.0)
            .map_err(|e| format!("drawImage failed: {:?}", e))?;
        #[allow(unexpected_cfgs)]
        #[cfg(web_sys_unstable_apis)]
        let image_data = ctx
            .get_image_data(0, 0, upload_width as i32, upload_height as i32)
            .map_err(|e| format!("getImageData failed: {:?}", e))?;
        #[allow(unexpected_cfgs)]
        #[cfg(not(web_sys_unstable_apis))]
        let image_data = ctx
            .get_image_data(0.0, 0.0, upload_width as f64, upload_height as f64)
            .map_err(|e| format!("getImageData failed: {:?}", e))?;
        let rgba = image_data.data().to_vec();

        bitmap.close();
        Ok((rgba, upload_width, upload_height, w, h))
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn fetch_full_image_with_headers(
        url: &str,
        headers: &HashMap<String, String>,
    ) -> Result<ImageCacheState, String> {
        let mut request_builder = ureq::get(url).header("User-Agent", "aimer/0.1.0");
        for (key, value) in headers {
            request_builder = request_builder.header(key, value);
        }

        let mut response = match request_builder.call() {
            Ok(response) => response,
            Err(ureq::Error::StatusCode(status)) => {
                return Err(format!("HTTP error: {status}"));
            }
            Err(error) => return Err(format!("Network Error: {error}")),
        };

        if !response.status().is_success() {
            return Err(format!("HTTP error: {}", response.status()));
        }

        let mut all_bytes = Vec::new();
        response
            .body_mut()
            .as_reader()
            .read_to_end(&mut all_bytes)
            .map_err(|error| format!("Failed to download bytes: {error}"))?;

        match image::load_from_memory(&all_bytes) {
            Ok(image) => {
                let image = image.into_rgba8();
                let width = image.width();
                let height = image.height();
                let rgba_bytes = image.into_raw();

                Ok(ImageCacheState::Ready(
                    rgba_bytes, width, height, width, height,
                ))
            }
            Err(_) => {
                // error!("Failed to decode image: {}", e);
                Err("Failed to decode image".into())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use super::{
        DECODED_CACHE_BUDGET_BYTES, DECODED_CACHE_IDLE_ACCESSES, IMAGE_CACHES, ImageCacheState,
        ImageCacheUpdate, ImageCaches, constrained_browser_image_size, drain_cache_updates,
        image_mime_type, new_retained_image_resource, prune_decoded_cache, send_cache_update,
    };

    #[cfg(not(target_arch = "wasm32"))]
    fn context() -> aimer_widget::base::BuildContext<'static> {
        use aimer_attribute::{BoxConstraint, Vec2d};
        use aimer_canvas::{FrameCanvas, InnerCanvas};
        use aimer_widget::base::{BuildContext, ResolvedSize, WindowHandle};

        static RUNTIME: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
        let runtime = RUNTIME.get_or_init(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("asset test runtime should build")
        });
        let _guard = runtime.enter();
        let inner = Box::leak(Box::new(InnerCanvas::new()));
        let mut context = BuildContext::new(
            FrameCanvas::new(inner),
            ResolvedSize {
                width: 32.0,
                height: 32.0,
            },
            1.0,
            Vec2d::default(),
            Vec2d::default(),
            WindowHandle::headless(Default::default(), 1.0),
            tokio::runtime::Handle::current(),
        );
        context.box_constraint = BoxConstraint {
            min_width: 0.0,
            min_height: 0.0,
            max_width: 32.0,
            max_height: 32.0,
        };
        context
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn native_network_load_reports_missing_venus_without_starting_work() {
        use aimer_venus::Venus;
        let context = context();

        Venus::uninstall();
        let url = "http://127.0.0.1:9/aimer-assets-no-venus.png";
        let result = super::ImageSource::load_network_image(&context, url);

        assert!(matches!(
            result,
            crate::ImageResult::Error(error) if error.contains("Venus runtime is unavailable")
        ));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn native_network_load_uses_ureq_and_forwards_headers() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::sync::{Arc, Mutex};
        use std::time::Duration;

        use aimer_venus::Venus;
        use image::{DynamicImage, ImageFormat, Rgba, RgbaImage};

        let mut image_bytes = std::io::Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(RgbaImage::from_pixel(1, 1, Rgba([255, 0, 0, 255])))
            .write_to(&mut image_bytes, ImageFormat::Png)
            .unwrap();
        let image_bytes = image_bytes.into_inner();
        let request_bytes = Arc::new(Mutex::new(Vec::new()));
        let request_copy = request_bytes.clone();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 4096];
            let length = stream.read(&mut request).unwrap();
            request_copy
                .lock()
                .unwrap()
                .extend_from_slice(&request[..length]);
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                image_bytes.len()
            )
            .unwrap();
            stream.write_all(&image_bytes).unwrap();
        });

        let context = context();
        let venus = Venus::new();
        venus.install();
        let url = format!("http://{address}/image.png");
        let mut headers = HashMap::new();
        headers.insert("X-Aimer-Test".to_owned(), "venus".to_owned());
        let mut result =
            super::ImageSource::load_network_image_with_headers(&context, &url, &headers);
        for _ in 0..500 {
            if matches!(result, crate::ImageResult::Success(_)) {
                break;
            }
            venus.run_microtasks();
            result = super::ImageSource::load_network_image_with_headers(&context, &url, &headers);
            std::thread::sleep(Duration::from_millis(1));
        }
        Venus::uninstall();
        server.join().unwrap();

        assert!(matches!(result, crate::ImageResult::Success(_)));
        let request = request_bytes.lock().unwrap();
        assert!(
            request
                .windows(b"X-Aimer-Test: venus".len())
                .any(|window| window.eq_ignore_ascii_case(b"X-Aimer-Test: venus"))
        );
    }

    #[test]
    fn detects_browser_supported_image_mime_types() {
        assert_eq!(image_mime_type(b"\x89PNG\r\n\x1a\n"), "image/png");
        assert_eq!(image_mime_type(b"\xff\xd8\xff\xe0"), "image/jpeg");
        assert_eq!(image_mime_type(b"GIF89a"), "image/gif");
        assert_eq!(image_mime_type(b"RIFF\x04\x00\x00\x00WEBP"), "image/webp");
    }

    #[test]
    fn unknown_image_data_uses_browser_decode_fallback() {
        assert_eq!(image_mime_type(b"unknown"), "image/png");
    }

    #[test]
    fn browser_decode_constrains_oversized_images_before_pixel_readback() {
        assert_eq!(constrained_browser_image_size(4096, 2048), (2048, 1024));
        assert_eq!(constrained_browser_image_size(2048, 4096), (1024, 2048));
        assert_eq!(constrained_browser_image_size(1024, 512), (1024, 512));
    }

    #[test]
    fn decoded_cache_prunes_cold_entries_but_keeps_visible_and_loading_work() {
        struct Entry {
            bytes: usize,
            loading: bool,
        }

        let now = DECODED_CACHE_IDLE_ACCESSES + 1;
        let mut cache = HashMap::from([
            (
                "cold",
                Entry {
                    bytes: DECODED_CACHE_BUDGET_BYTES,
                    loading: false,
                },
            ),
            (
                "visible",
                Entry {
                    bytes: DECODED_CACHE_BUDGET_BYTES,
                    loading: false,
                },
            ),
            (
                "loading",
                Entry {
                    bytes: DECODED_CACHE_BUDGET_BYTES,
                    loading: true,
                },
            ),
        ]);
        let mut access = HashMap::from([("cold", 0), ("visible", now), ("loading", 0)]);

        prune_decoded_cache(
            &mut cache,
            &mut access,
            now,
            |entry| entry.bytes,
            |entry| entry.loading,
        );

        assert!(!cache.contains_key("cold"));
        assert!(cache.contains_key("visible"));
        assert!(cache.contains_key("loading"));
        assert!(!access.contains_key("cold"));
    }

    /// Other images being looked up must not evict a texture the retained
    /// render tree still paints. The retained paint path reads a settled image
    /// without touching its access tick, so a stable visible avatar goes idle
    /// while the rest of the page keeps looking things up.
    #[test]
    fn painted_network_resource_survives_idle_pressure_from_other_images() {
        let url = "https://example.invalid/visible-avatar.png".to_string();
        let mut caches = ImageCaches::default();
        let resource = new_retained_image_resource(1, 1, 1, 1, vec![255; 4]);
        let id = resource.texture_id();
        // The retained render tree keeps its own handle to what it paints.
        let render_tree_handle = std::sync::Arc::clone(&resource);
        caches.set_network_loaded(url.clone(), id, 1, 1, resource);

        for index in 0..=DECODED_CACHE_IDLE_ACCESSES * 2 {
            caches.lookup_network(&format!("https://example.invalid/other-{index}.png"));
        }

        assert!(
            caches.retained_network_resource(&url).is_some(),
            "a resource held by the render tree was evicted while visible"
        );
        drop(render_tree_handle);
    }

    /// The guard above must not turn the cache into a leak: once nothing paints
    /// a resource, an idle entry is still released.
    #[test]
    fn unreferenced_network_resource_is_still_evicted_when_idle() {
        let url = "https://example.invalid/offscreen-avatar.png".to_string();
        let mut caches = ImageCaches::default();
        let resource = new_retained_image_resource(1, 1, 1, 1, vec![255; 4]);
        let id = resource.texture_id();
        caches.set_network_loaded(url.clone(), id, 1, 1, resource);

        for index in 0..=DECODED_CACHE_IDLE_ACCESSES * 2 {
            caches.lookup_network(&format!("https://example.invalid/other-{index}.png"));
        }

        assert!(caches.retained_network_resource(&url).is_none());
    }

    #[test]
    fn background_decode_updates_are_delivered_through_the_channel() {
        let path = PathBuf::from("__aimer_assets_channel_test__.png");
        IMAGE_CACHES.with(|caches| {
            caches.borrow_mut().remove_file(&path);
        });

        let update_path = path.clone();
        std::thread::spawn(move || {
            send_cache_update(ImageCacheUpdate::File {
                path: update_path,
                state: ImageCacheState::Ready(vec![1, 2, 3, 4], 1, 1, 1, 1),
            });
        })
        .join()
        .unwrap();

        IMAGE_CACHES.with(|caches| {
            assert!(!caches.borrow().file.contains_key(&path));
        });
        drain_cache_updates();
        IMAGE_CACHES.with(|caches| {
            assert!(matches!(
                caches.borrow().file.get(&path),
                Some(ImageCacheState::Ready(bytes, 1, 1, 1, 1)) if bytes == &[1, 2, 3, 4]
            ));
            caches.borrow_mut().remove_file(&path);
        });
    }
}
