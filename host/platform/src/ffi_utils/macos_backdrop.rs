//! The blurred window background of macOS: an `NSVisualEffectView`, or on macOS
//! 26 and later the glass of an `NSGlassEffectView`.
//!
//! The renderer's `CAMetalLayer` is the layer of the window's content view and
//! always draws over that view's own subviews, so the effect view cannot be put
//! inside the content view. It goes one level up instead, into the view that
//! holds the content view, directly below it. Everything the application draws
//! transparently then shows the effect behind it.

use std::cell::RefCell;
use std::collections::HashMap;

use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject};
use objc2::{MainThreadMarker, msg_send};
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSColor, NSGlassEffectView, NSGlassEffectViewStyle, NSView,
    NSVisualEffectBlendingMode, NSVisualEffectMaterial, NSVisualEffectState, NSVisualEffectView,
    NSWindow, NSWindowOrderingMode,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window;

use crate::window_backdrop::WindowMaterial;

/// The view that draws a window's backdrop.
enum Effect {
    Blur(Retained<NSVisualEffectView>),
    Glass(Retained<NSGlassEffectView>),
}

impl Effect {
    fn view(&self) -> &NSView {
        match self {
            Self::Blur(view) => view,
            Self::Glass(view) => view,
        }
    }

    fn superview(&self) -> Option<Retained<NSView>> {
        // SAFETY: nothing mutates the view hierarchy while this is read.
        unsafe { self.view().superview() }
    }

    fn remove(&self) {
        self.view().removeFromSuperview();
    }
}

/// What a window looked like before it was given a backdrop, and the view that
/// gives it one.
struct Applied {
    effect: Effect,
    was_opaque: bool,
    was_background: Retained<NSColor>,
    /// Whether the content view's layer was opaque; `None` when it has no layer.
    layer_was_opaque: Option<bool>,
}

aimer_tls::thread_local! {
    /// The windows that have a backdrop, by the address of their `NSWindow`.
    ///
    /// AppKit objects live on the main thread only, so this does too. An entry
    /// outlives its window if the window closes with its backdrop on; the view it
    /// holds is checked against the window before it is reused.
    static APPLIED: RefCell<HashMap<usize, Applied>> = RefCell::new(HashMap::new());
}

/// Gives `window` a backdrop of `material`, or removes it.
///
/// Returns whether the request was carried out.
pub fn apply(window: &Window, material: Option<WindowMaterial>) -> bool {
    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    let Some(ns_window) = native_window(window) else {
        return false;
    };
    match material {
        Some(material) => install(mtm, &ns_window, material),
        None => {
            remove(&ns_window);
            true
        }
    }
}

fn native_window(window: &Window) -> Option<Retained<NSWindow>> {
    let handle = window.window_handle().ok()?;
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return None;
    };
    // SAFETY: `window_handle` guarantees that the `NSView` stays valid for as
    // long as the borrowed winit window does, and it is retained here before
    // its window is asked for.
    let view = unsafe { Retained::<NSView>::retain(handle.ns_view.as_ptr().cast()) }?;
    view.window()
}

fn key(window: &NSWindow) -> usize {
    std::ptr::from_ref(window) as usize
}

/// Whether this system has glass views, which arrived with macOS 26.
fn glass_available() -> bool {
    AnyClass::get(c"NSGlassEffectView").is_some()
}

/// The glass style `material` asks for, if it asks for glass at all.
fn glass_style(material: WindowMaterial) -> Option<NSGlassEffectViewStyle> {
    match material {
        WindowMaterial::Glass => Some(NSGlassEffectViewStyle::Regular),
        WindowMaterial::ClearGlass => Some(NSGlassEffectViewStyle::Clear),
        _ => None,
    }
}

fn appkit_material(material: WindowMaterial) -> NSVisualEffectMaterial {
    match material {
        // The closest blur, for a system without glass.
        WindowMaterial::Glass | WindowMaterial::ClearGlass => NSVisualEffectMaterial::Sidebar,
        WindowMaterial::Sidebar => NSVisualEffectMaterial::Sidebar,
        WindowMaterial::Titlebar => NSVisualEffectMaterial::Titlebar,
        WindowMaterial::Content => NSVisualEffectMaterial::ContentBackground,
        WindowMaterial::Menu => NSVisualEffectMaterial::Menu,
        WindowMaterial::Popover => NSVisualEffectMaterial::Popover,
        WindowMaterial::Hud => NSVisualEffectMaterial::HUDWindow,
        WindowMaterial::Under => NSVisualEffectMaterial::UnderWindowBackground,
    }
}

/// Whether `layer`, an `CALayer`, is opaque.
///
/// # Safety
///
/// `layer` must be a live `CALayer`.
unsafe fn layer_is_opaque(layer: &AnyObject) -> bool {
    // SAFETY: the caller guarantees a live `CALayer`, which answers `isOpaque`.
    unsafe { msg_send![layer, isOpaque] }
}

/// # Safety
///
/// `layer` must be a live `CALayer`.
unsafe fn set_layer_opaque(layer: &AnyObject, opaque: bool) {
    // SAFETY: the caller guarantees a live `CALayer`, which takes `setOpaque:`.
    let () = unsafe { msg_send![layer, setOpaque: opaque] };
}

fn content_layer(content: &NSView) -> Option<Retained<AnyObject>> {
    // SAFETY: every `NSView` answers `layer`; the result is a `CALayer` or nil.
    unsafe { msg_send![content, layer] }
}

fn install(mtm: MainThreadMarker, window: &NSWindow, material: WindowMaterial) -> bool {
    let Some(content) = window.contentView() else {
        return false;
    };
    // SAFETY: the view is retained by the returned handle for as long as it is
    // used here, and nothing mutates the hierarchy meanwhile.
    let Some(frame_view) = (unsafe { content.superview() }) else {
        return false;
    };
    let appkit = appkit_material(material);
    let glass = glass_style(material).filter(|_| glass_available());

    // A view that is still in this window's hierarchy and of the wanted kind is
    // only retuned; one of the other kind is replaced, and the window keeps what
    // it looked like before any backdrop.
    let mut previous = None;
    let reused = APPLIED.with_borrow_mut(|applied| {
        let in_place = applied.get(&key(window)).is_some_and(|entry| {
            entry
                .effect
                .superview()
                .is_some_and(|parent| std::ptr::eq(&*parent, &*frame_view))
        });
        if !in_place {
            return false;
        }
        let entry = applied.get(&key(window)).expect("checked just above");
        match (&entry.effect, glass) {
            (Effect::Blur(view), None) => {
                view.setMaterial(appkit);
                true
            }
            (Effect::Glass(view), Some(style)) => {
                view.setStyle(style);
                true
            }
            _ => {
                previous = applied.remove(&key(window));
                false
            }
        }
    });
    if reused {
        return true;
    }

    let effect = match glass {
        Some(style) => {
            let view = NSGlassEffectView::new(mtm);
            view.setStyle(style);
            Effect::Glass(view)
        }
        None => {
            let view = NSVisualEffectView::new(mtm);
            view.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
            // Blur even when the window is not the active one, so a window does
            // not flash to a flat color each time focus moves away from it.
            view.setState(NSVisualEffectState::Active);
            view.setMaterial(appkit);
            Effect::Blur(view)
        }
    };
    let view = effect.view();
    view.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    view.setFrame(frame_view.bounds());
    frame_view.addSubview_positioned_relativeTo(view, NSWindowOrderingMode::Below, Some(&content));

    if let Some(previous) = previous {
        previous.effect.remove();
        APPLIED.with_borrow_mut(|applied| {
            applied.insert(
                key(window),
                Applied {
                    effect,
                    ..previous
                },
            )
        });
        return true;
    }

    let layer = content_layer(&content);
    let layer_was_opaque = layer.as_deref().map(|layer| {
        // SAFETY: the content view's layer is a `CALayer`.
        let opaque = unsafe { layer_is_opaque(layer) };
        unsafe { set_layer_opaque(layer, false) };
        opaque
    });
    let was_opaque = window.isOpaque();
    let was_background = window.backgroundColor();
    window.setOpaque(false);
    window.setBackgroundColor(Some(&NSColor::clearColor()));

    let entry = Applied {
        effect,
        was_opaque,
        was_background,
        layer_was_opaque,
    };
    let stale = APPLIED.with_borrow_mut(|applied| applied.insert(key(window), entry));
    if let Some(stale) = stale {
        // A closed window's entry whose address this window now has.
        stale.effect.remove();
    }
    true
}

fn remove(window: &NSWindow) {
    let Some(entry) = APPLIED.with_borrow_mut(|applied| applied.remove(&key(window))) else {
        return;
    };
    entry.effect.remove();
    if entry.was_opaque {
        window.setOpaque(true);
    }
    window.setBackgroundColor(Some(&entry.was_background));
    if let (Some(opaque), Some(content)) = (entry.layer_was_opaque, window.contentView()) {
        if let Some(layer) = content_layer(&content) {
            // SAFETY: the content view's layer is a `CALayer`.
            unsafe { set_layer_opaque(&layer, opaque) };
        }
    }
}
