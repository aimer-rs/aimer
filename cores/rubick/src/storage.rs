//! Raw payload storage shared by inline and heap owners.

use std::mem::MaybeUninit;
use std::ptr::NonNull;

/// A word-addressed payload buffer of `WORDS` machine words.
///
/// The buffer serves both storage modes. An inline owner keeps its concrete
/// value in the whole buffer, while a heap owner keeps the allocation pointer
/// in the first word and leaves the remaining words untouched. Because the
/// mode is a compile-time property of the stored concrete type, it lives in
/// the owner's static operation table instead of a runtime discriminant, so
/// this buffer needs no tag of its own.
///
/// `WORDS` must be at least one, otherwise the buffer cannot hold the heap
/// pointer. Owners assert this at compile time.
#[repr(C)]
pub(crate) struct InlineStorage<const WORDS: usize> {
    words: [MaybeUninit<*mut u8>; WORDS],
}

impl<const WORDS: usize> InlineStorage<WORDS> {
    /// Payload bytes available without a separate allocation.
    pub(crate) const CAPACITY: usize = WORDS * size_of::<*mut u8>();

    /// Creates an uninitialized buffer.
    #[inline]
    pub(crate) const fn uninit() -> Self {
        Self {
            words: [MaybeUninit::uninit(); WORDS],
        }
    }

    /// Returns the address of the first payload byte.
    #[inline]
    pub(crate) fn as_ptr(&self) -> *const u8 {
        self.words.as_ptr().cast()
    }

    /// Returns the mutable address of the first payload byte.
    #[inline]
    pub(crate) fn as_mut_ptr(&mut self) -> *mut u8 {
        self.words.as_mut_ptr().cast()
    }

    /// Reads the heap allocation pointer from the first word.
    ///
    /// # Safety
    ///
    /// The owner must be in heap mode, so the first word must hold a pointer
    /// written by [`InlineStorage::set_pointer`].
    #[inline]
    pub(crate) unsafe fn pointer(&self) -> *mut u8 {
        // SAFETY: Heap mode initializes the first word with a valid pointer.
        unsafe { self.words[0].assume_init() }
    }

    /// Stores the heap allocation pointer in the first word.
    #[inline]
    pub(crate) fn set_pointer(&mut self, pointer: NonNull<u8>) {
        self.words[0] = MaybeUninit::new(pointer.as_ptr());
    }
}
