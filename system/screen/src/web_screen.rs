//! The browser, from `screen.orientation.type`.
//!
//! The Screen Orientation API names the screen's position as primary or
//! secondary rather than by direction; [`DeviceOrientation`] documents how those
//! map. A browser without the API, or a window that does not exist, reports
//! nothing.

use web_sys::OrientationType;

use crate::DeviceOrientation;

pub(super) fn device_orientation() -> Option<DeviceOrientation> {
    let screen = web_sys::window()?.screen().ok()?;
    let kind = match screen.orientation().type_().ok()? {
        OrientationType::PortraitPrimary => "portrait-primary",
        OrientationType::PortraitSecondary => "portrait-secondary",
        OrientationType::LandscapePrimary => "landscape-primary",
        OrientationType::LandscapeSecondary => "landscape-secondary",
        _ => return None,
    };
    DeviceOrientation::from_screen_orientation_type(kind)
}
