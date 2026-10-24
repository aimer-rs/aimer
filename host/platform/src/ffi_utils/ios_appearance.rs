//! iOS' light / dark appearance, read from — and observed on — UIKit's trait
//! collection.
//!
//! winit answers `None` from `Window::theme()` on iOS and never emits
//! `ThemeChanged`, so UIKit is asked directly: the appearance is
//! `UITraitCollection.userInterfaceStyle`, and a change to it is a *trait
//! change* delivered to the objects in the view hierarchy rather than an event
//! in the run loop.
//!
//! Hearing that change needs an Objective-C receiver, so one small class is
//! registered with the runtime the first time it is needed. UIKit changed how
//! trait changes are delivered in iOS 17, and both ways are used, newest first:
//!
//! * iOS 17 and later: `registerForTraitChanges:withTarget:action:` on winit's
//!   `UIView`, narrowed to `UITraitUserInterfaceStyle` so the callback fires for
//!   an appearance switch and not for every rotation.
//! * iOS 16 and earlier: the observer is added to the view hierarchy as an
//!   empty, non-interactive subview and overrides the (now deprecated)
//!   `traitCollectionDidChange:`.
//!
//! Either way the cost while nothing changes is zero: there is no polling and
//! no per-frame query. When the appearance does change, the widgets that follow
//! it are rebuilt and a single frame is requested; an application whose theme
//! ignores the system asks for nothing.

// `objc`'s `msg_send!` / `sel!` expand to a `cfg(cargo-clippy)` check that this
// crate does not declare.
#![allow(unexpected_cfgs)]

extern crate objc;

use aimer_utils::info;
use aimer_widget::Brightness;
use objc::declare::ClassDecl;
use objc::runtime::{Class, Object, Sel};
use objc::{class, msg_send, sel, sel_impl};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window;

/// Name of the runtime-registered class that receives the trait changes.
const OBSERVER_CLASS_NAME: &str = "AimerAppearanceObserver";

// The observer, once created.
//
// UIKit is main-thread only and so is everything here, which is why the
// instance lives in thread-local storage rather than behind a lock. It is
// created once, never released, and outlives the window it observes.
std::thread_local! {
    static OBSERVER: std::cell::Cell<*mut Object> = const { std::cell::Cell::new(std::ptr::null_mut()) };
}

/// Reads the appearance UIKit is in right now.
///
/// `None` when UIKit has no answer yet — before any trait collection has been
/// resolved against a window — in which case the current appearance is left
/// alone.
pub fn current_appearance() -> Option<Brightness> {
    let class = Class::get("UITraitCollection")?;
    let traits: *mut Object = unsafe { msg_send![class, currentTraitCollection] };
    appearance_of_trait_collection(traits)
}

/// Starts reporting appearance changes for the hierarchy `window` lives in.
///
/// Repeated calls after the first are ignored: one observer hears every change.
pub fn observe_appearance_changes(window: &Window) {
    if !OBSERVER.get().is_null() {
        return;
    }
    let Some(ui_view) = winit_ui_view(window) else {
        return;
    };
    let Some(observer) = create_observer() else {
        return;
    };
    OBSERVER.set(observer);

    if register_for_trait_changes(ui_view, observer) {
        info!("observe_appearance_changes: observing UITraitUserInterfaceStyle");
        return;
    }
    // iOS 16 and earlier: only a member of the view hierarchy is told about a
    // trait change, so the observer joins it. An empty frame and no
    // interaction keep it out of layout and out of the touch path.
    unsafe {
        let _: () = msg_send![observer, setUserInteractionEnabled: false];
        let _: () = msg_send![ui_view, addSubview: observer];
    }
    info!("observe_appearance_changes: observing traitCollectionDidChange:");
}

/// Recovers winit's `UIView` from the window handle.
fn winit_ui_view(window: &Window) -> Option<*mut Object> {
    let handle = window.window_handle().ok()?;
    let RawWindowHandle::UiKit(uikit) = handle.as_raw() else {
        return None;
    };
    let ui_view = uikit.ui_view.as_ptr() as *mut Object;
    (!ui_view.is_null()).then_some(ui_view)
}

/// Registers `observer` for appearance trait changes on `ui_view`.
///
/// Answers `false` on iOS 16 and earlier, where the API does not exist and the
/// caller has to fall back to the view-hierarchy override.
fn register_for_trait_changes(ui_view: *mut Object, observer: *mut Object) -> bool {
    let selector = sel!(registerForTraitChanges:withTarget:action:);
    let responds: bool = unsafe { msg_send![ui_view, respondsToSelector: selector] };
    if !responds {
        return false;
    }
    // Narrowed to the one trait that matters: a rotation or a size-class change
    // must not cost a callback.
    let Some(trait_class) = Class::get("UITraitUserInterfaceStyle") else {
        return false;
    };
    unsafe {
        let traits: *mut Object = msg_send![class!(NSArray), arrayWithObject: trait_class];
        if traits.is_null() {
            return false;
        }
        let registration: *mut Object = msg_send![
            ui_view,
            registerForTraitChanges: traits
            withTarget: observer
            action: sel!(aimerAppearanceDidChangeIn:)
        ];
        // The registration is owned by the observed view and lasts as long as
        // the observer does, so the token is not kept.
        !registration.is_null()
    }
}

/// Registers the observer class with the Objective-C runtime, then instantiates
/// it.
///
/// The class subclasses `UIView` so it can join the view hierarchy on iOS 16
/// and earlier; on iOS 17 and later it never becomes visible and only serves as
/// an action target.
fn create_observer() -> Option<*mut Object> {
    let class = observer_class()?;
    let observer: *mut Object = unsafe { msg_send![class, new] };
    (!observer.is_null()).then_some(observer)
}

/// The observer class, registered on first use.
fn observer_class() -> Option<&'static Class> {
    if let Some(existing) = Class::get(OBSERVER_CLASS_NAME) {
        return Some(existing);
    }
    let superclass = Class::get("UIView")?;
    let mut decl = ClassDecl::new(OBSERVER_CLASS_NAME, superclass)?;
    unsafe {
        decl.add_method(
            sel!(traitCollectionDidChange:),
            trait_collection_did_change as extern "C" fn(&Object, Sel, *mut Object),
        );
        decl.add_method(
            sel!(aimerAppearanceDidChangeIn:),
            appearance_did_change_in as extern "C" fn(&Object, Sel, *mut Object),
        );
    }
    Some(decl.register())
}

/// iOS 16 and earlier: the appearance of the hierarchy this view sits in
/// changed.
extern "C" fn trait_collection_did_change(this: &Object, _cmd: Sel, previous: *mut Object) {
    unsafe {
        let _: () = msg_send![super(this, class!(UIView)), traitCollectionDidChange: previous];
    }
    report(this as *const Object as *mut Object);
}

/// iOS 17 and later: the `UITraitUserInterfaceStyle` of `environment` changed.
extern "C" fn appearance_did_change_in(_this: &Object, _cmd: Sel, environment: *mut Object) {
    report(environment);
}

/// Reports the appearance `trait_environment` is now in, and asks for the one
/// frame that redraws it.
///
/// The appearance is read from the environment that was handed the change
/// rather than from the process-wide current trait collection, which is only
/// meaningful inside some callbacks; an environment that has no answer falls
/// back to it.
fn report(trait_environment: *mut Object) {
    let brightness = appearance_of(trait_environment).or_else(current_appearance);
    let Some(brightness) = brightness else {
        return;
    };
    if aimer_widget::set_platform_brightness(brightness) > 0 {
        aimer_events::window::request_animation_frame();
    }
}

/// Reads the appearance from a `UITraitEnvironment`'s trait collection.
fn appearance_of(trait_environment: *mut Object) -> Option<Brightness> {
    if trait_environment.is_null() {
        return None;
    }
    let traits: *mut Object = unsafe { msg_send![trait_environment, traitCollection] };
    appearance_of_trait_collection(traits)
}

/// Reads `userInterfaceStyle` off a `UITraitCollection`.
fn appearance_of_trait_collection(traits: *mut Object) -> Option<Brightness> {
    if traits.is_null() {
        return None;
    }
    let style: isize = unsafe { msg_send![traits, userInterfaceStyle] };
    crate::system_appearance::from_user_interface_style(style)
}
