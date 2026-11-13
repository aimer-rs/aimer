//! The data a [`Column::list`](crate::Column::list) renders.
//!
//! A windowed list keeps its *data* and builds a row only when a frame asks for
//! it, and [`Widget::to_element`](aimer_widget::Widget::to_element) runs again on
//! every rebuild. If the list took ownership of a fresh `Vec<T>` each time, an
//! application whose rows live in a provider would clone the whole collection per
//! rebuild just to hand it over. [`ListItems`] is the shared, immutable slice the
//! list retains instead, and every way of supplying data converts into it:
//!
//! | Source                          | Cost                                        |
//! |---------------------------------|---------------------------------------------|
//! | [`ShareRef<[T]>`]               | a reference-count bump, no element touched  |
//! | `Rc<Vec<T>>`, `Rc<[T]>`         | a reference-count bump, no element touched  |
//! | `Vec<T>`, `[T; N]`              | the elements move once, none is cloned      |
//! | `Range<T>`, `RangeInclusive<T>` | collected once, which is what a range means |
//! | any iterator, via `collect()`   | collected once                              |
//!
//! Borrowed collections (`&Vec<T>`, `&[T]`) are deliberately *not* accepted: the
//! list outlives the borrow, so accepting one would hide a clone of every
//! element. Share the collection (for instance by projecting a provider snapshot
//! into a [`ShareRef`]) or clone it explicitly.

use std::ops::{Deref, Range, RangeInclusive};
use std::rc::Rc;

use aimer_rubick::{RcProjectExt, ShareRef, Shared};

/// An immutable, cheaply cloneable slice of the data behind a list.
///
/// Cloning a `ListItems` never clones its elements. The type is what
/// [`Column::list`](crate::Column::list), [`Row::list`](crate::Row::list), and
/// [`Flex::list`](crate::Flex::list) accept through `impl Into<ListItems<T>>`, so
/// callers rarely name it; collect an arbitrary iterator into one with
/// [`Iterator::collect`].
///
/// It dereferences to `[T]`.
///
/// # Examples
///
/// ```
/// use aimer_flex::ListItems;
///
/// let items: ListItems<u32> = (0..3).collect();
/// assert_eq!(&*items, &[0, 1, 2]);
///
/// // Sharing is by handle: no element is cloned.
/// let again = items.clone();
/// assert!(ListItems::ptr_eq(&items, &again));
/// ```
pub struct ListItems<T: 'static> {
    items: ShareRef<[T]>,
}

impl<T: 'static> ListItems<T> {
    /// Returns `true` when both handles refer to the same underlying elements.
    ///
    /// This is pointer identity, not equality: two lists built from equal but
    /// separately allocated collections are not `ptr_eq`. It is the cheap check
    /// for "the data did not change" when the owner swaps its allocation only on
    /// update, as a provider does.
    #[must_use]
    #[inline]
    pub fn ptr_eq(this: &Self, other: &Self) -> bool {
        ShareRef::ptr_eq(&this.items, &other.items)
    }

    /// Returns the shared slice, leaving the handle's ownership intact.
    #[inline]
    pub(crate) fn into_shared(self) -> ShareRef<[T]> {
        self.items
    }
}

impl<T: 'static> Clone for ListItems<T> {
    #[inline]
    fn clone(&self) -> Self {
        Self {
            items: self.items.clone(),
        }
    }
}

impl<T: 'static> Deref for ListItems<T> {
    type Target = [T];

    #[inline]
    fn deref(&self) -> &[T] {
        self.items.get()
    }
}

impl<T: 'static> From<ShareRef<[T]>> for ListItems<T> {
    #[inline]
    fn from(items: ShareRef<[T]>) -> Self {
        Self { items }
    }
}

impl<T: 'static> From<Vec<T>> for ListItems<T> {
    /// Moves the elements into shared storage; none is cloned.
    #[inline]
    fn from(items: Vec<T>) -> Self {
        // `Shared` retains the boxed slice as it is, so the elements are not
        // copied; `into_boxed_slice` only shrinks a vector with spare capacity.
        Shared::<[T]>::from(items.into_boxed_slice())
            .into_share_ref()
            .into()
    }
}

impl<T: 'static, const N: usize> From<[T; N]> for ListItems<T> {
    #[inline]
    fn from(items: [T; N]) -> Self {
        ShareRef::from(items).into()
    }
}

impl<T: 'static> From<Rc<[T]>> for ListItems<T> {
    #[inline]
    fn from(items: Rc<[T]>) -> Self {
        items.into_share_ref().into()
    }
}

impl<T: 'static> From<Rc<Vec<T>>> for ListItems<T> {
    #[inline]
    fn from(items: Rc<Vec<T>>) -> Self {
        items.project(|items| items.as_slice()).into()
    }
}

impl<T: 'static> From<Range<T>> for ListItems<T>
where
    Range<T>: Iterator<Item = T>,
{
    #[inline]
    fn from(range: Range<T>) -> Self {
        range.collect()
    }
}

impl<T: 'static> From<RangeInclusive<T>> for ListItems<T>
where
    RangeInclusive<T>: Iterator<Item = T>,
{
    #[inline]
    fn from(range: RangeInclusive<T>) -> Self {
        range.collect()
    }
}

impl<T: 'static> FromIterator<T> for ListItems<T> {
    /// Collects the iterator once into shared storage.
    #[inline]
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        Vec::from_iter(iter).into()
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    /// A datum that records a clone instead of allowing one silently.
    struct Tracked {
        id: u32,
        clones: Rc<Cell<usize>>,
    }

    impl Clone for Tracked {
        fn clone(&self) -> Self {
            self.clones.set(self.clones.get() + 1);
            Self {
                id: self.id,
                clones: Rc::clone(&self.clones),
            }
        }
    }

    fn tracked(len: u32, clones: &Rc<Cell<usize>>) -> Vec<Tracked> {
        (0..len)
            .map(|id| Tracked {
                id,
                clones: Rc::clone(clones),
            })
            .collect()
    }

    fn ids(items: &ListItems<Tracked>) -> Vec<u32> {
        items.iter().map(|item| item.id).collect()
    }

    #[test]
    fn a_vec_keeps_its_order_and_is_never_cloned() {
        let clones = Rc::new(Cell::new(0));

        let items = ListItems::from(tracked(4, &clones));

        assert_eq!(ids(&items), [0, 1, 2, 3]);
        assert_eq!(clones.get(), 0);
    }

    #[test]
    fn cloning_a_handle_shares_the_elements() {
        let clones = Rc::new(Cell::new(0));
        let items = ListItems::from(tracked(4, &clones));

        let again = items.clone();

        assert!(ListItems::ptr_eq(&items, &again));
        assert_eq!(clones.get(), 0);
    }

    #[test]
    fn equal_data_in_separate_allocations_is_not_ptr_eq() {
        let a = ListItems::from(vec![1, 2, 3]);
        let b = ListItems::from(vec![1, 2, 3]);

        assert_eq!(&*a, &*b);
        assert!(!ListItems::ptr_eq(&a, &b));
    }

    #[test]
    fn a_share_ref_is_adopted_without_touching_its_elements() {
        let clones = Rc::new(Cell::new(0));
        let owner = Rc::new(tracked(3, &clones));
        let shared = owner.project(|owner| owner.as_slice());
        let target = shared.get().as_ptr();

        let items = ListItems::from(shared);

        assert_eq!(ids(&items), [0, 1, 2]);
        assert_eq!(items.as_ptr(), target, "the elements must not move");
        assert_eq!(clones.get(), 0);
    }

    #[test]
    fn an_rc_vec_is_adopted_without_touching_its_elements() {
        let clones = Rc::new(Cell::new(0));
        let owner = Rc::new(tracked(3, &clones));
        let target = owner.as_ptr();

        let items = ListItems::from(Rc::clone(&owner));

        assert_eq!(items.as_ptr(), target, "the elements must not move");
        assert_eq!(clones.get(), 0);
    }

    #[test]
    fn an_rc_slice_is_adopted_without_touching_its_elements() {
        let clones = Rc::new(Cell::new(0));
        let owner: Rc<[Tracked]> = tracked(3, &clones).into();
        let target = owner.as_ptr();

        let items = ListItems::from(owner);

        assert_eq!(items.as_ptr(), target, "the elements must not move");
        assert_eq!(clones.get(), 0);
    }

    #[test]
    fn the_shared_handle_outlives_its_original_owner() {
        let owner = Rc::new(vec![7_u32, 8, 9]);
        let items = ListItems::from(Rc::clone(&owner));

        drop(owner);

        assert_eq!(&*items, &[7, 8, 9]);
    }

    #[test]
    fn arrays_ranges_and_iterators_collect_in_order() {
        assert_eq!(&*ListItems::from([3, 1, 2]), &[3, 1, 2]);
        assert_eq!(&*ListItems::from(2..5_u32), &[2, 3, 4]);
        assert_eq!(&*ListItems::from(2..=5_u32), &[2, 3, 4, 5]);
        assert_eq!(
            &*(0..6_u32).filter(|n| n % 2 == 0).collect::<ListItems<_>>(),
            &[0, 2, 4]
        );
    }

    #[test]
    fn empty_and_reversed_sources_are_empty() {
        assert!(ListItems::<u32>::from(Vec::new()).is_empty());
        assert!(ListItems::<u32>::from([]).is_empty());
        assert!(ListItems::from(5..5_u32).is_empty());
        #[allow(clippy::reversed_empty_ranges)]
        let reversed = ListItems::from(5..2_u32);
        assert!(reversed.is_empty());
    }

    #[test]
    fn a_vec_with_spare_capacity_keeps_only_its_elements() {
        let mut vec = Vec::with_capacity(64);
        vec.extend([1_u8, 2, 3]);

        let items = ListItems::from(vec);

        assert_eq!(&*items, &[1, 2, 3]);
    }
}
