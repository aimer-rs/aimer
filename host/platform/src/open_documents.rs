//! Files and links the user opens the application with, on macOS.
//!
//! Dropping a file on the Dock icon, choosing the application in "Open With", and
//! double-clicking a file the application is the handler of all end the same way:
//! AppKit sends the application's delegate `application:openURLs:`, which may be
//! before the application has finished launching. winit's delegate has no such
//! method, so [`install`] adds one that hands each file to [`crate::file_inbox`]
//! and each other URL — a custom scheme the application registered — to
//! [`crate::link_inbox`]. Both queue until the event loop drains them, which is
//! what makes the file that *launched* the application arrive too.
//!
//! The system only offers an application files it declares it can open, so the
//! application's bundle must name its document types (`CFBundleDocumentTypes`);
//! `aimer` writes them from the project's configuration. Other platforms report
//! files in other ways, and this does nothing there.

/// Teaches the native application to report the files and links it was opened
/// with.
///
/// Call it once on the main thread as soon as the native application exists,
/// **before the event loop runs**: a file that launches the application is
/// delivered while it is still launching, and an application that does not answer
/// yet is told it cannot open the file. Calling it again, or on another platform,
/// does nothing; a delegate that already answers the message is left alone.
#[cfg(target_os = "macos")]
pub fn install() {
    use std::ffi::{CStr, c_char};
    use std::path::PathBuf;

    use objc::runtime::{Object, Sel, class_addMethod};
    use objc::{class, msg_send, sel, sel_impl};

    type Id = *mut Object;

    /// The text of an `NSString`, or `None` for `nil` and invalid UTF-8.
    ///
    /// # Safety
    /// `string` is `nil` or a live `NSString`.
    unsafe fn text(string: Id) -> Option<String> {
        if string.is_null() {
            return None;
        }
        let utf8: *const c_char = msg_send![string, UTF8String];
        if utf8.is_null() {
            return None;
        }
        // SAFETY: `UTF8String` is NUL-terminated and lives as long as `string`.
        unsafe { CStr::from_ptr(utf8) }.to_str().ok().map(str::to_owned)
    }

    extern "C" fn open_urls(_this: &Object, _selector: Sel, _app: Id, urls: Id) {
        if urls.is_null() {
            return;
        }
        // SAFETY: AppKit calls this on the main thread with a live `NSArray` of
        // `NSURL`; every object is only read for the duration of the call.
        unsafe {
            let count: usize = msg_send![urls, count];
            let mut files = Vec::new();
            for index in 0..count {
                let url: Id = msg_send![urls, objectAtIndex: index];
                let is_file: objc::runtime::BOOL = msg_send![url, isFileURL];
                if is_file != objc::runtime::NO {
                    // `path`, not the URL's text: a file may be named by a
                    // reference URL that only the system can resolve.
                    let path: Id = msg_send![url, path];
                    files.extend(text(path).map(PathBuf::from));
                } else {
                    let absolute: Id = msg_send![url, absoluteString];
                    if let Some(link) = text(absolute) {
                        crate::link_inbox::post_link(link);
                    }
                }
            }
            crate::file_inbox::post_files(files);
        }
    }

    // SAFETY: called on the main thread, where `NSApplication` may be used.
    // `class_addMethod` is given a function whose signature matches the message —
    // object, selector, object, object, returning nothing — and a NUL-terminated
    // type encoding. Adding to a class that already answers the selector fails and
    // changes nothing.
    unsafe {
        let app: Id = msg_send![class!(NSApplication), sharedApplication];
        let delegate: Id = msg_send![app, delegate];
        if delegate.is_null() {
            return;
        }
        let class = (*delegate).class() as *const _ as *mut objc::runtime::Class;
        let implementation: objc::runtime::Imp =
            std::mem::transmute(open_urls as extern "C" fn(&Object, Sel, Id, Id));
        let added = class_addMethod(class, sel!(application:openURLs:), implementation, c"v@:@@".as_ptr());
        if added != objc::runtime::NO {
            // AppKit may have looked at what its delegate answers when it was
            // set, so it is set again to be asked afresh.
            let _: () = msg_send![app, setDelegate: delegate];
        }
    }
}

/// Makes the native application report the files it was opened with. This
/// platform reports them another way.
#[cfg(not(target_os = "macos"))]
pub fn install() {}
