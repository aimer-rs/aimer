mod os;

/// Compatibility path for the shared ownership types now implemented by
/// `aimer_rubick`.
///
/// ```
/// use aimer_std::read_only::{Shared, Weak};
///
/// let value = Shared::new(42);
/// let weak: Weak<i32> = Shared::downgrade(&value);
/// assert_eq!(*weak.upgrade().unwrap(), 42);
/// ```
pub use aimer_rubick::shared as read_only;

/// Selects a value based on a condition. Shorthand for `if`-`else` expressions.
///
/// The macro supports two forms:
///
/// - `case!(condition, when_true, when_false)`
/// - `case!(condition, when_true)`, using `Default::default()` when false
///
/// Only the selected expression is evaluated. A trailing comma is optional.
///
/// # Examples
///
/// ```
/// use aimer_std::case;
///
/// assert_eq!(case!(true, 10, 20), 10);
/// assert_eq!(case!(false, 10, 20), 20);
/// ```
///
/// The two-argument form uses the inferred type's default value:
///
/// ```
/// use aimer_std::case;
///
/// assert_eq!(case!(true, 10), 10);
/// assert_eq!(case!(false, 10), 0);
///
/// let value: String = case!(false, String::from("hello"));
/// assert_eq!(value, String::new());
/// ```
///
/// A trailing comma is accepted:
///
/// ```
/// use aimer_std::case;
///
/// let value = case!(
///     true,
///     10,
///     20,
/// );
///
/// let defaulted = case!(
///     false,
///     String::from("hello"),
/// );
/// ```
///
/// Only the selected branch is evaluated:
///
/// ```
/// use aimer_std::case;
///
/// let value = case!(
///     false,
///     panic!("not evaluated"),
///     42,
/// );
///
/// assert_eq!(value, 42);
/// ```
///
/// # Type inference
///
/// In the two-argument form, the true expression or surrounding context must
/// provide enough information to infer the result type:
///
/// ```
/// use aimer_std::case;
///
/// let values: Vec<i32> = case!(false, Vec::new());
/// assert!(values.is_empty());
/// ```
///
/// The result type must implement [`Default`] when using the two-argument form.
#[macro_export]
macro_rules! case {
    // Explicit false expression.
    ($condition:expr, $when_true:expr, $when_false:expr $(,)?) => {{ if $condition { $when_true } else { $when_false } }};

    // Use the inferred result type's default value when false.
    ($condition:expr, $when_true:expr $(,)?) => {{
        if $condition {
            $when_true
        } else {
            ::core::default::Default::default()
        }
    }};
}

#[cfg(test)]
mod tests {
    use crate::read_only::{Shared, Weak};
    use std::cell::{Cell, RefCell};
    use std::mem::{align_of, size_of};
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::rc::Rc;

    #[derive(Debug)]
    struct State {
        apple: u32,
        pineapple: String,
    }

    #[test]
    fn clones_share_one_value_and_track_the_strong_count() {
        let first = Shared::new(String::from("aimer"));
        let second = first.clone();

        assert_eq!(Shared::strong_count(&first), 2);
        assert!(Shared::ptr_eq(&first, &second));
        assert_eq!(&*second, "aimer");

        drop(second);
        assert_eq!(Shared::strong_count(&first), 1);
    }

    #[test]
    fn the_value_is_dropped_exactly_once() {
        struct CountDrops<'a>(&'a Cell<usize>);

        impl Drop for CountDrops<'_> {
            fn drop(&mut self) {
                self.0.set(self.0.get() + 1);
            }
        }

        let drops = Cell::new(0);
        let first = Shared::new(CountDrops(&drops));
        let second = first.clone();

        drop(first);
        assert_eq!(drops.get(), 0);

        drop(second);
        assert_eq!(drops.get(), 1);
    }

    #[test]
    fn zero_sized_values_are_supported() {
        let first = Shared::new(());
        let second = first.clone();

        assert_eq!(*second, ());
        assert_eq!(Shared::strong_count(&first), 2);
    }

    #[test]
    fn the_handle_is_one_pointer_wide_and_respects_over_alignment() {
        #[repr(align(128))]
        struct OverAligned(u8);

        let value = Shared::new(OverAligned(7));
        let address = (&*value as *const OverAligned).addr();

        assert_eq!(size_of::<Shared<u64>>(), size_of::<usize>());
        assert_eq!(size_of::<Weak<u64>>(), size_of::<usize>());
        assert_eq!(address % align_of::<OverAligned>(), 0);
        assert_eq!(value.0, 7);
    }

    #[test]
    fn get_mut_only_succeeds_for_a_unique_owner() {
        let mut value = Shared::new(String::from("pine"));
        Shared::get_mut(&mut value).unwrap().push_str("apple");
        assert_eq!(&*value, "pineapple");

        let clone = value.clone();
        assert!(Shared::get_mut(&mut value).is_none());
        drop(clone);
        assert!(Shared::get_mut(&mut value).is_some());
    }

    #[test]
    fn get_mut_rejects_live_weak_handles() {
        let mut value = Shared::new(String::from("pine"));
        let weak = Shared::downgrade(&value);

        assert!(Shared::get_mut(&mut value).is_none());

        drop(weak);
        assert!(Shared::get_mut(&mut value).is_some());
    }

    #[test]
    fn make_mut_detaches_live_weak_handles() {
        let mut value = Shared::new(String::from("pine"));
        let weak = Shared::downgrade(&value);

        Shared::make_mut(&mut value).push_str("apple");

        assert_eq!(&*value, "pineapple");
        assert!(weak.upgrade().is_none());
        drop(weak);
    }

    #[test]
    fn try_unwrap_moves_out_of_a_unique_allocation() {
        let value = Shared::new(String::from("pineapple"));
        assert_eq!(Shared::try_unwrap(value).unwrap(), "pineapple");
    }

    #[test]
    fn try_unwrap_returns_the_handle_when_it_is_shared() {
        let first = Shared::new(7_u32);
        let second = first.clone();

        let first = Shared::try_unwrap(first).unwrap_err();
        assert_eq!(Shared::strong_count(&first), 2);

        drop(second);
        assert_eq!(Shared::try_unwrap(first), Ok(7));
    }

    #[test]
    fn try_unwrap_releases_the_value_before_weak_allocation() {
        let value = Shared::new(String::from("pineapple"));
        let weak = Shared::downgrade(&value);

        assert_eq!(Shared::try_unwrap(value), Ok(String::from("pineapple")));
        assert_eq!(Weak::strong_count(&weak), 0);
        assert!(weak.upgrade().is_none());

        drop(weak);
    }

    #[test]
    fn a_projection_keeps_the_owner_alive() {
        let state = Shared::new(State {
            apple: 42,
            pineapple: String::from("golden pineapple"),
        });
        let pineapple = state.project(|state: &State| &state.pineapple);

        drop(state);

        assert_eq!(&*pineapple, "golden pineapple");
        assert_eq!(pineapple.get(), "golden pineapple");
    }

    #[test]
    fn weak_handles_do_not_keep_the_value_alive() {
        struct CountDrops<'a>(&'a Cell<usize>);

        impl Drop for CountDrops<'_> {
            fn drop(&mut self) {
                self.0.set(self.0.get() + 1);
            }
        }

        let drops = Cell::new(0);
        let value = Shared::new(CountDrops(&drops));
        let weak = Shared::downgrade(&value);

        assert_eq!(Shared::weak_count(&value), 1);
        assert_eq!(Weak::strong_count(&weak), 1);
        assert!(weak.upgrade().is_some());

        drop(value);

        assert_eq!(drops.get(), 1);
        assert_eq!(Weak::strong_count(&weak), 0);
        assert!(weak.upgrade().is_none());

        drop(weak);
    }

    #[test]
    fn weak_allocation_survives_a_panicking_value_drop() {
        struct PanicOnDrop;

        impl Drop for PanicOnDrop {
            fn drop(&mut self) {
                panic!("drop failed");
            }
        }

        let value = Shared::new(PanicOnDrop);
        let weak = Shared::downgrade(&value);
        let result = catch_unwind(AssertUnwindSafe(|| drop(value)));

        assert!(result.is_err());
        assert_eq!(Weak::strong_count(&weak), 0);
        assert!(weak.upgrade().is_none());

        drop(weak);
    }

    #[test]
    fn weak_back_edges_allow_a_parent_and_child_to_drop() {
        struct Node {
            child: RefCell<Option<Shared<Node>>>,
            _parent: RefCell<Option<Weak<Node>>>,
            drops: Rc<Cell<usize>>,
        }

        impl Drop for Node {
            fn drop(&mut self) {
                self.drops.set(self.drops.get() + 1);
            }
        }

        let drops = Rc::new(Cell::new(0));
        let parent = Shared::new(Node {
            child: RefCell::new(None),
            _parent: RefCell::new(None),
            drops: Rc::clone(&drops),
        });
        let child = Shared::new(Node {
            child: RefCell::new(None),
            _parent: RefCell::new(Some(Shared::downgrade(&parent))),
            drops: Rc::clone(&drops),
        });

        *parent.child.borrow_mut() = Some(child.clone());
        drop(child);
        drop(parent);

        assert_eq!(drops.get(), 2);
    }

    #[test]
    fn separate_fields_can_be_projected_from_one_owner() {
        let state = Shared::new(State {
            apple: 42,
            pineapple: String::from("pineapple"),
        });
        let apple = state.project(|state: &State| &state.apple);
        let pineapple = state.project(|state: &State| &state.pineapple);

        assert_eq!(*apple, 42);
        assert_eq!(&*pineapple, "pineapple");
        assert_eq!(Shared::strong_count(&state), 3);
    }

    #[test]
    fn cloning_a_projection_clones_its_ownership() {
        let state = Shared::new(State {
            apple: 42,
            pineapple: String::from("pineapple"),
        });
        let field = state.project(|state: &State| &state.pineapple);
        let clone = field.clone();

        drop(state);
        drop(field);

        assert_eq!(&*clone, "pineapple");
    }

    #[test]
    fn make_mut_uses_copy_on_write() {
        let mut first = Shared::new(String::from("pine"));
        let second = first.clone();

        Shared::make_mut(&mut first).push_str("apple");

        assert_eq!(&*first, "pineapple");
        assert_eq!(&*second, "pine");
        assert!(!Shared::ptr_eq(&first, &second));
    }
}
