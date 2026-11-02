//! Windows, from `RtlGetVersion`, the registry and `GlobalMemoryStatusEx`.
//!
//! `GetVersionEx` lies to any process without a compatibility manifest and
//! reports Windows 8.1, so the version comes from `RtlGetVersion`, which does
//! not. Model and manufacturer are the strings the firmware registered in
//! SMBIOS, which Windows mirrors under `HARDWARE\DESCRIPTION\System\BIOS`.

use std::mem::{size_of, zeroed};
use std::ptr;

use windows_sys::Wdk::System::SystemServices::RtlGetVersion;
use windows_sys::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW};
use windows_sys::Win32::System::SystemInformation::{
    ComputerNamePhysicalDnsHostname, GetComputerNameExW, GlobalMemoryStatusEx, MEMORYSTATUSEX,
    OSVERSIONINFOW,
};

use crate::virtualization::is_virtual_machine;
use crate::{
    DeviceInfo, DeviceInfoError, DeviceKind, OsVersion, Platform, PlatformDetails, Queried,
    WindowsDetails,
};

const BIOS_KEY: &str = r"HARDWARE\DESCRIPTION\System\BIOS";
const VERSION_KEY: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";

pub(super) fn query() -> Result<Queried, DeviceInfoError> {
    // SAFETY: an all-zero `OSVERSIONINFOW` is valid, and `RtlGetVersion` needs
    // only `dwOSVersionInfoSize` to be set before it fills the rest.
    let mut version: OSVERSIONINFOW = unsafe { zeroed() };
    version.dwOSVersionInfoSize = size_of::<OSVERSIONINFOW>() as u32;
    // SAFETY: `version` is a valid, correctly sized out-parameter.
    if unsafe { RtlGetVersion(&mut version) } != 0 {
        return Err(DeviceInfoError::Unavailable("RtlGetVersion failed".into()));
    }

    let manufacturer = registry_string(BIOS_KEY, "SystemManufacturer");
    let model = registry_string(BIOS_KEY, "SystemProductName");
    let is_physical = !is_virtual_machine(
        manufacturer.as_deref().unwrap_or_default(),
        model.as_deref().unwrap_or_default(),
    );

    let info = DeviceInfo {
        platform: Platform::Windows,
        kind: DeviceKind::Desktop,
        os_name: "Windows".into(),
        os_version: OsVersion::new(version.dwMajorVersion, version.dwMinorVersion, version.dwBuildNumber),
        os_build: Some(version.dwBuildNumber.to_string()),
        model,
        manufacturer,
        name: computer_name(),
        arch: std::env::consts::ARCH,
        is_physical,
        details: PlatformDetails::Windows(WindowsDetails {
            build_number: version.dwBuildNumber,
            product_name: registry_string(VERSION_KEY, "ProductName"),
            display_version: registry_string(VERSION_KEY, "DisplayVersion"),
            memory_bytes: physical_memory(),
        }),
    };
    Ok(Queried {
        info,
        complete: true,
    })
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

/// Reads a `REG_SZ` value under `HKEY_LOCAL_MACHINE`.
fn registry_string(subkey: &str, value: &str) -> Option<String> {
    let (subkey, value) = (wide(subkey), wide(value));
    let mut bytes = 0u32;
    // SAFETY: both names are NUL-terminated; a null data pointer asks only for
    // the size.
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            subkey.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_SZ,
            ptr::null_mut(),
            ptr::null_mut(),
            &mut bytes,
        )
    };
    if status != 0 || bytes < 2 {
        return None;
    }
    let mut buffer = vec![0u16; bytes as usize / 2 + 1];
    let mut bytes = (buffer.len() * 2) as u32;
    // SAFETY: `buffer` holds `bytes` writable bytes; `RRF_RT_REG_SZ` guarantees
    // the result is NUL-terminated.
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            subkey.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_SZ,
            ptr::null_mut(),
            buffer.as_mut_ptr().cast(),
            &mut bytes,
        )
    };
    if status != 0 {
        return None;
    }
    // `bytes` counts the terminator.
    let chars = (bytes as usize / 2).saturating_sub(1).min(buffer.len());
    let text = String::from_utf16_lossy(&buffer[..chars]);
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

fn computer_name() -> Option<String> {
    let mut buffer = [0u16; 256];
    let mut len = buffer.len() as u32;
    // SAFETY: `buffer` is `len` writable UTF-16 units.
    if unsafe { GetComputerNameExW(ComputerNamePhysicalDnsHostname, buffer.as_mut_ptr(), &mut len) } == 0 {
        return None;
    }
    // On success `len` is the length without the terminator.
    let name = String::from_utf16_lossy(&buffer[..(len as usize).min(buffer.len())]);
    (!name.is_empty()).then_some(name)
}

fn physical_memory() -> u64 {
    // SAFETY: an all-zero `MEMORYSTATUSEX` is valid; `dwLength` is set as the
    // API requires before it is filled.
    let mut status: MEMORYSTATUSEX = unsafe { zeroed() };
    status.dwLength = size_of::<MEMORYSTATUSEX>() as u32;
    // SAFETY: `status` is a valid, correctly sized out-parameter.
    if unsafe { GlobalMemoryStatusEx(&mut status) } == 0 {
        return 0;
    }
    status.ullTotalPhys
}
