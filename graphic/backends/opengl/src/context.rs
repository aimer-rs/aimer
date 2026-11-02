use std::any::Any;
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::Arc;

use raw_window_handle::{HasDisplayHandle, HasWindowHandle};

use super::gl::GlFns;

#[cfg(target_os = "linux")]
#[path = "context/linux.rs"]
mod platform;
#[cfg(target_os = "android")]
#[path = "context/linux.rs"]
mod platform;
#[cfg(target_os = "windows")]
#[path = "context/windows.rs"]
mod platform;

pub(super) struct NativeContext {
    platform: platform::Context,
    _window: Box<dyn Any>,
    // OpenGL contexts are current on one thread at a time; keeping this marker
    // makes accidental Send/Sync implementations impossible.
    _thread_affinity: PhantomData<Rc<()>>,
}

impl NativeContext {
    pub fn new<W>(window: Arc<W>, size: (u32, u32)) -> Result<Self, String>
    where
        W: HasDisplayHandle + HasWindowHandle + Send + Sync + 'static,
    {
        let display = window
            .display_handle()
            .map_err(|error| format!("get OpenGL display handle: {error}"))?
            .as_raw();
        let raw_window = window
            .window_handle()
            .map_err(|error| format!("get OpenGL window handle: {error}"))?
            .as_raw();
        let platform = platform::Context::new(display, raw_window, size)?;
        Ok(Self {
            platform,
            _window: Box::new(window),
            _thread_affinity: PhantomData,
        })
    }

    pub fn new_headless() -> Result<Self, String> {
        Ok(Self {
            platform: platform::Context::new_headless()?,
            _window: Box::new(()),
            _thread_affinity: PhantomData,
        })
    }

    pub fn make_current(&self) -> Result<(), String> {
        self.platform.make_current()
    }

    pub fn swap_buffers(&self) -> Result<(), String> {
        self.platform.swap_buffers()
    }

    pub fn resize(&self, size: (u32, u32)) -> Result<(), String> {
        self.platform.resize(size)
    }

    pub fn load_gl(&self) -> Result<GlFns, String> {
        // SAFETY: the platform resolver returns addresses belonging to the
        // current context or its OpenGL implementation. The context is owned
        // by this object and remains alive for the lifetime of the function table.
        unsafe { GlFns::load(&|name| self.platform.proc_address(name)) }
    }
}
