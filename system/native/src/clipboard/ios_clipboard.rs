//! iOS, through UIKit's `UIPasteboard`.
//!
//! `UIPasteboard.general` is the system pasteboard shared with every other
//! application, which is the one a user means by "copy". Writing never fails —
//! UIKit reports no error — while reading finds no string whenever the
//! pasteboard is empty or holds something that is not text.

use objc2_foundation::NSString;
use objc2_ui_kit::UIPasteboard;

use super::ClipboardError;

pub(super) fn set_text(text: &str) -> Result<(), ClipboardError> {
    let pasteboard = UIPasteboard::generalPasteboard();
    // SAFETY: `setString:` takes an optional `NSString` and copies it; the
    // temporary string is retained by the pasteboard for as long as it needs.
    unsafe { pasteboard.setString(Some(&NSString::from_str(text))) };
    Ok(())
}

pub(super) fn get_text() -> Result<String, ClipboardError> {
    let pasteboard = UIPasteboard::generalPasteboard();
    // SAFETY: `string` returns an owned, autorelease-safe `NSString` or `nil`.
    let text = unsafe { pasteboard.string() };
    text.map(|text| text.to_string())
        .ok_or(ClipboardError::Unsupported)
}
