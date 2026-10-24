use crate::base::BuildContext;
use crate::AnyElement;
use aimer_rubick::{Rubick, UiAllocator};
use std::ops::Deref;

pub mod child_builder;
mod recovery;
pub(crate) mod state_slots;
pub mod stateful;
pub mod stateless;

/// Provides the guest-side lowering capability for a [`Widget`].
///
/// `Widget` remains a supertrait during the Phase 15 migration so existing
/// builder APIs that return `impl Widget` continue to expose this capability.
/// Implementations are explicit: handwritten widgets can retain the default
/// unsupported-widget lowering, while `#[derive(PortableWidget)]` emits the
/// reflected guest lowering for schema-declared properties, callbacks, and
/// children.
#[doc(hidden)]
pub trait PortableWidget {
    /// Consumes this widget and appends its portable Widget IR node.
    ///
    /// The `Self: Widget` bound is intentional while `Widget` retains this
    /// trait as a supertrait. It lets the default diagnostic reuse the widget's
    /// public debug name without changing the existing builder return types.
    #[cfg(feature = "portable-guest")]
    fn to_portable_node(
        self,
        ctx: &mut crate::portable::PortableBuildContext,
        source: crate::portable::SourceFingerprint,
    ) -> Result<crate::portable::PortableNodeId, crate::portable::PortableBuildError>
    where
        Self: Sized + Widget,
    {
        let widget = Widget::debug_name(&self);
        drop(self);
        Err(ctx.unsupported_widget(widget, source))
    }
}

/// A configuration value that produces exactly one element.
///
/// # Consuming conversion
///
/// [`Widget::to_element`] takes `self` by value, because a widget is the
/// short-lived side of the tree: it is created inside a `build`, converted on
/// the next line, and dropped immediately afterwards, while the [`AnyElement`]
/// it produced is retained across frames. Nothing reads the widget after the
/// conversion, so an implementation should **move** its fields into the element
/// rather than clone them — a decorated container therefore costs no allocation
/// per build, and a widget does not have to be [`Clone`] at all.
///
/// # Example
///
/// ```
/// use aimer_widget::base::BuildContext;
/// use aimer_widget::{AnyElement, PortableWidget, Widget};
///
/// // Deliberately not `Clone`: nothing copies a widget.
/// struct Label(String);
///
/// impl PortableWidget for Label {}
///
/// impl Widget for Label {
///     fn to_element(self, ctx: &BuildContext) -> AnyElement {
///         // The string is moved into the element that keeps it.
///         aimer_widget::ErrorWidget::new(&self.0).to_element(ctx)
///     }
/// }
///
/// let widget = Label(String::from("Aimer")).boxed();
/// assert!(widget.is_inline());
/// ```
pub trait Widget: PortableWidget {
    fn key(&self) -> Option<crate::key::Key> {
        None
    }

    /// Consumes this widget and produces its element.
    ///
    /// The widget is gone once this returns, so an implementation moves its
    /// fields into the element instead of cloning them.
    fn to_element(self, ctx: &BuildContext) -> AnyElement
    where
        Self: Sized;

    fn debug_name(&self) -> &'static str {
        "Unknown"
    }

    /// Erases this widget into an inline-or-heap [`AnyWidget`].
    ///
    /// Values fitting the configured `Rubick` size and alignment are embedded
    /// directly in the returned owner. Other values use one heap allocation.
    /// Despite the historical method name, allocation is therefore not
    /// guaranteed. Moving an inline owner also moves this widget.
    /// 
    /// Using [`Widget::notype`] instead because this is not meaningful
    fn boxed(self) -> AnyWidget
    where
        Self: Sized + 'static,
    {
        AnyWidget::new(self)
    }


    /// Erases this widget into an inline-or-heap [`AnyWidget`].
    ///
    /// Values fitting the configured `Rubick` size and alignment are embedded
    /// directly in the returned owner. Other values use one heap allocation.
    /// Despite the historical method name, allocation is therefore not
    /// guaranteed. Moving an inline owner also moves this widget.
    fn notype(self) -> AnyWidget where Self: Sized + 'static {
        AnyWidget::new(self)
    }

    /// Erases this widget using the supplied application UI allocator.
    ///
    /// The erased owner retains the allocator for as long as its heap payload
    /// exists, even if it is dropped outside the allocator's active scope.
    #[inline]
    fn boxed_in(self, allocator: &UiAllocator) -> AnyWidget
    where
        Self: Sized + 'static,
    {
        AnyWidget::new_in(self, allocator)
    }

    /// Returns the text content if this is a text widget.
    /// Used by the reconciliation system to update text elements in-place.
    fn text_content(&self) -> Option<&str> {
        None
    }
}

/// The object-safe half of [`Widget`].
///
/// A consuming `fn to_element(self, ..)` cannot appear in a vtable: `dyn Widget`
/// has no size, so it cannot be passed by value. The erased handle still has to
/// convert whatever it holds, which leaves one way out — move the concrete value
/// out of its storage behind the erasure, then call the concrete method on the
/// moved value. [`DynWidget::to_element_in_place`] performs that move, because
/// it is implemented for every *sized* `W: Widget` and therefore knows the type
/// `dyn Widget` has forgotten.
///
/// This trait is an implementation detail of the erasure and is never written by
/// hand: the blanket implementation below covers every widget.
#[doc(hidden)]
pub trait DynWidget: 'static {
    /// Moves the widget out of `self` and converts it.
    ///
    /// # Safety
    ///
    /// The pointee is left uninitialized and must never be read or dropped
    /// again. The only caller is [`AnyWidget::into_element`], which marks the
    /// storage vacant before this runs.
    #[cfg(not(aimer_portable_guest))]
    unsafe fn to_element_in_place(&mut self, ctx: &BuildContext) -> AnyElement;

    /// Moves the widget out of `self` and converts it to portable Widget IR.
    #[cfg(feature = "portable-guest")]
    unsafe fn to_portable_node_in_place(
        &mut self,
        ctx: &mut crate::portable::PortableBuildContext,
        source: crate::portable::SourceFingerprint,
    ) -> Result<crate::portable::PortableNodeId, crate::portable::PortableBuildError>;

    /// Forwards [`Widget::key`].
    ///
    /// The forwarding methods are named apart from the ones they forward to, so
    /// that a widget reached through an [`AnyWidget`] never makes a plain
    /// `widget.key()` ambiguous at a call site that has both traits in scope.
    fn dyn_key(&self) -> Option<crate::key::Key>;

    /// Forwards [`Widget::debug_name`].
    fn dyn_debug_name(&self) -> &'static str;

    /// Forwards [`Widget::text_content`].
    fn dyn_text_content(&self) -> Option<&str>;
}

impl<W: Widget + 'static> DynWidget for W {
    #[cfg(not(aimer_portable_guest))]
    #[inline]
    unsafe fn to_element_in_place(&mut self, ctx: &BuildContext) -> AnyElement {
        // SAFETY: The caller marked the storage vacant, so this bit copy is the
        // single remaining owner of the widget and the value is never dropped
        // in place. The borrow is exclusive, so nothing observes the
        // uninitialized bytes left behind.
        let widget = unsafe { std::ptr::read(self as *mut W) };
        widget.to_element(ctx)
    }

    #[cfg(feature = "portable-guest")]
    #[inline]
    unsafe fn to_portable_node_in_place(
        &mut self,
        ctx: &mut crate::portable::PortableBuildContext,
        source: crate::portable::SourceFingerprint,
    ) -> Result<crate::portable::PortableNodeId, crate::portable::PortableBuildError> {
        // SAFETY: The erased owner marks this storage vacant before invoking
        // the method, exactly as on the native consuming conversion path.
        let widget = unsafe { std::ptr::read(self as *mut W) };
        PortableWidget::to_portable_node(widget, ctx, source)
    }

    #[inline]
    fn dyn_key(&self) -> Option<crate::key::Key> {
        Widget::key(self)
    }

    #[inline]
    fn dyn_debug_name(&self) -> &'static str {
        Widget::debug_name(self)
    }

    #[inline]
    fn dyn_text_content(&self) -> Option<&str> {
        Widget::text_content(self)
    }
}

// SAFETY: The template is `null::<W>()` coerced to the target, so it carries
// exactly `W`'s vtable and a null data address.
unsafe impl<W: DynWidget> aimer_rubick::ErasedFrom<W> for dyn DynWidget {
    const TEMPLATE: *const Self = std::ptr::null::<W>() as *const dyn DynWidget;
}

/// An owned, type-erased [`Widget`] with inline storage and heap fallback.
///
/// Widgets are the throwaway side of the tree: they are rebuilt every frame
/// and are small, so avoiding their allocation is worth spending owner bytes
/// on. `AnyWidget` reserves eight words of inline capacity, which covers the
/// common containers — a `Row` or `Column` is eight words — and leaves the
/// owner itself at nine words. Larger or over-aligned widgets transparently
/// take one pooled heap block.
///
/// `AnyWidget` is a thin wrapper around an [`aimer_rubick::Rubick`] owner, and
/// is exactly its size. `Deref` and `AsRef` expose the stored widget as
/// [`DynWidget`], the object-safe half of [`Widget`], and
/// [`AnyWidget::is_inline`] and [`AnyWidget::is_heap`] report the selected
/// mode. The conversion itself consumes the handle: [`AnyWidget::into_element`]
/// moves the widget out of this storage, so the erased path copies no more than
/// the direct one.
///
/// Moving an inline `AnyWidget` changes the address of its concrete widget. The
/// owner does not provide implicit unsizing or a stable-address guarantee;
/// construct it through [`Widget::boxed`] or [`AnyWidget::new`].
///
/// ```
/// use aimer_widget::base::BuildContext;
/// use aimer_widget::{AnyElement, PortableWidget, Widget};
///
/// struct Badge;
///
/// impl PortableWidget for Badge {}
///
/// impl Widget for Badge {
///     fn to_element(self, _ctx: &BuildContext) -> AnyElement {
///         unreachable!("this example only erases the widget")
///     }
/// }
///
/// let widget = Badge.boxed();
/// assert!(widget.is_inline());
/// ```
#[repr(transparent)]
pub struct AnyWidget(Rubick<dyn DynWidget, 8>);

impl AnyWidget {
    /// Payload bytes a widget may occupy and still be stored inside the handle.
    ///
    /// A wider or more strictly aligned widget takes one pooled heap block.
    pub const INLINE_CAPACITY: usize = Rubick::<dyn DynWidget, 8>::INLINE_CAPACITY;

    /// Erases `widget`, embedding it when it fits and pooling it otherwise.
    ///
    /// This is what [`Widget::boxed`] calls; it exists so a widget can be erased
    /// without importing the trait.
    #[inline]
    pub fn new<W: Widget + 'static>(widget: W) -> Self {
        Self(Rubick::erase(widget))
    }

    /// Erases `widget` using the supplied application UI allocator.
    ///
    /// The owner retains the allocator for as long as its heap payload exists,
    /// even if it is dropped outside the allocator's active scope.
    #[inline]
    pub fn new_in<W: Widget + 'static>(widget: W, allocator: &UiAllocator) -> Self {
        Self(Rubick::erase_in(widget, allocator))
    }

    /// Returns whether the widget is stored inside this handle, with no
    /// separate allocation.
    #[inline]
    pub fn is_inline(&self) -> bool {
        self.0.is_inline()
    }

    /// Returns whether the widget lives in a separate heap block.
    #[inline]
    pub fn is_heap(&self) -> bool {
        self.0.is_heap()
    }

    /// Consumes this handle and builds the widget's element.
    ///
    /// The widget is moved out of its storage, converted, and the storage is
    /// returned to the pool. Nothing is cloned, and the widget's destructor runs
    /// exactly once — inside [`Widget::to_element`], on whatever that method
    /// chooses not to keep — including when the conversion unwinds.
    #[inline]
    pub fn into_element(self, ctx: &BuildContext) -> AnyElement {
        // SAFETY: `take` installs the vacant operation table before the closure
        // runs and releases the pooled block afterwards, on both the normal and
        // the unwinding path. The closure moves the widget out exactly once and
        // never drops it in place, which is the contract `take` requires.
        #[cfg(not(aimer_portable_guest))]
        return unsafe { self.0.take(|widget| (*widget).to_element_in_place(ctx)) };

        #[cfg(aimer_portable_guest)]
        {
            let _ = ctx;
            panic!("native element conversion is unavailable in a portable guest")
        }
    }

    /// Consumes this erased handle and appends its portable Widget IR node.
    #[cfg(feature = "portable-guest")]
    #[inline]
    pub fn into_portable_node(
        self,
        ctx: &mut crate::portable::PortableBuildContext,
        source: crate::portable::SourceFingerprint,
    ) -> Result<crate::portable::PortableNodeId, crate::portable::PortableBuildError> {
        // SAFETY: `take` vacates the erased storage before the concrete widget
        // is moved out, and releases it on both success and error paths.
        unsafe { self.0.take(|widget| (*widget).to_portable_node_in_place(ctx, source)) }
    }
}

impl Deref for AnyWidget {
    type Target = dyn DynWidget;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &*self.0
    }
}

impl AsRef<dyn DynWidget> for AnyWidget {
    #[inline]
    fn as_ref(&self) -> &(dyn DynWidget + 'static) {
        &*self.0
    }
}

impl PortableWidget for AnyWidget {
    #[cfg(feature = "portable-guest")]
    #[inline]
    fn to_portable_node(
        self,
        ctx: &mut crate::portable::PortableBuildContext,
        source: crate::portable::SourceFingerprint,
    ) -> Result<crate::portable::PortableNodeId, crate::portable::PortableBuildError> {
        self.into_portable_node(ctx, source)
    }
}

impl Widget for AnyWidget {
    fn key(&self) -> Option<crate::key::Key> {
        self.as_ref().dyn_key()
    }

    #[inline]
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        self.into_element(ctx)
    }

    fn debug_name(&self) -> &'static str {
        self.as_ref().dyn_debug_name()
    }

    fn text_content(&self) -> Option<&str> {
        self.as_ref().dyn_text_content()
    }

    /// Returns this owner unchanged.
    ///
    /// An [`AnyWidget`] is already erased, so wrapping it again would store an
    /// owner inside an owner: one extra indirection on every borrow and, since
    /// the inner owner is larger than the inline capacity, one guaranteed
    /// allocation per rebuild.
    #[inline]
    fn boxed(self) -> AnyWidget
    where
        Self: Sized + 'static,
    {
        self
    }

    #[inline]
    fn boxed_in(self, _allocator: &UiAllocator) -> AnyWidget
    where
        Self: Sized + 'static,
    {
        self
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use super::*;
    use crate::{Drawable, Element, EventElement, LayoutElement, Rebuildable, VisitorElement};

    /// Payload bytes an [`AnyWidget`] holds without allocating.
    const WIDGET_CAPACITY: usize = AnyWidget::INLINE_CAPACITY;

    #[test]
    fn the_wrapper_is_exactly_as_large_as_the_owner_it_wraps() {
        assert_eq!(
            size_of::<AnyWidget>(),
            size_of::<Rubick<dyn DynWidget, 8>>(),
            "wrapping the owner must not add a word to every child handle"
        );
        assert_eq!(size_of::<AnyWidget>(), 9 * size_of::<usize>());
        assert_eq!(align_of::<AnyWidget>(), align_of::<Rubick<dyn DynWidget, 8>>());
    }

    #[test]
    fn new_and_boxed_erase_identically_and_a_boxed_handle_is_not_wrapped_again() {
        let direct = AnyWidget::new(StorageWidget([0; 1]));
        let boxed = StorageWidget([0; 1]).boxed();
        assert_eq!(direct.is_inline(), boxed.is_inline());
        assert_eq!(direct.debug_name(), boxed.debug_name());

        let rebox = direct.boxed();
        assert!(rebox.is_inline(), "boxing a handle returns it unchanged");
    }

    #[test]
    fn every_widget_is_a_portable_capability() {
        fn requires_portable_widget<T: Widget + PortableWidget>() {}

        requires_portable_widget::<StorageWidget<0>>();
    }

    struct StorageWidget<const N: usize>([u8; N]);

    impl<const N: usize> Widget for StorageWidget<N> {
        fn to_element(self, _ctx: &BuildContext) -> AnyElement {
            panic!("storage contract test does not build an element")
        }

        fn debug_name(&self) -> &'static str {
            "StorageWidget"
        }
    }

    impl<const N: usize> PortableWidget for StorageWidget<N> {}

    #[test]
    fn erased_widgets_select_inline_or_heap_storage_and_dispatch_after_moves() {
        let inline = StorageWidget([]).boxed();
        let heap = StorageWidget([0; WIDGET_CAPACITY + 1]).boxed();

        assert!(inline.is_inline());
        assert!(heap.is_heap());

        let owners = std::hint::black_box([inline, heap]);
        assert_eq!(Widget::debug_name(&owners[0]), "StorageWidget");
        assert_eq!(Widget::debug_name(&owners[1]), "StorageWidget");
    }

    #[test]
    fn boxed_in_uses_the_supplied_ui_memory() {
        let memory = aimer_rubick::UiMemory::new(2 * 1024 * 1024);
        let allocator = memory.allocator();
        let widget = StorageWidget([0; WIDGET_CAPACITY + 1]).boxed_in(&allocator);

        assert!(widget.is_heap());
        assert_eq!(allocator.committed_bytes(), 2 * 1024 * 1024);
    }

    #[test]
    fn an_erased_widget_costs_its_capacity_plus_one_word() {
        assert_eq!(WIDGET_CAPACITY / size_of::<usize>(), 8);
        assert_eq!(size_of::<AnyWidget>(), 9 * size_of::<usize>());
        assert_eq!(size_of::<AnyElement>(), 5 * size_of::<usize>());
    }

    #[test]
    fn container_sized_widgets_are_erased_without_allocating() {
        let container = StorageWidget([0; WIDGET_CAPACITY]).boxed();

        assert!(
            container.is_inline(),
            "a container sized widget must fit the erased owner"
        );
        assert!(
            container.0.is_direct(),
            "erasing must not store projection adapters"
        );
    }

    #[test]
    fn erasing_an_already_erased_widget_returns_the_same_owner() {
        let inline = StorageWidget([0; WIDGET_CAPACITY]).boxed();
        let inline = inline.boxed();

        assert!(
            inline.is_inline(),
            "re-erasing must not wrap the owner in another owner"
        );
        assert_eq!(Widget::debug_name(&inline), "StorageWidget");

        // A heap payload keeps its allocation across owner moves, so its
        // address proves that no second erasure took place.
        let heap = StorageWidget([0; WIDGET_CAPACITY + 1]).boxed();
        let address = heap.as_ref() as *const dyn DynWidget as *const u8;
        let heap = heap.boxed();

        assert!(heap.is_heap());
        assert_eq!(
            heap.as_ref() as *const dyn DynWidget as *const u8,
            address,
            "the payload must not be re-erased into a new allocation"
        );
    }

    struct DroppingWidget<const N: usize> {
        drops: Rc<Cell<usize>>,
        _bytes: [u8; N],
    }

    impl<const N: usize> Drop for DroppingWidget<N> {
        fn drop(&mut self) {
            self.drops.set(self.drops.get() + 1);
        }
    }

    impl<const N: usize> Widget for DroppingWidget<N> {
        fn to_element(self, _ctx: &BuildContext) -> AnyElement {
            panic!("drop contract test does not build an element")
        }
    }

    impl<const N: usize> PortableWidget for DroppingWidget<N> {}

    #[test]
    fn erased_widgets_drop_inline_and_heap_values_exactly_once() {
        let drops = Rc::new(Cell::new(0));
        {
            let inline = DroppingWidget {
                drops: Rc::clone(&drops),
                _bytes: [],
            }
            .boxed();
            let heap = DroppingWidget {
                drops: Rc::clone(&drops),
                _bytes: [0; WIDGET_CAPACITY],
            }
            .boxed();

            assert!(inline.is_inline());
            assert!(heap.is_heap());
        }

        assert_eq!(drops.get(), 2);
    }

    /// Records its own destruction, so a test can count how many times the
    /// value behind an erased widget is destroyed.
    struct DropRecorder {
        drops: Rc<Cell<usize>>,
    }

    impl Drop for DropRecorder {
        fn drop(&mut self) {
            self.drops.set(self.drops.get() + 1);
        }
    }

    fn recorder(drops: &Rc<Cell<usize>>) -> DropRecorder {
        DropRecorder {
            drops: Rc::clone(drops),
        }
    }

    /// A widget that is deliberately neither [`Clone`] nor [`Copy`], so the
    /// conversion has no way to reproduce it: whatever the element ends up
    /// holding must be the very value that was erased.
    struct Moved {
        recorder: DropRecorder,
        label: String,
        observed: Rc<Cell<*const u8>>,
    }

    /// Keeps the widget's fields alive, which is all the ownership tests ask of
    /// an element.
    struct MovedElement {
        _recorder: DropRecorder,
        _label: String,
    }

    impl VisitorElement for MovedElement {
        fn debug_name(&self) -> &'static str {
            "MovedElement"
        }
    }

    impl Drawable for MovedElement {
        fn update(&self, _ctx: &BuildContext) {}
    }

    impl EventElement for MovedElement {}
    impl LayoutElement for MovedElement {}
    impl Rebuildable for MovedElement {}

    impl Widget for Moved {
        fn to_element(self, _ctx: &BuildContext) -> AnyElement {
            self.observed.set(self.label.as_ptr());
            MovedElement {
                _recorder: self.recorder,
                _label: self.label,
            }
            .boxed()
        }
    }

    impl PortableWidget for Moved {}

    /// A widget too wide for inline storage, so its payload is pooled.
    struct Wide {
        recorder: DropRecorder,
        _bytes: [usize; 8],
    }

    impl Widget for Wide {
        fn to_element(self, _ctx: &BuildContext) -> AnyElement {
            MovedElement {
                _recorder: self.recorder,
                _label: String::new(),
            }
            .boxed()
        }
    }

    impl PortableWidget for Wide {}

    /// A widget whose conversion fails *after* the move has happened, which is
    /// the only window in which the value is owned by neither its storage nor
    /// its element.
    #[cfg(panic = "unwind")]
    struct Failing {
        _recorder: DropRecorder,
    }

    #[cfg(panic = "unwind")]
    impl Widget for Failing {
        fn to_element(self, _ctx: &BuildContext) -> AnyElement {
            panic!("exercise an unwind out of to_element");
        }
    }

    #[cfg(panic = "unwind")]
    impl PortableWidget for Failing {}

    /// Address of the payload behind an erased widget.
    fn payload_address(widget: &AnyWidget) -> *const u8 {
        widget.as_ref() as *const dyn DynWidget as *const u8
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
            crate::base::WindowHandle::headless(Default::default(), 1.0),
            tokio::runtime::Handle::current(),
        )
    }

    #[tokio::test]
    async fn building_an_erased_widget_moves_it_instead_of_copying_it() {
        let drops = Rc::new(Cell::new(0));
        let observed = Rc::new(Cell::new(std::ptr::null()));
        let widget = Moved {
            recorder: recorder(&drops),
            label: String::from("Aimer"),
            observed: Rc::clone(&observed),
        };
        let buffer = widget.label.as_ptr();

        let element = widget.boxed().into_element(&context());

        assert_eq!(element.debug_name(), "MovedElement");
        assert_eq!(
            observed.get(),
            buffer,
            "the string buffer travelled through the erasure without being copied"
        );
        assert_eq!(
            drops.get(),
            0,
            "the widget's fields were moved into the element, not copied and destroyed"
        );
    }

    #[tokio::test]
    async fn building_a_pooled_widget_returns_its_block() {
        let drops = Rc::new(Cell::new(0));
        let widget = Wide {
            recorder: recorder(&drops),
            _bytes: [0; 8],
        }
        .boxed();
        assert!(widget.is_heap(), "the fixture must exercise pooled storage");
        let block = payload_address(&widget);

        let element = widget.into_element(&context());
        assert_eq!(element.debug_name(), "MovedElement");

        let rebuilt = Wide {
            recorder: recorder(&drops),
            _bytes: [0; 8],
        }
        .boxed();

        assert_eq!(
            payload_address(&rebuilt),
            block,
            "a consumed widget must hand its block back to the pool"
        );
        assert_eq!(drops.get(), 0);
    }

    #[tokio::test]
    async fn a_built_widget_is_destroyed_exactly_once() {
        let drops = Rc::new(Cell::new(0));
        let element = Moved {
            recorder: recorder(&drops),
            label: String::from("built"),
            observed: Rc::new(Cell::new(std::ptr::null())),
        }
        .boxed()
        .into_element(&context());

        assert_eq!(drops.get(), 0, "the element owns the moved value");
        drop(element);
        assert_eq!(drops.get(), 1, "the moved value is destroyed exactly once");
    }

    #[cfg(panic = "unwind")]
    #[tokio::test]
    async fn a_panicking_conversion_destroys_the_widget_exactly_once() {
        use std::panic::{AssertUnwindSafe, catch_unwind};

        let drops = Rc::new(Cell::new(0));
        let ctx = context();
        let result = catch_unwind(AssertUnwindSafe(|| {
            Failing {
                _recorder: recorder(&drops),
            }
            .boxed()
            .into_element(&ctx)
        }));

        assert!(result.is_err());
        assert_eq!(
            drops.get(),
            1,
            "the moved widget is destroyed by the unwind and never again by its storage"
        );
    }

    #[tokio::test]
    async fn nested_handles_collapse_into_one_element() {
        let drops = Rc::new(Cell::new(0));
        let widget = Moved {
            recorder: recorder(&drops),
            label: String::from("nested"),
            observed: Rc::new(Cell::new(std::ptr::null())),
        }
        .boxed()
        .boxed();

        let element = widget.into_element(&context());

        assert_eq!(element.debug_name(), "MovedElement");
        assert_eq!(drops.get(), 0);
        drop(element);
        assert_eq!(
            drops.get(),
            1,
            "re-erasing must not add a second owner of the same value"
        );
    }
}
