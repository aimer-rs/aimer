pub enum Os {
    Windows,
    Linux,
    Android,
    MacOS,
    IOS(IOSVariant),
    Web,
}

pub enum IOSVariant {
    IPhone,
    IPad,
}

/// Representation of the current operating system.
///
/// Note: IOS has a different variant for iPhone and iPad.
/// It can get the exact IOS variant at runtime with [`IOSVariant::is_ipad`].
pub const OS: Os = current_platform();

#[inline(always)]
const fn current_platform() -> Os {
    #[cfg(target_os = "windows")]
    {
        Os::Windows
    }
    #[cfg(target_os = "ios")]
    {
        Os::IOS(IOSVariant::IPhone)
    }
    #[cfg(target_os = "macos")]
    {
        Os::MacOS
    }
    #[cfg(target_os = "android")]
    {
        Os::Android
    }
    #[cfg(target_os = "linux")]
    {
        Os::Linux
    }
    #[cfg(target_arch = "wasm32")]
    {
        Os::Web
    }
}

impl Os {
    pub const fn is_ios_family(&self) -> bool {
        matches!(
            self,
            Os::IOS(IOSVariant::IPhone) | Os::IOS(IOSVariant::IPad)
        )
    }
}

impl IOSVariant {
    #[cfg(target_os = "ios")]
    fn is_ipad() -> bool {
        use objc2::MainThreadMarker;
        use objc2::rc::Retained;
        use objc2_ui_kit::{UIDevice, UIUserInterfaceIdiom};

        let mtm = MainThreadMarker::new().expect("must be called on the main thread");

        let device: Retained<UIDevice> = unsafe { UIDevice::currentDevice(mtm) };
        let idiom: UIUserInterfaceIdiom = unsafe { device.userInterfaceIdiom() };
        idiom == UIUserInterfaceIdiom::Pad
    }
}
