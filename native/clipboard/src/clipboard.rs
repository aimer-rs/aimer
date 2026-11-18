//! The system clipboard, on every platform Aimer ships.
//!
//! One pair of calls — [`set_text`] and [`get_text`] — is backed by the
//! platform's own pasteboard: `arboard` on macOS, Windows and Linux,
//! `UIPasteboard` on iOS, `android.content.ClipboardManager` through JNI on
//! Android, and the asynchronous `navigator.clipboard` on the web.
//!
//! Availability: macOS, iOS, Android 7+, Windows, Linux
//!
//! Every call returns a [`Result`], because a clipboard is a shared system
//! resource that can legitimately be missing: a Linux session without a display
//! server, a browser that denies permission, an Android process whose activity
//! has gone away. Nothing here panics, and nothing here blocks the platform's
//! UI thread.
//!
//! # Examples
//!
//! ```
//! use aimer_clipboard::clipboard;
//!
//! // Both calls are fallible; a widget usually just ignores the failure.
//! let _ = clipboard::set_text("copied from Aimer");
//! ```
//!
//! # Web
//!
//! The browser clipboard is asynchronous and permission-gated. [`set_text`]
//! starts the write and returns immediately, which is what a pointer or key
//! handler needs. Where `navigator.clipboard` is missing altogether — Safari
//! outside a secure context, which is any page served over plain HTTP — the
//! write goes through `document.execCommand("copy")` instead, so a copy control
//! keeps working rather than throwing. [`get_text`] cannot block on a `Promise`
//! and therefore returns the text this process wrote last, or
//! [`ClipboardError::Unsupported`] when it has written none — see
//! [`get_text`]'s own documentation.

use std::fmt::{Display, Formatter};

#[cfg(target_os = "android")]
mod android_clipboard;
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
mod desktop_clipboard;
#[cfg(target_os = "ios")]
mod ios_clipboard;
#[cfg(target_os = "macos")]
mod macos_pasteboard;
#[cfg(target_arch = "wasm32")]
mod web_clipboard;

#[cfg(target_os = "android")]
use android_clipboard as platform;
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
use desktop_clipboard as platform;
#[cfg(target_os = "ios")]
use ios_clipboard as platform;
#[cfg(target_arch = "wasm32")]
use web_clipboard as platform;

/// Why a clipboard operation could not be carried out.
///
/// The distinction matters to a caller that wants to tell the user something:
/// [`Self::Unsupported`] is permanent and worth hiding a "Copy" control for,
/// while [`Self::Unavailable`] is a transient failure of an otherwise present
/// clipboard and worth retrying.
///
/// # Examples
///
/// ```
/// use aimer_clipboard::clipboard::ClipboardError;
///
/// let error = ClipboardError::Unavailable("no display server".into());
/// assert_eq!(
///     error.to_string(),
///     "clipboard is unavailable: no display server"
/// );
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClipboardError {
    /// The platform has a clipboard, but this operation failed on it.
    Unavailable(String),
    /// The platform offers no clipboard for this operation at all.
    Unsupported,
}

impl Display for ClipboardError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable(message) => write!(formatter, "clipboard is unavailable: {message}"),
            Self::Unsupported => formatter.write_str("clipboard is unsupported on this platform"),
        }
    }
}

impl std::error::Error for ClipboardError {}

/// Replaces the clipboard's contents with `text`.
///
/// # Errors
///
/// Returns [`ClipboardError::Unavailable`] when the platform clipboard exists
/// but refused the write — a headless Linux session, a revoked browser
/// permission, an Android activity that has been torn down — and
/// [`ClipboardError::Unsupported`] on a target with no clipboard at all.
///
/// # Examples
///
/// ```
/// use aimer_clipboard::clipboard;
///
/// // A copy control does not fail the frame over a clipboard it cannot reach.
/// if let Err(error) = clipboard::set_text("hello") {
///     eprintln!("copy failed: {error}");
/// }
/// ```
#[inline]
pub fn set_text(text: &str) -> Result<(), ClipboardError> {
    platform::set_text(text)
}

/// Text read from the clipboard, held where the platform delivered it.
///
/// It reads as a `str` and can be shared across threads. On macOS it is the bytes the pasteboard
/// handed over, so a paste of megabytes is not copied into a `String` first; elsewhere it is the
/// `String` that [`get_text`] returns. A caller that builds a rope or a buffer from it can slice
/// it in place and let the last slice drop it.
pub type ClipboardText = std::sync::Arc<dyn AsRef<str> + Send + Sync>;

/// The shortest text, in bytes, that [`set_text_chunks`] gives to the platform in pieces: a
/// shorter one is joined and written as [`set_text`] does, which is what every platform tests.
const CHUNKED_WRITE_MIN: usize = 64 * 1024;

/// Reads the clipboard's contents as plain text, without copying them into a `String` where the
/// platform allows: see [`ClipboardText`].
///
/// # Errors
///
/// The same as [`get_text`].
///
/// # Examples
///
/// ```
/// use aimer_clipboard::clipboard;
///
/// // A paste that is read in place: the text is a `str` for as long as it is held.
/// if let Ok(text) = clipboard::get_shared_text() {
///     let text: &str = (*text).as_ref();
///     let _ = text.len();
/// }
/// ```
pub fn get_shared_text() -> Result<ClipboardText, ClipboardError> {
    #[cfg(target_os = "macos")]
    if let Some(text) = macos_pasteboard::read() {
        return Ok(text);
    }
    platform::get_text().map(|text| std::sync::Arc::new(text) as ClipboardText)
}

/// Replaces the clipboard's contents with the text that is `chunks` one after the other.
///
/// A text that is already in pieces, such as the chunks of a rope, is written without being joined
/// into a `String` first: on macOS each piece is copied once, into the buffer the pasteboard
/// takes. A short text, and every platform without that path, joins the pieces and writes them
/// as [`set_text`] does.
///
/// # Errors
///
/// The same as [`set_text`].
///
/// # Examples
///
/// ```
/// use aimer_clipboard::clipboard;
///
/// let _ = clipboard::set_text_chunks(&["one ", "text ", "in three pieces"]);
/// ```
pub fn set_text_chunks(chunks: &[&str]) -> Result<(), ClipboardError> {
    let length: usize = chunks.iter().map(|chunk| chunk.len()).sum();
    #[cfg(target_os = "macos")]
    if length >= CHUNKED_WRITE_MIN && macos_pasteboard::write(chunks) {
        return Ok(());
    }
    let mut text = String::with_capacity(length);
    for chunk in chunks {
        text.push_str(chunk);
    }
    platform::set_text(&text)
}

/// Reads the clipboard's contents as plain text.
///
/// # Errors
///
/// Returns [`ClipboardError::Unavailable`] when the clipboard could not be
/// read, and [`ClipboardError::Unsupported`] when it holds no text — an image
/// on the pasteboard, an empty clipboard, or, on the web, a clipboard this
/// process has not written to, since the browser's read API is asynchronous and
/// cannot be awaited from a synchronous event handler.
///
/// # Examples
///
/// ```
/// use aimer_clipboard::clipboard;
///
/// // Paste is best-effort: an unreachable clipboard simply inserts nothing.
/// let pasted = clipboard::get_text().ok();
/// let _ = pasted;
/// ```
#[inline]
pub fn get_text() -> Result<String, ClipboardError> {
    platform::get_text()
}

#[cfg(all(
    not(target_arch = "wasm32"),
    not(any(
        target_os = "macos",
        target_os = "windows",
        target_os = "linux",
        target_os = "ios",
        target_os = "android"
    ))
))]
mod platform {
    use super::ClipboardError;

    pub(super) const fn set_text(_text: &str) -> Result<(), ClipboardError> {
        Err(ClipboardError::Unsupported)
    }

    pub(super) const fn get_text() -> Result<String, ClipboardError> {
        Err(ClipboardError::Unsupported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_names_the_reason_it_failed() {
        let error = ClipboardError::Unavailable("no display server".into());

        assert_eq!(
            error.to_string(),
            "clipboard is unavailable: no display server"
        );
    }

    #[test]
    fn unsupported_reads_as_a_permanent_absence() {
        assert_eq!(
            ClipboardError::Unsupported.to_string(),
            "clipboard is unsupported on this platform"
        );
    }

    /// Round-trips through the *real* platform clipboard, restoring whatever
    /// the developer had on it, and tolerates a machine that has none — a
    /// headless CI box is a legitimate `Unavailable`, not a failure of this
    /// crate.
    #[test]
    fn text_written_to_the_clipboard_reads_back() {
        let restore = get_text().ok();

        match set_text("aimer clipboard round trip") {
            Ok(()) => assert_eq!(
                get_text().expect("a clipboard that accepted a write can be read"),
                "aimer clipboard round trip"
            ),
            Err(ClipboardError::Unavailable(_)) => return,
            Err(ClipboardError::Unsupported) => return,
        }

        if let Some(previous) = restore {
            let _ = set_text(&previous);
        }
    }
}
