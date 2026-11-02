//! The data a device query returns.

use crate::OsVersion;

/// The operating-system family the process is running on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Platform {
    /// Android.
    Android,
    /// iOS and iPadOS.
    Ios,
    /// Linux distributions other than Android.
    Linux,
    /// macOS.
    MacOs,
    /// A browser, whatever operating system hosts it.
    Web,
    /// Windows.
    Windows,
}

impl Platform {
    /// The family this binary was compiled for.
    pub const CURRENT: Self = {
        if cfg!(target_os = "android") {
            Self::Android
        } else if cfg!(target_os = "ios") {
            Self::Ios
        } else if cfg!(target_os = "macos") {
            Self::MacOs
        } else if cfg!(target_os = "windows") {
            Self::Windows
        } else if cfg!(target_arch = "wasm32") {
            Self::Web
        } else {
            Self::Linux
        }
    };
}

/// The form factor of the device, as far as the platform reveals it.
///
/// Platforms disagree about how much they tell: iOS and Android state it, a
/// desktop operating system is a [`Desktop`](Self::Desktop) by definition, and
/// a browser only offers a user-agent string to guess from. A guess that cannot
/// be made is [`Unknown`](Self::Unknown), never a wrong answer.
///
/// On Android the split is the smallest screen width: 600 dp and above is a
/// [`Tablet`](Self::Tablet), the same threshold as the `sw600dp` resource
/// qualifier, and the kind is [`Unknown`](Self::Unknown) until the application
/// context exists. On the web it is read from the user agent, so a desktop-mode
/// browser on a tablet can be reported as [`Desktop`](Self::Desktop).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeviceKind {
    /// A desktop or laptop computer.
    Desktop,
    /// A handheld phone.
    Phone,
    /// A tablet.
    Tablet,
    /// The platform did not say.
    Unknown,
}

/// What the process learned about the device it runs on.
///
/// The common fields are filled on every platform; a field the platform cannot
/// answer is `None` rather than an empty string, so "no model" and "a model
/// called nothing" stay distinct. Anything specific to one operating system
/// lives in [`details`](Self::details).
///
/// Obtain one with [`device_info`](crate::device_info).
///
/// # Examples
///
/// ```
/// use aimer_device::{DeviceKind, device_info};
///
/// if let Ok(info) = device_info() {
///     // Branch on a form factor without caring which platform reported it.
///     let compact = matches!(info.kind, DeviceKind::Phone);
///     println!("{} {} (compact: {compact})", info.os_name, info.os_version);
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct DeviceInfo {
    /// The operating-system family.
    pub platform: Platform,
    /// The form factor.
    pub kind: DeviceKind,
    /// The operating system's own name: `macOS`, `iOS`, `Android`, `Windows`,
    /// a Linux distribution such as `Ubuntu`, or the browser's reported
    /// operating system on the web.
    pub os_name: String,
    /// The operating-system version, parsed for comparison.
    ///
    /// On the web this is whatever the user agent claims, and browsers freeze
    /// it: Windows 11 reports `10.0`, macOS reports `10.15.7`, and iPadOS
    /// reports no version at all (`0.0.0`).
    pub os_version: OsVersion,
    /// The operating-system build identifier, such as macOS's `23F79` or a
    /// Windows build number.
    pub os_build: Option<String>,
    /// The hardware model: `MacBookPro18,3`, `iPhone15,2`, `Pixel 8`, the
    /// product name a PC vendor registered.
    ///
    /// On the web this is `None` except on Android, where the user agent may
    /// carry it; Chrome's reduced user agent hides it there too.
    pub model: Option<String>,
    /// The hardware vendor: `Apple`, `samsung`, `Dell Inc.`.
    pub manufacturer: Option<String>,
    /// The name the user gave the device, or its host name.
    ///
    /// iOS 16 and later report a generic name unless the app holds a special
    /// entitlement, and it is `None` when the first query ran off the main
    /// thread. Android reports `None` until the user names the device, and the
    /// web always reports `None`.
    pub name: Option<String>,
    /// The CPU architecture of this binary, as `std::env::consts::ARCH`.
    pub arch: &'static str,
    /// `false` on a simulator or an emulator, `true` on real hardware.
    ///
    /// This is a heuristic on Android, where it compares the build strings
    /// against known emulator images, and on Windows and Linux, where it looks
    /// for a virtual machine. On the web nothing in a page can tell, so it is
    /// always `true`.
    pub is_physical: bool,
    /// Fields only one operating system has.
    pub details: PlatformDetails,
}

/// The part of a [`DeviceInfo`] that differs by operating system.
///
/// Match on the variant for the platform you target; the variant always agrees
/// with [`DeviceInfo::platform`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PlatformDetails {
    /// Fields of an Android device.
    Android(AndroidDetails),
    /// Fields of an iPhone or iPad.
    Ios(IosDetails),
    /// Fields of a Linux machine.
    Linux(LinuxDetails),
    /// Fields of a Mac.
    MacOs(MacOsDetails),
    /// Fields of a browser.
    Web(WebDetails),
    /// Fields of a Windows PC.
    Windows(WindowsDetails),
}

impl PlatformDetails {
    /// Whether this is an Android device.
    #[inline]
    pub const fn is_android(&self) -> bool {
        matches!(self, Self::Android(_))
    }

    /// Whether this is an iPhone, an iPod touch or an iPad.
    ///
    /// This is the whole iOS family, so it is `true` on an iPad too; use
    /// [`is_ipados`](Self::is_ipados) to tell them apart.
    #[inline]
    pub const fn is_ios(&self) -> bool {
        matches!(self, Self::Ios(_))
    }

    /// Whether this is an iPad.
    ///
    /// Only a native iPadOS app reports this. A browser on an iPad is
    /// [`PlatformDetails::Web`], which says nothing about the device under it.
    #[inline]
    pub const fn is_ipados(&self) -> bool {
        matches!(self, Self::Ios(IosDetails { is_ipad: true, .. }))
    }

    /// Whether this is a Linux machine other than Android.
    #[inline]
    pub const fn is_linux(&self) -> bool {
        matches!(self, Self::Linux(_))
    }

    /// Whether this is a Mac.
    #[inline]
    pub const fn is_macos(&self) -> bool {
        matches!(self, Self::MacOs(_))
    }

    /// Whether this is a browser, whatever operating system hosts it.
    #[inline]
    pub const fn is_web(&self) -> bool {
        matches!(self, Self::Web(_))
    }

    /// Whether this is a Windows PC.
    #[inline]
    pub const fn is_windows(&self) -> bool {
        matches!(self, Self::Windows(_))
    }
}

/// Android-only fields, from `android.os.Build`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct AndroidDetails {
    /// `Build.VERSION.SDK_INT`, the API level.
    pub sdk_int: u32,
    /// `Build.BRAND`, the consumer-visible brand.
    pub brand: String,
    /// `Build.DEVICE`, the industrial design name.
    pub device: String,
    /// `Build.PRODUCT`, the product name.
    pub product: String,
    /// `Build.BOARD`, the board name.
    pub board: String,
    /// `Build.HARDWARE`, the hardware name.
    pub hardware: String,
    /// `Build.FINGERPRINT`, which uniquely identifies the build.
    pub fingerprint: String,
    /// `Build.VERSION.SECURITY_PATCH`, e.g. `2024-05-01`.
    pub security_patch: Option<String>,
    /// `Build.SUPPORTED_ABIS`, most preferred first.
    pub supported_abis: Vec<String>,
}

/// iOS-only fields.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct IosDetails {
    /// `true` on an iPad, which runs iPadOS rather than iOS proper.
    pub is_ipad: bool,
    /// `UIDevice.identifierForVendor`: stable across launches for this vendor's
    /// apps, reset when the last of them is uninstalled.
    ///
    /// UIKit only answers on the main thread, so this is `None` when the first
    /// query came from another thread.
    pub identifier_for_vendor: Option<String>,
}

/// Linux-only fields, from `os-release(5)` and `uname(2)`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct LinuxDetails {
    /// `ID`, the lowercase distribution identifier such as `ubuntu`.
    pub id: Option<String>,
    /// `PRETTY_NAME`, e.g. `Ubuntu 24.04.1 LTS`.
    pub pretty_name: Option<String>,
    /// The kernel release, e.g. `6.8.0-45-generic`.
    pub kernel_release: String,
}

/// macOS-only fields.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct MacOsDetails {
    /// The processor's marketing name, e.g. `Apple M2 Pro`.
    pub cpu_brand: Option<String>,
    /// The number of logical processors currently active.
    pub active_cpus: u32,
    /// Installed physical memory in bytes.
    pub memory_bytes: u64,
}

/// Browser-only fields, from `navigator`.
///
/// A browser tells a page very little on purpose. The operating system and form
/// factor in [`DeviceInfo`] are inferred from [`user_agent`](Self::user_agent),
/// so treat them as hints, not facts.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct WebDetails {
    /// `navigator.userAgent`.
    pub user_agent: String,
    /// `navigator.platform`, which browsers keep only for compatibility.
    pub platform: String,
    /// `navigator.language`, the preferred language tag.
    pub language: String,
    /// `navigator.hardwareConcurrency`, the logical processor count.
    pub hardware_concurrency: u32,
    /// `navigator.maxTouchPoints`; `0` on a machine without a touch screen.
    pub max_touch_points: u32,
}

/// Windows-only fields.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct WindowsDetails {
    /// The build number, e.g. `22631`.
    pub build_number: u32,
    /// The marketing product name from the registry, e.g. `Windows 10 Pro`.
    ///
    /// Windows 11 still writes `Windows 10` here; use the build number to tell
    /// them apart (22000 and above is Windows 11).
    pub product_name: Option<String>,
    /// The feature-update label, e.g. `23H2`.
    pub display_version: Option<String>,
    /// Installed physical memory in bytes.
    pub memory_bytes: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all() -> [PlatformDetails; 7] {
        [
            PlatformDetails::Android(AndroidDetails::default()),
            PlatformDetails::Ios(IosDetails::default()),
            PlatformDetails::Ios(IosDetails {
                is_ipad: true,
                ..IosDetails::default()
            }),
            PlatformDetails::Linux(LinuxDetails::default()),
            PlatformDetails::MacOs(MacOsDetails::default()),
            PlatformDetails::Web(WebDetails::default()),
            PlatformDetails::Windows(WindowsDetails::default()),
        ]
    }

    /// Which of `[android, ios, ipados, linux, macos, web, windows]` hold.
    fn flags(details: &PlatformDetails) -> [bool; 7] {
        [
            details.is_android(),
            details.is_ios(),
            details.is_ipados(),
            details.is_linux(),
            details.is_macos(),
            details.is_web(),
            details.is_windows(),
        ]
    }

    #[test]
    fn each_variant_matches_exactly_its_own_predicates() {
        let expected = [
            [true, false, false, false, false, false, false],
            [false, true, false, false, false, false, false],
            // An iPad is both iOS family and iPadOS.
            [false, true, true, false, false, false, false],
            [false, false, false, true, false, false, false],
            [false, false, false, false, true, false, false],
            [false, false, false, false, false, true, false],
            [false, false, false, false, false, false, true],
        ];
        for (details, expected) in all().iter().zip(expected) {
            assert_eq!(flags(details), expected, "{details:?}");
        }
    }

    #[test]
    fn predicates_are_usable_in_const_context() {
        const {
            assert!(
                PlatformDetails::MacOs(MacOsDetails {
                    cpu_brand: None,
                    active_cpus: 0,
                    memory_bytes: 0,
                })
                .is_macos()
            );
        }
    }
}
