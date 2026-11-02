//! iOS, from `UIDevice.orientation`.
//!
//! That is the physical orientation the accelerometer reports, which is not the
//! orientation of the interface: an app locked to portrait is still told when the
//! phone is laid on its side. UIKit only starts reporting after
//! `beginGeneratingDeviceOrientationNotifications`, so the first query turns it
//! on; the answer may still be unknown at that moment and is right from the next
//! query, which a rotation always provides.
//!
//! UIKit answers on the main thread only, which `objc2` enforces with a
//! [`MainThreadMarker`]: from any other thread the orientation is unknown.
//!
//! # Folds and the hinge angle (iOS 27.1)
//!
//! UIKit has no list of folds. A foldable iPhone reports two things, and this
//! reads both from the key window of the foreground scene:
//!
//! * the **fold**, as a *division* reserved region — `UIView.reservedRegions`
//!   with the `division` kind, including the inactive ones so a layout can leave
//!   room before the device is folded. Its frame is in the window's points.
//! * the **angle**, from a `UIHingeInteraction` added to that window. Its
//!   handler is called with the initial state and on every change, and is the
//!   only way UIKit gives the angle (radians) and the status.
//!
//! `objc2-ui-kit` predates both, so the few selectors are sent dynamically. The
//! classes and the selector are looked up by name: on an older iOS they are
//! absent, and the app simply sees no fold. That lookup is also why this needs
//! no `#[available]` gate at the call site.
//!
//! Nothing is polled. The regions are re-read on [`refresh`](crate::refresh) —
//! a resize, which is how opening the device reaches the app — and whenever the
//! hinge interaction reports a change, which is posted to the inbox and wakes
//! the event loop through [`set_change_waker`](crate::set_change_waker).

use std::cell::Cell;

use block2::RcBlock;
use objc2::encode::{Encode, Encoding, RefEncode};
use objc2::rc::{Allocated, Retained};
use objc2::runtime::{AnyClass, AnyObject, Sel};
use objc2::{MainThreadMarker, msg_send, sel};
use objc2_ui_kit::UIDevice;

use crate::reserved_region::{DivisionRegion, features_from_divisions};
use crate::screen_inbox::wake;
use crate::{
    DeviceOrientation, HingeState, post_display_features, post_hinge, set_display_features,
};

thread_local! {
    static GENERATING: Cell<bool> = const { Cell::new(false) };
}

aimer_tls::thread_local! {
    /// The window the hinge interaction was added to, kept so it is added once
    /// per window and not once per resize.
    static HINGE_WINDOW: Cell<usize> = const { Cell::new(0) };
}

/// `UIViewReservedRegionQueryOptionsIncludeInactive`.
const INCLUDE_INACTIVE: usize = 1 << 0;

/// `UISceneActivationStateForegroundActive`.
const FOREGROUND_ACTIVE: isize = 0;

// `CGRect` as UIKit encodes it. Declared here, not taken from a binding crate,
// because three plain doubles are all that is read; a wrong layout would be
// caught by `msg_send!`'s encoding check in debug builds.
#[repr(C)]
#[derive(Clone, Copy)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CGSize {
    width: f64,
    height: f64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CGRect {
    origin: CGPoint,
    size: CGSize,
}

// SAFETY: `#[repr(C)]` structs of `f64` (`CGFloat` on every 64-bit Apple
// target) whose encodings are the ones Core Graphics registers.
unsafe impl Encode for CGPoint {
    const ENCODING: Encoding = Encoding::Struct("CGPoint", &[Encoding::Double, Encoding::Double]);
}
unsafe impl RefEncode for CGPoint {
    const ENCODING_REF: Encoding = Encoding::Pointer(&<Self as Encode>::ENCODING);
}
unsafe impl Encode for CGSize {
    const ENCODING: Encoding = Encoding::Struct("CGSize", &[Encoding::Double, Encoding::Double]);
}
unsafe impl RefEncode for CGSize {
    const ENCODING_REF: Encoding = Encoding::Pointer(&<Self as Encode>::ENCODING);
}
unsafe impl Encode for CGRect {
    const ENCODING: Encoding = Encoding::Struct(
        "CGRect",
        &[<CGPoint as Encode>::ENCODING, <CGSize as Encode>::ENCODING],
    );
}
unsafe impl RefEncode for CGRect {
    const ENCODING_REF: Encoding = Encoding::Pointer(&<Self as Encode>::ENCODING);
}

pub(super) fn device_orientation() -> Option<DeviceOrientation> {
    let device = UIDevice::currentDevice(MainThreadMarker::new()?);
    if !GENERATING.with(|generating| generating.replace(true)) {
        device.beginGeneratingDeviceOrientationNotifications();
    }
    DeviceOrientation::from_ui_device_orientation(device.orientation().0)
}

/// Re-reads the fold from the key window and returns how many widgets that
/// marked.
///
/// Also makes sure the window has a hinge interaction, so the angle starts
/// flowing. Does nothing on an iOS without reserved regions (before 27.1), off
/// the main thread, and while no window is on screen yet — the next
/// [`refresh`](crate::refresh) tries again.
pub(super) fn refresh_folds() -> usize {
    let Some(window) = MainThreadMarker::new().and_then(key_window) else {
        return 0;
    };
    if !supports_reserved_regions(&window) {
        return 0;
    }
    ensure_hinge_interaction(&window);
    set_display_features(features_from_divisions(&division_regions(&window)))
}

/// The key window of the foreground-active window scene, or of the first scene
/// that has one.
///
/// This is the window winit's view lives in, whose points are the logical
/// pixels the rest of the framework uses. With several windows on an iPad the
/// active scene's is the one the user is looking at.
fn key_window(_main: MainThreadMarker) -> Option<Retained<AnyObject>> {
    let app_class = AnyClass::get(c"UIApplication")?;
    let scene_class = AnyClass::get(c"UIWindowScene")?;
    // SAFETY: `_main` proves this is the main thread, the only one UIKit's
    // application and scene objects may be messaged from; every selector below
    // is public API with the signature it is sent with.
    unsafe {
        let app: Option<Retained<AnyObject>> = msg_send![app_class, sharedApplication];
        let scenes: Retained<AnyObject> = msg_send![&*app?, connectedScenes];
        let all: Retained<AnyObject> = msg_send![&*scenes, allObjects];
        let count: usize = msg_send![&*all, count];
        let mut fallback = None;
        for index in 0..count {
            let scene: Retained<AnyObject> = msg_send![&*all, objectAtIndex: index];
            let is_window_scene: bool = msg_send![&*scene, isKindOfClass: scene_class];
            if !is_window_scene {
                continue;
            }
            let window: Option<Retained<AnyObject>> = msg_send![&*scene, keyWindow];
            let Some(window) = window else { continue };
            let state: isize = msg_send![&*scene, activationState];
            if state == FOREGROUND_ACTIVE {
                return Some(window);
            }
            fallback.get_or_insert(window);
        }
        fallback
    }
}

fn supports_reserved_regions(window: &AnyObject) -> bool {
    if AnyClass::get(c"UIViewReservedRegionKind").is_none() {
        return false;
    }
    // SAFETY: `respondsToSelector:` is on `NSObject` and takes a selector.
    unsafe { msg_send![window, respondsToSelector: sel_reserved_regions()] }
}

fn sel_reserved_regions() -> Sel {
    sel!(reservedRegionsOfKind:options:)
}

/// Every division region overlapping the window, active or not, in its points.
fn division_regions(window: &AnyObject) -> Vec<DivisionRegion> {
    let Some(kind_class) = AnyClass::get(c"UIViewReservedRegionKind") else {
        return Vec::new();
    };
    // SAFETY: main thread (the callers hold the window, which is only obtained
    // with a `MainThreadMarker`); `reservedRegionsOfKind:options:` returns an
    // `NSArray<UIViewReservedRegion *>`, each of which has the `frame` and
    // `isActive` properties read here.
    unsafe {
        let kind: Retained<AnyObject> = msg_send![kind_class, divisionRegionKind];
        let regions: Retained<AnyObject> =
            msg_send![window, reservedRegionsOfKind: &*kind, options: INCLUDE_INACTIVE];
        let count: usize = msg_send![&*regions, count];
        (0..count)
            .map(|index| {
                let region: Retained<AnyObject> = msg_send![&*regions, objectAtIndex: index];
                let frame: CGRect = msg_send![&*region, frame];
                let active: bool = msg_send![&*region, isActive];
                DivisionRegion {
                    x: frame.origin.x,
                    y: frame.origin.y,
                    width: frame.size.width,
                    height: frame.size.height,
                    active,
                }
            })
            .collect()
    }
}

/// Adds a `UIHingeInteraction` to `window` unless it already has ours.
fn ensure_hinge_interaction(window: &Retained<AnyObject>) {
    let address = Retained::as_ptr(window) as usize;
    if HINGE_WINDOW.with(Cell::get) == address {
        return;
    }
    let Some(class) = AnyClass::get(c"UIHingeInteraction") else {
        return;
    };
    HINGE_WINDOW.with(|installed| installed.set(address));

    // The block captures nothing: the update carries everything, and the
    // regions are looked up again from the key window.
    let handler = RcBlock::new(|_interaction: *mut AnyObject, update: *mut AnyObject| {
        on_hinge_update(update);
    });
    // SAFETY: main thread (`window` came from `key_window`). The designated
    // initialiser takes an escaping `void (^)(UIHingeInteraction *,
    // UIHingeInteractionUpdate *)`, which `handler` is; UIKit copies it, and the
    // window retains the interaction, so neither is kept alive here.
    unsafe {
        let allocated: Allocated<AnyObject> = msg_send![class, alloc];
        let interaction: Option<Retained<AnyObject>> =
            msg_send![allocated, initWithUpdateHandler: &*handler];
        let Some(interaction) = interaction else {
            return;
        };
        let _: () = msg_send![&**window, addInteraction: &*interaction];
    }
}

/// Called by UIKit, on the main thread, with the initial hinge state and on
/// every change.
///
/// It runs outside the event loop's own handling, so it posts and wakes rather
/// than touching the widget tree.
fn on_hinge_update(update: *mut AnyObject) {
    // SAFETY: UIKit passes a live `UIHingeInteractionUpdate` for the duration of
    // the call; `hinge` is nil once the interaction left a hierarchy that
    // provides hinge updates, which is "no hinge".
    let hinge = unsafe {
        update.as_ref().and_then(|update| {
            let hinge: Option<Retained<AnyObject>> = msg_send![update, hinge];
            let hinge = hinge?;
            let status: isize = msg_send![&*hinge, status];
            let angle: f64 = msg_send![&*hinge, angle];
            Some(HingeState::from_ui_hinge(angle, status))
        })
    };
    post_hinge(hinge);
    // The fold may have moved with the hinge, and a half-open device does not
    // always resize the window.
    if let Some(window) = MainThreadMarker::new().and_then(key_window) {
        post_display_features(features_from_divisions(&division_regions(&window)));
    }
    wake();
}
