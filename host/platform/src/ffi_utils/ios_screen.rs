extern crate objc;

use aimer_utils::info;
use objc::runtime::{Class, Object};
use objc::{Encode, Encoding, msg_send, sel, sel_impl};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window;

/// Associate winit's `UIWindow` with the foreground-active `UIWindowScene`.
///
/// winit (0.30) creates its `UIWindow` with `initWithFrame:` and calls
/// `makeKeyAndVisible()`, but it never sets a `UIWindowScene`. Starting with
/// the iOS 26 / 27 SDK the UIScene life cycle is mandatory (see Apple TN3187):
/// a scene-less window is never displayed and never receives layout / redraw
/// callbacks, so the app shows a black screen and `RedrawRequested` never
/// fires.
///
/// Here we recover winit's `UIWindow` from the `UIView` exposed through
/// `raw-window-handle`, attach it to the active window scene and re-assert
/// `makeKeyAndVisible()` so the content — and the GPU redraw loop — start.
/// The initial frame follows that scene's bounds in points, which can be
/// smaller than the display in Split View or a freely resized iPad window.
/// Call only on UIKit's main thread, from the window's event loop.
pub fn attach_window_to_active_scene(window: &Window) {
    let ui_view = match window.window_handle() {
        Ok(handle) => match handle.as_raw() {
            RawWindowHandle::UiKit(uikit) => uikit.ui_view.as_ptr() as *mut Object,
            _ => return,
        },
        Err(_) => return,
    };

    if ui_view.is_null() {
        return;
    }

    // SAFETY: Called by ApplicationHandler::resumed on UIKit's main thread.
    // The borrowed winit window owns the view and UIWindow throughout these
    // synchronous messages; scene objects remain owned by UIApplication.
    unsafe {
        #[allow(unexpected_cfgs)]
        let ui_window: *mut Object = msg_send![ui_view, window];
        if ui_window.is_null() {
            info!("attach_window_to_active_scene: winit view has no UIWindow yet");
            return;
        }

        // Already associated with a scene: nothing to attach.
        #[allow(unexpected_cfgs)]
        let existing_scene: *mut Object = msg_send![ui_window, windowScene];
        if !existing_scene.is_null() {
            defer_right_edge_system_gestures(window);
            return;
        }

        let Some(app_class) = Class::get("UIApplication") else {
            return;
        };
        #[allow(unexpected_cfgs)]
        let app: *mut Object = msg_send![app_class, sharedApplication];
        if app.is_null() {
            return;
        }

        #[allow(unexpected_cfgs)]
        let scenes: *mut Object = msg_send![app, connectedScenes];
        #[allow(unexpected_cfgs)]
        let all: *mut Object = msg_send![scenes, allObjects];
        #[allow(unexpected_cfgs)]
        let count: usize = msg_send![all, count];

        let Some(window_scene_class) = Class::get("UIWindowScene") else {
            return;
        };

        // Prefer a foreground-active window scene, otherwise the first one found.
        let mut chosen: *mut Object = std::ptr::null_mut();
        for i in 0..count {
            #[allow(unexpected_cfgs)]
            let scene: *mut Object = msg_send![all, objectAtIndex: i];
            #[allow(unexpected_cfgs)]
            let is_window_scene: bool = msg_send![scene, isKindOfClass: window_scene_class];
            if !is_window_scene {
                continue;
            }
            // `UISceneActivationStateForegroundActive == 0`.
            #[allow(unexpected_cfgs)]
            let activation_state: isize = msg_send![scene, activationState];
            if activation_state == 0 {
                chosen = scene;
                break;
            }
            if chosen.is_null() {
                chosen = scene;
            }
        }

        if chosen.is_null() {
            info!("attach_window_to_active_scene: no UIWindowScene available yet");
            return;
        }

        #[allow(unexpected_cfgs)]
        let _: () = msg_send![ui_window, setWindowScene: chosen];
        // winit initialized the frame from UIScreen. Attaching a scene must
        // also adopt its current viewport before the first GPU surface exists.
        #[allow(unexpected_cfgs)]
        let space: *mut Object = msg_send![chosen, coordinateSpace];
        if !space.is_null() {
            #[allow(unexpected_cfgs)]
            let bounds: CGRect = msg_send![space, bounds];
            if bounds.size.width.is_finite()
                && bounds.size.height.is_finite()
                && bounds.size.width > 0.0
                && bounds.size.height > 0.0
            {
                #[allow(unexpected_cfgs)]
                let _: () = msg_send![ui_window, setFrame: bounds];
            }
        }
        #[allow(unexpected_cfgs)]
        let _: () = msg_send![ui_window, makeKeyAndVisible];
        #[allow(unexpected_cfgs)]
        let _: () = msg_send![ui_window, layoutIfNeeded];
        info!("attach_window_to_active_scene: attached winit UIWindow to active UIWindowScene");
    }
    // Only now, with the window in its scene: see the function below.
    defer_right_edge_system_gestures(window);
}

/// Hands touches near the right screen edge to the app without delay.
///
/// iOS holds back a third-party app's touch that begins within about 20 pt of a
/// screen edge, for roughly 0.76 s, while it waits to see whether the touch
/// becomes a system gesture. Apple's own apps are exempt. The scroll bar's thumb
/// sits at the right edge, so a finger on it reached the app three quarters of a
/// second after it landed.
///
/// `preferredScreenEdgesDeferringSystemGestures` removes that wait for the
/// edges it names. Only the right edge is named: the bottom and top edges are how
/// the user goes home and opens the control and notification centres, and
/// deferring them would make each of those take a second swipe.
///
/// This has to be applied after the window has its scene. winit applies the
/// preference when it creates the window, before [`attach_window_to_active_scene`]
/// gives the window its scene, and iOS does not read it again by itself, so a
/// preference set at creation never takes effect. winit's setter stores the
/// edges and asks UIKit to re-read them, which is the part that matters here;
/// a native app that attached its window the same way needed exactly this call.
fn defer_right_edge_system_gestures(window: &Window) {
    use winit::platform::ios::{ScreenEdge, WindowExtIOS};

    window.set_preferred_screen_edges_deferring_system_gestures(ScreenEdge::RIGHT);
}

// CGFloat is f64 on all supported (64-bit) iOS targets.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
struct CGRect {
    origin: CGPoint,
    size: CGSize,
}
#[derive(Debug, Clone, Copy)]
#[repr(C)]
struct CGPoint {
    x: f64,
    y: f64,
}
#[derive(Debug, Clone, Copy)]
#[repr(C)]
struct CGSize {
    width: f64,
    height: f64,
}

// SAFETY: repr(C) preserves CGPoint followed by CGSize, each holding two
// CGFloat doubles. The encoding is UIKit's CGRect argument/return ABI.
unsafe impl Encode for CGRect {
    fn encode() -> Encoding {
        unsafe { Encoding::from_str("{CGRect={CGPoint=dd}{CGSize=dd}}") }
    }
}

const _: () = {
    assert!(std::mem::size_of::<CGRect>() == 32);
    assert!(std::mem::align_of::<CGRect>() == 8);
    assert!(std::mem::offset_of!(CGRect, size) == 16);
};
