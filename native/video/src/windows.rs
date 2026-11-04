//! Windows: WinRT `MediaPlayer` in frame-server mode, frames as shared
//! Direct3D 11 textures.
//!
//! Media Foundation opens, streams (progressive http(s); HLS where the system
//! supports it), decodes and plays the soundtrack in sync on its own threads.
//! With `IsVideoFrameServerEnabled` the player renders no picture itself but
//! raises `VideoFrameAvailable` and copies the current one into any Direct3D 11
//! surface on request (`CopyFrameToVideoSurface`), converting from YUV on the
//! GPU.
//!
//! The surfaces are a small ring of `B8G8R8A8` textures created with an NT
//! shared handle, plus one shared fence. After each copy the pool signals the
//! fence; the renderer's Direct3D 12 backend opens the handles, makes its queue
//! wait for that fence value and samples the texture as it is (see
//! `NativeTexture::Dxgi`). A slot is reused only when no frame names it any
//! more, which is when the renderer has finished with it.
//!
//! Both sides use the system's default hardware adapter. On a machine with
//! several GPUs where the renderer picked another one, the texture cannot be
//! opened and the video draws nothing.

use std::any::Any;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use aimer_cupid::external_texture::NativeTexture;
use windows::Foundation::{TimeSpan, TypedEventHandler, Uri};
use windows::Graphics::DirectX::Direct3D11::IDirect3DSurface;
use windows::Media::Core::MediaSource;
use windows::Media::Playback::{
    IMediaPlaybackSource, MediaPlaybackSession, MediaPlaybackState, MediaPlayer,
    MediaPlayerFailedEventArgs,
};
use windows::Storage::StorageFile;
use windows::Win32::Foundation::{CloseHandle, GENERIC_ALL, HANDLE, HMODULE};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_CREATE_DEVICE_BGRA_SUPPORT,
    D3D11_CREATE_DEVICE_VIDEO_SUPPORT, D3D11_FENCE_FLAG_SHARED, D3D11_RESOURCE_MISC_SHARED,
    D3D11_RESOURCE_MISC_SHARED_NTHANDLE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC,
    D3D11_USAGE_DEFAULT, D3D11CreateDevice, ID3D11Device, ID3D11Device5, ID3D11DeviceContext,
    ID3D11DeviceContext4, ID3D11Fence, ID3D11Multithread, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::{
    DXGI_SHARED_RESOURCE_READ, DXGI_SHARED_RESOURCE_WRITE, IDXGIAdapter, IDXGIResource1, IDXGISurface,
};
use windows::Win32::System::Com::CoIncrementMTAUsage;
use windows::Win32::System::WinRT::Direct3D11::CreateDirect3D11SurfaceFromDXGISurface;
use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};
use windows::core::{HSTRING, IInspectable, Interface, Ref};

use crate::backend::{FrameSlot, PlatformVideo, StoredFrame};
use crate::{VideoError, VideoSource};

/// `TimeSpan` ticks per second (100 ns each).
const TICKS: f64 = 10_000_000.0;
/// Textures a pool grows to. The renderer keeps a frame for four renders after
/// it draws it, and one more is the latest and one is being written.
const RING: usize = 6;

fn describe(error: windows::core::Error) -> String {
    error.message().to_string()
}

fn playback(error: windows::core::Error) -> VideoError {
    VideoError::Playback(describe(error))
}

/// Opens `source` as a paused player.
pub(crate) fn open(source: &VideoSource) -> Result<Box<dyn PlatformVideo>, VideoError> {
    keep_apartment_alive();
    let target = match source {
        VideoSource::File(path) => Target::File(existing(path)?),
        VideoSource::Bundled(key) => {
            let exe = std::env::current_exe().ok();
            let path = crate::assets::bundled_candidates(key, exe.as_deref())
                .into_iter()
                .find(|candidate| candidate.is_file())
                .ok_or_else(|| VideoError::NotFound(key.clone()))?;
            Target::File(path)
        }
        VideoSource::Url(url) => {
            let lower = url.get(..8).unwrap_or(url).to_ascii_lowercase();
            if !(lower.starts_with("http://") || lower.starts_with("https://")) {
                return Err(VideoError::Network(format!("not an http(s) URL: {url}")));
            }
            Target::Url(url.clone())
        }
    };
    // Opening a file waits on an asynchronous operation, which must not block
    // a thread in a single-threaded apartment (a UI thread); a short-lived
    // helper thread in the multithreaded apartment does it. The objects it
    // makes are agile, and `keep_apartment_alive` keeps that apartment from
    // being torn down when the helper ends.
    std::thread::Builder::new()
        .name("aimer-video-open".into())
        .spawn(move || build(&target))
        .map_err(|error| VideoError::Playback(error.to_string()))?
        .join()
        .map_err(|_| VideoError::Playback("opening the video panicked".into()))?
        .map(|video| Box::new(video) as Box<dyn PlatformVideo>)
}

fn existing(path: &Path) -> Result<PathBuf, VideoError> {
    if path.is_file() {
        Ok(path.to_path_buf())
    } else {
        Err(VideoError::NotFound(path.display().to_string()))
    }
}

/// Keeps the process's multithreaded apartment alive without a thread of ours
/// staying in it.
fn keep_apartment_alive() {
    static KEPT: OnceLock<()> = OnceLock::new();
    KEPT.get_or_init(|| {
        // SAFETY: no arguments; the returned cookie is deliberately never
        // released, so the apartment lives as long as the process.
        let _ = unsafe { CoIncrementMTAUsage() };
    });
}

enum Target {
    File(PathBuf),
    Url(String),
}

/// What the player's event handlers report, from a system thread.
#[derive(Default)]
struct Events {
    ended: AtomicBool,
    /// A new picture is waiting to be copied out.
    frame_ready: AtomicBool,
    error: Mutex<Option<String>>,
}

fn build(target: &Target) -> Result<WindowsVideo, VideoError> {
    // SAFETY: the helper thread joins the multithreaded apartment; failing
    // means it already has one, which is fine.
    let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
    let source = match target {
        Target::File(path) => from_file(path)?,
        Target::Url(url) => {
            let uri = Uri::CreateUri(&HSTRING::from(url.as_str()))
                .map_err(|error| VideoError::Network(describe(error)))?;
            MediaSource::CreateFromUri(&uri).map_err(playback)?
        }
    };
    let player = MediaPlayer::new().map_err(playback)?;
    player.SetAutoPlay(false).map_err(playback)?;
    // A player in the app must not take over the media keys.
    player
        .CommandManager()
        .and_then(|manager| manager.SetIsEnabled(false))
        .map_err(playback)?;
    // The player draws no picture; we ask it for each one.
    player.SetIsVideoFrameServerEnabled(true).map_err(playback)?;

    let events = Arc::new(Events::default());
    let frame = events.clone();
    let frame_token = player
        .VideoFrameAvailable(&TypedEventHandler::new(move |_: Ref<MediaPlayer>, _: Ref<IInspectable>| {
            frame.frame_ready.store(true, Ordering::Release);
            Ok(())
        }))
        .map_err(playback)?;
    let ended = events.clone();
    let ended_token = player
        .MediaEnded(&TypedEventHandler::new(move |_: Ref<MediaPlayer>, _: Ref<IInspectable>| {
            ended.ended.store(true, Ordering::Release);
            Ok(())
        }))
        .map_err(playback)?;
    let failed = events.clone();
    let failed_token = player
        .MediaFailed(&TypedEventHandler::new(
            move |_: Ref<MediaPlayer>, args: Ref<MediaPlayerFailedEventArgs>| {
                let message = args
                    .as_ref()
                    .and_then(|args| args.ErrorMessage().ok())
                    .map(|message| message.to_string())
                    .filter(|message| !message.is_empty())
                    .unwrap_or_else(|| "the media could not be played".into());
                *failed.error.lock().unwrap_or_else(PoisonError::into_inner) = Some(message);
                Ok(())
            },
        ))
        .map_err(playback)?;
    let playable: IMediaPlaybackSource = source.cast().map_err(playback)?;
    player.SetSource(&playable).map_err(playback)?;
    let session = player.PlaybackSession().map_err(playback)?;
    Ok(WindowsVideo {
        player,
        session,
        events,
        tokens: [frame_token, ended_token, failed_token],
        intent: AtomicBool::new(false),
        pending_seek: Mutex::new(None),
        pool: Mutex::new(None),
    })
}

fn from_file(path: &Path) -> Result<MediaSource, VideoError> {
    // `StorageFile` wants an absolute path with backslashes.
    let absolute = std::fs::canonicalize(path).map_err(|error| VideoError::NotFound(error.to_string()))?;
    let text = absolute.to_string_lossy();
    // `canonicalize` yields a verbatim `\\?\` path, which WinRT rejects.
    let text = text.strip_prefix(r"\\?\").unwrap_or(&text);
    let file = StorageFile::GetFileFromPathAsync(&HSTRING::from(text))
        .and_then(|operation| operation.join())
        .map_err(|error| VideoError::NotFound(describe(error)))?;
    MediaSource::CreateFromStorageFile(&file).map_err(playback)
}

/// An NT handle closed on drop.
struct OwnedHandle(HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        // SAFETY: the handle was returned by `CreateSharedHandle` and is closed
        // exactly once, here.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

// SAFETY: an NT handle is a process-wide number, usable from any thread.
unsafe impl Send for OwnedHandle {}
unsafe impl Sync for OwnedHandle {}

/// The process's Direct3D 11 device, which every video shares.
struct Gfx {
    device: ID3D11Device,
    device5: ID3D11Device5,
    context: ID3D11DeviceContext4,
}

// SAFETY: the device is created free-threaded and the immediate context is
// switched to multithread-protected mode in `gfx`, so each call is serialised
// by the runtime. Media Foundation uses the same device from its own threads
// for the same reason.
unsafe impl Send for Gfx {}
unsafe impl Sync for Gfx {}

fn gfx() -> Result<Arc<Gfx>, String> {
    static GFX: OnceLock<Result<Arc<Gfx>, String>> = OnceLock::new();
    GFX.get_or_init(|| create_gfx().map(Arc::new).map_err(describe)).clone()
}

fn create_gfx() -> windows::core::Result<Gfx> {
    let levels: [D3D_FEATURE_LEVEL; 2] = [D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0];
    let mut device: Option<ID3D11Device> = None;
    let mut context: Option<ID3D11DeviceContext> = None;
    // SAFETY: out-pointers to locals; no adapter means the default one, as the
    // renderer's `D3D12CreateDevice(None, ..)` does.
    unsafe {
        D3D11CreateDevice(
            None::<&IDXGIAdapter>,
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
            Some(&levels),
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )?;
    }
    let device = device.ok_or_else(windows::core::Error::empty)?;
    let context = context.ok_or_else(windows::core::Error::empty)?;
    // Media Foundation drives this device from its own threads.
    let multithread: ID3D11Multithread = device.cast()?;
    // SAFETY: a plain flag on a valid device.
    let _ = unsafe { multithread.SetMultithreadProtected(true) };
    Ok(Gfx {
        device5: device.cast()?,
        context: context.cast()?,
        device,
    })
}

/// A texture the player copies pictures into, shared with the renderer.
struct Slot {
    index: u32,
    texture: ID3D11Texture2D,
    surface: IDirect3DSurface,
    handle: OwnedHandle,
    /// The pool's fence handle, kept open for as long as a frame names it.
    fence_handle: Arc<OwnedHandle>,
}

// SAFETY: see `Gfx`; the texture and surface are only used under the device's
// multithread protection, and the handle is a plain number.
unsafe impl Send for Slot {}
unsafe impl Sync for Slot {}

/// The textures for one picture size, and the fence that orders them.
struct Pool {
    /// Keeps the pool id live for the renderer's caches.
    registration: aimer_cupid_gpu::external_pool::PoolRegistration,
    size: (u32, u32),
    fence: ID3D11Fence,
    fence_handle: Arc<OwnedHandle>,
    /// The last value signalled.
    value: u64,
    slots: Vec<Arc<Slot>>,
}

// SAFETY: see `Gfx`.
unsafe impl Send for Pool {}

impl Pool {
    fn new(gfx: &Gfx, size: (u32, u32)) -> windows::core::Result<Self> {
        // SAFETY: valid device; the fence is created shared so another
        // device can open it.
        let mut fence: Option<ID3D11Fence> = None;
        unsafe { gfx.device5.CreateFence(0, D3D11_FENCE_FLAG_SHARED, &mut fence)? };
        let fence = fence.ok_or_else(windows::core::Error::empty)?;
        // SAFETY: valid fence; no security attributes and no name.
        let handle = unsafe { fence.CreateSharedHandle(None, GENERIC_ALL.0, None)? };
        Ok(Self {
            registration: aimer_cupid_gpu::external_pool::register(),
            size,
            fence,
            fence_handle: Arc::new(OwnedHandle(handle)),
            value: 0,
            slots: Vec::new(),
        })
    }

    /// A slot no frame names, making one while the ring is not full.
    fn acquire(&mut self, gfx: &Gfx) -> Option<Arc<Slot>> {
        // The pool holds one reference; any more is a frame the renderer (or
        // the player's own latest frame) still uses.
        if let Some(free) = self.slots.iter().find(|slot| Arc::strong_count(slot) == 1) {
            return Some(free.clone());
        }
        if self.slots.len() >= RING {
            return None;
        }
        let slot = Arc::new(self.make_slot(gfx, self.slots.len() as u32).ok()?);
        self.slots.push(slot.clone());
        Some(slot)
    }

    fn make_slot(&self, gfx: &Gfx, index: u32) -> windows::core::Result<Slot> {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: self.size.0,
            Height: self.size.1,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_DEFAULT,
            // The player renders into it; the renderer samples it.
            BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
            CPUAccessFlags: 0,
            MiscFlags: (D3D11_RESOURCE_MISC_SHARED_NTHANDLE.0 | D3D11_RESOURCE_MISC_SHARED.0) as u32,
        };
        let mut texture: Option<ID3D11Texture2D> = None;
        // SAFETY: a valid description and an out-pointer to a local.
        unsafe { gfx.device.CreateTexture2D(&desc, None, Some(&mut texture))? };
        let texture = texture.ok_or_else(windows::core::Error::empty)?;
        let resource: IDXGIResource1 = texture.cast()?;
        // SAFETY: valid resource; no security attributes and no name.
        let handle = unsafe {
            resource.CreateSharedHandle(None, DXGI_SHARED_RESOURCE_READ.0 | DXGI_SHARED_RESOURCE_WRITE.0, None)?
        };
        let handle = OwnedHandle(handle);
        let dxgi: IDXGISurface = texture.cast()?;
        // SAFETY: a valid DXGI surface.
        let surface: IDirect3DSurface = unsafe { CreateDirect3D11SurfaceFromDXGISurface(&dxgi)? }.cast()?;
        Ok(Slot {
            index,
            texture,
            surface,
            handle,
            fence_handle: self.fence_handle.clone(),
        })
    }
}

struct WindowsVideo {
    player: MediaPlayer,
    session: MediaPlaybackSession,
    events: Arc<Events>,
    /// `VideoFrameAvailable`, `MediaEnded` and `MediaFailed`, in that order.
    tokens: [i64; 3],
    /// Whether the player last asked to play. The system's own state lags a
    /// `Play` call, so it cannot tell a start from the end.
    intent: AtomicBool,
    /// A seek asked for before the source opened; applied once it has.
    pending_seek: Mutex<Option<f64>>,
    pool: Mutex<Option<Pool>>,
}

// SAFETY: `MediaPlayer` and its session are agile WinRT objects, usable from
// any thread; the pool is behind a mutex and its D3D 11 objects are used under
// the device's multithread protection (see `Gfx`).
unsafe impl Send for WindowsVideo {}
unsafe impl Sync for WindowsVideo {}

impl WindowsVideo {
    fn state(&self) -> MediaPlaybackState {
        self.session.PlaybackState().unwrap_or(MediaPlaybackState::None)
    }

    fn opened(&self) -> bool {
        !matches!(self.state(), MediaPlaybackState::None | MediaPlaybackState::Opening)
    }

    fn set_position(&self, seconds: f64) {
        let ticks = (seconds.max(0.0) * TICKS) as i64;
        let _ = self.session.SetPosition(TimeSpan { Duration: ticks });
    }

    fn apply_pending_seek(&self) {
        if !self.opened() {
            return;
        }
        let pending = self.pending_seek.lock().unwrap_or_else(PoisonError::into_inner).take();
        if let Some(seconds) = pending {
            self.set_position(seconds);
        }
    }

    fn fail(&self, why: &str) {
        let mut error = self.events.error.lock().unwrap_or_else(PoisonError::into_inner);
        error.get_or_insert_with(|| why.to_string());
    }
}

impl Drop for WindowsVideo {
    fn drop(&mut self) {
        let _ = self.player.RemoveVideoFrameAvailable(self.tokens[0]);
        let _ = self.player.RemoveMediaEnded(self.tokens[1]);
        let _ = self.player.RemoveMediaFailed(self.tokens[2]);
        let _ = self.player.Close();
    }
}

impl PlatformVideo for WindowsVideo {
    fn play(&self) {
        self.events.ended.store(false, Ordering::Release);
        self.intent.store(true, Ordering::Release);
        // Before the source opens the player starts as soon as it is ready.
        let _ = self.player.Play();
    }

    fn pause(&self) {
        self.intent.store(false, Ordering::Release);
        let _ = self.player.Pause();
    }

    fn seek(&self, seconds: f64) {
        self.events.ended.store(false, Ordering::Release);
        if self.opened() {
            self.set_position(seconds);
        } else {
            *self.pending_seek.lock().unwrap_or_else(PoisonError::into_inner) = Some(seconds);
        }
    }

    fn set_volume(&self, volume: f32) {
        let _ = self.player.SetVolume(f64::from(volume));
    }

    fn is_ready(&self) -> bool {
        self.opened()
    }

    fn position(&self) -> f64 {
        if let Some(seconds) = *self.pending_seek.lock().unwrap_or_else(PoisonError::into_inner) {
            return seconds;
        }
        self.session
            .Position()
            .map_or(0.0, |span| (span.Duration as f64 / TICKS).max(0.0))
    }

    fn duration(&self) -> f64 {
        if !self.opened() {
            return f64::NAN;
        }
        match self.session.NaturalDuration() {
            // A live stream reports no length.
            Ok(span) if span.Duration > 0 => span.Duration as f64 / TICKS,
            _ => f64::NAN,
        }
    }

    fn buffered(&self) -> f64 {
        let duration = self.duration();
        if !duration.is_finite() {
            return self.position();
        }
        // A local file reports 1.0 at once; a stream counts up.
        duration * self.session.DownloadProgress().unwrap_or(0.0).clamp(0.0, 1.0)
    }

    fn is_playing(&self) -> bool {
        self.intent.load(Ordering::Acquire) && !self.events.ended.load(Ordering::Acquire)
    }

    fn is_buffering(&self) -> bool {
        self.intent.load(Ordering::Acquire)
            && matches!(self.state(), MediaPlaybackState::Buffering | MediaPlaybackState::Opening)
    }

    fn error(&self) -> Option<VideoError> {
        let error = self.events.error.lock().unwrap_or_else(PoisonError::into_inner);
        error.clone().map(VideoError::Decode)
    }

    fn size(&self) -> Option<(u32, u32)> {
        if !self.opened() {
            return None;
        }
        let width = self.session.NaturalVideoWidth().ok()?;
        let height = self.session.NaturalVideoHeight().ok()?;
        (width > 0 && height > 0).then_some((width as u32, height as u32))
    }

    fn take_frame(&self, frames: &FrameSlot) -> bool {
        self.apply_pending_seek();
        if !self.events.frame_ready.swap(false, Ordering::AcqRel) {
            return false;
        }
        // Whatever stops this attempt, the picture is still waiting.
        let retry = || self.events.frame_ready.store(true, Ordering::Release);
        let Some(size) = self.size() else {
            retry();
            return false;
        };
        let gfx = match gfx() {
            Ok(gfx) => gfx,
            Err(why) => {
                self.fail(&format!("Direct3D 11 is not available: {why}"));
                return false;
            }
        };
        let mut guard = self.pool.lock().unwrap_or_else(PoisonError::into_inner);
        if guard.as_ref().is_none_or(|pool| pool.size != size) {
            // A new size starts a new pool; frames of the old one keep their
            // slots alive through their own references.
            match Pool::new(&gfx, size) {
                Ok(pool) => *guard = Some(pool),
                Err(error) => {
                    self.fail(&format!("could not create a shared texture: {}", describe(error)));
                    return false;
                }
            }
        }
        let Some(pool) = guard.as_mut() else {
            return false;
        };
        let Some(slot) = pool.acquire(&gfx) else {
            // Every texture is still in use by the renderer: try again soon.
            retry();
            return false;
        };
        // SAFETY: the surface is a B8G8R8A8 render-target texture of the
        // picture's size on a device Media Foundation can use.
        if self.player.CopyFrameToVideoSurface(&slot.surface).is_err() {
            retry();
            return false;
        }
        pool.value += 1;
        // SAFETY: valid context and fence of the same device; the signal
        // follows the copy in the context's order.
        if unsafe { gfx.context.Signal(&pool.fence, pool.value) }.is_err() {
            retry();
            return false;
        }
        let texture = NativeTexture::Dxgi {
            pool: pool.registration.id(),
            slot: slot.index,
            texture: slot.handle.0.0 as usize,
            fence: pool.fence_handle.0.0 as usize,
            value: pool.value,
        };
        let owner: Arc<dyn Any + Send + Sync> = slot;
        // SAFETY: `owner` keeps the slot's texture and both handles open for
        // as long as the frame lives, and the slot is not written again until
        // no frame holds it.
        let frame = unsafe { StoredFrame::new(texture, size.0, size.1, owner) };
        frames.store(Some(frame));
        true
    }
}
