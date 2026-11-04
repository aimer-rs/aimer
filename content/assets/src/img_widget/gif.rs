//! Animated GIF playback for the image widgets.
//!
//! The static image caches keep one texture per image, so a GIF is shown by
//! them as its first frame. Playback is layered on top of that:
//!
//! * the loaders remember the encoded bytes of every GIF they decode
//!   ([`register`]), keyed by source;
//! * a widget whose image settled looks those bytes up once and starts a
//!   [`GifPlayer`];
//! * the player is fed by a decoder that runs on its own thread and hands
//!   composited frames over a queue of [`FRAME_WINDOW`] entries. A full queue
//!   blocks the decoder, so only a few frames are ever decoded ahead, a paused
//!   animation decodes nothing, and dropping the player stops the thread.
//!
//! Frame timing is driven by the widget's per-frame sync, never by a timer: the
//! player accumulates the time between syncs and swaps the frame once the
//! current frame's delay is used up.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::io::Cursor;
use std::num::NonZeroU32;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use aimer_cupid::draw_cmd::ImageResource;
use aimer_utils::AnimInstant;
use aimer_widget::base::{BuildContext, WindowHandle};
use image::codecs::gif::GifDecoder;
use image::metadata::LoopCount;
use image::{AnimationDecoder, Frames, ImageDecoder};

use crate::img_widget::source::constrained_image_size;

/// How many decoded frames may wait for the renderer.
const FRAME_WINDOW: usize = 3;

/// Browsers show a GIF frame that asks for no more than this for
/// [`DEFAULT_FRAME_DELAY`] instead; encoders write such delays when they mean
/// "as fast as possible", which would otherwise spin the frame loop.
const MIN_FRAME_DELAY: Duration = Duration::from_millis(10);
const DEFAULT_FRAME_DELAY: Duration = Duration::from_millis(100);

/// The largest canvas, in pixels, a GIF may declare. A corrupt or hostile
/// header can claim gigabytes of canvas; the decoder would allocate all of it
/// before reading a single frame. 16 megapixels is a 4096 x 4096 canvas, far
/// beyond any real animation.
const MAX_CANVAS_PIXELS: u64 = 16 * 1024 * 1024;

const NO_SEEK: usize = usize::MAX;

#[derive(Debug)]
struct ControllerState {
    playing: AtomicBool,
    seek: AtomicUsize,
    frame: AtomicUsize,
    frame_count: AtomicUsize,
    finished: AtomicBool,
    waker: Mutex<Option<WindowHandle>>,
}

impl ControllerState {
    fn new(playing: bool) -> Self {
        Self {
            playing: AtomicBool::new(playing),
            seek: AtomicUsize::new(NO_SEEK),
            frame: AtomicUsize::new(0),
            frame_count: AtomicUsize::new(0),
            finished: AtomicBool::new(false),
            waker: Mutex::new(None),
        }
    }

    /// Asks the window the animation is shown in for a frame, so that a change
    /// made from an event handler, a timer or another thread is noticed even
    /// when the animation had stopped asking for frames.
    fn wake(&self) {
        if let Ok(waker) = self.waker.lock()
            && let Some(window) = waker.as_ref()
        {
            window.request_animation_frame();
        }
    }
}

/// A handle that plays, pauses and seeks the GIF shown by an image widget.
///
/// Clones share one state. Hand one clone to the image through
/// [`Image::gif_controller`](crate::Image::gif_controller) (or the matching
/// method of the other image widgets) and keep another to drive it. Calls are
/// cheap and may come from any thread; they take effect on the animation's
/// next frame. A controller that is not attached to a GIF, or is attached to
/// an image that is not animated, does nothing.
///
/// # Examples
///
/// ```
/// use aimer_assets::{GifController, Image};
///
/// let controller = GifController::paused();
/// let image = Image::new("loading.gif").gif_controller(controller.clone());
///
/// controller.play();
/// assert!(controller.is_playing());
/// ```
#[derive(Clone, Debug)]
pub struct GifController {
    state: Arc<ControllerState>,
}

impl Default for GifController {
    fn default() -> Self {
        Self::new()
    }
}

impl GifController {
    /// Creates a controller whose animation plays as soon as it is shown.
    #[inline]
    pub fn new() -> Self {
        Self {
            state: Arc::new(ControllerState::new(true)),
        }
    }

    /// Creates a controller whose animation shows its first frame and waits
    /// for [`play`](Self::play).
    #[inline]
    pub fn paused() -> Self {
        Self {
            state: Arc::new(ControllerState::new(false)),
        }
    }

    /// Resumes playback. A GIF that ran out of loops starts over.
    pub fn play(&self) {
        if self.state.finished.swap(false, Ordering::AcqRel) {
            self.state.seek.store(0, Ordering::Release);
        }
        self.state.playing.store(true, Ordering::Release);
        self.state.wake();
    }

    /// Holds the current frame until [`play`](Self::play).
    #[inline]
    pub fn pause(&self) {
        self.state.playing.store(false, Ordering::Release);
    }

    /// Pauses a playing animation and resumes a paused one.
    pub fn toggle(&self) {
        if self.is_playing() {
            self.pause();
        } else {
            self.play();
        }
    }

    /// Whether the animation is set to play. A finished GIF still reports
    /// `true`; see [`is_finished`](Self::is_finished).
    #[inline]
    pub fn is_playing(&self) -> bool {
        self.state.playing.load(Ordering::Acquire)
    }

    /// Jumps to `frame`, counting from `0`, whether playing or paused.
    ///
    /// A frame past the end of the GIF selects the last frame. Reaching a frame
    /// means decoding every frame before it, so seeking far into a long GIF
    /// takes proportionally longer; the old frame stays on screen meanwhile.
    pub fn seek_to_frame(&self, frame: usize) {
        self.state.finished.store(false, Ordering::Release);
        self.state
            .seek
            .store(frame.min(NO_SEEK - 1), Ordering::Release);
        self.state.wake();
    }

    /// Starts over from the first frame and plays.
    pub fn restart(&self) {
        self.state.playing.store(true, Ordering::Release);
        self.seek_to_frame(0);
    }

    /// The index of the frame most recently shown.
    #[inline]
    pub fn current_frame(&self) -> usize {
        self.state.frame.load(Ordering::Acquire)
    }

    /// The number of frames, known once the decoder has read the GIF to its
    /// end for the first time.
    #[inline]
    pub fn frame_count(&self) -> Option<usize> {
        match self.state.frame_count.load(Ordering::Acquire) {
            0 => None,
            count => Some(count),
        }
    }

    /// Whether playback stopped on the last frame because the GIF ran out of
    /// loops.
    #[inline]
    pub fn is_finished(&self) -> bool {
        self.state.finished.load(Ordering::Acquire)
    }
}

/// How many times a GIF plays.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum GifLoops {
    /// Follow the loop count stored in the file. A file that stores none plays
    /// forever.
    #[default]
    FromFile,
    /// Play forever, whatever the file says.
    Infinite,
    /// Play this many times in total, whatever the file says.
    Count(NonZeroU32),
}

impl GifLoops {
    /// Plays exactly once.
    pub const ONCE: Self = Self::Count(NonZeroU32::MIN);
}

/// The playback settings an image widget was built with.
#[derive(Clone, Debug, Default)]
pub struct GifOptions {
    controller: Option<GifController>,
    loops: GifLoops,
}

impl GifOptions {
    /// Creates the default settings: play forever or as the file says, with no
    /// controller.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Attaches a controller.
    #[inline]
    pub fn controller(mut self, controller: GifController) -> Self {
        self.controller = Some(controller);
        self
    }

    /// Sets how many times the GIF plays.
    #[inline]
    pub fn loops(mut self, loops: GifLoops) -> Self {
        self.loops = loops;
        self
    }
}

// ---------------------------------------------------------------------------
// Registry of encoded GIF bytes
// ---------------------------------------------------------------------------

const REGISTRY_MAX_ENTRIES: usize = 64;
const REGISTRY_MAX_BYTES: usize = 32 * 1024 * 1024;
const REGISTRY_MAX_ENTRY_BYTES: usize = 16 * 1024 * 1024;

/// The encoded bytes of the GIFs the loaders decoded, oldest first. Bounded:
/// the oldest are dropped first, and a player that already holds its bytes is
/// unaffected.
struct Registry {
    entries: VecDeque<(String, Arc<[u8]>)>,
    bytes: usize,
}

impl Registry {
    const fn new() -> Self {
        Self {
            entries: VecDeque::new(),
            bytes: 0,
        }
    }

    fn insert(&mut self, key: String, bytes: Arc<[u8]>) {
        if let Some(position) = self.entries.iter().position(|(entry, _)| *entry == key)
            && let Some((_, old)) = self.entries.remove(position)
        {
            self.bytes -= old.len();
        }
        self.bytes += bytes.len();
        self.entries.push_back((key, bytes));
        while self.entries.len() > REGISTRY_MAX_ENTRIES || self.bytes > REGISTRY_MAX_BYTES {
            match self.entries.pop_front() {
                Some((_, old)) => self.bytes -= old.len(),
                None => break,
            }
        }
    }

    fn get(&self, key: &str) -> Option<Arc<[u8]>> {
        self.entries
            .iter()
            .find(|(entry, _)| entry == key)
            .map(|(_, bytes)| Arc::clone(bytes))
    }
}

static REGISTRY: Mutex<Registry> = Mutex::new(Registry::new());

/// The registry key of a file image.
pub(crate) fn key_file(path: &std::path::Path) -> String {
    format!("file:{}", path.display())
}

/// The registry key of a bundled asset image.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(crate) fn key_asset(key: &str) -> String {
    format!("asset:{key}")
}

/// The registry key of a network image, and on the web of a bundled asset
/// fetched from the site root.
pub(crate) fn key_network(url: &str) -> String {
    format!("net:{url}")
}

/// Whether `bytes` start with a GIF signature.
#[inline]
pub(crate) fn is_gif(bytes: &[u8]) -> bool {
    bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a")
}

/// Remembers `bytes` under `key` when they are a GIF. Called by the loaders on
/// their decoding threads, before the decoded image is published, so a widget
/// that sees the image settle also finds its animation.
pub(crate) fn register(key: String, bytes: &[u8]) {
    if !is_gif(bytes) || bytes.len() > REGISTRY_MAX_ENTRY_BYTES {
        return;
    }
    let shared: Arc<[u8]> = Arc::from(bytes);
    if let Ok(mut registry) = REGISTRY.lock() {
        registry.insert(key, shared);
    }
}

/// The bytes registered under `key`.
pub(crate) fn lookup(key: &str) -> Option<Arc<[u8]>> {
    REGISTRY.lock().ok()?.get(key)
}

// ---------------------------------------------------------------------------
// Decoding
// ---------------------------------------------------------------------------

/// One composited frame, ready for the renderer.
pub(crate) struct DecodedFrame {
    index: usize,
    delay: Duration,
    resource: Arc<ImageResource>,
}

fn frame_delay(frame: &image::Frame) -> Duration {
    let (numerator, denominator) = frame.delay().numer_denom_ms();
    if denominator == 0 {
        return Duration::ZERO;
    }
    Duration::from_micros(u64::from(numerator) * 1000 / u64::from(denominator))
}

/// How long a frame stays up: browsers' rule for the delays encoders write
/// to mean "as fast as possible".
#[inline]
fn effective_delay(delay: Duration) -> Duration {
    if delay <= MIN_FRAME_DELAY {
        DEFAULT_FRAME_DELAY
    } else {
        delay
    }
}

/// A frame as the renderer holds it: scaled down to `limit` if larger (the same
/// rule the still images follow), with its texture identity worked out here on
/// the decoding thread.
fn frame_resource(image: image::RgbaImage, limit: u32) -> Option<Arc<ImageResource>> {
    let (width, height) = image.dimensions();
    #[cfg(not(target_arch = "wasm32"))]
    let (upload_width, upload_height) = constrained_image_size(width, height, limit);
    #[cfg(target_arch = "wasm32")]
    let (upload_width, upload_height) = {
        let _ = (constrained_image_size, limit);
        (width, height)
    };
    #[cfg(not(target_arch = "wasm32"))]
    let rgba = if (upload_width, upload_height) == (width, height) {
        image.into_raw()
    } else {
        crate::img_widget::source::downscale_rgba(
            image.as_raw(),
            width,
            height,
            upload_width,
            upload_height,
        )
    };
    #[cfg(target_arch = "wasm32")]
    let rgba = image.into_raw();

    let texture_id =
        aimer_cupid::draw_cmd::ImageResource::texture_id_for(&rgba, upload_width, upload_height);
    let revision =
        aimer_cupid::draw_cmd::ImageResource::content_revision_for(&rgba, upload_width, upload_height);
    ImageResource::rgba8_with_intrinsic_size(
        texture_id,
        revision,
        upload_width,
        upload_height,
        width,
        height,
        Arc::from(rgba),
    )
    .map(Arc::new)
}

fn open(bytes: &Arc<[u8]>) -> Option<(Frames<'static>, LoopCount)> {
    let decoder = GifDecoder::new(Cursor::new(Arc::clone(bytes))).ok()?;
    let (width, height) = decoder.dimensions();
    if u64::from(width) * u64::from(height) > MAX_CANVAS_PIXELS {
        return None;
    }
    let loops = decoder.loop_count();
    Some((decoder.into_frames(), loops))
}

/// Walks a GIF's frames in playback order, looping as asked.
///
/// Not `Send` (the image crate's frame iterator is not), so it is built on the
/// thread that uses it.
struct FrameSource {
    bytes: Arc<[u8]>,
    loops: GifLoops,
    limit: u32,
    controller: Arc<ControllerState>,
    frames: Option<Frames<'static>>,
    /// Plays still to start, `None` for forever. Known once the file is open.
    total_plays: Option<u32>,
    play: u32,
    index: usize,
    /// Frames before this one are decoded and dropped, to reach a seek target.
    skip: usize,
    /// The newest frame dropped while skipping, kept in case the target lies
    /// past the end and the last frame is wanted instead.
    held: Option<(usize, image::Frame)>,
    done: bool,
}

impl FrameSource {
    fn new(
        bytes: Arc<[u8]>,
        loops: GifLoops,
        limit: u32,
        controller: Arc<ControllerState>,
        skip: usize,
    ) -> Self {
        Self {
            bytes,
            loops,
            limit,
            controller,
            frames: None,
            total_plays: None,
            play: 0,
            index: 0,
            skip,
            held: None,
            done: false,
        }
    }

    fn emit(&mut self, index: usize, frame: image::Frame) -> Option<DecodedFrame> {
        let delay = frame_delay(&frame);
        match frame_resource(frame.into_buffer(), self.limit) {
            Some(resource) => Some(DecodedFrame {
                index,
                delay,
                resource,
            }),
            None => {
                self.done = true;
                None
            }
        }
    }

    /// The next frame, or `None` once the GIF has played all of its loops (or
    /// cannot be decoded).
    fn next(&mut self) -> Option<DecodedFrame> {
        loop {
            if self.done {
                return None;
            }
            if self.frames.is_none() {
                let Some((frames, file_loops)) = open(&self.bytes) else {
                    self.done = true;
                    return None;
                };
                self.total_plays = match (self.loops, file_loops) {
                    (GifLoops::Infinite, _) => None,
                    (GifLoops::Count(count), _) => Some(count.get()),
                    (GifLoops::FromFile, LoopCount::Infinite) => None,
                    // The file counts repeats after the first play.
                    (GifLoops::FromFile, LoopCount::Finite(count)) => {
                        Some(count.get().saturating_add(1))
                    }
                };
                self.frames = Some(frames);
                self.index = 0;
            }
            let next = self.frames.as_mut().and_then(Iterator::next);
            match next {
                Some(Ok(frame)) => {
                    let index = self.index;
                    self.index += 1;
                    if index < self.skip {
                        self.held = Some((index, frame));
                        continue;
                    }
                    self.held = None;
                    return self.emit(index, frame);
                }
                // A frame that cannot be decoded ends the animation where it is.
                Some(Err(_)) => {
                    self.done = true;
                    return None;
                }
                None => {
                    if self.index == 0 {
                        self.done = true;
                        return None;
                    }
                    self.controller
                        .frame_count
                        .store(self.index, Ordering::Release);
                    self.frames = None;
                    self.skip = 0;
                    self.play += 1;
                    if self.total_plays.is_some_and(|total| self.play >= total) {
                        self.done = true;
                    }
                    if let Some((index, frame)) = self.held.take() {
                        return self.emit(index, frame);
                    }
                }
            }
        }
    }
}

enum Feed {
    #[cfg(not(target_arch = "wasm32"))]
    Thread(crossbeam::channel::Receiver<DecodedFrame>),
    #[cfg(any(target_arch = "wasm32", test))]
    Inline(Box<FrameSource>),
}

enum Polled {
    Frame(DecodedFrame),
    /// The decoder has nothing ready yet.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    Pending,
    Ended,
}

impl Feed {
    fn poll(&mut self) -> Polled {
        match self {
            #[cfg(not(target_arch = "wasm32"))]
            Feed::Thread(receiver) => match receiver.try_recv() {
                Ok(frame) => Polled::Frame(frame),
                Err(crossbeam::channel::TryRecvError::Empty) => Polled::Pending,
                Err(crossbeam::channel::TryRecvError::Disconnected) => Polled::Ended,
            },
            #[cfg(any(target_arch = "wasm32", test))]
            Feed::Inline(source) => match source.next() {
                Some(frame) => Polled::Frame(frame),
                None => Polled::Ended,
            },
        }
    }
}

/// Starts the decoder thread. The thread ends when the GIF has played all of
/// its loops, or as soon as the returned feed is dropped.
#[cfg(not(target_arch = "wasm32"))]
fn spawn_feed(
    bytes: Arc<[u8]>,
    loops: GifLoops,
    limit: u32,
    controller: Arc<ControllerState>,
    skip: usize,
) -> Option<Feed> {
    let (sender, receiver) = crossbeam::channel::bounded(FRAME_WINDOW);
    std::thread::Builder::new()
        .name("aimer-gif-decoder".into())
        .spawn(move || {
            let mut source = FrameSource::new(bytes, loops, limit, controller, skip);
            while let Some(frame) = source.next() {
                if sender.send(frame).is_err() {
                    break;
                }
            }
        })
        .ok()?;
    Some(Feed::Thread(receiver))
}

// ---------------------------------------------------------------------------
// Playback
// ---------------------------------------------------------------------------

/// What one [`GifPlayer::tick`] found.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct Tick {
    /// A different frame is now current, so the picture must be recorded again.
    pub changed: bool,
    /// The player has more to show: the widget should ask for another frame.
    pub wants_frame: bool,
}

/// The playback state of one image widget.
pub(crate) struct GifPlayer {
    bytes: Arc<[u8]>,
    loops: GifLoops,
    limit: u32,
    controller: Arc<ControllerState>,
    feed: Option<Feed>,
    current: Option<DecodedFrame>,
    /// How long the current frame has been up, while playing.
    elapsed: Duration,
    last_tick: Option<AnimInstant>,
    finished: bool,
    /// No frame is current yet, or a seek is waiting for its frame.
    awaiting: bool,
}

impl GifPlayer {
    pub(crate) fn new(
        bytes: Arc<[u8]>,
        options: &GifOptions,
        limit: u32,
        window: &WindowHandle,
    ) -> Self {
        let controller = options
            .controller
            .as_ref()
            .map_or_else(|| Arc::new(ControllerState::new(true)), |c| c.state.clone());
        if let Ok(mut waker) = controller.waker.lock() {
            *waker = Some(window.clone());
        }
        controller.frame.store(0, Ordering::Release);
        controller.frame_count.store(0, Ordering::Release);
        controller.finished.store(false, Ordering::Release);
        let mut player = Self {
            bytes,
            loops: options.loops,
            limit,
            controller,
            feed: None,
            current: None,
            elapsed: Duration::ZERO,
            last_tick: None,
            finished: false,
            awaiting: true,
        };
        player.start_feed(0);
        player
    }

    fn start_feed(&mut self, skip: usize) {
        let bytes = Arc::clone(&self.bytes);
        let controller = Arc::clone(&self.controller);
        #[cfg(any(target_arch = "wasm32", test))]
        let feed = Some(Feed::Inline(Box::new(FrameSource::new(
            bytes,
            self.loops,
            self.limit,
            controller,
            skip,
        ))));
        #[cfg(not(any(target_arch = "wasm32", test)))]
        let feed = spawn_feed(bytes, self.loops, self.limit, controller, skip);
        self.finished = feed.is_none();
        self.feed = feed;
        self.awaiting = true;
        self.elapsed = Duration::ZERO;
    }

    fn show(&mut self, frame: DecodedFrame) {
        self.controller.frame.store(frame.index, Ordering::Release);
        self.current = Some(frame);
        self.elapsed = Duration::ZERO;
    }

    fn poll(&mut self) -> Polled {
        self.feed.as_mut().map_or(Polled::Ended, Feed::poll)
    }

    /// Brings the player to `now`: honors a seek, takes the next frame when the
    /// current one has been up long enough, and says whether another frame is
    /// wanted.
    pub(crate) fn tick(&mut self, now: AnimInstant) -> Tick {
        let elapsed = self
            .last_tick
            .map_or(Duration::ZERO, |last| now.duration_since(last));
        self.last_tick = Some(now);

        let seek = self.controller.seek.swap(NO_SEEK, Ordering::AcqRel);
        if seek != NO_SEEK {
            let target = match self.controller.frame_count.load(Ordering::Acquire) {
                0 => seek,
                count => seek.min(count - 1),
            };
            self.start_feed(target);
        }

        let playing = self.controller.playing.load(Ordering::Acquire);
        let mut changed = false;
        if self.awaiting {
            // The first frame, or the target of a seek, is shown even while
            // paused: that is what a paused GIF looks like.
            match self.poll() {
                Polled::Frame(frame) => {
                    self.show(frame);
                    self.awaiting = false;
                    changed = true;
                }
                Polled::Pending => {}
                Polled::Ended => {
                    self.awaiting = false;
                    self.finished = true;
                }
            }
        } else if playing && !self.finished {
            self.elapsed += elapsed;
            // A long stall must not replay the frames it skipped.
            for _ in 0..=FRAME_WINDOW {
                let Some(delay) = self.current.as_ref().map(|c| effective_delay(c.delay)) else {
                    break;
                };
                if self.elapsed < delay {
                    break;
                }
                match self.poll() {
                    Polled::Frame(frame) => {
                        let carried = self.elapsed - delay;
                        self.show(frame);
                        self.elapsed = carried;
                        changed = true;
                    }
                    Polled::Pending => {
                        // The decoder is behind; do not owe it a backlog.
                        self.elapsed = delay;
                        break;
                    }
                    Polled::Ended => {
                        self.finished = true;
                        break;
                    }
                }
            }
            if changed
                && let Some(delay) = self.current.as_ref().map(|c| effective_delay(c.delay))
            {
                self.elapsed = self.elapsed.min(delay);
            }
        }
        self.controller
            .finished
            .store(self.finished && !self.awaiting, Ordering::Release);

        Tick {
            changed,
            wants_frame: self.awaiting || (playing && !self.finished),
        }
    }

    #[inline]
    pub(crate) fn resource(&self) -> Option<&Arc<ImageResource>> {
        self.current.as_ref().map(|frame| &frame.resource)
    }
}

enum SlotState {
    /// Whether the image is a GIF has not been looked up yet.
    Unchecked,
    /// The image is not an animated GIF, or its bytes are gone.
    Static,
    Playing(Box<GifPlayer>),
}

/// The part of an image widget that plays a GIF, empty for every other image.
///
/// It holds nothing until the still image settles and the loaders turn out to
/// have registered GIF bytes for it.
#[doc(hidden)]
pub struct GifSlot {
    options: GifOptions,
    state: RefCell<SlotState>,
}

impl Default for GifSlot {
    fn default() -> Self {
        Self::new(GifOptions::default())
    }
}

impl GifSlot {
    /// Creates a slot that plays with `options` if its image is a GIF.
    #[inline]
    pub fn new(options: GifOptions) -> Self {
        Self {
            options,
            state: RefCell::new(SlotState::Unchecked),
        }
    }

    /// Advances the animation. `bytes` is asked at most once, when the still
    /// image has settled: that is the moment its GIF bytes are registered.
    pub(crate) fn sync(
        &self,
        ctx: &BuildContext,
        bytes: impl FnOnce() -> Option<Arc<[u8]>>,
    ) -> Tick {
        let mut state = self.state.borrow_mut();
        if matches!(*state, SlotState::Unchecked) {
            *state = match bytes() {
                Some(bytes) => SlotState::Playing(Box::new(GifPlayer::new(
                    bytes,
                    &self.options,
                    upload_limit(&ctx.window),
                    &ctx.window,
                ))),
                None => SlotState::Static,
            };
        }
        match &mut *state {
            SlotState::Playing(player) => player.tick(AnimInstant::now()),
            SlotState::Unchecked | SlotState::Static => Tick::default(),
        }
    }

    /// The frame to paint instead of the still image, once there is one.
    #[inline]
    pub(crate) fn current_resource(&self) -> Option<Arc<ImageResource>> {
        match &*self.state.borrow() {
            SlotState::Playing(player) => player.resource().cloned(),
            SlotState::Unchecked | SlotState::Static => None,
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn upload_limit(window: &WindowHandle) -> u32 {
    crate::img_widget::source::upload_limit(window)
}

#[cfg(target_arch = "wasm32")]
fn upload_limit(_window: &WindowHandle) -> u32 {
    u32::MAX
}

#[cfg(test)]
#[path = "gif_tests.rs"]
mod tests;
