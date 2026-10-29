use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

use winit::window::Window;

#[cfg(any(target_os = "ios", test))]
mod uikit;

#[doc(hidden)]
#[derive(Debug)]
pub struct HeadlessWindowState {
    width: AtomicU32,
    height: AtomicU32,
    scale_factor: AtomicU64,
    redraw_requested: AtomicBool,
    redraw_request_count: AtomicU64,
    coalesced_redraw_count: AtomicU64,
    display_tick_count: AtomicU64,
    cursor: AtomicUsize,
}

#[inline]
const fn cursor_id(cursor: winit::window::CursorIcon) -> usize {
    cursor as usize
}

#[inline]
fn cursor_from_id(id: usize) -> winit::window::CursorIcon {
    match id {
        0 => winit::window::CursorIcon::Default,
        1 => winit::window::CursorIcon::ContextMenu,
        2 => winit::window::CursorIcon::Help,
        3 => winit::window::CursorIcon::Pointer,
        4 => winit::window::CursorIcon::Progress,
        5 => winit::window::CursorIcon::Wait,
        6 => winit::window::CursorIcon::Cell,
        7 => winit::window::CursorIcon::Crosshair,
        8 => winit::window::CursorIcon::Text,
        9 => winit::window::CursorIcon::VerticalText,
        10 => winit::window::CursorIcon::Alias,
        11 => winit::window::CursorIcon::Copy,
        12 => winit::window::CursorIcon::Move,
        13 => winit::window::CursorIcon::NoDrop,
        14 => winit::window::CursorIcon::NotAllowed,
        15 => winit::window::CursorIcon::Grab,
        16 => winit::window::CursorIcon::Grabbing,
        17 => winit::window::CursorIcon::EResize,
        18 => winit::window::CursorIcon::NResize,
        19 => winit::window::CursorIcon::NeResize,
        20 => winit::window::CursorIcon::NwResize,
        21 => winit::window::CursorIcon::SResize,
        22 => winit::window::CursorIcon::SeResize,
        23 => winit::window::CursorIcon::SwResize,
        24 => winit::window::CursorIcon::WResize,
        25 => winit::window::CursorIcon::EwResize,
        26 => winit::window::CursorIcon::NsResize,
        27 => winit::window::CursorIcon::NeswResize,
        28 => winit::window::CursorIcon::NwseResize,
        29 => winit::window::CursorIcon::ColResize,
        30 => winit::window::CursorIcon::RowResize,
        31 => winit::window::CursorIcon::AllScroll,
        32 => winit::window::CursorIcon::ZoomIn,
        33 => winit::window::CursorIcon::ZoomOut,
        34 => winit::window::CursorIcon::DndAsk,
        35 => winit::window::CursorIcon::AllResize,
        _ => winit::window::CursorIcon::Default,
    }
}

#[derive(Clone, Debug)]
pub enum WindowHandle {
    /// A platform window, owned by whoever holds a handle to it: the window is
    /// destroyed when the last handle is dropped.
    Native(Arc<Window>),
    Headless(Arc<HeadlessWindowState>),
    #[cfg(feature = "portable-guest")]
    Portable(Arc<HeadlessWindowState>),
}

impl WindowHandle {
    pub fn native(window: Arc<Window>) -> Self {
        Self::Native(window)
    }

    pub fn headless(size: winit::dpi::PhysicalSize<u32>, scale_factor: f64) -> Self {
        Self::Headless(Arc::new(HeadlessWindowState {
            width: AtomicU32::new(size.width),
            height: AtomicU32::new(size.height),
            scale_factor: AtomicU64::new(scale_factor.to_bits()),
            redraw_requested: AtomicBool::new(false),
            redraw_request_count: AtomicU64::new(0),
            coalesced_redraw_count: AtomicU64::new(0),
            display_tick_count: AtomicU64::new(0),
            cursor: AtomicUsize::new(cursor_id(winit::window::CursorIcon::Default)),
        }))
    }

    #[cfg(feature = "portable-guest")]
    #[doc(hidden)]
    pub fn portable() -> Self {
        Self::Portable(Arc::new(HeadlessWindowState {
            width: AtomicU32::new(0),
            height: AtomicU32::new(0),
            scale_factor: AtomicU64::new(1.0_f64.to_bits()),
            redraw_requested: AtomicBool::new(false),
            redraw_request_count: AtomicU64::new(0),
            coalesced_redraw_count: AtomicU64::new(0),
            display_tick_count: AtomicU64::new(0),
            cursor: AtomicUsize::new(cursor_id(winit::window::CursorIcon::Default)),
        }))
    }

    /// Returns the drawable client area in physical pixels.
    ///
    /// On iOS this uses the UIView's local bounds and content scale, including
    /// safe areas, which widgets handle separately. Winit 0.30 projects its
    /// sizes into screen coordinates; those include window transforms during
    /// live resize and do not describe the drawable's pixel dimensions.
    pub fn inner_size(&self) -> winit::dpi::PhysicalSize<u32> {
        match self {
            #[cfg(all(not(aimer_portable_guest), not(target_os = "ios")))]
            Self::Native(window) => window.inner_size(),
            #[cfg(all(not(aimer_portable_guest), target_os = "ios"))]
            Self::Native(window) => uikit::inner_size(window),
            #[cfg(aimer_portable_guest)]
            Self::Native(_) => Default::default(),
            Self::Headless(state) => winit::dpi::PhysicalSize::new(
                state.width.load(Ordering::Relaxed),
                state.height.load(Ordering::Relaxed),
            ),
            #[cfg(feature = "portable-guest")]
            Self::Portable(state) => winit::dpi::PhysicalSize::new(
                state.width.load(Ordering::Relaxed),
                state.height.load(Ordering::Relaxed),
            ),
        }
    }

    pub fn scale_factor(&self) -> f64 {
        match self {
            #[cfg(not(aimer_portable_guest))]
            Self::Native(window) => window.scale_factor(),
            #[cfg(aimer_portable_guest)]
            Self::Native(_) => 1.0,
            Self::Headless(state) => f64::from_bits(state.scale_factor.load(Ordering::Relaxed)),
            #[cfg(feature = "portable-guest")]
            Self::Portable(state) => f64::from_bits(state.scale_factor.load(Ordering::Relaxed)),
        }
    }

    /// The key this window answers to in the thread's frame scheduling.
    ///
    /// Stable for as long as the window lives, and distinct from every other
    /// window alive at the same time: it is the address of the platform window
    /// or of the state the handle's clones share.
    #[inline]
    pub fn key(&self) -> aimer_events::window::WindowKey {
        let address = match self {
            Self::Native(window) => Arc::as_ptr(window) as usize,
            Self::Headless(state) => Arc::as_ptr(state) as usize,
            #[cfg(feature = "portable-guest")]
            Self::Portable(state) => Arc::as_ptr(state) as usize,
        };
        aimer_events::window::WindowKey::from_address(address)
    }

    pub fn request_redraw(&self) {
        crate::frame_work_stats::record_redraw_request();
        aimer_events::window::notify_redraw_requested();
        match self {
            #[cfg(not(aimer_portable_guest))]
            Self::Native(window) => window.request_redraw(),
            #[cfg(aimer_portable_guest)]
            Self::Native(_) => {},
            Self::Headless(state) => Self::request_headless_redraw(state),
            #[cfg(feature = "portable-guest")]
            Self::Portable(_) => {}
        }
    }

    /// Request the next frame through the platform's frame scheduler.
    ///
    /// This is the appropriate request for asynchronous work, such as a
    /// completed future or image decode. Native applications may route it
    /// through an event-loop proxy, while the web target routes it through
    /// the browser frame callback. [`Self::request_redraw`] remains the
    /// direct redraw operation used by event-loop handling code.
    pub fn request_animation_frame(&self) {
        crate::frame_work_stats::record_redraw_request();
        aimer_events::window::notify_redraw_requested();
        match self {
            #[cfg(not(aimer_portable_guest))]
            Self::Native(_) => aimer_events::window::request_animation_frame(),
            #[cfg(aimer_portable_guest)]
            Self::Native(_) => {},
            Self::Headless(state) => Self::request_headless_redraw(state),
            #[cfg(feature = "portable-guest")]
            Self::Portable(_) => {}
        }
    }

    #[inline]
    fn request_headless_redraw(state: &HeadlessWindowState) {
        if state.redraw_requested.swap(true, Ordering::AcqRel) {
            state.coalesced_redraw_count.fetch_add(1, Ordering::Relaxed);
        } else {
            state.redraw_request_count.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn set_cursor(&self, cursor: winit::window::CursorIcon) {
        match self {
            #[cfg(not(aimer_portable_guest))]
            Self::Native(window) => window.set_cursor(cursor),
            #[cfg(aimer_portable_guest)]
            Self::Native(_) => {},
            Self::Headless(state) => {
                state.cursor.store(cursor_id(cursor), Ordering::Relaxed);
            }
            #[cfg(feature = "portable-guest")]
            Self::Portable(_) => {}
        }
    }

    #[doc(hidden)]
    pub fn headless_cursor(&self) -> Option<winit::window::CursorIcon> {
        match self {
            Self::Native(_) => None,
            Self::Headless(state) => Some(cursor_from_id(state.cursor.load(Ordering::Relaxed))),
            #[cfg(feature = "portable-guest")]
            Self::Portable(_) => None,
        }
    }

    pub fn set_text_cursor(&self) {
        self.set_cursor(winit::window::CursorIcon::Text);
    }

    pub fn set_pointer_cursor(&self) {
        self.set_cursor(winit::window::CursorIcon::Pointer);
    }

    pub fn reset_cursor(&self) {
        self.set_cursor(winit::window::CursorIcon::Default);
    }

    pub fn native_window(&self) -> Option<&Arc<Window>> {
        match self {
            #[cfg(not(aimer_portable_guest))]
            Self::Native(window) => Some(window),
            #[cfg(aimer_portable_guest)]
            Self::Native(_) => None,
            Self::Headless(_) => None,
            #[cfg(feature = "portable-guest")]
            Self::Portable(_) => None,
        }
    }

    pub fn update_headless_metrics(&self, size: winit::dpi::PhysicalSize<u32>, scale_factor: f64) {
        match self {
            Self::Headless(state) => {
                state.width.store(size.width, Ordering::Relaxed);
                state.height.store(size.height, Ordering::Relaxed);
                state
                    .scale_factor
                    .store(scale_factor.to_bits(), Ordering::Relaxed);
            }
            #[cfg(feature = "portable-guest")]
            Self::Portable(state) => {
                state.width.store(size.width, Ordering::Relaxed);
                state.height.store(size.height, Ordering::Relaxed);
                state
                    .scale_factor
                    .store(scale_factor.to_bits(), Ordering::Relaxed);
            }
            Self::Native(_) => {}
        }
    }

    pub fn take_redraw_request(&self) -> bool {
        match self {
            Self::Native(_) => false,
            Self::Headless(state) => {
                let pending = state.redraw_requested.swap(false, Ordering::AcqRel);
                if pending {
                    state.display_tick_count.fetch_add(1, Ordering::Relaxed);
                }
                pending
            }
            #[cfg(feature = "portable-guest")]
            Self::Portable(_) => false,
        }
    }

    /// Returns headless redraw-request counters for native acceptance probes.
    #[doc(hidden)]
    pub fn headless_redraw_request_counts(&self) -> Option<(u64, u64, u64)> {
        match self {
            Self::Headless(state) => Some((
                state.redraw_request_count.load(Ordering::Relaxed),
                state.coalesced_redraw_count.load(Ordering::Relaxed),
                state.display_tick_count.load(Ordering::Relaxed),
            )),
            Self::Native(_) => None,
            #[cfg(feature = "portable-guest")]
            Self::Portable(_) => None,
        }
    }
}

