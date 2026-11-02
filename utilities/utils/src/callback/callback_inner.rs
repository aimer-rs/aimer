use std::cell::RefCell;
use std::pin::Pin;
use std::rc::Rc;

/// The body of an asynchronous callback: a producer of one boxed future per
/// invocation.
///
/// The future resolves to `()` rather than to the callback's return type
/// because an asynchronous callback is fire-and-forget — it is handed to an
/// executor and nobody is left holding a channel to receive a value from it.
///
/// It is deliberately **not** [`Send`]. The future is polled by
/// [`aimer_venus`] on the thread that owns the frame, which is the same thread
/// that owns the element tree, so a handler may capture a `StateUpdater`, a
/// controller, or any other [`Rc`] the tree handed it. A `Send` bound here
/// would have made those captures impossible for no benefit, because the work
/// has to come back to this thread to mutate anything anyway.
type AsyncBody<P> = dyn Fn(P) -> Pin<Box<dyn Future<Output = ()>>>;

/// A callback body, in whichever of the two flavours it was registered with.
///
/// Held behind an [`Rc`] rather than a [`Box`] so that cloning a
/// [`crate::callback::Callback`] — which every widget rebuild does — is one
/// refcount bump and the closure itself is allocated exactly once.
///
/// # Examples
///
/// ```
/// use std::rc::Rc;
///
/// use aimer_utils::callback::RawInnerCallback;
///
/// let doubled: RawInnerCallback<i32, i32> = RawInnerCallback::Sync(Rc::new(|n| n * 2));
/// assert_eq!(format!("{doubled:?}"), "Callback::Sync(i32 -> i32)");
/// ```
pub enum RawInnerCallback<P, R> {
    /// Runs to completion on the calling thread and yields a value.
    Sync(Rc<dyn Fn(P) -> R>),
    /// Produces a future for an executor to drive. Its value is discarded, so
    /// [`crate::callback::Callback::call`] reports nothing for this flavour.
    Async(Rc<AsyncBody<P>>),
}

impl<P, R> Clone for RawInnerCallback<P, R> {
    fn clone(&self) -> Self {
        match self {
            RawInnerCallback::Sync(body) => RawInnerCallback::Sync(Rc::clone(body)),
            RawInnerCallback::Async(body) => RawInnerCallback::Async(Rc::clone(body)),
        }
    }
}

impl<P, R> std::fmt::Debug for RawInnerCallback<P, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let p_type = std::any::type_name::<P>();
        let r_type = std::any::type_name::<R>();
        match self {
            RawInnerCallback::Sync(_) => write!(f, "Callback::Sync({p_type} -> {r_type})"),
            RawInnerCallback::Async(_) => write!(f, "Callback::Async({p_type} -> {r_type})"),
        }
    }
}

/// Wrapper that turns an `async` closure into an asynchronous callback.
///
/// The closure is [`FnOnce`], so it may consume what it captured. It is taken
/// on its first invocation and every later invocation does nothing — the same
/// contract [`crate::callback::VoidCallback::from_async`] documents, and the
/// reason a widget wanting to react to *every* press must clone its state into
/// the future instead of moving it.
///
/// # Examples
///
/// ```
/// use aimer_utils::callback::{AsyncCallback, Callback};
///
/// let callback: Callback<u32, ()> = AsyncCallback(|id: u32| async move {
///     let _ = id;
/// })
/// .into();
/// // Nothing runs synchronously, so there is no value to observe here.
/// assert_eq!(callback.call(7), None);
/// ```
pub struct AsyncCallback<F>(pub F);

impl<P, R, F, Fut> From<AsyncCallback<F>> for RawInnerCallback<P, R>
where
    F: FnOnce(P) -> Fut + 'static,
    Fut: Future<Output = ()> + 'static,
{
    fn from(callback: AsyncCallback<F>) -> Self {
        let body = RefCell::new(Some(callback.0));
        RawInnerCallback::Async(Rc::new(move |param| match body.borrow_mut().take() {
            Some(body) => Box::pin(body(param)),
            None => Box::pin(async {}),
        }))
    }
}
