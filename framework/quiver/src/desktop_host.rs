//! The event loop of a desktop application with more than one window.
//!
//! One loop, one UI thread, one scheduler and one async runtime serve every
//! window. What belongs to a window stays with it: its renderer, its widget
//! tree, its input state, what it has recorded for its next frame. The host's
//! job is to know which window an event is for, to make that window the active
//! one while its code runs (see [`aimer_events::window::enter_window_scope`]),
//! and to decide what closing a window means for the application.
//!
//! Mobile and web platforms have a single surface and keep the loop in
//! `handler.rs`; the host is for macOS, Windows and Linux.

use std::any::Any;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::AtomicU8;

use aimer_events::window::{
    WindowKey, WindowScopeConfig, WindowScopeRegistration, enter_window_scope,
    register_window_scope_with,
};
use aimer_rubick::UiMemory;
use aimer_venus::Venus;
use aimer_widget::{AnyWidget, Widget};
use aimer_widget::base::WindowHandle;
use tokio::runtime::Handle;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::window::WindowId;

use crate::aimer_app::{
    AimerNativePlatformEvent, EVENT_PROXY, complete_frame_ready_request,
    current_frame_request_kind, frame_ready_delivered, observe_frame_request,
    try_begin_frame_ready_request,
};
use crate::handler::event_handler::WindowEventHandler;
use crate::handler::user_events::{handle_user_event, request_window_redraw};
use crate::handler::{AimerApplicationHandler, StartupHook, WindowRenderTree, run_startup_hooks};
use crate::window_attr::WindowAttr;
use crate::window_locals::WindowThreadState;
use aimer_window::WindowId as AppWindowId;
use aimer_window::host::{AfterClose, OpenRequest, Request, WindowCommand};
use aimer_window::ExitPolicy;
use aimer_cupid::AntiAlias;
use aimer_utils::info;

/// A window's application, with every window's root widget erased to one type
/// so a single list can hold them all.
pub(crate) type WindowApp = AimerApplicationHandler<AnyWidget>;

/// What every window of the application shares.
pub(crate) struct Shared {
    pub(crate) venus: Rc<Venus>,
    pub(crate) async_handle: Handle,
    pub(crate) ui_memory: UiMemory,
    pub(crate) antialiasing: AntiAlias,
    pub(crate) render_mode: crate::RenderMode,
}

impl Shared {
    /// Builds the application of one window, not yet on screen.
    pub(crate) fn window_app(&self, attr: WindowAttr, root: AnyWidget) -> WindowApp {
        #[cfg(all(target_os = "windows", feature = "native", not(feature = "wgpu")))]
        let show_window_after_first_frame = attr.is_visible();
        AimerApplicationHandler {
            window: None,
            hosted: true,
            closed: false,
            inherited_states: None,
            macos_windowing: Default::default(),
            render_ctx: crate::render_ctx::configured_context(self.antialiasing, self.render_mode),
            ui_memory: self.ui_memory.clone(),
            #[cfg(debug_assertions)]
            debug_overlay: Default::default(),
            window_attr: attr,
            render_tree: WindowRenderTree::default(),
            #[cfg(all(target_os = "windows", feature = "native", not(feature = "wgpu")))]
            show_window_after_first_frame,
            widget_root: None,
            event_dispatcher: aimer_widget::EventDispatcher::new(),
            scroll_smoother: crate::handler::scroll_classifier::DualScroller::new(),
            frame_request_reason: Rc::new(std::cell::Cell::new(None)),
            pending_widget: Some(root),
            cursor_pos: crate::handler::event_handler::CURSOR_OUTSIDE_POSITION,
            touch_pointer: false,
            pressed_button: None,
            current_modifiers: Default::default(),
            ime_composing: false,
            window_scale: 1.0,
            native_window_size: None,
            pending_resize: None,
            startup_hooks: Vec::new(),
            startup_resources: Vec::new(),
            async_handle: self.async_handle.clone(),
            active_touch_id: None,
            venus: self.venus.clone(),
            file_drag: crate::handler::file_drag::FileDrag::new(),
            #[cfg(feature = "wasm-hot-reload")]
            live_reload: None,
        }
    }
}

/// One window of the host.
struct HostedWindow {
    id: WindowId,
    key: WindowKey,
    app: WindowApp,
    /// The id the application knows this window by.
    app_id: AppWindowId,
    /// Called once the window has closed, however it closed.
    on_closed: Option<Box<dyn FnOnce()>>,
    /// Whether this is the window the application started with.
    is_main: bool,
    /// The one frame this window has asked for and not yet been given, so a
    /// burst of requests costs the event loop a single wake.
    frame_slot: Arc<AtomicU8>,
    scope: WindowScopeRegistration,
    /// Whether the log has already said this window drew.
    drew: bool,
}

impl HostedWindow {
    /// Takes the window off the screen, drops what it held, and lets go of it.
    ///
    /// The platform window is owned by reference counting, and is destroyed by
    /// whoever drops the last reference. After the application, its renderer
    /// and its scope are gone, this is normally the last one; when something
    /// still holds the window the log says so, and it is destroyed as soon as
    /// that lets go. It is hidden either way.
    fn retire(self) {
        let Self { key, app, scope, .. } = self;
        let window = app.native_window().cloned();
        // Before the window is hidden and destroyed, not after: see the helper
        // for what the animation costs when it has not finished.
        #[cfg(target_os = "macos")]
        if let Some(window) = &window {
            aimer_platform::ffi_utils::macos_window::skip_close_animation(window);
        }
        if let Some(window) = &window {
            window.set_visible(false);
        }
        {
            // The tree's teardown may ask for frames or record repaints; those
            // belong to this window while it is still registered.
            let _active = enter_window_scope(key);
            drop(app);
        }
        drop(scope);
        if let Some(window) = window {
            let others = Arc::strong_count(&window) - 1;
            if others == 0 {
                info!("window {:?} released", window.id());
            } else {
                info!(
                    "window {:?} is still held by {others} other owner(s); it is destroyed when they let go",
                    window.id()
                );
            }
        }
    }
}

/// The application loop for a desktop application.
pub(crate) struct DesktopHost {
    windows: Vec<HostedWindow>,
    /// The window the application starts with, until the loop creates it.
    main: Option<WindowApp>,
    shared: Shared,
    policy: ExitPolicy,
    startup_hooks: Vec<StartupHook>,
    startup_resources: Vec<Box<dyn Any>>,
    /// Whether the windows have been created. The loop may resume more than
    /// once, and the application starts only once.
    started: bool,
    /// The window that last had keyboard focus, which platform input that names
    /// no window is delivered to.
    focused: Option<WindowId>,
}

impl DesktopHost {
    pub(crate) fn new(
        shared: Shared,
        main: WindowApp,
        policy: ExitPolicy,
        startup_hooks: Vec<StartupHook>,
    ) -> Self {
        Self {
            windows: Vec::new(),
            main: Some(main),
            shared,
            policy,
            startup_hooks,
            startup_resources: Vec::new(),
            started: false,
            focused: None,
        }
    }

    fn index_of(&self, id: WindowId) -> Option<usize> {
        self.windows.iter().position(|window| window.id == id)
    }

    /// The window input that names none is delivered to: the focused one, or
    /// failing that the oldest.
    fn input_target(&mut self) -> Option<&mut HostedWindow> {
        let index = self
            .focused
            .and_then(|id| self.index_of(id))
            .or(if self.windows.is_empty() { None } else { Some(0) })?;
        self.windows.get_mut(index)
    }

    /// Creates `app`'s platform window and brings it up as one of the host's.
    fn open(
        &mut self,
        event_loop: &ActiveEventLoop,
        mut app: WindowApp,
        app_id: AppWindowId,
        on_closed: Option<Box<dyn FnOnce()>>,
        is_main: bool,
    ) {
        app.create_native_window(event_loop);
        let Some(native) = app.native_window() else {
            return;
        };
        let key = WindowHandle::native(native.clone()).key();
        let id = native.id();
        let frame_slot = Arc::new(AtomicU8::new(0));
        let scope = register_window_scope_with(
            key,
            WindowScopeConfig::new(window_requester(id, frame_slot.clone()), {
                let reason = app.frame_request_reason.clone();
                move || observe_frame_request(&reason)
            })
            .native(native.clone())
            .locals(Rc::new(WindowThreadState::default())),
        );
        self.windows.push(HostedWindow {
            id,
            key,
            app,
            app_id,
            on_closed,
            is_main,
            frame_slot,
            scope,
            drew: false,
        });
        aimer_window::host::mark_open(app_id);
        info!("window {id:?} opened ({} open)", self.windows.len());
        let window = self.windows.last_mut().expect("the window was just added");
        let _active = enter_window_scope(key);
        window.app.resume_window(event_loop);
    }

    /// Opens the windows the application asked for, carries out what it asked
    /// of the ones it has, and settles whatever that ended.
    fn process_requests(&mut self, event_loop: &ActiveEventLoop) {
        for request in aimer_window::host::drain() {
            match request {
                Request::Open(open) => self.open_requested(event_loop, open),
                Request::Command(id, command) => self.command(event_loop, id, command),
            }
        }
        self.reap(event_loop);
    }

    fn open_requested(&mut self, event_loop: &ActiveEventLoop, open: OpenRequest) {
        let OpenRequest {
            id,
            attr,
            root,
            inherited,
            on_closed,
        } = open;
        let root = aimer_modal::ModalHost::new().child(root).boxed();
        let mut app = self.shared.window_app(attr, root);
        app.inherited_states = inherited;
        self.open(event_loop, app, id, on_closed, false);
    }

    /// Carries out one request for a window that may or may not still exist.
    fn command(&mut self, event_loop: &ActiveEventLoop, id: AppWindowId, command: WindowCommand) {
        let Some(window) = self.windows.iter_mut().find(|window| window.app_id == id) else {
            return;
        };
        let _active = enter_window_scope(window.key);
        info!("window {:?}: {command:?}", window.id);
        if matches!(command, WindowCommand::Close) {
            WindowEventHandler::close_window(&mut window.app, event_loop);
            return;
        }
        let Some(native) = window.app.native_window() else {
            return;
        };
        match command {
            WindowCommand::Close => {}
            WindowCommand::Focus => native.focus_window(),
            WindowCommand::SetTitle(title) => native.set_title(&title),
            WindowCommand::Resize(width, height) => {
                // A refusal is the platform's answer: the window may be
                // maximized, or fixed in size.
                let _ = native.request_inner_size(winit::dpi::LogicalSize::new(width, height));
            }
            WindowCommand::SetMinimized(minimized) => native.set_minimized(minimized),
            WindowCommand::SetMaximized(maximized) => native.set_maximized(maximized),
        }
    }

    /// Runs one frame of the scheduler for an application with no window.
    ///
    /// Tasks are polled by the frames of windows, so with none open — a
    /// menu-bar application between its windows — nothing would poll them: a
    /// timer would fire into the void and a request would never be answered.
    /// The same phases a window frame runs are run here, minus everything that
    /// needs a window.
    fn run_scheduler_without_windows(&self) {
        let venus = &self.shared.venus;
        venus.begin_frame();
        venus.run_frame_tasks();
        venus.run_microtasks();
        let budget = venus.idle_budget();
        venus.run_idle(&budget);
        venus.end_frame();
        if venus.has_ready_work() {
            aimer_events::window::request_animation_frame();
        }
    }

    /// Removes the windows that finished closing and applies the exit policy.
    fn reap(&mut self, event_loop: &ActiveEventLoop) {
        let mut closed_any = false;
        let mut main_closed = false;
        let mut index = 0;
        while index < self.windows.len() {
            if self.windows[index].app.closed {
                let mut window = self.windows.remove(index);
                main_closed |= window.is_main;
                if self.focused == Some(window.id) {
                    self.focused = None;
                }
                let id = window.id;
                let app_id = window.app_id;
                let on_closed = window.on_closed.take();
                window.retire();
                aimer_window::host::mark_closed(app_id);
                // After the window is gone, so the callback sees it that way.
                if let Some(on_closed) = on_closed {
                    on_closed();
                }
                closed_any = true;
                info!("window {id:?} closed ({} left)", self.windows.len());
            } else {
                index += 1;
            }
        }
        if !closed_any {
            return;
        }
        match self.policy.after_close(self.windows.len(), main_closed) {
            AfterClose::Nothing => {}
            AfterClose::Exit => {
                info!("last window closed: leaving the event loop");
                event_loop.exit();
            }
            AfterClose::CloseRemaining => {
                for window in &mut self.windows {
                    let _active = enter_window_scope(window.key);
                    WindowEventHandler::close_window(&mut window.app, event_loop);
                }
                // Windows whose renderer was already idle are done at once.
                self.reap(event_loop);
            }
        }
    }
}

/// Asks the loop for `id`'s next frame, once however often it is asked.
///
/// Runs on the UI thread, inside the window's scope, so the reason the frame
/// is wanted (scrolling, a compositor animation, anything else) is the one the
/// code that asked has marked.
fn window_requester(id: WindowId, slot: Arc<AtomicU8>) -> impl Fn() + 'static {
    move || {
        if !try_begin_frame_ready_request(&slot, current_frame_request_kind()) {
            crate::frame_stats::record_frame_request_coalesced();
            return;
        }
        crate::frame_stats::record_frame_request_accepted();
        let sent = EVENT_PROXY.get().is_some_and(|proxy| {
            proxy
                .send_event(AimerNativePlatformEvent::WindowFrameReady(id))
                .is_ok()
        });
        if !sent {
            complete_frame_ready_request(&slot);
        }
    }
}

impl ApplicationHandler<AimerNativePlatformEvent> for DesktopHost {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        // Before the first frame builds the widget tree: the link that launched
        // the app has to be waiting when the navigator is created.
        crate::link_inbox::deliver_posted_links();
        if self.started {
            for window in &mut self.windows {
                let _active = enter_window_scope(window.key);
                window.app.resume_window(event_loop);
            }
            return;
        }
        self.started = true;
        run_startup_hooks(&mut self.startup_hooks, &mut self.startup_resources);
        if let Some(main) = self.main.take() {
            self.open(event_loop, main, AppWindowId::MAIN, None, true);
        }
        // Windows asked for before the loop ran.
        self.process_requests(event_loop);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let Some(index) = self.index_of(id) else {
            return;
        };
        if matches!(event, WindowEvent::Focused(true)) {
            self.focused = Some(id);
        }
        {
            let window = &mut self.windows[index];
            if !window.drew && matches!(event, WindowEvent::RedrawRequested) {
                window.drew = true;
                info!("window {id:?} is drawing its first frame");
            }
            let _active = enter_window_scope(window.key);
            WindowEventHandler::handle_events(&mut window.app, event_loop, id, event);
        }
        self.reap(event_loop);
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: AimerNativePlatformEvent) {
        match event {
            AimerNativePlatformEvent::ShutdownRequested => {
                // `exiting` follows, which tells the app's services to finish.
                event_loop.exit();
            }
            AimerNativePlatformEvent::WindowRequests => self.process_requests(event_loop),
            AimerNativePlatformEvent::WindowFrameReady(id) => {
                if let Some(window) = self.index_of(id).map(|index| &mut self.windows[index]) {
                    let kind = complete_frame_ready_request(&window.frame_slot);
                    crate::frame_stats::record_display_tick();
                    let _active = enter_window_scope(window.key);
                    window.app.note_frame_request(kind);
                    request_window_redraw(&mut window.app, kind);
                }
            }
            AimerNativePlatformEvent::FrameReady => {
                // A wake nobody attributed to a window — a task finishing on
                // another thread. Which window the task belongs to is known
                // only once it is polled, so every window is offered the frame.
                let kind = frame_ready_delivered();
                if self.windows.is_empty() {
                    self.run_scheduler_without_windows();
                    // Anything it started — a window asked for, a quit — is
                    // already waiting in its queue.
                    self.process_requests(event_loop);
                }
                for window in &mut self.windows {
                    let _active = enter_window_scope(window.key);
                    window.app.note_frame_request(kind);
                    request_window_redraw(&mut window.app, kind);
                }
            }
            AimerNativePlatformEvent::RenderWorkerReady => {
                // The worker that finished is not named, and checking a window
                // for completions costs a queue poll.
                for window in &mut self.windows {
                    let _active = enter_window_scope(window.key);
                    // A window held back until its first frame is hidden, and a
                    // hidden window never receives WM_PAINT, so the redraw that
                    // completion delivery requests would never produce that
                    // frame. Render it directly; `render` delivers the
                    // completions itself.
                    #[cfg(all(target_os = "windows", feature = "native", not(feature = "wgpu")))]
                    if window.app.show_window_after_first_frame {
                        window.app.render(event_loop);
                        continue;
                    }
                    window.app.deliver_render_completions(event_loop);
                }
                self.reap(event_loop);
            }
            AimerNativePlatformEvent::Back(signal) => {
                // Only Android reports a step of the system back, and no
                // desktop window is closed by one.
                let _ = crate::handler::system_back::handle_signal(signal);
            }
            other => {
                if let Some(window) = self.input_target() {
                    let _active = enter_window_scope(window.key);
                    handle_user_event(&mut window.app, other, request_window_redraw);
                }
            }
        }
    }

    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        // First, while the app is still fully alive: the last warning before
        // the process can be killed on a platform that does that.
        aimer_venus::notify_lifecycle(aimer_venus::Lifecycle::Background);
        for window in &mut self.windows {
            window.app.render_ctx.suspend();
        }
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        // Every way out of the loop ends here: a window close, a quit command,
        // and the platform's own terminate notification.
        aimer_venus::notify_lifecycle(aimer_venus::Lifecycle::Exit);
        for window in &mut self.windows {
            window.app.render_ctx.suspend();
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        for window in &mut self.windows {
            #[cfg(target_os = "macos")]
            {
                let _active = enter_window_scope(window.key);
                if let Some(native) = window.app.window.as_ref().and_then(WindowHandle::native_window) {
                    window.app.macos_windowing.window_redraw_requested(native);
                }
                // A file drag is the one gesture the platform stops describing
                // once it has begun: macOS delivers no cursor motion at all
                // while its own drag session runs, so the position is asked for
                // here, and a frame in turn.
                if window.app.file_drag.is_active() {
                    WindowEventHandler::poll_file_drag(&mut window.app);
                    window.app.request_full_redraw();
                }
            }
            #[cfg(not(target_os = "macos"))]
            let _ = window;
        }
    }
}
