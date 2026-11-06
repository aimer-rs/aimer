//! The browser: a hidden `<input type="file">`.
//!
//! A page cannot open a file dialog directly; it can click an input element and
//! hear back through its events. [`pick`] creates one, configures `accept` and
//! `multiple`, clicks it, and resolves when `change` fires with the chosen
//! `File`s or `cancel` fires with none. Browsers only honour the click while the
//! page has *transient user activation*, so the task that calls
//! [`FilePicker::pick`](crate::FilePicker::pick) must start from the click or tap
//! that asked for it.
//!
//! Browsers older than the `cancel` event (before 2023) say nothing when the
//! dialog is dismissed, so the future stays pending; dropping it removes the
//! element and its listeners.
//!
//! A `File` has no path: the handle reads it with `Blob.arrayBuffer`.

use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use wasm_bindgen_futures::JsFuture;
use web_sys::{Event, File, HtmlInputElement};

use aimer_venus::oneshot;

use crate::error::PickError;
use crate::filter::FileFilter;
use crate::future::{PickFuture, ready};
use crate::picked::{PickedFile, Source};
use crate::request::FilePicker;

pub(crate) fn pick(request: FilePicker) -> PickFuture<Vec<PickedFile>> {
    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return ready(Err(PickError::Unsupported));
    };
    let Some(input) = document
        .create_element("input")
        .ok()
        .and_then(|element| element.dyn_into::<HtmlInputElement>().ok())
    else {
        return ready(Err(PickError::Unsupported));
    };
    input.set_type("file");
    input.set_multiple(request.multiple);
    let accept = accept(&request.filters);
    if !accept.is_empty() {
        input.set_accept(&accept);
    }
    // Safari only opens the dialog for an element that is in the document.
    let _ = input.style().set_property("display", "none");
    if let Some(body) = document.body() {
        let _ = body.append_child(&input);
    }

    let (sender, receiver) = oneshot::channel::<Vec<File>>();
    let sender = std::rc::Rc::new(std::cell::RefCell::new(Some(sender)));

    let on_change = {
        let sender = sender.clone();
        let input = input.clone();
        Closure::<dyn FnMut(Event)>::new(move |_event: Event| {
            let mut files = Vec::new();
            if let Some(list) = input.files() {
                files.extend((0..list.length()).filter_map(|index| list.get(index)));
            }
            if let Some(sender) = sender.borrow_mut().take() {
                let _ = sender.send(files);
            }
        })
    };
    let on_cancel = Closure::<dyn FnMut(Event)>::new(move |_event: Event| {
        if let Some(sender) = sender.borrow_mut().take() {
            let _ = sender.send(Vec::new());
        }
    });
    let _ = input.add_event_listener_with_callback("change", on_change.as_ref().unchecked_ref());
    let _ = input.add_event_listener_with_callback("cancel", on_cancel.as_ref().unchecked_ref());

    let mut element = Element {
        input,
        _on_change: on_change,
        _on_cancel: on_cancel,
    };
    element.input.click();

    Box::pin(async move {
        let files = receiver.await.map_err(|_| PickError::Interrupted)?;
        element.close();
        Ok(files
            .into_iter()
            .map(|file| {
                PickedFile::from_parts(
                    file.name(),
                    Some(file.type_()),
                    Some(file.size() as u64),
                    Source::Blob(file),
                )
            })
            .collect())
    })
}

/// The input element and the listeners that must outlive it.
struct Element {
    input: HtmlInputElement,
    _on_change: Closure<dyn FnMut(Event)>,
    _on_cancel: Closure<dyn FnMut(Event)>,
}

impl Element {
    /// Takes the element out of the page. Also done on drop, so a dropped future
    /// leaves nothing behind.
    fn close(&mut self) {
        self.input.remove();
    }
}

impl Drop for Element {
    fn drop(&mut self) {
        self.close();
    }
}

/// The `accept` attribute: `.ext` and MIME types, comma-separated.
fn accept(filters: &[FileFilter]) -> String {
    let mut tokens: Vec<String> = Vec::new();
    let mut add = |token: String| {
        if !tokens.contains(&token) {
            tokens.push(token);
        }
    };
    for filter in filters {
        for extension in filter.extension_list() {
            add(format!(".{extension}"));
        }
        for mime in filter.resolved_mime_types() {
            add(mime);
        }
    }
    tokens.join(",")
}

/// Reads a browser `File` through `Blob.arrayBuffer`.
pub(crate) fn read(file: File) -> PickFuture<Vec<u8>> {
    Box::pin(async move {
        let buffer = JsFuture::from(file.array_buffer())
            .await
            .map_err(|error| PickError::Platform(format!("{error:?}")))?;
        Ok(js_sys::Uint8Array::new(&buffer).to_vec())
    })
}
