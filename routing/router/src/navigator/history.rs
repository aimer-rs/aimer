use crate::NavigationAction;

pub(super) fn history_action(previous: Option<u64>, next: Option<u64>) -> NavigationAction {
    match (previous, next) {
        (Some(previous), Some(next)) if next < previous => NavigationAction::Pop,
        (Some(previous), Some(next)) if next > previous => NavigationAction::Push,
        _ => NavigationAction::History,
    }
}

fn parse_position(state: &str) -> Option<u64> {
    state.strip_prefix("aimer_router:v1:")?.parse().ok()
}

#[cfg(all(target_arch = "wasm32", not(aimer_portable_guest)))]
pub(super) fn browser_position(state: &wasm_bindgen::JsValue) -> Option<u64> {
    state.as_string().as_deref().and_then(parse_position)
}

#[cfg(all(target_arch = "wasm32", not(aimer_portable_guest)))]
pub(super) fn browser_current_position() -> Option<u64> {
    let state = web_sys::window()?.history().ok()?.state().ok()?;
    browser_position(&state)
}

#[cfg(all(target_arch = "wasm32", not(aimer_portable_guest)))]
pub(super) fn browser_push_state(path: &str, position: u64) {
    if let Some(history) = web_sys::window().and_then(|window| window.history().ok()) {
        let state = wasm_bindgen::JsValue::from_str(&format!("aimer_router:v1:{position}"));
        let _ = history.push_state_with_url(&state, "", Some(path));
    }
}

#[cfg(all(target_arch = "wasm32", not(aimer_portable_guest)))]
pub(crate) fn browser_replace_state(path: &str) {
    if let Some(history) = web_sys::window().and_then(|window| window.history().ok()) {
        // Redirects and shell updates must preserve the entry's direction marker.
        let state = history.state().unwrap_or(wasm_bindgen::JsValue::NULL);
        let _ = history.replace_state_with_url(&state, "", Some(path));
    }
}

#[cfg(all(target_arch = "wasm32", not(aimer_portable_guest)))]
pub(super) fn browser_current_path() -> Option<String> {
    let location = web_sys::window()?.location();
    let mut path = location.pathname().ok()?;
    if let Ok(query) = location.search() { path.push_str(&query); }
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NavigationAction;

    #[test]
    fn browser_entries_choose_direction_only_when_both_positions_are_known() {
        assert_eq!(history_action(Some(3), Some(2)), NavigationAction::Pop);
        assert_eq!(history_action(Some(2), Some(3)), NavigationAction::Push);
        assert_eq!(history_action(None, Some(1)), NavigationAction::History);
        assert_eq!(history_action(Some(1), None), NavigationAction::History);
        assert_eq!(history_action(Some(1), Some(1)), NavigationAction::History);
    }

    #[test]
    fn unrelated_and_invalid_browser_state_has_unknown_direction() {
        assert_eq!(parse_position("aimer_router:v1:42"), Some(42));
        for value in ["42", "other:v1:42", "aimer_router:v1:-1", "aimer_router:v1:NaN"] {
            assert_eq!(parse_position(value), None);
        }
    }
}
