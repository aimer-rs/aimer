//! What macOS and iOS share: `sysctl`, `NSProcessInfo`, and the host name.

use std::ffi::CStr;
use std::ptr;

use objc2_foundation::NSProcessInfo;

use crate::OsVersion;

/// Reads a string-valued `sysctl`, without its trailing NUL.
pub(crate) fn sysctl_string(name: &CStr) -> Option<String> {
    let mut len = 0usize;
    // SAFETY: a null output buffer asks `sysctlbyname` only for the size, and
    // `name` is a valid NUL-terminated string.
    if unsafe { libc::sysctlbyname(name.as_ptr(), ptr::null_mut(), &mut len, ptr::null_mut(), 0) }
        != 0
        || len == 0
    {
        return None;
    }
    let mut buffer = vec![0u8; len];
    // SAFETY: `buffer` is `len` writable bytes, which is what is passed.
    if unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            buffer.as_mut_ptr().cast(),
            &mut len,
            ptr::null_mut(),
            0,
        )
    } != 0
    {
        return None;
    }
    buffer.truncate(len);
    while buffer.last() == Some(&0) {
        buffer.pop();
    }
    let text = String::from_utf8(buffer).ok()?;
    (!text.is_empty()).then_some(text)
}

/// Reads an integer-valued `sysctl` whose size is exactly `size_of::<T>()`.
///
/// # Safety
///
/// Every bit pattern of `T` must be a valid `T`, which holds for the plain
/// integers the two wrappers below instantiate it with.
#[cfg(target_os = "macos")]
unsafe fn sysctl_integer<T: Copy>(name: &CStr) -> Option<T> {
    use std::mem::{MaybeUninit, size_of};

    let mut value = MaybeUninit::<T>::zeroed();
    let mut len = size_of::<T>();
    // SAFETY: `value` is `len` writable bytes, and `name` is NUL-terminated.
    let status = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            value.as_mut_ptr().cast(),
            &mut len,
            ptr::null_mut(),
            0,
        )
    };
    // SAFETY: the buffer was zeroed and, on success, filled with `len` bytes of
    // the value; the caller guarantees any bit pattern is a valid `T`.
    (status == 0 && len == size_of::<T>()).then(|| unsafe { value.assume_init() })
}

#[cfg(target_os = "macos")]
pub(crate) fn sysctl_u32(name: &CStr) -> Option<u32> {
    // SAFETY: `u32` accepts every bit pattern.
    unsafe { sysctl_integer(name) }
}

#[cfg(target_os = "macos")]
pub(crate) fn sysctl_u64(name: &CStr) -> Option<u64> {
    // SAFETY: `u64` accepts every bit pattern.
    unsafe { sysctl_integer(name) }
}

/// The running operating system's version, from `NSProcessInfo`, which reports
/// the real version even to an app built against an older SDK.
pub(crate) fn os_version() -> OsVersion {
    let version = NSProcessInfo::processInfo().operatingSystemVersion();
    let component = |value: isize| u32::try_from(value).unwrap_or(0);
    OsVersion::new(
        component(version.majorVersion),
        component(version.minorVersion),
        component(version.patchVersion),
    )
}

/// The BSD host name, which is the only name available without an entitlement
/// or a main-thread UIKit call.
#[cfg(target_os = "macos")]
pub(crate) fn host_name() -> Option<String> {
    let mut buffer = [0u8; 256];
    // SAFETY: `buffer` is writable for the length passed.
    if unsafe { libc::gethostname(buffer.as_mut_ptr().cast(), buffer.len()) } != 0 {
        return None;
    }
    let end = buffer.iter().position(|&byte| byte == 0)?;
    let name = std::str::from_utf8(&buffer[..end]).ok()?;
    (!name.is_empty()).then(|| name.to_owned())
}
