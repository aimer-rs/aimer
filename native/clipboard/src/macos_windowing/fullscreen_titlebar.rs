//! Makes the titlebar that macOS shows over a full-screen window invisible, and keeps the traffic lights on screen.
//!
//! In full screen macOS hides the titlebar, and shows it with the traffic lights when the pointer reaches the top
//! edge. It is not part of the window: AppKit makes a window for it (an `NSToolbarFullScreenWindow`) whose content
//! view holds the titlebar, whose `NSTitlebarBackgroundView` draws a translucent gray backdrop over the top of the
//! application's content: it is hidden on the desktop, where the titlebar is transparent, and shown again in full screen.
//! Making the window's own titlebar transparent does not reach it. It is made clear here, its content is hidden, and the
//! window is made to let the mouse through, since it covers the top of the application and is clicked in its place.
//!
//! The lights are in that window, and so are shown only while it is. When the application says where they go, the
//! titlebar's own lights are hidden while the window is full screen and lights of its own are put in the window's frame
//! view, at that place, which keeps them there whatever the pointer does. They are new buttons and not the titlebar's moved:
//! AppKit still counts the titlebar's as its own and lays them out again whenever it shows the titlebar, in a place that is
//! not this one. The lights go when the window leaves full screen, and the titlebar's are shown again.

use std::cell::{Cell, RefCell};
use std::ptr::NonNull;
use std::rc::Rc;

use block2::RcBlock;
use objc2::rc::{Retained, Weak};
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{AllocAnyThread, DefinedClass, MainThreadOnly, class, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSButton, NSTrackingArea, NSTrackingAreaOptions, NSView, NSViewFrameDidChangeNotification, NSWindow,
    NSWindowButton, NSWindowDidEnterFullScreenNotification, NSWindowStyleMask, NSWindowWillEnterFullScreenNotification,
    NSWindowWillExitFullScreenNotification,
};
use objc2_foundation::{MainThreadMarker, NSArray, NSNotification, NSNotificationCenter, NSObjectProtocol, NSPoint, NSRect, NSSize};

use super::TrafficLightPosition;
use super::traffic_lights::ButtonPlacement;

/// The class of the window that AppKit shows the full-screen titlebar in.
const TITLEBAR_WINDOW: &str = "NSToolbarFullScreenWindow";
/// The classes of the views that draw its backdrop: the background view of the titlebar, which holds the blur it is
/// drawn with, and the effect views beside it.
const BACKDROPS: [&str; 2] = ["NSTitlebarBackgroundView", "NSVisualEffectView"];

/// What AppKit puts before the name of a class when something observes an object of it: the object is then of a subclass
/// with that name.
const OBSERVED: &str = "NSKVONotifying_";

/// The class of an object with the name of the class AppKit made it a subclass of to observe it taken off.
fn class_without_observing(class: &str) -> &str {
    class.strip_prefix(OBSERVED).unwrap_or(class)
}

/// Whether a window of the class `class` is the one the full-screen titlebar is in.
fn is_titlebar_window(class: &str) -> bool {
    class_without_observing(class) == TITLEBAR_WINDOW
}

/// Whether a view of the class `class` draws the backdrop of the full-screen titlebar.
fn is_backdrop(class: &str) -> bool {
    BACKDROPS.contains(&class_without_observing(class))
}

/// How much larger than the lights the group they are in is, on every side: where the pointer is inside the group.
const GROUP_MARGIN: f64 = 4.0;

/// Where the group of lights is, how large, and where each light is in it.
#[derive(Clone, Copy, Debug, PartialEq)]
struct GroupLayout {
    /// The corner of the group in its frame view: its top-left in one that counts y down from the top, and its bottom-left
    /// in AppKit's usual one.
    origin: (f64, f64),
    size: (f64, f64),
    /// The corner of each light in the group, which counts y up as every view that is not flipped does.
    lights: [(f64, f64); 3],
}

/// The group of lights for a frame view `frame_height` high, for lights of `button_size`. In a frame view whose y axis
/// starts at the top (`flipped`) the distance from the top is the y; in one whose y axis starts at the bottom, which is
/// AppKit's usual, it is the height less the distance less the group's own.
fn group_layout(placement: &ButtonPlacement, frame_height: f64, button_size: (f64, f64), flipped: bool) -> GroupLayout {
    let (width, height) = button_size;
    let size = (placement.x[2] - placement.x[0] + width + 2.0 * GROUP_MARGIN, height + 2.0 * GROUP_MARGIN);
    let top = placement.top() - GROUP_MARGIN;
    let y = if flipped { top } else { frame_height - top - size.1 };
    GroupLayout {
        origin: (placement.x[0] - GROUP_MARGIN, y),
        size,
        lights: placement.x.map(|x| (GROUP_MARGIN + x - placement.x[0], GROUP_MARGIN)),
    }
}

/// A move smaller than this is not one: AppKit rounds to the pixel grid.
const SAME: f64 = 0.01;

/// Whether a light that is at `current` is somewhere else than `target`.
fn is_elsewhere(target: (f64, f64), current: (f64, f64)) -> bool {
    (current.0 - target.0).abs() > SAME || (current.1 - target.1).abs() > SAME
}

/// The lights, in the order that they are in the group.
const LIGHT_CLOSE: usize = 0;
const LIGHT_MINIMIZE: usize = 1;
const LIGHT_ZOOM: usize = 2;

/// Whether the light `light` of the group is enabled, which is what makes it colored and show its glyph: all are gray with
/// the pointer away, and with it over the group, close and zoom are, and minimize is not, since a window in full screen
/// cannot be minimized.
fn light_enabled(light: usize, hovering: bool) -> bool {
    hovering && light != LIGHT_MINIMIZE
}

/// What the group of lights knows: whether the pointer is over it, and the lights that it is told to show.
struct GroupIvars {
    hovering: Cell<bool>,
    lights: RefCell<Vec<Retained<NSButton>>>,
}

define_class!(
    // SAFETY: `NSView` has no subclassing requirements that this does not meet, and the class neither implements `Drop` nor
    // is used from any thread but the main one.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "AimerTrafficLightGroup"]
    #[ivars = GroupIvars]
    struct LightsGroup;

    impl LightsGroup {
        /// What a window button asks of the view it is in to know whether to show its glyph: the titlebar answers it with
        /// whether the pointer is over any of the three, so that all three show theirs together.
        #[unsafe(method(_mouseInGroup:))]
        fn mouse_in_group(&self, _button: Option<&AnyObject>) -> bool {
            self.ivars().hovering.get()
        }

        #[unsafe(method(mouseEntered:))]
        fn mouse_entered(&self, _event: Option<&AnyObject>) {
            self.set_hovering(true);
        }

        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, _event: Option<&AnyObject>) {
            self.set_hovering(false);
        }
    }
);

impl LightsGroup {
    /// A group of the lights, which tells them when the pointer comes and goes.
    fn new(mtm: MainThreadMarker, frame: NSRect) -> Retained<Self> {
        let this = mtm.alloc::<Self>().set_ivars(GroupIvars { hovering: Cell::new(false), lights: RefCell::new(Vec::new()) });
        // SAFETY: `initWithFrame:` of `NSView` takes a rectangle.
        let group: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: frame] };
        let options = NSTrackingAreaOptions::MouseEnteredAndExited | NSTrackingAreaOptions::ActiveAlways | NSTrackingAreaOptions::InVisibleRect;
        // SAFETY: the area is owned by the view and the view is its owner, which AppKit asks for the events; the rectangle
        // is ignored for an area that follows the visible rectangle.
        let area = unsafe {
            NSTrackingArea::initWithRect_options_owner_userInfo(NSTrackingArea::alloc(), NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(0.0, 0.0)), options, Some(&group), None)
        };
        group.addTrackingArea(&area);
        group
    }

    /// Puts the lights in the group, as gray as they are with the pointer away.
    fn add_lights(&self, lights: &[Retained<NSButton>]) {
        for (index, light) in lights.iter().enumerate() {
            self.addSubview(light);
            light.setEnabled(light_enabled(index, self.ivars().hovering.get()));
        }
        *self.ivars().lights.borrow_mut() = lights.to_vec();
    }

    /// Says whether the pointer is over the group, and has the lights colored and with their glyphs, or gray.
    fn set_hovering(&self, hovering: bool) {
        if self.ivars().hovering.replace(hovering) != hovering {
            for (index, light) in self.ivars().lights.borrow().iter().enumerate() {
                light.setEnabled(light_enabled(index, hovering));
                light.setNeedsDisplay();
            }
        }
    }
}

/// The lights of the window while it is full screen.
struct Hosted {
    /// The titlebar's own, which are hidden while the window's are shown.
    originals: Vec<Retained<NSButton>>,
    /// The group that the window's own are in, in its frame view; the lights are close, minimize and zoom in it, in that
    /// order.
    group: Retained<LightsGroup>,
}

type HostedLights = Rc<RefCell<Option<Hosted>>>;

/// The observers that put the lights back where they go when the frame view is resized.
type Keepers = Rc<RefCell<Vec<Retained<ProtocolObject<dyn NSObjectProtocol>>>>>;

/// The observers that keep one window's full-screen titlebar clear. They are removed when it is dropped.
pub(super) struct FullscreenTitlebar {
    observers: Vec<Retained<ProtocolObject<dyn NSObjectProtocol>>>,
    hosted: HostedLights,
    keepers: Keepers,
}

impl FullscreenTitlebar {
    /// Clears the backdrop of the titlebar when `window` enters full screen, and keeps the lights in the window while it
    /// is, at `lights` when there is a place for them.
    pub(super) fn watch(window: &Retained<NSWindow>, lights: Option<TrafficLightPosition>) -> Self {
        let hosted: HostedLights = Rc::default();
        let keepers: Keepers = Rc::default();
        let weak = Weak::from_retained(window);
        let clear = RcBlock::new(move |_notification: NonNull<NSNotification>| clear_backdrops(lights.is_some()));
        let host = {
            let (weak, hosted, keepers) = (weak.clone(), Rc::clone(&hosted), Rc::clone(&keepers));
            RcBlock::new(move |_notification: NonNull<NSNotification>| {
                clear_backdrops(lights.is_some());
                if let (Some(window), Some(position)) = (weak.load(), lights) {
                    host_lights(&window, position, &hosted, &keepers);
                }
            })
        };
        let restore = {
            let (hosted, keepers) = (Rc::clone(&hosted), Rc::clone(&keepers));
            RcBlock::new(move |_notification: NonNull<NSNotification>| restore_lights(&hosted, &keepers))
        };
        let center = NSNotificationCenter::defaultCenter();
        let object: &AnyObject = window.as_ref();
        // SAFETY: the names are constants that AppKit defines, and the blocks are called on the main thread, where the
        // window posts, and hold only weak references and what only they touch.
        let observers = unsafe {
            [
                center.addObserverForName_object_queue_usingBlock(Some(NSWindowWillEnterFullScreenNotification), Some(object), None, &clear),
                center.addObserverForName_object_queue_usingBlock(Some(NSWindowDidEnterFullScreenNotification), Some(object), None, &host),
                center.addObserverForName_object_queue_usingBlock(Some(NSWindowWillExitFullScreenNotification), Some(object), None, &restore),
            ]
        }
        .into();
        Self { observers, hosted, keepers }
    }
}

impl Drop for FullscreenTitlebar {
    fn drop(&mut self) {
        restore_lights(&self.hosted, &self.keepers);
        let center = NSNotificationCenter::defaultCenter();
        for observer in &self.observers {
            let object: &AnyObject = observer.as_ref();
            // SAFETY: `observer` is the token `addObserverForName…` returned.
            unsafe { center.removeObserver(object) };
        }
    }
}

/// Whether `window` is full screen, or entering it.
pub(super) fn is_full_screen(window: &NSWindow) -> bool {
    window.styleMask().contains(NSWindowStyleMask::FullScreen)
}

/// The name of the class of `object`.
fn class_name(object: &AnyObject) -> String {
    object.class().name().to_string_lossy().into_owned()
}

/// The windows AppKit shows the full-screen titlebars of the application in.
fn titlebar_windows() -> Vec<Retained<AnyObject>> {
    // SAFETY: the application's shared instance and its windows, read on the main thread, where AppKit posts the
    // notifications and calls the window's layout.
    let windows: Retained<NSArray<AnyObject>> = unsafe {
        let app: Retained<AnyObject> = msg_send![class!(NSApplication), sharedApplication];
        msg_send![&*app, windows]
    };
    windows.iter().filter(|window| is_titlebar_window(&class_name(window))).collect()
}

/// Makes every backdrop of every full-screen titlebar of the application transparent, and when `own_lights`, which is when
/// the window has lights of its own, hides what is in those windows and makes them let the mouse through.
pub(super) fn clear_backdrops(own_lights: bool) {
    for window in titlebar_windows() {
        // SAFETY: plain queries and setters of a window, on the main thread.
        unsafe {
            if own_lights {
                let _: () = msg_send![&*window, setIgnoresMouseEvents: true];
            }
            let content: Option<Retained<NSView>> = msg_send![&*window, contentView];
            if let Some(content) = content {
                clear_in(&content);
            }
        }
        if own_lights {
            set_content_hidden(&window, true);
        }
    }
}

/// Hides or shows what is drawn in a full-screen titlebar window, which the titlebar's lights are part of. It is a method
/// of AppKit's own window class that is not documented, so it is only used where the window has it.
fn set_content_hidden(window: &AnyObject, hidden: bool) {
    // SAFETY: the selector is checked for before it is sent, and it takes a BOOL.
    unsafe {
        let responds: bool = msg_send![window, respondsToSelector: sel!(setContentHidden:)];
        if responds {
            let _: () = msg_send![window, setContentHidden: hidden];
        }
    }
}

fn clear_in(view: &NSView) {
    if is_backdrop(&class_name(view.as_ref())) {
        // SAFETY: `alphaValue` and `setAlphaValue:` take and give a CGFloat, which is an f64 on the platforms with AppKit;
        // they are called on the main thread. The messages are sent instead of the bindings so that this does not need the
        // feature of the bindings that CGFloat is behind.
        unsafe {
            let alpha: f64 = msg_send![view, alphaValue];
            if alpha != 0.0 {
                let _: () = msg_send![view, setAlphaValue: 0.0_f64];
            }
        }
    }
    for sub in view.subviews().iter() {
        clear_in(&sub);
    }
}

/// Hides the titlebar's lights and puts lights of the window's own in its frame view, at `position` from its top-left
/// corner, above what is drawn there. Does nothing while they are there.
fn host_lights(window: &Retained<NSWindow>, position: TrafficLightPosition, hosted: &HostedLights, keepers: &Keepers) {
    if hosted.borrow().is_some() {
        return;
    }
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    // The frame view is the parent of the content view, and holds the titlebar on the desktop.
    let Some(frame_view) = window.contentView().and_then(|content| unsafe { content.superview() }) else {
        return;
    };
    let which = [NSWindowButton::CloseButton, NSWindowButton::MiniaturizeButton, NSWindowButton::ZoomButton];
    let originals: Vec<Retained<NSButton>> = which.into_iter().filter_map(|which| window.standardWindowButton(which)).collect();
    let [close, minimize, zoom] = originals.as_slice() else {
        return;
    };
    let button_size = close.frame().size;
    let placement = ButtonPlacement::new(position, [minimize.frame().origin.x - close.frame().origin.x, zoom.frame().origin.x - close.frame().origin.x]);
    // The style of the window as it is, which has full screen in it: the zoom button then leaves it.
    let style = window.styleMask();
    let ours: Vec<Retained<NSButton>> = which.into_iter().filter_map(|which| NSWindow::standardWindowButton_forStyleMask(which, style, mtm)).collect();
    if ours.len() != 3 {
        return;
    }
    let layout = group_layout(&placement, frame_view.frame().size.height, (button_size.width, button_size.height), frame_view.isFlipped());
    let group = LightsGroup::new(mtm, NSRect::new(NSPoint::new(layout.origin.0, layout.origin.1), NSSize::new(layout.size.0, layout.size.1)));
    for ((original, button), (x, y)) in originals.iter().zip(&ours).zip(layout.lights) {
        original.setHidden(true);
        button.setFrameOrigin(NSPoint::new(x, y));
    }
    group.add_lights(&ours);
    frame_view.addSubview(&group);
    // Not resized with the frame view: where it goes is found again from its size whenever it changes.
    group.setAutoresizingMask(NSAutoresizingMaskOptions::empty());
    *hosted.borrow_mut() = Some(Hosted { originals, group });
    keep_lights(&frame_view, hosted, placement, keepers);
}

/// Puts the lights back where they go whenever the frame view changes size.
fn keep_lights(frame_view: &Retained<NSView>, hosted: &HostedLights, placement: ButtonPlacement, keepers: &Keepers) {
    let weak = Weak::from_retained(frame_view);
    let hosted = Rc::clone(hosted);
    // Moving a light does not post for the frame view; this is for the case where it somehow did.
    let busy = Rc::new(Cell::new(false));
    let block = RcBlock::new(move |_notification: NonNull<NSNotification>| {
        if busy.replace(true) {
            return;
        }
        if let (Some(frame_view), Some(hosted)) = (weak.load(), hosted.borrow().as_ref()) {
            place_hosted(&hosted.group, &placement, &frame_view);
        }
        busy.set(false);
    });
    frame_view.setPostsFrameChangedNotifications(true);
    let object: &AnyObject = frame_view.as_ref();
    // SAFETY: the block is called on the main thread, where the view changes, and holds a weak reference to the view, the
    // lights that this keeps, and a flag only it touches.
    let observer = unsafe {
        NSNotificationCenter::defaultCenter().addObserverForName_object_queue_usingBlock(Some(NSViewFrameDidChangeNotification), Some(object), None, &block)
    };
    *keepers.borrow_mut() = vec![observer];
}

/// Puts the group of lights where `placement` says in `frame_view`, if it is somewhere else.
fn place_hosted(group: &LightsGroup, placement: &ButtonPlacement, frame_view: &NSView) {
    let size = group.frame().size;
    // The lights are the group less its margin on every side.
    let layout = group_layout(placement, frame_view.frame().size.height, (size.width - 2.0 * GROUP_MARGIN - (placement.x[2] - placement.x[0]), size.height - 2.0 * GROUP_MARGIN), frame_view.isFlipped());
    let current = group.frame().origin;
    if is_elsewhere(layout.origin, (current.x, current.y)) {
        group.setFrameOrigin(NSPoint::new(layout.origin.0, layout.origin.1));
    }
}

/// Takes the window's own lights away and shows the titlebar's again.
fn restore_lights(hosted: &HostedLights, keepers: &Keepers) {
    let center = NSNotificationCenter::defaultCenter();
    for observer in keepers.take() {
        let object: &AnyObject = observer.as_ref();
        // SAFETY: `observer` is the token `addObserverForName…` returned.
        unsafe { center.removeObserver(object) };
    }
    let Some(lights) = hosted.take() else {
        return;
    };
    // The lights go with the group they are in.
    lights.group.removeFromSuperview();
    for original in &lights.originals {
        original.setHidden(false);
    }
    for window in titlebar_windows() {
        set_content_hidden(&window, false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_of_the_full_screen_titlebar_is_found_by_its_class_and_no_other_is() {
        assert!(is_titlebar_window("NSToolbarFullScreenWindow"));
        assert!(
            is_titlebar_window("NSKVONotifying_NSToolbarFullScreenWindow"),
            "AppKit observes the window, which makes its class a subclass of that name: it is how the window is found in a running app"
        );
        assert!(!is_titlebar_window("WinitWindow"), "the application's own window");
        assert!(!is_titlebar_window("_NSFullScreenTransitionOverlayWindow"), "the window that fades between the two");
        assert!(!is_titlebar_window(""));
    }

    #[test]
    fn only_the_views_that_draw_a_backdrop_are_cleared() {
        assert!(is_backdrop("NSTitlebarBackgroundView"), "the gray bar, which holds the blur that it is drawn with");
        assert!(is_backdrop("NSVisualEffectView"));
        assert!(is_backdrop("NSKVONotifying_NSTitlebarBackgroundView"), "a view that is observed is of a subclass of its class");
        assert!(!is_backdrop("NSTitlebarView"), "it holds the traffic lights");
        assert!(!is_backdrop("NSTitlebarContainerView"), "it holds both");
        assert!(!is_backdrop("_NSTitlebarDecorationView"));
        assert!(!is_backdrop("_NSThemeCloseWidget"));
        assert!(!is_backdrop("NSToolbarFullScreenContentView"));
    }

    #[test]
    fn the_group_of_lights_is_a_little_larger_than_them_and_each_light_is_in_it_at_its_place() {
        let placement = ButtonPlacement::new(TrafficLightPosition { x: 16.0, y: 14.0 }, [23.0, 46.0]);
        // Lights 14 wide and 16 high: the group is a margin larger on every side.
        let flipped = group_layout(&placement, 1130.0, (14.0, 16.0), true);
        assert_eq!(flipped.origin, (16.0 - GROUP_MARGIN, 14.0 - GROUP_MARGIN), "from the top-left corner of a view that counts down");
        assert_eq!(flipped.size, (46.0 + 14.0 + 2.0 * GROUP_MARGIN, 16.0 + 2.0 * GROUP_MARGIN));
        assert_eq!(flipped.lights, [(GROUP_MARGIN, GROUP_MARGIN), (GROUP_MARGIN + 23.0, GROUP_MARGIN), (GROUP_MARGIN + 46.0, GROUP_MARGIN)], "as far apart as AppKit has them");
        let upright = group_layout(&placement, 1130.0, (14.0, 16.0), false);
        assert_eq!(upright.origin.0, flipped.origin.0);
        assert_eq!(upright.origin.1, 1130.0 - 14.0 - 16.0 - GROUP_MARGIN, "from the top of a view that counts up");
        assert_eq!(upright.lights, flipped.lights, "in the group, which counts up, they are where they are in either");
        assert_eq!(group_layout(&placement, 400.0, (14.0, 16.0), true), flipped, "whatever the height of a view that counts down");
    }

    #[test]
    fn the_lights_are_gray_until_the_pointer_is_over_them_and_the_one_that_cannot_be_used_stays_gray() {
        // A button that is not enabled is drawn gray, without a glyph: it is how the lights look as the pointer is away.
        for light in [LIGHT_CLOSE, LIGHT_MINIMIZE, LIGHT_ZOOM] {
            assert!(!light_enabled(light, false), "light {light} is gray as the pointer is away");
        }
        assert!(light_enabled(LIGHT_CLOSE, true), "red, with its glyph");
        assert!(light_enabled(LIGHT_ZOOM, true), "green, with its glyph");
        assert!(!light_enabled(LIGHT_MINIMIZE, true), "a window cannot be minimized in full screen, whatever the pointer does");
    }

    #[test]
    fn a_light_is_put_back_when_it_is_somewhere_else_and_left_alone_when_it_is_where_it_goes() {
        assert!(is_elsewhere((16.0, 14.0), (16.0, 1102.0)), "at the bottom");
        assert!(is_elsewhere((16.0, 14.0), (20.0, 14.0)));
        assert!(!is_elsewhere((16.0, 14.0), (16.0, 14.0)));
        assert!(!is_elsewhere((16.0, 14.0), (16.004, 14.0)), "AppKit rounds to the pixel grid, and a move that small is not one");
    }
}
