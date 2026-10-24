//! Per-concrete-type operation tables and pointer reconstruction.
//!
//! Every distinct `(concrete type, target, capacity)` triple needs exactly one
//! [`Operations`] table. The table is built in a constant and referenced as
//! `&'static`, so an owner stores a single word instead of copying function
//! pointers into every instance, and construction writes one word instead of
//! three.

use std::alloc::Layout;
use std::marker::PhantomData;
use std::ptr;

use crate::INLINE_ALIGNMENT;
use crate::erase::ErasedFrom;
use crate::pool;

/// A concrete value stored together with its projection adapters.
pub(crate) struct Projected<U, P, PM> {
    pub(crate) value: U,
    pub(crate) project: P,
    pub(crate) project_mut: PM,
}

/// How an owner turns a payload address into a pointer to its target.
pub(crate) enum Projection<T: ?Sized> {
    /// A pointer template with a null data address and the payload's
    /// metadata. Borrowing only rewrites the data word, so no call occurs.
    Direct(*const T),
    /// Explicit adapters for projections that are not plain unsizing, such as
    /// borrowing an interior field or dispatching on the value.
    Adapted {
        project: unsafe fn(*const u8) -> *const T,
        project_mut: unsafe fn(*mut u8) -> *mut T,
    },
}

impl<T: ?Sized> Projection<T> {
    /// Produces a shared pointer to the target stored at `data`.
    ///
    /// # Safety
    ///
    /// `data` must address the initialized concrete value this projection was
    /// built for.
    #[inline(always)]
    pub(crate) unsafe fn shared(&self, data: *const u8) -> *const T {
        match self {
            Self::Direct(template) => rebuild(*template, data),
            // SAFETY: Forwarded from this method's contract.
            Self::Adapted { project, .. } => unsafe { project(data) },
        }
    }

    /// Returns `true` when borrowing needs no adapter call.
    #[inline(always)]
    pub(crate) fn is_direct(&self) -> bool {
        matches!(self, Self::Direct(_))
    }

    /// Produces an exclusive pointer to the target stored at `data`.
    ///
    /// # Safety
    ///
    /// `data` must address the initialized concrete value this projection was
    /// built for, and the caller must hold exclusive access to it.
    #[inline(always)]
    pub(crate) unsafe fn exclusive(&self, data: *mut u8) -> *mut T {
        match self {
            Self::Direct(template) => rebuild_mut(template.cast_mut(), data),
            // SAFETY: Forwarded from this method's contract.
            Self::Adapted { project_mut, .. } => unsafe { project_mut(data) },
        }
    }
}

/// Rebuilds a shared pointer to `T` by replacing a template's data address.
///
/// A pointer to a possibly unsized type is either one machine word — a thin
/// pointer whose only content is the data address — or two words, where the
/// first word is the data address and the second is metadata that does not
/// depend on that address. Overwriting the first word therefore produces a
/// pointer to the same kind of value living at `data`, which is exactly what
/// `ptr::from_raw_parts` does on nightly. The layout assumption is checked at
/// compile time for every instantiation.
#[inline(always)]
fn rebuild<T: ?Sized>(template: *const T, data: *const u8) -> *const T {
    assert_pointer_layout::<T>();
    let mut pointer = template;
    // SAFETY: `pointer` is a local pointer value whose first word is its data
    // address, and `data` is a pointer of exactly that width. The write only
    // replaces the address, leaving any metadata word untouched.
    unsafe { (&raw mut pointer).cast::<*const u8>().write(data) };
    pointer
}

/// Rebuilds an exclusive pointer to `T` by replacing a template's data
/// address. See [`rebuild`] for the layout reasoning.
#[inline(always)]
fn rebuild_mut<T: ?Sized>(template: *mut T, data: *mut u8) -> *mut T {
    assert_pointer_layout::<T>();
    let mut pointer = template;
    // SAFETY: Identical to `rebuild`, with an exclusive data address so the
    // rebuilt pointer keeps write provenance.
    unsafe { (&raw mut pointer).cast::<*mut u8>().write(data) };
    pointer
}

/// Asserts that pointers to `T` have the layout this crate relies on.
#[inline(always)]
const fn assert_pointer_layout<T: ?Sized>() {
    const {
        assert!(
            size_of::<*const T>() == size_of::<usize>()
                || size_of::<*const T>() == 2 * size_of::<usize>(),
            "aimer_rubick requires thin or two-word pointers"
        );
    }
}

/// The static description of one concrete storage type.
pub(crate) struct Operations<T: ?Sized> {
    /// How to borrow the payload as the erased target.
    pub(crate) projection: Projection<T>,
    /// Runs the concrete destructor without freeing storage.
    pub(crate) drop_in_place: unsafe fn(*mut u8),
    /// The concrete layout, used to allocate, free, and reuse heap blocks.
    pub(crate) layout: Layout,
    /// The complete allocation layout, including Rubick's origin header.
    pub(crate) heap_layout: Option<Layout>,
    /// Offset from the block base to the concrete payload.
    pub(crate) heap_data_offset: usize,
    /// The pool class serving `heap_layout`, resolved once at compile time.
    pub(crate) heap_class: u8,
    /// Whether the payload lives in the owner instead of a heap block.
    ///
    /// The decision is a compile-time function of the concrete layout and the
    /// owner's capacity, which is why it belongs here and not in the owner.
    pub(crate) inline: bool,
}

impl<T: ?Sized> Operations<T> {
    /// Describes a concrete storage type `U` for an owner of `capacity` bytes.
    const fn describe<U>(projection: Projection<T>, capacity: usize) -> Self {
        let layout = Layout::new::<U>();
        let (heap_layout, heap_data_offset, heap_class) = match pool::heap_layout(layout) {
            Some((heap_layout, data_offset)) => {
                (Some(heap_layout), data_offset, pool::class_of(heap_layout))
            }
            None => (None, 0, pool::UNPOOLED),
        };
        Self {
            projection,
            drop_in_place: drop_in_place::<U>,
            layout,
            heap_layout,
            heap_data_offset,
            heap_class,
            inline: size_of::<U>() <= capacity && align_of::<U>() <= INLINE_ALIGNMENT,
        }
    }
}

impl<T: ?Sized + 'static> Operations<T> {
    /// Describes an owner whose payload has already been destroyed.
    ///
    /// [`Rubick::replace`](crate::Rubick::replace) installs this table while
    /// the storage is momentarily vacant, so a panic between destruction and
    /// re-initialization can never drop the payload twice.
    const VACANT: Self = Self {
        projection: Projection::Adapted {
            project: vacant_shared::<T>,
            project_mut: vacant_exclusive::<T>,
        },
        drop_in_place: drop_in_place::<()>,
        layout: Layout::new::<()>(),
        heap_layout: match pool::heap_layout(Layout::new::<()>()) {
            Some((layout, _)) => Some(layout),
            None => None,
        },
        heap_data_offset: match pool::heap_layout(Layout::new::<()>()) {
            Some((_, offset)) => offset,
            None => 0,
        },
        heap_class: match pool::heap_layout(Layout::new::<()>()) {
            Some((layout, _)) => pool::class_of(layout),
            None => pool::UNPOOLED,
        },
        inline: true,
    };

    /// A shared reference to [`Operations::VACANT`].
    pub(crate) const VACANT_REF: &'static Self = &Self::VACANT;
}

/// The operation table for a sized owner that stores its target directly.
pub(crate) struct IdentityTable<T, const WORDS: usize>(PhantomData<fn() -> T>);

impl<T: 'static, const WORDS: usize> IdentityTable<T, WORDS> {
    const OPS: Operations<T> = Operations::describe::<T>(
        Projection::Direct(ptr::null::<T>()),
        WORDS * size_of::<*mut u8>(),
    );

    /// The promoted table for this concrete type and capacity.
    pub(crate) const REF: &'static Operations<T> = &Self::OPS;
}

/// The operation table for a payload erased by unsizing.
pub(crate) struct ErasedTable<U, T: ?Sized, const WORDS: usize>(PhantomData<(U, *const T)>);

impl<U: 'static, T: ?Sized + ErasedFrom<U>, const WORDS: usize> ErasedTable<U, T, WORDS> {
    const OPS: Operations<T> = Operations::describe::<U>(
        Projection::Direct(<T as ErasedFrom<U>>::TEMPLATE),
        WORDS * size_of::<*mut u8>(),
    );

    /// The promoted table for this concrete type, target, and capacity.
    pub(crate) const REF: &'static Operations<T> = &Self::OPS;
}

/// The operation table for a payload borrowed through explicit adapters.
pub(crate) struct AdaptedTable<U, P, PM, T: ?Sized, const WORDS: usize>(
    PhantomData<(U, P, PM, *const T)>,
);

impl<U, P, PM, T, const WORDS: usize> AdaptedTable<U, P, PM, T, WORDS>
where
    U: 'static,
    P: for<'a> Fn(&'a U) -> &'a T + 'static,
    PM: for<'a> Fn(&'a mut U) -> &'a mut T + 'static,
    T: ?Sized + 'static,
{
    const OPS: Operations<T> = Operations::describe::<Projected<U, P, PM>>(
        Projection::Adapted {
            project: project_stored::<U, P, PM, T>,
            project_mut: project_stored_mut::<U, P, PM, T>,
        },
        WORDS * size_of::<*mut u8>(),
    );

    /// The promoted table for this projected storage type and capacity.
    pub(crate) const REF: &'static Operations<T> = &Self::OPS;
}

unsafe fn project_stored<U, P, PM, T: ?Sized>(pointer: *const u8) -> *const T
where
    P: for<'a> Fn(&'a U) -> &'a T,
{
    // SAFETY: This adapter is installed only when the storage contains the
    // matching `Projected<U, P, PM>` concrete type.
    let stored = unsafe { &*pointer.cast::<Projected<U, P, PM>>() };
    (stored.project)(&stored.value)
}

unsafe fn project_stored_mut<U, P, PM, T: ?Sized>(pointer: *mut u8) -> *mut T
where
    PM: for<'a> Fn(&'a mut U) -> &'a mut T,
{
    // SAFETY: This adapter is installed only when the storage contains the
    // matching `Projected<U, P, PM>` concrete type and the caller has
    // exclusive access to its owner.
    let stored = unsafe { &mut *pointer.cast::<Projected<U, P, PM>>() };
    (stored.project_mut)(&mut stored.value)
}

unsafe fn vacant_shared<T: ?Sized>(_pointer: *const u8) -> *const T {
    unreachable!("a vacant Rubick is never borrowed")
}

unsafe fn vacant_exclusive<T: ?Sized>(_pointer: *mut u8) -> *mut T {
    unreachable!("a vacant Rubick is never borrowed")
}

unsafe fn drop_in_place<U>(pointer: *mut u8) {
    // SAFETY: The caller passes the address of one initialized `U`.
    unsafe { ptr::drop_in_place(pointer.cast::<U>()) };
}
