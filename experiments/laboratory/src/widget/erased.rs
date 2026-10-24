//! Type erasure for a widget that is consumed by its conversion.
//!
//! A consuming `fn to_element(self)` is not callable through a trait object:
//! `dyn Widget` has no size, so it cannot be passed by value. The tree still
//! needs a uniform handle for children, which leaves exactly one way out —
//! move the concrete value out of its storage behind the erasure, then call
//! the concrete method on the moved value.
//!
//! That move is split in two:
//!
//! - [`DynWidget::to_element_in_place`] reads the value out of a pointer whose
//!   concrete type it *does* know, because it is implemented for every sized
//!   `W: Widget` rather than for `dyn Widget`;
//! - [`aimer_rubick::Rubick::take`] makes that read sound: it marks the
//!   storage vacant beforehand, so the value is never destroyed a second time,
//!   and it returns a pooled block afterwards, so nothing leaks.
//!
//! Widget authors never see either one. They write a plain, safe
//! [`Widget::to_element`](crate::Widget::to_element).

use std::ptr;

use aimer_rubick::{ErasedFrom, Rubick};

use crate::element::AnyElement;
use crate::widget::Widget;

/// Machine words an [`AnyWidget`] reserves for its payload.
///
/// Widgets are wider than elements — a container carries decoration, padding,
/// and a child handle — and every byte over the limit becomes a pooled
/// allocation on every build. Eight words keeps the common nodes inline.
const WIDGET_WORDS: usize = 8;

/// The object-safe half of [`Widget`].
///
/// This trait is an implementation detail of the erasure and is never written
/// by hand: the blanket implementation below covers every widget. It exists
/// only because the safe, consuming method cannot appear in a vtable.
#[doc(hidden)]
pub trait DynWidget: 'static {
    /// Moves the widget out of `self` and converts it.
    ///
    /// # Safety
    ///
    /// The pointee is left uninitialized and must never be dropped or read
    /// again. The only caller is [`AnyWidget::into_element`], which marks the
    /// storage vacant before this runs.
    unsafe fn to_element_in_place(&mut self) -> AnyElement;

    /// Forwards [`Widget::debug_name`].
    fn debug_name(&self) -> &'static str;
}

impl<W: Widget> DynWidget for W {
    #[inline]
    unsafe fn to_element_in_place(&mut self) -> AnyElement {
        // SAFETY: The caller guarantees the storage is already vacant, so this
        // bit copy is the single remaining owner of the widget. The borrow is
        // exclusive, so no other reader observes the uninitialized bytes.
        let widget = unsafe { ptr::read(self as *mut W) };
        widget.to_element()
    }

    #[inline]
    fn debug_name(&self) -> &'static str {
        Widget::debug_name(self)
    }
}

// SAFETY: The template is `null::<W>()` coerced to the target, so it carries
// exactly `W`'s vtable and a null data address.
unsafe impl<W: DynWidget> ErasedFrom<W> for dyn DynWidget {
    const TEMPLATE: *const Self = ptr::null::<W>() as *const dyn DynWidget;
}

/// An owned, type-erased widget.
///
/// The handle owns its widget exclusively — there is no reference counting and
/// no shared access — which is what makes [`AnyWidget::into_element`] able to
/// give the widget away instead of copying it.
pub struct AnyWidget(Rubick<dyn DynWidget, WIDGET_WORDS>);

impl AnyWidget {
    /// Erases a concrete widget.
    #[inline]
    pub fn new<W: Widget>(widget: W) -> Self {
        Self(Rubick::erase(widget))
    }

    /// Consumes this handle and builds the widget's element.
    ///
    /// The widget is moved out of its storage, converted, and the storage is
    /// returned to the pool. Neither the widget nor any of its fields is
    /// cloned, and the destructor of the widget runs exactly once — inside
    /// [`Widget::to_element`], on whatever that method chooses not to keep.
    #[inline]
    pub fn into_element(self) -> AnyElement {
        // SAFETY: `take` marks the storage vacant before the closure runs and
        // releases the block afterwards, on both the normal and the unwinding
        // path. The closure moves the widget out exactly once and never drops
        // it in place, which is the contract `take` requires.
        unsafe { self.0.take(|widget| (*widget).to_element_in_place()) }
    }

    /// Returns the erased widget's diagnostic name.
    #[inline]
    pub fn debug_name(&self) -> &'static str {
        self.0.debug_name()
    }

    /// Returns `true` when the widget needs no separate allocation.
    #[inline]
    pub fn is_inline(&self) -> bool {
        self.0.is_inline()
    }

    /// Returns the address of the erased widget, for storage assertions.
    #[cfg(test)]
    fn payload_address(&self) -> *const u8 {
        (&*self.0) as *const dyn DynWidget as *const u8
    }
}

/// An erased widget is itself a widget, so handles nest without a special
/// case in the tree.
impl Widget for AnyWidget {
    #[inline]
    fn to_element(self) -> AnyElement {
        self.into_element()
    }

    #[inline]
    fn debug_name(&self) -> &'static str {
        AnyWidget::debug_name(self)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::rc::Rc;

    use super::*;
    use crate::element::Element;

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

    /// A widget that is deliberately neither `Clone` nor `Copy`.
    struct Moved {
        recorder: DropRecorder,
        label: String,
    }

    /// Keeps the widget's fields alive, which is the only thing the ownership
    /// experiment asks of an element.
    struct MovedElement {
        _recorder: DropRecorder,
        _label: String,
    }

    impl Element for MovedElement {
        fn debug_name(&self) -> &'static str {
            "MovedElement"
        }
    }

    impl Widget for Moved {
        fn to_element(self) -> AnyElement {
            LABEL_ADDRESS.with(|address| address.set(self.label.as_ptr()));
            AnyElement::new(MovedElement {
                _recorder: self.recorder,
                _label: self.label,
            })
        }
    }

    thread_local! {
        /// Address of the last label seen by `Moved::to_element`.
        static LABEL_ADDRESS: Cell<*const u8> = const { Cell::new(ptr::null()) };
    }

    /// A widget too wide for inline storage, so its payload is pooled.
    struct Wide {
        recorder: DropRecorder,
        _bytes: [usize; WIDGET_WORDS],
    }

    struct WideElement {
        _recorder: DropRecorder,
    }

    impl Element for WideElement {
        fn debug_name(&self) -> &'static str {
            "WideElement"
        }
    }

    impl Widget for Wide {
        fn to_element(self) -> AnyElement {
            AnyElement::new(WideElement {
                _recorder: self.recorder,
            })
        }
    }

    /// A widget whose conversion fails after the move has happened.
    struct Failing {
        _recorder: DropRecorder,
        _bytes: [usize; WIDGET_WORDS],
    }

    impl Widget for Failing {
        fn to_element(self) -> AnyElement {
            panic!("exercise an unwind out of to_element");
        }
    }

    fn recorder(drops: &Rc<Cell<usize>>) -> DropRecorder {
        DropRecorder {
            drops: Rc::clone(drops),
        }
    }

    #[test]
    fn building_moves_the_widget_instead_of_cloning_it() {
        let drops = Rc::new(Cell::new(0));
        let widget = AnyWidget::new(Moved {
            recorder: recorder(&drops),
            label: String::from("Aimer"),
        });

        let element = widget.into_element();

        assert_eq!(element.debug_name(), "MovedElement");
        assert_eq!(
            drops.get(),
            0,
            "the widget's fields were moved into the element, not copied and destroyed"
        );
        drop(element);
        assert_eq!(drops.get(), 1, "the moved value is destroyed exactly once");
    }

    #[test]
    fn a_dropped_handle_still_destroys_its_widget_once() {
        let drops = Rc::new(Cell::new(0));
        drop(AnyWidget::new(Moved {
            recorder: recorder(&drops),
            label: String::from("unused"),
        }));

        assert_eq!(drops.get(), 1);
    }

    #[test]
    fn building_a_pooled_widget_returns_its_block() {
        let drops = Rc::new(Cell::new(0));
        let widget = AnyWidget::new(Wide {
            recorder: recorder(&drops),
            _bytes: [0; WIDGET_WORDS],
        });
        assert!(
            !widget.is_inline(),
            "the fixture must exercise heap storage"
        );
        let block = widget.payload_address();

        let element = widget.into_element();
        assert_eq!(element.debug_name(), "WideElement");
        assert!(
            element.is_inline(),
            "the element must not compete for the widget's block"
        );

        let rebuilt = AnyWidget::new(Wide {
            recorder: recorder(&drops),
            _bytes: [0; WIDGET_WORDS],
        });

        assert_eq!(
            rebuilt.payload_address(),
            block,
            "a consumed widget must hand its block back to the pool"
        );
        assert_eq!(drops.get(), 0);
    }

    #[test]
    fn a_panicking_conversion_destroys_the_widget_exactly_once() {
        let drops = Rc::new(Cell::new(0));
        let result = catch_unwind(AssertUnwindSafe({
            let drops = Rc::clone(&drops);
            move || {
                AnyWidget::new(Failing {
                    _recorder: recorder(&drops),
                    _bytes: [0; WIDGET_WORDS],
                })
                .into_element()
            }
        }));

        assert!(result.is_err());
        assert_eq!(
            drops.get(),
            1,
            "the moved widget is destroyed by the unwind and never again by its storage"
        );
    }

    #[test]
    fn nested_handles_collapse_into_one_element() {
        let drops = Rc::new(Cell::new(0));
        let widget = AnyWidget::new(AnyWidget::new(Moved {
            recorder: recorder(&drops),
            label: String::from("nested"),
        }));

        let element = widget.into_element();

        assert_eq!(element.debug_name(), "MovedElement");
        assert_eq!(drops.get(), 0);
    }

    #[test]
    fn the_widget_payload_reaches_the_element_untouched() {
        let drops = Rc::new(Cell::new(0));
        let widget = Moved {
            recorder: recorder(&drops),
            label: String::from("payload"),
        };
        let address = widget.label.as_ptr();

        let element = widget.boxed().into_element();

        assert_eq!(element.debug_name(), "MovedElement");
        assert_eq!(
            LABEL_ADDRESS.with(Cell::get),
            address,
            "the string buffer travelled through the erasure without being copied"
        );
        assert_eq!(drops.get(), 0);
    }
}
