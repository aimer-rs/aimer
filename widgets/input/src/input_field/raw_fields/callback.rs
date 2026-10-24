/// Write text to the system clipboard.
///
/// Copy is best-effort: a clipboard the platform will not give us is not worth
/// failing an edit over.
fn clipboard_write(text: &str) {
    let _ = aimer_native::clipboard::set_text(text);
}

/// Read text from the system clipboard.
///
/// On the web the platform clipboard is asynchronous and cannot be awaited from
/// a key handler, so the hidden `<input>` the IME already writes through is used
/// as the fallback — the browser fills it on a native paste.
fn clipboard_read() -> Option<String> {
    if let Ok(text) = aimer_native::clipboard::get_text() {
        return Some(text);
    }
    #[cfg(target_arch = "wasm32")]
    {
        let window = web_sys::window()?;
        let document = window.document()?;
        let element = document.get_element_by_id(HIDDEN_INPUT_ID)?;
        use wasm_bindgen::JsCast;
        let input: web_sys::HtmlInputElement = element.unchecked_into();
        let value = input.value();
        return if value.is_empty() { None } else { Some(value) };
    }
    #[cfg(not(target_arch = "wasm32"))]
    None
}
type BoxedTextFieldFuture = Box<dyn Fn(String) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send>;

/// Inner enum distinguishing sync vs async text-field callbacks.
#[cfg(not(target_arch = "wasm32"))]
enum TextFieldCb {
    Sync(Box<dyn Fn(String)>),
    Async(BoxedTextFieldFuture),
}

#[cfg(target_arch = "wasm32")]
enum TextFieldCb {
    Sync(Box<dyn Fn(String)>),
    Async(Box<dyn Fn(String) -> Pin<Box<dyn Future<Output = ()>>>>),
}

/// A cloneable, optional callback that receives the current text value.
///
/// Used for `on_changed` (fired after every text mutation) and
/// `on_submitted` (fired when the user presses Enter).
///
/// Supports both synchronous and asynchronous closures.
///
/// # Examples
/// ```rust,ignore
/// // Sync
/// TextField::create_new()
///     .on_changed(|text| println!("changed: {text}"))
///
/// // Async (wrap with AsyncTextFieldCallback)
/// TextField::create_new()
///     .on_changed(AsyncTextFieldCallback(|text| async move {
///         println!("changed: {text}");
///     }))
/// ```
#[derive(Clone, Default)]
pub struct TextFieldCallback(Option<Rc<TextFieldCb>>);

/// Wrapper to convert an async closure that takes a `String` into a
/// `TextFieldCallback`.
///
/// # Examples
/// ```rust,ignore
/// use control::input::AsyncTextFieldCallback;
///
/// TextField::create_new()
///     .on_changed(AsyncTextFieldCallback(|text| async move {
///         println!("async changed: {text}");
///     }))
/// ```
#[derive(Default)]
pub struct AsyncTextFieldCallback<F>(pub F);

impl TextFieldCallback {
    /// Invoke the callback if one is set.
    pub fn call(&self, text: &str) {
        if let Some(cb) = &self.0 {
            match cb.as_ref() {
                TextFieldCb::Sync(f) => f(text.to_owned()),
                TextFieldCb::Async(f) => {
                    let fut = f(text.to_owned());
                    #[cfg(not(target_arch = "wasm32"))]
                    {
                        if let Ok(handle) = tokio::runtime::Handle::try_current() {
                            handle.spawn(fut);
                        }
                    }
                    #[cfg(target_arch = "wasm32")]
                    {
                        wasm_bindgen_futures::spawn_local(fut);
                    }
                }
            }
        }
    }

    /// Returns `true` if a callback is set.
    pub fn is_some(&self) -> bool {
        self.0.is_some()
    }
}

impl<F> From<F> for TextFieldCallback
where
    F: Fn(String) + 'static,
{
    fn from(f: F) -> Self {
        Self(Some(Rc::new(TextFieldCb::Sync(Box::new(f)))))
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl<F, Fut> From<AsyncTextFieldCallback<F>> for TextFieldCallback
where
    F: Fn(String) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    fn from(ac: AsyncTextFieldCallback<F>) -> Self {
        Self(Some(Rc::new(TextFieldCb::Async(Box::new(move |s| {
            Box::pin(ac.0(s))
        })))))
    }
}

#[cfg(target_arch = "wasm32")]
impl<F, Fut> From<AsyncTextFieldCallback<F>> for TextFieldCallback
where
    F: Fn(String) -> Fut + 'static,
    Fut: Future<Output = ()> + 'static,
{
    fn from(ac: AsyncTextFieldCallback<F>) -> Self {
        Self(Some(Rc::new(TextFieldCb::Async(Box::new(move |s| {
            Box::pin(ac.0(s))
        })))))
    }
}

impl std::fmt::Debug for TextFieldCallback {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.0.is_some() {
            write!(f, "TextFieldCallback(Some(...))")
        } else {
            write!(f, "TextFieldCallback(None)")
        }
    }
}

