//! What the AppKit and UIKit accessibility bridges share: Core Graphics value
//! types that `objc` can pass by value, and `NSString` construction.

// The `objc` 0.2 macros test for a `cargo-clippy` cfg this workspace never sets.
#![allow(unexpected_cfgs)]

use objc::rc::StrongPtr;
use objc::runtime::Object;
use objc::{Encode, Encoding, class, msg_send, sel, sel_impl};

/// `NSUTF8StringEncoding`.
const UTF8: usize = 4;

/// `CGPoint`, 64-bit.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub(super) struct Point {
    pub x: f64,
    pub y: f64,
}

/// `CGSize`, 64-bit.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub(super) struct Size {
    pub width: f64,
    pub height: f64,
}

/// `CGRect`, 64-bit.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub(super) struct Rect {
    pub origin: Point,
    pub size: Size,
}

// SAFETY: each encoding matches the `repr(C)` declaration above — structs of
// `double`s with the names Core Graphics gives them — which is what `objc`
// checks against the runtime's method signatures.
unsafe impl Encode for Point {
    fn encode() -> Encoding {
        unsafe { Encoding::from_str("{CGPoint=dd}") }
    }
}

unsafe impl Encode for Size {
    fn encode() -> Encoding {
        unsafe { Encoding::from_str("{CGSize=dd}") }
    }
}

unsafe impl Encode for Rect {
    fn encode() -> Encoding {
        unsafe { Encoding::from_str("{CGRect={CGPoint=dd}{CGSize=dd}}") }
    }
}

/// An owned `NSString` copy of `text`.
pub(super) fn ns_string(text: &str) -> StrongPtr {
    // SAFETY: `alloc`/`initWithBytes:length:encoding:` on `NSString` with a
    // valid UTF-8 buffer that outlives the call.
    unsafe {
        let object: *mut Object = msg_send![class!(NSString), alloc];
        let object: *mut Object = msg_send![
            object,
            initWithBytes: text.as_ptr()
            length: text.len()
            encoding: UTF8
        ];
        StrongPtr::new(object)
    }
}
