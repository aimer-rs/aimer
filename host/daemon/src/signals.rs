//! Turning the operating system's "please stop" into [`DaemonHandle::quit`].
//!
//! * Unix: `SIGTERM`, `SIGINT` and `SIGHUP`. A signal handler may do almost
//!   nothing, so it only writes one byte to a pipe; a dedicated thread reads the
//!   pipe and asks the daemon to quit from ordinary thread context. Unlike
//!   masking the signals in every thread, this does not depend on how many
//!   threads exist when it is installed.
//! * Windows: Ctrl-C, Ctrl-Break and console close.
//! * Elsewhere (mobile): nothing; the OS owns the process lifecycle.
//!
//! The first request asks for a graceful quit. A second one restores the default
//! action, so a wedged daemon can always still be stopped with a second Ctrl-C.

use std::sync::Mutex;

use crate::DaemonHandle;

/// The daemon the next request is delivered to.
static TARGET: Mutex<Option<DaemonHandle>> = Mutex::new(None);

fn set_target(handle: Option<DaemonHandle>) {
    *TARGET.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = handle;
}

/// Mobile and web hosts deliver no requests, so nothing reads the target there.
#[cfg_attr(any(target_os = "android", target_os = "ios", target_arch = "wasm32"), allow(dead_code))]
fn target() -> Option<DaemonHandle> {
    TARGET.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone()
}

/// Routes termination requests to `handle` until [`release`].
pub(crate) fn install(handle: DaemonHandle) {
    set_target(Some(handle));
    imp::install();
}

/// Stops routing requests, so the next one terminates the process as usual.
pub(crate) fn release() {
    set_target(None);
    imp::reset();
}

#[cfg(all(unix, not(any(target_os = "android", target_os = "ios"))))]
mod imp {
    use std::sync::Once;
    use std::sync::atomic::{AtomicI32, AtomicU32, Ordering};

    use super::target;

    const SIGNALS: [libc::c_int; 3] = [libc::SIGTERM, libc::SIGINT, libc::SIGHUP];

    /// Write end of the pipe, read by the handler. `-1` until installed.
    static WRITE_FD: AtomicI32 = AtomicI32::new(-1);
    /// Requests seen since the daemon last started.
    static REQUESTS: AtomicU32 = AtomicU32::new(0);
    static INSTALL: Once = Once::new();

    /// Runs in signal context: only atomics and async-signal-safe calls.
    extern "C" fn on_signal(signal: libc::c_int) {
        if REQUESTS.fetch_add(1, Ordering::SeqCst) > 0 {
            terminate_as_by_default(signal);
            return;
        }
        let fd = WRITE_FD.load(Ordering::Relaxed);
        let byte = signal as u8;
        // SAFETY: `write` is async-signal-safe, and `byte` outlives the call.
        // The pipe is non-blocking, so a full pipe drops the byte instead of
        // blocking; one pending byte is all the reader needs.
        let written = unsafe { libc::write(fd, (&raw const byte).cast(), 1) };
        if written != 1 {
            terminate_as_by_default(signal);
        }
    }

    fn terminate_as_by_default(signal: libc::c_int) {
        // SAFETY: `signal` and `raise` are async-signal-safe; restoring the
        // default action and re-raising ends the process the way it would have
        // ended without this module.
        unsafe {
            libc::signal(signal, libc::SIG_DFL);
            libc::raise(signal);
        }
    }

    pub(super) fn install() {
        INSTALL.call_once(|| {
            if let Err(error) = try_install() {
                // The daemon still works; it just terminates abruptly on signals.
                eprintln!("aimer_daemon: signal handling unavailable: {error}");
            }
        });
        REQUESTS.store(0, Ordering::SeqCst);
    }

    pub(super) fn reset() {
        REQUESTS.store(0, Ordering::SeqCst);
    }

    fn try_install() -> std::io::Result<()> {
        let mut fds = [0 as libc::c_int; 2];
        // SAFETY: `fds` has room for the two descriptors `pipe` writes.
        if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        let [read_fd, write_fd] = fds;
        for fd in fds {
            // SAFETY: `fd` is a descriptor this function just created.
            unsafe {
                let flags = libc::fcntl(fd, libc::F_GETFD);
                libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC);
            }
        }
        // SAFETY: `write_fd` is a valid descriptor.
        unsafe {
            let flags = libc::fcntl(write_fd, libc::F_GETFL);
            libc::fcntl(write_fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
        }
        WRITE_FD.store(write_fd, Ordering::SeqCst);

        std::thread::Builder::new()
            .name("aimer-daemon-signals".to_owned())
            .spawn(move || read_requests(read_fd))?;

        for signal in SIGNALS {
            // SAFETY: a zeroed `sigaction` is valid to fill in; the handler is a
            // plain `extern "C"` function that only does signal-safe work.
            unsafe {
                let mut action: libc::sigaction = std::mem::zeroed();
                action.sa_sigaction = on_signal as extern "C" fn(libc::c_int) as usize;
                action.sa_flags = libc::SA_RESTART;
                libc::sigemptyset(&mut action.sa_mask);
                libc::sigaction(signal, &action, std::ptr::null_mut());
            }
        }
        Ok(())
    }

    fn read_requests(read_fd: libc::c_int) {
        loop {
            let mut byte = 0u8;
            // SAFETY: `byte` is a valid one-byte buffer.
            let read = unsafe { libc::read(read_fd, (&raw mut byte).cast(), 1) };
            if read == 1 {
                match target() {
                    Some(handle) => handle.quit(),
                    None => terminate_as_by_default(byte as libc::c_int),
                }
            } else if read < 0 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                continue;
            } else {
                return;
            }
        }
    }
}

#[cfg(target_os = "windows")]
mod imp {
    use std::sync::atomic::{AtomicU32, Ordering};

    use windows_sys::Win32::Foundation::{FALSE, TRUE};
    use windows_sys::Win32::System::Console::{
        CTRL_BREAK_EVENT, CTRL_C_EVENT, CTRL_CLOSE_EVENT, SetConsoleCtrlHandler,
    };
    use windows_sys::core::BOOL;

    use super::target;

    static REQUESTS: AtomicU32 = AtomicU32::new(0);

    /// Runs on a thread Windows creates for the purpose, so taking locks is fine.
    unsafe extern "system" fn on_ctrl(kind: u32) -> BOOL {
        if !matches!(kind, CTRL_C_EVENT | CTRL_BREAK_EVENT | CTRL_CLOSE_EVENT) {
            return FALSE;
        }
        let Some(handle) = target() else { return FALSE };
        // A second request falls through to the default handler, which ends
        // the process.
        if REQUESTS.fetch_add(1, Ordering::SeqCst) > 0 {
            return FALSE;
        }
        handle.quit();
        TRUE
    }

    pub(super) fn install() {
        REQUESTS.store(0, Ordering::SeqCst);
        // SAFETY: `on_ctrl` has the signature `SetConsoleCtrlHandler` expects.
        // Registering the same routine twice is harmless: it is a no-op.
        unsafe { SetConsoleCtrlHandler(Some(on_ctrl), TRUE) };
    }

    pub(super) fn reset() {
        REQUESTS.store(0, Ordering::SeqCst);
    }
}

#[cfg(not(any(
    all(unix, not(any(target_os = "android", target_os = "ios"))),
    target_os = "windows"
)))]
mod imp {
    pub(super) fn install() {}
    pub(super) fn reset() {}
}
