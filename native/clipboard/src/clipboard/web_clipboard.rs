//! The web, through `navigator.clipboard` where it exists and a selected
//! `<textarea>` where it does not.
//!
//! `writeText` returns a `Promise`, and a browser event handler cannot block on
//! one: the write is therefore started and left to finish on its own, which is
//! exactly what a copy control needs.
//!
//! `navigator.clipboard` is **not always there**. Safari — desktop and mobile —
//! only defines it in a secure context, so a page served over plain HTTP, such
//! as a phone opening a development server by IP address, has the property
//! missing altogether. web-sys' getter is structural and hands back an
//! `undefined` wearing [`web_sys::Clipboard`]'s clothes, so calling `writeText`
//! on it throws `undefined is not an object` out of Wasm. Every write therefore
//! checks the property before touching it and falls back to
//! `document.execCommand("copy")`, which is the only path that works there.
//!
//! Reading is the awkward direction. `readText` is asynchronous *and*
//! permission-gated, so there is nothing to return synchronously except the
//! text this process wrote last — kept here for that reason. A clipboard filled
//! by another page or another application is not readable from a synchronous
//! call, and [`super::get_text`] reports that as
//! [`super::ClipboardError::Unsupported`].

use std::cell::RefCell;

use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;

use super::ClipboardError;

thread_local! {
    /// The last text this process copied, which is all a synchronous read can
    /// honestly offer on the web.
    static LAST_WRITTEN: RefCell<Option<String>> = const { RefCell::new(None) };
}

pub(super) fn set_text(text: &str) -> Result<(), ClipboardError> {
    let window = web_sys::window()
        .ok_or_else(|| ClipboardError::Unavailable("the browser window is missing".into()))?;
    LAST_WRITTEN.with(|last| *last.borrow_mut() = Some(text.to_owned()));

    match async_clipboard(&window) {
        Some(clipboard) => {
            write_asynchronously(&window, &clipboard, text);
            Ok(())
        }
        None => copy_through_selection(&window, text),
    }
}

pub(super) fn get_text() -> Result<String, ClipboardError> {
    LAST_WRITTEN
        .with(|last| last.borrow().clone())
        .ok_or(ClipboardError::Unsupported)
}

/// `navigator.clipboard`, or `None` where the browser does not expose it.
///
/// The guard is a value test rather than a `Result`, because the getter never
/// fails: it is the *call* on the missing object that throws.
fn async_clipboard(window: &web_sys::Window) -> Option<web_sys::Clipboard> {
    let clipboard = window.navigator().clipboard();
    let value: &JsValue = clipboard.as_ref();
    (!value.is_undefined() && !value.is_null()).then_some(clipboard)
}

/// Starts an asynchronous write, recovering through the legacy path if the
/// browser rejects it.
///
/// The rejection handler is not optional politeness: an unhandled rejection is
/// a console error on every browser, and a Safari that *has*
/// `navigator.clipboard` can still refuse the write when it does not credit the
/// call to a user gesture.
fn write_asynchronously(window: &web_sys::Window, clipboard: &web_sys::Clipboard, text: &str) {
    let promise = clipboard.write_text(text);
    let window = window.clone();
    let text = text.to_owned();
    let recover = Closure::once(move |_rejection: JsValue| {
        let _ = copy_through_selection(&window, &text);
    });
    let _ = promise.catch(&recover);
    // The handler outlives this call by construction — the browser settles the
    // promise after it returns — so it is handed to the JS side for good. It is
    // one closure per copy, and a copy is a user gesture, not a frame.
    recover.forget();
}

/// The pre-`navigator.clipboard` copy: a throwaway `<textarea>`, selected and
/// handed to `document.execCommand("copy")`.
///
/// It must run inside the gesture that asked for the copy, which it does —
/// [`set_text`] is called straight from a pointer or key handler.
fn copy_through_selection(window: &web_sys::Window, text: &str) -> Result<(), ClipboardError> {
    let document = window
        .document()
        .ok_or_else(|| ClipboardError::Unavailable("the document is missing".into()))?;
    let body = document
        .body()
        .ok_or_else(|| ClipboardError::Unavailable("the document has no body".into()))?;
    let textarea: web_sys::HtmlTextAreaElement = document
        .create_element("textarea")
        .map_err(|_| ClipboardError::Unavailable("a textarea could not be created".into()))?
        .unchecked_into();

    textarea.set_value(text);
    // Read-only keeps the on-screen keyboard away on iOS while still allowing
    // the selection `execCommand` copies from.
    textarea.set_read_only(true);
    let style = textarea.style();
    for (property, value) in [
        ("position", "fixed"),
        ("top", "0"),
        ("left", "0"),
        ("width", "1px"),
        ("height", "1px"),
        ("padding", "0"),
        ("border", "none"),
        ("outline", "none"),
        ("box-shadow", "none"),
        ("background", "transparent"),
        // Anything under 16px makes Safari zoom the page as the field focuses.
        ("font-size", "16px"),
    ] {
        let _ = style.set_property(property, value);
    }

    let previously_focused = document.active_element();
    body.append_child(&textarea)
        .map_err(|_| ClipboardError::Unavailable("the textarea could not be attached".into()))?;
    let _ = textarea.focus();
    textarea.select();
    // UTF-16 units, because that is what the DOM counts.
    let _ = textarea.set_selection_range(0, text.encode_utf16().count() as u32);

    let html_document: &web_sys::HtmlDocument = document.unchecked_ref();
    let copied = html_document.exec_command("copy").unwrap_or(false);

    let _ = body.remove_child(&textarea);
    let previously_focused =
        previously_focused.and_then(|element| element.dyn_into::<web_sys::HtmlElement>().ok());
    if let Some(previous) = previously_focused {
        let _ = previous.focus();
    }

    if copied {
        Ok(())
    } else {
        Err(ClipboardError::Unavailable(
            "the browser refused document.execCommand(\"copy\")".into(),
        ))
    }
}
