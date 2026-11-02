//! The browser, from `navigator` and its user-agent string.
//!
//! A browser tells a page very little on purpose, so most of this is inferred
//! from the user agent (see [`crate::user_agent`]) and the model and name are
//! always unknown. `is_physical` is `true`: nothing in a page can tell a
//! browser on a real machine from one in a virtual machine.

use crate::user_agent::parse;
use crate::{
    DeviceInfo, DeviceInfoError, DeviceKind, OsVersion, Platform, PlatformDetails, Queried,
    WebDetails,
};

pub(super) fn query() -> Result<Queried, DeviceInfoError> {
    let navigator = web_sys::window()
        .ok_or_else(|| DeviceInfoError::Unavailable("the browser window is missing".into()))?
        .navigator();
    let user_agent = navigator.user_agent().unwrap_or_default();
    let max_touch_points = navigator.max_touch_points().max(0) as u32;
    let mut agent = parse(&user_agent);

    // iPadOS 13 and later sends a Mac's user agent so sites serve it the
    // desktop page; a "Mac" with a multi-touch screen is an iPad. Its frozen
    // `10_15_7` is not an iPadOS version, so none is claimed.
    if agent.os_name == "macOS" && max_touch_points > 1 {
        agent.os_name = "iPadOS";
        agent.os_version = OsVersion::default();
        agent.kind = DeviceKind::Tablet;
    }

    let info = DeviceInfo {
        platform: Platform::Web,
        kind: agent.kind,
        os_name: agent.os_name.into(),
        os_version: agent.os_version,
        os_build: None,
        model: agent.model,
        manufacturer: None,
        name: None,
        arch: std::env::consts::ARCH,
        is_physical: true,
        details: PlatformDetails::Web(WebDetails {
            platform: navigator.platform().unwrap_or_default(),
            language: navigator.language().unwrap_or_default(),
            hardware_concurrency: navigator.hardware_concurrency().max(0.0) as u32,
            max_touch_points,
            user_agent,
        }),
    };
    Ok(Queried {
        info,
        complete: true,
    })
}
