//! Parts of a `build` output that survive from one build to the next.

use std::any::Any;

use crate::key::Key;

use super::BuildConsumer;

/// The retained parts one build scope keeps between its builds.
///
/// A scope holds a handful of parts at most, so a vector scanned by key beats a
/// hash map in both lookup and allocation; the builds that matter most hold one.
#[derive(Default)]
pub(super) struct Slots {
    /// Counts builds of the owning scope, so a slot can tell whether the build
    /// that just ended asked for it.
    build: u64,
    slots: Vec<Slot>,
    /// Values other than widgets that a scope keeps between its builds, such as
    /// a running request. Released by the same rule as `slots`.
    values: Vec<ValueSlot>,
}

struct Slot {
    key: Key,
    /// The build that last asked for this slot.
    requested: u64,
    /// A cheap clone of the part, type-erased because the layer that knows what
    /// a part is lives above this one.
    part: Box<dyn Any>,
}

struct ValueSlot {
    key: Key,
    /// The build that last asked for this slot.
    requested: u64,
    value: Box<dyn Any>,
}

impl Slots {
    /// Starts a build: slots are measured against this build from now on.
    #[inline]
    pub(super) fn begin_build(&mut self) {
        self.build += 1;
    }

    /// Ends a build: a slot this build did not ask for is released, so a part
    /// the owner stopped showing does not keep its element alive.
    ///
    /// Returns the values that were released. Dropping one can run arbitrary
    /// code, such as cancelling a request, so the caller drops them after it has
    /// given up its borrow of these slots.
    #[inline]
    pub(super) fn end_build(&mut self) -> Vec<Box<dyn Any>> {
        let build = self.build;
        if !self.slots.is_empty() {
            self.slots.retain(|slot| slot.requested == build);
        }
        let mut released = Vec::new();
        let mut index = 0;
        while index < self.values.len() {
            if self.values[index].requested == build {
                index += 1;
            } else {
                released.push(self.values.swap_remove(index).value);
            }
        }
        released
    }

    /// Returns the value kept under `key`, if there is one of type `V`, and
    /// counts it as asked for by this build.
    fn get_value<V: Any + Clone>(&mut self, key: &Key) -> Option<V> {
        let build = self.build;
        let slot = self.values.iter_mut().find(|slot| &slot.key == key)?;
        let value = slot.value.downcast_ref::<V>()?.clone();
        slot.requested = build;
        Some(value)
    }

    /// Keeps `value` under `key` for the following builds.
    ///
    /// A value already kept under the key, of any type, is replaced and
    /// returned for the caller to drop.
    fn insert_value<V: Any>(&mut self, key: Key, value: V) -> Option<Box<dyn Any>> {
        let build = self.build;
        if let Some(slot) = self.values.iter_mut().find(|slot| slot.key == key) {
            slot.requested = build;
            return Some(std::mem::replace(&mut slot.value, Box::new(value)));
        }
        self.values.push(ValueSlot {
            key,
            requested: build,
            value: Box::new(value),
        });
        None
    }

    fn get<P: Any + Clone>(&mut self, key: &Key) -> Option<P> {
        let build = self.build;
        let slot = self.slots.iter_mut().find(|slot| &slot.key == key)?;
        let part = slot.part.downcast_ref::<P>()?.clone();
        slot.requested = build;
        Some(part)
    }

    fn insert<P: Any + Clone>(&mut self, key: Key, part: P) {
        self.slots.push(Slot {
            key,
            requested: self.build,
            part: Box::new(part),
        });
    }

    /// Keeps `part` under `key`, replacing a part already kept there.
    ///
    /// The replaced part is returned for the caller to drop: dropping a subtree
    /// can run arbitrary code, which must not happen under this borrow.
    fn replace<P: Any + Clone>(&mut self, key: Key, part: P) -> Option<Box<dyn Any>> {
        let build = self.build;
        if let Some(slot) = self.slots.iter_mut().find(|slot| slot.key == key) {
            slot.requested = build;
            return Some(std::mem::replace(&mut slot.part, Box::new(part)));
        }
        self.insert(key, part);
        None
    }
}

impl BuildConsumer {
    /// Returns the part of type `P` this scope keeps under `key`, counting it
    /// as asked for by the build in progress.
    ///
    /// A part kept under `key` with another type is not returned.
    #[doc(hidden)]
    pub fn retained_part<P: Any + Clone>(&self, key: &Key) -> Option<P> {
        self.retained.borrow_mut().get(key)
    }

    /// Keeps `part` under `key` until a build of this scope stops asking for it
    /// or the scope ends. The caller has checked that no part is kept under
    /// `key` yet.
    #[doc(hidden)]
    pub fn retain_part<P: Any + Clone>(&self, key: Key, part: P) {
        self.retained.borrow_mut().insert(key, part);
    }

    /// Keeps `part` under `key` like [`BuildConsumer::retain_part`], replacing
    /// the part already kept there, if any.
    #[doc(hidden)]
    pub fn replace_part<P: Any + Clone>(&self, key: Key, part: P) {
        let replaced = self.retained.borrow_mut().replace(key, part);
        // Outside the borrow, as for a released value.
        drop(replaced);
    }

    /// Returns the value of type `V` this scope keeps under `key`, counting it
    /// as asked for by the build in progress.
    ///
    /// A value kept under `key` with another type is not returned.
    pub fn retained_value<V: Any + Clone>(&self, key: &Key) -> Option<V> {
        self.retained.borrow_mut().get_value(key)
    }

    /// Keeps `value` under `key` until a build of this scope stops asking for
    /// it or the scope ends.
    pub fn retain_value<V: Any>(&self, key: Key, value: V) {
        let displaced = self.retained.borrow_mut().insert_value(key, value);
        drop(displaced);
    }
}
