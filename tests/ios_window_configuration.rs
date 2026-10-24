//! UIKit chooses continuous or discrete resizing from the app bundle, before
//! the Rust window and renderer exist. Keep the example's bundle resizable.

#[test]
fn ios_bundle_distinguishes_mouse_clicks_from_finger_touches_on_older_ios() {
    let info = include_str!("../app/jaime/builds/ios/Jaime/Info.plist");
    let value = info
        .split_once("<key>UIApplicationSupportsIndirectInputEvents</key>")
        .expect("iOS 14–16 require an explicit opt-in for indirect pointer touches")
        .1;
    assert!(
        value.trim_start().starts_with("<true/>"),
        "the native mouse bridge needs UITouchTypeIndirectPointer"
    );
}

#[test]
fn ios_bundle_allows_continuous_ipad_resizing() {
    let info = include_str!("../app/jaime/builds/ios/Jaime/Info.plist");
    let project = include_str!("../app/jaime/builds/ios/Jaime.xcodeproj/project.pbxproj");
    assert!(
        !info.contains("<key>UIRequiresFullScreen</key>"),
        "UIRequiresFullScreen selects discrete resizing on iPadOS 27"
    );
    assert!(
        !project.contains("INFOPLIST_KEY_UIRequiresFullScreen"),
        "Xcode must not add full-screen compatibility back to the bundle"
    );
    assert!(info.contains("<key>UIApplicationSceneManifest</key>"));
    assert!(info.contains("<key>UILaunchScreen</key>"));
    let ipad = info
        .split_once("<key>UISupportedInterfaceOrientations~ipad</key>")
        .expect("iPad requires all four orientations independently of iPhone")
        .1
        .split_once("</array>")
        .expect("iPad orientation array")
        .0;
    for orientation in [
        "UIInterfaceOrientationPortrait",
        "UIInterfaceOrientationPortraitUpsideDown",
        "UIInterfaceOrientationLandscapeLeft",
        "UIInterfaceOrientationLandscapeRight",
    ] {
        assert!(
            ipad.contains(&format!("<string>{orientation}</string>")),
            "missing iPad orientation: {orientation}"
        );
    }
}
