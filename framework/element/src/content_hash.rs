//! Content hashes: a cheap fingerprint of what a widget or element was
//! configured with.
//!
//! `Widget::hash` and [`Element::hash`](crate::components::element::Element::hash)
//! return `Option<ContentHash>`. `None` means "no hash is available" and is never
//! taken to equal another `None`; `Some` hashes are compared for equality, so
//! the one thing a hash must never do is be *equal* for two different
//! configurations. Two equal configurations hashing differently only costs a
//! rebuild, which makes that the safe direction to be wrong in.
//!
//! A widget builds its hash with a [`ContentHasher`]:
//!
//! ```
//! use aimer_element::content_hash::{ContentHash, ContentHasher};
//! use std::hash::Hash;
//!
//! struct Badge {
//!     count: u32,
//!     scale: f32,
//! }
//!
//! impl Badge {
//!     fn content_hash(&self) -> Option<ContentHash> {
//!         let mut hasher = ContentHasher::new::<Self>();
//!         self.count.hash(&mut hasher);
//!         hasher.write_f32(self.scale);
//!         hasher.finish_hash()
//!     }
//! }
//!
//! let a = Badge { count: 1, scale: 1.0 };
//! let b = Badge { count: 2, scale: 1.0 };
//! assert_eq!(a.content_hash(), a.content_hash());
//! assert_ne!(a.content_hash(), b.content_hash());
//! ```

use std::any::TypeId;
use std::hash::{Hash, Hasher};
use std::num::NonZeroU64;

/// A 64-bit fingerprint of a configuration.
///
/// Never zero, so `Option<ContentHash>` is one word and "no hash" needs no
/// reserved value. Fixed-width on every target, unlike `usize`, whose 32 bits on
/// wasm32 would make a collision, which shows a stale part, far likelier.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ContentHash(NonZeroU64);

impl ContentHash {
    /// The fingerprint as a number.
    #[inline]
    pub const fn get(self) -> u64 {
        self.0.get()
    }
}

/// Builds a [`ContentHash`].
///
/// It implements [`Hasher`], so anything that is `Hash` — integers, enums that
/// derive it, `&str`, `Color` — is hashed with `value.hash(&mut hasher)`. Floats
/// are not `Hash`; use [`ContentHasher::write_f32`]. A child widget contributes
/// through [`ContentHasher::write_child`], which makes the whole hash `None` as
/// soon as one child has none.
///
/// The hasher is seeded with the type it hashes for, so two widget types with
/// the same field values never share a hash.
pub struct ContentHasher {
    state: u64,
    /// Set when a child had no hash, which leaves this one without too.
    unavailable: bool,
}

impl ContentHasher {
    const MULTIPLIER: u64 = 0x9e37_79b9_7f4a_7c15;

    /// Starts a hash for configurations of type `T`.
    #[inline]
    pub fn new<T: 'static>() -> Self {
        let mut hasher = Self {
            state: 0x243f_6a88_85a3_08d3,
            unavailable: false,
        };
        TypeId::of::<T>().hash(&mut hasher);
        hasher
    }

    /// Starts a hash seeded by a name instead of a type.
    ///
    /// For a generic whose parameters are not `'static`, which a `TypeId` cannot
    /// name. The name must be unique to the kind of configuration being hashed,
    /// and the parameters' own hashes belong in through
    /// [`ContentHasher::write_child`].
    #[inline]
    pub fn named(name: &'static str) -> Self {
        let mut hasher = Self {
            state: 0x243f_6a88_85a3_08d3,
            unavailable: false,
        };
        name.hash(&mut hasher);
        hasher
    }

    #[inline]
    fn mix(&mut self, word: u64) {
        self.state = (self.state.rotate_left(23) ^ word).wrapping_mul(Self::MULTIPLIER);
    }

    /// Adds a float by its bits, with the sign of zero dropped so that numbers
    /// which compare equal hash equal.
    #[inline]
    pub fn write_f32(&mut self, value: f32) {
        self.mix(u64::from((value + 0.0).to_bits()));
    }

    /// Adds the identity of a shared object: its address, nothing of its content.
    ///
    /// For handles that keep their identity across rebuilds, such as a controller
    /// or a retained child. A copy made elsewhere is a different object, which
    /// is the safe answer. The address cannot be reused while something that
    /// holds the object is alive, and the part a hash was taken for holds it.
    #[inline]
    pub fn write_identity<T: ?Sized>(&mut self, object: *const T) {
        self.mix(object as *const () as usize as u64);
    }

    /// Adds a child's hash. A child without one leaves this hash without one.
    #[inline]
    pub fn write_child(&mut self, child: Option<ContentHash>) {
        match child {
            Some(hash) => self.mix(hash.get()),
            None => self.unavailable = true,
        }
    }

    /// Marks this configuration as one that cannot be hashed, for example because
    /// it holds a closure.
    #[inline]
    pub fn mark_unavailable(&mut self) {
        self.unavailable = true;
    }

    /// Finishes the hash, or `None` when a child or the caller said none exists.
    #[inline]
    pub fn finish_hash(self) -> Option<ContentHash> {
        if self.unavailable {
            return None;
        }
        // A finalizer with full avalanche, so the weak per-word mixing above
        // does not leave structure in the bits that get compared.
        let mut value = self.state;
        value ^= value >> 33;
        value = value.wrapping_mul(0xff51_afd7_ed55_8ccd);
        value ^= value >> 33;
        value = value.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
        value ^= value >> 33;
        // Zero is the one value a `NonZeroU64` cannot hold.
        Some(ContentHash(NonZeroU64::new(value).unwrap_or(NonZeroU64::MIN)))
    }
}

impl Hasher for ContentHasher {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        let mut chunks = bytes.chunks_exact(8);
        for chunk in &mut chunks {
            self.mix(u64::from_ne_bytes(chunk.try_into().expect("a chunk of eight bytes")));
        }
        let rest = chunks.remainder();
        if !rest.is_empty() {
            let mut last = [0u8; 8];
            last[..rest.len()].copy_from_slice(rest);
            // The length keeps `[1]` and `[1, 0]` apart.
            self.mix(u64::from_ne_bytes(last) ^ ((rest.len() as u64) << 56));
        }
    }

    #[inline]
    fn write_u8(&mut self, value: u8) {
        self.mix(u64::from(value));
    }

    #[inline]
    fn write_u32(&mut self, value: u32) {
        self.mix(u64::from(value));
    }

    #[inline]
    fn write_u64(&mut self, value: u64) {
        self.mix(value);
    }

    #[inline]
    fn write_usize(&mut self, value: usize) {
        self.mix(value as u64);
    }

    /// The running state, not the finished hash; use
    /// [`ContentHasher::finish_hash`] for that.
    #[inline]
    fn finish(&self) -> u64 {
        self.state
    }
}

/// A value that can feed a [`ContentHasher`] by what it holds.
///
/// `Hash` does not cover floats, and a configuration is mostly numbers, so this
/// is the trait a field of a hashed configuration implements. Floats are hashed
/// as [`ContentHasher::write_f32`] does, so values that compare equal hash equal.
/// Types that are already `Hash` get an implementation through
/// [`impl_hash_content_via_hash!`].
pub trait HashContent {
    /// Adds this value to `hasher`.
    fn hash_content(&self, hasher: &mut ContentHasher);
}

impl HashContent for f32 {
    #[inline]
    fn hash_content(&self, hasher: &mut ContentHasher) {
        hasher.write_f32(*self);
    }
}

impl HashContent for f64 {
    #[inline]
    fn hash_content(&self, hasher: &mut ContentHasher) {
        // Adding zero drops the sign of zero, as for `f32`.
        hasher.write_u64((*self + 0.0).to_bits());
    }
}

/// Implements [`HashContent`] for types that are `Hash`, by hashing them.
#[macro_export]
macro_rules! impl_hash_content_via_hash {
    ($($type:ty),+ $(,)?) => {
        $(
            impl $crate::content_hash::HashContent for $type {
                #[inline]
                fn hash_content(&self, hasher: &mut $crate::content_hash::ContentHasher) {
                    ::std::hash::Hash::hash(self, hasher);
                }
            }
        )+
    };
}

impl_hash_content_via_hash!(
    bool, char, u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize, (), str, String
);

impl_hash_content_via_hash!(aimer_color::prelude::Color, aimer_color::prelude::Colors);

impl<T: HashContent + ?Sized> HashContent for &T {
    #[inline]
    fn hash_content(&self, hasher: &mut ContentHasher) {
        (**self).hash_content(hasher);
    }
}

impl<T: HashContent> HashContent for Option<T> {
    fn hash_content(&self, hasher: &mut ContentHasher) {
        hasher.write_u8(u8::from(self.is_some()));
        if let Some(value) = self {
            value.hash_content(hasher);
        }
    }
}

impl<T: HashContent> HashContent for [T] {
    fn hash_content(&self, hasher: &mut ContentHasher) {
        hasher.write_usize(self.len());
        for value in self {
            value.hash_content(hasher);
        }
    }
}

impl<T: HashContent, const N: usize> HashContent for [T; N] {
    #[inline]
    fn hash_content(&self, hasher: &mut ContentHasher) {
        self.as_slice().hash_content(hasher);
    }
}

impl<T: HashContent> HashContent for Vec<T> {
    #[inline]
    fn hash_content(&self, hasher: &mut ContentHasher) {
        self.as_slice().hash_content(hasher);
    }
}

macro_rules! tuple_hash_content {
    ($($name:ident),+) => {
        impl<$($name: HashContent),+> HashContent for ($($name,)+) {
            #[allow(non_snake_case)]
            fn hash_content(&self, hasher: &mut ContentHasher) {
                let ($($name,)+) = self;
                $($name.hash_content(hasher);)+
            }
        }
    };
}
tuple_hash_content!(A);
tuple_hash_content!(A, B);
tuple_hash_content!(A, B, C);
tuple_hash_content!(A, B, C, D);

#[cfg(test)]
mod tests {
    use super::*;

    struct A;
    struct B;

    fn of<T: 'static>(build: impl FnOnce(&mut ContentHasher)) -> Option<ContentHash> {
        let mut hasher = ContentHasher::new::<T>();
        build(&mut hasher);
        hasher.finish_hash()
    }

    #[test]
    fn plain_values_hash_their_content() {
        let hash = |value: &dyn Fn(&mut ContentHasher)| {
            let mut hasher = ContentHasher::new::<A>();
            value(&mut hasher);
            hasher.finish_hash()
        };
        assert_eq!(hash(&|h| 1.5f32.hash_content(h)), hash(&|h| 1.5f32.hash_content(h)));
        assert_ne!(hash(&|h| 1.5f32.hash_content(h)), hash(&|h| 2.5f32.hash_content(h)));
        assert_eq!(hash(&|h| 0.0f32.hash_content(h)), hash(&|h| (-0.0f32).hash_content(h)));
        assert_ne!(hash(&|h| 1i32.hash_content(h)), hash(&|h| 2i32.hash_content(h)));
        assert_ne!(hash(&|h| true.hash_content(h)), hash(&|h| false.hash_content(h)));
        assert_ne!(hash(&|h| (1.0f32, 2.0f32).hash_content(h)), hash(&|h| (2.0f32, 1.0f32).hash_content(h)));
        assert_ne!(hash(&|h| Some(1.0f32).hash_content(h)), hash(&|h| None::<f32>.hash_content(h)));
        assert_ne!(hash(&|h| [1.0f32, 2.0].hash_content(h)), hash(&|h| [1.0f32, 3.0].hash_content(h)));
    }

    #[test]
    fn equal_input_gives_an_equal_hash() {
        let first = of::<A>(|h| (1u32, "x").hash(h));
        let second = of::<A>(|h| (1u32, "x").hash(h));
        assert!(first.is_some());
        assert_eq!(first, second);
    }

    #[test]
    fn different_input_gives_a_different_hash() {
        assert_ne!(of::<A>(|h| 1u32.hash(h)), of::<A>(|h| 2u32.hash(h)));
        assert_ne!(of::<A>(|h| "ab".hash(h)), of::<A>(|h| "ac".hash(h)));
        assert_ne!(of::<A>(|h| h.write_f32(1.0)), of::<A>(|h| h.write_f32(1.5)));
    }

    #[test]
    fn a_name_seeds_like_a_type_does() {
        let named = |name| {
            let mut hasher = ContentHasher::named(name);
            1u32.hash(&mut hasher);
            hasher.finish_hash()
        };
        assert_eq!(named("a"), named("a"));
        assert_ne!(named("a"), named("b"));
        assert_ne!(named("a"), of::<A>(|h| 1u32.hash(h)));
    }

    #[test]
    fn the_same_fields_of_another_type_hash_differently() {
        assert_ne!(of::<A>(|h| 1u32.hash(h)), of::<B>(|h| 1u32.hash(h)));
    }

    #[test]
    fn negative_zero_hashes_like_zero() {
        assert_eq!(of::<A>(|h| h.write_f32(0.0)), of::<A>(|h| h.write_f32(-0.0)));
    }

    #[test]
    fn field_boundaries_matter() {
        assert_ne!(
            of::<A>(|h| ("ab", "c").hash(h)),
            of::<A>(|h| ("a", "bc").hash(h))
        );
    }

    #[test]
    fn a_child_without_a_hash_leaves_the_parent_without_one() {
        let child = of::<B>(|h| 1u32.hash(h));
        assert!(of::<A>(|h| h.write_child(child)).is_some());
        assert!(of::<A>(|h| h.write_child(None)).is_none());
        assert!(of::<A>(|h| h.mark_unavailable()).is_none());
    }

    #[test]
    fn a_child_changes_the_parent() {
        let one = of::<B>(|h| 1u32.hash(h));
        let two = of::<B>(|h| 2u32.hash(h));
        assert_ne!(
            of::<A>(|h| h.write_child(one)),
            of::<A>(|h| h.write_child(two))
        );
    }

    /// Hashing small consecutive integers, the shape of real configurations,
    /// must not collide.
    #[test]
    fn consecutive_values_do_not_collide() {
        let mut seen = std::collections::HashSet::new();
        for value in 0..50_000u32 {
            assert!(seen.insert(of::<A>(|h| value.hash(h)).expect("hashable")));
        }
    }
}
