//! Linux, from `os-release(5)`, `uname(2)` and the firmware tables in `/sys`.
//!
//! Every file is optional: a container has no DMI tables and a minimal image
//! may have no `os-release`, so each missing piece is a `None`, not a failure.

use std::ffi::CStr;
use std::fs;
use std::mem::MaybeUninit;

use crate::linux_parse::{has_hypervisor_flag, kind_from_chassis_type, parse_os_release};
use crate::{
    DeviceInfo, DeviceInfoError, DeviceKind, LinuxDetails, OsVersion, Platform, PlatformDetails,
    Queried,
};

const DMI: &str = "/sys/class/dmi/id";

pub(super) fn query() -> Result<Queried, DeviceInfoError> {
    let (kernel_release, host_name) = uname()?;
    let release = parse_os_release(
        &fs::read_to_string("/etc/os-release")
            .or_else(|_| fs::read_to_string("/usr/lib/os-release"))
            .unwrap_or_default(),
    );

    // Boards without firmware tables (a Raspberry Pi) name themselves in the
    // device tree instead.
    let model = read_trimmed(&format!("{DMI}/product_name"))
        .or_else(|| read_trimmed("/proc/device-tree/model"));
    let kind = read_trimmed(&format!("{DMI}/chassis_type"))
        .and_then(|code| code.parse().ok())
        .map_or(DeviceKind::Desktop, kind_from_chassis_type);

    let info = DeviceInfo {
        platform: Platform::Linux,
        kind,
        os_name: release.name.unwrap_or_else(|| "Linux".into()),
        os_version: release
            .version_id
            .as_deref()
            .map_or_else(OsVersion::default, OsVersion::parse),
        os_build: release.build_id,
        model,
        manufacturer: read_trimmed(&format!("{DMI}/sys_vendor")),
        name: (!host_name.is_empty()).then_some(host_name),
        arch: std::env::consts::ARCH,
        is_physical: !fs::read_to_string("/proc/cpuinfo").is_ok_and(|text| has_hypervisor_flag(&text)),
        details: PlatformDetails::Linux(LinuxDetails {
            id: release.id,
            pretty_name: release.pretty_name,
            kernel_release,
        }),
    };
    Ok(Queried {
        info,
        complete: true,
    })
}

/// Reads a one-line file, dropping surrounding whitespace and the trailing NUL
/// that device-tree strings carry.
fn read_trimmed(path: &str) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    let text = text.trim_matches(|c: char| c.is_whitespace() || c == '\0');
    (!text.is_empty()).then(|| text.to_owned())
}

/// The kernel release and the node (host) name from `uname(2)`.
fn uname() -> Result<(String, String), DeviceInfoError> {
    let mut raw = MaybeUninit::<libc::utsname>::zeroed();
    // SAFETY: `uname` fills the struct it is given.
    if unsafe { libc::uname(raw.as_mut_ptr()) } != 0 {
        return Err(DeviceInfoError::Unavailable("uname failed".into()));
    }
    // SAFETY: the call succeeded, so every field is initialised.
    let raw = unsafe { raw.assume_init() };
    let text = |field: &[libc::c_char]| {
        // SAFETY: `uname` NUL-terminates each fixed-size field it fills.
        unsafe { CStr::from_ptr(field.as_ptr()) }
            .to_string_lossy()
            .into_owned()
    };
    Ok((text(&raw.release), text(&raw.nodename)))
}
