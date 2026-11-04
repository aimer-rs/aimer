//! Android: `android.media.MediaPlayer` into a `SurfaceTexture`, converted to
//! RGBA on the GPU and shared as `AHardwareBuffer`s.
//!
//! `MediaPlayer` opens, streams (progressive http(s) and HLS), decodes and
//! plays the soundtrack in sync. Its picture goes to a `Surface` made from a
//! `SurfaceTexture`, which exposes it as an OpenGL ES *external* texture in the
//! decoder's own (often YUV) format. The renderer's image pipeline samples
//! plain RGBA, so a small GL pass draws that texture into an `AHardwareBuffer`
//! of RGBA pixels; the Vulkan backend imports the buffer and samples it where
//! it lies (`NativeTexture::AndroidHardwareBuffer`). The picture is converted
//! once, on the GPU, and never touches the CPU.
//!
//! One worker thread per player owns everything with thread affinity: the EGL
//! context, the `SurfaceTexture` (which latches frames on the thread whose
//! context it was attached to), and the `MediaPlayer`. The player handle sends
//! it commands and reads a snapshot of its state, so no call from the UI thread
//! waits on JNI or the GPU.
//!
//! No Java listener is installed: implementing one needs a Java class, which a
//! Rust library cannot ship. So the worker polls instead: a new picture is
//! noticed by the `SurfaceTexture`'s timestamp changing after
//! `updateTexImage`, the end by `isPlaying()` turning false near the end, and
//! a stream that stops early is reported as failed. Mid-stream stalls are not
//! visible, and the buffered range of a stream is unknown.

use std::any::Any;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use aimer_cupid::external_texture::NativeTexture;
use aimer_cupid_gpu::external_pool::{self, PoolRegistration};
use jni::objects::{Global, JFloatArray, JObject, JString, JValue};
use jni::{Env, JavaVM, jni_sig, jni_str};

use crate::android_gl::{Converter, HardwareBuffer, Target};
use crate::backend::{FrameSlot, PlatformVideo, StoredFrame};
use crate::ring::Ring;
use crate::{VideoError, VideoSource};

type JniResult<T> = Result<T, jni::errors::Error>;

/// Buffers a player renders into in turn: one the renderer is drawing, a few
/// it still holds for in-flight frames, and one being written.
const RING: usize = 6;
/// How often the worker looks for a new picture while one is wanted.
const PICTURE_TICK: Duration = Duration::from_millis(4);
/// How often it looks at anything when nothing is wanted.
const IDLE_TICK: Duration = Duration::from_millis(100);
/// How long after a start, seek or open a picture is still being waited for.
const PICTURE_WAIT: Duration = Duration::from_millis(2000);
/// A stream that stops more than this before its end has failed, not finished.
const EARLY_STOP_MS: i32 = 1500;
/// A file that stops this close to its end has finished.
const END_SLACK_MS: i32 = 800;

fn vm() -> Result<JavaVM, String> {
    let vm = ndk_context::android_context().vm() as *mut jni::sys::JavaVM;
    if vm.is_null() {
        return Err("the Android runtime is not running".into());
    }
    // SAFETY: `ndk-context` hands out the `JavaVM` owned by the android-activity
    // runtime, valid for the life of the process.
    Ok(unsafe { JavaVM::from_raw(vm) })
}

/// Runs `call` with this thread attached to the VM.
fn with_env<T>(call: impl FnOnce(&mut Env) -> JniResult<T>) -> Result<T, String> {
    vm()?.attach_current_thread(call).map_err(|error| error.to_string())
}

/// The application context, owned by the android-activity runtime.
fn context<'local>(env: &Env<'local>) -> JniResult<JObject<'local>> {
    let raw = ndk_context::android_context().context() as jni::sys::jobject;
    if raw.is_null() {
        return Err(jni::errors::Error::NullPtr("application context"));
    }
    // SAFETY: the context is a global reference owned by the runtime, valid
    // for the life of the process.
    Ok(unsafe { JObject::from_raw(env, raw) })
}

/// Where a player reads from.
enum Origin {
    Path(PathBuf),
    Asset(String),
    Url(String),
}

/// Opens `source` as a paused player.
pub(crate) fn open(source: &VideoSource) -> Result<Box<dyn PlatformVideo>, VideoError> {
    let (target, streaming) = match source {
        VideoSource::File(path) => {
            if !path.is_file() {
                return Err(VideoError::NotFound(path.display().to_string()));
            }
            (Origin::Path(path.clone()), false)
        }
        VideoSource::Bundled(key) => (Origin::Asset(key.clone()), false),
        VideoSource::Url(url) => {
            let lower = url.get(..8).unwrap_or(url).to_ascii_lowercase();
            if !(lower.starts_with("http://") || lower.starts_with("https://")) {
                return Err(VideoError::Network(format!("not an http(s) URL: {url}")));
            }
            (Origin::Url(url.clone()), true)
        }
    };
    let shared = Arc::new(Shared::default());
    let (commands, receiver) = mpsc::channel();
    let worker_shared = shared.clone();
    std::thread::Builder::new()
        .name("aimer-video".into())
        // The worker holds GL objects, so it is built on the thread that owns them.
        .spawn(move || Worker::new(worker_shared, receiver, streaming).run(target))
        .map_err(|error| VideoError::Playback(error.to_string()))?;
    Ok(Box::new(AndroidVideo {
        shared,
        commands: Mutex::new(commands),
        intent: AtomicBool::new(false),
        streaming,
        shown: Mutex::new(0),
    }))
}

enum Command {
    Play,
    Pause,
    Seek(f64),
    Volume(f32),
    Stop,
}

/// What the worker tells the player handle, behind a lock.
#[derive(Default)]
struct Snapshot {
    /// The source has opened: its length and size are known.
    ready: bool,
    position: f64,
    /// Seconds, or `None` for a live stream or before it opened.
    duration: Option<f64>,
    size: Option<(u32, u32)>,
    /// The playback reached its end.
    ended: bool,
    error: Option<VideoError>,
    /// The newest picture and its number.
    frame: Option<(Arc<Slot>, u64)>,
}

#[derive(Default)]
struct Shared {
    snapshot: Mutex<Snapshot>,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Snapshot> {
        self.snapshot.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// One shared picture buffer. Frames hold it, so it is not drawn into again
/// while the renderer may sample it.
struct Slot {
    /// Keeps the pool id live for the renderer's caches.
    pool: Arc<PoolRegistration>,
    index: u32,
    buffer: HardwareBuffer,
}

// SAFETY: see `HardwareBuffer`; the rest is plain data.
unsafe impl Send for Slot {}
unsafe impl Sync for Slot {}

struct AndroidVideo {
    shared: Arc<Shared>,
    commands: Mutex<Sender<Command>>,
    /// Whether the player last asked to play (the system's own state lags).
    intent: AtomicBool,
    streaming: bool,
    /// The number of the picture last handed to the frame slot.
    shown: Mutex<u64>,
}

impl AndroidVideo {
    fn send(&self, command: Command) {
        let _ = self
            .commands
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .send(command);
    }
}

impl Drop for AndroidVideo {
    fn drop(&mut self) {
        // The worker releases the player itself, whenever it gets there: it may
        // be blocked preparing a stream, which must not hold up the UI thread.
        self.send(Command::Stop);
    }
}

impl PlatformVideo for AndroidVideo {
    fn play(&self) {
        self.intent.store(true, Ordering::Release);
        self.shared.lock().ended = false;
        self.send(Command::Play);
    }

    fn pause(&self) {
        self.intent.store(false, Ordering::Release);
        self.send(Command::Pause);
    }

    fn seek(&self, seconds: f64) {
        {
            let mut snapshot = self.shared.lock();
            snapshot.ended = false;
            // Show the target at once; the worker confirms as it plays.
            snapshot.position = seconds;
        }
        self.send(Command::Seek(seconds));
    }

    fn set_volume(&self, volume: f32) {
        self.send(Command::Volume(volume));
    }

    fn is_ready(&self) -> bool {
        self.shared.lock().ready
    }

    fn position(&self) -> f64 {
        self.shared.lock().position
    }

    fn duration(&self) -> f64 {
        self.shared.lock().duration.unwrap_or(f64::NAN)
    }

    fn buffered(&self) -> f64 {
        let snapshot = self.shared.lock();
        match snapshot.duration {
            // `MediaPlayer` reports buffering only to a Java listener, so a
            // stream's range is unknown; a file is all there.
            Some(duration) if !self.streaming => duration,
            _ => snapshot.position,
        }
    }

    fn is_playing(&self) -> bool {
        self.intent.load(Ordering::Acquire) && !self.shared.lock().ended
    }

    fn is_buffering(&self) -> bool {
        self.intent.load(Ordering::Acquire) && !self.shared.lock().ready
    }

    fn error(&self) -> Option<VideoError> {
        self.shared.lock().error.clone()
    }

    fn size(&self) -> Option<(u32, u32)> {
        self.shared.lock().size
    }

    fn take_frame(&self, frames: &FrameSlot) -> bool {
        let (slot, serial, size) = {
            let snapshot = self.shared.lock();
            let Some((slot, serial)) = snapshot.frame.clone() else {
                return false;
            };
            let Some(size) = snapshot.size else {
                return false;
            };
            (slot, serial, size)
        };
        let mut shown = self.shown.lock().unwrap_or_else(PoisonError::into_inner);
        if *shown == serial && frames.has_frame() {
            return false;
        }
        *shown = serial;
        let texture = NativeTexture::AndroidHardwareBuffer {
            pool: slot.pool.id(),
            slot: slot.index,
            buffer: slot.buffer.as_ptr() as usize,
            serial,
        };
        let owner: Arc<dyn Any + Send + Sync> = slot;
        // SAFETY: `owner` keeps the buffer allocated, the worker finished
        // drawing into it before publishing it (it waits for the GPU), and it
        // does not draw into it again while any frame holds `owner`.
        let frame = unsafe { StoredFrame::new(texture, size.0, size.1, owner) };
        frames.store(Some(frame));
        true
    }
}

/// A buffer's GL side, owned by the worker.
struct Drawing {
    target: Target,
}

struct Worker {
    shared: Arc<Shared>,
    commands: Receiver<Command>,
    streaming: bool,
    /// Whether the player last asked to play.
    intent: bool,
    volume: f32,
    /// A seek asked for before the source opened.
    pending_seek: Option<f64>,
    /// Pictures are wanted until this time even when paused (after a start or
    /// seek the first picture is still on its way).
    wanted_until: Instant,
    last_timestamp: i64,
    serial: u64,
    pool: Option<Arc<PoolRegistration>>,
    ring: Ring<Slot, Drawing>,
    ring_size: (u32, u32),
    last_refresh: Instant,
    stopped: bool,
}

/// The Java objects of one player.
struct Java {
    player: Global<JObject<'static>>,
    texture: Global<JObject<'static>>,
    matrix: Global<JFloatArray<'static>>,
}

impl Worker {
    fn new(shared: Arc<Shared>, commands: Receiver<Command>, streaming: bool) -> Self {
        Self {
            shared,
            commands,
            streaming,
            intent: false,
            volume: 1.0,
            pending_seek: None,
            wanted_until: Instant::now() + PICTURE_WAIT,
            last_timestamp: i64::MIN,
            serial: 0,
            pool: None,
            ring: Ring::new(RING),
            ring_size: (0, 0),
            last_refresh: Instant::now(),
            stopped: false,
        }
    }

    fn fail(&self, error: VideoError) {
        self.shared.lock().error.get_or_insert(error);
    }

    fn run(mut self, target: Origin) {
        let converter = match Converter::new() {
            Ok(converter) => converter,
            Err(why) => {
                self.fail(VideoError::Playback(why));
                return;
            }
        };
        let java = match self.open_java(&converter, &target) {
            Ok(java) => java,
            Err(error) => {
                self.fail(error);
                return;
            }
        };
        while !self.stopped {
            let active = self.intent || Instant::now() < self.wanted_until;
            let tick = if active { PICTURE_TICK } else { IDLE_TICK };
            match self.commands.recv_timeout(tick) {
                Ok(command) => self.handle(&java, command),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            while let Ok(command) = self.commands.try_recv() {
                self.handle(&java, command);
            }
            if self.stopped {
                break;
            }
            let now = Instant::now();
            let active = self.intent || now < self.wanted_until;
            if active {
                self.latch_picture(&converter, &java);
            }
            if now.duration_since(self.last_refresh) >= Duration::from_millis(if active { 16 } else { 250 }) {
                self.last_refresh = now;
                self.refresh(&java);
            }
        }
        // Release everything on this thread, where the GL context is current.
        for (_, drawing) in self.ring.take_all() {
            converter.destroy_target(drawing.target);
        }
        let _ = with_env(|env| {
            env.call_method(&*java.player, jni_str!("release"), jni_sig!("()V"), &[])?.v()?;
            env.call_method(&*java.texture, jni_str!("release"), jni_sig!("()V"), &[])?.v()
        });
    }

    /// Creates the `SurfaceTexture`, `Surface` and `MediaPlayer`, and prepares
    /// the player on `target`.
    fn open_java(&mut self, converter: &Converter, target: &Origin) -> Result<Java, VideoError> {
        let texture_name = converter.external_texture() as i32;
        let java = with_env(|env| {
            let texture = env.new_object(
                jni_str!("android/graphics/SurfaceTexture"),
                jni_sig!("(I)V"),
                &[JValue::Int(texture_name)],
            )?;
            let surface = env.new_object(
                jni_str!("android/view/Surface"),
                jni_sig!("(Landroid/graphics/SurfaceTexture;)V"),
                &[JValue::Object(&texture)],
            )?;
            let player = env.new_object(jni_str!("android/media/MediaPlayer"), jni_sig!("()V"), &[])?;
            let prepared = (|| {
                set_source(env, &player, target)?;
                env.call_method(
                    &player,
                    jni_str!("setSurface"),
                    jni_sig!("(Landroid/view/Surface;)V"),
                    &[JValue::Object(&surface)],
                )?
                .v()?;
                // Blocks until prepared; the worker has nothing else to do.
                env.call_method(&player, jni_str!("prepare"), jni_sig!("()V"), &[])?.v()
            })();
            let _ = env.call_method(&surface, jni_str!("release"), jni_sig!("()V"), &[]);
            if let Err(error) = prepared {
                let _ = env.call_method(&player, jni_str!("release"), jni_sig!("()V"), &[]);
                let _ = env.call_method(&texture, jni_str!("release"), jni_sig!("()V"), &[]);
                return Err(error);
            }
            let matrix: JFloatArray = env.new_float_array(16)?;
            Ok(Java {
                player: env.new_global_ref(&player)?,
                texture: env.new_global_ref(&texture)?,
                matrix: env.new_global_ref(&matrix)?,
            })
        })
        .map_err(|why| if self.streaming { VideoError::Network(why) } else { VideoError::Decode(why) })?;
        self.apply_after_prepare(&java);
        Ok(java)
    }

    /// Applies what was asked for while the source opened, and publishes what
    /// it turned out to be.
    fn apply_after_prepare(&mut self, java: &Java) {
        let _ = with_env(|env| {
            let ms = |value: i32| if value > 0 { Some(f64::from(value) / 1000.0) } else { None };
            let duration = env.call_method(&*java.player, jni_str!("getDuration"), jni_sig!("()I"), &[])?.i()?;
            let width = env.call_method(&*java.player, jni_str!("getVideoWidth"), jni_sig!("()I"), &[])?.i()?;
            let height = env.call_method(&*java.player, jni_str!("getVideoHeight"), jni_sig!("()I"), &[])?.i()?;
            {
                let mut snapshot = self.shared.lock();
                snapshot.duration = ms(duration);
                snapshot.size = (width > 0 && height > 0).then_some((width as u32, height as u32));
                snapshot.ready = true;
            }
            env.call_method(
                &*java.player,
                jni_str!("setVolume"),
                jni_sig!("(FF)V"),
                &[JValue::Float(self.volume), JValue::Float(self.volume)],
            )?
            .v()?;
            // A prepared player draws no picture until it plays or seeks, so a
            // paused one seeks to the start (or where it was asked to) to show it.
            let start = self.pending_seek.take().unwrap_or(0.0);
            let ms = (start.max(0.0) * 1000.0).min(f64::from(i32::MAX)) as i32;
            env.call_method(&*java.player, jni_str!("seekTo"), jni_sig!("(I)V"), &[JValue::Int(ms)])?
                .v()?;
            if self.intent {
                env.call_method(&*java.player, jni_str!("start"), jni_sig!("()V"), &[])?.v()?;
            }
            Ok(())
        });
        self.wanted_until = Instant::now() + PICTURE_WAIT;
    }

    fn handle(&mut self, java: &Java, command: Command) {
        let ready = self.shared.lock().ready;
        match command {
            Command::Stop => self.stopped = true,
            Command::Volume(volume) => {
                self.volume = volume;
                let _ = with_env(|env| {
                    env.call_method(
                        &*java.player,
                        jni_str!("setVolume"),
                        jni_sig!("(FF)V"),
                        &[JValue::Float(volume), JValue::Float(volume)],
                    )?
                    .v()
                });
            }
            Command::Play => {
                self.intent = true;
                self.wanted_until = Instant::now() + PICTURE_WAIT;
                if ready {
                    self.call_void(java, jni_str!("start"));
                }
            }
            Command::Pause => {
                self.intent = false;
                if ready {
                    self.call_void(java, jni_str!("pause"));
                }
            }
            Command::Seek(seconds) => {
                self.wanted_until = Instant::now() + PICTURE_WAIT;
                let ms = (seconds.max(0.0) * 1000.0).min(f64::from(i32::MAX)) as i32;
                let _ = with_env(|env| {
                    env.call_method(&*java.player, jni_str!("seekTo"), jni_sig!("(I)V"), &[JValue::Int(ms)])?
                        .v()
                });
            }
        }
    }

    fn call_void(&self, java: &Java, name: &'static jni::strings::JNIStr) {
        if let Err(why) = with_env(|env| env.call_method(&*java.player, name, jni_sig!("()V"), &[])?.v()) {
            self.fail(VideoError::Playback(why));
        }
    }

    /// Reads the playhead and notices the end.
    fn refresh(&mut self, java: &Java) {
        if !self.shared.lock().ready {
            return;
        }
        let state = with_env(|env| {
            let position = env
                .call_method(&*java.player, jni_str!("getCurrentPosition"), jni_sig!("()I"), &[])?
                .i()?;
            let playing = env.call_method(&*java.player, jni_str!("isPlaying"), jni_sig!("()Z"), &[])?.z()?;
            let duration = env.call_method(&*java.player, jni_str!("getDuration"), jni_sig!("()I"), &[])?.i()?;
            Ok((position, playing, duration))
        });
        let (position, playing, duration) = match state {
            Ok(state) => state,
            Err(why) => {
                self.fail(VideoError::Playback(why));
                return;
            }
        };
        let mut snapshot = self.shared.lock();
        snapshot.position = f64::from(position.max(0)) / 1000.0;
        if duration > 0 && snapshot.duration.is_none() {
            snapshot.duration = Some(f64::from(duration) / 1000.0);
        }
        if self.intent && !playing {
            if duration > 0 && position >= duration - END_SLACK_MS {
                snapshot.ended = true;
                self.intent = false;
            } else if self.streaming && (duration <= 0 || position < duration - EARLY_STOP_MS) {
                // A stream that stopped by itself lost its connection;
                // `MediaPlayer` only tells a Java listener so.
                snapshot.error.get_or_insert(VideoError::Network("the stream stopped".into()));
                self.intent = false;
            } else if !self.streaming {
                snapshot.ended = true;
                self.intent = false;
            }
        }
    }

    /// Latches the decoder's newest picture, and converts it when it is new.
    fn latch_picture(&mut self, converter: &Converter, java: &Java) {
        if !self.shared.lock().ready {
            return;
        }
        let latched = with_env(|env| {
            env.call_method(&*java.texture, jni_str!("updateTexImage"), jni_sig!("()V"), &[])?
                .v()?;
            let timestamp = env
                .call_method(&*java.texture, jni_str!("getTimestamp"), jni_sig!("()J"), &[])?
                .j()?;
            if timestamp == self.last_timestamp {
                return Ok(None);
            }
            env.call_method(
                &*java.texture,
                jni_str!("getTransformMatrix"),
                jni_sig!("([F)V"),
                &[JValue::Object(&*java.matrix)],
            )?
            .v()?;
            let mut matrix = [0.0_f32; 16];
            java.matrix.get_region(env, 0, &mut matrix)?;
            Ok(Some((timestamp, matrix)))
        });
        let (timestamp, matrix) = match latched {
            Ok(Some(latched)) => latched,
            Ok(None) => return,
            Err(why) => {
                self.fail(VideoError::Decode(why));
                return;
            }
        };
        self.last_timestamp = timestamp;
        let Some(size) = self.shared.lock().size else {
            return;
        };
        if size != self.ring_size {
            for (_, drawing) in self.ring.take_all() {
                converter.destroy_target(drawing.target);
            }
            self.ring_size = size;
            self.pool = None;
        }
        let Some(index) = self.acquire(converter, size) else {
            // Every buffer is still in use by the renderer: drop this picture.
            return;
        };
        let (slot, drawing) = self.ring.get(index);
        converter.draw(&drawing.target, size, &matrix);
        self.serial += 1;
        let slot = slot.clone();
        self.shared.lock().frame = Some((slot, self.serial));
        // The first picture arrived.
        if !self.intent {
            self.wanted_until = Instant::now();
        }
    }

    /// A buffer to draw into: a free one, or a new one while the ring grows.
    fn acquire(&mut self, converter: &Converter, size: (u32, u32)) -> Option<usize> {
        if let Some(index) = self.ring.free() {
            return Some(index);
        }
        if !self.ring.can_grow() {
            return None;
        }
        let pool = self.pool.get_or_insert_with(|| Arc::new(external_pool::register())).clone();
        let made = HardwareBuffer::new(size.0, size.1).and_then(|buffer| {
            let target = converter.target(&buffer)?;
            Ok((buffer, target))
        });
        match made {
            Ok((buffer, target)) => {
                let index = self.ring.len();
                let slot = Arc::new(Slot {
                    pool,
                    index: index as u32,
                    buffer,
                });
                Some(self.ring.push(slot, Drawing { target }))
            }
            Err(why) => {
                self.fail(VideoError::Playback(why));
                None
            }
        }
    }
}

/// Points `player` at `target`.
fn set_source(env: &mut Env, player: &JObject, target: &Origin) -> JniResult<()> {
    match target {
        Origin::Path(path) => set_path(env, player, &path.to_string_lossy()),
        Origin::Url(url) => set_path(env, player, url),
        Origin::Asset(key) => {
            let in_place = (|| {
                let assets = context(env)?;
                let assets = env
                    .call_method(&assets, jni_str!("getAssets"), jni_sig!("()Landroid/content/res/AssetManager;"), &[])?
                    .l()?;
                let name = env.new_string(key)?;
                let fd = env
                    .call_method(
                        &assets,
                        jni_str!("openFd"),
                        jni_sig!("(Ljava/lang/String;)Landroid/content/res/AssetFileDescriptor;"),
                        &[JValue::Object(&name)],
                    )?
                    .l()?;
                let descriptor = env
                    .call_method(&fd, jni_str!("getFileDescriptor"), jni_sig!("()Ljava/io/FileDescriptor;"), &[])?
                    .l()?;
                let offset = env.call_method(&fd, jni_str!("getStartOffset"), jni_sig!("()J"), &[])?.j()?;
                let length = env.call_method(&fd, jni_str!("getLength"), jni_sig!("()J"), &[])?.j()?;
                let set = env.call_method(
                    player,
                    jni_str!("setDataSource"),
                    jni_sig!("(Ljava/io/FileDescriptor;JJ)V"),
                    &[JValue::Object(&descriptor), JValue::Long(offset), JValue::Long(length)],
                );
                // The player keeps its own duplicate of the descriptor.
                let _ = env.call_method(&fd, jni_str!("close"), jni_sig!("()V"), &[]);
                set?.v()
            })();
            // `openFd` refuses compressed assets; those are copied out.
            in_place.or_else(|_| {
                let copy = copy_asset(env, key)?;
                set_path(env, player, &copy)
            })
        }
    }
}

fn set_path(env: &mut Env, player: &JObject, path: &str) -> JniResult<()> {
    let path = env.new_string(path)?;
    env.call_method(
        player,
        jni_str!("setDataSource"),
        jni_sig!("(Ljava/lang/String;)V"),
        &[JValue::Object(&path)],
    )?
    .v()
}

/// Copies a compressed asset to the app's cache directory and returns the path.
fn copy_asset(env: &mut Env, key: &str) -> JniResult<String> {
    let context = context(env)?;
    let cache = env
        .call_method(&context, jni_str!("getCacheDir"), jni_sig!("()Ljava/io/File;"), &[])?
        .l()?;
    let directory = env
        .call_method(&cache, jni_str!("getAbsolutePath"), jni_sig!("()Ljava/lang/String;"), &[])?
        .l()?;
    let directory: JString = env.cast_local::<JString>(directory)?;
    let directory = directory.try_to_string(env)?;
    let name: String = key.chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' { c } else { '_' }).collect();
    let path = PathBuf::from(directory).join(format!("aimer-video-{name}"));
    if !path.is_file() {
        let assets = env
            .call_method(&context, jni_str!("getAssets"), jni_sig!("()Landroid/content/res/AssetManager;"), &[])?
            .l()?;
        let asset = env.new_string(key)?;
        let stream = env
            .call_method(
                &assets,
                jni_str!("open"),
                jni_sig!("(Ljava/lang/String;)Ljava/io/InputStream;"),
                &[JValue::Object(&asset)],
            )?
            .l()?;
        let mut bytes = Vec::new();
        let chunk: jni::objects::JByteArray = env.new_byte_array(64 * 1024)?;
        let mut scratch = vec![0_i8; 64 * 1024];
        loop {
            let read = env
                .call_method(&stream, jni_str!("read"), jni_sig!("([B)I"), &[JValue::Object(&chunk)])?
                .i()?;
            if read < 0 {
                break;
            }
            let read = read as usize;
            chunk.get_region(env, 0, &mut scratch[..read])?;
            bytes.extend(scratch[..read].iter().map(|&byte| byte as u8));
        }
        let _ = env.call_method(&stream, jni_str!("close"), jni_sig!("()V"), &[]);
        std::fs::write(&path, bytes).map_err(|_| jni::errors::Error::NullPtr("the cache directory"))?;
    }
    Ok(path.to_string_lossy().into_owned())
}
