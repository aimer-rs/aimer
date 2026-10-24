use aimer_rubick::ShareRef;
use aimer_widget::base::{BuildConsumer, BuildContext, ResolvedSize, Size, Vec2d, WindowHandle};
use aimer_widget::{
    AnyElement, AnyWidget, ChildBuilder, Drawable, Element, EventElement, EventTreeRole,
    LayoutElement, Rebuildable, RequiredChild, State, StateUpdater, StatefulElement,
    StatefulWidget, VisitorElement, Widget,
};
use std::any::type_name;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::error::Error;
use std::fmt;
use std::marker::PhantomData;
use std::ops::Deref;
use std::rc::{Rc, Weak};

use aimer_widget::portable::__anteros::{ValueSchemaMetadata, Version};
use aimer_widget::portable::PortableValue;

/// The error returned by a portable provider value codec.
///
/// Provider codecs run in the guest build and must turn failures into a
/// candidate diagnostic instead of panicking. The message is intentionally
/// owned so a codec can report a validation detail without borrowing the value
/// or its temporary input buffer.
/// 
#[derive(Debug)]
pub struct PortableProviderCodecError(String);

impl PortableProviderCodecError {
    /// Creates a codec error with an actionable message.
    #[inline]
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for PortableProviderCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for PortableProviderCodecError {}

/// A bounded, versioned codec for one provider value type.
///
/// The codec is a pair of function pointers rather than a captured closure.
/// That keeps the contract deterministic and makes it explicit that provider
/// values cross the guest boundary as bytes only. Native handles, `Rc`,
/// `RefCell`, closures, and platform resources must remain host-owned.
/// 
#[derive(Clone, Copy)]
pub struct PortableProviderCodec<T> {
    schema: ValueSchemaMetadata<'static>,
    encode: fn(&T) -> Result<Vec<u8>, PortableProviderCodecError>,
    decode: fn(&[u8], Version) -> Result<T, PortableProviderCodecError>,
}

impl<T> PortableProviderCodec<T> {
    /// Creates a provider codec with stable value metadata and explicit
    /// encode/decode functions.
    #[inline]
    pub const fn new(
        schema: ValueSchemaMetadata<'static>,
        encode: fn(&T) -> Result<Vec<u8>, PortableProviderCodecError>,
        decode: fn(&[u8], Version) -> Result<T, PortableProviderCodecError>,
    ) -> Self {
        Self {
            schema,
            encode,
            decode,
        }
    }

    /// Returns the stable identity, version, and byte ceiling for this codec.
    #[inline]
    pub const fn schema(&self) -> ValueSchemaMetadata<'static> {
        self.schema
    }

    /// Encodes one provider snapshot using the codec's bounded contract.
    #[inline]
    pub fn encode(&self, value: &T) -> Result<Vec<u8>, PortableProviderCodecError> {
        (self.encode)(value)
    }

    /// Decodes one validated provider snapshot.
    #[inline]
    pub fn decode(&self, bytes: &[u8], version: Version) -> Result<T, PortableProviderCodecError> {
        (self.decode)(bytes, version)
    }
}

impl<T: PortableValue> PortableProviderCodec<T> {
    /// Creates a provider codec from a derived or handwritten PortableValue.
    ///
    /// The value's schema identity, version, limits, and four-byte wire header
    /// are reused verbatim, so provider snapshots and reflected property blobs
    /// cannot silently drift onto separate formats.
    /// 
    #[inline]
    pub fn from_portable_value() -> Self {
        PortableProviderCodec::new(
            T::SCHEMA,
            encode_portable_value::<T>,
            decode_portable_value::<T>,
        )
    }
}

fn encode_portable_value<T: PortableValue>(
    value: &T,
) -> Result<Vec<u8>, PortableProviderCodecError> {
    value
        .encode_value()
        .map_err(|error| PortableProviderCodecError::new(error.to_string()))
}

fn decode_portable_value<T: PortableValue>(
    bytes: &[u8],
    version: Version,
) -> Result<T, PortableProviderCodecError> {
    T::decode_value(bytes, version)
        .map_err(|error| PortableProviderCodecError::new(error.to_string()))
}

struct Subscriber<T> {
    consumer: Weak<BuildConsumer>,
    should_notify: Box<dyn FnMut(&T) -> bool>,
}

/// A callback registered with [`ProviderHandle::observe`].
type Observer<T> = Box<dyn FnMut(&Snapshot<T>)>;

struct ProviderStore<T> {
    value: RefCell<Rc<T>>,
    subscribers: RefCell<HashMap<u64, Subscriber<T>>>,
    next_subscriber: Cell<u64>,
    observers: RefCell<Vec<(u64, Observer<T>)>>,
    next_observer: Cell<u64>,
}

/// Keeps a [`ProviderHandle::observe`] callback registered; dropping it
/// unregisters the callback.
#[must_use = "the observer is removed as soon as the observation is dropped"]
pub struct Observation(Option<Box<dyn FnOnce()>>);

impl Drop for Observation {
    fn drop(&mut self) {
        if let Some(remove) = self.0.take() {
            remove();
        }
    }
}

/// An immutable, cheaply cloneable version of a provided value.
///
/// A snapshot dereferences to `T`, so fields and methods can be accessed
/// directly. It keeps the version observed at read time alive even after the
/// provider is updated.
pub struct Snapshot<T>(Rc<T>);

impl<T> Snapshot<T> {
    /// Creates a read-only handle to a borrowed part of `T`.
    #[inline]
    pub fn project<Field, Select>(&self, select: Select) -> ShareRef<Field>
    where
        T: 'static,
        Field: ?Sized + 'static,
        Select: for<'a> Fn(&'a T) -> &'a Field + 'static,
    {
        ShareRef::from_rc(&self.0).project(select)
    }
}

impl<T> Clone for Snapshot<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T> Deref for Snapshot<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.0.as_ref()
    }
}

/// A handle for reading and updating one provided value outside a widget build.
///
/// Cloning a handle keeps access to the same provider store. Reading creates a
/// cheap [`Snapshot`], while updating uses copy-on-write to preserve snapshots
/// that are still alive.
pub struct ProviderHandle<T>(Rc<ProviderStore<T>>);

impl<T> Clone for ProviderHandle<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T: 'static> ProviderHandle<T> {
    /// Creates a handle containing `value` for use by a provider scope.
    pub fn new(value: T) -> Self {
        Self(Rc::new(ProviderStore {
            value: RefCell::new(Rc::new(value)),
            subscribers: RefCell::new(HashMap::new()),
            next_subscriber: Cell::new(0),
            observers: RefCell::new(Vec::new()),
            next_observer: Cell::new(0),
        }))
    }
    /// Returns an immutable snapshot of the current value without subscribing a
    /// widget.
    pub fn read(&self) -> Snapshot<T> {
        Snapshot(self.0.value.borrow().clone())
    }

    /// Returns the nearest handle for `T`, or `None` when no matching provider
    /// is in scope.
    pub fn try_of(context: &BuildContext) -> Option<Self> {
        context
            .get_state::<Provided<T>>()
            .map(|provided| provided.0.clone())
    }

    /// Returns the nearest handle for `T`.
    ///
    /// # Panics
    ///
    /// Panics when no matching provider is in the current widget scope.
    ///
    /// The panic is raised in the body of this `#[track_caller]` method rather
    /// than inside a closure: a closure is an untracked frame, so panicking
    /// there blames this file instead of the widget that asked for the value.
    #[track_caller]
    pub fn of(context: &BuildContext) -> Self {
        let Some(handle) = Self::try_of(context) else {
            panic!(
                "No provider for `{}` found in the current widget scope",
                type_name::<T>()
            )
        };
        handle
    }

    /// Mutates the current value and notifies subscribed widgets.
    ///
    /// The value is cloned only when an existing [`Snapshot`] still shares the
    /// previous version.
    pub fn update(&self, mutation: impl FnOnce(&mut T))
    where
        T: Clone,
    {
        {
            let mut value = self
                .0
                .value
                .try_borrow_mut()
                .expect("provider value is already borrowed during an update");
            mutation(Rc::make_mut(&mut value));
        }
        self.notify();
    }

    /// Applies an action through a reducer and notifies subscribed widgets.
    pub fn dispatch<A>(&self, action: A, reducer: impl FnOnce(&mut T, A))
    where
        T: Clone,
    {
        self.update(|value| reducer(value, action));
    }

    /// Calls `observer` with the new value after every update, whichever handle
    /// made it, including the handle a widget gets from its build context.
    ///
    /// Unlike a widget subscription, an observer is not tied to a build: it is
    /// how state outside the widget tree, such as a saver that persists the
    /// value, follows a provider. The callback runs synchronously inside the
    /// update, so it must be quick and must not update or observe this provider
    /// itself; hand the snapshot to a task instead. It stays registered until
    /// the returned [`Observation`] is dropped.
    pub fn observe(&self, observer: impl FnMut(&Snapshot<T>) + 'static) -> Observation {
        let id = self.0.next_observer.get();
        self.0.next_observer.set(id.wrapping_add(1));
        self.0.observers.borrow_mut().push((id, Box::new(observer)));
        let store = Rc::downgrade(&self.0);
        Observation(Some(Box::new(move || {
            if let Some(store) = store.upgrade() {
                store
                    .observers
                    .borrow_mut()
                    .retain(|(observer, _)| *observer != id);
            }
        })))
    }

    fn notify(&self) {
        {
            let value = self.0.value.borrow();
            self.0.subscribers.borrow_mut().retain(|_, subscriber| {
                let Some(consumer) = subscriber.consumer.upgrade() else {
                    return false;
                };
                if (subscriber.should_notify)(value.as_ref()) {
                    consumer.mark_needs_rebuild();
                }
                true
            });
        }
        if !self.0.observers.borrow().is_empty() {
            let snapshot = self.read();
            for (_, observer) in self.0.observers.borrow_mut().iter_mut() {
                observer(&snapshot);
            }
        }
    }

    fn add_subscriber(
        &self,
        consumer: &Rc<BuildConsumer>,
        window: &WindowHandle,
        should_notify: impl FnMut(&T) -> bool + 'static,
    ) {
        let id = self.0.next_subscriber.get();
        self.0.next_subscriber.set(id.wrapping_add(1));
        let window = window.clone();
        let mut should_notify = should_notify;
        self.0.subscribers.borrow_mut().insert(
            id,
            Subscriber {
                consumer: Rc::downgrade(consumer),
                should_notify: Box::new(move |value| {
                    let notify = should_notify(value);
                    if notify {
                        window.request_redraw();
                    }
                    notify
                }),
            },
        );
        let store = Rc::downgrade(&self.0);
        consumer.add_cleanup(move || {
            if let Some(store) = store.upgrade() {
                store.subscribers.borrow_mut().remove(&id);
            }
        });
    }

    fn subscribe_watch(&self, consumer: &Rc<BuildConsumer>, window: &WindowHandle) {
        let identity = Rc::as_ptr(&self.0) as usize;
        if consumer.register_dependency(identity) {
            self.add_subscriber(consumer, window, |_| true);
        }
    }

    fn subscribe_selector<R: PartialEq + 'static>(
        &self,
        consumer: &Rc<BuildConsumer>,
        window: &WindowHandle,
        selector: impl Fn(&T) -> R + 'static,
    ) {
        let mut selected = selector(self.0.value.borrow().as_ref());
        self.add_subscriber(consumer, window, move |value| {
            let next = selector(value);
            if next == selected {
                false
            } else {
                selected = next;
                true
            }
        });
    }

    #[cfg(test)]
    fn subscriber_count(&self) -> usize {
        self.0.subscribers.borrow().len()
    }
}

struct Provided<T>(ProviderHandle<T>);

impl<T> Clone for Provided<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

struct StoreDispatcher<A>(Rc<dyn Fn(A)>);
type StoreReducer<T, A> = dyn Fn(&mut T, A);

impl<A> Clone for StoreDispatcher<A> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

/// Accesses providers from a [`BuildContext`].
///
/// [`read`](ProviderContext::read) observes a value without registering a
/// dependency, while [`watch`](ProviderContext::watch) rebuilds the current
/// widget after any update. Use [`select`](ProviderContext::select) when only a
/// projection should trigger rebuilding.
pub trait ProviderContext {
    /// Returns a snapshot of the nearest provided `T`, without subscribing, if
    /// one exists.
    fn try_read<T: 'static>(&self) -> Option<Snapshot<T>>;

    /// Returns a snapshot of the nearest provided `T` without subscribing.
    ///
    /// # Panics
    ///
    /// Panics when no matching provider is in the current widget scope.
    fn read<T: 'static>(&self) -> Snapshot<T>;

    /// Returns a copy of the nearest provided `T` without subscribing.
    ///
    /// # Panics
    ///
    /// Panics when no matching provider is in the current widget scope.
    fn copied<T: Copy + 'static>(&self) -> T {
        *self.read::<T>()
    }

    /// Return a copy of `Option<T>` without subscribing
    fn try_copied<T: Copy + 'static>(&self) -> Option<T> {
        self.try_read::<T>().map(|snapshot| *snapshot)
    }

    /// Returns and subscribes to the nearest provided `T`, if one exists.
    ///
    /// # Panics
    ///
    /// Panics when called outside a widget build while a matching provider
    /// exists.
    fn try_watch<T: 'static>(&self) -> Option<Snapshot<T>>;

    /// Returns and subscribes to the nearest provided `T`.
    ///
    /// # Panics
    ///
    /// Panics when no matching provider exists or when called outside a widget
    /// build.
    fn watch<T: 'static>(&self) -> Snapshot<T>;

    /// Returns a projection and rebuilds the current widget only when that
    /// projection changes.
    fn select<T: 'static, R: PartialEq + 'static>(&self, selector: impl Fn(&T) -> R + 'static)
    -> R;

    /// Mutates the nearest provided `T` and notifies its subscribers.
    fn update<T: Clone + 'static>(&self, mutation: impl FnOnce(&mut T));

    /// Dispatches an action to the nearest store accepting `A`.
    fn dispatch<A: 'static>(&self, action: A);
}

impl ProviderContext for BuildContext<'_> {
    fn try_read<T: 'static>(&self) -> Option<Snapshot<T>> {
        self.get_state::<Provided<T>>()
            .map(|provided| provided.0.read())
    }

    #[track_caller]
    fn read<T: 'static>(&self) -> Snapshot<T> {
        let Some(snapshot) = self.try_read::<T>() else {
            panic!(
                "No provider for `{}` found in the current widget scope",
                type_name::<T>()
            )
        };
        snapshot
    }

    #[track_caller]
    fn try_watch<T: 'static>(&self) -> Option<Snapshot<T>> {
        let provided = self.get_state_untracked::<Provided<T>>()?;
        let Some(consumer) = self.current_build_consumer() else {
            #[cfg(feature = "portable-guest")]
            if self.is_portable() {
                return Some(provided.0.read());
            }
            panic!(
                "watch::<{}>() must be called while building a widget",
                type_name::<T>()
            )
        };
        provided.0.subscribe_watch(&consumer, &self.window);
        Some(provided.0.read())
    }
    #[track_caller]
    fn watch<T: 'static>(&self) -> Snapshot<T> {
        let Some(snapshot) = self.try_watch::<T>() else {
            panic!(
                "No provider for `{}` found in the current widget scope",
                type_name::<T>()
            )
        };
        snapshot
    }

    #[track_caller]
    fn select<T: 'static, R: PartialEq + 'static>(
        &self,
        selector: impl Fn(&T) -> R + 'static,
    ) -> R {
        let Some(provided) = self.get_state_untracked::<Provided<T>>() else {
            panic!(
                "No provider for `{}` found in the current widget scope",
                type_name::<T>()
            )
        };
        let Some(consumer) = self.current_build_consumer() else {
            #[cfg(feature = "portable-guest")]
            if self.is_portable() {
                return selector(&provided.0.read());
            }
            panic!(
                "select::<{}>() must be called while building a widget",
                type_name::<T>()
            )
        };
        let selected = selector(&provided.0.read());
        provided
            .0
            .subscribe_selector(&consumer, &self.window, selector);
        selected
    }

    #[track_caller]
    fn update<T: Clone + 'static>(&self, mutation: impl FnOnce(&mut T)) {
        let Some(provided) = self.get_state::<Provided<T>>() else {
            panic!(
                "No provider for `{}` found in the current widget scope",
                type_name::<T>()
            )
        };
        provided.0.update(mutation);
    }

    #[track_caller]
    fn dispatch<A: 'static>(&self, action: A) {
        let Some(dispatcher) = self.get_state::<StoreDispatcher<A>>() else {
            panic!(
                "No store accepting `{}` found in the current widget scope",
                type_name::<A>()
            )
        };
        dispatcher.0(action);
    }
}

/// Installs a provider handle in the guest ambient context for one lowering
/// operation.
///
/// This hidden bridge is used by reflected scopes such as `AnimatedTheme` that
/// emit their own AWIR node instead of nesting a second provider node. Native
/// subscriptions and handles remain outside the serialized document.
#[doc(hidden)]
#[cfg(feature = "portable-guest")]
pub fn with_portable_provider<T: 'static, R>(
    context: &mut aimer_widget::portable::PortableBuildContext,
    handle: ProviderHandle<T>,
    callback: impl FnOnce(&mut aimer_widget::portable::PortableBuildContext) -> R,
) -> R {
    context.with_state(Provided(handle), callback)
}

/// Provides one state value to a descendant widget subtree.
///
/// Construct a provider with [`Provider::new`], configure its initializer with
/// [`Provider::create`], and attach the subtree with [`Provider::child`].
/// Descendants access the value through [`ProviderContext`] or
/// [`ProviderHandle`].
#[derive(aimer_widget::PortableWidget)]
#[portable_widget(id = "aimer_provider::Provider", schema_only, manual_lowering)]
pub struct Provider<T, W = RequiredChild> {
    #[portable_skip]
    create: Option<Rc<dyn Fn() -> T>>,
    #[portable_skip]
    handle: Option<ProviderHandle<T>>,
    #[portable_skip]
    portable_codec: Option<PortableProviderCodec<T>>,
    #[portable_child]
    child: ChildBuilder,
    #[portable_skip]
    marker: PhantomData<W>,
}

/// An alias for [`Provider`] used when the provided value is updated directly.
pub type NotifierProvider<T, W = RequiredChild> = Provider<T, W>;

impl<T> Provider<T> {
    /// Creates an unconfigured provider.
    #[inline]
    pub fn new() -> Self {
        Self {
            create: None,
            handle: None,
            portable_codec: None,
            child: ChildBuilder::required(),
            marker: PhantomData,
        }
    }
}

impl<T> Default for Provider<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T, W> Provider<T, W> {
    /// Sets the initializer called once when the provider state is created.
    #[inline]
    pub fn create(mut self, create: impl Fn() -> T + 'static) -> Self {
        self.create = Some(Rc::new(create));
        self
    }

    /// Uses an existing handle instead of creating a new provider value.
    #[inline]
    pub fn handle(mut self, handle: ProviderHandle<T>) -> Self {
        self.handle = Some(handle);
        self
    }

    /// Supplies the stable bounded codec used when this provider is lowered
    /// into portable Widget IR.
    ///
    /// Providers without a codec remain valid native widgets, but a portable
    /// guest build rejects them with a source-annotated diagnostic. This keeps
    /// arbitrary Rust values and native-only resources out of the guest ABI.
    #[inline]
    pub fn portable_codec(mut self, codec: PortableProviderCodec<T>) -> Self {
        self.portable_codec = Some(codec);
        self
    }

    /// Attaches the descendant widget subtree and produces a valid widget.
    ///
    /// The child is stored as a [`ChildBuilder`], so the provider can build the
    /// same subtree again on every one of its rebuilds while keeping the key
    /// and the debug name the child was written with.
    #[inline]
    pub fn child<C: Widget + 'static>(self, child: C) -> Provider<T, C> {
        Provider {
            create: self.create,
            handle: self.handle,
            portable_codec: self.portable_codec,
            child: ChildBuilder::from_widget(child),
            marker: PhantomData,
        }
    }

    /// Attaches the descendant subtree and type-erases the completed provider
    /// widget.
    ///
    /// This is equivalent to calling [`Provider::child`] followed by
    /// [`Widget::boxed`]. Use it when different code paths must return one
    /// [`AnyWidget`] type.
    #[inline]
    pub fn box_child<C: Widget + 'static>(self, child: C) -> AnyWidget
    where
        T: 'static,
    {
        self.child(child).boxed()
    }
}

#[doc(hidden)]
pub struct ProviderState<T, W> {
    handle: ProviderHandle<T>,
    child: ChildBuilder,
    // The child is erased, so `W` survives here only to keep one `State` impl
    // per provider type; without it `ProviderState<T>` would implement
    // `State<Provider<T, W>>` for every `W` and no call could be inferred.
    marker: PhantomData<W>,
}

impl<T: 'static, W: Widget + 'static> StatefulWidget for Provider<T, W> {
    type State = ProviderState<T, W>;

    fn create_state(self) -> Self::State {
        let handle = self.handle.clone().unwrap_or_else(|| {
            let create = self.create.as_ref().unwrap_or_else(|| {
                panic!(
                    "Provider::<{}>::create or handle must be called before child",
                    type_name::<T>()
                )
            });
            ProviderHandle::new(create())
        });
        ProviderState {
            handle,
            child: self.child.clone(),
            marker: PhantomData,
        }
    }
}

impl<T: 'static, W: Widget + 'static> State<Provider<T, W>> for ProviderState<T, W> {
    fn init_state(&mut self, _updater: StateUpdater<Self>) {}

    fn adopt_config_from(&mut self, new: Self) {
        self.child = new.child;
    }

    fn build(&self, _ctx: &BuildContext) -> impl Widget {
        ProviderScope {
            handle: self.handle.clone(),
            child: self.child.clone(),
        }
    }
}

impl<T: 'static, W: Widget + 'static> Widget for Provider<T, W> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name(self, ctx, type_name::<Provider<T>>(), None)
            .0
            .boxed()
    }

    fn debug_name(&self) -> &'static str {
        type_name::<Provider<T>>()
    }
}

#[cfg(not(feature = "portable-guest"))]
impl<T: 'static, W: Widget + 'static> aimer_widget::PortableWidget for Provider<T, W> {}

#[cfg(feature = "portable-guest")]
impl<T: 'static, W: Widget + 'static> aimer_widget::PortableWidget for Provider<T, W> {
    fn to_portable_node(
        self,
        ctx: &mut aimer_widget::portable::PortableBuildContext,
        source: aimer_widget::portable::SourceFingerprint,
    ) -> Result<aimer_widget::portable::PortableNodeId, aimer_widget::portable::PortableBuildError>
    {
        let Some(codec) = self.portable_codec else {
            return Err(
                aimer_widget::portable::PortableBuildError::ProviderEncoding {
                    provider: type_name::<Provider<T, W>>(),
                    value: type_name::<T>(),
                    source,
                    message: "no stable portable provider codec was registered".to_owned(),
                },
            );
        };
        let handle = match self.handle {
            Some(handle) => handle,
            None => {
                let Some(create) = self.create.as_ref() else {
                    return Err(
                        aimer_widget::portable::PortableBuildError::ProviderEncoding {
                            provider: type_name::<Provider<T, W>>(),
                            value: type_name::<T>(),
                            source,
                            message: "Provider::create or Provider::handle is required".to_owned(),
                        },
                    );
                };
                ProviderHandle::new(create())
            }
        };
        let snapshot = handle.read();
        let bytes = codec.encode(&snapshot).map_err(|error| {
            aimer_widget::portable::PortableBuildError::ProviderEncoding {
                provider: type_name::<Provider<T, W>>(),
                value: type_name::<T>(),
                source,
                message: error.to_string(),
            }
        })?;
        if bytes.len() > codec.schema().maximum_encoded_bytes() as usize {
            return Err(
                aimer_widget::portable::PortableBuildError::ProviderEncoding {
                    provider: type_name::<Provider<T, W>>(),
                    value: type_name::<T>(),
                    source,
                    message: format!(
                        "encoded snapshot is {} bytes, above the {} byte limit",
                        bytes.len(),
                        codec.schema().maximum_encoded_bytes(),
                    ),
                },
            );
        }
        let value = ctx.push_owned_blob(bytes)?;
        let child = ctx.with_state(Provided(handle), |ctx| {
            self.child.into_portable_node(ctx, source.child(0))
        })?;
        let properties = [
            aimer_widget::portable::__anteros::WidgetProperty::new(
                aimer_widget::portable::__anteros::PROPERTY_PROVIDER_TYPE,
                aimer_widget::portable::__anteros::PropertyValue::I64(
                    codec.schema().id().value() as i64
                ),
            ),
            aimer_widget::portable::__anteros::WidgetProperty::new(
                aimer_widget::portable::__anteros::PROPERTY_PROVIDER_SCHEMA_VERSION,
                aimer_widget::portable::__anteros::PropertyValue::I64(pack_version(
                    codec.schema().version(),
                )),
            ),
            aimer_widget::portable::__anteros::WidgetProperty::new(
                aimer_widget::portable::__anteros::PROPERTY_PROVIDER_VALUE,
                value,
            ),
        ];
        let key = aimer_widget::Key::fixed(source.identity().to_bytes());
        ctx.push_node(
            aimer_widget::portable::__anteros::WIDGET_PROVIDER,
            aimer_widget::portable::__anteros::BUILTIN_WIDGET_SCHEMA_VERSION,
            Some(&key),
            source,
            &properties,
            &[child],
        )
    }
}

#[cfg(feature = "portable-guest")]
#[inline]
const fn pack_version(version: Version) -> i64 {
    (((version.major() as u64) << 32) | version.minor() as u64) as i64
}

struct ProviderScope<T> {
    handle: ProviderHandle<T>,
    child: ChildBuilder,
}

impl<T: 'static> Widget for ProviderScope<T> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let child = ctx.with_state(Provided(self.handle.clone()), |ctx| self.child.build(ctx));
        ProviderElement {
            handle: self.handle.clone(),
            child,
        }
        .boxed()
    }
}

impl<T: 'static> aimer_widget::PortableWidget for ProviderScope<T> {}

struct ProviderElement<T> {
    handle: ProviderHandle<T>,
    child: AnyElement,
}

impl<T: 'static> ProviderElement<T> {
    fn scoped<R>(&self, ctx: &BuildContext, callback: impl FnOnce(&BuildContext) -> R) -> R {
        ctx.with_state(Provided(self.handle.clone()), callback)
    }
}

impl<T: 'static> VisitorElement for ProviderElement<T> {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }

    fn debug_name(&self) -> &'static str {
        "ProviderScope"
    }
}

impl<T: 'static> Drawable for ProviderElement<T> {
    fn update(&self, ctx: &BuildContext) {
        self.scoped(ctx, |ctx| self.child.update(ctx));
    }

    fn can_paint_local_v2(&self, _ctx: &BuildContext) -> bool {
        true
    }

    fn paint_local_v2(&self, _ctx: &BuildContext) {}

    fn retained_v2_child_context<'a>(
        &self,
        ctx: &BuildContext<'a>,
        _child: &dyn Element,
    ) -> Option<BuildContext<'a>> {
        Some(ctx.fork_with_state(Provided(self.handle.clone())))
    }

    fn sync_paint_geometry(&self, ctx: &BuildContext) {
        self.scoped(ctx, |ctx| self.child.sync_paint_geometry(ctx));
    }
}

impl<T: 'static> LayoutElement for ProviderElement<T> {
    fn pos(&self) -> Option<Vec2d> {
        self.child.pos()
    }
    fn size(&self) -> Option<Size> {
        self.child.size()
    }
    fn layout(&self, ctx: &BuildContext) -> ResolvedSize {
        self.scoped(ctx, |ctx| self.child.layout(ctx))
    }
    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.scoped(ctx, |ctx| self.child.computed_size(ctx))
    }
    fn content_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.scoped(ctx, |ctx| self.child.content_size(ctx))
    }
    fn layer(&self) -> u32 {
        self.child.layer()
    }
    fn flex(&self) -> Option<f32> {
        self.child.flex()
    }
    fn get_size_from_child(&self) -> Option<Size> {
        self.child.get_size_from_child()
    }
    fn invalidate_layout(&self) {
        self.child.invalidate_layout();
    }
    fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        self.child.pos_start_end()
    }
}

impl<T: 'static> EventElement for ProviderElement<T> {
    fn event_tree_role(&self) -> EventTreeRole {
        EventTreeRole::Transparent
    }
}

impl<T: 'static> Rebuildable for ProviderElement<T> {
    fn rebuild_if_dirty(&self, ctx: &BuildContext) {
        self.scoped(ctx, |ctx| self.child.rebuild_if_dirty(ctx));
    }

    fn with_rebuild_context(&self, ctx: &BuildContext, callback: &mut dyn FnMut(&BuildContext)) {
        self.scoped(ctx, callback);
    }

    fn is_carry_state(&self) -> bool {
        true
    }

    fn mark_needs_rebuild(&self) {
        self.child.mark_needs_rebuild();
    }
}

/// Provides reducer-managed state to a descendant widget subtree.
///
/// Descendants read or watch `T` through [`ProviderContext`] and send `A`
/// values with [`ProviderContext::dispatch`].
#[derive(aimer_widget::PortableWidget)]
#[portable_widget(id = "aimer_provider::StoreProvider", schema_only)]
pub struct StoreProvider<T, A, W = RequiredChild> {
    #[portable_skip]
    create: Option<Rc<dyn Fn() -> T>>,
    #[portable_skip]
    reducer: Option<Rc<StoreReducer<T, A>>>,
    // Same shape as [`Provider`]: the child is erased into a [`ChildBuilder`],
    // `W` only marks whether one was attached, and the un-attached slot holds
    // the allocation-free [`ChildBuilder::required`] placeholder.
    #[portable_child]
    child: ChildBuilder,
    #[portable_skip]
    marker: PhantomData<W>,
}

impl<T, A> StoreProvider<T, A> {
    /// Creates an unconfigured store provider.
    #[inline]
    pub fn new() -> Self {
        Self {
            create: None,
            reducer: None,
            child: ChildBuilder::required(),
            marker: PhantomData,
        }
    }
}

impl<T, A> Default for StoreProvider<T, A> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T, A, W> StoreProvider<T, A, W> {
    /// Sets the initializer called once when the store state is created.
    #[inline]
    pub fn create(mut self, create: impl Fn() -> T + 'static) -> Self {
        self.create = Some(Rc::new(create));
        self
    }
    /// Sets the reducer used to apply dispatched actions.
    #[inline]
    pub fn reducer(mut self, reducer: impl Fn(&mut T, A) + 'static) -> Self {
        self.reducer = Some(Rc::new(reducer));
        self
    }
    /// Attaches the descendant widget subtree and produces a valid widget.
    ///
    /// The child is stored as a [`ChildBuilder`], so the store can build the
    /// same subtree again on every one of its rebuilds.
    #[inline]
    pub fn child<C: Widget + 'static>(self, child: C) -> StoreProvider<T, A, C> {
        StoreProvider {
            create: self.create,
            reducer: self.reducer,
            child: ChildBuilder::from_widget(child),
            marker: PhantomData,
        }
    }

    /// Attaches the descendant subtree and type-erases the completed store
    /// provider widget.
    ///
    /// This is equivalent to calling [`StoreProvider::child`] followed by
    /// [`Widget::boxed`]. Use it when different code paths must return one
    /// [`AnyWidget`] type.
    #[inline]
    pub fn box_child<C: Widget + 'static>(self, child: C) -> AnyWidget
    where
        T: Clone + 'static,
        A: 'static,
    {
        self.child(child).boxed()
    }
}

#[doc(hidden)]
pub struct StoreState<T, A, W> {
    handle: ProviderHandle<T>,
    reducer: Rc<StoreReducer<T, A>>,
    child: ChildBuilder,
    // See [`ProviderState`]: kept only so the `State` impl stays unique.
    marker: PhantomData<W>,
}

impl<T: Clone + 'static, A: 'static, W: Widget + 'static> StatefulWidget
    for StoreProvider<T, A, W>
{
    type State = StoreState<T, A, W>;
    fn create_state(self) -> Self::State {
        let create = self.create.as_ref().unwrap_or_else(|| {
            panic!(
                "StoreProvider::<{}>::create must be called before child",
                type_name::<T>()
            )
        });
        let reducer = self.reducer.as_ref().unwrap_or_else(|| {
            panic!(
                "StoreProvider::<{}>::reducer must be called before child",
                type_name::<T>()
            )
        });
        StoreState {
            handle: ProviderHandle::new(create()),
            reducer: reducer.clone(),
            child: self.child.clone(),
            marker: PhantomData,
        }
    }
}

impl<T: Clone + 'static, A: 'static, W: Widget + 'static> State<StoreProvider<T, A, W>>
    for StoreState<T, A, W>
{
    fn init_state(&mut self, _updater: StateUpdater<Self>) {}
    fn adopt_config_from(&mut self, new: Self) {
        self.reducer = new.reducer;
        self.child = new.child;
    }
    fn build(&self, _ctx: &BuildContext) -> impl Widget {
        StoreScope {
            handle: self.handle.clone(),
            reducer: self.reducer.clone(),
            child: self.child.clone(),
        }
    }
}

impl<T: Clone + 'static, A: 'static, W: Widget + 'static> Widget for StoreProvider<T, A, W> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name(self, ctx, type_name::<StoreProvider<T, A>>(), None)
            .0
            .boxed()
    }

    fn debug_name(&self) -> &'static str {
        type_name::<StoreProvider<T, A>>()
    }
}

struct StoreScope<T, A> {
    handle: ProviderHandle<T>,
    reducer: Rc<StoreReducer<T, A>>,
    child: ChildBuilder,
}

impl<T: Clone + 'static, A: 'static> Widget for StoreScope<T, A> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let handle = self.handle.clone();
        let reducer = self.reducer.clone();
        let dispatcher = StoreDispatcher(Rc::new(move |action| {
            let reducer = reducer.clone();
            handle.update(|state| reducer(state, action));
        }));
        let child = ctx.with_state(Provided(self.handle.clone()), |ctx| {
            ctx.with_state(dispatcher.clone(), |ctx| self.child.build(ctx))
        });
        StoreElement {
            handle: self.handle.clone(),
            dispatcher,
            child,
            marker: PhantomData,
        }
        .boxed()
    }
}

impl<T: Clone + 'static, A: 'static> aimer_widget::PortableWidget for StoreScope<T, A> {}

struct StoreElement<T, A> {
    handle: ProviderHandle<T>,
    dispatcher: StoreDispatcher<A>,
    child: AnyElement,
    marker: PhantomData<A>,
}

impl<T: 'static, A: 'static> StoreElement<T, A> {
    fn scoped<R>(&self, ctx: &BuildContext, callback: impl FnOnce(&BuildContext) -> R) -> R {
        ctx.with_state(Provided(self.handle.clone()), |ctx| {
            ctx.with_state(self.dispatcher.clone(), callback)
        })
    }
}

impl<T: 'static, A: 'static> VisitorElement for StoreElement<T, A> {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }
    fn debug_name(&self) -> &'static str {
        "StoreProviderScope"
    }
}
impl<T: 'static, A: 'static> Drawable for StoreElement<T, A> {
    fn update(&self, ctx: &BuildContext) {
        self.scoped(ctx, |ctx| self.child.update(ctx));
    }

    fn can_paint_local_v2(&self, _ctx: &BuildContext) -> bool {
        true
    }

    fn paint_local_v2(&self, _ctx: &BuildContext) {}

    fn retained_v2_child_context<'a>(
        &self,
        ctx: &BuildContext<'a>,
        _child: &dyn Element,
    ) -> Option<BuildContext<'a>> {
        Some(
            ctx.fork_with_state(Provided(self.handle.clone()))
                .fork_with_state(self.dispatcher.clone()),
        )
    }

    fn sync_paint_geometry(&self, ctx: &BuildContext) {
        self.scoped(ctx, |ctx| self.child.sync_paint_geometry(ctx));
    }
}
impl<T: 'static, A: 'static> LayoutElement for StoreElement<T, A> {
    fn pos(&self) -> Option<Vec2d> {
        self.child.pos()
    }
    fn size(&self) -> Option<Size> {
        self.child.size()
    }
    fn layout(&self, ctx: &BuildContext) -> ResolvedSize {
        self.scoped(ctx, |ctx| self.child.layout(ctx))
    }
    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.scoped(ctx, |ctx| self.child.computed_size(ctx))
    }
    fn content_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.scoped(ctx, |ctx| self.child.content_size(ctx))
    }
    fn layer(&self) -> u32 {
        self.child.layer()
    }
    fn flex(&self) -> Option<f32> {
        self.child.flex()
    }
    fn get_size_from_child(&self) -> Option<Size> {
        self.child.get_size_from_child()
    }
    fn invalidate_layout(&self) {
        self.child.invalidate_layout();
    }
    fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        self.child.pos_start_end()
    }
}
impl<T: 'static, A: 'static> EventElement for StoreElement<T, A> {
    fn event_tree_role(&self) -> EventTreeRole {
        EventTreeRole::Transparent
    }
}
impl<T: 'static, A: 'static> Rebuildable for StoreElement<T, A> {
    fn rebuild_if_dirty(&self, ctx: &BuildContext) {
        self.scoped(ctx, |ctx| self.child.rebuild_if_dirty(ctx));
    }

    fn with_rebuild_context(&self, ctx: &BuildContext, callback: &mut dyn FnMut(&BuildContext)) {
        self.scoped(ctx, callback);
    }

    fn is_carry_state(&self) -> bool {
        true
    }

    fn mark_needs_rebuild(&self) {
        self.child.mark_needs_rebuild();
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::rc::Rc;

    use aimer_utils::PanicSite;
    use aimer_widget::base::{BuildConsumer, BuildContext, WindowHandle};
    use aimer_widget::{
        Drawable, Element, EventElement, EventTreeRole, LayoutElement, Rebuildable, State,
        StateUpdater, StatefulElement, StatefulWidget, StatelessElement, VisitorElement, Widget,
    };

    use super::*;

    #[derive(Clone, Debug, Default, PartialEq)]
    struct Counter {
        count: usize,
        label: &'static str,
    }

    struct NonClone {
        value: usize,
    }

    struct Leaf;

    impl VisitorElement for Leaf {
        fn debug_name(&self) -> &'static str {
            "Leaf"
        }
    }
    impl Drawable for Leaf {
        fn update(&self, _context: &BuildContext) {}
    }
    impl EventElement for Leaf {}
    impl LayoutElement for Leaf {}
    impl Rebuildable for Leaf {}

    #[test]
    fn an_observer_sees_every_update_from_any_clone_of_the_handle() {
        let handle = ProviderHandle::new(Counter::default());
        let seen = Rc::new(RefCell::new(Vec::new()));

        let log = seen.clone();
        let observation = handle.observe(move |snapshot| log.borrow_mut().push(snapshot.count));

        handle.update(|counter| counter.count = 1);
        // A widget reaches the provider through a clone; it must be observed too.
        let from_context = handle.clone();
        from_context.dispatch(5, |counter, by| counter.count += by);
        assert_eq!(*seen.borrow(), [1, 6]);

        drop(observation);
        handle.update(|counter| counter.count = 100);
        assert_eq!(
            *seen.borrow(),
            [1, 6],
            "a dropped observation must stay silent"
        );
    }

    #[test]
    fn observers_are_independent_and_unregister_individually() {
        let handle = ProviderHandle::new(Counter::default());
        let (first, second) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(0)));

        let counted = first.clone();
        let first_observation = handle.observe(move |_| counted.set(counted.get() + 1));
        let counted = second.clone();
        let _second_observation = handle.observe(move |_| counted.set(counted.get() + 1));

        handle.update(|counter| counter.count += 1);
        drop(first_observation);
        handle.update(|counter| counter.count += 1);

        assert_eq!((first.get(), second.get()), (1, 2));
    }

    #[test]
    fn provider_wrappers_are_transparent_to_event_routing() {
        let provider = ProviderElement {
            handle: ProviderHandle::new(Counter::default()),
            child: Leaf.boxed(),
        };
        let store = StoreElement {
            handle: ProviderHandle::new(Counter::default()),
            dispatcher: StoreDispatcher(Rc::new(|_: ()| {})),
            child: Leaf.boxed(),
            marker: PhantomData,
        };

        assert_eq!(provider.event_tree_role(), EventTreeRole::Transparent);
        assert_eq!(store.event_tree_role(), EventTreeRole::Transparent);
    }

    struct ReadingWidget {
        observed: Rc<Cell<usize>>,
    }

    impl Widget for ReadingWidget {
        fn to_element(self, context: &BuildContext) -> AnyElement {
            self.observed
                .set(ProviderContext::read::<Counter>(context).count);
            Leaf.boxed()
        }
    }

    impl aimer_widget::PortableWidget for ReadingWidget {}

    struct LeafWidget;

    impl Widget for LeafWidget {
        fn to_element(self, _context: &BuildContext) -> AnyElement {
            Leaf.boxed()
        }
    }

    impl aimer_widget::PortableWidget for LeafWidget {}

    struct PortableLeaf;

    impl Widget for PortableLeaf {
        fn to_element(self, _context: &BuildContext) -> AnyElement {
            Leaf.boxed()
        }
    }

    impl aimer_widget::PortableWidget for PortableLeaf {
        #[cfg(feature = "portable-guest")]
        fn to_portable_node(
            self,
            context: &mut aimer_widget::portable::PortableBuildContext,
            source: aimer_widget::portable::SourceFingerprint,
        ) -> Result<
            aimer_widget::portable::PortableNodeId,
            aimer_widget::portable::PortableBuildError,
        > {
            context.push_node(
                aimer_widget::portable::__anteros::WIDGET_SIZED_BOX,
                aimer_widget::portable::__anteros::Version::new(1, 0),
                None,
                source,
                &[],
                &[],
            )
        }
    }

    /// Counts how many times the provider asked for its subtree, and records
    /// the value that was in scope on every one of those builds.
    struct ProbeWidget {
        builds: Rc<Cell<usize>>,
        observed: Rc<RefCell<Vec<usize>>>,
    }

    impl Widget for ProbeWidget {
        fn to_element(self, context: &BuildContext) -> AnyElement {
            self.builds.set(self.builds.get() + 1);
            self.observed
                .borrow_mut()
                .push(ProviderContext::read::<Counter>(context).count);
            Leaf.boxed()
        }

        fn key(&self) -> Option<aimer_widget::Key> {
            Some(aimer_widget::Key::Static("probe"))
        }

        fn debug_name(&self) -> &'static str {
            "ProbeWidget"
        }
    }

    impl aimer_widget::PortableWidget for ProbeWidget {}

    /// A child that rebuilds itself, the way a derived widget does.
    ///
    /// It counts its conversions apart from its builds, because those are now
    /// two different things: a provider that rebuilds reuses its child's element
    /// (one conversion) and asks the subtree inside it to build again (one build
    /// per rebuild), which is how a new provided value reaches it.
    struct RebuildingProbe {
        conversions: Rc<Cell<usize>>,
        observed: Rc<RefCell<Vec<usize>>>,
    }

    impl Widget for RebuildingProbe {
        fn to_element(self, context: &BuildContext) -> AnyElement {
            self.conversions.set(self.conversions.get() + 1);
            let observed = self.observed;
            aimer_widget::Element::boxed(aimer_widget::StatelessElement::from_builder(
                context,
                move |ctx| {
                    observed
                        .borrow_mut()
                        .push(ProviderContext::read::<Counter>(ctx).count);
                    let _ = ctx;
                    Leaf.boxed()
                },
                None,
                "RebuildingProbe",
            ))
        }

        fn debug_name(&self) -> &'static str {
            "RebuildingProbe"
        }
    }

    impl aimer_widget::PortableWidget for RebuildingProbe {}

    struct WatchingWidget {
        builds: Rc<Cell<usize>>,
        handle: Rc<RefCell<Option<ProviderHandle<Counter>>>>,
        select_count: bool,
    }

    struct WatchingState {
        builds: Rc<Cell<usize>>,
        handle: Rc<RefCell<Option<ProviderHandle<Counter>>>>,
        select_count: bool,
    }

    struct MultiSelectorWidget {
        builds: Rc<Cell<usize>>,
        handle: Rc<RefCell<Option<ProviderHandle<Counter>>>>,
    }

    struct MultiSelectorState {
        builds: Rc<Cell<usize>>,
        handle: Rc<RefCell<Option<ProviderHandle<Counter>>>>,
    }

    impl StatefulWidget for WatchingWidget {
        type State = WatchingState;

        fn create_state(self) -> Self::State {
            WatchingState {
                builds: self.builds.clone(),
                handle: self.handle.clone(),
                select_count: self.select_count,
            }
        }
    }

    impl State<WatchingWidget> for WatchingState {
        fn init_state(&mut self, _updater: StateUpdater<Self>) {}

        fn build(&self, context: &BuildContext) -> impl Widget {
            self.builds.set(self.builds.get() + 1);
            if self.select_count {
                ProviderContext::select::<Counter, usize>(context, |counter| counter.count);
            } else {
                ProviderContext::watch::<Counter>(context);
            }
            *self.handle.borrow_mut() = Some(ProviderHandle::of(context));
            LeafWidget
        }
    }

    impl Widget for WatchingWidget {
        fn to_element(self, context: &BuildContext) -> AnyElement {
            StatefulElement::new_with_name(self, context, "WatchingWidget", None)
                .0
                .boxed()
        }
    }

    impl aimer_widget::PortableWidget for WatchingWidget {}

    struct KeyedProviderConsumer;

    struct KeyedProviderConsumerState;

    impl StatefulWidget for KeyedProviderConsumer {
        type State = KeyedProviderConsumerState;

        fn create_state(self) -> Self::State {
            KeyedProviderConsumerState
        }
    }

    impl State<KeyedProviderConsumer> for KeyedProviderConsumerState {
        fn init_state(&mut self, _updater: StateUpdater<Self>) {}

        fn build(&self, context: &BuildContext) -> impl Widget {
            let _ = ProviderContext::read::<Counter>(context);
            LeafWidget
        }
    }

    impl Widget for KeyedProviderConsumer {
        fn to_element(self, context: &BuildContext) -> AnyElement {
            StatefulElement::new_with_name(
                self,
                context,
                "KeyedProviderConsumer",
                Some(aimer_widget::Key::Static("keyed-provider-consumer")),
            )
            .0
            .boxed()
        }
    }

    impl aimer_widget::PortableWidget for KeyedProviderConsumer {}

    impl StatefulWidget for MultiSelectorWidget {
        type State = MultiSelectorState;

        fn create_state(self) -> Self::State {
            MultiSelectorState {
                builds: self.builds.clone(),
                handle: self.handle.clone(),
            }
        }
    }

    impl State<MultiSelectorWidget> for MultiSelectorState {
        fn init_state(&mut self, _updater: StateUpdater<Self>) {}

        fn build(&self, context: &BuildContext) -> impl Widget {
            self.builds.set(self.builds.get() + 1);
            ProviderContext::select::<Counter, usize>(context, |counter| counter.count);
            ProviderContext::select::<Counter, &'static str>(context, |counter| counter.label);
            *self.handle.borrow_mut() = Some(ProviderHandle::of(context));
            LeafWidget
        }
    }

    impl Widget for MultiSelectorWidget {
        fn to_element(self, context: &BuildContext) -> AnyElement {
            StatefulElement::new_with_name(self, context, "MultiSelectorWidget", None)
                .0
                .boxed()
        }
    }

    impl aimer_widget::PortableWidget for MultiSelectorWidget {}

    #[cfg(not(target_arch = "wasm32"))]
    fn dummy_async_handle() -> tokio::runtime::Handle {
        use std::sync::OnceLock;

        static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
        let runtime = RUNTIME.get_or_init(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
        });
        let _guard = runtime.enter();
        tokio::runtime::Handle::current()
    }

    fn context() -> BuildContext<'static> {
        let canvas = {
            let inner = Box::leak(Box::new(aimer_canvas::InnerCanvas::new()));
            aimer_canvas::FrameCanvas::new(inner)
        };
        BuildContext::new(
            canvas,
            Default::default(),
            1.0,
            Default::default(),
            Default::default(),
            WindowHandle::headless(Default::default(), 1.0),
            #[cfg(not(target_arch = "wasm32"))]
            dummy_async_handle(),
        )
    }

    fn contains_error_element(element: &dyn Element) -> bool {
        if element.debug_name() == "ErrorWidget" {
            return true;
        }

        let mut found = false;
        element.visit_children(&mut |child| found |= contains_error_element(child));
        found
    }

    #[cfg(feature = "portable-guest")]
    fn portable_context() -> aimer_widget::portable::PortableBuildContext {
        aimer_widget::portable::PortableBuildContext::new(
            1,
            1,
            aimer_widget::portable::PortableWidgetLimits::new(8, 16, 16, 16, 64, 2_048)
                .with_max_blob_bytes(128),
            aimer_widget::portable::PortableLimits::new(8, 16, 64, 128, 1_024),
        )
        .unwrap()
    }

    #[cfg(feature = "portable-guest")]
    fn portable_codec() -> PortableProviderCodec<u32> {
        fn encode(value: &u32) -> Result<Vec<u8>, PortableProviderCodecError> {
            Ok(value.to_le_bytes().to_vec())
        }
        fn decode(bytes: &[u8], version: Version) -> Result<u32, PortableProviderCodecError> {
            if version != Version::new(1, 0) || bytes.len() != 4 {
                return Err(PortableProviderCodecError::new("invalid u32 snapshot"));
            }
            Ok(u32::from_le_bytes(bytes.try_into().unwrap()))
        }
        PortableProviderCodec::new(
            ValueSchemaMetadata::from_canonical_name(
                "aimer.test:aimer_provider::u32",
                Version::new(1, 0),
                4,
            ),
            encode,
            decode,
        )
    }

    #[cfg(feature = "portable-guest")]
    #[test]
    fn portable_context_reads_and_restores_nested_provider_values() {
        let context = BuildContext::portable();
        let outer = ProviderHandle::new(1_u32);
        let inner = ProviderHandle::new(2_u32);

        context.with_state(Provided(outer), |context| {
            assert_eq!(*ProviderContext::read::<u32>(context), 1);
            assert_eq!(*ProviderContext::watch::<u32>(context), 1);
            assert_eq!(
                ProviderContext::select::<u32, _>(context, |value| *value),
                1
            );
            context.with_state(Provided(inner), |context| {
                assert_eq!(*ProviderContext::read::<u32>(context), 2);
            });
            assert_eq!(*ProviderContext::read::<u32>(context), 1);
        });
        assert!(ProviderContext::try_read::<u32>(&context).is_none());
    }

    #[cfg(feature = "portable-guest")]
    #[test]
    fn provider_portable_lowering_emits_a_versioned_snapshot_and_child() {
        let mut context = portable_context();
        let provider = Provider::new()
            .handle(ProviderHandle::new(7_u32))
            .portable_codec(portable_codec())
            .child(PortableLeaf);
        let root = aimer_widget::PortableWidget::to_portable_node(
            provider,
            &mut context,
            aimer_widget::portable::SourceFingerprint::new(
                aimer_widget::portable::StableId128::from_bytes([7; 16]),
            ),
        )
        .unwrap();
        let document = context.finish_document(root).unwrap();
        let limits = document.model_limits();
        let image = document.encode().unwrap();
        let view =
            aimer_widget::portable::__anteros::WidgetDocumentView::decode(&image, limits).unwrap();
        let node = view.node(view.root_node()).unwrap();

        assert_eq!(
            node.widget_type(),
            aimer_widget::portable::__anteros::WIDGET_PROVIDER
        );
        assert_eq!(node.children().count(), 1);
        assert!(node.properties().any(|property| {
            property.property_id() == aimer_widget::portable::__anteros::PROPERTY_PROVIDER_VALUE
                && property.value() == aimer_widget::portable::__anteros::PropertyValue::BlobRef(0)
        }));
        assert_eq!(view.blob(0), Some(7_u32.to_le_bytes().as_slice()));
    }

    #[cfg(feature = "portable-guest")]
    #[test]
    fn provider_without_a_codec_is_rejected_at_its_guest_source() {
        let mut context = portable_context();
        let error = aimer_widget::PortableWidget::to_portable_node(
            Provider::new()
                .handle(ProviderHandle::new(7_u32))
                .child(PortableLeaf),
            &mut context,
            aimer_widget::portable::SourceFingerprint::new(
                aimer_widget::portable::StableId128::from_bytes([8; 16]),
            ),
        )
        .expect_err("a provider without a stable codec must not cross the guest boundary");

        assert!(matches!(
            error,
            aimer_widget::portable::PortableBuildError::ProviderEncoding { .. }
        ));
    }

    #[test]
    fn update_mutates_the_value_and_notifies_a_watcher() {
        let handle = ProviderHandle::new(Counter::default());
        let dirty = Rc::new(Cell::new(false));
        let consumer = BuildConsumer::new(dirty.clone());
        let window = WindowHandle::headless(Default::default(), 1.0);
        handle.subscribe_watch(&consumer, &window);

        handle.update(|counter| counter.count += 1);

        assert_eq!(handle.read().count, 1);
        assert!(dirty.get());
        assert!(window.take_redraw_request());
    }

    #[test]
    fn a_missing_provider_is_blamed_on_the_calling_line() {
        let context = context();

        let watch = PanicSite::watch();
        let expected_line = line!() + 2;
        let payload = catch_unwind(AssertUnwindSafe(|| {
            let _ = ProviderContext::read::<Counter>(&context);
        }))
        .expect_err("reading a missing provider should panic");
        let site = watch
            .take_site()
            .expect("the panic site should be recorded");
        let message = payload
            .downcast_ref::<String>()
            .expect("the diagnostic should be an owned message");

        assert_eq!(site.file(), file!());
        assert_eq!(site.line(), expected_line);
        assert_eq!(message.lines().count(), 1, "{message}");
        assert!(message.contains("Counter"), "{message}");
    }

    #[test]
    fn watching_outside_a_build_is_blamed_on_the_calling_line() {
        let context = context();
        let handle = ProviderHandle::new(Counter::default());

        context.with_state(Provided(handle), |context| {
            let watch = PanicSite::watch();
            let expected_line = line!() + 2;
            let failed = catch_unwind(AssertUnwindSafe(|| {
                let _ = ProviderContext::watch::<Counter>(context);
            }))
            .is_err();
            let site = watch
                .take_site()
                .expect("the panic site should be recorded");

            assert!(failed);
            assert_eq!(site.file(), file!());
            assert_eq!(site.line(), expected_line);
        });
    }

    #[test]
    fn a_missing_store_is_blamed_on_the_calling_line() {
        let context = context();

        let watch = PanicSite::watch();
        let expected_line = line!() + 2;
        let failed = catch_unwind(AssertUnwindSafe(|| {
            ProviderContext::dispatch(&context, 7_u32);
        }))
        .is_err();
        let site = watch
            .take_site()
            .expect("the panic site should be recorded");

        assert!(failed);
        assert_eq!(site.file(), file!());
        assert_eq!(site.line(), expected_line);
    }

    #[test]
    fn read_returns_a_snapshot_without_requiring_the_value_to_be_clone() {
        let context = context();
        let handle = ProviderHandle::new(NonClone { value: 7 });

        context.with_state(Provided(handle), |context| {
            let snapshot: Snapshot<NonClone> = ProviderContext::read(context);

            assert_eq!(snapshot.value, 7);
        });
    }

    #[test]
    fn copied_returns_a_copy_without_subscribing() {
        let context = context();
        let handle = ProviderHandle::new(7_u32);

        context.with_state(Provided(handle.clone()), |context| {
            let value = ProviderContext::copied::<u32>(context);

            assert_eq!(value, 7);
            assert_eq!(handle.subscriber_count(), 0);
        });
    }

    #[test]
    fn watch_returns_a_snapshot_without_requiring_the_value_to_be_clone() {
        let context = context();
        let handle = ProviderHandle::new(NonClone { value: 7 });
        let consumer = BuildConsumer::new(Rc::new(Cell::new(false)));

        context.with_state(Provided(handle), |context| {
            context.with_build_consumer(consumer, |context| {
                let snapshot: Snapshot<NonClone> = ProviderContext::watch(context);

                assert_eq!(snapshot.value, 7);
            });
        });
    }

    #[test]
    fn snapshot_keeps_the_version_that_was_read_before_an_update() {
        let handle = ProviderHandle::new(Counter::default());
        let before = handle.read();

        handle.update(|counter| counter.count = 1);
        let after = handle.read();

        assert_eq!(before.count, 0);
        assert_eq!(after.count, 1);
    }

    #[test]
    fn repeated_watch_in_one_build_is_deduplicated() {
        let handle = ProviderHandle::new(Counter::default());
        let consumer = BuildConsumer::new(Rc::new(Cell::new(false)));
        let window = WindowHandle::headless(Default::default(), 1.0);

        handle.subscribe_watch(&consumer, &window);
        handle.subscribe_watch(&consumer, &window);

        assert_eq!(handle.subscriber_count(), 1);
    }

    #[test]
    fn selector_only_notifies_when_its_projection_changes() {
        let handle = ProviderHandle::new(Counter::default());
        let dirty = Rc::new(Cell::new(false));
        let consumer = BuildConsumer::new(dirty.clone());
        let window = WindowHandle::headless(Default::default(), 1.0);
        handle.subscribe_selector(&consumer, &window, |counter| counter.count);

        handle.update(|counter| counter.label = "changed");
        assert!(!dirty.get());

        handle.update(|counter| counter.count = 1);
        assert!(dirty.get());
    }

    #[test]
    fn reducer_dispatch_uses_the_same_notification_path() {
        #[derive(Clone, Copy)]
        enum Action {
            Increment,
        }

        let handle = ProviderHandle::new(Counter::default());
        let dirty = Rc::new(Cell::new(false));
        let consumer = BuildConsumer::new(dirty.clone());
        let window = WindowHandle::headless(Default::default(), 1.0);
        handle.subscribe_watch(&consumer, &window);

        handle.dispatch(Action::Increment, |counter, action| match action {
            Action::Increment => counter.count += 1,
        });

        assert_eq!(handle.read().count, 1);
        assert!(dirty.get());
    }

    #[test]
    fn lookup_uses_the_nearest_scope_and_restores_the_outer_provider() {
        let context = context();
        let outer = ProviderHandle::new(Counter {
            count: 1,
            label: "outer",
        });
        let inner = ProviderHandle::new(Counter {
            count: 2,
            label: "inner",
        });

        context.with_state(Provided(outer), |context| {
            assert_eq!(ProviderContext::read::<Counter>(context).label, "outer");
            context.with_state(Provided(inner), |context| {
                assert_eq!(ProviderContext::read::<Counter>(context).label, "inner");
            });
            assert_eq!(ProviderContext::read::<Counter>(context).label, "outer");
        });
        assert!(ProviderContext::try_read::<Counter>(&context).is_none());
    }

    #[test]
    #[should_panic(expected = "No provider for")]
    fn required_lookup_has_a_clear_missing_provider_diagnostic() {
        ProviderContext::read::<Counter>(&context());
    }

    #[test]
    fn missing_handle_diagnostic_highlights_its_external_caller() {
        let watch = PanicSite::watch();
        let rendered = catch_unwind(|| {
            let _ = ProviderHandle::<Counter>::of(&context());
        })
        .err()
        .and_then(|_| watch.take_site())
        .expect("the panic site should be recorded")
        .to_string();

        assert!(rendered.starts_with("at "), "{rendered}");
        assert!(rendered.contains(file!()), "{rendered}");
        assert!(
            rendered.contains("ProviderHandle::<Counter>::of(&context())"),
            "{rendered}"
        );
        assert!(
            rendered.contains("^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^"),
            "{rendered}"
        );
    }

    #[test]
    fn read_does_not_subscribe_and_conditional_watch_is_cleaned_up() {
        let context = context();
        let handle = ProviderHandle::new(Counter::default());
        let consumer = BuildConsumer::new(Rc::new(Cell::new(false)));

        context.with_state(Provided(handle.clone()), |context| {
            context.with_build_consumer(consumer.clone(), |context| {
                ProviderContext::read::<Counter>(context);
                ProviderContext::watch::<Counter>(context);
            });
        });
        assert_eq!(handle.subscriber_count(), 1);

        context.with_build_consumer(consumer, |_| {});
        assert_eq!(handle.subscriber_count(), 0);
    }

    #[test]
    fn context_dispatch_resolves_the_scoped_store_by_action_type() {
        #[derive(Clone, Copy)]
        struct Increment;

        let context = context();
        let handle = ProviderHandle::new(Counter::default());
        let dispatch_handle = handle.clone();
        let dispatcher = StoreDispatcher(Rc::new(move |Increment| {
            dispatch_handle.update(|counter| counter.count += 1);
        }));

        context.with_state(dispatcher, |context| {
            ProviderContext::dispatch(context, Increment);
        });

        assert_eq!(handle.read().count, 1);
    }

    #[test]
    fn provider_widget_publishes_to_a_non_clone_child() {
        let observed = Rc::new(Cell::new(0));
        let provider = Provider::<Counter>::new()
            .create(|| Counter {
                count: 7,
                label: "provided",
            })
            .child(ReadingWidget {
                observed: observed.clone(),
            });

        let _element = provider.to_element(&context());

        assert_eq!(observed.get(), 7);
    }

    #[test]
    fn provider_supplies_its_value_on_every_build_of_the_child() {
        let conversions = Rc::new(Cell::new(0));
        let observed = Rc::new(RefCell::new(Vec::new()));
        let handle = ProviderHandle::new(Counter {
            count: 3,
            label: "provided",
        });
        let provider = Provider::<Counter>::new()
            .handle(handle.clone())
            .child(RebuildingProbe {
                conversions: conversions.clone(),
                observed: observed.clone(),
            });
        let context = context();
        let state = provider.create_state();

        let _first = state.build(&context).to_element(&context);
        handle.update(|counter| counter.count = 4);
        let second = state.build(&context).to_element(&context);
        second.rebuild_if_dirty(&context);

        assert_eq!(
            conversions.get(),
            1,
            "a rebuilt provider reuses its child's element instead of rebuilding \
             the subtree from the widget"
        );
        assert_eq!(
            observed.borrow().as_slice(),
            [3, 4],
            "every build of the child sees the value that is in scope"
        );
    }

    #[test]
    fn keyed_state_carry_reenters_provider_scope() {
        let handle = ProviderHandle::new(Counter::default());
        let initial_context = context();
        let build_handle = handle.clone();
        let root = StatelessElement::from_builder(
            &initial_context,
            move |context| {
                Provider::new()
                    .handle(build_handle.clone())
                    .child(KeyedProviderConsumer)
                    .to_element(context)
            },
            None,
            "ProviderParent",
        );

        Rebuildable::mark_needs_rebuild(&root);
        let fresh_context = context();

        // The keyed descendant is rebuilt while state is carried from the old
        // tree. ProviderElement must re-enter its scope for that build.
        root.rebuild_if_dirty(&fresh_context);

        assert!(!contains_error_element(&root));
    }

    #[test]
    fn provider_keeps_the_identity_of_its_child() {
        let provider = Provider::<Counter>::new()
            .create(Counter::default)
            .child(ProbeWidget {
                builds: Rc::new(Cell::new(0)),
                observed: Rc::new(RefCell::new(Vec::new())),
            });

        let state = provider.create_state();

        assert_eq!(
            Widget::key(&state.child),
            Some(aimer_widget::Key::Static("probe")),
            "reconciliation matches the child on its own key, not on its holder"
        );
        assert_eq!(Widget::debug_name(&state.child), "ProbeWidget");
    }

    #[test]
    fn provider_owned_value_is_dropped_with_its_element() {
        struct Droppable(Rc<Cell<usize>>);
        impl Drop for Droppable {
            fn drop(&mut self) {
                self.0.set(self.0.get() + 1);
            }
        }
        struct Child;
        impl Widget for Child {
            fn to_element(self, _context: &BuildContext) -> AnyElement {
                Leaf.boxed()
            }
        }

        impl aimer_widget::PortableWidget for Child {}

        let drops = Rc::new(Cell::new(0));
        let provider = Provider::<Droppable>::new()
            .create({
                let drops = drops.clone();
                move || Droppable(drops.clone())
            })
            .child(Child);
        let element = provider.to_element(&context());
        assert_eq!(drops.get(), 0);

        drop(element);

        assert_eq!(drops.get(), 1);
    }

    #[test]
    fn provider_handle_can_be_captured_for_later_updates() {
        let context = context();
        let handle = ProviderHandle::new(Counter::default());

        let captured = context.with_state(Provided(handle), ProviderHandle::<Counter>::of);
        captured.update(|counter| counter.count = 9);

        assert_eq!(captured.read().count, 9);
    }

    #[test]
    fn watched_descendant_rebuilds_through_the_provider_scope() {
        let builds = Rc::new(Cell::new(0));
        let handle = Rc::new(RefCell::new(None));
        let provider = Provider::<Counter>::new()
            .create(Counter::default)
            .child(WatchingWidget {
                builds: builds.clone(),
                handle: handle.clone(),
                select_count: false,
            });
        let context = context();
        let element = provider.to_element(&context);
        assert_eq!(builds.get(), 1);

        handle
            .borrow()
            .as_ref()
            .unwrap()
            .update(|counter| counter.count += 1);
        element.rebuild_if_dirty(&context);

        assert_eq!(builds.get(), 2);
    }

    #[test]
    fn root_reconciliation_preserves_the_provider_scope_for_descendant_rebuilds() {
        let builds = Rc::new(Cell::new(0));
        let handle = Rc::new(RefCell::new(None));
        let provider = Provider::<Counter>::new()
            .create(Counter::default)
            .child(WatchingWidget {
                builds: builds.clone(),
                handle,
                select_count: false,
            });
        let context = context();
        let element = provider.to_element(&context);
        let root = element
            .option_any()
            .and_then(|element| element.downcast_ref::<StatefulElement>())
            .expect("Provider should create a stateful root element");
        assert!(!root.is_dirty());

        element.mark_needs_rebuild();
        let carries_state = Cell::new(false);
        root.visit_children(&mut |child| carries_state.set(child.is_carry_state()));
        assert!(carries_state.get());
        assert!(!root.is_dirty());
        element.rebuild_if_dirty(&context);

        assert_eq!(builds.get(), 2);
    }

    #[test]
    fn selected_descendant_ignores_unrelated_updates() {
        let builds = Rc::new(Cell::new(0));
        let handle = Rc::new(RefCell::new(None));
        let provider = Provider::<Counter>::new()
            .create(Counter::default)
            .child(WatchingWidget {
                builds: builds.clone(),
                handle: handle.clone(),
                select_count: true,
            });
        let context = context();
        let element = provider.to_element(&context);

        handle
            .borrow()
            .as_ref()
            .unwrap()
            .update(|counter| counter.label = "unrelated");
        element.rebuild_if_dirty(&context);
        assert_eq!(builds.get(), 1);

        handle
            .borrow()
            .as_ref()
            .unwrap()
            .update(|counter| counter.count = 1);
        element.rebuild_if_dirty(&context);
        assert_eq!(builds.get(), 2);
    }

    #[test]
    fn multiple_selectors_from_one_consumer_track_independent_values() {
        let builds = Rc::new(Cell::new(0));
        let handle = Rc::new(RefCell::new(None));
        let provider =
            Provider::<Counter>::new()
                .create(Counter::default)
                .child(MultiSelectorWidget {
                    builds: builds.clone(),
                    handle: handle.clone(),
                });
        let context = context();
        let element = provider.to_element(&context);

        handle
            .borrow()
            .as_ref()
            .unwrap()
            .update(|counter| counter.label = "changed");
        element.rebuild_if_dirty(&context);

        assert_eq!(builds.get(), 2);
    }
}
