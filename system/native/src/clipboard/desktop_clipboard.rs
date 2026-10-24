//! macOS, Windows and Linux, through `arboard`.
//!
//! The handle is opened once per calling thread and kept for that thread's
//! lifetime. That is not only cheaper than opening one per copy — on X11 the
//! clipboard's contents are *owned by the process that put them there*, so a
//! handle dropped right after the write takes the copied text with it. A
//! thread-local owner keeps the platform handle out of a process-wide lock.

use std::cell::RefCell;

use super::ClipboardError;

thread_local! {
    static CLIPBOARD: RefCell<Option<arboard::Clipboard>> = const { RefCell::new(None) };
}

pub(super) fn set_text(text: &str) -> Result<(), ClipboardError> {
    with_clipboard(|clipboard| clipboard.set_text(text).map_err(unavailable))
}

pub(super) fn get_text() -> Result<String, ClipboardError> {
    with_clipboard(|clipboard| clipboard.get_text().map_err(unavailable))
}

/// Runs `call` against the calling thread's persistent clipboard handle.
fn with_clipboard<T>(
    call: impl FnOnce(&mut arboard::Clipboard) -> Result<T, ClipboardError>,
) -> Result<T, ClipboardError> {
    CLIPBOARD.with(|slot| {
        let mut slot = slot.try_borrow_mut().map_err(|_| {
            ClipboardError::Unavailable("the clipboard is already in use on this thread".into())
        })?;
        if slot.is_none() {
            *slot = arboard::Clipboard::new().ok();
        }
        let clipboard = slot.as_mut().ok_or_else(|| {
            ClipboardError::Unavailable("the platform clipboard could not be opened".into())
        })?;
        call(clipboard)
    })
}

fn unavailable(error: arboard::Error) -> ClipboardError {
    ClipboardError::Unavailable(error.to_string())
}
