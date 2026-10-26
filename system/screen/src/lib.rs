//! The screen the application is shown on: how the window and the device are
//! turned, and where a foldable device bends.
//!
//! [`MediaQuery`] is how a widget asks. It answers four different questions, and
//! a widget is rebuilt only when the answer to the one it asked changes:
//!
//! | Question | Reads | Rebuilt when |
//! |----------|-------|--------------|
//! | How big is the window? | [`MediaQuery::of`], [`MediaQuery::select`] | the window is resized, or moves to another display |
//! | Is the window portrait or landscape? | [`MediaQuery::orientation_of`] | the window flips shape |
//! | Which way is the *device* turned? | [`MediaQuery::device_orientation_of`] | the device is rotated |
//! | Where are the folds and hinges? | [`MediaQuery::display_features_of`], [`MediaQuery::panes_of`] | a foldable is opened, closed or half opened |
//!
//! The window's shape and the device's orientation are different things. A phone
//! held sideways in split-screen shows a portrait window; a desktop window has a
//! shape and no rotation at all.
//!
//! # Where each answer comes from
//!
//! | Platform | Device orientation | Folds and hinges |
//! |----------|--------------------|------------------|
//! | iOS | `UIDevice.orientation` (main thread) | none |
//! | Android | `Display.getRotation()` over JNI | Jetpack WindowManager, through `ScreenBridge.kt` |
//! | Web | `screen.orientation.type` | none |
//! | macOS, Windows, Linux | none (`None`) | none |
//!
//! The platform host calls [`refresh`] when the window changes shape, which is
//! how a rotation reaches an application, and [`apply_pending`] when Android
//! reports a fold. Nothing is polled per frame.
//!
//! # Folds and hinges
//!
//! A [`DisplayFeature`] is a hinge (it hides the content under it, as on the
//! Microsoft Surface Duo) or a fold (a bend in a continuous display). Where one
//! divides the window, [`split_panes`] and [`MediaQuery::panes_of`] give the two
//! usable areas to lay out independently.
//!
//! On Android the features come from `android/ScreenBridge.kt` in this crate:
//! copy it into the app's `com.aimer` package, add `androidx.window:window` and
//! `androidx.window:window-java` to its dependencies, and call
//! `ScreenBridge.attach(this)` from the activity's `onCreate`. Without that the
//! app runs normally and sees no features.
//!
//! # Examples
//!
//! ```
//! use aimer_screen::{Bounds, DisplayFeature, FoldState, split_panes};
//!
//! // A hinge down the middle of a dual-screen phone, 2784 x 1800 logical pixels.
//! let hinge = DisplayFeature {
//!     bounds: Bounds::new(1350.0, 0.0, 1434.0, 1800.0),
//!     occludes: true,
//!     state: FoldState::HalfOpened,
//!     separating: true,
//! };
//!
//! let panes = split_panes(2784.0, 1800.0, &[hinge]).expect("the hinge divides the window");
//! assert_eq!(panes.first.right, 1350.0);
//! assert_eq!(panes.second.left, 1434.0);
//! ```

#[cfg(target_os = "android")]
mod android_screen;
mod display_feature;
mod media_query;
#[cfg(target_os = "ios")]
mod ios_screen;
mod orientation;
mod screen_inbox;
mod screen_state;
#[cfg(all(target_arch = "wasm32", not(aimer_portable_guest)))]
mod web_screen;
mod wire;

#[cfg(target_os = "android")]
use android_screen as platform;
#[cfg(target_os = "ios")]
use ios_screen as platform;
#[cfg(all(target_arch = "wasm32", not(aimer_portable_guest)))]
use web_screen as platform;

/// Desktop computers do not rotate, and a portable guest cannot ask.
#[cfg(not(any(
    target_os = "android",
    target_os = "ios",
    all(target_arch = "wasm32", not(aimer_portable_guest))
)))]
mod platform {
    pub(super) fn device_orientation() -> Option<crate::DeviceOrientation> {
        None
    }
}

/// Asks the platform which way the device is turned and records the answer.
///
/// Returns how many widgets were marked for rebuild. The platform host calls
/// this when the window changes shape — on a phone, a rotation arrives as a
/// resize — and once at start-up; it is one query, not a poll. An answer of
/// "unknown" (a device lying flat, a desktop) leaves the last known orientation
/// alone rather than erasing it.
///
/// Call it on the thread that owns the widget tree. On iOS that must be the main
/// thread, which is the only one UIKit answers on.
pub fn refresh() -> usize {
    match platform::device_orientation() {
        Some(orientation) => set_device_orientation(Some(orientation)),
        None => 0,
    }
}

pub use display_feature::{
    Bounds, DisplayFeature, DisplayFeatureKind, FoldState, Panes, Posture, split_panes,
};
pub use media_query::MediaQuery;
pub use orientation::{DeviceOrientation, Orientation};
pub use screen_inbox::{apply_pending, post_device_orientation, post_display_features};
pub use screen_state::{
    device_orientation, display_features, set_device_orientation, set_display_features,
};
pub use wire::decode_display_features;
