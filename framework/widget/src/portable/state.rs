use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::rc::Rc;

use super::{
    AimerReflectionType, PortableApply, PortableBuildContext, PortableBuildError, PortableEncode,
    StableSlotId, StateRegistryError, StableTypeId,
};
use crate::{State, StateUpdater, StatefulWidget};
use crate::widget::state_slots::StateStorage;

type PortableStateMutation<S> = Box<dyn FnOnce(&mut S)>;

/// Shared typed storage used by the portable [`StateUpdater`] backend.
///
/// The handle is confined to the guest UI thread. `RefCell` provides checked
/// in-generation access without the unchecked state aliasing required by the
/// native retained-element implementation.
#[doc(hidden)]
pub(crate) struct PortableStateHandle<S> {
    state: Rc<RefCell<S>>,
    mutations: Rc<RefCell<VecDeque<PortableStateMutation<S>>>>,
    dirty: Rc<Cell<bool>>,
}

impl<S> Clone for PortableStateHandle<S> {
    #[inline]
    fn clone(&self) -> Self {
        Self {
            state: self.state.clone(),
            mutations: self.mutations.clone(),
            dirty: self.dirty.clone(),
        }
    }
}

impl<S> PortableStateHandle<S> {
    #[inline]
    fn new(state: S) -> Self {
        Self {
            state: Rc::new(RefCell::new(state)),
            mutations: Rc::new(RefCell::new(VecDeque::new())),
            dirty: Rc::new(Cell::new(false)),
        }
    }

    #[inline]
    pub(crate) fn queue(&self, mutation: impl FnOnce(&mut S) + 'static) {
        self.mutations.borrow_mut().push_back(Box::new(mutation));
        if !self.dirty.replace(true) {
            crate::components::element::advance_rebuild_invalidation_generation();
        }
    }

    #[inline]
    pub(crate) fn read<R>(&self, callback: impl FnOnce(&S) -> R) -> R {
        callback(&self.state.borrow())
    }

    #[inline]
    pub(crate) fn borrow(&self) -> std::cell::Ref<'_, S> {
        self.state.borrow()
    }

    #[inline]
    pub(crate) fn try_borrow(&self) -> Option<std::cell::Ref<'_, S>> {
        self.state.try_borrow().ok()
    }

    fn drain(&self) -> usize {
        let mut applied = 0;
        let mut state = self.state.borrow_mut();
        loop {
            let mutation = self.mutations.borrow_mut().pop_front();
            let Some(mutation) = mutation else {
                break;
            };
            mutation(&mut state);
            applied += 1;
        }
        applied
    }

    #[inline]
    fn take_dirty(&self) -> bool {
        self.dirty.replace(false)
    }
}

struct LiveStateEntry {
    type_id: StableTypeId,
    handle: Box<dyn Any>,
    _state_storage: StateStorage,
    drain: fn(&dyn Any, StableSlotId, &mut super::StateRegistry) -> Result<(), StateRegistryError>,
}

fn drain_live_state<S>(
    erased: &dyn Any,
    slot: StableSlotId,
    registry: &mut super::StateRegistry,
) -> Result<(), StateRegistryError>
where
    S: AimerReflectionType + PortableEncode + 'static,
{
    let handle = erased
        .downcast_ref::<PortableStateHandle<S>>()
        .expect("a live state's drain function must match its erased handle");
    if handle.drain() != 0 {
        registry.refresh(slot, &*handle.state.borrow())?;
        handle.take_dirty();
    }
    Ok(())
}

/// Type-erased live state retained between portable Widget IR generations.
#[doc(hidden)]
pub(super) struct PortableLiveStateRegistry {
    entries: BTreeMap<StableSlotId, LiveStateEntry>,
    claimed: BTreeSet<StableSlotId>,
    created_entries: BTreeSet<StableSlotId>,
    created_snapshots: BTreeSet<StableSlotId>,
}

impl PortableLiveStateRegistry {
    #[inline]
    pub(super) const fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
            claimed: BTreeSet::new(),
            created_entries: BTreeSet::new(),
            created_snapshots: BTreeSet::new(),
        }
    }

    #[inline]
    pub(super) fn finish_generation(&mut self) {
        self.entries.retain(|slot, _| self.claimed.contains(slot));
        self.claimed.clear();
        self.created_entries.clear();
        self.created_snapshots.clear();
    }

    #[inline]
    pub(super) fn abort_build(&mut self, state_registry: &mut super::StateRegistry) {
        for slot in std::mem::take(&mut self.created_entries) {
            self.entries.remove(&slot);
        }
        for slot in std::mem::take(&mut self.created_snapshots) {
            state_registry.remove(slot);
        }
        self.claimed.clear();
    }

    pub(super) fn drain_all(
        &self,
        registry: &mut super::StateRegistry,
    ) -> Result<(), StateRegistryError> {
        for (slot, entry) in &self.entries {
            (entry.drain)(entry.handle.as_ref(), *slot, registry)?;
        }
        Ok(())
    }
}

impl PortableBuildContext {
    /// Seeds one stable slot with a freshly configured state candidate.
    ///
    /// The first seed restores any compatible retained snapshot, installs a
    /// portable updater, and invokes [`State::init_state`]. Later generations
    /// preserve that live value and call [`State::adopt_config_from`] with the
    /// new candidate. A slot may be seeded only once per Widget IR generation.
    pub fn seed_stateful_state<W>(
        &mut self,
        slot: StableSlotId,
        mut candidate: W::State,
    ) -> Result<(), PortableBuildError>
    where
        W: StatefulWidget + 'static,
        W::State: AimerReflectionType + PortableApply + PortableEncode + 'static,
    {
        if self.live_states.claimed.contains(&slot) {
            return Err(PortableBuildError::DuplicateSlot { slot });
        }

        if let Some(entry) = self.live_states.entries.get(&slot) {
            if entry.type_id != W::State::TYPE_ID {
                return Err(StateRegistryError::TypeMismatch {
                    slot_id: slot,
                    expected: W::State::TYPE_ID,
                    actual: entry.type_id,
                }
                .into());
            }
            let handle = entry
                .handle
                .downcast_ref::<PortableStateHandle<W::State>>()
                .expect("matching stable state type must have its generation-local Rust type");
            {
                let mut live = handle.state.borrow_mut();
                self.state_registry().restore_into(slot, &mut *live)?;
                live.adopt_config_from(candidate);
            }
            self.live_states.claimed.insert(slot);
            return Ok(());
        }

        let has_snapshot = self.state_registry().revision(slot).is_some();
        if has_snapshot {
            self.state_registry().restore_into(slot, &mut candidate)?;
        } else {
            self.state_registry_mut().insert(slot, 0, &candidate)?;
        }
        let handle = PortableStateHandle::new(candidate);
        let (state_storage, updater) = StateUpdater::from_portable(handle.clone());
        handle.state.borrow_mut().init_state(updater);
        self.live_states.entries.insert(slot, LiveStateEntry {
            type_id: W::State::TYPE_ID,
            handle: Box::new(handle),
            _state_storage: state_storage,
            drain: drain_live_state::<W::State>,
        });
        self.live_states.created_entries.insert(slot);
        if !has_snapshot {
            self.live_states.created_snapshots.insert(slot);
        }
        self.live_states.claimed.insert(slot);
        Ok(())
    }

    /// Runs generated build/lowering code with one seeded live state.
    ///
    /// The callback receives the state, a resource-free native-compatible
    /// [`BuildContext`](crate::base::BuildContext), and this IR context. Any
    /// `set_state` calls made by the callback are drained in FIFO order after
    /// it returns, encoded once at the next revision, and coalesced into one
    /// rebuild request.
    pub fn with_stateful_state<W, R>(
        &mut self,
        slot: StableSlotId,
        callback: impl FnOnce(
            &W::State,
            &crate::base::BuildContext<'static>,
            &mut Self,
        ) -> Result<R, PortableBuildError>,
    ) -> Result<R, PortableBuildError>
    where
        W: StatefulWidget + 'static,
        W::State: AimerReflectionType + PortableApply + PortableEncode + 'static,
    {
        let entry = self
            .live_states
            .entries
            .get(&slot)
            .ok_or(StateRegistryError::UnknownSlot { slot_id: slot })?;
        if entry.type_id != W::State::TYPE_ID {
            return Err(StateRegistryError::TypeMismatch {
                slot_id: slot,
                expected: W::State::TYPE_ID,
                actual: entry.type_id,
            }
            .into());
        }
        let handle = entry
            .handle
            .downcast_ref::<PortableStateHandle<W::State>>()
            .expect("matching stable state type must have its generation-local Rust type")
            .clone();
        {
            let mut state = handle.state.borrow_mut();
            self.state_registry().restore_into(slot, &mut *state)?;
        }

        let build_context = self.build_context();
        let result = {
            let state = handle.state.borrow();
            callback(&state, &build_context, self)?
        };
        let applied = handle.drain();
        if applied != 0 {
            self.state_registry_mut().refresh(slot, &*handle.state.borrow())?;
            if handle.take_dirty() {
                self.queue_rebuild();
            }
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use aimer_anteros::{AsyncCallbackSchemaMetadata, EventId, Version, WidgetSchemaId};

    use super::super::{
        AimerReflectionType, Decoder, Encoder, FieldDescriptor, FieldKind, PortableApply,
        PortableBuildContext, PortableBuildError, PortableDecode, PortableEncode, PortableLimits,
        PortableWidgetLimits, SourceFingerprint, StableId128, StateRegistry, StateRegistryError,
        TypeSchema,
    };
    use crate::{State, StateUpdater, StatefulWidget};

    const COUNTER_FIELDS: &[FieldDescriptor] = &[
        FieldDescriptor::new("count", "u32", FieldKind::Retained),
        FieldDescriptor::new("label", "String", FieldKind::Fresh),
    ];
    const COUNTER_SCHEMA: TypeSchema = TypeSchema::new(
        "PortableCounterState",
        StableId128::from_path("type", "tests::PortableCounterState"),
        COUNTER_FIELDS,
    );

    struct CounterWidget;

    struct CounterState {
        count: u32,
        label: String,
        updater: StateUpdater<Self>,
        initializations: Rc<Cell<usize>>,
    }

    impl CounterState {
        fn new(count: u32, label: &str, initializations: Rc<Cell<usize>>) -> Self {
            Self {
                count,
                label: label.into(),
                updater: StateUpdater::new(),
                initializations,
            }
        }
    }

    impl StatefulWidget for CounterWidget {
        type State = CounterState;

        fn create_state(self) -> Self::State {
            unreachable!("tests provide the candidate directly")
        }
    }

    impl State<CounterWidget> for CounterState {
        fn init_state(&mut self, updater: StateUpdater<Self>) {
            self.initializations.set(self.initializations.get() + 1);
            self.updater = updater;
        }

        fn adopt_config_from(&mut self, new: Self) {
            self.label = new.label;
        }

        fn build(&self, _ctx: &crate::base::BuildContext) -> impl crate::Widget {
            crate::ErrorWidget::new("counter")
        }
    }

    impl AimerReflectionType for CounterState {
        const TYPE_ID: StableId128 =
            StableId128::from_path("type", "tests::PortableCounterState");

        fn schema() -> &'static TypeSchema {
            &COUNTER_SCHEMA
        }
    }

    impl PortableEncode for CounterState {
        fn encode(&self, encoder: &mut Encoder<'_>) -> Result<(), super::super::EncodeError> {
            encoder.field(&COUNTER_FIELDS[0], |encoder| self.count.encode(encoder))?;
            encoder.field(&COUNTER_FIELDS[1], |_| unreachable!("fresh fields are not encoded"))
        }
    }

    impl PortableDecode for CounterState {
        fn decode(decoder: &mut Decoder<'_>) -> Result<Self, super::super::DecodeError> {
            Ok(Self {
                count: decoder.field(&COUNTER_FIELDS[0])?.unwrap(),
                label: String::new(),
                updater: StateUpdater::new(),
                initializations: Rc::new(Cell::new(0)),
            })
        }
    }

    impl PortableApply for CounterState {
        type Retained = u32;

        fn decode_retained(
            decoder: &mut Decoder<'_>,
        ) -> Result<Self::Retained, super::super::DecodeError> {
            Ok(decoder.field(&COUNTER_FIELDS[0])?.unwrap())
        }

        fn apply_retained(&mut self, retained: Self::Retained) {
            self.count = retained;
        }
    }

    const OTHER_SCHEMA: TypeSchema = TypeSchema::new(
        "OtherState",
        StableId128::from_path("type", "tests::OtherState"),
        &[],
    );

    struct OtherWidget;
    struct OtherState;

    impl StatefulWidget for OtherWidget {
        type State = OtherState;

        fn create_state(self) -> Self::State {
            OtherState
        }
    }

    impl State<OtherWidget> for OtherState {
        fn init_state(&mut self, _updater: StateUpdater<Self>) {}

        fn build(&self, _ctx: &crate::base::BuildContext) -> impl crate::Widget {
            crate::ErrorWidget::new("other")
        }
    }

    impl AimerReflectionType for OtherState {
        const TYPE_ID: StableId128 = StableId128::from_path("type", "tests::OtherState");

        fn schema() -> &'static TypeSchema {
            &OTHER_SCHEMA
        }
    }

    impl PortableEncode for OtherState {
        fn encode(&self, _encoder: &mut Encoder<'_>) -> Result<(), super::super::EncodeError> {
            Ok(())
        }
    }

    impl PortableApply for OtherState {
        type Retained = ();

        fn decode_retained(
            _decoder: &mut Decoder<'_>,
        ) -> Result<Self::Retained, super::super::DecodeError> {
            Ok(())
        }

        fn apply_retained(&mut self, _retained: Self::Retained) {}
    }

    fn state_limits() -> PortableLimits {
        PortableLimits::new(8, 32, 256, 256, 4_096)
    }

    fn context() -> PortableBuildContext {
        PortableBuildContext::new(
            1,
            0,
            PortableWidgetLimits::new(16, 16, 16, 16, 256, 4_096),
            state_limits(),
        )
        .unwrap()
    }

    fn slot(value: u128) -> StableId128 {
        StableId128::from_u128(value)
    }

    fn finish_generation(context: &mut PortableBuildContext, discriminator: u64) {
        let root = context
            .push_node(
                WidgetSchemaId::new(99),
                Version::new(1, 0),
                None,
                SourceFingerprint::new(slot(discriminator as u128)),
                &[],
                &[],
            )
            .unwrap();
        context.finish_document(root).unwrap();
        context.take_rebuild_request();
    }

    #[test]
    fn portable_updater_reads_synchronously_and_drains_fifo_with_one_rebuild() {
        let initializations = Rc::new(Cell::new(0));
        let mut context = context();
        let state_slot = slot(1);
        context
            .seed_stateful_state::<CounterWidget>(
                state_slot,
                CounterState::new(1, "first", initializations.clone()),
            )
            .unwrap();

        context
            .with_stateful_state::<CounterWidget, _>(state_slot, |state, build, _portable| {
                assert!(build.is_portable());
                assert_eq!(state.updater.read(|state| state.count), 1);
                assert_eq!(state.updater.read_state().count, 1);
                state.updater.set_state(|state| state.count += 2);
                state.updater.set_state(|state| state.count *= 3);
                Ok(())
            })
            .unwrap();

        assert_eq!(initializations.get(), 1);
        assert_eq!(context.state_registry().revision(state_slot), Some(1));
        assert_eq!(context.state_registry().restore::<CounterState>(state_slot).unwrap().count, 9);
        assert!(context.take_rebuild_request());
        assert!(!context.take_rebuild_request());
    }

    #[test]
    fn portable_unchecked_update_uses_the_live_state_slot() {
        let initializations = Rc::new(Cell::new(0));
        let state_slot = slot(11);
        let mut context = context();
        context
            .seed_stateful_state::<CounterWidget>(
                state_slot,
                CounterState::new(1, "unchecked", initializations),
            )
            .unwrap();

        let updater = context
            .with_stateful_state::<CounterWidget, _>(state_slot, |state, _, _| {
                assert!(state.updater.has_state());
                unsafe {
                    state.updater.set_state_unchecked(|state| state.count += 4);
                }
                Ok(state.updater)
            })
            .unwrap();

        assert_eq!(updater.try_read(|state| state.count), Some(5));
        assert!(updater.has_state());
    }

    #[test]
    fn imported_retained_state_is_restored_while_new_configuration_stays_fresh() {
        let initializations = Rc::new(Cell::new(0));
        let state_slot = slot(2);
        let mut imported = StateRegistry::new(state_limits());
        imported
            .insert(
                state_slot,
                7,
                &CounterState::new(42, "old", initializations.clone()),
            )
            .unwrap();
        let snapshot = imported.export().unwrap();

        let mut context = context();
        context
            .seed_stateful_state::<CounterWidget>(
                state_slot,
                CounterState::new(0, "new", initializations.clone()),
            )
            .unwrap();
        context.state_registry_mut().import(&snapshot).unwrap();
        context
            .with_stateful_state::<CounterWidget, _>(state_slot, |state, _build, _portable| {
                assert_eq!(state.count, 42);
                assert_eq!(state.label, "new");
                Ok(())
            })
            .unwrap();

        assert_eq!(initializations.get(), 1);
        assert_eq!(context.state_registry().revision(state_slot), Some(7));
    }

    #[test]
    fn later_generation_preserves_live_state_and_adopts_fresh_configuration() {
        let initializations = Rc::new(Cell::new(0));
        let state_slot = slot(3);
        let mut context = context();
        context
            .seed_stateful_state::<CounterWidget>(
                state_slot,
                CounterState::new(5, "first", initializations.clone()),
            )
            .unwrap();
        finish_generation(&mut context, 30);

        context
            .seed_stateful_state::<CounterWidget>(
                state_slot,
                CounterState::new(0, "second", initializations.clone()),
            )
            .unwrap();
        context
            .with_stateful_state::<CounterWidget, _>(state_slot, |state, _build, _portable| {
                assert_eq!(state.count, 5);
                assert_eq!(state.label, "second");
                Ok(())
            })
            .unwrap();

        assert_eq!(initializations.get(), 1);
    }

    #[test]
    fn unclaimed_state_is_pruned_and_reappearing_slots_get_new_updaters() {
        let initializations = Rc::new(Cell::new(0));
        let state_slot = slot(31);
        let mut context = context();
        context
            .seed_stateful_state::<CounterWidget>(
                state_slot,
                CounterState::new(1, "first", initializations.clone()),
            )
            .unwrap();
        let old_updater = context
            .with_stateful_state::<CounterWidget, _>(state_slot, |state, _, _| {
                Ok(state.updater)
            })
            .unwrap();

        finish_generation(&mut context, 310);
        finish_generation(&mut context, 311);

        assert_eq!(old_updater.try_read(|state| state.count), None);
        assert_eq!(old_updater.try_set_state(|state| state.count = 99), None);

        context
            .seed_stateful_state::<CounterWidget>(
                state_slot,
                CounterState::new(8, "reappeared", initializations),
            )
            .unwrap();
        let new_updater = context
            .with_stateful_state::<CounterWidget, _>(state_slot, |state, _, _| {
                Ok(state.updater)
            })
            .unwrap();

        assert_eq!(new_updater.try_read(|state| state.count), Some(1));
        assert_eq!(old_updater.try_read(|state| state.count), None);
    }

    #[test]
    fn dropping_the_portable_context_invalidates_copied_updaters() {
        let initializations = Rc::new(Cell::new(0));
        let state_slot = slot(32);
        let mut context = context();
        context
            .seed_stateful_state::<CounterWidget>(
                state_slot,
                CounterState::new(1, "context", initializations),
            )
            .unwrap();
        let updater = context
            .with_stateful_state::<CounterWidget, _>(state_slot, |state, _, _| {
                Ok(state.updater)
            })
            .unwrap();

        drop(context);

        assert_eq!(updater.try_read(|state| state.count), None);
        assert_eq!(updater.try_set_state(|state| state.count = 2), None);
    }

    #[test]
    fn delayed_callback_rejects_all_state_operations_after_portable_context_drop() {
        let initializations = Rc::new(Cell::new(0));
        let state_slot = slot(33);
        let mut context = context();
        context
            .seed_stateful_state::<CounterWidget>(
                state_slot,
                CounterState::new(1, "delayed", initializations),
            )
            .unwrap();
        let updater = context
            .with_stateful_state::<CounterWidget, _>(state_slot, |state, _, _| {
                Ok(state.updater)
            })
            .unwrap();
        let read_called = Rc::new(Cell::new(false));
        let mutation_called = Rc::new(Cell::new(false));
        let delayed: Rc<RefCell<Option<Box<dyn FnOnce()>>>> = Rc::new(RefCell::new(None));
        let delayed_callback = delayed.clone();
        let read_called_by_callback = read_called.clone();
        let mutation_called_by_callback = mutation_called.clone();
        delayed_callback.borrow_mut().replace(Box::new(move || {
            assert!(!updater.has_state());
            assert_eq!(
                updater.try_read(move |_| {
                    read_called_by_callback.set(true);
                }),
                None,
            );
            assert_eq!(
                updater.try_set_state(move |_| mutation_called_by_callback.set(true)),
                None,
            );
            assert!(updater.try_read_state().is_none());
        }));

        drop(context);
        delayed
            .borrow_mut()
            .take()
            .expect("delayed callback should be retained")();

        assert!(!read_called.get());
        assert!(!mutation_called.get());
    }

    #[test]
    fn duplicate_slots_and_live_type_mismatches_are_rejected() {
        let initializations = Rc::new(Cell::new(0));
        let state_slot = slot(4);
        let mut context = context();
        context
            .seed_stateful_state::<CounterWidget>(
                state_slot,
                CounterState::new(1, "first", initializations.clone()),
            )
            .unwrap();
        let duplicate = context
            .seed_stateful_state::<CounterWidget>(
                state_slot,
                CounterState::new(2, "duplicate", initializations),
            )
            .unwrap_err();
        assert!(matches!(duplicate, PortableBuildError::DuplicateSlot { slot } if slot == state_slot));

        finish_generation(&mut context, 40);
        let mismatch = context
            .seed_stateful_state::<OtherWidget>(state_slot, OtherState)
            .unwrap_err();
        assert!(matches!(
            mismatch,
            PortableBuildError::State(StateRegistryError::TypeMismatch { slot_id, .. })
                if slot_id == state_slot
        ));
    }

    #[test]
    fn failed_build_keeps_previous_live_entries_and_discards_new_slots() {
        let initializations = Rc::new(Cell::new(0));
        let retained_slot = slot(34);
        let new_slot = slot(35);
        let mut context = context();
        context
            .seed_stateful_state::<CounterWidget>(
                retained_slot,
                CounterState::new(3, "retained", initializations.clone()),
            )
            .unwrap();
        let retained_updater = context
            .with_stateful_state::<CounterWidget, _>(retained_slot, |state, _, _| {
                Ok(state.updater)
            })
            .unwrap();
        finish_generation(&mut context, 340);

        let failed_updater = Rc::new(RefCell::new(None));
        let failed_updater_for_build = failed_updater.clone();
        let failed = context.with_build_transaction(|context| {
            context.seed_stateful_state::<CounterWidget>(
                new_slot,
                CounterState::new(8, "new", initializations.clone()),
            )?;
            let updater = context
                .with_stateful_state::<CounterWidget, _>(new_slot, |state, _, _| {
                    Ok(state.updater)
                })?;
            failed_updater_for_build.replace(Some(updater));
            Err::<(), PortableBuildError>(PortableBuildError::IncompleteTree)
        });

        assert!(matches!(failed, Err(PortableBuildError::IncompleteTree)));
        assert!(retained_updater.has_state());
        assert_eq!(
            retained_updater.try_read(|state| (state.count, state.label.clone())),
            Some((3, String::from("retained"))),
        );
        assert_eq!(
            failed_updater
                .borrow()
                .as_ref()
                .expect("failed build should publish its temporary updater")
                .try_read(|state| state.count),
            None,
        );
        context
            .with_stateful_state::<CounterWidget, _>(retained_slot, |state, _, _| {
                assert_eq!(state.count, 3);
                assert_eq!(state.label, "retained");
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn failed_build_releases_claimed_slots_for_the_next_attempt() {
        let initializations = Rc::new(Cell::new(0));
        let state_slot = slot(41);
        let failed_updater = Rc::new(RefCell::new(None));
        let mut context = context();

        let failed_updater_for_build = failed_updater.clone();
        let failed = context.with_build_transaction(|context| {
            context.seed_stateful_state::<CounterWidget>(
                state_slot,
                CounterState::new(1, "failed", initializations.clone()),
            )?;
            let updater = context.with_stateful_state::<CounterWidget, _>(
                state_slot,
                |state, _, _| Ok(state.updater),
            )?;
            failed_updater_for_build.replace(Some(updater));
            Err::<(), PortableBuildError>(PortableBuildError::IncompleteTree)
        });
        assert!(matches!(failed, Err(PortableBuildError::IncompleteTree)));
        assert_eq!(context.state_registry().revision(state_slot), None);
        assert_eq!(failed_updater.borrow().as_ref().unwrap().try_read(|state| state.count), None);

        context
            .with_build_transaction(|context| {
                context.seed_stateful_state::<CounterWidget>(
                    state_slot,
                    CounterState::new(2, "retry", initializations),
                )?;
                context.with_stateful_state::<CounterWidget, _>(
                    state_slot,
                    |state, _, _| {
                        assert_eq!(state.count, 2);
                        Ok(())
                    },
                )?;
                finish_generation(context, 410);
                Ok::<(), PortableBuildError>(())
            })
            .unwrap();
    }

    #[test]
    fn separate_stateful_slots_do_not_alias() {
        let initializations = Rc::new(Cell::new(0));
        let first = slot(5);
        let second = slot(6);
        let mut context = context();
        context
            .seed_stateful_state::<CounterWidget>(
                first,
                CounterState::new(1, "first", initializations.clone()),
            )
            .unwrap();
        context
            .seed_stateful_state::<CounterWidget>(
                second,
                CounterState::new(10, "second", initializations),
            )
            .unwrap();
        context
            .with_stateful_state::<CounterWidget, _>(first, |state, _, _| {
                state.updater.set_state(|state| state.count += 1);
                Ok(())
            })
            .unwrap();

        assert_eq!(context.state_registry().restore::<CounterState>(first).unwrap().count, 2);
        assert_eq!(context.state_registry().restore::<CounterState>(second).unwrap().count, 10);
    }

    #[test]
    fn callback_dispatch_drains_all_state_updates_refreshes_once_and_coalesces_rebuild() {
        let initializations = Rc::new(Cell::new(0));
        let state_slot = slot(7);
        let callback_id = slot(70);
        let mut context = context();
        context
            .seed_stateful_state::<CounterWidget>(
                state_slot,
                CounterState::new(2, "callback", initializations),
            )
            .unwrap();
            let updater = context
                .with_stateful_state::<CounterWidget, _>(state_slot, |state, _, _| {
                Ok(state.updater)
            })
            .unwrap();
        let node = context
            .push_node_with_callbacks(
                WidgetSchemaId::new(99),
                Version::new(1, 0),
                None,
                SourceFingerprint::new(slot(700)),
                &[],
                vec![super::super::PortableCallback::new(
                    EventId::new(1),
                    Version::new(1, 0),
                    callback_id,
                    move || {
                        updater.set_state(|state| state.count += 3);
                        updater.set_state(|state| state.count *= 2);
                        Ok(())
                    },
                )],
                &[],
            )
            .unwrap();
        context.finish_document(node).unwrap();
        let registry = context.take_callback_registry();

        registry.dispatch(callback_id, &mut context).unwrap();

        assert_eq!(context.state_registry().revision(state_slot), Some(1));
        assert_eq!(
            context
                .state_registry()
                .restore::<CounterState>(state_slot)
                .unwrap()
                .count,
            10
        );
        assert!(context.take_rebuild_request());
        assert!(!context.take_rebuild_request());
    }

    #[test]
    fn async_callback_state_is_drained_at_the_next_rebuild_boundary() {
        let initializations = Rc::new(Cell::new(0));
        let state_slot = slot(8);
        let callback_id = slot(80);
        let mut context = context();
        context
            .seed_stateful_state::<CounterWidget>(
                state_slot,
                CounterState::new(1, "async", initializations),
            )
            .unwrap();
        let updater = context
            .with_stateful_state::<CounterWidget, _>(state_slot, |state, _, _| {
                Ok(state.updater)
            })
            .unwrap();
        let node = context
            .push_node_with_callbacks(
                WidgetSchemaId::new(99),
                Version::new(1, 0),
                None,
                SourceFingerprint::new(slot(800)),
                &[],
                vec![super::super::PortableCallback::new_async(
                    EventId::new(1),
                    Version::new(1, 0),
                    AsyncCallbackSchemaMetadata::new(Version::new(1, 0), 1, 32),
                    callback_id,
                    move || {
                        let updater = updater;
                        Box::pin(async move {
                            updater.set_state(|state| state.count += 5);
                        })
                    },
                )],
                &[],
            )
            .unwrap();
        context.finish_document(node).unwrap();
        let callbacks = context.take_callback_registry();

        let started = callbacks.dispatch_start(callback_id, &mut context).unwrap();
        assert!(matches!(
            started,
            super::super::PortableCallbackStart::Started { .. }
        ));
        context.run_async_microtasks();
        assert_eq!(
            context
                .state_registry()
                .restore::<CounterState>(state_slot)
                .unwrap()
                .count,
            1,
            "the serialized snapshot remains unchanged until the rebuild boundary",
        );

        context.apply_queued_mutations().unwrap();
        assert_eq!(
            context
                .state_registry()
                .restore::<CounterState>(state_slot)
                .unwrap()
                .count,
            6,
        );
        assert!(context.take_rebuild_request());
    }
}
