//! iOS, from `UIDevice.orientation`.
//!
//! That is the physical orientation the accelerometer reports, which is not the
//! orientation of the interface: an app locked to portrait is still told when the
//! phone is laid on its side. UIKit only starts reporting after
//! `beginGeneratingDeviceOrientationNotifications`, so the first query turns it
//! on; the answer may still be unknown at that moment and is right from the next
//! query, which a rotation always provides.
//!
//! UIKit answers on the main thread only, which `objc2` enforces with a
//! [`MainThreadMarker`]: from any other thread the orientation is unknown.

use std::cell::Cell;

use objc2::MainThreadMarker;
use objc2_ui_kit::UIDevice;

use crate::DeviceOrientation;

thread_local! {
    static GENERATING: Cell<bool> = const { Cell::new(false) };
}

pub(super) fn device_orientation() -> Option<DeviceOrientation> {
    let device = UIDevice::currentDevice(MainThreadMarker::new()?);
    if !GENERATING.with(|generating| generating.replace(true)) {
        device.beginGeneratingDeviceOrientationNotifications();
    }
    DeviceOrientation::from_ui_device_orientation(device.orientation().0)
}
