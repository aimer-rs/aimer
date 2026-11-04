//! Linux: a minimal ALSA playback device, loaded at run time.
//!
//! `libasound.so.2` is opened with `dlopen` rather than linked, so building
//! needs no ALSA development package and a machine without it gets an error
//! at the first load instead of a binary that will not start. On a desktop the
//! `default` device routes through PipeWire or PulseAudio.

use std::ffi::{CStr, CString, c_char, c_int, c_long, c_uint, c_ulong, c_void};
use std::sync::OnceLock;

type Handle = *mut c_void;

/// `SND_PCM_STREAM_PLAYBACK`.
const STREAM_PLAYBACK: c_int = 0;
/// `SND_PCM_FORMAT_S16_LE`.
const FORMAT_S16_LE: c_int = 2;
/// `SND_PCM_ACCESS_RW_INTERLEAVED`.
const ACCESS_RW_INTERLEAVED: c_int = 3;

/// The handful of `libasound` entry points the output uses.
struct Library {
    open: unsafe extern "C" fn(*mut Handle, *const c_char, c_int, c_int) -> c_int,
    set_params:
        unsafe extern "C" fn(Handle, c_int, c_int, c_uint, c_uint, c_int, c_uint) -> c_int,
    writei: unsafe extern "C" fn(Handle, *const c_void, c_ulong) -> c_long,
    recover: unsafe extern "C" fn(Handle, c_int, c_int) -> c_int,
    prepare: unsafe extern "C" fn(Handle) -> c_int,
    drop: unsafe extern "C" fn(Handle) -> c_int,
    close: unsafe extern "C" fn(Handle) -> c_int,
    strerror: unsafe extern "C" fn(c_int) -> *const c_char,
}

/// Loads `libasound` once per process; the library stays loaded for good.
fn library() -> Result<&'static Library, String> {
    static LIBRARY: OnceLock<Result<Library, String>> = OnceLock::new();
    LIBRARY.get_or_init(load).as_ref().map_err(Clone::clone)
}

fn load() -> Result<Library, String> {
    // SAFETY: `dlopen` takes a NUL-terminated name; the handle is never closed,
    // so every symbol below stays valid for the life of the process.
    let lib = unsafe { libc::dlopen(c"libasound.so.2".as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
    if lib.is_null() {
        return Err("libasound.so.2 (ALSA) is not installed".into());
    }
    let symbol = |name: &CStr| -> Result<*mut c_void, String> {
        // SAFETY: `lib` is a live handle and `name` is NUL-terminated.
        let found = unsafe { libc::dlsym(lib, name.as_ptr()) };
        if found.is_null() {
            Err(format!("libasound has no {}", name.to_string_lossy()))
        } else {
            Ok(found)
        }
    };
    // SAFETY: each symbol is the documented ALSA function whose C signature the
    // field type spells out.
    unsafe {
        Ok(Library {
            open: function(symbol(c"snd_pcm_open")?),
            set_params: function(symbol(c"snd_pcm_set_params")?),
            writei: function(symbol(c"snd_pcm_writei")?),
            recover: function(symbol(c"snd_pcm_recover")?),
            prepare: function(symbol(c"snd_pcm_prepare")?),
            drop: function(symbol(c"snd_pcm_drop")?),
            close: function(symbol(c"snd_pcm_close")?),
            strerror: function(symbol(c"snd_strerror")?),
        })
    }
}

/// Reinterprets a `dlsym` result as the function pointer type `F`.
///
/// # Safety
///
/// `F` must be an `extern "C"` function pointer type whose signature matches
/// the C function at `symbol`.
unsafe fn function<F: Copy>(symbol: *mut c_void) -> F {
    const { assert!(size_of::<F>() == size_of::<*mut c_void>()) };
    // SAFETY: same size (checked above); the caller vouches for the signature.
    unsafe { std::mem::transmute_copy(&symbol) }
}

fn describe(lib: &Library, code: c_int) -> String {
    // SAFETY: `snd_strerror` returns a static NUL-terminated string.
    let text = unsafe { CStr::from_ptr((lib.strerror)(code)) };
    text.to_string_lossy().into_owned()
}

/// An open, interleaved, signed 16-bit playback device.
pub(crate) struct Pcm {
    lib: &'static Library,
    handle: Handle,
    channels: usize,
}

// SAFETY: an ALSA PCM handle may move between threads as long as only one
// uses it at a time; `Pcm` is owned by the output thread alone.
unsafe impl Send for Pcm {}

impl Pcm {
    /// Opens `device` (for example `default`) at `rate` with `channels`, asking
    /// for about `latency_us` of buffering. ALSA's plug layer converts to
    /// whatever the hardware takes.
    pub fn open(device: &str, rate: u32, channels: usize, latency_us: u32) -> Result<Self, String> {
        let lib = library()?;
        let name = CString::new(device).map_err(|_| format!("bad ALSA device name: {device}"))?;
        let mut handle: Handle = std::ptr::null_mut();
        // SAFETY: `handle` receives a new PCM on success; `name` outlives the call.
        let code = unsafe { (lib.open)(&mut handle, name.as_ptr(), STREAM_PLAYBACK, 0) };
        if code < 0 {
            return Err(format!("cannot open ALSA device {device}: {}", describe(lib, code)));
        }
        let pcm = Self {
            lib,
            handle,
            channels,
        };
        // SAFETY: `handle` is the PCM just opened.
        let code = unsafe {
            (lib.set_params)(
                handle,
                FORMAT_S16_LE,
                ACCESS_RW_INTERLEAVED,
                channels as c_uint,
                rate,
                1, // let ALSA resample if the device cannot run at `rate`
                latency_us,
            )
        };
        if code < 0 {
            return Err(format!("cannot configure ALSA device {device}: {}", describe(lib, code)));
        }
        Ok(pcm)
    }

    /// Writes interleaved frames, blocking while the device buffer is full.
    /// An underrun or a suspend is recovered from once before giving up.
    pub fn write(&mut self, samples: &[i16]) -> Result<(), String> {
        let mut rest = samples;
        while !rest.is_empty() {
            let frames = (rest.len() / self.channels) as c_ulong;
            // SAFETY: `rest` holds `frames` whole interleaved frames.
            let written = unsafe { (self.lib.writei)(self.handle, rest.as_ptr().cast(), frames) };
            if written < 0 {
                // SAFETY: `handle` is open; `written` is the negative errno.
                let code = unsafe { (self.lib.recover)(self.handle, written as c_int, 1) };
                if code < 0 {
                    return Err(describe(self.lib, code));
                }
                continue;
            }
            rest = &rest[written as usize * self.channels..];
        }
        Ok(())
    }

    /// Stops at once, discarding what is buffered; [`Pcm::prepare`] resumes.
    pub fn stop(&mut self) {
        // SAFETY: `handle` is open.
        unsafe { (self.lib.drop)(self.handle) };
    }

    /// Readies a stopped device for writing again.
    pub fn prepare(&mut self) {
        // SAFETY: `handle` is open.
        unsafe { (self.lib.prepare)(self.handle) };
    }
}

impl Drop for Pcm {
    fn drop(&mut self) {
        // SAFETY: `handle` is open and is not used again.
        unsafe { (self.lib.close)(self.handle) };
    }
}
