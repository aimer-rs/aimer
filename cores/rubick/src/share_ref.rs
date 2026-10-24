use crate::{ErasedFrom, Rubick, Shared, UiAllocator};
use std::borrow::Borrow;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use std::ops::Deref;
use std::rc::Rc;
use std::str::FromStr;

/// An owning, read-only projection into a [`Shared`] value.
///
/// An internal projected reference stores a strong owner plus a selector. It
/// never stores a raw pointer into the selected field.
struct SharedRef<Owner: ?Sized, Field: ?Sized, Select> {
    owner: Shared<Owner>,
    select: Select,
    field: PhantomData<*const Field>,
}

impl<Owner: ?Sized, Field, Select> SharedRef<Owner, Field, Select>
where
    Field: ?Sized,
    Select: for<'a> Fn(&'a Owner) -> &'a Field,
{
    /// Borrows the selected field.
    #[must_use]
    #[inline]
    pub fn get(&self) -> &Field {
        (self.select)(&self.owner)
    }
}

trait SharedValue<T: ?Sized> {
    fn get(&self) -> &T;
}

impl<Owner: ?Sized, Field, Select> SharedValue<Field> for SharedRef<Owner, Field, Select>
where
    Owner: 'static,
    Field: ?Sized + 'static,
    Select: for<'a> Fn(&'a Owner) -> &'a Field + 'static,
{
    #[inline]
    fn get(&self) -> &Field {
        SharedRef::get(self)
    }
}

// SAFETY: The template carries the vtable for the concrete `SharedRef`, whose
// `SharedValue<Field>` implementation returns a borrow tied to that owner.
unsafe impl<Owner: ?Sized, Field, Select> ErasedFrom<SharedRef<Owner, Field, Select>>
    for dyn SharedValue<Field>
where
    Owner: 'static,
    Field: ?Sized + 'static,
    Select: for<'a> Fn(&'a Owner) -> &'a Field + 'static,
{
    const TEMPLATE: *const Self =
        std::ptr::null::<SharedRef<Owner, Field, Select>>() as *const dyn SharedValue<Field>;
}

struct ProjectedShareRef<T: ?Sized + 'static, U: ?Sized + 'static, Select> {
    source: ShareRef<T>,
    select: Select,
    target: PhantomData<*const U>,
}

impl<T, U, Select> SharedValue<U> for ProjectedShareRef<T, U, Select>
where
    T: ?Sized + 'static,
    U: ?Sized + 'static,
    Select: for<'a> Fn(&'a T) -> &'a U,
{
    #[inline]
    fn get(&self) -> &U {
        (self.select)(self.source.get())
    }
}

// SAFETY: The template carries the vtable for `ProjectedShareRef`, whose
// `SharedValue<U>` implementation returns a borrow tied to the source handle.
unsafe impl<T, U, Select> ErasedFrom<ProjectedShareRef<T, U, Select>> for dyn SharedValue<U>
where
    T: ?Sized + 'static,
    U: ?Sized + 'static,
    Select: for<'a> Fn(&'a T) -> &'a U + 'static,
{
    const TEMPLATE: *const Self =
        std::ptr::null::<ProjectedShareRef<T, U, Select>>() as *const dyn SharedValue<U>;
}

enum ShareRefStorage<T: ?Sized + 'static> {
    Static(&'static T),
    Shared(Shared<Rubick<dyn SharedValue<T>, 1>>),
}

/// A type-erased, owning read-only reference to a shared value.
///
/// `ShareRef<T>` is the stable interface for APIs that need to retain a
/// projected value without knowing the owner or selector types used to obtain
/// it. Cloning the handle clones its ownership reference; it never clones the
/// selected value. The returned borrow from [`ShareRef::get`] is tied to the
/// handle. Static-backed handles retain a static reference directly.
///
/// [`Shared::project`] creates a projected handle directly. Use
/// [`ShareRef::from_rc`] to retain a value held by an [`Rc`], or
/// [`ShareRef::from_static`] for a value with a static lifetime.
pub struct ShareRef<T: ?Sized + 'static> {
    value: ShareRefStorage<T>,
}

/// Adds projection and [`ShareRef`] conversion methods to [`Rc`] values.
///
/// Import this trait to project a field from an `Rc` into an owning
/// [`ShareRef`]. The returned handle retains an `Rc` clone, so it remains
/// valid after the original `Rc` is dropped; the selected field is not cloned.
///
/// # Examples
///
/// ```
/// use aimer_rubick::RcProjectExt;
/// use std::rc::Rc;
///
/// struct State { title: String }
///
/// let state = Rc::new(State { title: String::from("Aimer") });
/// let title = state.project(|state| state.title.as_str());
/// drop(state);
///
/// assert_eq!(&*title, "Aimer");
/// ```
pub trait RcProjectExt<T: ?Sized + 'static> {
    /// Creates an owning handle to the complete `Rc` value.
    ///
    /// The returned handle retains an `Rc` clone. Use
    /// [`into_share_ref`](Self::into_share_ref) to transfer the `Rc` instead.
    fn as_share_ref(&self) -> ShareRef<T>;

    /// Converts this `Rc` into an owning read-only handle to its value.
    ///
    /// This moves the `Rc` into the returned handle without incrementing its
    /// strong-reference count.
    fn into_share_ref(self) -> ShareRef<T>;

    /// Creates an owning handle to a borrowed part of the `Rc` value.
    fn project<Field, Select>(&self, select: Select) -> ShareRef<Field>
    where
        Field: ?Sized + 'static,
        Select: for<'a> Fn(&'a T) -> &'a Field + 'static;
}

impl<T: ?Sized + 'static> RcProjectExt<T> for Rc<T> {
    #[inline]
    fn as_share_ref(&self) -> ShareRef<T> {
        ShareRef::from_rc(self)
    }

    #[inline]
    fn into_share_ref(self) -> ShareRef<T> {
        ShareRef::from_projection(Shared::new(self), |owner: &Rc<T>| owner.as_ref())
    }

    #[inline]
    fn project<Field, Select>(&self, select: Select) -> ShareRef<Field>
    where
        Field: ?Sized + 'static,
        Select: for<'a> Fn(&'a T) -> &'a Field + 'static,
    {
        ShareRef::from_projection(Shared::new(Rc::clone(self)), move |owner: &Rc<T>| {
            select(owner.as_ref())
        })
    }
}

impl<T: ?Sized + 'static> ShareRef<T> {
    #[inline]
    pub(crate) fn from_projection<Owner: ?Sized, Select>(
        owner: Shared<Owner>,
        select: Select,
    ) -> Self
    where
        Owner: 'static,
        Select: for<'a> Fn(&'a Owner) -> &'a T + 'static,
    {
        let allocator = UiAllocator::current().or_else(|| owner.ui_allocator());
        let reference = SharedRef {
            owner,
            select,
            field: PhantomData,
        };
        match allocator {
            Some(allocator) => allocator.scope(|| {
                let value: Rubick<dyn SharedValue<T>, 1> = Rubick::erase(reference);
                Self {
                    value: ShareRefStorage::Shared(Shared::new_in(value, &allocator)),
                }
            }),
            None => {
                let value: Rubick<dyn SharedValue<T>, 1> = Rubick::erase(reference);
                Self {
                    value: ShareRefStorage::Shared(Shared::new(value)),
                }
            }
        }
    }

    /// Creates a handle to a value whose reference is valid for the program's
    /// lifetime.
    ///
    /// The static reference is stored directly, so construction and cloning do
    /// not allocate.
    #[must_use]
    #[inline]
    pub fn from_static(value: &'static T) -> Self {
        Self {
            value: ShareRefStorage::Static(value),
        }
    }

    /// Creates a shared handle from an existing [`Rc`].
    ///
    /// The `Rc` is retained without cloning its value. Use [`ShareRef::project`]
    /// to narrow a `ShareRef<String>` to a `ShareRef<str>` without copying the
    /// string contents.
    #[must_use]
    #[inline]
    pub fn from_rc(rc: &Rc<T>) -> Self {
        Self::from_projection(Shared::new(Rc::clone(rc)), Rc::as_ref)
    }

    /// Projects this handle to a borrowed subvalue without cloning it.
    #[must_use]
    #[inline]
    pub fn project<U, Select>(self, select: Select) -> ShareRef<U>
    where
        U: ?Sized + 'static,
        Select: for<'a> Fn(&'a T) -> &'a U + 'static,
    {
        match self.value {
            ShareRefStorage::Static(value) => ShareRef::from_static(select(value)),
            ShareRefStorage::Shared(value) => {
                let allocator = UiAllocator::current().or_else(|| value.ui_allocator());
                let source = Self {
                    value: ShareRefStorage::Shared(value),
                };
                match allocator {
                    Some(allocator) => allocator.scope(|| {
                        let projected = ProjectedShareRef {
                            source,
                            select,
                            target: PhantomData,
                        };
                        let value: Rubick<dyn SharedValue<U>, 1> = Rubick::erase(projected);
                        ShareRef {
                            value: ShareRefStorage::Shared(Shared::new_in(value, &allocator)),
                        }
                    }),
                    None => {
                        let projected = ProjectedShareRef {
                            source,
                            select,
                            target: PhantomData,
                        };
                        let value: Rubick<dyn SharedValue<U>, 1> = Rubick::erase(projected);
                        ShareRef {
                            value: ShareRefStorage::Shared(Shared::new(value)),
                        }
                    }
                }
            }
        }
    }

    /// Borrows the retained value.
    #[must_use]
    #[inline]
    pub fn get(&self) -> &T {
        match &self.value {
            ShareRefStorage::Static(value) => value,
            ShareRefStorage::Shared(value) => value.get(),
        }
    }

    /// Returns `true` when both handles resolve to the same target pointer.
    #[must_use]
    #[inline]
    pub fn ptr_eq(this: &Self, other: &Self) -> bool {
        std::ptr::eq(this.get(), other.get())
    }
}

impl<T: Clone + 'static> ShareRef<[T]> {
    /// Clones the elements of `value` into an owned shared slice.
    #[must_use]
    #[inline]
    pub fn from_slice(value: &[T]) -> Self {
        Shared::<[T]>::from_slice(value).into_share_ref()
    }
}

impl<T: Clone + 'static, const N: usize> From<&[T; N]> for ShareRef<[T]> {
    fn from(value: &[T; N]) -> Self {
        Self::from_slice(value)
    }
}

impl<T: 'static, const N: usize> From<[T; N]> for ShareRef<[T]> {
    fn from(value: [T; N]) -> Self {
        Shared::<[T]>::from(value).into_share_ref()
    }
}

impl ShareRef<str> {
    /// Copies `value` into an owned string retained by this handle.
    #[must_use]
    #[inline]
    pub fn from_str_slice(value: &str) -> Self {
        Shared::<str>::from(value).into_share_ref()
    }
}

impl From<String> for ShareRef<str> {
    fn from(value: String) -> Self {
        Shared::<str>::from(value).into_share_ref()
    }
}

impl<T: FromStr + 'static> FromStr for ShareRef<T> {
    type Err = T::Err;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        T::from_str(value).map(|value| Shared::new(value).project(|value: &T| value))
    }
}

impl Default for ShareRef<str> {
    #[inline]
    fn default() -> Self {
        Shared::<str>::from("").into_share_ref()
    }
}

impl<T> From<Shared<T>> for ShareRef<T> {
    fn from(this: Shared<T>) -> Self {
        this.into_share_ref()
    }
}

impl FromStr for ShareRef<str> {
    type Err = std::convert::Infallible;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let owner = Shared::<str>::from_box(value.to_owned().into_boxed_str());
        Ok(owner.project(|value: &str| value))
    }
}

impl<T: ?Sized + 'static> Clone for ShareRef<T> {
    #[inline]
    fn clone(&self) -> Self {
        Self {
            value: match &self.value {
                ShareRefStorage::Static(value) => ShareRefStorage::Static(*value),
                ShareRefStorage::Shared(value) => ShareRefStorage::Shared(value.clone()),
            },
        }
    }
}

impl<T: ?Sized + 'static> Deref for ShareRef<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        self.get()
    }
}

impl<T: ?Sized + 'static> AsRef<T> for ShareRef<T> {
    #[inline]
    fn as_ref(&self) -> &T {
        self.get()
    }
}

impl<T: ?Sized + 'static> Borrow<T> for ShareRef<T> {
    fn borrow(&self) -> &T {
        self.get()
    }
}

impl<T: Borrow<str> + 'static> Borrow<str> for ShareRef<T> {
    fn borrow(&self) -> &str {
        Borrow::<str>::borrow(self.get())
    }
}

impl<T: ?Sized + Hash + 'static> Hash for ShareRef<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.get().hash(state);
    }
}

impl<T: ?Sized + fmt::Debug + 'static> fmt::Debug for ShareRef<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.get(), formatter)
    }
}

impl<T: ?Sized + fmt::Display + 'static> fmt::Display for ShareRef<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self.get(), formatter)
    }
}

impl<T, U> PartialEq<ShareRef<U>> for ShareRef<T>
where
    T: ?Sized + PartialEq<U> + 'static,
    U: ?Sized + 'static,
{
    fn eq(&self, other: &ShareRef<U>) -> bool {
        self.get() == other.get()
    }
}

impl<T: ?Sized + 'static> From<&'static T> for ShareRef<T> {
    fn from(value: &'static T) -> Self {
        ShareRef::from_static(value)
    }
}

impl<T: ?Sized + Eq + 'static> Eq for ShareRef<T> {}
