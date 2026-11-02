//! Telling real hardware from emulators and virtual machines.
//!
//! These look only at the identifying strings a platform reports, so they are
//! compiled — and tested — on every host.

/// The `android.os.Build` fields that give an emulator away.
#[cfg(any(target_os = "android", test))]
pub(crate) struct AndroidBuild<'a> {
    pub(crate) fingerprint: &'a str,
    pub(crate) model: &'a str,
    pub(crate) manufacturer: &'a str,
    pub(crate) brand: &'a str,
    pub(crate) device: &'a str,
    pub(crate) product: &'a str,
    pub(crate) hardware: &'a str,
}

/// Whether `build` describes an Android emulator (the SDK's, Genymotion, or
/// the x86 images) rather than a phone or tablet.
#[cfg(any(target_os = "android", test))]
pub(crate) fn is_android_emulator(build: &AndroidBuild<'_>) -> bool {
    build.fingerprint.starts_with("generic")
        || build.fingerprint.starts_with("unknown")
        || build.model.contains("google_sdk")
        || build.model.contains("sdk_gphone")
        || build.model.contains("droid4x")
        || build.model.contains("Emulator")
        || build.model.contains("Android SDK built for")
        || build.manufacturer.contains("Genymotion")
        || (build.brand.starts_with("generic") && build.device.starts_with("generic"))
        || build.product == "google_sdk"
        || build.product.contains("sdk_gphone")
        // The emulator's own kernels: `goldfish` is the legacy one, `ranchu`
        // the current one.
        || matches!(build.hardware, "goldfish" | "ranchu")
}

/// Whether SMBIOS `vendor` and `product` strings, as a PC reports them,
/// belong to a virtual machine.
#[cfg(any(target_os = "windows", test))]
pub(crate) fn is_virtual_machine(vendor: &str, product: &str) -> bool {
    const VENDORS: [&str; 6] = ["vmware", "qemu", "innotek", "xen", "parallels", "bochs"];
    let vendor = vendor.to_ascii_lowercase();
    // Hyper-V presents itself as Microsoft hardware; only the product tells.
    VENDORS.iter().any(|known| vendor.contains(known))
        || (vendor.starts_with("microsoft") && product == "Virtual Machine")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build<'a>() -> AndroidBuild<'a> {
        AndroidBuild {
            fingerprint: "google/shiba/shiba:14/UQ1A.240205.002/11224170:user/release-keys",
            model: "Pixel 8",
            manufacturer: "Google",
            brand: "google",
            device: "shiba",
            product: "shiba",
            hardware: "shiba",
        }
    }

    #[test]
    fn a_retail_phone_is_not_an_emulator() {
        assert!(!is_android_emulator(&build()));
    }

    #[test]
    fn recognises_the_sdk_emulator_images() {
        let modern = AndroidBuild {
            fingerprint: "google/sdk_gphone64_arm64/emu64a:14/UE1A/1:user/release-keys",
            model: "sdk_gphone64_arm64",
            product: "sdk_gphone64_arm64",
            hardware: "ranchu",
            ..build()
        };
        assert!(is_android_emulator(&modern));

        let legacy = AndroidBuild {
            fingerprint: "generic/sdk/generic:8.0/x:eng/test-keys",
            model: "Android SDK built for x86",
            brand: "generic",
            device: "generic_x86",
            ..build()
        };
        assert!(is_android_emulator(&legacy));
    }

    #[test]
    fn recognises_third_party_emulators() {
        let genymotion = AndroidBuild {
            manufacturer: "Genymotion",
            ..build()
        };
        assert!(is_android_emulator(&genymotion));
        let goldfish = AndroidBuild {
            hardware: "goldfish",
            ..build()
        };
        assert!(is_android_emulator(&goldfish));
    }

    #[test]
    fn recognises_common_hypervisor_vendors() {
        assert!(is_virtual_machine("VMware, Inc.", "VMware7,1"));
        assert!(is_virtual_machine("QEMU", "Standard PC (Q35 + ICH9, 2009)"));
        assert!(is_virtual_machine("innotek GmbH", "VirtualBox"));
        assert!(is_virtual_machine("Microsoft Corporation", "Virtual Machine"));
        assert!(is_virtual_machine("Parallels Software International Inc.", "Parallels"));
    }

    #[test]
    fn real_pc_vendors_are_not_virtual() {
        assert!(!is_virtual_machine("Dell Inc.", "XPS 15 9530"));
        assert!(!is_virtual_machine("Microsoft Corporation", "Surface Laptop 5"));
        assert!(!is_virtual_machine("LENOVO", "21HD"));
        assert!(!is_virtual_machine("", ""));
    }
}
