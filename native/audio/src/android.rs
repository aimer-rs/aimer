//! Android: `android.media.MediaPlayer` over JNI.
//!
//! One `MediaPlayer` is one voice. It plays files, APK assets (through an
//! `AssetFileDescriptor`) and `http(s)` URLs, progressively. Data held in memory
//! (a downloaded file, a sound) is written once to the app's cache directory,
//! because `MediaPlayer` cannot read a Java byte array before API 23.
//!
//! No Java listener is installed: implementing one needs a Java class, which a
//! Rust library cannot ship. So the state is read by polling instead:
//!
//! - A local source is prepared synchronously on the engine thread, which is
//!   quick. A stream is prepared on a helper thread, because `prepare` blocks
//!   on the network; the voice touches the player only once that returns.
//! - The end shows as `isPlaying()` turning false. A stream that stops well
//!   before its end, or a player whose calls start throwing, is reported as
//!   failed rather than finished.
//! - Mid-stream stalls are not visible, so [`Voice::is_buffering`] covers only
//!   the wait before a stream starts, and [`Voice::buffered`] is unknown.
//!
//! The engine thread is attached to the VM permanently on first use.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::thread;

use jni::objects::{Global, JByteArray, JObject, JString, JValue};
use jni::{Env, JavaVM, jni_sig, jni_str};

use crate::AudioError;
use crate::backend::{Backend, Voice, VoiceData};

/// Bytes copied per JNI round trip when reading a Java `InputStream`.
const READ_CHUNK: usize = 64 * 1024;
/// A stream that stops more than this before its end has failed, not finished.
const EARLY_STOP_MS: i32 = 1500;

type JniResult<T> = Result<T, jni::errors::Error>;

fn vm() -> Result<JavaVM, AudioError> {
    let vm = ndk_context::android_context().vm() as *mut jni::sys::JavaVM;
    if vm.is_null() {
        return Err(AudioError::Playback("the Android runtime is not running".into()));
    }
    // SAFETY: `ndk-context` hands out the `JavaVM` owned by the android-activity
    // runtime, valid for the life of the process.
    Ok(unsafe { JavaVM::from_raw(vm) })
}

/// Runs `call` with this thread attached to the VM. A Java exception thrown
/// inside is caught, cleared, and returned as an error by the attachment.
fn with_env<T>(
    map: impl FnOnce(jni::errors::Error) -> AudioError,
    call: impl FnOnce(&mut Env) -> JniResult<T>,
) -> Result<T, AudioError> {
    vm()?.attach_current_thread(call).map_err(map)
}

fn playback(error: jni::errors::Error) -> AudioError {
    AudioError::Playback(error.to_string())
}

fn decode(error: jni::errors::Error) -> AudioError {
    AudioError::Decode(error.to_string())
}

/// The application context, owned by the android-activity runtime.
fn context<'local>(env: &Env<'local>) -> JniResult<JObject<'local>> {
    let raw = ndk_context::android_context().context() as jni::sys::jobject;
    if raw.is_null() {
        return Err(jni::errors::Error::NullPtr("application context"));
    }
    // SAFETY: the context is a global reference owned by the runtime, valid
    // for the life of the process, so it needs no local frame of its own.
    Ok(unsafe { JObject::from_raw(env, raw) })
}

fn asset_manager<'local>(env: &mut Env<'local>) -> JniResult<JObject<'local>> {
    let context = context(env)?;
    env.call_method(
        &context,
        jni_str!("getAssets"),
        jni_sig!("()Landroid/content/res/AssetManager;"),
        &[],
    )?
    .l()
}

/// Reads a Java `InputStream` to its end and closes it.
fn read_stream(env: &mut Env, stream: &JObject) -> JniResult<Vec<u8>> {
    let chunk: JByteArray = env.new_byte_array(READ_CHUNK)?;
    let mut bytes = Vec::new();
    let mut scratch = vec![0_i8; READ_CHUNK];
    let result = loop {
        let read = match env.call_method(stream, jni_str!("read"), jni_sig!("([B)I"), &[JValue::Object(&chunk)]) {
            Ok(read) => read.i()?,
            Err(error) => break Err(error),
        };
        if read < 0 {
            break Ok(());
        }
        let read = read as usize;
        chunk.get_region(env, 0, &mut scratch[..read])?;
        // Java bytes are signed; reinterpret each as the same bit pattern.
        bytes.extend(scratch[..read].iter().map(|&byte| byte as u8));
    };
    let _ = env.call_method(stream, jni_str!("close"), jni_sig!("()V"), &[]);
    result.map(|()| bytes)
}

pub(crate) struct AndroidBackend {
    cache_dir: PathBuf,
    /// In-memory data written to the cache directory, by the data it came
    /// from; the file is deleted once the data is gone.
    spilled: Vec<(Weak<[u8]>, PathBuf)>,
    /// Compressed APK assets copied out once, by key, for the backend's life.
    asset_copies: Vec<(String, PathBuf)>,
}

impl AndroidBackend {
    /// Writes `bytes` to a cache file once and returns its path.
    fn spill(&mut self, bytes: &Arc<[u8]>) -> Result<PathBuf, AudioError> {
        self.spilled.retain(|(data, path)| {
            let alive = data.strong_count() > 0;
            if !alive {
                let _ = std::fs::remove_file(path);
            }
            alive
        });
        for (data, path) in &self.spilled {
            if data.upgrade().is_some_and(|data| Arc::ptr_eq(&data, bytes)) {
                return Ok(path.clone());
            }
        }
        let path = self.cache_file(bytes)?;
        self.spilled.push((Arc::downgrade(bytes), path.clone()));
        Ok(path)
    }

    /// Writes `bytes` to a new file in the cache directory.
    fn cache_file(&self, bytes: &[u8]) -> Result<PathBuf, AudioError> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let name = format!(
            "aimer-audio-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let path = self.cache_dir.join(name);
        std::fs::write(&path, bytes).map_err(|error| AudioError::Io(error.to_string()))?;
        Ok(path)
    }

    /// A player prepared on `path`.
    fn open_path(&self, path: &Path) -> Result<Box<dyn Voice>, AudioError> {
        let path = path.to_string_lossy().into_owned();
        let player = with_env(decode, |env| {
            let player = new_player(env)?;
            let path = env.new_string(&path)?;
            env.call_method(
                &player,
                jni_str!("setDataSource"),
                jni_sig!("(Ljava/lang/String;)V"),
                &[JValue::Object(&path)],
            )?;
            prepared(env, player)
        })?;
        Ok(Box::new(AndroidVoice::ready(player)))
    }

    /// A player prepared on an APK asset, read in place when it is stored
    /// uncompressed and from a cache copy otherwise.
    fn open_asset(&mut self, key: &str) -> Result<Box<dyn Voice>, AudioError> {
        let in_place = with_env(decode, |env| {
            let assets = asset_manager(env)?;
            let name = env.new_string(key)?;
            let fd = env.call_method(
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
            let player = new_player(env)?;
            let set = env.call_method(
                &player,
                jni_str!("setDataSource"),
                jni_sig!("(Ljava/io/FileDescriptor;JJ)V"),
                &[JValue::Object(&descriptor), JValue::Long(offset), JValue::Long(length)],
            );
            // The player keeps its own duplicate of the descriptor.
            let _ = env.call_method(&fd, jni_str!("close"), jni_sig!("()V"), &[]);
            set?;
            prepared(env, player)
        });
        match in_place {
            Ok(player) => Ok(Box::new(AndroidVoice::ready(player))),
            // `openFd` refuses compressed assets; copy those out instead.
            Err(_) => {
                let known = self.asset_copies.iter().find(|(copied, _)| copied == key);
                let path = match known {
                    Some((_, path)) => path.clone(),
                    None => {
                        let path = self.cache_file(&read_asset(key)?)?;
                        self.asset_copies.push((key.to_string(), path.clone()));
                        path
                    }
                };
                self.open_path(&path)
            }
        }
    }

    /// A player that prepares `url` on a helper thread.
    fn open_stream(&self, url: &str) -> Result<Box<dyn Voice>, AudioError> {
        let network = |error: jni::errors::Error| AudioError::Network(error.to_string());
        let (player, preparing) = with_env(network, |env| {
            let player = new_player(env)?;
            let url = env.new_string(url)?;
            env.call_method(
                &player,
                jni_str!("setDataSource"),
                jni_sig!("(Ljava/lang/String;)V"),
                &[JValue::Object(&url)],
            )?;
            let player = env.new_global_ref(&player)?;
            let preparing = env.new_global_ref(&*player)?;
            Ok((player, preparing))
        })?;
        let prep = Arc::new(Prep::default());
        let shared = prep.clone();
        let spawned = thread::Builder::new()
            .name("aimer-audio-prepare".into())
            .spawn(move || {
                let result = with_env(network, |env| {
                    env.call_method(&*preparing, jni_str!("prepare"), jni_sig!("()V"), &[])?
                        .v()
                });
                let mut state = shared.lock();
                if state.released {
                    // The voice was dropped while this blocked; free it here.
                    let _ = with_env(playback, |env| release(env, &preparing));
                    return;
                }
                state.result = Some(result);
            });
        if let Err(error) = spawned {
            return Err(AudioError::Io(error.to_string()));
        }
        Ok(Box::new(AndroidVoice::new(player, Some(prep), true)))
    }
}

fn new_player<'local>(env: &mut Env<'local>) -> JniResult<JObject<'local>> {
    env.new_object(jni_str!("android/media/MediaPlayer"), jni_sig!("()V"), &[])
}

/// Prepares a player on a local source and keeps it globally.
fn prepared(env: &mut Env, player: JObject) -> JniResult<Global<JObject<'static>>> {
    if let Err(error) = env.call_method(&player, jni_str!("prepare"), jni_sig!("()V"), &[]) {
        let _ = env.call_method(&player, jni_str!("release"), jni_sig!("()V"), &[]);
        return Err(error);
    }
    env.new_global_ref(&player)
}

fn release(env: &mut Env, player: &Global<JObject<'static>>) -> JniResult<()> {
    env.call_method(&**player, jni_str!("release"), jni_sig!("()V"), &[])?.v()
}

fn read_asset(key: &str) -> Result<Vec<u8>, AudioError> {
    let not_found = |_| AudioError::NotFound(key.to_string());
    with_env(not_found, |env| {
        let assets = asset_manager(env)?;
        let name = env.new_string(key)?;
        let stream = env
            .call_method(
                &assets,
                jni_str!("open"),
                jni_sig!("(Ljava/lang/String;)Ljava/io/InputStream;"),
                &[JValue::Object(&name)],
            )?
            .l()?;
        read_stream(env, &stream)
    })
}

impl Backend for AndroidBackend {
    fn new() -> Self {
        // Attach now, and find the cache directory while at it.
        let cache_dir = with_env(playback, |env| {
            let context = context(env)?;
            let dir = env
                .call_method(&context, jni_str!("getCacheDir"), jni_sig!("()Ljava/io/File;"), &[])?
                .l()?;
            let path = env
                .call_method(&dir, jni_str!("getAbsolutePath"), jni_sig!("()Ljava/lang/String;"), &[])?
                .l()?;
            let path: JString = env.cast_local::<JString>(path)?;
            path.try_to_string(env)
        })
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir());
        Self {
            cache_dir,
            spilled: Vec::new(),
            asset_copies: Vec::new(),
        }
    }

    fn open(&mut self, data: &VoiceData) -> Result<Box<dyn Voice>, AudioError> {
        match data {
            VoiceData::Path(path) => self.open_path(path),
            VoiceData::Bytes(bytes) => {
                let path = self.spill(bytes)?;
                self.open_path(&path)
            }
            VoiceData::Stream(url) => self.open_stream(url),
            VoiceData::Asset(key) => self.open_asset(key),
        }
    }

    fn fetch(url: &str) -> Result<Vec<u8>, AudioError> {
        let network = |error: jni::errors::Error| AudioError::Network(error.to_string());
        with_env(network, |env| {
            let spec = env.new_string(url)?;
            let url = env.new_object(
                jni_str!("java/net/URL"),
                jni_sig!("(Ljava/lang/String;)V"),
                &[JValue::Object(&spec)],
            )?;
            let stream = env
                .call_method(&url, jni_str!("openStream"), jni_sig!("()Ljava/io/InputStream;"), &[])?
                .l()?;
            read_stream(env, &stream)
        })
    }

    fn resolve_bundled(key: &str, want_bytes: bool) -> Option<Result<VoiceData, AudioError>> {
        // Bundled assets live in the APK, not on the file system.
        Some(if want_bytes {
            read_asset(key).map(|bytes| VoiceData::Bytes(bytes.into()))
        } else {
            Ok(VoiceData::Asset(key.to_string()))
        })
    }
}

/// A stream's `prepare`, running on a helper thread.
#[derive(Default)]
struct Prep {
    state: Mutex<PrepState>,
}

#[derive(Default)]
struct PrepState {
    result: Option<Result<(), AudioError>>,
    /// The voice is gone; the helper thread releases the player.
    released: bool,
}

impl Prep {
    fn lock(&self) -> std::sync::MutexGuard<'_, PrepState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

struct AndroidVoice {
    player: Global<JObject<'static>>,
    /// Present while a stream prepares; the player is not touched until then.
    prep: Option<Arc<Prep>>,
    streaming: bool,
    intent: bool,
    volume: f32,
    looping: bool,
    pending_seek: Option<f64>,
    /// A failure seen by a call that cannot return one.
    failure: RefCell<Option<AudioError>>,
}

impl AndroidVoice {
    fn new(player: Global<JObject<'static>>, prep: Option<Arc<Prep>>, streaming: bool) -> Self {
        Self {
            player,
            prep,
            streaming,
            intent: false,
            volume: 1.0,
            looping: false,
            pending_seek: None,
            failure: RefCell::new(None),
        }
    }

    /// A voice on a prepared local source.
    fn ready(player: Global<JObject<'static>>) -> Self {
        Self::new(player, None, false)
    }

    fn is_ready(&self) -> bool {
        self.prep.is_none()
    }

    /// Calls the player, remembering a failure for [`Voice::error`].
    fn call<T>(&self, call: impl FnOnce(&mut Env, &JObject) -> JniResult<T>) -> Option<T> {
        if !self.is_ready() || self.failure.borrow().is_some() {
            return None;
        }
        match with_env(playback, |env| call(env, &self.player)) {
            Ok(value) => Some(value),
            Err(error) => {
                *self.failure.borrow_mut() = Some(error);
                None
            }
        }
    }

    fn int(&self, name: &'static jni::strings::JNIStr) -> Option<i32> {
        self.call(|env, player| env.call_method(player, name, jni_sig!("()I"), &[])?.i())
    }

    fn void(&self, name: &'static jni::strings::JNIStr) {
        self.call(|env, player| env.call_method(player, name, jni_sig!("()V"), &[])?.v());
    }

    fn seek_now(&self, seconds: f64) {
        let ms = (seconds.max(0.0) * 1000.0).min(f64::from(i32::MAX)) as i32;
        self.call(|env, player| {
            env.call_method(player, jni_str!("seekTo"), jni_sig!("(I)V"), &[JValue::Int(ms)])?
                .v()
        });
    }

    fn apply_volume(&self) {
        let volume = self.volume;
        self.call(|env, player| {
            env.call_method(
                player,
                jni_str!("setVolume"),
                jni_sig!("(FF)V"),
                &[JValue::Float(volume), JValue::Float(volume)],
            )?
            .v()
        });
    }

    fn apply_looping(&self) {
        let looping = self.looping;
        self.call(|env, player| {
            env.call_method(player, jni_str!("setLooping"), jni_sig!("(Z)V"), &[JValue::Bool(looping)])?
                .v()
        });
    }
}

impl Drop for AndroidVoice {
    fn drop(&mut self) {
        if let Some(prep) = &self.prep {
            let mut state = prep.lock();
            if state.result.is_none() {
                // Still preparing: the helper thread releases it when done.
                state.released = true;
                return;
            }
        }
        let _ = with_env(playback, |env| release(env, &self.player));
    }
}

impl Voice for AndroidVoice {
    fn duration(&self) -> f64 {
        match self.int(jni_str!("getDuration")) {
            // A live stream has no length (-1).
            Some(ms) if ms > 0 => f64::from(ms) / 1000.0,
            _ => f64::NAN,
        }
    }

    fn position(&self) -> f64 {
        if let Some(seconds) = self.pending_seek {
            return seconds;
        }
        self.int(jni_str!("getCurrentPosition"))
            .map_or(0.0, |ms| f64::from(ms.max(0)) / 1000.0)
    }

    fn is_playing(&self) -> bool {
        if !self.is_ready() {
            // A stream asked to play starts once prepared.
            return self.intent && self.failure.borrow().is_none();
        }
        self.call(|env, player| env.call_method(player, jni_str!("isPlaying"), jni_sig!("()Z"), &[])?.z())
            .unwrap_or(false)
    }

    fn play(&mut self) -> Result<(), AudioError> {
        self.intent = true;
        if !self.is_ready() {
            return Ok(());
        }
        self.void(jni_str!("start"));
        match self.failure.borrow().clone() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    fn pause(&mut self) {
        self.intent = false;
        if self.is_ready() {
            self.void(jni_str!("pause"));
        }
    }

    fn stop(&mut self) {
        self.pause();
        self.seek(0.0);
    }

    fn seek(&mut self, seconds: f64) {
        if self.is_ready() {
            self.seek_now(seconds);
        } else {
            self.pending_seek = Some(seconds);
        }
    }

    fn set_volume(&mut self, volume: f32) {
        self.volume = volume;
        if self.is_ready() {
            self.apply_volume();
        }
    }

    fn set_extra_loops(&mut self, extra: i64) {
        self.looping = extra < 0;
        if self.is_ready() {
            self.apply_looping();
        }
    }

    fn is_buffering(&self) -> bool {
        self.intent && !self.is_ready()
    }

    fn buffered(&self) -> f64 {
        if self.streaming {
            // `MediaPlayer` reports buffering only to a Java listener.
            f64::NAN
        } else {
            self.duration()
        }
    }

    fn error(&self) -> Option<AudioError> {
        self.failure.borrow().clone()
    }

    fn poll(&mut self) {
        if let Some(prep) = &self.prep {
            let Some(result) = prep.lock().result.take() else {
                return;
            };
            self.prep = None;
            if let Err(error) = result {
                *self.failure.borrow_mut() = Some(error);
                return;
            }
            // Apply what was asked for while it prepared.
            self.apply_volume();
            self.apply_looping();
            if let Some(seconds) = self.pending_seek.take() {
                self.seek_now(seconds);
            }
            if self.intent {
                self.void(jni_str!("start"));
            }
            return;
        }
        // A stream that stopped by itself well before its end lost its
        // connection; `MediaPlayer` only tells a Java listener so.
        if self.streaming && self.intent && !self.is_playing() && self.failure.borrow().is_none() {
            let position = self.int(jni_str!("getCurrentPosition")).unwrap_or(0);
            let duration = self.int(jni_str!("getDuration")).unwrap_or(-1);
            if duration <= 0 || position < duration - EARLY_STOP_MS {
                *self.failure.borrow_mut() = Some(AudioError::Network("the stream stopped".into()));
            }
        }
    }
}
