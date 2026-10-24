# Copy-implemented `StateUpdater`

> Implementation status (2026-09-02): Phase 0 storage, the copyable native
> and portable handle construction, common read guards, stale `try_*` methods,
> `has_state`, the unsafe `set_state_unchecked` and `read_state_unchecked`
> fast paths, native/keyed owner handoff, portable live-entry pruning, and
> failed-build cleanup for newly created slots are implemented and covered by
> focused tests. Native and portable delayed callbacks now reject all safe
> state operations after teardown, and rollback tests cover preserving an old
> live owner while rejecting a failed candidate/new slot. Child-reconciliation
> panics are now contained in both old/new owner adoption and ordinary dirty
> rebuilds, with stable error elements and candidate rejection covered by
> focused native and portable-feature tests. Arena owner/read-lease release is
> also safe when a generated portable context outlives the arena's TLS
> destructor. Framework and example call-site cleanup is complete, and the
> current native performance baseline is recorded below. A historical
> before/after comparison still requires a clean pre-migration checkout, which
> is not present in this workspace.

This plan redesigns Aimer's `StateUpdater<S>` so framework users can move the same updater into multiple callbacks
without calling `.clone()`, while keeping state cleanup deterministic and preserving the native and portable backends.

The public `StateUpdater<S>` becomes a small, non-owning handle backed by
`StateUpdaterGeneration<T>`. The live state, mutation queue, dirty flag, and failure state remain owned by
`StateStorage`. Copying the handle copies only its slot identity. It does not copy `S`, increment a reference count, or
extend the state lifetime.

`StateStorage` is an owner lease, not the lookup table itself. A single UI-thread-local `StateSlotArena` owns
type-erased slot metadata and values for the lifetime of that UI thread. Each inserted slot receives an arena index and
a generation; the live `(index, generation)` pair is unique. The arena keeps the generation metadata after a slot is
invalidated, clears the value, increments the generation, and reuses the index only after that check. A generation
counter that would overflow retires its slot permanently instead of wrapping.

This topology gives a copied updater a safe lookup path without putting an
`Rc`, `Weak`, context pointer, or owner reference in the public handle. It also means that `StatefulElement` can remain
type-erased: the arena downcasts a slot to `UpdaterSlot<S>` only after validating the key. The thread-local arena is
safe for the UI-thread-only API and avoids a raw pointer to an element field whose address could change while an element
is being boxed or moved.

## Decision

Use an Aimer-owned `StateStorage` and `StateUpdaterGeneration<T>` seam instead of exposing `Rc` through the public
updater. The implementation may borrow the generational-storage technique used by Freya, but the Aimer interface should
use these names and own the lifecycle rules.

Implement the first version with `std` collections: a `Vec` of erased slot values plus a free-index list and a
thread-local arena. Do not add a third-party arena dependency unless a benchmark demonstrates that the standard
implementation is inadequate. The storage seam must remain private to
`aimer_widget`.

The current implementation in
`crates/aimer_widget/src/widget/stateful.rs` cannot implement `Copy` because
`StateUpdaterInner<S>` contains four `Rc` values:

```rust
struct StateUpdaterInner<S> {
    tx: Rc<StateMutationQueue<S>>,
    state: Rc<SyncState<S>>,
    dirty_source: Rc<DirtySource>,
    failure: Rc<FailureState>,
}
```

`Rc::clone` increments a reference count. A bitwise copy of an `Rc` would not increment that count and would be unsound.
Keep these `Rc` values in a private
`StateStorage` slot and expose only a copyable `StateUpdaterGeneration` key.

## User-facing result

After this change, code in `State::build` can look like this:

```rust
fn build(&self, _ctx: &BuildContext) -> impl Widget {
    let updater = self.updater;

    // before copy: let on_press_updater = updater.clone();
    Column::new()
        .child(
            Button::new()
                .on_press(move || updater.set_state(|state| state.count += 1))
                .child(Text::new("+")),
        )
        .child(
            Button::new()
                .on_press(move || updater.set_state(|state| state.count -= 1))
                .child(Text::new("-")),
        )
}
```

`Button::on_press` is a zero-argument callback and a completed `Button` must have a child; the example intentionally
shows those repository conventions. Each `move` closure receives a copy of the handle. Both callbacks address the same
live state, but neither callback owns that state. The `StateUpdater`
remains UI-thread-only and is still not a general `Send + Sync` handle.

## Target ownership model

```text
StatefulElement
    owns a non-copyable StateStorage lease
        claims one StateSlotArena entry
            owns state, mutation queue, dirty source, and failure state

StateUpdater<S>
    stores Option<StateUpdaterGeneration<UpdaterSlot<S>>>
    is Copy and Clone
    resolves through the UI-thread-local StateSlotArena
    does not own the slot or lease
```

The owner lease is the RAII lifetime guard. The arena stores no strong owner reference, so a copied updater cannot keep
the lease alive. During old/new element overlap, an internal `StateStorage::clone_for_reconciliation` may share the
owner lease; the slot remains live until the last live element-side lease drops. The keyed registry stores only a `Weak`
owner reference. When the last owner lease drops, the arena clears the slot value and advances its generation. A later
operation through a stale handle therefore cannot reach a reused slot.

The stale-handle contract is fixed rather than deferred: `try_read` returns
`None` and `try_set_state` returns `None` for an empty, dropped, wrong-type, or consumed slot, and neither invokes or
queues its callback. Existing panicking methods retain the uninitialized diagnostic and report a dropped or consumed
live slot with a framework diagnostic. `set_state` remains a no-op after a recorded state failure, matching the current
recovery behavior.

## Public handle shape

The native core should have a shape equivalent to:

```rust
use std::{marker::PhantomData, rc::Rc};

pub struct StateUpdater<S> {
    slot: Option<StateUpdaterGeneration<UpdaterSlot<S>>>,
    _ui_thread_only: PhantomData<Rc<()>>,
}

impl<S> Copy for StateUpdater<S> {}

impl<S> Clone for StateUpdater<S> {
    fn clone(&self) -> Self {
        *self
    }
}

enum UpdaterSlot<S> {
    Native {
        tx: Rc<StateMutationQueue<S>>,
        state: Rc<SyncState<S>>,
        dirty_source: Rc<DirtySource>,
        failure: Rc<FailureState>,
    },
    #[cfg(feature = "portable-guest")]
    Portable(crate::portable::state::PortableStateHandle<S>),
}
```

The `Copy` and `Clone` implementations for both `StateUpdater` and
`StateUpdaterGeneration` must remain manual. A derive may add an unnecessary
`S: Copy` or `T: Copy` bound. The `S` type itself must not need to implement
`Copy`. `StateUpdaterGeneration<T>` contains a `StateSlotKey` and
`PhantomData<fn() -> T>`; it never stores a `T` value.

The concrete private key types are:

```rust
#[derive(Clone, Copy)]
struct StateSlotKey {
    index: u32,
    generation: u64,
}

struct StateUpdaterGeneration<T: ?Sized> {
    key: StateSlotKey,
    _marker: PhantomData<fn() -> T>,
}

impl<T: ?Sized> Copy for StateUpdaterGeneration<T> {}

impl<T: ?Sized> Clone for StateUpdaterGeneration<T> {
    fn clone(&self) -> Self {
        *self
    }
}
```

`StateUpdaterGeneration::try_resolve` looks up `key` in the thread-local arena, validates the generation, downcasts the
erased slot to `T`, and runs a closure while the arena borrow is held. It never returns a reference whose lifetime
outlives that closure unless a guard explicitly retains the validated borrow.

The framework creates the slot under `StateStorage`:

```rust
let (storage, slot) = StateStorage::insert(UpdaterSlot::Native {
tx: tx.clone(),
state: state_cell.clone(),
dirty_source: dirty_source.clone(),
failure: failure.clone(),
});

let updater = StateUpdater {
slot: Some(slot),
_ui_thread_only: PhantomData,
};
```

The `StateStorage` instance must be retained by the live `StatefulElement`. Returning a `StateUpdaterGeneration` after
dropping its storage creates a stale, but still safely checkable, handle. `StateStorage` owns an internal lease token
whose `Drop` invalidates the arena entry. The arena metadata remains stable while copied generations can still be
checked; dropping a state recycles the value slot without freeing the generation metadata. The key is arena-wide, not
local to one `StateStorage`, so two live elements cannot alias through the same `(index, generation)` pair.

## Method behavior

Every operation resolves the slot before touching the current backend values. The native and portable variants share
generation lookup and then apply their existing backend-specific queue, dirty, failure, and redraw behavior:

```rust
impl<S: 'static> StateUpdater<S> {
    pub fn set_state(&self, mutation: impl FnOnce(&mut S) + 'static) {
        let slot = self.slot.expect("StateUpdater is not initialized");
        slot.try_resolve(|slot| match slot {
            UpdaterSlot::Native { tx, dirty_source, failure, .. } => {
                if failure.is_failed() {
                    return;
                }
                tx.push(Box::new(mutation));
                if dirty_source.mark() {
                    request_animation_frame();
                }
            }
            #[cfg(feature = "portable-guest")]
            UpdaterSlot::Portable(handle) => handle.queue(mutation),
        })
            .unwrap_or_else(|error| panic_state_updater_resolution(error));
    }

    pub fn read<R>(&self, read: impl FnOnce(&S) -> R) -> R {
        let slot = self.slot.expect("StateUpdater is not initialized");
        slot.try_resolve(|slot| match slot {
            UpdaterSlot::Native { state, .. } => {
                let state = unsafe {
                    (&*state.0.get())
                        .as_ref()
                        .expect("State has already been consumed during reconciliation")
                };
                read(state)
            }
            #[cfg(feature = "portable-guest")]
            UpdaterSlot::Portable(handle) => handle.read(read),
        })
            .unwrap_or_else(|error| panic_state_updater_resolution(error))
    }
}
```

The production implementation should use Aimer's existing diagnostic and recovery paths instead of the abbreviated panic
messages above. Preserve the existing `track_caller`, `frame_work_stats`, failure short-circuit,
`request_animation_frame`, and portable dirty/rebuild behavior while moving the backend access behind the generation
resolver.

Keep `set_state_with<V: Clone + Send + 'static>` and its
`FnOnce(&mut S, V) + Send + 'static` bounds source-compatible. It must clone the supplied value exactly once before
queueing the mutation, as it does now; the generation lookup must not add another value or updater clone.

Add a non-panicking operation for callbacks that may legitimately outlive a widget:

```rust
pub fn try_read<R>(&self, read: impl FnOnce(&S) -> R) -> Option<R>;
pub fn try_set_state(&self, mutation: impl FnOnce(&mut S) + 'static) -> Option<()>;
pub fn has_state(&self) -> bool;
pub unsafe fn set_state_unchecked(&self, mutation: impl FnOnce(&mut S) + 'static);
pub unsafe fn read_state_unchecked(&self) -> StateReadGuard<'_, S>;
```

`try_read` and `try_set_state` perform the same generation, type, failure, and consumed-state checks as the panicking
methods. A rejected operation must not run a supplied closure, enqueue a mutation, request a frame, or emit a repeated
diagnostic for an expected stale callback.

`has_state` is a quiet liveness check and returns `false` for an empty, stale, consumed, failed, or wrong-type slot.
`set_state_unchecked` is an explicit unsafe fast path: callers must prove that the updater is initialized, its owner is
still live, the state has not been consumed or failed, and the call is on the UI thread. It does not provide a fallback
for stale callbacks.
`read_state_unchecked` has the same lifetime and thread requirements, returns a
`StateReadGuard` that retains the slot read lease, and additionally requires that no conflicting portable `RefCell`
borrow is active.

Change both native and portable `read_state` methods to return the same
`StateReadGuard<'_, S>` representation. The guard must retain the validated slot borrow/token until it is dropped; it
must never be a bare `&S` obtained from a temporary owner or lookup borrow. Keep `read` as the preferred API and add
`try_read_state() -> Option<StateReadGuard<'_, S>>` if direct field syntax is still needed by callbacks. The guard's
`Deref` preserves ordinary
`updater.read_state().field` use, but changing the native return type is a public API change: update affected call sites
and document the migration.

## Stateful element integration

Add a non-copy storage owner field to `StatefulElement`:

```rust
use std::cell::RefCell;

struct StatefulElement {
    state_storage: RefCell<StateStorage>,
    // existing fields
}
```

`StateStorage` itself owns an internal `Rc<SlotOwner>` only for framework-side handoff. It is not exposed through
`StateUpdater`, and it is not stored strongly in the keyed registry. `StatefulElement` is non-generic, so the storage
value is type-erased in the arena while its slot is downcast to the requested `UpdaterSlot<S>` at use time.

During initial construction:

1. Create the `StateStorage`.
2. Insert the updater slot into that storage.
3. Pass the copyable updater to `init_state`.
4. Retain the storage in the element.

The type-erased `state_sender` can store an `Rc<dyn Any>` containing the copyable `StateUpdater<S>`, or an equivalent
type-erased slot handle. The
`state_updater<S>()` helper should downcast the handle and return `Some(*handle)`
instead of rebuilding an updater from four cloned `Rc` values.

The existing `state_any` state cell can remain separate during the first native migration so configuration adoption
stays localized. The updater slot can continue to point at the existing `Rc<SyncState<S>>` and mutation queue. This
keeps the first change small while moving ownership of the public updater behind the new handle seam.

Preserve the current failure and reconciliation fields, including
`state_revision`, `failed`, and `failure`; the updater generation is a lifetime identity and must not be substituted for
the mutation revision used to choose among duplicate keyed copies.

## Reconciliation and keyed state

Reconciliation must preserve the `StateStorage` lease that owns the live state:

1. An unkeyed fresh element creates a fresh candidate owner and candidate state.
2. `adopt_state_from` identifies the old element as the live state source and obtains an internal owner-lease clone
   before changing any type-erased field.
3. The new element adopts the old state cell, updater handle, rebuild closure, and owner lease. Replacing its candidate
   lease drops the candidate slot.
4. The old element may retain its lease during the old/new overlap, but the keyed registry retains only a `Weak` lease
   and cannot extend the lifetime.
5. The copied updater stored in the live `S` continues to resolve the old live slot.

The handoff must be rollback-safe. If configuration adoption, rebuilding, or child reconciliation fails, the old live
owner and its failure diagnostics must remain valid; do not invalidate the old slot before the replacement has
successfully claimed it.

The keyed construction fast path is different: it creates a temporary state cell only for `adopt_config_from`, reuses
the live owner lease from
`LiveKeyedState`, and does not initialize or register a second live updater slot. The temporary candidate cell and any
candidate lease are dropped after configuration adoption. The unkeyed `adopt_state_from` path and the keyed
`try_new_with_identity` path must both be covered independently.

Add a `Weak<SlotOwner>` to `KeyedStateEntry` and a temporary strong owner lease to `LiveKeyedState`.
`register_keyed_state` must publish the lease without creating a strong registry root; otherwise a removed keyed element
would stay alive solely because the lookup table still contains it. A copied handle without the live owner is allowed to
become stale.

Treat `StateUpdater<S>` as a runtime field in `State::adopt_config_from`. Document that implementations must not assign
`new`'s updater, owner, queue, or other runtime handles into the live state. The generic framework cannot rebind an
arbitrary user field after `adopt_config_from` returns. Built-in states and examples must follow this rule, and the
candidate-updater test must prove that a published candidate handle becomes stale after adoption.

Add assertions that after adoption:

- the live updater and the rebuilt element resolve the same slot;
- a candidate updater does not continue targeting the discarded candidate;
- the old storage remains alive until the new live element has adopted it;
- dropping the owning storage invalidates all copied handles.

## Portable backend

The `portable-guest` branch must follow the same public model. It cannot keep
`PortableStateHandle<S>` directly inside `StateUpdater`, because that handle currently contains `Rc` values.

Use the same thread-local `StateSlotArena` as native code. The portable
`UpdaterSlot<S>` variant stores the existing `PortableStateHandle<S>` in the arena, while the public updater contains
only the common generation key.
`PortableLiveStateRegistry` retains a `StateStorage` lease alongside each
`PortableStateHandle`, and its drain function continues to apply queued mutations and refresh the serialized
`StateRegistry` entry at the existing rebuild boundary. This keeps `StateUpdater::set_state` independent of a
`PortableBuildContext` argument while preserving portable FIFO and coalescing behavior.

`finish_generation` must retain only entries claimed by the completed generation. Before clearing the claim set, remove
unclaimed live entries and drop their leases and `PortableStateHandle`s. Keep the encoded `StateRegistry`
snapshot separately so a stable slot that reappears can restore its retained fields into a new live state; its new arena
generation must not match any old copied updater. `abort_build` must preserve entries from the last completed generation
and roll back only live entries created by the failed transaction.

When a `PortableBuildContext` is dropped, all remaining live-state leases must drop and invalidate their arena slots. A
callback registry or async future from an older context may still hold a copied updater, but its `try_` operation must
return the stale result after context teardown or slot removal. If the same stable slot is claimed again, it receives a
new arena generation. Add tests for context drop, unclaimed-slot pruning, slot reappearance, and delayed async
callbacks.

Use the common `StateReadGuard` representation for native and portable reads. Portable `RefCell` borrows and native
validated borrow tokens must both prevent the underlying slot from being invalidated while a guard is alive.

## Dependency and module seam

Create `StateStorage` and `StateUpdaterGeneration<T>` as private implementation modules in `aimer_widget` unless another
Aimer crate needs the same storage seam. Ordinary widget users should only see `StateUpdater<S>` and its documented
methods.

The private storage seam consists of:

```rust
struct StateSlotArena {
    slots: Vec<ArenaSlot>,
    free_indices: Vec<u32>,
}

struct ArenaSlot {
    generation: u64,
    value: Option<Box<dyn Any>>,
}

struct StateStorage {
    owner: Rc<SlotOwner>,
}
```

`StateStorage::insert` registers a type-erased value in the thread-local arena and returns
`(StateStorage, StateUpdaterGeneration<T>)`. `SlotOwner` stores the
`StateSlotKey` and invalidates it when its last internal lease drops. The keyed registry may hold only a
`Weak<SlotOwner>`. `StateUpdaterGeneration<T>` validates the key and downcasts the erased value before running a
closure. No raw pointer may bypass both the generation check and the owner lifetime protocol.

The arena and its metadata remain alive for the UI thread, while slot values and owner leases are reclaimed normally.
The implementation must document the single-threaded invariant around native `UnsafeCell` access and ensure a read guard
prevents owner invalidation until the guard is dropped.

## Migration phases

### Phase 0: contract and storage design

- Implement the thread-local `StateSlotArena`, `StateSlotKey`, and
  `StateStorage` lease with type-erased slot validation.
- Implement manual `Copy` and `Clone` for `StateUpdaterGeneration<T>` without a
  `T: Copy` bound.
- Fix the stale, empty, failed, and consumed operation contract before changing the public updater implementation.
- Add direct storage tests for independent slots, owner drop, generation reuse, wrong-type lookup, and generation
  overflow retirement.

### Phase 1: compile-time contract

- Implement manual `Copy` and `Clone` for the public updater.
- Add compile assertions proving `StateUpdater<NonCopyState>: Copy`.
- Preserve `set_state`, `set_state_with`, `read`, `empty`, and the existing frame-stat and diagnostic behavior.
- Preserve the UI-thread-only auto-trait behavior.
- Change native and portable `read_state` to the common guard representation, update call sites, and document the public
  API migration.
- Add the unsafe `read_state_unchecked` fast path without weakening the guard's slot-lifetime invariant.

### Phase 2: native construction and reconciliation

- Create and retain the owner in `StatefulElement`.
- Build updaters from one copyable slot.
- Carry the live owner through keyed and unkeyed state adoption.
- Replace the four-`Rc` downcast path in `state_updater`.
- Keep the keyed registry weak and make owner handoff rollback-safe.
- Remove redundant updater clones from framework construction code where the only purpose was ownership transfer.

### Phase 3: stale-handle contract

- Add `try_read`, `try_set_state`, and `try_read_state` with the fixed result behavior described above.
- Keep existing panicking methods for programmer errors and make their uninitialized, dropped, and consumed diagnostics
  distinct.
- Add lifecycle tests for callbacks retained after unmount.

### Phase 4: portable backend

- Store the portable backend behind the same copyable generational handle.
- Preserve stable slot restoration and state revision behavior.
- Prune unclaimed live entries at a completed generation while retaining the serialized snapshot policy described above.
- Test first-generation creation, retained generations, duplicate slots, slot removal, context drop, and delayed
  callbacks.

### Phase 5: user migration and cleanup

- Completed: examples and built-in widgets use direct updater copies at the audited callback and ownership-transfer
  sites.
- `.clone()` remains source-compatible because `Clone` is still implemented, while unnecessary framework-internal
  updater clones were removed. The remaining updater-named clones are intentional `Rc`/owner or fixture-storage clones.
- Document that `Copy` does not extend widget lifetime and that stale handles must not be used for new work after
  unmount.
- A current native allocation/frame baseline is recorded below. A historical before/after run remains pending until a
  clean pre-migration checkout is available.

## Verification plan

### Compile-time tests

```rust
fn assert_copy<T: Copy>() {}

fn compile_copy_contract() {
    assert_copy::<StateUpdater<NonCopyState>>();
    assert_copy::<StateUpdaterGeneration<UpdaterSlot<NonCopyState>>>();
}
```

Also retain compile-fail checks proving that `StateUpdater<S>` is not `Send`
or `Sync` in both default and `portable-guest` builds. Add compile coverage for the common `StateReadGuard` return type
and the unchanged `set_state_with`
signature.

### Runtime tests

- Two callbacks copied from one updater mutate the same state.
- Copying an updater does not clone or move the state value.
- A non-`Copy` state type works without additional bounds.
- Multiple queued updates preserve FIFO order and existing coalescing rules.
- An empty updater still reports the existing initialization diagnostic.
- A stale updater after unmount cannot access freed state.
- A stale generation cannot access a newly reused slot.
- Dropping the last `StateStorage` lease drops a non-`Copy` state value, while a copied updater does not keep it alive.
- Reconciliation keeps the old live updater target.
- Candidate state and candidate updater are discarded together, including the keyed construction fast path.
- Keyed reorder preserves the correct updater target.
- Portable state handles preserve the same behavior after context teardown, unclaimed-slot pruning, slot reappearance,
  and delayed async callbacks.
- A read guard keeps its slot valid until the guard is dropped.
- Built-in `adopt_config_from` implementations preserve the live updater rather than copying the candidate's runtime
  handle.
- Failed portable build transactions retain the previous live entries and roll back only entries created by the failed
  attempt.
- Child-reconciliation failures are recovered without leaking a panic, retain the previous live owner where applicable,
  and install a stable diagnostic element for the failed candidate/rebuild.
- Owner and read-lease drops after thread-local arena shutdown are ignored safely; the shutdown ordering regression is
  covered by a spawned-thread test.

### Performance checks

The expected source-level improvement is zero explicit `StateUpdater` clones at the audited framework/example call
sites. Since the new `Clone` implementation is itself `*self`, removing the spelling does not prove a runtime
improvement; the measurements below are a current baseline and do not claim a frame-time gain.

The reproducible native probe is
`cargo run -p aimer_laboratory --example state_updater_profile --release`. It uses seven rounds, a one-million-copy
loop, a 32-callback stateful tree, a 256-update batch, and a thread-local redraw requester. On 2026-09-02 the final
release run reported:

| operation                            |            p50 |            p95 | allocations/op | frame requests |
|--------------------------------------|---------------:|---------------:|---------------:|---------------:|
| `StateUpdater` copy, 1M moves        |        5.16 ns |        5.72 ns |           0.00 |              — |
| stateful construction, 32 callbacks  |       31.19 µs |       41.10 µs |         752.81 |              — |
| queue + rebuild, empty callback tree | 0.03 µs/update | 0.03 µs/update |           0.03 |     1.00/batch |
| queue + rebuild, 32-callback tree    | 0.67 µs/update | 0.72 µs/update |           4.65 |     1.00/batch |

The allocator column counts allocation/reallocation calls on the benchmark thread, not allocated bytes. Construction
includes the retained stateful element and its 32-button child tree; queue-plus-rebuild includes the normal
reconciliation work.

The end-to-end native probes were also run in release mode:

- `framework_phase_profile`: for 32/256/2048 nodes, dirty reconciliation p50 was 30.90/140.37/901.76 µs; cached draw p50
  was 12.83/10.24/5.53 µs.
- `framework_baseline`: the 256-row stateful cached-frame case was 10.46 µs p50 with 1.00 allocation operation per
  frame; the 256-row eager cold frame was 36.88 µs p50 with 44.14 allocation operations.
- The native profile executable was 16,960,032 bytes under the workspace release profile. No portable guest executable
  is produced by this laboratory target, so portable binary size is not compared here.

Portable behavior was compile- and test-checked with
`cargo check -p aimer --features portable-guest` and
`cargo test -p aimer_widget --features portable-guest --lib` (329 passed). Repeat the same profile against a clean
pre-migration revision before making an optimization or regression claim.

## Resolved decisions and remaining risks

- The public updater is a key into one UI-thread-local arena. The key is arena-wide, and consists of an index plus
  generation; it is not an index into an individual element's storage.
- `StateStorage` is the only element-side lifetime lease. The keyed registry is weak, and copied updaters never own a
  lease. The arena keeps only enough metadata to reject stale generations.
- `try_read`, `try_set_state`, and `try_read_state` reject empty, dropped, wrong-type, failed, and consumed slots
  without running user closures. The panicking methods retain framework diagnostics for programmer errors.
- Native and portable `read_state` use the common guard representation; the native bare-reference API is intentionally
  not retained.
- `read_state_unchecked` skips validity checks only under an explicit unsafe contract and still returns a guard that
  retains the slot read lease.
- `State::adopt_config_from` must preserve runtime handles, including the updater. This is a documented trait invariant
  because the framework cannot rewrite arbitrary user fields after the callback returns.
- Portable live-state entries are pruned when no longer claimed, while the encoded `StateRegistry` snapshot remains
  available for explicit stable-slot restoration. Failed builds do not prune the last completed generation.
- Arena release paths use `try_with` and fail closed during TLS destruction; no state lookup is needed once the owning
  thread is shutting down.
- Do not implement `Copy` by storing an `Rc` or a pointer to an element field. Any unsafe native access must be confined
  to the UI thread and protected by the arena generation and guard lifetime rules.

The remaining risks are the cost of a thread-local type-erased lookup on callback-heavy paths and the existing native
`UnsafeCell` invariants. The current allocation/frame cost is measured, but a historical comparison still requires the
pre-migration implementation.

## Acceptance criteria

The plan is complete when:

- `StateUpdater<NonCopyState>` implements `Copy` and `Clone` without an
  `S: Copy` bound;
- `StateUpdaterGeneration<T>` also implements `Copy` and `Clone` without a
  `T: Copy` bound;
- users can move one updater variable into multiple `move` callbacks without writing `.clone()`;
- all callbacks address the same live state slot;
- state is released when its last element-side owner lease is removed;
- stale copied handles fail safely according to the fixed `try_` and panicking method contracts;
- `read_state` never exposes a reference that outlives its validated guard;
- keyed/unkeyed reconciliation preserves the live owner and state;
- portable live entries are pruned without losing the separately defined serialized snapshot behavior;
- `adopt_config_from` preserves runtime updater handles;
- `set_state_with` and existing diagnostics remain source-compatible;
- native and portable backends pass focused lifecycle and mutation tests;
- the existing UI-thread-only contract remains intact.
