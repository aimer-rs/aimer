use std::any::Any;
use std::cell::RefCell;
use std::marker::PhantomData;
use std::rc::Rc;

const INITIAL_GENERATION: u64 = 1;

/// A generation-qualified index into the UI thread's state-slot arena.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct StateSlotKey {
    index: u32,
    generation: u64,
}

/// Errors returned when a copied updater generation cannot resolve its slot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum StateSlotResolutionError {
    /// The arena no longer contains the requested slot index.
    Missing,
    /// The slot index exists, but its generation is no longer current.
    Stale,
    /// The current generation has no value, such as during a consumed state.
    Consumed,
    /// The slot exists, but its erased value has a different concrete type.
    WrongType,
}

/// A non-owning, copyable generation key for an erased state-slot value.
pub(super) struct StateUpdaterGeneration<T: ?Sized> {
    key: StateSlotKey,
    _marker: PhantomData<fn() -> T>,
}

impl<T: ?Sized> Copy for StateUpdaterGeneration<T> {}

impl<T: ?Sized> Clone for StateUpdaterGeneration<T> {
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: ?Sized> StateUpdaterGeneration<T> {
    #[inline]
    fn from_key(key: StateSlotKey) -> Self {
        Self {
            key,
            _marker: PhantomData,
        }
    }

    #[inline]
    #[cfg(test)]
    pub(super) fn key(self) -> StateSlotKey {
        self.key
    }
}

impl<T: Any + 'static> StateUpdaterGeneration<T> {
    /// Resolves this generation and runs `callback` while the arena borrow is held.
    #[inline]
    pub(super) fn try_resolve<R>(
        self,
        callback: impl FnOnce(&T) -> R,
    ) -> Result<R, StateSlotResolutionError> {
        with_arena(|arena| {
            let value = arena.resolve::<T>(self.key)?;
            Ok(callback(value))
        })
    }

    /// Resolves this generation without checking the slot index, generation,
    /// value presence, or erased value type.
    ///
    /// # Safety
    ///
    /// The caller must guarantee that this generation identifies a live slot
    /// containing a `T`, that its owner lease remains live for the duration of
    /// `callback`, and that all access remains on the owning UI thread.
    #[inline]
    pub(super) unsafe fn resolve_unchecked<R>(
        self,
        callback: impl FnOnce(&T) -> R,
    ) -> R {
        with_arena(|arena| {
            // SAFETY: guaranteed by the caller's live-generation precondition.
            let slot = unsafe { arena.slots.get_unchecked(self.key.index as usize) };
            // SAFETY: guaranteed by the caller's live-generation precondition.
            let value = unsafe { slot.value.as_ref().unwrap_unchecked() };
            // SAFETY: guaranteed by the caller's erased-type precondition.
            let value = unsafe { value.downcast_ref::<T>().unwrap_unchecked() };
            callback(value)
        })
    }

    /// Acquires a read guard without checking the slot index, generation,
    /// value presence, or erased value type.
    ///
    /// # Safety
    ///
    /// The caller must guarantee that this generation identifies a live slot
    /// containing a `T`, that its owner lease remains live while the guard is
    /// acquired, and that all access remains on the owning UI thread.
    #[inline]
    pub(super) unsafe fn read_guard_unchecked(self) -> StateSlotReadGuard<T> {
        let (value, key) = with_arena_mut(|arena| unsafe {
            arena.acquire_read_unchecked::<T>(self.key)
        });
        StateSlotReadGuard {
            value,
            _lease: Rc::new(ReadLease { index: key.index }),
            _marker: PhantomData,
        }
    }

    /// Acquires a checked read guard that keeps this slot's erased value alive.
    #[inline]
    pub(super) fn try_read_guard(self) -> Result<StateSlotReadGuard<T>, StateSlotResolutionError> {
        let (value, key) = with_arena_mut(|arena| arena.acquire_read::<T>(self.key))?;
        Ok(StateSlotReadGuard {
            value,
            _lease: Rc::new(ReadLease { index: key.index }),
            _marker: PhantomData,
        })
    }
}

struct ArenaSlot {
    generation: u64,
    value: Option<Box<dyn Any>>,
    readers: usize,
    owner_released: bool,
    retired: bool,
}

struct ReadLease {
    index: u32,
}

impl Drop for ReadLease {
    fn drop(&mut self) {
        release_reader(self.index);
    }
}

/// A borrow of an erased arena value.
///
/// The arena retains the value until this guard is dropped, including when the
/// element-side owner is released while the guard is live. The raw pointer is
/// therefore valid for the guard's lifetime; all access remains confined to the
/// UI thread because the guard owns an `Rc` lease.
pub(super) struct StateSlotReadGuard<T: ?Sized> {
    value: *const T,
    _lease: Rc<ReadLease>,
    _marker: PhantomData<fn() -> T>,
}

impl<T: ?Sized> StateSlotReadGuard<T> {
    #[inline]
    pub(super) fn get(&self) -> &T {
        // SAFETY: `lease` holds the arena read count. The arena cannot remove
        // or replace the value until the last such lease is dropped.
        unsafe { &*self.value }
    }
}

impl<T: ?Sized> std::ops::Deref for StateSlotReadGuard<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.get()
    }
}

/// UI-thread-local erased storage for live state updater backends.
pub(super) struct StateSlotArena {
    slots: Vec<ArenaSlot>,
    free_indices: Vec<u32>,
}

impl StateSlotArena {
    #[inline]
    const fn new() -> Self {
        Self {
            slots: Vec::new(),
            free_indices: Vec::new(),
        }
    }

    #[inline]
    fn insert(&mut self, value: Box<dyn Any>) -> StateSlotKey {
        let index = loop {
            let Some(index) = self.free_indices.pop() else {
                break u32::try_from(self.slots.len())
                    .expect("exhausted all state slot indices");
            };
            let Some(slot) = self.slots.get(index as usize) else {
                continue;
            };
            if slot.value.is_none() && slot.generation != u64::MAX {
                break index;
            }
        };

        if index as usize == self.slots.len() {
            self.slots.push(ArenaSlot {
                generation: INITIAL_GENERATION,
                value: Some(value),
                readers: 0,
                owner_released: false,
                retired: false,
            });
        } else {
            let slot = &mut self.slots[index as usize];
            debug_assert!(slot.value.is_none());
            debug_assert_ne!(slot.generation, u64::MAX);
            slot.value = Some(value);
            slot.readers = 0;
            slot.owner_released = false;
            slot.retired = false;
        }

        StateSlotKey {
            index,
            generation: self.slots[index as usize].generation,
        }
    }

    #[inline]
    fn resolve<T: Any + 'static>(
        &self,
        key: StateSlotKey,
    ) -> Result<&T, StateSlotResolutionError> {
        let Some(slot) = self.slots.get(key.index as usize) else {
            return Err(StateSlotResolutionError::Missing);
        };
        if slot.generation != key.generation || slot.owner_released {
            return Err(StateSlotResolutionError::Stale);
        }
        let Some(value) = slot.value.as_ref() else {
            return Err(StateSlotResolutionError::Consumed);
        };
        value
            .downcast_ref::<T>()
            .ok_or(StateSlotResolutionError::WrongType)
    }

    #[inline]
    fn release(&mut self, key: StateSlotKey) {
        let (value, reusable) = {
            let Some(slot) = self.slots.get_mut(key.index as usize) else {
                return;
            };
            if slot.generation != key.generation || slot.owner_released || slot.value.is_none() {
                return;
            }

            slot.owner_released = true;
            let reusable = if slot.generation == u64::MAX {
                slot.retired = true;
                false
            } else {
                slot.generation += 1;
                true
            };
            if slot.readers != 0 {
                return;
            }
            let value = slot
                .value
                .take()
                .expect("checked that the state slot contains a value");
            (value, reusable)
        };

        if reusable {
            self.free_indices.push(key.index);
        }
        drop(value);
    }

    #[inline]
    fn acquire_read<T: Any + 'static>(
        &mut self,
        key: StateSlotKey,
    ) -> Result<(*const T, StateSlotKey), StateSlotResolutionError> {
        let Some(slot) = self.slots.get_mut(key.index as usize) else {
            return Err(StateSlotResolutionError::Missing);
        };
        if slot.generation != key.generation || slot.owner_released {
            return Err(StateSlotResolutionError::Stale);
        }
        let Some(value) = slot.value.as_ref() else {
            return Err(StateSlotResolutionError::Consumed);
        };
        let value = value
            .downcast_ref::<T>()
            .ok_or(StateSlotResolutionError::WrongType)?
            as *const T;
        slot.readers += 1;
        Ok((value, key))
    }

    /// Acquires a read pointer without validating the slot key or value.
    ///
    /// # Safety
    ///
    /// The caller must guarantee that `key.index` identifies an existing slot
    /// whose value is a live `T`. The returned pointer is protected from slot
    /// removal by the incremented reader count and must be paired with one
    /// `release_reader` call through [`StateSlotReadGuard`].
    #[inline]
    unsafe fn acquire_read_unchecked<T: Any + 'static>(
        &mut self,
        key: StateSlotKey,
    ) -> (*const T, StateSlotKey) {
        // SAFETY: guaranteed by the caller's live-slot precondition.
        let slot = unsafe { self.slots.get_unchecked_mut(key.index as usize) };
        // SAFETY: guaranteed by the caller's live-value precondition.
        let value = unsafe { slot.value.as_ref().unwrap_unchecked() };
        // SAFETY: guaranteed by the caller's erased-type precondition.
        let value = unsafe { value.downcast_ref::<T>().unwrap_unchecked() } as *const T;
        slot.readers += 1;
        (value, key)
    }

    #[inline]
    fn release_reader(&mut self, index: u32) {
        let Some(slot) = self.slots.get_mut(index as usize) else {
            return;
        };
        if slot.readers == 0 {
            return;
        }
        slot.readers -= 1;
        if slot.readers != 0 || !slot.owner_released {
            return;
        }

        let value = slot.value.take();
        if !slot.retired {
            self.free_indices.push(index);
        }
        drop(value);
    }
}

pub(super) struct SlotOwner {
    key: StateSlotKey,
}

impl Drop for SlotOwner {
    fn drop(&mut self) {
        release_slot(self.key);
    }
}

/// The element-side RAII lease that keeps one erased state slot live.
pub(crate) struct StateStorage {
    owner: Rc<SlotOwner>,
}

impl StateStorage {
    /// Inserts a value and returns its non-owning generation key with its lease.
    #[inline]
    pub(super) fn insert<T: 'static>(value: T) -> (Self, StateUpdaterGeneration<T>) {
        let key = with_arena_mut(|arena| arena.insert(Box::new(value)));
        let storage = Self {
            owner: Rc::new(SlotOwner { key }),
        };
        (storage, StateUpdaterGeneration::from_key(key))
    }

    /// Shares the lease during an old/new reconciliation overlap.
    #[inline]
    pub(super) fn clone_for_reconciliation(&self) -> Self {
        Self {
            owner: self.owner.clone(),
        }
    }

    #[inline]
    pub(super) fn downgrade(&self) -> std::rc::Weak<SlotOwner> {
        Rc::downgrade(&self.owner)
    }

    #[inline]
    pub(super) fn from_owner(owner: Rc<SlotOwner>) -> Self {
        Self { owner }
    }
}

thread_local! {
    static STATE_SLOT_ARENA: RefCell<StateSlotArena> = const {
        RefCell::new(StateSlotArena::new())
    };
    static PENDING_SLOT_RELEASES: RefCell<Vec<StateSlotKey>> = const {
        RefCell::new(Vec::new())
    };
    static PENDING_READER_RELEASES: RefCell<Vec<u32>> = const {
        RefCell::new(Vec::new())
    };
}

#[inline]
fn with_arena<R>(callback: impl FnOnce(&StateSlotArena) -> R) -> R {
    flush_pending_releases();
    let result = STATE_SLOT_ARENA.with(|arena| {
        let arena = arena.borrow();
        callback(&arena)
    });
    flush_pending_releases();
    result
}

#[inline]
fn with_arena_mut<R>(callback: impl FnOnce(&mut StateSlotArena) -> R) -> R {
    flush_pending_releases();
    let result = STATE_SLOT_ARENA.with(|arena| {
        let mut arena = arena.borrow_mut();
        callback(&mut arena)
    });
    flush_pending_releases();
    result
}

fn release_slot(key: StateSlotKey) {
    let released = STATE_SLOT_ARENA.try_with(|arena| {
        let Ok(mut arena) = arena.try_borrow_mut() else {
            return false;
        };
        arena.release(key);
        true
    });
    if let Ok(false) = released {
        let _ = PENDING_SLOT_RELEASES.try_with(|pending| pending.borrow_mut().push(key));
    }
}

fn release_reader(index: u32) {
    let released = STATE_SLOT_ARENA.try_with(|arena| {
        let Ok(mut arena) = arena.try_borrow_mut() else {
            return false;
        };
        arena.release_reader(index);
        true
    });
    if let Ok(false) = released {
        // A reader can only be dropped while another arena operation is active
        // on this thread. Releasing it after that operation preserves the slot
        // until the guard's actual lifetime ends.
        let _ = PENDING_READER_RELEASES.try_with(|pending| pending.borrow_mut().push(index));
    }
}

fn flush_pending_releases() {
    loop {
        let mut pending = match PENDING_SLOT_RELEASES
            .try_with(|pending| std::mem::take(&mut *pending.borrow_mut()))
        {
            Ok(pending) => pending,
            Err(_) => return,
        };
        if pending.is_empty() {
            flush_pending_readers();
            return;
        }

        let released = STATE_SLOT_ARENA.try_with(|arena| {
            let Ok(mut arena) = arena.try_borrow_mut() else {
                return false;
            };
            for key in pending.drain(..) {
                arena.release(key);
            }
            true
        });
        match released {
            Ok(true) => {}
            Ok(false) => {
                let _ = PENDING_SLOT_RELEASES.try_with(|deferred| {
                    deferred.borrow_mut().append(&mut pending)
                });
                return;
            }
            Err(_) => return,
        }

        flush_pending_readers();
    }
}

fn flush_pending_readers() {
    loop {
        let mut pending = match PENDING_READER_RELEASES
            .try_with(|pending| std::mem::take(&mut *pending.borrow_mut()))
        {
            Ok(pending) => pending,
            Err(_) => return,
        };
        if pending.is_empty() {
            return;
        }
        let released = STATE_SLOT_ARENA.try_with(|arena| {
            let Ok(mut arena) = arena.try_borrow_mut() else {
                return false;
            };
            for index in pending.drain(..) {
                arena.release_reader(index);
            }
            true
        });
        match released {
            Ok(true) => {}
            Ok(false) => {
                let _ = PENDING_READER_RELEASES.try_with(|deferred| {
                    deferred.borrow_mut().append(&mut pending)
                });
                return;
            }
            Err(_) => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use super::{StateSlotArena, StateStorage, StateUpdaterGeneration,
        StateSlotResolutionError};

    thread_local! {
        static STORAGE_DROPPED_AFTER_ARENA: RefCell<Option<StateStorage>> = const {
            RefCell::new(None)
        };
    }

    #[derive(Debug)]
    struct NonCopyState {
        drops: Rc<Cell<usize>>,
        value: String,
    }

    impl Drop for NonCopyState {
        fn drop(&mut self) {
            self.drops.set(self.drops.get() + 1);
        }
    }

    fn assert_copy<T: Copy>() {}

    #[test]
    fn public_updater_is_copy_without_a_copy_state_bound() {
        assert_copy::<crate::StateUpdater<NonCopyState>>();
    }

    #[test]
    fn generation_is_copy_without_copying_the_state_value() {
        let drops = Rc::new(Cell::new(0));
        let (storage, generation) = StateStorage::insert(NonCopyState {
            drops: drops.clone(),
            value: String::from("state"),
        });

        assert_copy::<StateUpdaterGeneration<NonCopyState>>();
        let copied = generation;
        assert_eq!(generation.key(), copied.key());
        assert_eq!(generation.try_resolve(|state| state.value.clone()), Ok(String::from("state")));
        assert_eq!(drops.get(), 0);

        drop(storage);
        assert_eq!(drops.get(), 1);
    }

    #[test]
    fn independent_slots_resolve_their_own_values() {
        let (first_storage, first) = StateStorage::insert(String::from("first"));
        let (second_storage, second) = StateStorage::insert(String::from("second"));

        assert_eq!(first.try_resolve(|value| value.clone()), Ok(String::from("first")));
        assert_eq!(second.try_resolve(|value| value.clone()), Ok(String::from("second")));
        assert_ne!(first.key(), second.key());

        drop(first_storage);
        drop(second_storage);
    }

    #[test]
    fn dropping_the_last_owner_invalidates_copied_generations() {
        let (storage, generation) = StateStorage::insert(41_u32);
        let copied = generation;

        assert_eq!(copied.try_resolve(|value| *value), Ok(41));
        drop(storage);

        assert_eq!(generation.try_resolve(|value| *value), Err(StateSlotResolutionError::Stale));
        assert_eq!(copied.try_resolve(|value| *value), Err(StateSlotResolutionError::Stale));
    }

    #[test]
    fn reused_slots_advance_generation_before_accepting_new_values() {
        let (old_storage, old_generation) = StateStorage::insert(String::from("old"));
        let old_key = old_generation.key();
        drop(old_storage);

        let (new_storage, new_generation) = StateStorage::insert(String::from("new"));
        assert_eq!(old_key.index, new_generation.key().index);
        assert_ne!(old_key.generation, new_generation.key().generation);
        assert_eq!(old_generation.try_resolve(|value| value.clone()), Err(StateSlotResolutionError::Stale));
        assert_eq!(new_generation.try_resolve(|value| value.clone()), Ok(String::from("new")));

        drop(new_storage);
    }

    #[test]
    fn wrong_type_lookup_is_rejected_without_running_the_callback() {
        let (storage, generation) = StateStorage::insert(7_u32);
        let wrong = StateUpdaterGeneration::<String>::from_key(generation.key());
        let called = Cell::new(false);

        assert_eq!(
            wrong.try_resolve(|_| {
                called.set(true);
            }),
            Err(StateSlotResolutionError::WrongType),
        );
        assert!(!called.get());

        drop(storage);
    }

    #[test]
    fn a_reconciliation_lease_keeps_the_slot_alive_until_the_last_owner_drops() {
        let (storage, generation) = StateStorage::insert(9_u32);
        let handoff = storage.clone_for_reconciliation();

        drop(storage);
        assert_eq!(generation.try_resolve(|value| *value), Ok(9));

        drop(handoff);
        assert_eq!(generation.try_resolve(|value| *value), Err(StateSlotResolutionError::Stale));
    }

    #[test]
    fn generation_overflow_retires_a_slot_instead_of_reusing_its_key() {
        let mut arena = StateSlotArena::new();
        let key = arena.insert(Box::new(1_u32));
        arena.slots[key.index as usize].generation = u64::MAX;

        arena.release(key);
        assert!(arena.free_indices.is_empty());

        let replacement = arena.insert(Box::new(2_u32));
        assert_ne!(replacement.index, key.index);
    }

    #[test]
    fn owner_drop_during_resolution_is_deferred_until_the_lookup_finishes() {
        let (storage, generation) = StateStorage::insert(13_u32);

        let result = generation.try_resolve(|value| {
            assert_eq!(*value, 13);
            drop(storage);
            13
        });

        assert_eq!(result, Ok(13));
        assert_eq!(generation.try_resolve(|value| *value), Err(StateSlotResolutionError::Stale));
    }

    #[test]
    fn a_read_guard_keeps_the_erased_value_alive_until_it_is_dropped() {
        let (storage, generation) = StateStorage::insert(String::from("guarded"));
        let guard = generation.try_read_guard().unwrap();

        drop(storage);
        assert_eq!(guard.as_str(), "guarded");
        drop(guard);
        assert_eq!(generation.try_resolve(|value| value.clone()), Err(StateSlotResolutionError::Stale));
    }

    #[test]
    fn owner_drop_after_arena_shutdown_does_not_abort_the_thread() {
        let result = std::thread::spawn(|| {
            // Initialize this test TLS before the arena. Rust then drops the
            // arena first, leaving this owner to exercise the shutdown path.
            STORAGE_DROPPED_AFTER_ARENA.with(|storage| {
                let (state_storage, _) = StateStorage::insert(17_u32);
                *storage.borrow_mut() = Some(state_storage);
            });
        })
        .join();

        assert!(result.is_ok());
    }
}
