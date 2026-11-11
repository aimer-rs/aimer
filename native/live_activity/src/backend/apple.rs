//! iOS: ActivityKit, through a Swift bridge. macOS looks for the same bridge
//! and never finds one.
//!
//! ActivityKit is a Swift-only framework: `Activity<Attributes>` is generic and
//! has no Objective-C surface, so Rust cannot call it the way it calls
//! `UNUserNotificationCenter`. The application ships a small Swift file
//! (`swift/AimerLiveActivityBridge.swift` in this crate) that exports three
//! `@_cdecl` functions, and this backend finds them with `dlsym`:
//!
//! | Symbol                           | Signature                       |
//! |----------------------------------|---------------------------------|
//! | `aimer_live_activity_availability` | `() -> Int32`                 |
//! | `aimer_live_activity_show`       | `(*const u8, usize) -> Int32`   |
//! | `aimer_live_activity_end`        | `(u64) -> Int32`                |
//!
//! `show` takes the activity as the JSON of [`crate::payload::encode`] and
//! starts or updates it. Every function answers with a status code that
//! [`crate::payload::bridge_status`] turns into a result.
//!
//! Looking the symbols up instead of linking them means an application that
//! does not ship the bridge still links and runs: [`create`] reports
//! [`LiveActivityError::Unsupported`], and the tray takes over if the
//! application offered an icon. That is also what happens on macOS, whose SDK
//! marks ActivityKit unavailable, so the bridge cannot be built there.

use std::ffi::{CStr, c_char, c_void};

use super::Backend;
use crate::payload::{self, bridge_status};
use crate::{LiveActivity, LiveActivityError, LiveActivityId};

/// `RTLD_DEFAULT`: search every image loaded into the process.
const RTLD_DEFAULT: *mut c_void = -2isize as *mut c_void;

unsafe extern "C" {
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}

type AvailabilityFn = unsafe extern "C" fn() -> i32;
type ShowFn = unsafe extern "C" fn(*const u8, usize) -> i32;
type EndFn = unsafe extern "C" fn(u64) -> i32;

/// Finds the exported function `name`, or `None` when no image has it.
///
/// # Safety
///
/// `F` must be the `extern "C"` function pointer type the symbol really has.
unsafe fn symbol<F: Copy>(name: &CStr) -> Option<F> {
    const { assert!(size_of::<F>() == size_of::<*mut c_void>()) };
    // SAFETY: `RTLD_DEFAULT` is a valid handle and `name` is NUL-terminated.
    let address = unsafe { dlsym(RTLD_DEFAULT, name.as_ptr()) };
    // SAFETY: the caller vouches for `F`, and the sizes match (asserted above).
    (!address.is_null()).then(|| unsafe { std::mem::transmute_copy::<*mut c_void, F>(&address) })
}

struct Apple {
    show: ShowFn,
    end: EndFn,
}

pub(super) fn create() -> Result<Box<dyn Backend>, LiveActivityError> {
    // SAFETY: each type is the signature `AimerLiveActivityBridge.swift`
    // exports the symbol with.
    let (availability, show, end) = unsafe {
        (
            symbol::<AvailabilityFn>(c"aimer_live_activity_availability"),
            symbol::<ShowFn>(c"aimer_live_activity_show"),
            symbol::<EndFn>(c"aimer_live_activity_end"),
        )
    };
    let (Some(availability), Some(show), Some(end)) = (availability, show, end) else {
        return Err(LiveActivityError::Unsupported);
    };
    // SAFETY: the symbol takes no argument and has no precondition.
    let code = unsafe { availability() };
    // A system without ActivityKit has nothing to show on. A user who turned
    // live activities off is reported by `show`, because they can turn them on
    // again while the application runs.
    if code == payload::STATUS_OLD_SYSTEM {
        return Err(LiveActivityError::Unsupported);
    }
    Ok(Box::new(Apple { show, end }))
}

impl Backend for Apple {
    fn show(&self, activity: &LiveActivity) -> Result<(), LiveActivityError> {
        let json = payload::encode(activity);
        // SAFETY: the pointer and length describe `json`, which outlives the
        // call; the bridge copies what it needs before returning.
        let code = unsafe { (self.show)(json.as_ptr(), json.len()) };
        bridge_status(code)
    }

    fn end(&self, id: LiveActivityId) -> Result<(), LiveActivityError> {
        // SAFETY: the symbol takes a plain integer.
        let code = unsafe { (self.end)(id.0) };
        bridge_status(code)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_the_bridge_linked_the_native_surface_is_unsupported() {
        // The test binary does not contain the Swift bridge.
        assert_eq!(create().err(), Some(LiveActivityError::Unsupported));
    }
}
