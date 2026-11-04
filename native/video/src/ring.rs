//! A few buffers a producer writes in turn and a consumer reads.
//!
//! A decoder shares each picture as a frame that holds an `Arc` of the buffer
//! it names. The producer may write a buffer again only when nothing else holds
//! that `Arc`, which is when the renderer has finished with every frame that
//! named it. The ring hands out such buffers, and grows to its capacity when
//! all of them are busy.

use std::sync::Arc;

/// Buffers (with per-buffer data `E` only the producer needs).
pub(crate) struct Ring<T, E> {
    entries: Vec<(Arc<T>, E)>,
    capacity: usize,
}

impl<T, E> Ring<T, E> {
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            entries: Vec::with_capacity(capacity),
            capacity,
        }
    }

    /// The index of a buffer nobody else holds.
    pub(crate) fn free(&self) -> Option<usize> {
        // The ring's own reference is the one.
        self.entries.iter().position(|(buffer, _)| Arc::strong_count(buffer) == 1)
    }

    /// Whether another buffer may be added.
    pub(crate) fn can_grow(&self) -> bool {
        self.entries.len() < self.capacity
    }

    /// Adds a buffer, returning its index.
    pub(crate) fn push(&mut self, buffer: Arc<T>, extra: E) -> usize {
        self.entries.push((buffer, extra));
        self.entries.len() - 1
    }

    pub(crate) fn get(&self, index: usize) -> &(Arc<T>, E) {
        &self.entries[index]
    }

    /// Removes every buffer, for the producer to release.
    pub(crate) fn take_all(&mut self) -> Vec<(Arc<T>, E)> {
        std::mem::take(&mut self.entries)
    }

    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unused_buffer_is_free_and_a_held_one_is_not() {
        let mut ring: Ring<u32, ()> = Ring::new(2);
        let first = Arc::new(1);
        ring.push(first.clone(), ());
        assert_eq!(ring.free(), None, "a frame still holds it");
        drop(first);
        assert_eq!(ring.free(), Some(0));
    }

    #[test]
    fn the_ring_grows_to_its_capacity_and_no_further() {
        let mut ring: Ring<u32, ()> = Ring::new(2);
        assert!(ring.can_grow());
        let a = Arc::new(1);
        let b = Arc::new(2);
        ring.push(a.clone(), ());
        ring.push(b.clone(), ());
        assert!(!ring.can_grow());
        assert_eq!(ring.len(), 2);
        assert_eq!(ring.free(), None);
    }

    #[test]
    fn the_first_free_buffer_is_chosen() {
        let mut ring: Ring<u32, &str> = Ring::new(3);
        let held = Arc::new(1);
        ring.push(held.clone(), "a");
        ring.push(Arc::new(2), "b");
        ring.push(Arc::new(3), "c");
        let index = ring.free().unwrap();
        assert_eq!(index, 1);
        assert_eq!(ring.get(index).1, "b");
    }

    #[test]
    fn take_all_empties_the_ring() {
        let mut ring: Ring<u32, ()> = Ring::new(2);
        ring.push(Arc::new(1), ());
        assert_eq!(ring.take_all().len(), 1);
        assert_eq!(ring.len(), 0);
        assert!(ring.can_grow());
    }
}
