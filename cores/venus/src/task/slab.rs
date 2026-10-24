use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};
use std::task::Waker;

use crate::task::{LocalFuture, Phase, ScopeId, TaskId};

/// The "no neighbour" marker for the intrusive per-scope lists.
const NONE: u32 = u32::MAX;

/// One spawned task, as the scheduler stores it.
///
/// `lease` is an [`Option`] because polling *moves the future out* of the
/// slab: the scheduler's interior state must be un-borrowed while a task runs,
/// or a task that spawns another task would panic on a re-entrant borrow. The
/// waker travels with the future so that a poll moves two words instead of
/// bumping an [`std::sync::Arc`] refcount — an atomic pair per poll adds up at
/// 120 Hz.
pub(crate) struct Task {
    lease: Option<(LocalFuture, Waker)>,
    pub(crate) phase: Phase,
    pub(crate) scope: ScopeId,
    /// Whether this task is already sitting in a ready queue. Without it a task
    /// woken five times before it next runs would be polled five times.
    pub(crate) queued: bool,
    /// Neighbours in this task's scope list — see [`TaskSlab::scopes`].
    prev_in_scope: u32,
    next_in_scope: u32,
}

struct Slot {
    generation: u32,
    task: Option<Task>,
}

/// Hashes a [`ScopeId`] to itself.
///
/// Scope ids are sequential `u64`s handed out by the scheduler, never attacker
/// input, so SipHash's collision resistance buys nothing here — and its cost
/// would be paid on every spawn and every task completion.
#[derive(Default)]
struct ScopeIdHasher(u64);

impl Hasher for ScopeIdHasher {
    #[inline]
    fn finish(&self) -> u64 {
        self.0
    }

    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.0 = self.0.rotate_left(8) ^ u64::from(byte);
        }
    }

    #[inline]
    fn write_u64(&mut self, id: u64) {
        self.0 = id;
    }
}

type ScopeIndex = HashMap<ScopeId, u32, BuildHasherDefault<ScopeIdHasher>>;

/// A generational arena of live tasks.
///
/// Vacated slots are reused, so a long-lived application spawning a task per
/// frame does not grow the arena. Each reuse bumps the slot's generation, which
/// is the whole reason [`TaskId`] carries one: a ready queue or a cross-thread
/// wake may still name a task that has since been cancelled, and that stale id
/// must resolve to "gone" rather than to whoever moved into the slot.
pub(crate) struct TaskSlab {
    slots: Vec<Slot>,
    vacant: Vec<u32>,
    live: usize,
    /// The head of each scope's doubly-linked task list, threaded through the
    /// slots themselves.
    ///
    /// This is what keeps scope cancellation proportional to the *scope*: the
    /// framework drops one [`crate::TaskScope`] per unmounting element, and an
    /// element that spawned nothing must pay one missed lookup — not a sweep of
    /// every slot the application has ever allocated.
    scopes: ScopeIndex,
}

impl TaskSlab {
    pub(crate) fn new() -> Self {
        Self {
            slots: Vec::new(),
            vacant: Vec::new(),
            live: 0,
            scopes: ScopeIndex::default(),
        }
    }

    /// Reserves an id without occupying it.
    ///
    /// A task's waker names the task, so the id has to exist before the [`Task`]
    /// can be built. Every `reserve` is immediately followed by [`Self::occupy`].
    pub(crate) fn reserve(&mut self) -> TaskId {
        match self.vacant.pop() {
            Some(index) => TaskId::new(index, self.slots[index as usize].generation),
            None => {
                let index = self.slots.len() as u32;
                self.slots.push(Slot {
                    generation: 0,
                    task: None,
                });
                TaskId::new(index, 0)
            }
        }
    }

    /// Stores `task` in the slot reserved for `id`, linking it into its
    /// scope's list.
    pub(crate) fn occupy(
        &mut self,
        id: TaskId,
        future: LocalFuture,
        waker: Waker,
        phase: Phase,
        scope: ScopeId,
    ) {
        let index = id.index();
        let next_in_scope = self.scopes.insert(scope, index).unwrap_or(NONE);
        if next_in_scope != NONE {
            self.slots[next_in_scope as usize]
                .task
                .as_mut()
                .expect("a scope list names only live tasks")
                .prev_in_scope = index;
        }

        let slot = &mut self.slots[index as usize];
        debug_assert!(slot.task.is_none(), "a reserved slot was occupied twice");
        slot.task = Some(Task {
            lease: Some((future, waker)),
            phase,
            scope,
            queued: true,
            prev_in_scope: NONE,
            next_in_scope,
        });
        self.live += 1;
    }

    /// The live task `id` names, or `None` when `id` is stale.
    pub(crate) fn get_mut(&mut self, id: TaskId) -> Option<&mut Task> {
        let slot = self.slots.get_mut(id.index() as usize)?;
        if slot.generation != id.generation() {
            return None;
        }
        slot.task.as_mut()
    }

    /// Whether `id` still names a live task.
    pub(crate) fn contains(&self, id: TaskId) -> bool {
        self.slots
            .get(id.index() as usize)
            .is_some_and(|slot| slot.generation == id.generation() && slot.task.is_some())
    }

    /// Lends the future out for one poll, together with the task's waker.
    ///
    /// Returns `None` when `id` is stale, or when the future is already out —
    /// which is how a task woken while it is running avoids being polled
    /// re-entrantly.
    pub(crate) fn lend(&mut self, id: TaskId) -> Option<(LocalFuture, Waker)> {
        self.get_mut(id)?.lease.take()
    }

    /// Gives a lent future and its waker back after a `Poll::Pending`.
    ///
    /// Returns `false` when the task was cancelled while it was running — a
    /// widget unmounting itself from its own handler — in which case the future
    /// is dropped here instead.
    pub(crate) fn restore(&mut self, id: TaskId, future: LocalFuture, waker: Waker) -> bool {
        match self.get_mut(id) {
            Some(task) => {
                task.lease = Some((future, waker));
                true
            }
            None => false,
        }
    }

    /// Drops the task `id` names, vacating its slot.
    pub(crate) fn remove(&mut self, id: TaskId) -> bool {
        let Some(slot) = self.slots.get_mut(id.index() as usize) else {
            return false;
        };
        if slot.generation != id.generation() {
            return false;
        }
        let Some(task) = slot.task.take() else {
            return false;
        };

        slot.generation = slot.generation.wrapping_add(1);
        self.vacant.push(id.index());
        self.live -= 1;
        self.unlink(&task, id.index());
        true
    }

    /// Takes a removed task out of its scope's list.
    fn unlink(&mut self, task: &Task, index: u32) {
        let Task {
            prev_in_scope: prev,
            next_in_scope: next,
            scope,
            ..
        } = *task;

        if next != NONE {
            self.slots[next as usize]
                .task
                .as_mut()
                .expect("a scope list names only live tasks")
                .prev_in_scope = prev;
        }

        if prev != NONE {
            self.slots[prev as usize]
                .task
                .as_mut()
                .expect("a scope list names only live tasks")
                .next_in_scope = next;
        } else if next != NONE {
            *self
                .scopes
                .get_mut(&scope)
                .expect("a live task's scope is indexed") = next;
        } else {
            debug_assert_eq!(self.scopes.get(&scope), Some(&index));
            self.scopes.remove(&scope);
        }
    }

    /// Drops every task belonging to `scope`, returning how many there were.
    ///
    /// This is what an unmounting element relies on: no abort handle per task,
    /// no generation counter per widget, one walk of the scope's own list. A
    /// scope that spawned nothing — most of them, since the framework keeps one
    /// per element — costs a single missed hash lookup.
    pub(crate) fn remove_scope(&mut self, scope: ScopeId) -> usize {
        let Some(mut index) = self.scopes.remove(&scope) else {
            return 0;
        };

        let mut removed = 0;
        while index != NONE {
            let slot = &mut self.slots[index as usize];
            let task = slot
                .task
                .take()
                .expect("a scope list names only live tasks");
            slot.generation = slot.generation.wrapping_add(1);
            self.vacant.push(index);
            removed += 1;
            index = task.next_in_scope;
        }

        self.live -= removed;
        removed
    }

    /// How many tasks are alive.
    pub(crate) const fn len(&self) -> usize {
        self.live
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn insert(slab: &mut TaskSlab, scope: ScopeId) -> TaskId {
        let id = slab.reserve();
        slab.occupy(
            id,
            Box::pin(async {}),
            Waker::noop().clone(),
            Phase::Microtask,
            scope,
        );
        id
    }

    #[test]
    fn a_reused_slot_invalidates_the_id_that_vacated_it() {
        let mut slab = TaskSlab::new();

        let first = insert(&mut slab, ScopeId::ROOT);
        assert!(slab.remove(first));

        let second = insert(&mut slab, ScopeId::ROOT);

        assert_eq!(first.index(), second.index(), "the slot should be reused");
        assert!(!slab.contains(first));
        assert!(slab.contains(second));
        assert_eq!(slab.len(), 1);
    }

    #[test]
    fn a_lent_future_cannot_be_lent_twice() {
        let mut slab = TaskSlab::new();
        let id = insert(&mut slab, ScopeId::ROOT);

        let (future, waker) = slab.lend(id).expect("the future to be available");
        assert!(slab.lend(id).is_none());

        assert!(slab.restore(id, future, waker));
        assert!(slab.lend(id).is_some());
    }

    #[test]
    fn a_future_lent_by_a_cancelled_task_is_not_restored() {
        let mut slab = TaskSlab::new();
        let id = insert(&mut slab, ScopeId::ROOT);

        let (future, waker) = slab.lend(id).expect("the future to be available");
        slab.remove(id);

        assert!(!slab.restore(id, future, waker));
        assert_eq!(slab.len(), 0);
    }

    #[test]
    fn cancelling_a_scope_with_no_tasks_removes_nothing() {
        let mut slab = TaskSlab::new();
        let _busy = insert(&mut slab, ScopeId::new(1));

        assert_eq!(slab.remove_scope(ScopeId::new(2)), 0);
        assert_eq!(slab.len(), 1);
    }

    #[test]
    fn cancelling_a_scope_twice_removes_nothing_the_second_time() {
        let mut slab = TaskSlab::new();
        let scope = ScopeId::new(1);
        insert(&mut slab, scope);
        insert(&mut slab, scope);

        assert_eq!(slab.remove_scope(scope), 2);
        assert_eq!(slab.remove_scope(scope), 0);
        assert_eq!(slab.len(), 0);
    }

    // Whichever position a task holds in its scope's bookkeeping — first
    // spawned, last spawned, or in between — finishing early must take exactly
    // that task out of the scope and nothing else.
    #[test]
    fn a_task_that_finished_is_not_counted_when_its_scope_is_cancelled() {
        for removed_first in 0..3 {
            let mut slab = TaskSlab::new();
            let scope = ScopeId::new(1);
            let tasks = [
                insert(&mut slab, scope),
                insert(&mut slab, scope),
                insert(&mut slab, scope),
            ];

            assert!(slab.remove(tasks[removed_first]));
            assert_eq!(
                slab.remove_scope(scope),
                2,
                "early finisher #{removed_first}"
            );
            assert_eq!(slab.len(), 0);
        }
    }

    #[test]
    fn a_reused_slot_does_not_resurrect_its_old_scope() {
        let mut slab = TaskSlab::new();
        let old = ScopeId::new(1);
        let new = ScopeId::new(2);

        let first = insert(&mut slab, old);
        assert!(slab.remove(first));
        let second = insert(&mut slab, new);
        assert_eq!(first.index(), second.index(), "the slot should be reused");

        assert_eq!(slab.remove_scope(old), 0);
        assert!(slab.contains(second));
        assert_eq!(slab.remove_scope(new), 1);
        assert_eq!(slab.len(), 0);
    }

    #[test]
    fn removing_a_scope_leaves_every_other_scope_alone() {
        let mut slab = TaskSlab::new();
        let doomed = ScopeId::new(1);
        let kept = ScopeId::new(2);

        let first = insert(&mut slab, doomed);
        let second = insert(&mut slab, kept);
        let third = insert(&mut slab, doomed);

        assert_eq!(slab.remove_scope(doomed), 2);

        assert!(!slab.contains(first));
        assert!(slab.contains(second));
        assert!(!slab.contains(third));
        assert_eq!(slab.len(), 1);
    }
}
