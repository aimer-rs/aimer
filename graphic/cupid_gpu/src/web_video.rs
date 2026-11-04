//! Where a browser `<video>` element is found by id.
//!
//! A frame names its texture by number (see
//! [`NativeTextureHandle::WebVideo`](crate::NativeTextureHandle::WebVideo)),
//! so the producer, which owns the element, and the backend, which copies from
//! it, meet here. The table lives on the thread that registered the element:
//! the web has one thread, and a DOM object cannot leave it anyway.
//!
//! The table stores `dyn Any` so this crate stays free of browser bindings;
//! the producer and the backend agree that the value is an
//! `HtmlVideoElement`.

use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

aimer_tls::thread_local! {
    static TABLE: RefCell<Table> = RefCell::new(Table { slots: Vec::new(), free: Vec::new() });
}

struct Table {
    slots: Vec<Option<Rc<dyn Any>>>,
    free: Vec<u32>,
}

/// Keeps an element findable by [`lookup`] while it lives.
///
/// Frames hold it as their owner, so an id stays valid for as long as a frame
/// that names it can still be drawn.
#[derive(Debug)]
pub struct WebVideoRegistration {
    id: u32,
}

// SAFETY: the registration is only a number. The table it clears on drop is
// thread-local, and the web, the only place elements exist, has one thread, so
// there is no other thread to move it to.
unsafe impl Send for WebVideoRegistration {}
unsafe impl Sync for WebVideoRegistration {}

impl WebVideoRegistration {
    /// The id to put in a frame.
    #[inline]
    pub fn id(&self) -> u32 {
        self.id
    }
}

impl Drop for WebVideoRegistration {
    fn drop(&mut self) {
        // `try_with`: the table may already be gone when the thread ends.
        let _ = TABLE.try_with(|table| {
            let mut table = table.borrow_mut();
            if let Some(slot) = table.slots.get_mut(self.id as usize)
                && slot.take().is_some()
            {
                table.free.push(self.id);
            }
        });
    }
}

/// Registers `element` (an `HtmlVideoElement`) and returns its registration.
pub fn register(element: Rc<dyn Any>) -> WebVideoRegistration {
    TABLE.with(|table| {
        let mut table = table.borrow_mut();
        let id = match table.free.pop() {
            Some(id) => {
                table.slots[id as usize] = Some(element);
                id
            }
            None => {
                table.slots.push(Some(element));
                (table.slots.len() - 1) as u32
            }
        };
        WebVideoRegistration { id }
    })
}

/// The element registered under `id`, if it is still registered.
pub fn lookup(id: u32) -> Option<Rc<dyn Any>> {
    TABLE.with(|table| table.borrow().slots.get(id as usize)?.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_element_is_found_until_its_registration_drops() {
        let registration = register(Rc::new(7_u32));
        let id = registration.id();
        let found = lookup(id).expect("registered");
        assert_eq!(found.downcast_ref::<u32>(), Some(&7));
        drop(registration);
        assert!(lookup(id).is_none());
    }

    #[test]
    fn a_freed_id_is_reused_by_the_next_element() {
        let first = register(Rc::new(1_u32));
        let id = first.id();
        drop(first);
        let second = register(Rc::new(2_u32));
        assert_eq!(second.id(), id);
        assert_eq!(lookup(id).unwrap().downcast_ref::<u32>(), Some(&2));
    }

    #[test]
    fn unknown_ids_find_nothing() {
        assert!(lookup(u32::MAX).is_none());
    }
}
