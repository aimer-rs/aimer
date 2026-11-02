//! What a browser's `User-Agent` string reveals about the device.
//!
//! Browsers freeze and reduce these strings on purpose — Chrome reports
//! `Windows NT 10.0` on Windows 11 and `Android 10; K` for every Android model
//! — so the result is a best effort and every field says when it knows nothing.

use crate::{DeviceKind, OsVersion};

/// The device facts a user agent states.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct UserAgentInfo {
    pub(crate) os_name: &'static str,
    pub(crate) os_version: OsVersion,
    pub(crate) kind: DeviceKind,
    pub(crate) model: Option<String>,
}

/// Reads the operating system, form factor and (on Android) model out of
/// `user_agent`.
pub(crate) fn parse(user_agent: &str) -> UserAgentInfo {
    let after = |marker: &str| {
        user_agent
            .find(marker)
            .map(|at| &user_agent[at + marker.len()..])
    };
    let known = |os_name, os_version, kind| UserAgentInfo {
        os_name,
        os_version,
        kind,
        model: None,
    };

    // Order matters: an Android agent also says "Linux", and an iPhone's says
    // "like Mac OS X".
    if let Some(rest) = after("Android ") {
        let kind = if user_agent.contains("Mobile") {
            DeviceKind::Phone
        } else {
            DeviceKind::Tablet
        };
        return UserAgentInfo {
            model: android_model(rest),
            ..known("Android", version_at(rest), kind)
        };
    }
    if user_agent.contains("iPhone") || user_agent.contains("iPod") {
        let version = after("OS ").map_or_else(OsVersion::default, version_at);
        return known("iOS", version, DeviceKind::Phone);
    }
    if user_agent.contains("iPad") {
        let version = after("OS ").map_or_else(OsVersion::default, version_at);
        return known("iPadOS", version, DeviceKind::Tablet);
    }
    if let Some(rest) = after("Windows NT ") {
        return known("Windows", version_at(rest), DeviceKind::Desktop);
    }
    if let Some(rest) = after("Mac OS X ") {
        return known("macOS", version_at(rest), DeviceKind::Desktop);
    }
    if let Some(rest) = after("CrOS ") {
        // `CrOS x86_64 14541.0.0`: the architecture comes first.
        let version = rest.split_once(' ').map_or_else(OsVersion::default, |(_, v)| version_at(v));
        return known("ChromeOS", version, DeviceKind::Desktop);
    }
    if user_agent.contains("Linux") || user_agent.contains("X11") {
        return known("Linux", OsVersion::default(), DeviceKind::Desktop);
    }
    known("Unknown", OsVersion::default(), DeviceKind::Unknown)
}

/// Reads a version that starts `text`, accepting Apple's `17_5_1` spelling.
fn version_at(text: &str) -> OsVersion {
    let end = text
        .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '_'))
        .unwrap_or(text.len());
    OsVersion::parse(&text[..end].replace('_', "."))
}

/// The model in `Android 14; Pixel 8) ...`, given the text after `Android `.
///
/// Returns `None` for the placeholders browsers put there instead of a model:
/// Chrome's reduced `K`, and Firefox's `Mobile` / `Tablet`.
fn android_model(after_android: &str) -> Option<String> {
    let (_, rest) = after_android.split_once("; ")?;
    let end = rest.find([';', ')']).unwrap_or(rest.len());
    let model = rest[..end].split(" Build/").next()?.trim();
    (!matches!(model, "" | "K" | "Mobile" | "Tablet")).then(|| model.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHROME_WINDOWS: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36";
    const SAFARI_MAC: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Safari/605.1.15";
    const SAFARI_IPHONE: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 17_5_1 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Mobile/15E148 Safari/604.1";
    const SAFARI_IPAD: &str = "Mozilla/5.0 (iPad; CPU OS 16_6 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/16.6 Mobile/15E148 Safari/604.1";
    const CHROME_ANDROID_PHONE: &str = "Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Mobile Safari/537.36";
    const CHROME_ANDROID_TABLET: &str = "Mozilla/5.0 (Linux; Android 13; SM-X700) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36";
    const CHROME_ANDROID_REDUCED: &str = "Mozilla/5.0 (Linux; Android 10; K) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Mobile Safari/537.36";
    const FIREFOX_LINUX: &str = "Mozilla/5.0 (X11; Ubuntu; Linux x86_64; rv:127.0) Gecko/20100101 Firefox/127.0";
    const CHROME_OS: &str = "Mozilla/5.0 (X11; CrOS x86_64 14541.0.0) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36";

    #[test]
    fn desktop_systems() {
        let windows = parse(CHROME_WINDOWS);
        assert_eq!(
            (windows.os_name, windows.os_version, windows.kind),
            ("Windows", OsVersion::new(10, 0, 0), DeviceKind::Desktop)
        );
        let mac = parse(SAFARI_MAC);
        assert_eq!(
            (mac.os_name, mac.os_version, mac.kind),
            ("macOS", OsVersion::new(10, 15, 7), DeviceKind::Desktop)
        );
        let linux = parse(FIREFOX_LINUX);
        assert_eq!(
            (linux.os_name, linux.os_version, linux.kind),
            ("Linux", OsVersion::default(), DeviceKind::Desktop)
        );
        let chrome_os = parse(CHROME_OS);
        assert_eq!(
            (chrome_os.os_name, chrome_os.os_version.major),
            ("ChromeOS", 14541)
        );
    }

    #[test]
    fn apple_handhelds() {
        let iphone = parse(SAFARI_IPHONE);
        assert_eq!(
            (iphone.os_name, iphone.os_version, iphone.kind),
            ("iOS", OsVersion::new(17, 5, 1), DeviceKind::Phone)
        );
        let ipad = parse(SAFARI_IPAD);
        assert_eq!(
            (ipad.os_name, ipad.os_version, ipad.kind),
            ("iPadOS", OsVersion::new(16, 6, 0), DeviceKind::Tablet)
        );
    }

    #[test]
    fn android_phone_tablet_and_model() {
        let phone = parse(CHROME_ANDROID_PHONE);
        assert_eq!(
            (phone.os_name, phone.os_version, phone.kind),
            ("Android", OsVersion::new(14, 0, 0), DeviceKind::Phone)
        );
        assert_eq!(phone.model.as_deref(), Some("Pixel 8"));

        let tablet = parse(CHROME_ANDROID_TABLET);
        assert_eq!(tablet.kind, DeviceKind::Tablet);
        assert_eq!(tablet.model.as_deref(), Some("SM-X700"));
    }

    #[test]
    fn a_reduced_android_model_is_not_a_model() {
        assert_eq!(parse(CHROME_ANDROID_REDUCED).model, None);
    }

    #[test]
    fn an_unrecognised_agent_is_unknown() {
        let unknown = parse("curl/8.0");
        assert_eq!(unknown.os_name, "Unknown");
        assert_eq!(unknown.kind, DeviceKind::Unknown);
        assert_eq!(parse("").os_name, "Unknown");
    }
}
