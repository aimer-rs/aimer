//! macOS, from `sysctl` and `NSProcessInfo`.
//!
//! Nothing here goes through AppKit, so the query is safe from any thread.

use crate::apple_device::{host_name, os_version, sysctl_string, sysctl_u32, sysctl_u64};
use crate::{
    DeviceInfo, DeviceInfoError, DeviceKind, MacOsDetails, Platform, PlatformDetails, Queried,
};

pub(super) fn query() -> Result<Queried, DeviceInfoError> {
    let info = DeviceInfo {
        platform: Platform::MacOs,
        kind: DeviceKind::Desktop,
        os_name: "macOS".into(),
        os_version: os_version(),
        os_build: sysctl_string(c"kern.osversion"),
        model: sysctl_string(c"hw.model"),
        manufacturer: Some("Apple".into()),
        name: host_name(),
        arch: std::env::consts::ARCH,
        // `kern.hv_vmm_present` is 1 inside a virtual machine and absent or 0
        // on bare metal.
        is_physical: sysctl_u32(c"kern.hv_vmm_present") != Some(1),
        details: PlatformDetails::MacOs(MacOsDetails {
            cpu_brand: sysctl_string(c"machdep.cpu.brand_string"),
            active_cpus: sysctl_u32(c"hw.activecpu").unwrap_or(0),
            memory_bytes: sysctl_u64(c"hw.memsize").unwrap_or(0),
        }),
    };
    Ok(Queried {
        info,
        complete: true,
    })
}
