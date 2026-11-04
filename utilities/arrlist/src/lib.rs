#![doc = include_str!("../README.md")]

#[doc(hidden)]
pub mod __private {
    pub use aimer_widget_core::AnyWidget;
}

/// Builds a `Vec<AnyWidget>` from widgets of any type.
///
/// Every item is erased with [`AnyWidget::new`](aimer_widget_core::AnyWidget::new),
/// so the items may all have different types. Small widgets stay inline in the
/// handle. The vector is allocated once with exact capacity. A trailing comma is
/// accepted, and `list![]` is an empty `Vec<AnyWidget>`.
///
/// When every item has the same type, prefer a plain array such as
/// `[A, A, A]`, which erases nothing.
///
/// # Examples
///
/// ```
/// use aimer_arrlist::list;
/// use aimer_widget_core::base::BuildContext;
/// use aimer_widget_core::{AnyElement, AnyWidget, Widget};
///
/// struct A;
/// struct B;
/// # impl Widget for A {
/// #     fn to_element(self, _: &BuildContext<'_>) -> AnyElement { unreachable!() }
/// # }
/// # impl Widget for B {
/// #     fn to_element(self, _: &BuildContext<'_>) -> AnyElement { unreachable!() }
/// # }
///
/// let widgets: Vec<AnyWidget> = list![A, B, A];
/// assert_eq!(widgets.len(), 3);
/// ```
#[macro_export]
macro_rules! list {
    () => {
        ::std::vec::Vec::<$crate::__private::AnyWidget>::new()
    };
    ($($item:expr),+ $(,)?) => {{
        // `$crate::__private` keeps the caller from importing `Widget` or
        // `AnyWidget` just to use the macro.
        let mut items = ::std::vec::Vec::<$crate::__private::AnyWidget>::with_capacity(
            $crate::list!(@count $($item),+),
        );
        $(items.push($crate::__private::AnyWidget::new($item));)+
        items
    }};
    // Counts the items at compile time so `with_capacity` is exact.
    (@count) => { 0usize };
    (@count $head:expr $(, $tail:expr)*) => { 1usize + $crate::list!(@count $($tail),*) };
}

#[cfg(test)]
mod tests {
    use aimer_widget_core::base::BuildContext;
    use aimer_widget_core::{AnyElement, AnyWidget, Widget};

    struct A(u8);
    struct B;

    impl Widget for A {
        fn to_element(self, _: &BuildContext<'_>) -> AnyElement {
            unreachable!()
        }
    }
    impl Widget for B {
        fn to_element(self, _: &BuildContext<'_>) -> AnyElement {
            unreachable!()
        }
    }

    #[test]
    fn empty_is_an_any_widget_vec() {
        let v = list![];
        let _: &Vec<AnyWidget> = &v;
        assert!(v.is_empty());
    }

    #[test]
    fn same_type_is_erased_in_order_with_exact_capacity() {
        let v = list![A(1), A(2), A(3)];
        assert_eq!(v.len(), 3);
        assert_eq!(v.capacity(), 3);
    }

    #[test]
    fn mixed_types_are_erased() {
        let v = list![A(1), B, A(2)];
        let _: &Vec<AnyWidget> = &v;
        assert_eq!(v.len(), 3);
        assert_eq!(v.capacity(), 3);
    }

    #[test]
    fn trailing_comma_is_accepted() {
        assert_eq!(list![A(1), B,].len(), 2);
    }

    #[test]
    fn items_are_evaluated_once_in_order() {
        let mut order = Vec::new();
        let _ = list![
            {
                order.push(1);
                A(1)
            },
            {
                order.push(2);
                B
            },
        ];
        assert_eq!(order, [1, 2]);
    }
}
