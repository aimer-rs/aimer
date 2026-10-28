use std::ffi::c_void;
use std::marker::PhantomData;
use std::ptr::NonNull;
use std::rc::Rc;

use winit::window::{Window, WindowAttributes};

type WindowCreated = Box<dyn for<'window> FnOnce(MacosWindowHandle<'window>)>;

#[derive(Clone, Copy, Debug, PartialEq)]
struct TrafficLightPosition {
    x: f64,
    y: f64,
}

#[cfg(any(target_os = "macos", test))]
#[derive(Default)]
struct TrafficLightLayoutState {
    deferred_reapplication: bool,
}

#[cfg(any(target_os = "macos", test))]
impl TrafficLightLayoutState {
    #[inline]
    fn native_layout_changed(&mut self) {
        self.deferred_reapplication = true;
    }

    #[inline]
    fn take_deferred_reapplication(&mut self) -> bool {
        std::mem::take(&mut self.deferred_reapplication)
    }
}

#[cfg(any(target_os = "macos", test))]
struct TrafficLightLayout {
    titlebar_height: f64,
    titlebar_y: f64,
    button_origins: [(f64, f64); 3],
}

#[cfg(any(target_os = "macos", test))]
fn traffic_light_layout(
    position: TrafficLightPosition,
    window_height: f64,
    button_height: f64,
    native_button_x: [f64; 3],
) -> TrafficLightLayout {
    let minimize_offset = native_button_x[1] - native_button_x[0];
    let zoom_offset = native_button_x[2] - native_button_x[0];
    let titlebar_height = button_height + position.y.max(0.0) * 2.0;
    let button_y = titlebar_height - button_height - position.y;

    TrafficLightLayout {
        titlebar_height,
        titlebar_y: window_height - titlebar_height,
        button_origins: [
            (position.x, button_y),
            (position.x + minimize_offset, button_y),
            (position.x + zoom_offset, button_y),
        ],
    }
}

/// Where the window's outer origin has to be, in physical pixels, for the point
/// of it that the pointer took hold of — `grab`, in logical points from the
/// window's top-left corner — to be under the pointer now at `cursor`, reported
/// in the same space relative to the window where it is now.
///
/// The pointer's position is relative to the window and the window is what is
/// being moved, so the correction is the difference between where the pointer
/// is and where it took hold, applied to wherever the window is.
fn dragged_origin(outer: (i32, i32), grab: (f32, f32), cursor: (f32, f32), scale: f64) -> (i32, i32) {
    let shift = |outer: i32, grab: f32, cursor: f32| {
        (f64::from(outer) + f64::from(cursor - grab) * scale).round() as i32
    };
    (shift(outer.0, grab.0, cursor.0), shift(outer.1, grab.1, cursor.1))
}

/// Picks the height of the strip a press drags the window from: what the
/// application asked for, else the strip the traffic lights were laid out in,
/// else the standard titlebar. A height of zero leaves no strip at all.
#[cfg(any(target_os = "macos", test))]
fn resolve_drag_band(
    requested: Option<f64>,
    traffic_lights: Option<f64>,
    standard: f64,
) -> Option<f64> {
    let height = requested.or(traffic_lights).unwrap_or(standard);
    (height > 0.0).then_some(height)
}

/// Declarative macOS window customization installed during Aimer startup.
///
/// Build the desired configuration and call [`Self::install`] from
/// [`AimerApp::setup`](https://docs.rs/aimer/latest/aimer/struct.AimerApp.html#method.setup).
/// Aimer applies creation-time attributes before creating its first window and
/// then invokes the optional [`Self::on_created`] callback with the underlying
/// `NSWindow` pointer.
///
/// The type and all of its builder methods are available on every platform. On
/// platforms other than macOS, installing it has no effect, so application code
/// does not need conditional compilation around common window configuration.
///
/// # Examples
///
/// ```no_run
/// use aimer_native::macos_windowing::MacosWindowing;
///
/// let windowing = MacosWindowing::new()
///     .titlebar_transparent(true)
///     .title_hidden(true)
///     .fullsize_content_view(true)
///     .movable_by_window_background(true)
///     .traffic_light_position(16.0, 14.0);
///
/// windowing.install();
/// ```
pub struct MacosWindowing {
    movable_by_window_background: Option<bool>,
    titlebar_transparent: Option<bool>,
    title_hidden: Option<bool>,
    titlebar_hidden: Option<bool>,
    titlebar_buttons_hidden: Option<bool>,
    fullsize_content_view: Option<bool>,
    has_shadow: Option<bool>,
    accepts_first_mouse: Option<bool>,
    traffic_light_position: Option<TrafficLightPosition>,
    native_titlebar_drag: Option<bool>,
    titlebar_drag_height: Option<f64>,
    /// The band measured from the created window, in logical points; see
    /// [`Self::window_drag_height`].
    drag_band: Option<f64>,
    /// The point of the window, in logical points from its top-left corner,
    /// that the pointer took hold of; present while the framework is moving
    /// the window.
    window_drag: Option<(f32, f32)>,
    #[cfg(any(target_os = "macos", test))]
    traffic_light_layout_state: TrafficLightLayoutState,
    on_created: Option<WindowCreated>,
}

impl MacosWindowing {
    /// Creates a configuration that leaves every `winit` default unchanged.
    #[inline]
    pub fn new() -> Self {
        Self {
            movable_by_window_background: None,
            titlebar_transparent: None,
            title_hidden: None,
            titlebar_hidden: None,
            titlebar_buttons_hidden: None,
            fullsize_content_view: None,
            has_shadow: None,
            accepts_first_mouse: None,
            traffic_light_position: None,
            native_titlebar_drag: None,
            titlebar_drag_height: None,
            drag_band: None,
            window_drag: None,
            #[cfg(any(target_os = "macos", test))]
            traffic_light_layout_state: TrafficLightLayoutState::default(),
            on_created: None,
        }
    }

    /// Controls whether dragging the window background moves the window.
    #[inline]
    pub fn movable_by_window_background(mut self, movable: bool) -> Self {
        self.movable_by_window_background = Some(movable);
        self
    }

    /// Controls whether the titlebar draws a background.
    ///
    /// This is commonly combined with [`Self::fullsize_content_view`] so the
    /// application's content is visible behind the titlebar controls.
    #[inline]
    pub fn titlebar_transparent(mut self, transparent: bool) -> Self {
        self.titlebar_transparent = Some(transparent);
        self
    }

    /// Controls whether the text in the titlebar is hidden.
    #[inline]
    pub fn title_hidden(mut self, hidden: bool) -> Self {
        self.title_hidden = Some(hidden);
        self
    }

    /// Controls whether the complete native titlebar is hidden.
    #[inline]
    pub fn titlebar_hidden(mut self, hidden: bool) -> Self {
        self.titlebar_hidden = Some(hidden);
        self
    }

    /// Controls whether the close, minimize, and zoom buttons are hidden.
    #[inline]
    pub fn titlebar_buttons_hidden(mut self, hidden: bool) -> Self {
        self.titlebar_buttons_hidden = Some(hidden);
        self
    }

    /// Controls whether content occupies the full window area behind the titlebar.
    #[inline]
    pub fn fullsize_content_view(mut self, fullsize: bool) -> Self {
        self.fullsize_content_view = Some(fullsize);
        self
    }

    /// Controls whether AppKit draws the standard window shadow.
    #[inline]
    pub fn has_shadow(mut self, has_shadow: bool) -> Self {
        self.has_shadow = Some(has_shadow);
        self
    }

    /// Controls whether the window accepts the first click while inactive.
    #[inline]
    pub fn accepts_first_mouse(mut self, accepts: bool) -> Self {
        self.accepts_first_mouse = Some(accepts);
        self
    }

    /// Positions the standard close, minimize, and zoom buttons on macOS.
    ///
    /// `x` and `y` are logical AppKit points from the top-left corner of the
    /// window to the top-left corner of the close button. The minimize and zoom
    /// buttons keep their native spacing from the close button. Aimer reapplies
    /// the position when AppKit lays out the window again during resizing or a
    /// fullscreen transition.
    ///
    /// This setting has no effect outside macOS. Both coordinates must be
    /// finite; negative coordinates are supported when deliberately placing
    /// the controls partly outside the window.
    ///
    /// # Panics
    ///
    /// Panics if either coordinate is NaN or infinite.
    #[inline]
    pub fn traffic_light_position(mut self, x: f64, y: f64) -> Self {
        assert!(
            x.is_finite() && y.is_finite(),
            "traffic-light coordinates must be finite"
        );
        self.traffic_light_position = Some(TrafficLightPosition { x, y });
        self
    }

    /// Keeps AppKit in charge of dragging the window by its titlebar.
    ///
    /// When [`Self::fullsize_content_view`] puts the content under the titlebar,
    /// Aimer takes window dragging over: AppKit would otherwise move the window
    /// for any press in the titlebar strip before the application could say that
    /// a button or a field there had been pressed instead. Aimer turns AppKit's
    /// dragging off and starts the drag itself, from a press that no element
    /// claimed. Pass `true` to leave AppKit's behavior as it is, at the price of
    /// elements in the strip not being able to take a press from it.
    #[inline]
    pub fn native_titlebar_drag(mut self, native: bool) -> Self {
        self.native_titlebar_drag = Some(native);
        self
    }

    /// Sets the height, in logical points, of the strip at the top of the window
    /// that a press drags the window from when no element claims it.
    ///
    /// The default is the height of the titlebar: the strip the traffic lights
    /// are centred in when [`Self::traffic_light_position`] is set, and
    /// AppKit's standard titlebar height otherwise. It only matters when
    /// [`Self::fullsize_content_view`] is on, which is when Aimer owns window
    /// dragging; see [`Self::native_titlebar_drag`].
    ///
    /// # Panics
    ///
    /// Panics if `height` is negative, NaN or infinite.
    #[inline]
    pub fn titlebar_drag_height(mut self, height: f64) -> Self {
        assert!(
            height.is_finite() && height >= 0.0,
            "titlebar drag height must be finite and not negative"
        );
        self.titlebar_drag_height = Some(height);
        self
    }

    /// Registers advanced one-time customization for the created `NSWindow`.
    ///
    /// The callback runs on Aimer's event-loop thread immediately after window
    /// creation and before rendering begins. It is never invoked off macOS.
    /// The handle is borrowed for the callback and cannot be sent to another
    /// thread or retained beyond the call.
    #[inline]
    pub fn on_created(
        mut self,
        callback: impl for<'window> FnOnce(MacosWindowHandle<'window>) + 'static,
    ) -> Self {
        self.on_created = Some(Box::new(callback));
        self
    }

    /// Installs this configuration for Aimer's next macOS window.
    ///
    /// Call this from `AimerApp::setup`, which runs on the native event-loop
    /// thread before the window is created. If several setup callbacks install
    /// configurations, the last installed value is used.
    #[cfg(target_os = "macos")]
    #[inline]
    pub fn install(self) {
        PENDING.with(|pending| pending.replace(Some(self)));
    }

    /// Does nothing on platforms that do not create AppKit windows.
    #[cfg(not(target_os = "macos"))]
    #[inline]
    pub fn install(self) {}

    /// Whether Aimer, not AppKit, decides when a press in the titlebar strip
    /// drags the window.
    #[inline]
    fn framework_owns_titlebar_drag(&self) -> bool {
        self.fullsize_content_view == Some(true)
            && self.native_titlebar_drag != Some(true)
            && self.movable_by_window_background != Some(true)
    }

    /// The height of the strip, in logical points from the top of the window,
    /// that the framework drags the window from, or `None` when it does not
    /// own dragging.
    ///
    /// A press in the strip that no element consumed or captured starts a
    /// window drag; one an element claimed stays that element's.
    #[doc(hidden)]
    #[inline]
    pub fn window_drag_height(&self) -> Option<f32> {
        if !self.framework_owns_titlebar_drag() {
            return None;
        }
        self.titlebar_drag_height
            .or(self.drag_band)
            .filter(|height| *height > 0.0)
            .map(|height| height as f32)
    }

    /// Starts moving the window with the pointer, which took hold of the point
    /// `grab` of it: logical points from the window's top-left corner.
    ///
    /// The framework moves the window itself rather than handing the press to
    /// AppKit. AppKit decides whether a press drags the window before the press
    /// reaches the content, and it cannot be told afterwards that an element
    /// took it; see [`Self::native_titlebar_drag`].
    #[doc(hidden)]
    #[inline]
    pub fn begin_window_drag(&mut self, grab: (f32, f32)) {
        self.window_drag = Some(grab);
    }

    /// Returns whether the framework is moving the window.
    #[doc(hidden)]
    #[inline]
    pub fn window_drag_active(&self) -> bool {
        self.window_drag.is_some()
    }

    /// Returns the point of the window the pointer took hold of, if it is
    /// being moved.
    #[doc(hidden)]
    #[inline]
    pub fn window_drag_grab(&self) -> Option<(f32, f32)> {
        self.window_drag
    }

    /// Stops moving the window.
    #[doc(hidden)]
    #[inline]
    pub fn end_window_drag(&mut self) {
        self.window_drag = None;
    }

    /// Moves `window` so the point the pointer took hold of is under `cursor`,
    /// which is the pointer's position in logical points from the window's
    /// top-left corner. Does nothing when no drag is active, or when the
    /// platform cannot report or set the window's position.
    #[doc(hidden)]
    pub fn move_window_with_pointer(&self, window: &Window, cursor: (f32, f32)) {
        let Some(grab) = self.window_drag else {
            return;
        };
        let Ok(outer) = window.outer_position() else {
            return;
        };
        let (x, y) = dragged_origin((outer.x, outer.y), grab, cursor, window.scale_factor());
        if (x, y) != (outer.x, outer.y) {
            window.set_outer_position(winit::dpi::PhysicalPosition::new(x, y));
        }
    }

    #[doc(hidden)]
    #[cfg(target_os = "macos")]
    pub fn apply_attributes(&self, mut attributes: WindowAttributes) -> WindowAttributes {
        use winit::platform::macos::WindowAttributesExtMacOS;

        if let Some(value) = self.movable_by_window_background {
            attributes = attributes.with_movable_by_window_background(value);
        }
        if let Some(value) = self.titlebar_transparent {
            attributes = attributes.with_titlebar_transparent(value);
        }
        if let Some(value) = self.title_hidden {
            attributes = attributes.with_title_hidden(value);
        }
        if let Some(value) = self.titlebar_hidden {
            attributes = attributes.with_titlebar_hidden(value);
        }
        if let Some(value) = self.titlebar_buttons_hidden {
            attributes = attributes.with_titlebar_buttons_hidden(value);
        }
        if let Some(value) = self.fullsize_content_view {
            attributes = attributes.with_fullsize_content_view(value);
        }
        if let Some(value) = self.has_shadow {
            attributes = attributes.with_has_shadow(value);
        }
        if let Some(value) = self.accepts_first_mouse {
            attributes = attributes.with_accepts_first_mouse(value);
        }
        attributes
    }

    #[doc(hidden)]
    #[cfg(not(target_os = "macos"))]
    pub fn apply_attributes(&self, attributes: WindowAttributes) -> WindowAttributes {
        attributes
    }

    #[doc(hidden)]
    #[cfg(target_os = "macos")]
    pub fn window_created(&mut self, window: &Window) {
        if self.traffic_light_position.is_none()
            && self.on_created.is_none()
            && !self.framework_owns_titlebar_drag()
        {
            return;
        }
        let Some(native_window) = native_window(window) else {
            return;
        };

        let traffic_light_height = self
            .traffic_light_position
            .and_then(|position| apply_traffic_light_position(&native_window, position));
        if self.framework_owns_titlebar_drag() {
            // AppKit would move the window for any press in the titlebar strip,
            // whatever the application did with it. With dragging off, the press
            // reaches the elements first, and the framework starts the drag from
            // the ones nobody claimed.
            native_window.setMovable(false);
            self.drag_band = resolve_drag_band(
                self.titlebar_drag_height,
                traffic_light_height,
                standard_titlebar_height(&native_window),
            );
        }
        let Some(callback) = self.on_created.take() else {
            return;
        };
        let pointer = NonNull::from(&*native_window).cast::<c_void>();

        callback(MacosWindowHandle {
            pointer,
            _lifetime: PhantomData,
            _main_thread: PhantomData,
        });
    }

    #[doc(hidden)]
    #[cfg(not(target_os = "macos"))]
    pub fn window_created(&mut self, _window: &Window) {}

    #[doc(hidden)]
    #[cfg(target_os = "macos")]
    pub fn window_layout_changed(&mut self, window: &Window) {
        let Some(position) = self.traffic_light_position else {
            return;
        };
        self.traffic_light_layout_state.native_layout_changed();
        let Some(native_window) = native_window(window) else {
            return;
        };

        apply_traffic_light_position(&native_window, position);
    }

    #[doc(hidden)]
    #[cfg(not(target_os = "macos"))]
    pub fn window_layout_changed(&mut self, _window: &Window) {}

    #[doc(hidden)]
    #[cfg(target_os = "macos")]
    pub fn window_redraw_requested(&mut self, window: &Window) {
        if !self
            .traffic_light_layout_state
            .take_deferred_reapplication()
        {
            return;
        }
        let Some(position) = self.traffic_light_position else {
            return;
        };
        let Some(native_window) = native_window(window) else {
            return;
        };

        apply_traffic_light_position(&native_window, position);
    }

    #[doc(hidden)]
    #[cfg(not(target_os = "macos"))]
    pub fn window_redraw_requested(&mut self, _window: &Window) {}
}

#[cfg(target_os = "macos")]
fn native_window(window: &Window) -> Option<objc2::rc::Retained<objc2_app_kit::NSWindow>> {
    use objc2::rc::Retained;
    use objc2_app_kit::NSView;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    let handle = window.window_handle().ok()?;
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return None;
    };

    // SAFETY: `window_handle` guarantees that AppKit's `ns_view` pointer
    // remains valid for the lifetime of the borrowed winit window. Retaining
    // the view keeps it alive while its owning NSWindow is being queried.
    let view = unsafe { Retained::<NSView>::retain(handle.ns_view.as_ptr().cast()) }?;
    view.window()
}

#[cfg(target_os = "macos")]
fn apply_traffic_light_position(
    window: &objc2_app_kit::NSWindow,
    position: TrafficLightPosition,
) -> Option<f64> {
    use objc2_app_kit::NSWindowButton;
    use objc2_foundation::NSPoint;

    let close = window.standardWindowButton(NSWindowButton::CloseButton)?;
    let minimize = window.standardWindowButton(NSWindowButton::MiniaturizeButton)?;
    let zoom = window.standardWindowButton(NSWindowButton::ZoomButton)?;

    // SAFETY: AppKit owns the titlebar hierarchy. The retained views returned
    // here remain valid for this main-thread layout operation.
    let buttons_view = unsafe { close.superview() }?;
    // SAFETY: Same ownership invariant as above; this is the titlebar container
    // that AppKit may recreate or resize during a native window relayout.
    let titlebar_view = unsafe { buttons_view.superview() }?;

    let close_frame = close.frame();
    let minimize_frame = minimize.frame();
    let zoom_frame = zoom.frame();
    let layout = traffic_light_layout(
        position,
        window.frame().size.height,
        close_frame.size.height,
        [
            close_frame.origin.x,
            minimize_frame.origin.x,
            zoom_frame.origin.x,
        ],
    );

    let mut titlebar_frame = titlebar_view.frame();
    titlebar_frame.size.height = layout.titlebar_height;
    titlebar_frame.origin.y = layout.titlebar_y;
    titlebar_view.setFrame(titlebar_frame);

    close.setFrameOrigin(NSPoint {
        x: layout.button_origins[0].0,
        y: layout.button_origins[0].1,
    });
    minimize.setFrameOrigin(NSPoint {
        x: layout.button_origins[1].0,
        y: layout.button_origins[1].1,
    });
    zoom.setFrameOrigin(NSPoint {
        x: layout.button_origins[2].0,
        y: layout.button_origins[2].1,
    });
    Some(layout.titlebar_height)
}

/// The height AppKit gives the titlebar of `window`, which a full-size content
/// view lays out under: what is left of the frame once the content layout
/// rectangle is taken.
#[cfg(target_os = "macos")]
fn standard_titlebar_height(window: &objc2_app_kit::NSWindow) -> f64 {
    (window.frame().size.height - window.contentLayoutRect().size.height).max(0.0)
}

impl Default for MacosWindowing {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

/// A main-thread-only borrowed pointer to the AppKit `NSWindow`.
///
/// The wrapper deliberately exposes only a raw pointer: typed AppKit access is
/// an advanced operation whose safety requirements depend on the Objective-C
/// messages the caller sends. The pointer is non-null and valid only for the
/// duration of the callback passed to [`MacosWindowing::on_created`].
#[derive(Clone, Copy)]
pub struct MacosWindowHandle<'window> {
    pointer: NonNull<c_void>,
    _lifetime: PhantomData<&'window Window>,
    _main_thread: PhantomData<Rc<()>>,
}

impl MacosWindowHandle<'_> {
    /// Returns the borrowed `NSWindow` pointer.
    ///
    /// The caller must not release the object, transfer it to another thread,
    /// or retain the pointer after the callback returns.
    #[inline]
    pub fn as_ptr(self) -> *mut c_void {
        self.pointer.as_ptr()
    }

    /// Returns the non-null borrowed `NSWindow` pointer.
    #[inline]
    pub fn as_non_null(self) -> NonNull<c_void> {
        self.pointer
    }
}

#[cfg(target_os = "macos")]
aimer_tls::thread_local! {
    static PENDING: std::cell::RefCell<Option<MacosWindowing>> = const {
        std::cell::RefCell::new(None)
    };
}

#[doc(hidden)]
#[cfg(target_os = "macos")]
pub fn take_pending() -> MacosWindowing {
    PENDING.with(|pending| pending.take().unwrap_or_default())
}

#[doc(hidden)]
#[cfg(not(target_os = "macos"))]
pub fn take_pending() -> MacosWindowing {
    MacosWindowing::new()
}

#[cfg(test)]
mod tests {
    use super::MacosWindowing;

    #[test]
    fn macos_windowing_defaults_do_not_override_winit() {
        let windowing = MacosWindowing::new();

        assert_eq!(windowing.movable_by_window_background, None);
        assert_eq!(windowing.titlebar_transparent, None);
        assert_eq!(windowing.title_hidden, None);
        assert_eq!(windowing.titlebar_hidden, None);
        assert_eq!(windowing.titlebar_buttons_hidden, None);
        assert_eq!(windowing.fullsize_content_view, None);
        assert_eq!(windowing.has_shadow, None);
        assert_eq!(windowing.accepts_first_mouse, None);
        assert_eq!(windowing.traffic_light_position, None);
        assert!(windowing.on_created.is_none());
    }

    #[test]
    fn macos_windowing_builders_preserve_every_customization() {
        let windowing = MacosWindowing::new()
            .movable_by_window_background(true)
            .titlebar_transparent(true)
            .title_hidden(true)
            .titlebar_hidden(false)
            .titlebar_buttons_hidden(true)
            .fullsize_content_view(true)
            .has_shadow(false)
            .accepts_first_mouse(true)
            .traffic_light_position(16.0, 14.0)
            .on_created(|_| {});

        assert_eq!(windowing.movable_by_window_background, Some(true));
        assert_eq!(windowing.titlebar_transparent, Some(true));
        assert_eq!(windowing.title_hidden, Some(true));
        assert_eq!(windowing.titlebar_hidden, Some(false));
        assert_eq!(windowing.titlebar_buttons_hidden, Some(true));
        assert_eq!(windowing.fullsize_content_view, Some(true));
        assert_eq!(windowing.has_shadow, Some(false));
        assert_eq!(windowing.accepts_first_mouse, Some(true));
        assert_eq!(
            windowing.traffic_light_position,
            Some(super::TrafficLightPosition { x: 16.0, y: 14.0 })
        );
        assert!(windowing.on_created.is_some());
    }

    #[test]
    fn the_framework_does_not_own_window_dragging_by_default() {
        assert_eq!(MacosWindowing::new().window_drag_height(), None);
        assert_eq!(
            MacosWindowing::new()
                .titlebar_drag_height(40.0)
                .window_drag_height(),
            None,
            "a titlebar that is not under the content keeps AppKit's own dragging"
        );
    }

    #[test]
    fn content_under_the_titlebar_hands_dragging_to_the_framework() {
        let windowing = MacosWindowing::new()
            .fullsize_content_view(true)
            .titlebar_drag_height(42.0);

        assert_eq!(windowing.window_drag_height(), Some(42.0));
    }

    #[test]
    fn the_band_is_unknown_until_the_window_has_measured_its_titlebar() {
        assert_eq!(
            MacosWindowing::new()
                .fullsize_content_view(true)
                .window_drag_height(),
            None
        );
    }

    #[test]
    fn an_application_can_keep_appkit_in_charge_of_dragging() {
        let native = MacosWindowing::new()
            .fullsize_content_view(true)
            .titlebar_drag_height(42.0)
            .native_titlebar_drag(true);
        let background = MacosWindowing::new()
            .fullsize_content_view(true)
            .titlebar_drag_height(42.0)
            .movable_by_window_background(true);

        assert_eq!(native.window_drag_height(), None);
        assert_eq!(background.window_drag_height(), None);
    }

    #[test]
    fn the_band_prefers_what_was_asked_then_the_traffic_lights_then_the_standard_titlebar() {
        assert_eq!(super::resolve_drag_band(Some(50.0), Some(42.0), 28.0), Some(50.0));
        assert_eq!(super::resolve_drag_band(None, Some(42.0), 28.0), Some(42.0));
        assert_eq!(super::resolve_drag_band(None, None, 28.0), Some(28.0));
        assert_eq!(
            super::resolve_drag_band(None, None, 0.0),
            None,
            "a window with no titlebar has no band to drag from"
        );
        assert_eq!(super::resolve_drag_band(Some(0.0), Some(42.0), 28.0), None);
    }

    #[test]
    #[should_panic(expected = "titlebar drag height must be finite and not negative")]
    fn the_drag_height_rejects_a_negative_value() {
        let _ = MacosWindowing::new().titlebar_drag_height(-1.0);
    }

    #[test]
    #[should_panic(expected = "titlebar drag height must be finite and not negative")]
    fn the_drag_height_rejects_a_non_finite_value() {
        let _ = MacosWindowing::new().titlebar_drag_height(f64::NAN);
    }

    #[test]
    fn a_dragged_window_keeps_the_grabbed_point_under_the_pointer() {
        // At rest the window stays where it is.
        assert_eq!(
            super::dragged_origin((100, 200), (30.0, 20.0), (30.0, 20.0), 2.0),
            (100, 200)
        );
        // The pointer moved (10, 4) logical points inside the window, which is
        // (20, 8) physical pixels on a 2x display, so the window follows.
        assert_eq!(
            super::dragged_origin((100, 200), (30.0, 20.0), (40.0, 24.0), 2.0),
            (120, 208)
        );
        assert_eq!(
            super::dragged_origin((100, 200), (30.0, 20.0), (25.0, 10.0), 1.0),
            (95, 190)
        );
        assert_eq!(
            super::dragged_origin((0, 0), (0.0, 0.0), (0.4, 0.6), 1.0),
            (0, 1)
        );
    }

    #[test]
    fn a_drag_lasts_from_the_press_to_the_release() {
        let mut windowing = MacosWindowing::new();
        assert!(!windowing.window_drag_active());
        assert_eq!(windowing.window_drag_grab(), None);

        windowing.begin_window_drag((30.0, 20.0));
        assert!(windowing.window_drag_active());
        assert_eq!(windowing.window_drag_grab(), Some((30.0, 20.0)));

        windowing.end_window_drag();
        assert!(!windowing.window_drag_active());
        windowing.end_window_drag();
        assert!(!windowing.window_drag_active(), "ending twice is harmless");
    }

    #[test]
    fn traffic_light_position_accepts_the_window_origin() {
        let windowing = MacosWindowing::new().traffic_light_position(0.0, 0.0);

        assert_eq!(
            windowing.traffic_light_position,
            Some(super::TrafficLightPosition { x: 0.0, y: 0.0 })
        );
    }

    #[test]
    fn traffic_light_layout_uses_top_left_coordinates_and_native_spacing() {
        let layout = super::traffic_light_layout(
            super::TrafficLightPosition { x: 16.0, y: 14.0 },
            780.0,
            14.0,
            [7.0, 27.0, 47.0],
        );

        assert_eq!(layout.titlebar_height, 42.0);
        assert_eq!(layout.titlebar_y, 738.0);
        assert_eq!(
            layout.button_origins,
            [(16.0, 14.0), (36.0, 14.0), (56.0, 14.0)]
        );
    }

    #[test]
    fn traffic_light_layout_supports_zero_insets() {
        let layout = super::traffic_light_layout(
            super::TrafficLightPosition { x: 0.0, y: 0.0 },
            780.0,
            14.0,
            [7.0, 27.0, 47.0],
        );

        assert_eq!(layout.titlebar_height, 14.0);
        assert_eq!(layout.titlebar_y, 766.0);
        assert_eq!(
            layout.button_origins,
            [(0.0, 0.0), (20.0, 0.0), (40.0, 0.0)]
        );
    }

    #[test]
    fn traffic_light_layout_supports_negative_offsets_without_invalid_geometry() {
        let layout = super::traffic_light_layout(
            super::TrafficLightPosition { x: -4.0, y: -10.0 },
            780.0,
            14.0,
            [7.0, 27.0, 47.0],
        );

        assert_eq!(layout.titlebar_height, 14.0);
        assert_eq!(layout.titlebar_y, 766.0);
        assert_eq!(
            layout.button_origins,
            [(-4.0, 10.0), (16.0, 10.0), (36.0, 10.0)]
        );
    }

    #[test]
    fn native_relayout_schedules_exactly_one_deferred_reapplication() {
        let mut state = super::TrafficLightLayoutState::default();

        assert!(!state.take_deferred_reapplication());
        state.native_layout_changed();
        assert!(state.take_deferred_reapplication());
        assert!(!state.take_deferred_reapplication());
    }

    #[test]
    #[should_panic(expected = "traffic-light coordinates must be finite")]
    fn traffic_light_position_rejects_a_non_finite_horizontal_coordinate() {
        let _ = MacosWindowing::new().traffic_light_position(f64::NAN, 14.0);
    }

    #[test]
    #[should_panic(expected = "traffic-light coordinates must be finite")]
    fn traffic_light_position_rejects_a_non_finite_vertical_coordinate() {
        let _ = MacosWindowing::new().traffic_light_position(16.0, f64::INFINITY);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn installing_macos_windowing_makes_it_pending_once() {
        drop(super::take_pending());
        MacosWindowing::new().title_hidden(true).install();

        let installed = super::take_pending();
        assert_eq!(installed.title_hidden, Some(true));
        assert_eq!(super::take_pending().title_hidden, None);
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn installing_macos_windowing_is_a_no_op_on_other_platforms() {
        MacosWindowing::new()
            .titlebar_transparent(true)
            .traffic_light_position(16.0, 14.0)
            .on_created(|_| panic!("a macOS callback must not run on another platform"))
            .install();

        let pending = super::take_pending();
        assert_eq!(pending.titlebar_transparent, None);
        assert_eq!(pending.traffic_light_position, None);
        assert!(pending.on_created.is_none());
    }
}
