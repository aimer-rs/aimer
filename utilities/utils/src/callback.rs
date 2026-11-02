pub mod callback_inner;

use std::any::type_name;
use std::fmt::Debug;
use std::pin::Pin;
use std::rc::Rc;

pub use callback_inner::*;

/// One invocation of an [`RawInnerCallback::Async`] body, ready to be spawned.
type SpawnedFuture = Pin<Box<dyn Future<Output = ()>>>;

/// Hands `future` to Aimer's runtime, and says so out loud when there is none.
///
/// The future becomes a microtask, so its effect is visible to the *build phase
/// of the frame it was raised in* rather than a frame later. That ordering is
/// the whole reason a callback no longer carries a runtime handle: a general
/// purpose executor can promise "soon", but only the thread that owns the
/// frame can promise "before the next build".
///
/// One policy for the whole framework, which is the point: this used to be
/// hand-written at every call site, and the copies disagreed — some reached for
/// the ambient runtime, one silently did nothing at all.
fn spawn(future: SpawnedFuture) {
    if aimer_venus::spawn_local(future).is_none() {
        crate::log::warn("an async callback was discarded: no Venus runtime is installed");
    }
}

/// The contract for invoking a callback, however it was registered.
///
/// Implementors expose their body through [`Self::raw`]; [`Self::execute`] then
/// runs it, and is the only place in the framework that decides what an
/// asynchronous callback means. Implementing this trait is how a type that is
/// *not* a [`Callback`] — a newtype, a field wrapper — becomes invocable
/// through the same generic code.
///
/// # Associated Types
/// - `Args`: the argument the body accepts.
/// - `Output`: the value a synchronous body returns.
///
/// # Examples
///
/// ```
/// use std::rc::Rc;
///
/// use aimer_utils::callback::{CallbackExecutor, RawInnerCallback};
///
/// struct Greeter {
///     callback: Option<RawInnerCallback<i32, String>>,
/// }
///
/// impl CallbackExecutor for Greeter {
///     type Args = i32;
///     type Output = String;
///
///     fn raw(&self) -> Option<&RawInnerCallback<Self::Args, Self::Output>> {
///         self.callback.as_ref()
///     }
/// }
///
/// let greeter = Greeter {
///     callback: Some(RawInnerCallback::Sync(Rc::new(|n| format!("{n} times")))),
/// };
/// assert_eq!(greeter.execute(3), Some("3 times".to_owned()));
/// ```
pub trait CallbackExecutor {
    type Args;
    type Output;

    /// The registered body, or `None` when nothing was registered.
    fn raw(&self) -> Option<&RawInnerCallback<Self::Args, Self::Output>>;

    /// Invokes the callback, returning the value a synchronous body produced.
    ///
    /// Returns `None` when nothing was registered, and when the body is
    /// asynchronous — that value belongs to the runtime, not to the caller.
    ///
    /// An asynchronous body becomes a microtask on the Venus runtime installed
    /// for this thread, so it runs before the next build phase. Only when no
    /// runtime is installed at all is the work discarded, and that is logged
    /// rather than passed over in silence.
    fn execute(&self, args: Self::Args) -> Option<Self::Output> {
        match self.raw()? {
            RawInnerCallback::Sync(body) => Some(body(args)),
            RawInnerCallback::Async(body) => {
                spawn(body(args));
                None
            }
        }
    }
}

/// A callback taking `Args` and returning `Return`, which may or may not have
/// been registered.
///
/// Unregistered is the overwhelmingly common case — a widget with eleven
/// possible handlers typically sets one — so "empty" is the `None` of an
/// [`Option`], costing a write and nothing else. Cloning a registered callback
/// is one refcount bump, which matters because every widget rebuild clones
/// every callback it carries.
///
/// # Type Parameters
/// - `Args`: the argument the callback accepts. Defaults to `()`.
/// - `Return`: the value it returns. Defaults to `()`.
///
/// # Examples
///
/// ```
/// use aimer_utils::callback::Callback;
///
/// let increment = Callback::from(|x: i32| x + 1);
/// assert_eq!(increment.call(1), Some(2));
///
/// // A callback nobody registered simply reports that there was nothing to run.
/// assert_eq!(Callback::<i32, i32>::default().call(1), None);
/// ```
pub struct Callback<Args = (), Return = ()> {
    inner: Option<RawInnerCallback<Args, Return>>,
}

impl<Args, Return> Callback<Args, Return> {
    /// Whether a body was registered, synchronous or asynchronous.
    ///
    /// A default callback is not set. A widget that forwards an optional
    /// callback can use this to avoid declaring interest in an event nobody
    /// handles.
    ///
    /// # Examples
    ///
    /// ```
    /// use aimer_utils::callback::Callback;
    ///
    /// assert!(!Callback::<i32, i32>::default().is_set());
    /// assert!(Callback::from(|x: i32| x + 1).is_set());
    /// ```
    #[inline]
    pub fn is_set(&self) -> bool {
        self.inner.is_some()
    }

    /// Invokes a **synchronous** callback with `args`, returning its result.
    ///
    /// Returns `None` when nothing was registered, and when the body is
    /// asynchronous — there is no executor to hand it to here. Use
    /// [`CallbackExecutor::execute`] where an asynchronous body must run.
    #[inline]
    pub fn call(&self, args: Args) -> Option<Return> {
        match self.inner.as_ref()? {
            RawInnerCallback::Sync(body) => Some(body(args)),
            RawInnerCallback::Async(_) => None,
        }
    }
}

impl<Args, Return> Default for Callback<Args, Return> {
    #[inline]
    fn default() -> Self {
        Self { inner: None }
    }
}

impl<Args, Return> Clone for Callback<Args, Return> {
    #[inline]
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<Args, Return> Debug for Callback<Args, Return> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let args_type = type_name::<Args>();
        let return_type = type_name::<Return>();
        write!(f, "Callback({args_type} -> {return_type})",)
    }
}

impl<Args, Return, F: Fn(Args) -> Return + 'static> From<F> for Callback<Args, Return> {
    #[inline]
    fn from(f: F) -> Self {
        Self {
            inner: Some(RawInnerCallback::Sync(Rc::new(f))),
        }
    }
}

impl<Args, Return, F, Fut> From<AsyncCallback<F>> for Callback<Args, Return>
where
    F: FnOnce(Args) -> Fut + 'static,
    Fut: Future<Output = ()> + 'static,
{
    #[inline]
    fn from(callback: AsyncCallback<F>) -> Self {
        Self {
            inner: Some(RawInnerCallback::from(callback)),
        }
    }
}

impl<P, R> CallbackExecutor for Callback<P, R> {
    type Args = P;
    type Output = R;

    #[inline]
    fn raw(&self) -> Option<&RawInnerCallback<Self::Args, Self::Output>> {
        self.inner.as_ref()
    }
}

pub type VoidParamedFunction<R> = Callback<R, ()>;

/// A callback taking no argument and returning nothing.
///
/// A distinct type rather than an alias for `Callback<(), ()>` because the
/// blanket `From<F: Fn(Args) -> Return>` impl would demand `Fn(())`, and a
/// plain `|| ...` does not coerce to that. Everything else is delegated, so the
/// two share one representation and one dispatch.
///
/// # Examples
///
/// ```
/// use std::cell::Cell;
/// use std::rc::Rc;
///
/// use aimer_utils::callback::{CallbackExecutor, VoidCallback};
///
/// let presses = Rc::new(Cell::new(0));
/// let counted = presses.clone();
/// let on_press = VoidCallback::from(move || counted.set(counted.get() + 1));
///
/// on_press.execute(());
/// assert_eq!(presses.get(), 1);
/// ```
#[derive(Default, Clone)]
pub struct VoidCallback(Callback<(), ()>);

impl VoidCallback {
    /// Whether a body was registered, synchronous or asynchronous.
    ///
    /// A default callback is not set. See [`Callback::is_set`].
    ///
    /// # Examples
    ///
    /// ```
    /// use aimer_utils::callback::VoidCallback;
    ///
    /// assert!(!VoidCallback::default().is_set());
    /// assert!(VoidCallback::from(|| {}).is_set());
    /// ```
    #[inline]
    pub fn is_set(&self) -> bool {
        self.0.is_set()
    }

    /// Registers an `async` closure, to be driven by Aimer's runtime.
    ///
    /// Unlike the `From<F>` impl — which takes an `Fn()` — this accepts an
    /// [`FnOnce`], so the closure may consume what it captured. It is taken on
    /// its first invocation and every later one does nothing; a handler that
    /// must react to *every* press has to clone its state into the future
    /// rather than move it.
    ///
    /// Neither the closure nor its future has to be [`Send`], because both are
    /// polled on the thread that owns the frame. That is what allows a handler
    /// to `await` while still holding a `StateUpdater` or a controller.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::cell::Cell;
    /// use std::rc::Rc;
    ///
    /// use aimer_utils::callback::VoidCallback;
    ///
    /// let pressed = Rc::new(Cell::new(false));
    /// let flag = pressed.clone();
    /// let callback = VoidCallback::from_async(move || async move {
    ///     flag.set(true);
    /// });
    /// ```
    #[inline]
    pub fn from_async<F, Fut>(f: F) -> Self
    where
        F: FnOnce() -> Fut + 'static,
        Fut: Future<Output = ()> + 'static,
    {
        Self(Callback::from(AsyncCallback(move |()| f())))
    }
}

impl Debug for VoidCallback {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "VoidCallback()")
    }
}

impl<F: Fn() + 'static> From<F> for VoidCallback {
    #[inline]
    fn from(f: F) -> Self {
        Self(Callback::from(move |()| f()))
    }
}

impl CallbackExecutor for VoidCallback {
    type Args = ();
    type Output = ();

    #[inline]
    fn raw(&self) -> Option<&RawInnerCallback<Self::Args, Self::Output>> {
        self.0.raw()
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use aimer_venus::Venus;

    use super::*;

    /// Installs a runtime for this test's thread, the way an event loop does
    /// for the thread it draws on.
    ///
    /// Every test runs on a thread of its own, so the installed runtime is this
    /// test's alone and nothing has to be torn down between them.
    fn installed_runtime() -> Rc<Venus> {
        let venus = Venus::new();
        venus.install();
        venus
    }

    #[test]
    fn a_callback_says_whether_anything_was_registered() {
        assert!(!Callback::<i32, i32>::default().is_set());
        assert!(Callback::from(|value: i32| value).is_set());
        assert!(!VoidCallback::default().is_set());
        assert!(VoidCallback::from(|| {}).is_set());
        assert!(VoidCallback::from_async(|| async {}).is_set());
    }

    #[test]
    fn an_unset_callback_reports_nothing_to_call() {
        let callback = Callback::<i32, i32>::default();

        assert_eq!(callback.call(7), None);
        assert_eq!(callback.execute(7), None);
    }

    #[test]
    fn a_sync_callback_returns_its_value_through_both_entry_points() {
        let callback = Callback::from(|value: i32| value + 1);

        assert_eq!(callback.call(1), Some(2));
        assert_eq!(callback.execute(1), Some(2));
    }

    #[test]
    fn clones_share_the_one_closure() {
        let calls = Rc::new(Cell::new(0));
        let counted = calls.clone();
        let callback = Callback::from(move |_: ()| counted.set(counted.get() + 1));

        callback.clone().call(());
        callback.call(());

        assert_eq!(calls.get(), 2);
    }

    // `call` is the synchronous entry point: it cannot spawn, so rather than
    // running an async body on the calling thread it reports that there was
    // nothing to run synchronously.
    #[test]
    fn call_refuses_an_async_callback_rather_than_running_it() {
        let _venus = installed_runtime();
        let ran = Rc::new(Cell::new(0));
        let counted = ran.clone();
        let callback: Callback<(), ()> = AsyncCallback(move |_| {
            let counted = counted.clone();
            async move { counted.set(counted.get() + 1) }
        })
        .into();

        assert_eq!(callback.call(()), None);
        assert_eq!(ran.get(), 0);
    }

    // The property the whole migration was for: a handler may capture an `Rc`
    // from the element tree — a `StateUpdater`, a controller — and still hold it
    // across an await. Under a work-stealing runtime this did not compile.
    #[test]
    fn an_async_callback_may_hold_an_rc_across_an_await() {
        let venus = installed_runtime();
        let state = Rc::new(Cell::new(0));
        let updater = state.clone();
        let callback = VoidCallback::from_async(move || async move {
            aimer_venus::yield_now().await;
            updater.set(updater.get() + 1);
        });

        assert_eq!(callback.execute(()), None);
        while venus.task_count() > 0 {
            venus.run_microtasks();
        }

        assert_eq!(state.get(), 1);
    }

    // An asynchronous handler is a microtask, so its effect lands before the
    // build phase of the frame it was raised in rather than a frame later.
    #[test]
    fn an_async_callback_lands_before_the_build_of_the_frame_it_was_raised_in() {
        let venus = installed_runtime();
        let state = Rc::new(Cell::new(0));
        let built_with = Rc::new(Cell::new(-1));

        let updater = state.clone();
        let callback = VoidCallback::from_async(move || async move { updater.set(7) });
        callback.execute(());

        assert_eq!(state.get(), 0, "nothing runs before the frame drains");

        let read = state.clone();
        let observed = built_with.clone();
        venus.drive_frame(|| observed.set(read.get()));

        assert_eq!(built_with.get(), 7);
    }

    // The reach `aimer_svg` and `aimer_text` used to hand-roll a fallback for:
    // an element that was handed nothing still finds the runtime, because the
    // runtime belongs to the thread rather than to whoever passed it down.
    #[test]
    fn an_async_callback_reaches_the_runtime_without_being_handed_one() {
        let venus = installed_runtime();
        let ran = Rc::new(Cell::new(0));
        let counted = ran.clone();
        let callback = VoidCallback::from_async(move || async move {
            counted.set(counted.get() + 1);
        });

        callback.execute(());
        venus.run_microtasks();

        assert_eq!(ran.get(), 1);
    }

    // A one-shot body is taken on its first invocation. `Button` documents that
    // a second press does nothing, so a second invocation must not panic — the
    // two async constructors disagreed about this before they shared a body.
    #[test]
    fn a_second_invocation_of_a_one_shot_async_callback_does_nothing() {
        let venus = installed_runtime();
        let ran = Rc::new(Cell::new(0));
        let counted = ran.clone();
        let callback = VoidCallback::from_async(move || async move {
            counted.set(counted.get() + 1);
        });

        callback.execute(());
        callback.execute(());
        venus.run_microtasks();

        assert_eq!(ran.get(), 1);
    }

    // A widget exercised in isolation has no event loop, and losing a task there
    // must not take the test process with it.
    #[test]
    fn an_async_callback_without_any_runtime_is_dropped_rather_than_panicking() {
        let callback = VoidCallback::from_async(|| async {});

        assert_eq!(callback.execute(()), None);
    }

    #[test]
    fn a_void_callback_delegates_to_the_shared_representation() {
        let calls = Rc::new(Cell::new(0));
        let counted = calls.clone();
        let callback = VoidCallback::from(move || counted.set(counted.get() + 1));

        callback.execute(());
        callback.clone().execute(());
        VoidCallback::default().execute(());

        assert_eq!(calls.get(), 2);
    }

    #[test]
    fn debug_names_the_signature() {
        assert_eq!(
            format!("{:?}", Callback::<i32, bool>::default()),
            "Callback(i32 -> bool)"
        );
        assert_eq!(format!("{:?}", VoidCallback::default()), "VoidCallback()");
    }
}
