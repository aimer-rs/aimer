//! Zero-cost type erasure through statically known pointer metadata.

/// Declares that the erased target `Self` can be produced from the concrete
/// type `U` by a plain unsizing coercion.
///
/// Stable Rust exposes neither `CoerceUnsized` nor `Pointee::Metadata` to
/// user code, so an owner cannot derive a `*const dyn Trait` from a
/// `*const Concrete` on its own. This trait supplies the missing piece as a
/// single associated constant: a *template* pointer whose data address is
/// null and whose metadata — the vtable word for a trait object, the element
/// count for a slice — belongs to `U`.
///
/// Because metadata never depends on the value's address, an owner can
/// rebuild a valid `*const Self` at any time by copying the payload address
/// into the template's data word. That removes the indirect adapter call that
/// [`Rubick::new_projected`](crate::Rubick::new_projected) needs, leaving a
/// borrow as cheap as dereferencing a [`Box`].
///
/// The trait is implemented on the *target*, not on the concrete type, so a
/// downstream crate can write one blanket implementation for its own trait
/// object without violating the orphan rules:
///
/// ```
/// use aimer_rubick::{ErasedFrom, Rubick};
///
/// trait Shape {
///     fn area(&self) -> f32;
/// }
///
/// // SAFETY: The template is `null::<S>()` coerced to the target.
/// unsafe impl<S: Shape + 'static> ErasedFrom<S> for dyn Shape {
///     const TEMPLATE: *const Self = std::ptr::null::<S>() as *const dyn Shape;
/// }
///
/// struct Square(f32);
///
/// impl Shape for Square {
///     fn area(&self) -> f32 {
///         self.0 * self.0
///     }
/// }
///
/// let shape: Rubick<dyn Shape> = Rubick::erase(Square(3.0));
/// assert_eq!(shape.area(), 9.0);
/// ```
///
/// # Safety
///
/// `TEMPLATE` must be exactly `core::ptr::null::<U>() as *const Self`, that
/// is, a pointer carrying `U`'s metadata for `Self` and a null data address.
/// Any other value lets an owner build a pointer whose metadata does not
/// describe the value it points at, which is immediate undefined behavior on
/// the next borrow. Never dereference `TEMPLATE` itself.
pub unsafe trait ErasedFrom<U: 'static>: 'static {
    /// A pointer to `Self` with `U`'s metadata and a null data address.
    const TEMPLATE: *const Self;
}

/// Every sized type erases to itself, which makes `Rubick<T>` with a sized
/// target share the metadata-free fast path.
// SAFETY: The template is a null `*const U` and `Self` is `U`, so its
// (empty) metadata trivially describes `U`.
unsafe impl<U: Sized + 'static> ErasedFrom<U> for U {
    const TEMPLATE: *const Self = std::ptr::null::<U>();
}
