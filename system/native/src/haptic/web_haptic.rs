use std::sync::LazyLock;
use wasm_bindgen::JsCast;
use web_sys::window;
use web_sys::{HtmlElement, HtmlInputElement};

static IS_IOS_SAFARI: LazyLock<bool> = LazyLock::new(is_ios_safari);

pub fn web_perform_haptic() {
    // if *IS_IOS_SAFARI {
    //     return;
    // }

    let Some(window) = window() else { return };
    let Some(document) = window.document() else {
        return;
    };
    let Some(body) = document.body() else { return };

    // Try to find an existing switch element first
    if let Ok(Some(existing)) = document.query_selector("[data-haptic-trigger]") {
        if existing.dyn_into::<HtmlInputElement>().is_ok() {
            return; // already exists, nothing to do
        }
    }

    // Not found — create a new one
    let Ok(switch_el) = document.create_element("input") else {
        return;
    };
    let Ok(switch_el) = switch_el.dyn_into::<HtmlInputElement>() else {
        return;
    };

    switch_el.set_type("checkbox");
    let _ = switch_el.set_attribute("switch", "");
    let _ = switch_el.set_attribute("data-haptic-trigger", "");
    let _ = switch_el.set_attribute("aria-hidden", "true");
    switch_el.set_tab_index(-1);

    // Apply inline styles
    let style = switch_el.style();
    let _ = style.set_property("position", "absolute");
    let _ = style.set_property("inset", "0");
    let _ = style.set_property("width", "100%");
    let _ = style.set_property("height", "100%");
    let _ = style.set_property("margin", "0");
    let _ = style.set_property("opacity", "0");
    let _ = style.set_property("clip-path", "inset(0 round 999px)");
    let _ = style.set_property("touch-action", "manipulation");
    let _ = style.set_property("-webkit-tap-highlight-color", "transparent");

    // Ensure body has relative positioning if it's static
    if let Ok(Some(computed_style)) = window.get_computed_style(&body) {
        if let Ok(position) = computed_style.get_property_value("position") {
            if position == "static" {
                let _ = body.style().set_property("position", "relative");
            }
        }
    }

    // Append to body
    let _ = body.append_child(&switch_el);
}

fn is_ios_safari() -> bool {
    let Some(window) = window() else { return false };
    let Some(navigator) = Some(window.navigator()) else {
        return false;
    };

    let Ok(user_agent) = navigator.user_agent() else {
        return false;
    };
    let ua = user_agent.to_lowercase();

    let is_ios = ua.contains("iphone") || ua.contains("ipad") || ua.contains("ipod")
        // iPadOS 13+ reports as Mac, so check for touch support too
        || (ua.contains("macintosh") && is_touch_device());

    let is_safari = ua.contains("safari")
        && !ua.contains("crios")
        && !ua.contains("fxios")
        && !ua.contains("edgios");

    is_ios && is_safari
}

fn is_touch_device() -> bool {
    window()
        .and_then(|w| w.navigator().max_touch_points().try_into().ok())
        .map(|points: i32| points > 0)
        .unwrap_or(false)
}
