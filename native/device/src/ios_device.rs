//! iOS and iPadOS, from `sysctl`, `NSProcessInfo` and `UIDevice`.
//!
//! The hardware identifier and the version are thread-safe. The device's name
//! and `identifierForVendor` are UIKit properties that `objc2` only exposes
//! behind a [`MainThreadMarker`], so they are read when the caller is on the
//! main thread and left out — with the answer marked incomplete, so it is not
//! cached — when it is not.

use objc2::MainThreadMarker;
use objc2_ui_kit::UIDevice;

use crate::apple_device::{os_version, sysctl_string};
use crate::{
    DeviceInfo, DeviceInfoError, DeviceKind, IosDetails, Platform, PlatformDetails, Queried,
};

/// Whether this binary can only run on the iOS simulator: the `-sim` targets,
/// and the x86-64 one, which predates the `sim` ABI marker.
const IS_SIMULATOR: bool = cfg!(target_abi = "sim") || cfg!(target_arch = "x86_64");

pub(super) fn query() -> Result<Queried, DeviceInfoError> {
    // On the simulator `hw.machine` names the host's CPU (`arm64`); the device
    // being simulated is in the environment.
    let model = if IS_SIMULATOR {
        std::env::var("SIMULATOR_MODEL_IDENTIFIER").ok()
    } else {
        sysctl_string(c"hw.machine")
    };
    let kind = match model.as_deref() {
        Some(model) if model.starts_with("iPhone") || model.starts_with("iPod") => {
            DeviceKind::Phone
        }
        Some(model) if model.starts_with("iPad") => DeviceKind::Tablet,
        _ => DeviceKind::Unknown,
    };

    let main_thread = MainThreadMarker::new();
    let (name, identifier_for_vendor) = match main_thread {
        Some(mtm) => {
            let device = UIDevice::currentDevice(mtm);
            (
                Some(device.name().to_string()),
                device
                    .identifierForVendor()
                    .map(|uuid| uuid.UUIDString().to_string()),
            )
        }
        None => (None, None),
    };

    let info = DeviceInfo {
        platform: Platform::Ios,
        kind,
        os_name: if kind == DeviceKind::Tablet { "iPadOS" } else { "iOS" }.into(),
        os_version: os_version(),
        os_build: sysctl_string(c"kern.osversion"),
        model,
        manufacturer: Some("Apple".into()),
        name,
        arch: std::env::consts::ARCH,
        is_physical: !IS_SIMULATOR,
        details: PlatformDetails::Ios(IosDetails {
            is_ipad: kind == DeviceKind::Tablet,
            identifier_for_vendor,
        }),
    };
    Ok(Queried {
        info,
        complete: main_thread.is_some(),
    })
}
