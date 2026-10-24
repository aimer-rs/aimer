//! The browser's safe area, read through the `env(safe-area-inset-*)` CSS
//! variables.
//!
//! A page on a notched phone gets the same treatment a native application does:
//! the notch and the home indicator overlap the viewport, and the browser states
//! how much of each edge that costs — but only inside CSS, as the four
//! `env(safe-area-inset-*)` variables. There is no DOM property and no event
//! carrying them, so they are read the one way CSS values can be read from
//! Rust: a value is *asked for* in a stylesheet declaration and the browser's
//! answer is taken back off the computed style.
//!
//! The asking is done by a probe — one empty, hidden, non-interactive `div`
//! whose four paddings are declared as the four environment variables. It is
//! created once, never removed, and takes part in neither layout nor hit
//! testing:
//!
//! ```css
//! position: fixed; top: 0; left: 0; width: 0; height: 0;
//! visibility: hidden; pointer-events: none;
//! padding: env(safe-area-inset-top) env(safe-area-inset-right)
//!          env(safe-area-inset-bottom) env(safe-area-inset-left);
//! ```
//!
//! Reading it back costs one `getComputedStyle` and four property lookups, on
//! the events that can have changed the answer — the window resizing, or the
//! device turning — and never per frame.
//!
//! The variables are only ever non-zero when the document asks to be laid out
//! under the cutouts in the first place, with `viewport-fit=cover` in its
//! viewport meta tag; a page that does not is reported as fully usable, which is
//! exactly what it is.

use aimer_widget::SafeAreaInsets;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use web_sys::{Element, Event};

/// The declarations that make the probe invisible, weightless and untouchable.
const PROBE_STYLE: &str = "position:fixed;top:0;left:0;width:0;height:0;\
     visibility:hidden;pointer-events:none;\
     padding-top:env(safe-area-inset-top,0px);\
     padding-right:env(safe-area-inset-right,0px);\
     padding-bottom:env(safe-area-inset-bottom,0px);\
     padding-left:env(safe-area-inset-left,0px);";

std::thread_local! {
    /// The probe, once it is in the document.
    ///
    /// The browser is single-threaded and so is everything here, which is why
    /// the element lives in thread-local storage rather than behind a lock.
    static PROBE: std::cell::RefCell<Option<Element>> = const { std::cell::RefCell::new(None) };

    /// Whether the resize listeners have been installed.
    static OBSERVED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Reports the region the browser reserves in the viewport, and asks for the
/// one frame that redraws it.
///
/// A reservation that is already in effect asks for no frame, and neither does
/// one no widget follows — see [`aimer_widget::set_safe_area_insets`]. A
/// document the probe cannot be attached to reports nothing at all.
///
/// # Examples
///
/// ```ignore
/// // The device turned; the cutouts are on other edges now.
/// report_safe_area();
/// ```
pub fn report_safe_area() {
    let Some(insets) = read_probe() else {
        return;
    };
    if aimer_widget::set_safe_area_insets(insets) > 0 {
        aimer_events::window::request_animation_frame();
    }
}

/// Starts reporting the reservation whenever the viewport's shape changes.
///
/// Repeated calls after the first are ignored: one pair of listeners hears
/// every change. `resize` covers the rotation on every current browser and
/// `orientationchange` is kept for the ones that only fire that, so both are
/// listened to; a rotation that fires both reports twice and marks nothing the
/// second time.
///
/// # Examples
///
/// ```ignore
/// // Called once, right after the window exists.
/// observe_safe_area_changes();
/// ```
pub fn observe_safe_area_changes() {
    if OBSERVED.get() {
        return;
    }
    let Some(window) = web_sys::window() else {
        return;
    };
    OBSERVED.set(true);

    let listener = Closure::<dyn FnMut(Event)>::new(|_: Event| report_safe_area());
    let callback = listener.as_ref().unchecked_ref();
    for name in ["resize", "orientationchange"] {
        let _ = window.add_event_listener_with_callback(name, callback);
    }
    // The listeners live as long as the document does, so the closure is handed
    // to the browser rather than kept and dropped.
    listener.forget();
}

/// Reads the four environment variables off the probe's computed style.
///
/// `None` when there is no document to probe, which is the answer in a worker
/// and before the page has a body.
fn read_probe() -> Option<SafeAreaInsets> {
    let window = web_sys::window()?;
    let probe = probe(&window)?;
    let style = window.get_computed_style(&probe).ok()??;
    let edge = |name: &str| {
        let value = style.get_property_value(name).unwrap_or_default();
        crate::system_safe_area::from_css_pixels(&value)
    };
    Some(SafeAreaInsets::new(
        edge("padding-left"),
        edge("padding-top"),
        edge("padding-right"),
        edge("padding-bottom"),
    ))
}

/// The probe, created and attached to the document on first use.
fn probe(window: &web_sys::Window) -> Option<Element> {
    if let Some(existing) = PROBE.with(|probe| probe.borrow().clone()) {
        return Some(existing);
    }
    let document = window.document()?;
    let body = document.body()?;
    let probe = document.create_element("div").ok()?;
    probe.set_attribute("style", PROBE_STYLE).ok()?;
    probe.set_attribute("aria-hidden", "true").ok()?;
    body.append_child(&probe).ok()?;
    PROBE.with(|slot| *slot.borrow_mut() = Some(probe.clone()));
    Some(probe)
}
