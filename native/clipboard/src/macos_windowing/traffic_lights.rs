//! Keeps the traffic lights where the application put them while AppKit lays
//! the titlebar out.
//!
//! AppKit puts the buttons back at their native place whenever it lays the
//! titlebar out, and it does so on every frame of the animation that leaves
//! full screen, after the window's resize has been reported. Placing them in
//! response to the resize therefore loses: for the length of the animation they
//! sit at the native height, then jump when something finally places them
//! again. So the buttons are placed again from inside AppKit's own write, the
//! moment one of them or their container moves, before the frame is drawn.
//!
//! While the window is full screen, or entering it, the buttons are left to
//! AppKit: its full-screen titlebar has positions of its own, and placing them
//! there would be undone by AppKit on every layout, forever.
//!
//! This only ever moves the three buttons. The titlebar container's frame is
//! left to AppKit here: writing it from inside a layout makes AppKit lay out
//! again and recurse until the stack is gone.

use std::cell::Cell;
use std::ptr::NonNull;
use std::rc::Rc;

use block2::RcBlock;
use objc2::rc::{Retained, Weak};
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2_app_kit::{NSView, NSViewFrameDidChangeNotification, NSWindow, NSWindowButton, NSWindowStyleMask};
use objc2_foundation::{NSNotification, NSNotificationCenter, NSObjectProtocol, NSPoint};

use super::TrafficLightPosition;

/// The observers that place one window's traffic lights again. They are
/// removed when it is dropped.
pub(super) struct TrafficLightKeeper {
    observers: Vec<Retained<ProtocolObject<dyn NSObjectProtocol>>>,
}

/// Where the buttons go: the distance of the top-left corner of the close
/// button from the top-left corner of their container, and how far the other
/// two are to its right.
///
/// Measured from the top, so it holds for any height of the container, which
/// AppKit changes while it animates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct ButtonPlacement {
    pub(super) x: [f64; 3],
    top: f64,
}

impl ButtonPlacement {
    /// `spacing` is the native distance from the close button to the
    /// minimize and zoom buttons, which AppKit never changes.
    pub(super) fn new(position: TrafficLightPosition, spacing: [f64; 2]) -> Self {
        Self { x: [position.x, position.x + spacing[0], position.x + spacing[1]], top: position.y }
    }

    /// How far the top of the buttons is from the top of their container.
    pub(super) fn top(&self) -> f64 {
        self.top
    }

    /// The y of a button of `button_height` in a container `height` high, in
    /// AppKit's bottom-up coordinates.
    pub(super) fn y(&self, height: f64, button_height: f64) -> f64 {
        height - self.top - button_height
    }
}

/// A move smaller than this is not one: AppKit rounds to the pixel grid.
const SAME: f64 = 0.01;

impl TrafficLightKeeper {
    /// Watches the three buttons of `window` and their container, putting the
    /// buttons back at `position` whenever AppKit moves them.
    ///
    /// `None` when the window has no standard buttons to watch.
    pub(super) fn watch(window: &Retained<NSWindow>, position: TrafficLightPosition) -> Option<Self> {
        let close = window.standardWindowButton(NSWindowButton::CloseButton)?;
        let minimize = window.standardWindowButton(NSWindowButton::MiniaturizeButton)?;
        let zoom = window.standardWindowButton(NSWindowButton::ZoomButton)?;
        // SAFETY: AppKit owns the titlebar hierarchy; the views are only used
        // on the main thread, inside this call.
        let container = unsafe { close.superview() }?;

        let spacing = [
            minimize.frame().origin.x - close.frame().origin.x,
            zoom.frame().origin.x - close.frame().origin.x,
        ];
        let placement = ButtonPlacement::new(position, spacing);
        let weak = Weak::from_retained(window);
        // Moving a button posts again: the repeat is ignored, the placement it
        // came from is already complete.
        let busy = Rc::new(Cell::new(false));
        let block = RcBlock::new(move |_notification: NonNull<NSNotification>| {
            if busy.replace(true) {
                return;
            }
            if let Some(window) = weak.load() {
                place_buttons(&window, placement);
            }
            busy.set(false);
        });

        let center = NSNotificationCenter::defaultCenter();
        let watched: [&NSView; 4] = [&container, &close, &minimize, &zoom];
        let observers = watched
            .into_iter()
            .map(|view| {
                view.setPostsFrameChangedNotifications(true);
                let object: &AnyObject = view.as_ref();
                // SAFETY: the block is called on the thread that moves the
                // view, which is the main thread, and holds nothing but a weak
                // reference to the window and a flag only it touches.
                unsafe {
                    center.addObserverForName_object_queue_usingBlock(
                        Some(NSViewFrameDidChangeNotification),
                        Some(object),
                        None,
                        &block,
                    )
                }
            })
            .collect();
        Some(Self { observers })
    }
}

/// Whether `window` is full screen, or entering it: AppKit sets the mask at
/// the start of the transition and clears it at the start of the way out.
///
/// The buttons then belong to AppKit's full-screen titlebar, a strip it lays
/// out with positions of its own. Placing them there puts them where they do
/// not belong, and AppKit puts them back on every layout, so the two would
/// overwrite each other for as long as the window stays full screen.
pub(super) fn is_full_screen(window: &NSWindow) -> bool {
    window.styleMask().contains(NSWindowStyleMask::FullScreen)
}

/// Puts the three buttons of `window` at `placement`, writing only the ones
/// that are somewhere else.
fn place_buttons(window: &NSWindow, placement: ButtonPlacement) {
    if is_full_screen(window) {
        return;
    }
    let buttons = [NSWindowButton::CloseButton, NSWindowButton::MiniaturizeButton, NSWindowButton::ZoomButton];
    for (index, which) in buttons.into_iter().enumerate() {
        let Some(button) = window.standardWindowButton(which) else {
            return;
        };
        // SAFETY: AppKit owns the hierarchy; used inside this main-thread call.
        let Some(container) = (unsafe { button.superview() }) else {
            return;
        };
        let target = NSPoint {
            x: placement.x[index],
            y: placement.y(container.frame().size.height, button.frame().size.height),
        };
        let current = button.frame().origin;
        if (current.x - target.x).abs() > SAME || (current.y - target.y).abs() > SAME {
            button.setFrameOrigin(target);
        }
    }
}

impl Drop for TrafficLightKeeper {
    fn drop(&mut self) {
        let center = NSNotificationCenter::defaultCenter();
        for observer in &self.observers {
            let object: &AnyObject = observer.as_ref();
            // SAFETY: `observer` is the token `addObserverForName…` returned.
            unsafe { center.removeObserver(object) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_buttons_are_the_same_distance_from_the_top_whatever_the_container_height() {
        let placement = ButtonPlacement::new(TrafficLightPosition { x: 16.0, y: 14.0 }, [20.0, 40.0]);

        // The bottom-up y of a 12 high button: its top edge is 14 from the top
        // of a 40 high titlebar and of a 28 high one alike.
        assert_eq!(placement.y(40.0, 12.0), 14.0);
        assert_eq!(placement.y(28.0, 12.0), 2.0);
        assert_eq!(40.0 - placement.y(40.0, 12.0) - 12.0, 28.0 - placement.y(28.0, 12.0) - 12.0);
    }

    #[test]
    fn the_buttons_keep_their_native_spacing_from_the_close_button() {
        let placement = ButtonPlacement::new(TrafficLightPosition { x: 16.0, y: 14.0 }, [20.0, 40.0]);

        assert_eq!(placement.x, [16.0, 36.0, 56.0]);
    }
}
