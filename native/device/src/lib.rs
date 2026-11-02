//! What this process can learn about the device it runs on.
//!
//! [`device_info`] returns one [`DeviceInfo`]: the form factor, operating
//! system, model, manufacturer and name that every platform can state, plus a
//! [`PlatformDetails`] section for the fields only one operating system has
//! (Android's API level, a Mac's memory, a browser's user agent).
//!
//! The answer does not change while the process runs, so it is gathered once
//! and every later call returns the same shared [`Arc`] without touching the
//! operating system. The first call is the expensive one — a few `sysctl`s, a
//! JNI round-trip, a registry read — and should not be made inside a frame.
//!
//! # Examples
//!
//! ```
//! use aimer_device::{DeviceInfo, PlatformDetails, device_info};
//!
//! match device_info() {
//!     Ok(info) => {
//!         println!("{} {}", info.os_name, info.os_version);
//!         if let PlatformDetails::Android(android) = &info.details {
//!             println!("API level {}", android.sdk_int);
//!         }
//!     }
//!     // A caller that merely decorates a UI ignores the failure.
//!     Err(error) => eprintln!("no device info: {error}"),
//! }
//! ```

use std::fmt::{Display, Formatter};
use std::sync::{Arc, OnceLock};

mod info;
mod os_version;

#[cfg(target_os = "android")]
mod android_device;
#[cfg(any(target_os = "macos", target_os = "ios"))]
mod apple_device;
#[cfg(target_os = "ios")]
mod ios_device;
#[cfg(target_os = "linux")]
mod linux_device;
#[cfg(any(target_os = "linux", test))]
mod linux_parse;
#[cfg(target_os = "macos")]
mod macos_device;
#[cfg(any(all(target_arch = "wasm32", not(aimer_portable_guest)), test))]
mod user_agent;
#[cfg(any(target_os = "android", target_os = "windows", test))]
mod virtualization;
#[cfg(all(target_arch = "wasm32", not(aimer_portable_guest)))]
mod web_device;
#[cfg(target_os = "windows")]
mod windows_device;

#[cfg(target_os = "android")]
use android_device as platform;
#[cfg(target_os = "ios")]
use ios_device as platform;
#[cfg(target_os = "linux")]
use linux_device as platform;
#[cfg(target_os = "macos")]
use macos_device as platform;
#[cfg(all(target_arch = "wasm32", not(aimer_portable_guest)))]
use web_device as platform;
#[cfg(target_os = "windows")]
use windows_device as platform;

#[cfg(all(target_arch = "wasm32", aimer_portable_guest))]
mod platform {
    use super::{DeviceInfoError, Queried};

    pub(super) fn query() -> Result<Queried, DeviceInfoError> {
        Err(DeviceInfoError::Unsupported)
    }
}

pub use info::{
    AndroidDetails, DeviceInfo, DeviceKind, IosDetails, LinuxDetails, MacOsDetails, Platform,
    PlatformDetails, WebDetails, WindowsDetails,
};
pub use os_version::OsVersion;

/// Why the device could not be described.
///
/// # Examples
///
/// ```
/// use aimer_device::DeviceInfoError;
///
/// let error = DeviceInfoError::Unavailable("the Android activity is not running".into());
/// assert_eq!(
///     error.to_string(),
///     "device information is unavailable: the Android activity is not running"
/// );
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceInfoError {
    /// The platform can describe itself, but the query failed — an Android
    /// activity that has not started, a browser window that does not exist.
    /// Retrying later may succeed.
    Unavailable(String),
    /// This target has no way to describe the device.
    Unsupported,
}

impl Display for DeviceInfoError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable(message) => {
                write!(formatter, "device information is unavailable: {message}")
            }
            Self::Unsupported => {
                formatter.write_str("device information is unsupported on this platform")
            }
        }
    }
}

impl std::error::Error for DeviceInfoError {}

/// A platform's answer, and whether it may be remembered.
///
/// An answer that is missing something only because of *where* the query ran —
/// iOS can name the device only from the main thread — must not become the
/// process-wide answer, or one early background call would blank the field
/// forever.
pub(crate) struct Queried {
    pub(crate) info: DeviceInfo,
    pub(crate) complete: bool,
}

static CACHE: OnceLock<Arc<DeviceInfo>> = OnceLock::new();

/// Describes the device this process runs on.
///
/// The first successful call queries the platform; every later call returns the
/// same [`Arc`]. A failed call is not remembered, so it can be retried.
///
/// On iOS, [`DeviceInfo::name`] and [`IosDetails::identifier_for_vendor`] come
/// from UIKit, which only answers on the main thread. Called from another
/// thread, this returns them as `None` and does not cache, so a later call from
/// the main thread still fills them in.
///
/// # Errors
///
/// Returns [`DeviceInfoError::Unavailable`] when the platform query failed, and
/// [`DeviceInfoError::Unsupported`] on a target with no query at all.
///
/// # Examples
///
/// ```
/// use aimer_device::device_info;
///
/// if let Ok(info) = device_info() {
///     assert_eq!(info.arch, std::env::consts::ARCH);
/// }
/// ```
pub fn device_info() -> Result<Arc<DeviceInfo>, DeviceInfoError> {
    cached(&CACHE, platform::query)
}

/// Returns `cache`'s answer, or runs `query` and remembers its result only if
/// it succeeded and is complete.
fn cached(
    cache: &OnceLock<Arc<DeviceInfo>>,
    query: impl FnOnce() -> Result<Queried, DeviceInfoError>,
) -> Result<Arc<DeviceInfo>, DeviceInfoError> {
    if let Some(cached) = cache.get() {
        return Ok(Arc::clone(cached));
    }
    let Queried { info, complete } = query()?;
    let info = Arc::new(info);
    if complete {
        // Another thread may have won the race; its answer is equivalent, and
        // handing every caller the one stored `Arc` keeps them pointer-equal.
        return Ok(Arc::clone(cache.get_or_init(|| info)));
    }
    Ok(info)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(name: &str) -> Queried {
        Queried {
            info: DeviceInfo {
                platform: Platform::CURRENT,
                kind: DeviceKind::Unknown,
                os_name: name.into(),
                os_version: OsVersion::default(),
                os_build: None,
                model: None,
                manufacturer: None,
                name: None,
                arch: std::env::consts::ARCH,
                is_physical: true,
                details: PlatformDetails::Linux(LinuxDetails::default()),
            },
            complete: true,
        }
    }

    #[test]
    fn a_complete_answer_is_queried_once_and_shared() {
        let cache = OnceLock::new();
        let first = cached(&cache, || Ok(sample("first"))).unwrap();
        let second = cached(&cache, || panic!("a cached answer must not be queried again")).unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(second.os_name, "first");
    }

    #[test]
    fn an_incomplete_answer_is_returned_but_not_remembered() {
        let cache = OnceLock::new();
        let partial = cached(&cache, || {
            Ok(Queried {
                complete: false,
                ..sample("partial")
            })
        })
        .unwrap();
        assert_eq!(partial.os_name, "partial");
        assert!(cache.get().is_none());

        let full = cached(&cache, || Ok(sample("full"))).unwrap();
        assert_eq!(full.os_name, "full");
        assert!(cache.get().is_some());
    }

    #[test]
    fn a_failure_is_not_remembered() {
        let cache = OnceLock::new();
        let error = cached(&cache, || Err(DeviceInfoError::Unavailable("early".into())));
        assert_eq!(error.unwrap_err(), DeviceInfoError::Unavailable("early".into()));
        assert!(cached(&cache, || Ok(sample("retry"))).is_ok());
    }

    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
    #[test]
    fn the_host_describes_itself() {
        let info = device_info().expect("a desktop host can always be described");
        assert_eq!(info.platform, Platform::CURRENT);
        assert_eq!(info.arch, std::env::consts::ARCH);
        assert_eq!(info.kind, DeviceKind::Desktop);
        assert!(info.os_version.major > 0, "{:?}", info.os_version);
        assert!(!info.os_name.is_empty());
        assert!(Arc::ptr_eq(&info, &device_info().unwrap()));
        // The detail variant always agrees with the platform field.
        assert!(
            matches!(
                (&info.details, info.platform),
                (PlatformDetails::MacOs(_), Platform::MacOs)
                    | (PlatformDetails::Linux(_), Platform::Linux)
                    | (PlatformDetails::Windows(_), Platform::Windows)
            ),
            "{:?}",
            info.details
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_mac_reports_its_hardware() {
        let info = device_info().unwrap();
        assert_eq!(info.os_name, "macOS");
        assert_eq!(info.manufacturer.as_deref(), Some("Apple"));
        assert!(info.model.as_deref().is_some_and(|model| !model.is_empty()));
        let PlatformDetails::MacOs(mac) = &info.details else {
            panic!("expected macOS details, got {:?}", info.details);
        };
        assert!(mac.memory_bytes > 0);
        assert!(mac.active_cpus > 0);
    }
}
