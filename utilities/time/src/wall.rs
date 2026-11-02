/// Reads the wall clock as whole seconds and sub-second nanoseconds since the
/// Unix epoch (UTC). `nanos` is always below one billion.
///
/// Native targets use [`std::time::SystemTime`]; the browser has no such
/// clock, so wasm reads `Date.now()` through `js-sys`, which has millisecond
/// resolution.
pub(crate) fn unix_now() -> (i64, u32) {
    #[cfg(target_arch = "wasm32")]
    {
        let millis = js_sys::Date::now().floor() as i64;
        (millis.div_euclid(1_000), (millis.rem_euclid(1_000) * 1_000_000) as u32)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::time::{SystemTime, UNIX_EPOCH};
        match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(after) => (after.as_secs() as i64, after.subsec_nanos()),
            Err(err) => {
                // The clock is set before 1970: floor towards negative infinity.
                let before = err.duration();
                match before.subsec_nanos() {
                    0 => (-(before.as_secs() as i64), 0),
                    nanos => (-(before.as_secs() as i64) - 1, 1_000_000_000 - nanos),
                }
            }
        }
    }
}
