use std::cell::Cell;
use std::collections::BTreeMap;
use std::fmt;
use std::rc::Rc;

use crate::{
    AsyncCallbackEventKind, AsyncCallbackEventView, AsyncTaskId, CallbackEvent, CallbackEventView,
    EventId, GenerationId, ModelError, ModelLimits, StableId128, Version, WidgetDocumentView,
};
use aimer_venus::{LocalScheduler, TaskId, TaskScope};

/// A failure while preparing or resolving callback bindings for one generation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CallbackBindingError {
    /// The callback event is not a canonical bounded `AEVT` document.
    EventDocument(ModelError),
    /// The candidate declares more callback bindings than the host permits.
    BindingLimitExceeded { count: usize, limit: u32 },
    /// Two candidate bindings claim the same stable callback identity.
    DuplicateCallbackId { callback_id: StableId128 },
    /// The event targets a generation other than this owner.
    GenerationMismatch {
        expected: GenerationId,
        actual: GenerationId,
    },
    /// The callback generation has retired and cannot accept another event.
    RetiredGeneration { generation_id: GenerationId },
    /// The active callback snapshot does not contain the requested identity.
    UnknownCallback { callback_id: StableId128 },
    /// The event source does not match the widget that owns the callback.
    WidgetKeyMismatch {
        callback_id: StableId128,
        expected: Option<StableId128>,
        actual: Option<StableId128>,
    },
    /// The callback does not accept the event's kind.
    EventKindMismatch {
        callback_id: StableId128,
        expected: EventId,
        actual: EventId,
    },
    /// The callback does not accept the event payload's schema.
    EventSchemaMismatch {
        callback_id: StableId128,
        expected: Version,
        actual: Version,
    },
    /// The event sequence was already consumed or arrived out of order.
    EventSequenceNotMonotonic { previous: u64, actual: u64 },
}

impl fmt::Display for CallbackBindingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EventDocument(error) => write!(formatter, "invalid callback event: {error}"),
            Self::BindingLimitExceeded { count, limit } => {
                write!(
                    formatter,
                    "callback binding count {count} exceeds limit {limit}"
                )
            }
            Self::DuplicateCallbackId { callback_id } => write!(
                formatter,
                "duplicate callback ID {:02x?}",
                callback_id.as_bytes()
            ),
            Self::GenerationMismatch { expected, actual } => write!(
                formatter,
                "callback event targets generation {} instead of {}",
                actual.get(),
                expected.get()
            ),
            Self::RetiredGeneration { generation_id } => write!(
                formatter,
                "callback generation {} has retired",
                generation_id.get()
            ),
            Self::UnknownCallback { callback_id } => write!(
                formatter,
                "callback ID {:02x?} is not active",
                callback_id.as_bytes()
            ),
            Self::WidgetKeyMismatch {
                callback_id,
                expected,
                actual,
            } => write!(
                formatter,
                "callback ID {:02x?} expects widget key {expected:?}, got {actual:?}",
                callback_id.as_bytes()
            ),
            Self::EventKindMismatch {
                callback_id,
                expected,
                actual,
            } => write!(
                formatter,
                "callback ID {:02x?} expects event kind {expected}, got {actual}",
                callback_id.as_bytes()
            ),
            Self::EventSchemaMismatch {
                callback_id,
                expected,
                actual,
            } => write!(
                formatter,
                "callback ID {:02x?} expects event schema {}.{}, got {}.{}",
                callback_id.as_bytes(),
                expected.major(),
                expected.minor(),
                actual.major(),
                actual.minor()
            ),
            Self::EventSequenceNotMonotonic { previous, actual } => write!(
                formatter,
                "callback event sequence {actual} does not follow consumed sequence {previous}"
            ),
        }
    }
}

impl std::error::Error for CallbackBindingError {}

impl From<ModelError> for CallbackBindingError {
    #[inline]
    fn from(error: ModelError) -> Self {
        Self::EventDocument(error)
    }
}

/// Limits applied to host-owned async callback tasks in one generation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GenerationAsyncLimits {
    max_in_flight_tasks: u32,
    max_completion_bytes: u32,
    max_callback_fuel: u32,
    max_retained_resources: u32,
}

impl GenerationAsyncLimits {
    /// Creates explicit ceilings for task count and completion payloads.
    #[inline]
    pub const fn new(max_in_flight_tasks: u32, max_completion_bytes: u32) -> Self {
        Self {
            max_in_flight_tasks,
            max_completion_bytes,
            max_callback_fuel: u32::MAX,
            max_retained_resources: u32::MAX,
        }
    }

    /// Sets the callback fuel ceiling advertised to the guest capability.
    #[inline]
    pub const fn with_max_callback_fuel(mut self, max_callback_fuel: u32) -> Self {
        self.max_callback_fuel = max_callback_fuel;
        self
    }

    /// Sets the retained async-resource ceiling advertised to the guest.
    #[inline]
    pub const fn with_max_retained_resources(mut self, max_retained_resources: u32) -> Self {
        self.max_retained_resources = max_retained_resources;
        self
    }

    /// Returns the maximum number of live async tasks.
    #[inline]
    pub const fn max_in_flight_tasks(self) -> u32 {
        self.max_in_flight_tasks
    }

    /// Returns the maximum completion payload size.
    #[inline]
    pub const fn max_completion_bytes(self) -> u32 {
        self.max_completion_bytes
    }

    /// Returns the callback fuel ceiling.
    #[inline]
    pub const fn max_callback_fuel(self) -> u32 {
        self.max_callback_fuel
    }

    /// Returns the retained async-resource ceiling.
    #[inline]
    pub const fn max_retained_resources(self) -> u32 {
        self.max_retained_resources
    }
}

impl Default for GenerationAsyncLimits {
    fn default() -> Self {
        Self::new(64, 4_096)
    }
}

/// Failure while registering or consuming a generation-owned async event.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AsyncCallbackError {
    /// The async event is not a canonical bounded `AASY` document.
    EventDocument(ModelError),
    /// The event targets another generation.
    GenerationMismatch {
        expected: GenerationId,
        actual: GenerationId,
    },
    /// The generation no longer accepts async work.
    RetiredGeneration,
    /// The callback identity is not active in this generation.
    UnknownCallback { callback_id: StableId128 },
    /// The callback exists but does not advertise an async contract.
    NotAsyncCallback { callback_id: StableId128 },
    /// The generation has reached its in-flight task ceiling.
    TaskLimitExceeded { limit: u32 },
    /// The non-repeating task identity space is exhausted.
    TaskIdentityExhausted,
    /// The completion names no live task in this generation.
    UnknownTask { task_id: AsyncTaskId },
    /// The completion names the right task but the wrong callback.
    CallbackMismatch {
        task_id: AsyncTaskId,
        expected: StableId128,
        actual: StableId128,
    },
    /// The completion event is too old or duplicated.
    EventSequenceNotMonotonic { previous: u64, actual: u64 },
    /// The completion payload exceeds the generation's async ceiling.
    CompletionTooLarge { length: usize, limit: u32 },
    /// Cancellation events must not carry a result payload.
    CancellationPayloadNotEmpty,
}

impl fmt::Display for AsyncCallbackError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EventDocument(error) => {
                write!(formatter, "invalid async callback event: {error}")
            }
            Self::GenerationMismatch { expected, actual } => write!(
                formatter,
                "async callback event targets generation {} instead of {}",
                actual.get(),
                expected.get()
            ),
            Self::RetiredGeneration => formatter.write_str("generation has retired"),
            Self::UnknownCallback { callback_id } => write!(
                formatter,
                "async callback ID {:02x?} is not active",
                callback_id.as_bytes()
            ),
            Self::NotAsyncCallback { callback_id } => write!(
                formatter,
                "callback ID {:02x?} does not advertise an async contract",
                callback_id.as_bytes()
            ),
            Self::TaskLimitExceeded { limit } => {
                write!(formatter, "async task limit {limit} is exhausted")
            }
            Self::TaskIdentityExhausted => {
                formatter.write_str("async task identities are exhausted")
            }
            Self::UnknownTask { task_id } => {
                write!(formatter, "async task {} is not active", task_id.get())
            }
            Self::CallbackMismatch {
                task_id,
                expected,
                actual,
            } => write!(
                formatter,
                "async task {} belongs to callback {:02x?}, not {:02x?}",
                task_id.get(),
                expected.as_bytes(),
                actual.as_bytes()
            ),
            Self::EventSequenceNotMonotonic { previous, actual } => write!(
                formatter,
                "async callback event sequence {actual} does not follow consumed sequence {previous}"
            ),
            Self::CompletionTooLarge { length, limit } => write!(
                formatter,
                "async completion payload {length} exceeds limit {limit}"
            ),
            Self::CancellationPayloadNotEmpty => {
                formatter.write_str("async cancellation carries a result payload")
            }
        }
    }
}

impl std::error::Error for AsyncCallbackError {}

impl From<ModelError> for AsyncCallbackError {
    #[inline]
    fn from(error: ModelError) -> Self {
        Self::EventDocument(error)
    }
}

/// Per-kind ceilings for host resources owned by one generation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GenerationLimits {
    max_resources_per_kind: u32,
    max_handle_serial: u64,
}

impl GenerationLimits {
    /// Creates limits with the same independent ceiling for every resource kind.
    #[inline]
    pub const fn new(max_resources_per_kind: u32) -> Self {
        Self {
            max_resources_per_kind,
            max_handle_serial: u64::MAX - 1,
        }
    }

    /// Sets the largest resource-handle serial this generation may issue.
    ///
    /// This independent ceiling permits deterministic exhaustion tests and
    /// prevents serial wraparound from reviving stale opaque handles.
    #[inline]
    pub const fn max_handle_serial(mut self, max_handle_serial: u64) -> Self {
        self.max_handle_serial = max_handle_serial;
        self
    }
}

/// The host resource classes whose lifetimes are bounded by one generation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GenerationResourceKind {
    /// A host timer that may produce callback events.
    Timer,
    /// A platform or application event subscription.
    Subscription,
    /// An in-flight asynchronous request.
    Request,
    /// An opaque handle returned by a capability provider.
    Capability,
}

impl GenerationResourceKind {
    const COUNT: usize = 4;

    #[inline]
    const fn index(self) -> usize {
        match self {
            Self::Timer => 0,
            Self::Subscription => 1,
            Self::Request => 2,
            Self::Capability => 3,
        }
    }
}

/// An opaque generation-tagged reference to one host resource.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GenerationHandle {
    generation_id: GenerationId,
    kind: GenerationResourceKind,
    slot: u32,
    serial: u64,
}

impl GenerationHandle {
    /// Returns the generation that owns this handle.
    #[inline]
    pub const fn generation_id(self) -> GenerationId {
        self.generation_id
    }

    /// Returns the resource class without exposing the host resource itself.
    #[inline]
    pub const fn kind(self) -> GenerationResourceKind {
        self.kind
    }
}

/// A host-owned resource that has an explicit deterministic release operation.
///
/// Implementations wrap timers, subscriptions, requests, or native capability
/// handles. They must not retain guest pointers; the generation calls
/// [`Self::release`] exactly once after successful registration.
pub trait GenerationResource: 'static {
    /// Cancels or releases the resource.
    fn release(self: Box<Self>);
}

/// A failure while registering, using, or completing generation-owned work.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GenerationResourceError {
    /// The generation is retired and cannot accept work.
    RetiredGeneration,
    /// The handle belongs to another generation.
    GenerationMismatch {
        expected: GenerationId,
        actual: GenerationId,
    },
    /// The opaque handle is stale, released, or otherwise unknown.
    InvalidHandle { handle: GenerationHandle },
    /// One independently bounded resource class reached its ceiling.
    LimitExceeded {
        kind: GenerationResourceKind,
        limit: u32,
    },
    /// The generation exhausted its non-repeating handle serial space.
    HandleExhausted,
}

impl fmt::Display for GenerationResourceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RetiredGeneration => formatter.write_str("generation has retired"),
            Self::GenerationMismatch { expected, actual } => write!(
                formatter,
                "resource belongs to generation {}, not {}",
                actual.get(),
                expected.get()
            ),
            Self::InvalidHandle { handle } => write!(
                formatter,
                "resource handle for generation {} is invalid",
                handle.generation_id.get()
            ),
            Self::LimitExceeded { kind, limit } => {
                write!(formatter, "{kind:?} resource limit {limit} is exhausted")
            }
            Self::HandleExhausted => {
                formatter.write_str("generation resource handles are exhausted")
            }
        }
    }
}

impl std::error::Error for GenerationResourceError {}

/// A guard that rejects asynchronous delivery after its generation retires.
#[derive(Clone, Debug)]
pub struct GenerationCompletionToken {
    active: Rc<Cell<bool>>,
}

impl GenerationCompletionToken {
    /// Delivers `completion` only while the owning generation remains active.
    pub fn deliver<T>(self, completion: impl FnOnce() -> T) -> Result<T, GenerationResourceError> {
        if self.active.get() {
            Ok(completion())
        } else {
            Err(GenerationResourceError::RetiredGeneration)
        }
    }
}

/// One complete, immutable callback table prepared from validated Widget IR.
///
/// Entries are copied from the candidate image because the active native tree
/// outlives temporary Widget IR views. The table remains sorted by callback ID,
/// allowing resolution without a hash table or retained guest pointers.
#[derive(Debug)]
pub struct CallbackBindingSnapshot {
    bindings: Vec<OwnedCallbackBinding>,
}

impl CallbackBindingSnapshot {
    /// Creates a callback snapshot containing no bindings.
    ///
    /// This is useful for an initial or headless generation whose native tree
    /// cannot emit guest callbacks. The snapshot remains immutable after
    /// construction, just like one prepared from Widget IR.
    #[inline]
    pub const fn empty() -> Self {
        Self {
            bindings: Vec::new(),
        }
    }

    /// Copies and validates every callback binding in `document`.
    ///
    /// Duplicate callback IDs reject the complete snapshot even when they are
    /// attached to different widget nodes. No partially built table is exposed.
    pub fn from_document(
        document: &WidgetDocumentView<'_>,
        max_bindings: u32,
    ) -> Result<Self, CallbackBindingError> {
        let mut bindings = Vec::new();
        for node_index in 0..document.node_count() {
            let node = document.node(node_index).expect("validated node index");
            for binding in node.callbacks() {
                let count = bindings.len() + 1;
                if count > max_bindings as usize {
                    return Err(CallbackBindingError::BindingLimitExceeded {
                        count,
                        limit: max_bindings,
                    });
                }
                bindings.push(OwnedCallbackBinding {
                    callback_id: binding.callback_id(),
                    widget_key: node.key(),
                    event_kind: binding.event_kind(),
                    event_schema: binding.event_schema(),
                    async_schema: binding.async_schema(),
                });
            }
        }
        bindings.sort_unstable_by_key(|binding| binding.callback_id);
        if let Some(duplicate) = bindings
            .windows(2)
            .find(|pair| pair[0].callback_id == pair[1].callback_id)
        {
            return Err(CallbackBindingError::DuplicateCallbackId {
                callback_id: duplicate[0].callback_id,
            });
        }
        Ok(Self { bindings })
    }

    fn find(&self, callback_id: StableId128) -> Option<&OwnedCallbackBinding> {
        self.bindings
            .binary_search_by_key(&callback_id, |binding| binding.callback_id)
            .ok()
            .map(|index| &self.bindings[index])
    }
}

#[derive(Debug)]
struct OwnedCallbackBinding {
    callback_id: StableId128,
    widget_key: Option<StableId128>,
    event_kind: EventId,
    event_schema: Version,
    async_schema: Option<Version>,
}

#[derive(Clone, Copy, Debug)]
struct AsyncTaskRecord {
    callback_id: StableId128,
}

struct ResourceSlot {
    serial: u64,
    kind: GenerationResourceKind,
    resource: Option<Box<dyn GenerationResource>>,
}

/// The coordinator-owned callback lifecycle for one application generation.
///
/// This initial owner contains only the immutable callback snapshot and event
/// replay state. Generation-scoped tasks and host resources are added through
/// the same lifecycle without coupling the public boundary to an interpreter.
pub struct Generation<G = ()> {
    id: GenerationId,
    callbacks: CallbackBindingSnapshot,
    last_event_sequence: Option<u64>,
    active: Rc<Cell<bool>>,
    limits: GenerationLimits,
    scheduler: Rc<LocalScheduler>,
    task_scope: TaskScope,
    async_limits: GenerationAsyncLimits,
    async_tasks: BTreeMap<AsyncTaskId, AsyncTaskRecord>,
    next_async_task_id: u64,
    last_async_event_sequence: Option<u64>,
    resources: Vec<ResourceSlot>,
    free_resource_slots: Vec<u32>,
    resource_counts: [u32; GenerationResourceKind::COUNT],
    next_resource_serial: u64,
    guest: G,
}

impl Generation<()> {
    /// Creates a callback-only generation with no host-resource capacity.
    ///
    /// Hosts that schedule work should use [`Self::with_scheduler`] so the
    /// generation shares the permanent event loop's scheduler.
    #[inline]
    pub fn new(id: GenerationId, callbacks: CallbackBindingSnapshot) -> Self {
        Self::with_scheduler(
            id,
            callbacks,
            LocalScheduler::new(),
            GenerationLimits::new(0),
        )
    }

    /// Creates a generation that owns one scope in the host scheduler.
    #[inline]
    pub fn with_scheduler(
        id: GenerationId,
        callbacks: CallbackBindingSnapshot,
        scheduler: Rc<LocalScheduler>,
        limits: GenerationLimits,
    ) -> Self {
        Generation::with_guest(id, callbacks, scheduler, limits, ())
    }
}

impl<G> Generation<G> {
    /// Creates a generation that owns `guest` and every associated host resource.
    pub fn with_guest(
        id: GenerationId,
        callbacks: CallbackBindingSnapshot,
        scheduler: Rc<LocalScheduler>,
        limits: GenerationLimits,
        guest: G,
    ) -> Self {
        let task_scope = scheduler.scope();
        Self {
            id,
            callbacks,
            last_event_sequence: None,
            active: Rc::new(Cell::new(true)),
            limits,
            scheduler,
            task_scope,
            async_limits: GenerationAsyncLimits::default(),
            async_tasks: BTreeMap::new(),
            next_async_task_id: 1,
            last_async_event_sequence: None,
            resources: Vec::new(),
            free_resource_slots: Vec::new(),
            resource_counts: [0; GenerationResourceKind::COUNT],
            next_resource_serial: 1,
            guest,
        }
    }

    /// Returns the reload-coordinator identity of this generation.
    #[inline]
    pub const fn generation_id(&self) -> GenerationId {
        self.id
    }

    /// Borrows the guest payload owned by this generation.
    #[inline]
    pub const fn guest(&self) -> &G {
        &self.guest
    }

    /// Mutably borrows the guest payload for one serialized generation call.
    #[inline]
    pub const fn guest_mut(&mut self) -> &mut G {
        &mut self.guest
    }

    /// Encodes one callback invocation from the immutable active binding.
    ///
    /// The generation supplies the trusted widget key, event kind, and schema;
    /// callers provide only the stable callback identity and event-local data.
    /// This prevents a native widget closure from duplicating or forging ABI
    /// metadata while still producing a canonical bounded `AEVT` document.
    pub fn encode_callback_event(
        &self,
        callback_id: StableId128,
        event_sequence: u64,
        monotonic_timestamp: u64,
        payload: &[u8],
        limits: ModelLimits,
    ) -> Result<Vec<u8>, CallbackBindingError> {
        if !self.active.get() {
            return Err(CallbackBindingError::RetiredGeneration {
                generation_id: self.id,
            });
        }
        let binding = self
            .callbacks
            .find(callback_id)
            .ok_or(CallbackBindingError::UnknownCallback { callback_id })?;
        let mut event = CallbackEvent::new(
            self.id.get(),
            event_sequence,
            callback_id,
            binding.event_kind,
            binding.event_schema,
            monotonic_timestamp,
            payload,
        );
        if let Some(widget_key) = binding.widget_key {
            event = event.widget_key(widget_key);
        }
        event
            .encode(limits)
            .map_err(CallbackBindingError::EventDocument)
    }

    /// Replaces bindings after a callback rebuild within this generation.
    ///
    /// The replacement is decoded completely before publication, so malformed
    /// or over-limit Widget IR leaves the currently dispatchable table intact.
    pub fn replace_callback_bindings(
        &mut self,
        document: &WidgetDocumentView<'_>,
        max_bindings: u32,
    ) -> Result<(), CallbackBindingError> {
        let callbacks = CallbackBindingSnapshot::from_document(document, max_bindings)?;
        self.async_tasks.retain(|_, task| {
            callbacks
                .find(task.callback_id)
                .is_some_and(|binding| binding.async_schema.is_some())
        });
        self.callbacks = callbacks;
        Ok(())
    }

    /// Validates and consumes one callback event before guest dispatch.
    ///
    /// The sequence advances only after generation, identity, widget, kind, and
    /// schema checks succeed. A rejected event can therefore never consume a
    /// sequence number or fall through to a previous generation's callback.
    pub fn validate_event<'a>(
        &mut self,
        event_bytes: &'a [u8],
        limits: ModelLimits,
    ) -> Result<CallbackEventView<'a>, CallbackBindingError> {
        if !self.active.get() {
            return Err(CallbackBindingError::RetiredGeneration {
                generation_id: self.id,
            });
        }
        let event = CallbackEventView::decode(event_bytes, limits)?;
        let actual_generation = GenerationId::new(event.generation_id());
        if actual_generation != self.id {
            return Err(CallbackBindingError::GenerationMismatch {
                expected: self.id,
                actual: actual_generation,
            });
        }
        let callback_id = event.callback_id();
        let binding = self
            .callbacks
            .find(callback_id)
            .ok_or(CallbackBindingError::UnknownCallback { callback_id })?;
        if event.widget_key() != binding.widget_key {
            return Err(CallbackBindingError::WidgetKeyMismatch {
                callback_id,
                expected: binding.widget_key,
                actual: event.widget_key(),
            });
        }
        if event.event_kind() != binding.event_kind {
            return Err(CallbackBindingError::EventKindMismatch {
                callback_id,
                expected: binding.event_kind,
                actual: event.event_kind(),
            });
        }
        if event.event_schema() != binding.event_schema {
            return Err(CallbackBindingError::EventSchemaMismatch {
                callback_id,
                expected: binding.event_schema,
                actual: event.event_schema(),
            });
        }
        let sequence = event.event_sequence();
        if let Some(previous) = self.last_event_sequence
            && sequence <= previous
        {
            return Err(CallbackBindingError::EventSequenceNotMonotonic {
                previous,
                actual: sequence,
            });
        }
        self.last_event_sequence = Some(sequence);
        Ok(event)
    }

    /// Stops this generation from accepting new callback events.
    ///
    /// Retirement is idempotent so cleanup paths may call it defensively.
    #[inline]
    pub fn retire(&mut self) {
        let result = self.retire_with_disposal(|| Ok::<(), std::convert::Infallible>(()));
        debug_assert!(result.is_ok());
    }

    /// Replaces the generation's async task ceilings before work is started.
    ///
    /// Existing task registrations are intentionally not reconfigured. A
    /// candidate must negotiate its limits before exposing a task to the host.
    #[inline]
    pub fn with_async_limits(mut self, limits: GenerationAsyncLimits) -> Self {
        debug_assert!(self.async_tasks.is_empty());
        self.async_limits = limits;
        self
    }

    /// Returns the async task ceilings owned by this generation.
    #[inline]
    pub const fn async_limits(&self) -> GenerationAsyncLimits {
        self.async_limits
    }

    /// Registers one host-owned async task for an active async callback.
    ///
    /// The returned identity is generation-local and is the only task handle
    /// that may appear in a later `AASY` completion. The request itself must be
    /// implemented by a typed capability or another host resource; no native
    /// executor handle or closure is retained in the portable document.
    pub fn register_async_task(
        &mut self,
        callback_id: StableId128,
    ) -> Result<AsyncTaskId, AsyncCallbackError> {
        if !self.active.get() {
            return Err(AsyncCallbackError::RetiredGeneration);
        }
        let binding = self
            .callbacks
            .find(callback_id)
            .ok_or(AsyncCallbackError::UnknownCallback { callback_id })?;
        if binding.async_schema.is_none() {
            return Err(AsyncCallbackError::NotAsyncCallback { callback_id });
        }
        let maximum = self
            .async_limits
            .max_in_flight_tasks
            .min(self.async_limits.max_retained_resources);
        if self.async_tasks.len() >= maximum as usize {
            return Err(AsyncCallbackError::TaskLimitExceeded { limit: maximum });
        }
        let task_id = AsyncTaskId::new(self.next_async_task_id);
        self.next_async_task_id = self
            .next_async_task_id
            .checked_add(1)
            .ok_or(AsyncCallbackError::TaskIdentityExhausted)?;
        self.async_tasks
            .insert(task_id, AsyncTaskRecord { callback_id });
        Ok(task_id)
    }

    /// Cancels one live host-owned async task without consuming an event slot.
    pub fn cancel_async_task(&mut self, task_id: AsyncTaskId) -> Result<(), AsyncCallbackError> {
        if !self.active.get() {
            return Err(AsyncCallbackError::RetiredGeneration);
        }
        self.async_tasks
            .remove(&task_id)
            .map(|_| ())
            .ok_or(AsyncCallbackError::UnknownTask { task_id })
    }

    /// Returns the number of live host-owned async tasks.
    #[inline]
    pub fn async_task_count(&self) -> usize {
        self.async_tasks.len()
    }

    /// Validates and consumes one bounded async completion or cancellation.
    ///
    /// Every check happens before the task is removed or the event sequence is
    /// advanced. A malformed, stale, duplicated, out-of-order, or over-limit
    /// event therefore leaves the active generation unchanged.
    pub fn validate_async_event<'a>(
        &mut self,
        event_bytes: &'a [u8],
        limits: ModelLimits,
    ) -> Result<AsyncCallbackEventView<'a>, AsyncCallbackError> {
        if !self.active.get() {
            return Err(AsyncCallbackError::RetiredGeneration);
        }
        let event = AsyncCallbackEventView::decode(event_bytes, limits)?;
        let actual_generation = GenerationId::new(event.generation_id());
        if actual_generation != self.id {
            return Err(AsyncCallbackError::GenerationMismatch {
                expected: self.id,
                actual: actual_generation,
            });
        }
        let task_id = event.task_id();
        let task = self
            .async_tasks
            .get(&task_id)
            .ok_or(AsyncCallbackError::UnknownTask { task_id })?;
        if task.callback_id != event.callback_id() {
            return Err(AsyncCallbackError::CallbackMismatch {
                task_id,
                expected: task.callback_id,
                actual: event.callback_id(),
            });
        }
        if event.payload().len() > self.async_limits.max_completion_bytes as usize {
            return Err(AsyncCallbackError::CompletionTooLarge {
                length: event.payload().len(),
                limit: self.async_limits.max_completion_bytes,
            });
        }
        if event.kind() == AsyncCallbackEventKind::Cancelled && !event.payload().is_empty() {
            return Err(AsyncCallbackError::CancellationPayloadNotEmpty);
        }
        let sequence = event.event_sequence();
        if let Some(previous) = self.last_async_event_sequence
            && sequence <= previous
        {
            return Err(AsyncCallbackError::EventSequenceNotMonotonic {
                previous,
                actual: sequence,
            });
        }
        self.last_async_event_sequence = Some(sequence);
        self.async_tasks.remove(&task_id);
        Ok(event)
    }

    /// Spawns one microtask in this generation's cancellation scope.
    pub fn spawn_task(
        &self,
        future: impl Future<Output = ()> + 'static,
    ) -> Result<TaskId, GenerationResourceError> {
        self.ensure_active()?;
        Ok(self.scheduler.spawn_in(self.task_scope.id(), future))
    }

    /// Registers one bounded host resource and returns an opaque owner-tagged handle.
    pub fn register_resource(
        &mut self,
        kind: GenerationResourceKind,
        resource: impl GenerationResource,
    ) -> Result<GenerationHandle, GenerationResourceError> {
        let resource = Box::new(resource) as Box<dyn GenerationResource>;
        if let Err(error) = self.ensure_active() {
            resource.release();
            return Err(error);
        }
        if self.resource_counts[kind.index()] >= self.limits.max_resources_per_kind {
            resource.release();
            return Err(GenerationResourceError::LimitExceeded {
                kind,
                limit: self.limits.max_resources_per_kind,
            });
        }
        if self.next_resource_serial > self.limits.max_handle_serial {
            resource.release();
            return Err(GenerationResourceError::HandleExhausted);
        }
        let serial = self.next_resource_serial;
        let Some(next_serial) = serial.checked_add(1) else {
            resource.release();
            return Err(GenerationResourceError::HandleExhausted);
        };
        let slot = if let Some(slot) = self.free_resource_slots.pop() {
            self.resources[slot as usize] = ResourceSlot {
                serial,
                kind,
                resource: Some(resource),
            };
            slot
        } else {
            let Ok(slot) = u32::try_from(self.resources.len()) else {
                resource.release();
                return Err(GenerationResourceError::HandleExhausted);
            };
            self.resources.push(ResourceSlot {
                serial,
                kind,
                resource: Some(resource),
            });
            slot
        };
        self.next_resource_serial = next_serial;
        self.resource_counts[kind.index()] += 1;
        Ok(GenerationHandle {
            generation_id: self.id,
            kind,
            slot,
            serial,
        })
    }

    /// Releases one resource after validating its generation and incarnation.
    pub fn release_resource(
        &mut self,
        handle: GenerationHandle,
    ) -> Result<(), GenerationResourceError> {
        if handle.generation_id != self.id {
            return Err(GenerationResourceError::GenerationMismatch {
                expected: self.id,
                actual: handle.generation_id,
            });
        }
        let Some(slot) = self.resources.get_mut(handle.slot as usize) else {
            return Err(GenerationResourceError::InvalidHandle { handle });
        };
        if slot.serial != handle.serial || slot.kind != handle.kind {
            return Err(GenerationResourceError::InvalidHandle { handle });
        }
        let Some(resource) = slot.resource.take() else {
            return Err(GenerationResourceError::InvalidHandle { handle });
        };
        self.resource_counts[slot.kind.index()] -= 1;
        self.free_resource_slots.push(handle.slot);
        resource.release();
        Ok(())
    }

    /// Creates a late-delivery guard sharing this generation's activity state.
    #[inline]
    pub fn completion_token(&self) -> GenerationCompletionToken {
        GenerationCompletionToken {
            active: self.active.clone(),
        }
    }

    /// Retires and cleans up before returning the guest disposal result.
    ///
    /// The activity flag is cleared first, then tasks and resources are
    /// cancelled. `dispose` runs only once and its error is preserved after all
    /// host-owned cleanup has completed.
    pub fn retire_with_disposal<E>(
        &mut self,
        dispose: impl FnOnce() -> Result<(), E>,
    ) -> Result<(), E> {
        if !self.active.replace(false) {
            return Ok(());
        }
        self.task_scope.cancel();
        self.async_tasks.clear();
        self.release_all_resources();
        dispose()
    }

    #[inline]
    fn ensure_active(&self) -> Result<(), GenerationResourceError> {
        if self.active.get() {
            Ok(())
        } else {
            Err(GenerationResourceError::RetiredGeneration)
        }
    }

    fn release_all_resources(&mut self) {
        for slot in &mut self.resources {
            if let Some(resource) = slot.resource.take() {
                self.resource_counts[slot.kind.index()] -= 1;
                resource.release();
            }
        }
        self.free_resource_slots.clear();
    }
}

impl<G> Drop for Generation<G> {
    fn drop(&mut self) {
        self.retire();
    }
}
