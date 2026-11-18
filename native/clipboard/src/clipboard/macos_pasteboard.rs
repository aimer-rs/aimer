//! The macOS pasteboard read and written without a `String` in between.
//!
//! `arboard` reads text as an `NSString` and copies it into a `String`, and writes it by
//! copying a `String` into an `NSString`. For a text of megabytes that is two copies more than
//! the system needs. Here the pasteboard's own bytes are used where they are: a read keeps the
//! `NSData` the pasteboard delivered and lends it out as a `str` ([`PasteboardText`]), and a
//! write appends the pieces of a text to one `NSMutableData` that the pasteboard takes.
//!
//! Both are an addition to `arboard`, not a replacement: when the pasteboard has no UTF-8 text in
//! the form these read (a clipboard that holds only rich text, say), they say so and the caller
//! falls back to `arboard`, which converts.

use std::ptr::NonNull;
use std::sync::Arc;

use objc2::rc::{Retained, autoreleasepool};
use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString};
use objc2_foundation::{NSCopying, NSData, NSMutableData};

use super::ClipboardText;

/// UTF-8 text that is the bytes of an `NSData`, which it keeps alive.
struct PasteboardText {
    /// Held so the bytes stay where they are; never read through again.
    _data: Retained<NSData>,
    bytes: NonNull<u8>,
    len: usize,
}

// SAFETY: the data is an immutable `NSData` (a copy of what the pasteboard returned: copying an
// immutable one is the same object, and a mutable one becomes immutable), which Foundation
// documents as safe to read from any thread. The text is never written to.
unsafe impl Send for PasteboardText {}
unsafe impl Sync for PasteboardText {}

impl PasteboardText {
    /// `data` as text, or `None` when it is not valid UTF-8.
    fn new(data: &NSData) -> Option<Self> {
        // An immutable copy: whatever the pasteboard handed out, nobody can change these bytes.
        let data = data.copy();
        // SAFETY: the data is immutable and is kept alive by `self`, so the bytes it points to are
        // neither changed nor freed. Asking for them once fixes the pointer, which for a data that
        // is made of several regions is where they are gathered.
        let bytes = unsafe { data.as_bytes_unchecked() };
        std::str::from_utf8(bytes).ok()?;
        let (pointer, len) = (NonNull::new(bytes.as_ptr().cast_mut()).unwrap_or(NonNull::dangling()), bytes.len());
        Some(Self { _data: data, bytes: pointer, len })
    }
}

impl AsRef<str> for PasteboardText {
    #[inline]
    fn as_ref(&self) -> &str {
        // SAFETY: `new` checked that the bytes are UTF-8, and they live as long as `self` does.
        unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(self.bytes.as_ptr(), self.len)) }
    }
}

/// The first text on `pasteboard`, as the bytes it was delivered in. `None` when there is none in
/// UTF-8, which is when the caller asks `arboard`.
pub(super) fn read_from(pasteboard: &NSPasteboard) -> Option<ClipboardText> {
    autoreleasepool(|_| {
        // SAFETY: plain message sends to a live pasteboard; the types are those the bindings declare.
        let items = unsafe { pasteboard.pasteboardItems() }?;
        for item in items {
            if let Some(data) = unsafe { item.dataForType(NSPasteboardTypeString) } {
                return PasteboardText::new(&data).map(|text| Arc::new(text) as ClipboardText);
            }
        }
        None
    })
}

/// Replaces the contents of `pasteboard` with the text that is `pieces` one after the other,
/// copying each once, into the one buffer the pasteboard takes. `false` when it refused.
pub(super) fn write_to(pasteboard: &NSPasteboard, pieces: &[&str]) -> bool {
    autoreleasepool(|_| {
        let length: usize = pieces.iter().map(|piece| piece.len()).sum();
        let data = NSMutableData::dataWithCapacity(length).unwrap_or_else(NSMutableData::new);
        for piece in pieces {
            if let Some(bytes) = NonNull::new(piece.as_ptr().cast_mut().cast()) {
                // SAFETY: `bytes` points to `piece.len()` readable bytes, which the data copies.
                unsafe { data.appendBytes_length(bytes, piece.len()) };
            }
        }
        // SAFETY: plain message sends to a live pasteboard with a type the bindings declare.
        unsafe {
            pasteboard.clearContents();
            pasteboard.setData_forType(Some(&data), NSPasteboardTypeString)
        }
    })
}

/// The general pasteboard.
fn general() -> Option<Retained<NSPasteboard>> {
    // SAFETY: a class method with no arguments.
    Some(unsafe { NSPasteboard::generalPasteboard() })
}

pub(super) fn read() -> Option<ClipboardText> {
    let pasteboard = general()?;
    read_from(&pasteboard)
}

pub(super) fn write(pieces: &[&str]) -> bool {
    general().is_some_and(|pasteboard| write_to(&pasteboard, pieces))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pasteboard of its own, so a test neither reads nor disturbs the user's clipboard.
    fn private_pasteboard() -> Retained<NSPasteboard> {
        // SAFETY: a class method with no arguments.
        unsafe { NSPasteboard::pasteboardWithUniqueName() }
    }

    #[test]
    fn what_is_written_in_pieces_reads_back_as_one_text() {
        let pasteboard = private_pasteboard();
        let big = "caf\u{e9} \u{4f60}\u{597d} line\n".repeat(200_000);
        for pieces in [vec![], vec![""], vec!["hello"], vec!["a", "b\u{e9}", "\u{1f468}\u{200d}\u{1f469}", "\n"], vec![big.as_str()]] {
            assert!(write_to(&pasteboard, &pieces), "{} pieces", pieces.len());
            let read = read_from(&pasteboard);
            let expected: String = pieces.concat();
            match read {
                Some(text) => assert_eq!(<dyn AsRef<str>>::as_ref(&*text), expected),
                // An empty text has no item to read: the caller asks `arboard`, which says so.
                None => assert!(expected.is_empty()),
            }
        }
    }

    #[test]
    fn the_text_read_outlives_the_pasteboard_it_came_from() {
        let pasteboard = private_pasteboard();
        assert!(write_to(&pasteboard, &["kept ", "alive"]));
        let text = read_from(&pasteboard).expect("text was written");
        assert!(write_to(&pasteboard, &["something else entirely"]));
        drop(pasteboard);
        assert_eq!((*text).as_ref(), "kept alive");
        let on_another_thread = std::thread::spawn(move || (*text).as_ref().to_owned()).join().expect("the thread ends");
        assert_eq!(on_another_thread, "kept alive");
    }

    #[test]
    fn bytes_that_are_not_utf8_are_not_text() {
        let pasteboard = private_pasteboard();
        let data = NSMutableData::new();
        let bytes = [0x66_u8, 0xff, 0xfe, 0x67];
        // SAFETY: the pointer and length describe `bytes`.
        unsafe { data.appendBytes_length(NonNull::new(bytes.as_ptr().cast_mut().cast()).expect("not null"), bytes.len()) };
        // SAFETY: plain message sends to a live pasteboard.
        unsafe {
            pasteboard.clearContents();
            pasteboard.setData_forType(Some(&data), NSPasteboardTypeString);
        }
        assert!(read_from(&pasteboard).is_none());
    }
}
