use std::any::{Any, TypeId};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::error::Error;
use std::fmt;
use std::future::Future;
use std::hash::{Hash, Hasher};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll};

use aimer_anteros::{
    AsyncCallbackEventKind, AsyncCallbackEventView, AsyncCallbackSchemaMetadata, CallbackBinding,
    EventId, GuestDiagnostic, GuestDiagnosticCategory, GuestOperation, ModelError, ModelLimits,
    PropertyId, PropertyValue, StableId128 as AnterosId, Version, WidgetDocument, WidgetProperty,
    WidgetSchemaId,
};
use aimer_venus::{LocalScheduler, TaskId, TaskScope};

use super::codec::{PortableDecode, PortableEncode, PortableLimits};
use super::identity::{StableHasher, StableId128, StableSlotId};
use super::registry::{StateRegistry, StateRegistryError};
use super::schema::AimerReflectionType;
use super::semantic_graph::PortableSemanticGraph;
use super::state::PortableLiveStateRegistry;
use crate::Key;

const DOCUMENT_HEADER_BYTES: usize = 64;
const NODE_BYTES: usize = 64;
const PROPERTY_BYTES: usize = 32;
const CALLBACK_BYTES: usize = 40;
const CHILD_BYTES: usize = 4;
const STRING_RANGE_BYTES: usize = 8;
const BLOB_RANGE_BYTES: usize = 8;

/// Identifies the bounded Widget IR resource that rejected an operation.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PortableWidgetResource {
    /// Number of widget nodes.
    Nodes,
    /// Aggregate number of widget properties.
    Properties,
    /// Aggregate number of callback bindings.
    Callbacks,
    /// Aggregate number of child edges.
    Children,
    /// Number of stable node identities.
    Keys,
    /// Bytes in one UTF-8 string.
    StringBytes,
    /// Bytes in one opaque blob.
    BlobBytes,
    /// Bytes in the complete encoded Widget IR document.
    DocumentBytes,
    /// Number of deferred state mutations.
    Mutations,
}

/// Explicit resource ceilings for owned portable Widget IR construction.
///
/// Every operation checks its resulting count and exact encoded byte cost
/// before growing owned storage. The limits are independent so applications can
/// constrain fan-out, property density, and identity count separately.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PortableWidgetLimits {
    max_nodes: usize,
    max_properties: usize,
    max_callbacks: usize,
    max_children: usize,
    max_keys: usize,
    max_string_bytes: usize,
    max_blob_bytes: usize,
    max_document_bytes: usize,
}

impl PortableWidgetLimits {
    /// Creates Widget IR construction ceilings with blob storage disabled.
    ///
    /// Call [`Self::with_max_blob_bytes`] to opt into bounded blob payloads.
    #[inline]
    pub const fn new(
        max_nodes: usize,
        max_properties: usize,
        max_children: usize,
        max_keys: usize,
        max_string_bytes: usize,
        max_document_bytes: usize,
    ) -> Self {
        Self {
            max_nodes,
            max_properties,
            max_callbacks: max_properties,
            max_children,
            max_keys,
            max_string_bytes,
            max_blob_bytes: 0,
            max_document_bytes,
        }
    }

    /// Replaces the node-count ceiling.
    #[inline]
    pub const fn with_max_nodes(mut self, maximum: usize) -> Self {
        self.max_nodes = maximum;
        self
    }

    /// Replaces the aggregate property-count ceiling.
    #[inline]
    pub const fn with_max_properties(mut self, maximum: usize) -> Self {
        self.max_properties = maximum;
        self
    }

    /// Replaces the aggregate callback-count ceiling.
    #[inline]
    pub const fn with_max_callbacks(mut self, maximum: usize) -> Self {
        self.max_callbacks = maximum;
        self
    }

    pub(super) fn model_limits(self) -> ModelLimits {
        ModelLimits::new(
            self.max_document_bytes as u32,
            u32::MAX,
            self.max_string_bytes as u32,
            self.max_blob_bytes as u32,
        )
        .max_widget_depth(self.max_nodes as u32)
    }

    /// Replaces the aggregate child-edge ceiling.
    #[inline]
    pub const fn with_max_children(mut self, maximum: usize) -> Self {
        self.max_children = maximum;
        self
    }

    /// Replaces the stable-key count ceiling.
    #[inline]
    pub const fn with_max_keys(mut self, maximum: usize) -> Self {
        self.max_keys = maximum;
        self
    }

    /// Replaces the per-string UTF-8 byte ceiling.
    #[inline]
    pub const fn with_max_string_bytes(mut self, maximum: usize) -> Self {
        self.max_string_bytes = maximum;
        self
    }

    /// Replaces the per-blob byte ceiling.
    #[inline]
    pub const fn with_max_blob_bytes(mut self, maximum: usize) -> Self {
        self.max_blob_bytes = maximum;
        self
    }

    /// Replaces the complete encoded-document byte ceiling.
    #[inline]
    pub const fn with_max_document_bytes(mut self, maximum: usize) -> Self {
        self.max_document_bytes = maximum;
        self
    }
}

/// A generated source-site identity used when a widget has no explicit key.
///
/// Macro expansion should inject a deterministic fingerprint of the declaring
/// package, module, item, and expression site. It must not include absolute
/// filesystem paths or compiler-session data.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct SourceFingerprint(StableId128);

impl SourceFingerprint {
    /// Creates a fingerprint from generated stable metadata.
    #[inline]
    pub const fn new(identity: StableId128) -> Self {
        Self(identity)
    }

    /// Returns the generated stable identity.
    #[inline]
    pub const fn identity(self) -> StableId128 {
        self.0
    }

    /// Derives one deterministic nested source identity without allocating.
    ///
    /// Generated wrappers use discriminators that are stable within their
    /// expansion, such as a child field index. Framing and domain separation
    /// prevent a nested identity from colliding with either its parent or a
    /// fingerprint produced directly from source metadata.
    #[inline]
    pub const fn child(self, discriminator: u64) -> Self {
        let mut hasher = StableHasher::new();
        hasher.write_str("aimer.portable.source.child.v1");
        hasher.write_bytes(&self.0.to_bytes());
        hasher.write_u64(discriminator);
        Self(hasher.finish())
    }
}

/// An index into the document currently owned by a [`PortableBuildContext`].
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct PortableNodeId(u32);

impl PortableNodeId {
    #[inline]
    pub(super) const fn new(index: u32) -> Self { Self(index) }
    /// Returns the document-local node index.
    #[inline]
    pub const fn index(self) -> u32 {
        self.0
    }
}

/// An owned explanation returned by a portable callback body.
#[doc(hidden)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PortableCallbackFailure(String);

impl PortableCallbackFailure {
    /// Creates a callback failure from a stable diagnostic message.
    #[inline]
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for PortableCallbackFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// The bounded result of starting one portable callback.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PortableCallbackStart {
    /// A synchronous callback completed before the dispatch returned.
    Completed,
    /// An async callback owns a guest task in the current generation.
    Started { task_id: PortableTaskId },
}

/// A generation-local identity for one guest-owned async callback task.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct PortableTaskId(u64);

impl PortableTaskId {
    /// Returns the generation-local task number.
    #[inline]
    pub const fn value(self) -> u64 {
        self.0
    }
}

/// Bounds applied to async callback work retained by one portable generation.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PortableAsyncLimits {
    max_in_flight_tasks: usize,
    max_completion_bytes: usize,
    max_callback_fuel: u32,
    max_retained_resources: usize,
}

impl PortableAsyncLimits {
    /// Creates explicit async task, completion, fuel, and resource ceilings.
    #[inline]
    pub const fn new(
        max_in_flight_tasks: usize,
        max_completion_bytes: usize,
        max_callback_fuel: u32,
        max_retained_resources: usize,
    ) -> Self {
        Self {
            max_in_flight_tasks,
            max_completion_bytes,
            max_callback_fuel,
            max_retained_resources,
        }
    }

    /// Returns the maximum number of live guest tasks.
    #[inline]
    pub const fn max_in_flight_tasks(self) -> usize {
        self.max_in_flight_tasks
    }

    /// Returns the maximum completion payload size.
    #[inline]
    pub const fn max_completion_bytes(self) -> usize {
        self.max_completion_bytes
    }

    /// Returns the callback fuel budget.
    #[inline]
    pub const fn max_callback_fuel(self) -> u32 {
        self.max_callback_fuel
    }

    /// Returns the retained-resource ceiling.
    #[inline]
    pub const fn max_retained_resources(self) -> usize {
        self.max_retained_resources
    }
}

impl Default for PortableAsyncLimits {
    fn default() -> Self {
        Self::new(64, 4_096, u32::MAX, 64)
    }
}

/// A bounded diagnostic produced by a guest or host-owned async task.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PortableAsyncFailure {
    task_id: PortableTaskId,
    callback_id: StableId128,
    message: String,
}

impl PortableAsyncFailure {
    /// Returns the task that produced the failure.
    #[inline]
    pub const fn task_id(&self) -> PortableTaskId {
        self.task_id
    }

    /// Returns the stable callback identity that owns the task.
    #[inline]
    pub const fn callback_id(&self) -> StableId128 {
        self.callback_id
    }

    /// Returns the bounded, secret-free diagnostic message.
    #[inline]
    pub fn message(&self) -> &str {
        &self.message
    }
}

struct PortableAsyncTask {
    scheduler_id: Option<TaskId>,
    callback_id: StableId128,
    host_owned: bool,
    completion_limit: usize,
}

struct PortableAsyncCompletion {
    task_id: PortableTaskId,
    callback_id: StableId128,
    result: Result<(), String>,
}

struct PortableAsyncState {
    next_task_id: u64,
    tasks: BTreeMap<PortableTaskId, PortableAsyncTask>,
    completions: VecDeque<PortableAsyncCompletion>,
    failures: VecDeque<PortableAsyncFailure>,
    last_event_sequence: Option<u64>,
    limits: PortableAsyncLimits,
}

struct PortableAsyncRuntime {
    scheduler: Rc<LocalScheduler>,
    scope: TaskScope,
    state: Rc<RefCell<PortableAsyncState>>,
}

struct PortableAsyncFuture {
    future: Pin<Box<dyn Future<Output = ()> + 'static>>,
    state: Rc<RefCell<PortableAsyncState>>,
    task_id: PortableTaskId,
    callback_id: StableId128,
    fuel_remaining: u32,
}

impl Future for PortableAsyncFuture {
    type Output = ();

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        if this.fuel_remaining == 0 {
            this.state
                .borrow_mut()
                .completions
                .push_back(PortableAsyncCompletion {
                    task_id: this.task_id,
                    callback_id: this.callback_id,
                    result: Err("async callback fuel exhausted".to_owned()),
                });
            return Poll::Ready(());
        }
        this.fuel_remaining -= 1;
        let result = catch_unwind(AssertUnwindSafe(|| this.future.as_mut().poll(context)));
        match result {
            Ok(Poll::Pending) => Poll::Pending,
            Ok(Poll::Ready(())) => {
                this.state
                    .borrow_mut()
                    .completions
                    .push_back(PortableAsyncCompletion {
                        task_id: this.task_id,
                        callback_id: this.callback_id,
                        result: Ok(()),
                    });
                Poll::Ready(())
            }
            Err(_) => {
                this.state
                    .borrow_mut()
                    .completions
                    .push_back(PortableAsyncCompletion {
                        task_id: this.task_id,
                        callback_id: this.callback_id,
                        result: Err("async callback panicked".to_owned()),
                    });
                Poll::Ready(())
            }
        }
    }
}

impl PortableAsyncRuntime {
    fn new(limits: PortableAsyncLimits) -> Self {
        let scheduler = LocalScheduler::new();
        let scope = scheduler.scope();
        Self {
            scheduler,
            scope,
            state: Rc::new(RefCell::new(PortableAsyncState {
                next_task_id: 1,
                tasks: BTreeMap::new(),
                completions: VecDeque::new(),
                failures: VecDeque::new(),
                last_event_sequence: None,
                limits,
            })),
        }
    }

    fn start(
        &self,
        callback_id: StableId128,
        schema: AsyncCallbackSchemaMetadata,
        future: Pin<Box<dyn Future<Output = ()> + 'static>>,
    ) -> Result<PortableTaskId, PortableCallbackFailure> {
        let (task_id, fuel_remaining) = {
            let mut state = self.state.borrow_mut();
            let maximum = task_capacity(&state, schema);
            if state.tasks.len() >= maximum {
                return Err(PortableCallbackFailure::new(format!(
                    "async retained resource capacity exceeded: maximum {}",
                    maximum
                )));
            }
            let task_id = PortableTaskId(state.next_task_id);
            state.next_task_id = state
                .next_task_id
                .checked_add(1)
                .ok_or_else(|| PortableCallbackFailure::new("async task identity exhausted"))?;
            let fuel_remaining = state
                .limits
                .max_callback_fuel
                .min(schema.maximum_callback_fuel());
            (task_id, fuel_remaining)
        };
        let wrapped = PortableAsyncFuture {
            future,
            state: self.state.clone(),
            task_id,
            callback_id,
            fuel_remaining,
        };
        let scheduler_id = self.scheduler.spawn_in(self.scope.id(), wrapped);
        self.state.borrow_mut().tasks.insert(
            task_id,
            PortableAsyncTask {
                scheduler_id: Some(scheduler_id),
                callback_id,
                host_owned: false,
                completion_limit: schema.maximum_completion_bytes() as usize,
            },
        );
        Ok(task_id)
    }

    fn register_host_task(
        &self,
        callback_id: StableId128,
        schema: AsyncCallbackSchemaMetadata,
    ) -> Result<PortableTaskId, PortableCallbackFailure> {
        let mut state = self.state.borrow_mut();
        let maximum = task_capacity(&state, schema);
        if state.tasks.len() >= maximum {
            return Err(PortableCallbackFailure::new(format!(
                "async retained resource capacity exceeded: maximum {}",
                maximum
            )));
        }
        let task_id = PortableTaskId(state.next_task_id);
        state.next_task_id = state
            .next_task_id
            .checked_add(1)
            .ok_or_else(|| PortableCallbackFailure::new("async task identity exhausted"))?;
        state.tasks.insert(
            task_id,
            PortableAsyncTask {
                scheduler_id: None,
                callback_id,
                host_owned: true,
                completion_limit: schema.maximum_completion_bytes() as usize,
            },
        );
        Ok(task_id)
    }

    fn cancel(&self, task_id: PortableTaskId) -> bool {
        let Some(task) = self.state.borrow_mut().tasks.remove(&task_id) else {
            return false;
        };
        task.scheduler_id
            .map(|scheduler_id| self.scheduler.abort(scheduler_id))
            .unwrap_or(true)
    }

    fn run_microtasks(&self) -> (usize, usize) {
        let polled = self.scheduler.run_microtasks();
        let mut state = self.state.borrow_mut();
        let mut completed = 0;
        while let Some(completion) = state.completions.pop_front() {
            if state.tasks.remove(&completion.task_id).is_none() {
                continue;
            }
            completed += 1;
            if let Err(message) = completion.result {
                state.failures.push_back(PortableAsyncFailure {
                    task_id: completion.task_id,
                    callback_id: completion.callback_id,
                    message,
                });
            }
        }
        (polled, completed)
    }

    fn take_failure(&self) -> Option<PortableAsyncFailure> {
        self.state.borrow_mut().failures.pop_front()
    }

    fn has_failure(&self) -> bool {
        !self.state.borrow().failures.is_empty()
    }

    fn has_ready_work(&self) -> bool {
        self.scheduler.has_ready_work()
    }

    fn task_count(&self) -> usize {
        self.state.borrow().tasks.len()
    }

    fn dispatch_host_event(
        &self,
        generation_id: u64,
        event: &AsyncCallbackEventView<'_>,
    ) -> Result<(), PortableAsyncEventError> {
        if event.generation_id() != generation_id {
            return Err(PortableAsyncEventError::GenerationMismatch {
                expected: generation_id,
                actual: event.generation_id(),
            });
        }
        let task_id = PortableTaskId(event.task_id().get());
        let callback_id = StableId128::from_bytes(*event.callback_id().as_bytes());
        let mut state = self.state.borrow_mut();
        let task = state
            .tasks
            .get(&task_id)
            .ok_or(PortableAsyncEventError::UnknownTask { id: task_id })?;
        if task.callback_id != callback_id {
            return Err(PortableAsyncEventError::CallbackMismatch {
                id: task_id,
                expected: task.callback_id,
                actual: callback_id,
            });
        }
        if !task.host_owned {
            return Err(PortableAsyncEventError::NotHostOwned { id: task_id });
        }
        let limit = state
            .limits
            .max_completion_bytes
            .min(task.completion_limit);
        if event.payload().len() > limit {
            return Err(PortableAsyncEventError::CompletionTooLarge {
                length: event.payload().len(),
                limit,
            });
        }
        if event.kind() == AsyncCallbackEventKind::Cancelled && !event.payload().is_empty() {
            return Err(PortableAsyncEventError::CancellationPayloadNotEmpty);
        }
        if let Some(previous) = state.last_event_sequence
            && event.event_sequence() <= previous
        {
            return Err(PortableAsyncEventError::EventSequenceNotMonotonic {
                previous,
                actual: event.event_sequence(),
            });
        }
        state.last_event_sequence = Some(event.event_sequence());
        state.tasks.remove(&task_id);
        if event.kind() == AsyncCallbackEventKind::Failure {
            state.failures.push_back(PortableAsyncFailure {
                task_id,
                callback_id,
                message: String::from_utf8_lossy(event.payload()).into_owned(),
            });
        }
        Ok(())
    }

    fn dispatch_external_event(
        &self,
        generation_id: u64,
        event: &AsyncCallbackEventView<'_>,
        completion_limit: usize,
    ) -> Result<(), PortableAsyncEventError> {
        if event.generation_id() != generation_id {
            return Err(PortableAsyncEventError::GenerationMismatch {
                expected: generation_id,
                actual: event.generation_id(),
            });
        }
        let mut state = self.state.borrow_mut();
        let limit = state.limits.max_completion_bytes.min(completion_limit);
        if event.payload().len() > limit {
            return Err(PortableAsyncEventError::CompletionTooLarge {
                length: event.payload().len(),
                limit,
            });
        }
        if event.kind() == AsyncCallbackEventKind::Cancelled && !event.payload().is_empty() {
            return Err(PortableAsyncEventError::CancellationPayloadNotEmpty);
        }
        if let Some(previous) = state.last_event_sequence
            && event.event_sequence() <= previous
        {
            return Err(PortableAsyncEventError::EventSequenceNotMonotonic {
                previous,
                actual: event.event_sequence(),
            });
        }
        state.last_event_sequence = Some(event.event_sequence());
        if event.kind() == AsyncCallbackEventKind::Failure {
            state.failures.push_back(PortableAsyncFailure {
                task_id: PortableTaskId(event.task_id().get()),
                callback_id: StableId128::from_bytes(*event.callback_id().as_bytes()),
                message: String::from_utf8_lossy(event.payload()).into_owned(),
            });
        }
        Ok(())
    }
}

#[inline]
fn task_capacity(state: &PortableAsyncState, schema: AsyncCallbackSchemaMetadata) -> usize {
    state
        .limits
        .max_in_flight_tasks
        .min(schema.maximum_in_flight_tasks() as usize)
        .min(state.limits.max_retained_resources)
        .min(schema.maximum_retained_resources() as usize)
}

type PortableCallbackBody = Box<dyn Fn() -> Result<(), PortableCallbackFailure>>;
type PortableAsyncCallbackBody = Rc<dyn Fn() -> Pin<Box<dyn Future<Output = ()> + 'static>>>;

enum PortableCallbackRegistration {
    Sync(PortableCallbackBody),
    Async {
        schema: AsyncCallbackSchemaMetadata,
        body: PortableAsyncCallbackBody,
    },
}

/// One owned callback registration staged alongside its Widget IR node.
#[doc(hidden)]
pub struct PortableCallback {
    callback_id: StableId128,
    event_kind: EventId,
    event_schema: Version,
    async_schema: Option<AsyncCallbackSchemaMetadata>,
    body: PortableCallbackRegistration,
}

impl PortableCallback {
    /// Creates a typed callback registration with a stable identity.
    #[inline]
    pub fn new(
        event_kind: EventId,
        event_schema: Version,
        callback_id: StableId128,
        body: impl Fn() -> Result<(), PortableCallbackFailure> + 'static,
    ) -> Self {
        Self {
            callback_id,
            event_kind,
            event_schema,
            async_schema: None,
            body: PortableCallbackRegistration::Sync(Box::new(body)),
        }
    }

    /// Creates an async callback registration without moving its future into
    /// the portable document.
    #[doc(hidden)]
    #[inline]
    pub fn new_async(
        event_kind: EventId,
        event_schema: Version,
        async_schema: AsyncCallbackSchemaMetadata,
        callback_id: StableId128,
        body: impl Fn() -> Pin<Box<dyn Future<Output = ()> + 'static>> + 'static,
    ) -> Self {
        Self {
            callback_id,
            event_kind,
            event_schema,
            async_schema: Some(async_schema),
            body: PortableCallbackRegistration::Async {
                schema: async_schema,
                body: Rc::new(body),
            },
        }
    }
}

/// A precise callback registration or dispatch failure.
#[doc(hidden)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PortableCallbackError {
    /// Two callbacks in one generation derived the same identity.
    Duplicate { id: StableId128 },
    /// The configured callback ceiling was exceeded.
    Capacity { max: usize, actual: usize },
    /// The active registry does not own the requested identity.
    Unknown { id: StableId128 },
    /// The async task ceiling was reached before a future was retained.
    AsyncCapacity { max: usize, actual: usize },
    /// A task identity is not active in this generation.
    UnknownTask { id: PortableTaskId },
    /// A callback returned an error or unwound.
    CallbackFailed { id: StableId128, message: String },
    /// A newer completed document replaced this registry.
    Retired,
}

impl fmt::Display for PortableCallbackError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Duplicate { id } => {
                write!(formatter, "duplicate portable callback ID {id}")
            }
            Self::Capacity { max, actual } => write!(
                formatter,
                "portable callback capacity exceeded: maximum {max}, got {actual}"
            ),
            Self::Unknown { id } => {
                write!(formatter, "unknown portable callback ID {id}")
            }
            Self::AsyncCapacity { max, actual } => write!(
                formatter,
                "portable async task capacity exceeded: maximum {max}, got {actual}"
            ),
            Self::UnknownTask { id } => {
                write!(formatter, "unknown portable async task {}", id.value())
            }
            Self::CallbackFailed { id, message } => {
                write!(formatter, "portable callback {id} failed: {message}")
            }
            Self::Retired => formatter.write_str("portable callback registry is retired"),
        }
    }
}

impl Error for PortableCallbackError {}

/// A malformed, stale, or otherwise unsafe host-owned async completion.
#[doc(hidden)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PortableAsyncEventError {
    /// The completion targets another portable generation.
    GenerationMismatch { expected: u64, actual: u64 },
    /// The completion names no live task in this generation.
    UnknownTask { id: PortableTaskId },
    /// The completion names no active async callback contract.
    UnknownCallback { id: StableId128 },
    /// The completion names the right task but the wrong callback identity.
    CallbackMismatch {
        id: PortableTaskId,
        expected: StableId128,
        actual: StableId128,
    },
    /// A guest-owned task cannot be completed by the host event channel.
    NotHostOwned { id: PortableTaskId },
    /// The completion payload exceeds the async resource ceiling.
    CompletionTooLarge { length: usize, limit: usize },
    /// A cancellation event must not carry a completion payload.
    CancellationPayloadNotEmpty,
    /// The event sequence is duplicated or out of order.
    EventSequenceNotMonotonic { previous: u64, actual: u64 },
}

impl fmt::Display for PortableAsyncEventError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GenerationMismatch { expected, actual } => write!(
                formatter,
                "async event targets generation {actual} instead of {expected}"
            ),
            Self::UnknownTask { id } => write!(formatter, "unknown async task {}", id.value()),
            Self::UnknownCallback { id } => write!(formatter, "unknown async callback {id}"),
            Self::CallbackMismatch {
                id,
                expected,
                actual,
            } => write!(
                formatter,
                "async task {} belongs to callback {expected}, not {actual}",
                id.value()
            ),
            Self::NotHostOwned { id } => {
                write!(formatter, "async task {} is guest-owned", id.value())
            }
            Self::CompletionTooLarge { length, limit } => write!(
                formatter,
                "async completion payload {length} exceeds limit {limit}"
            ),
            Self::CancellationPayloadNotEmpty => {
                formatter.write_str("async cancellation carries a result payload")
            }
            Self::EventSequenceNotMonotonic { previous, actual } => write!(
                formatter,
                "async event sequence {actual} does not follow {previous}"
            ),
        }
    }
}

impl Error for PortableAsyncEventError {}

/// A failure to construct portable Widget IR or update its retained state.
#[doc(hidden)]
#[derive(Debug)]
pub enum PortableBuildError {
    /// A configured resource ceiling would be exceeded.
    LimitExceeded {
        resource: PortableWidgetResource,
        max: usize,
        actual: usize,
    },
    /// A configured length cannot be represented by the version-one format.
    LengthOverflow { resource: PortableWidgetResource, actual: usize },
    /// A widget has no portable lowering implementation.
    UnsupportedWidget {
        widget: &'static str,
        source: SourceFingerprint,
    },
    /// A child does not belong to the active document.
    InvalidChild { child: PortableNodeId, node_count: usize },
    /// One child was supplied more than once to a parent.
    DuplicateChild { child: PortableNodeId },
    /// A child was already attached to another parent.
    ChildAlreadyAttached { child: PortableNodeId },
    /// Two nodes derived the same stable identity.
    DuplicateSlot { slot: StableSlotId },
    /// The declared root does not own every constructed node.
    IncompleteTree,
    /// A property references a missing string or blob table entry.
    InvalidPropertyReference { index: u32 },
    /// A floating-point property is not finite.
    NonFiniteFloat,
    /// A reflected Rust property value has no representation in its declared
    /// semantic AWIR conversion.
    InvalidPropertyValue { rust_type: &'static str },
    /// A property encoding failure annotated with its stable schema field and
    /// generated source location.
    PropertyEncoding {
        property: PropertyId,
        property_name: &'static str,
        source: SourceFingerprint,
        cause: Box<PortableBuildError>,
    },
    /// A derived PortableValue rejected its bounded structural payload.
    ValueCodec {
        rust_type: &'static str,
        message: String,
    },
    /// A callback could not be registered or dispatched.
    Callback(PortableCallbackError),
    /// A host-owned async completion failed closed before changing the tree.
    AsyncEvent(PortableAsyncEventError),
    /// A native-only widget property cannot be represented faithfully.
    UnsupportedProperty {
        widget: &'static str,
        property: &'static str,
        source: SourceFingerprint,
    },
    /// A provider value has no stable guest codec or its codec rejected the
    /// bounded snapshot payload.
    ProviderEncoding {
        provider: &'static str,
        value: &'static str,
        source: SourceFingerprint,
        message: String,
    },
    /// A widget callback cannot cross the selected portable event contract.
    UnsupportedCallback {
        widget: &'static str,
        event_kind: EventId,
        reason: &'static str,
        source: SourceFingerprint,
    },
    /// Typed retained-state processing failed.
    State(StateRegistryError),
    /// Anteros rejected the completed canonical document.
    Model(ModelError),
}

impl fmt::Display for PortableBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LimitExceeded { resource, max, actual } => write!(
                formatter,
                "portable widget {resource:?} limit exceeded: maximum {max}, got {actual}"
            ),
            Self::LengthOverflow { resource, actual } => write!(
                formatter,
                "portable widget {resource:?} length {actual} exceeds the version-one range"
            ),
            Self::UnsupportedWidget { widget, source } => write!(
                formatter,
                "widget `{widget}` at portable source {} has no portable lowering",
                source.0
            ),
            Self::InvalidChild { child, node_count } => write!(
                formatter,
                "portable child {} is outside the current {node_count}-node document",
                child.0
            ),
            Self::DuplicateChild { child } => {
                write!(formatter, "portable child {} occurs twice in one parent", child.0)
            }
            Self::ChildAlreadyAttached { child } => write!(
                formatter,
                "portable child {} already belongs to another parent",
                child.0
            ),
            Self::DuplicateSlot { slot } => {
                write!(formatter, "duplicate portable widget slot {slot}")
            }
            Self::IncompleteTree => formatter.write_str(
                "portable widget root does not own every node in the document",
            ),
            Self::InvalidPropertyReference { index } => write!(
                formatter,
                "portable widget property references missing table index {index}"
            ),
            Self::NonFiniteFloat => {
                formatter.write_str("portable widget floating-point property is not finite")
            }
            Self::InvalidPropertyValue { rust_type } => write!(
                formatter,
                "portable property value of `{rust_type}` has no valid AWIR representation"
            ),
            Self::PropertyEncoding { property, property_name, source, cause } => {
                let property = if property_name.is_empty() {
                    property.to_string()
                } else {
                    (*property_name).to_owned()
                };
                write!(
                    formatter,
                    "portable property {property} at source {} failed: {cause}",
                    source.0,
                )
            }
            Self::ValueCodec { rust_type, message } => write!(
                formatter,
                "portable value `{rust_type}` failed its bounded codec: {message}",
            ),
            Self::Callback(error) => error.fmt(formatter),
            Self::AsyncEvent(error) => error.fmt(formatter),
            Self::UnsupportedProperty { widget, property, .. } => write!(
                formatter,
                "{} property `{property}` has no portable lowering",
                widget.to_ascii_lowercase(),
            ),
            Self::ProviderEncoding { provider, value, message, .. } => write!(
                formatter,
                "provider `{provider}` cannot encode `{value}` for portable context: {message}",
            ),
            Self::UnsupportedCallback { widget, event_kind, reason, .. } => write!(
                formatter,
                "{} event {event_kind} {reason}",
                widget.to_ascii_lowercase(),
            ),
            Self::State(error) => error.fmt(formatter),
            Self::Model(error) => error.fmt(formatter),
        }
    }
}

impl Error for PortableBuildError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Callback(error) => Some(error),
            Self::AsyncEvent(error) => Some(error),
            Self::PropertyEncoding { cause, .. } => Some(cause.as_ref()),
            Self::State(error) => Some(error),
            Self::Model(error) => Some(error),
            _ => None,
        }
    }
}

impl From<PortableCallbackError> for PortableBuildError {
    #[inline]
    fn from(error: PortableCallbackError) -> Self {
        Self::Callback(error)
    }
}

impl From<PortableAsyncEventError> for PortableBuildError {
    #[inline]
    fn from(error: PortableAsyncEventError) -> Self {
        Self::AsyncEvent(error)
    }
}

impl From<StateRegistryError> for PortableBuildError {
    #[inline]
    fn from(error: StateRegistryError) -> Self {
        Self::State(error)
    }
}

impl From<ModelError> for PortableBuildError {
    #[inline]
    fn from(error: ModelError) -> Self {
        Self::Model(error)
    }
}

impl PortableBuildError {
    /// Converts this guest-side lowering failure into a bounded ABI diagnostic.
    #[doc(hidden)]
    pub fn into_guest_diagnostic(self) -> GuestDiagnostic {
        let category = match &self {
            Self::UnsupportedWidget { .. } => GuestDiagnosticCategory::UnsupportedWidget,
            Self::UnsupportedProperty { .. } => GuestDiagnosticCategory::UnsupportedProperty,
            Self::ProviderEncoding { .. } => GuestDiagnosticCategory::PropertyEncoding,
            Self::PropertyEncoding { .. } => GuestDiagnosticCategory::PropertyEncoding,
            Self::ValueCodec { .. } => GuestDiagnosticCategory::PropertyEncoding,
            Self::LimitExceeded { .. } => GuestDiagnosticCategory::LimitExceeded,
            Self::LengthOverflow { .. } => GuestDiagnosticCategory::LengthOverflow,
            Self::InvalidChild { .. } => GuestDiagnosticCategory::InvalidChild,
            Self::DuplicateChild { .. } => GuestDiagnosticCategory::DuplicateChild,
            Self::ChildAlreadyAttached { .. } => GuestDiagnosticCategory::ChildAlreadyAttached,
            Self::DuplicateSlot { .. } => GuestDiagnosticCategory::DuplicateSlot,
            Self::IncompleteTree => GuestDiagnosticCategory::IncompleteTree,
            Self::InvalidPropertyReference { .. } => GuestDiagnosticCategory::InvalidPropertyReference,
            Self::NonFiniteFloat => GuestDiagnosticCategory::NonFiniteFloat,
            Self::InvalidPropertyValue { .. } => GuestDiagnosticCategory::InvalidPropertyValue,
            Self::Callback(_) => GuestDiagnosticCategory::Callback,
            Self::AsyncEvent(_) => GuestDiagnosticCategory::Callback,
            Self::UnsupportedCallback { .. } => GuestDiagnosticCategory::Callback,
            Self::State(_) => GuestDiagnosticCategory::State,
            Self::Model(_) => GuestDiagnosticCategory::Model,
        };
        let message = match &self {
            Self::PropertyEncoding { cause, .. } => format!("property codec error: {cause}"),
            Self::ValueCodec { message, .. } => format!("property codec error: {message}"),
            _ => self.to_string(),
        };
        let mut diagnostic = GuestDiagnostic::new(GuestOperation::Unknown, category, message);
        match self {
            Self::UnsupportedWidget { widget, source } => {
                diagnostic = diagnostic
                    .with_widget(widget)
                    .with_source(anteros_source(source));
            }
            Self::UnsupportedProperty { widget, property, source } => {
                diagnostic = diagnostic
                    .with_widget(widget)
                    .with_property(property)
                    .with_source(anteros_source(source));
            }
            Self::ProviderEncoding { provider, value, source, .. } => {
                diagnostic = diagnostic
                    .with_widget(provider)
                    .with_property(value)
                    .with_source(anteros_source(source));
            }
            Self::PropertyEncoding { property, property_name, source, .. } => {
                let property = if property_name.is_empty() {
                    property.to_string()
                } else {
                    property_name.to_owned()
                };
                diagnostic = diagnostic
                    .with_property(property)
                    .with_source(anteros_source(source));
            }
            Self::ValueCodec { rust_type, .. } => {
                diagnostic = diagnostic.with_property(rust_type);
            }
            Self::UnsupportedCallback { widget, source, .. } => {
                diagnostic = diagnostic
                    .with_widget(widget)
                    .with_source(anteros_source(source));
            }
            Self::LimitExceeded { max, actual, .. } => {
                diagnostic = diagnostic.with_limits(max as u64, actual as u64);
            }
            Self::LengthOverflow { actual, .. } => {
                diagnostic = diagnostic.with_limits(u32::MAX as u64, actual as u64);
            }
            _ => {}
        }
        diagnostic
    }

    #[inline]
    fn with_property_context(
        self,
        property: PropertyId,
        property_name: &'static str,
        source: SourceFingerprint,
    ) -> Self {
        Self::PropertyEncoding {
            property,
            property_name,
            source,
            cause: Box::new(self),
        }
    }
}

fn anteros_source(source: SourceFingerprint) -> aimer_anteros::StableId128 {
    aimer_anteros::StableId128::from_bytes(source.identity().to_bytes())
}

pub(super) struct OwnedNode {
    pub(super) widget_type: WidgetSchemaId,
    pub(super) widget_schema: Version,
    pub(super) key: AnterosId,
    pub(super) properties: Vec<WidgetProperty>,
    pub(super) callbacks: Vec<CallbackBinding>,
    pub(super) children: Vec<u32>,
}

struct PortableCallbackRegistryInner {
    generation_id: u64,
    retired: Cell<bool>,
    callbacks: BTreeMap<StableId128, PortableCallbackRegistration>,
}

/// An immutable callback snapshot belonging to one completed portable generation.
#[doc(hidden)]
#[derive(Clone)]
pub struct PortableCallbackRegistry {
    inner: Rc<PortableCallbackRegistryInner>,
}

impl PortableCallbackRegistry {
    fn empty(generation_id: u64) -> Self {
        Self {
            inner: Rc::new(PortableCallbackRegistryInner {
                generation_id,
                retired: Cell::new(false),
                callbacks: BTreeMap::new(),
            }),
        }
    }

    /// Dispatches the current closure and coalesces all resulting state work.
    pub fn dispatch(
        &self,
        callback_id: StableId128,
        context: &mut PortableBuildContext,
    ) -> Result<(), PortableCallbackError> {
        self.dispatch_start(callback_id, context).map(|_| ())
    }

    /// Starts one callback and returns whether it completed synchronously or
    /// owns a generation-local async task.
    pub fn dispatch_start(
        &self,
        callback_id: StableId128,
        context: &mut PortableBuildContext,
    ) -> Result<PortableCallbackStart, PortableCallbackError> {
        if self.inner.retired.get()
            || self.inner.generation_id != context.generation_id
            || !Rc::ptr_eq(&self.inner, &context.active_callbacks.inner)
        {
            return Err(PortableCallbackError::Retired);
        }
        let callback = self.inner.callbacks.get(&callback_id).ok_or(
            PortableCallbackError::Unknown { id: callback_id },
        )?;
        let start = match callback {
            PortableCallbackRegistration::Sync(body) => {
                match catch_unwind(AssertUnwindSafe(body)) {
                    Ok(Ok(())) => PortableCallbackStart::Completed,
                    Ok(Err(error)) => {
                        return Err(PortableCallbackError::CallbackFailed {
                            id: callback_id,
                            message: error.to_string(),
                        });
                    }
                    Err(_) => {
                        return Err(PortableCallbackError::CallbackFailed {
                            id: callback_id,
                            message: "callback panicked".to_owned(),
                        });
                    }
                }
            }
            PortableCallbackRegistration::Async { schema, body } => {
                let future = match catch_unwind(AssertUnwindSafe(|| body())) {
                    Ok(future) => future,
                    Err(_) => {
                        return Err(PortableCallbackError::CallbackFailed {
                            id: callback_id,
                            message: "async callback panicked while starting".to_owned(),
                        });
                    }
                };
                let task_id = context
                    .start_async_task(callback_id, *schema, future)
                    .map_err(|error| PortableCallbackError::CallbackFailed {
                        id: callback_id,
                        message: error.to_string(),
                    })?;
                PortableCallbackStart::Started { task_id }
            }
        };
        context
            .drain_live_state_mutations()
            .map_err(|error| PortableCallbackError::CallbackFailed {
                id: callback_id,
                message: error.to_string(),
            })?;
        if matches!(start, PortableCallbackStart::Completed) {
            context.queue_rebuild();
        }
        Ok(start)
    }
}

type QueuedMutation = Box<dyn FnOnce(&mut StateRegistry) -> Result<(), StateRegistryError>>;

/// Owns one bounded Widget IR build and the retained state used by generated wrappers.
///
/// Children are constructed before their parent. This ordering makes cycles
/// unrepresentable and allows parent ownership and depth to be checked without
/// a second graph allocation. [`Self::finish_graph`] moves the completed IR into
/// an immutable semantic snapshot while retaining state and queued-work
/// machinery for the next rebuild. [`Self::finish_document`] additionally
/// compiles that snapshot into a binary-encodable document.
#[doc(hidden)]
pub struct PortableBuildContext {
    generation_id: u64,
    document_revision: u64,
    limits: PortableWidgetLimits,
    document_bytes: usize,
    property_count: usize,
    callback_count: usize,
    child_count: usize,
    nodes: Vec<OwnedNode>,
    strings: Vec<String>,
    string_indices: BTreeMap<StableId128, u32>,
    blobs: Vec<Vec<u8>>,
    blob_indices: BTreeMap<StableId128, u32>,
    parented: Vec<bool>,
    depths: Vec<usize>,
    slots: BTreeSet<StableSlotId>,
    building_callbacks: BTreeMap<StableId128, PortableCallbackRegistration>,
    active_callbacks: PortableCallbackRegistry,
    async_runtime: PortableAsyncRuntime,
    state_registry: StateRegistry,
    pub(super) live_states: PortableLiveStateRegistry,
    mutations: VecDeque<QueuedMutation>,
    rebuild_requested: bool,
    frame_requested: bool,
    animation_states: BTreeMap<StableSlotId, Box<dyn Any>>,
    animation_slots: BTreeSet<StableSlotId>,
    inherited_states: Rc<RefCell<crate::components::context::InheritedStates>>,
    portable_window: crate::base::WindowHandle,
}

struct PortableStateScopeGuard {
    states: Rc<RefCell<crate::components::context::InheritedStates>>,
    type_id: TypeId,
    previous: Option<Rc<dyn Any>>,
}

impl Drop for PortableStateScopeGuard {
    fn drop(&mut self) {
        let mut states = self.states.borrow_mut();
        if let Some(previous) = self.previous.take() {
            states.insert(self.type_id, previous);
        } else {
            states.remove(&self.type_id);
        }
    }
}

impl PortableBuildContext {
    /// Creates an empty document and retained-state owner with explicit bounds.
    pub fn new(
        generation_id: u64,
        document_revision: u64,
        limits: PortableWidgetLimits,
        state_limits: PortableLimits,
    ) -> Result<Self, PortableBuildError> {
        check_wire_limit(PortableWidgetResource::Nodes, limits.max_nodes)?;
        check_wire_limit(PortableWidgetResource::Properties, limits.max_properties)?;
        check_wire_limit(PortableWidgetResource::Callbacks, limits.max_callbacks)?;
        check_wire_limit(PortableWidgetResource::Children, limits.max_children)?;
        check_wire_limit(PortableWidgetResource::Keys, limits.max_keys)?;
        check_wire_limit(PortableWidgetResource::StringBytes, limits.max_string_bytes)?;
        check_wire_limit(PortableWidgetResource::BlobBytes, limits.max_blob_bytes)?;
        check_wire_limit(
            PortableWidgetResource::DocumentBytes,
            limits.max_document_bytes,
        )?;
        check_limit(
            PortableWidgetResource::DocumentBytes,
            limits.max_document_bytes,
            DOCUMENT_HEADER_BYTES,
        )?;
        Ok(Self {
            generation_id,
            document_revision,
            limits,
            document_bytes: DOCUMENT_HEADER_BYTES,
            property_count: 0,
            callback_count: 0,
            child_count: 0,
            nodes: Vec::new(),
            strings: Vec::new(),
            string_indices: BTreeMap::new(),
            blobs: Vec::new(),
            blob_indices: BTreeMap::new(),
            parented: Vec::new(),
            depths: Vec::new(),
            slots: BTreeSet::new(),
            building_callbacks: BTreeMap::new(),
            active_callbacks: PortableCallbackRegistry::empty(generation_id),
            async_runtime: PortableAsyncRuntime::new(PortableAsyncLimits::default()),
            state_registry: StateRegistry::new(state_limits),
            live_states: PortableLiveStateRegistry::new(),
            mutations: VecDeque::new(),
            rebuild_requested: false,
            frame_requested: false,
            animation_states: BTreeMap::new(),
            animation_slots: BTreeSet::new(),
            inherited_states: Rc::new(RefCell::new(crate::components::context::InheritedStates::default())),
            portable_window: crate::base::WindowHandle::portable(),
        })
    }

    /// Returns the permanent generation identity attached to this guest
    /// context.
    #[doc(hidden)]
    #[inline]
    pub const fn generation_id(&self) -> u64 {
        self.generation_id
    }

    /// Publishes the logical frame clock before a portable guest build lowers
    /// widgets. Native and ordinary browser builds keep their own clocks.
    #[doc(hidden)]
    #[inline]
    pub fn begin_build(&self) {
        #[cfg(aimer_portable_guest)]
        {
            aimer_utils::set_portable_frame_time(self.document_revision);
            aimer_venus::set_portable_frame_time(self.document_revision);
        }
    }

    /// Runs one guest build transaction and discards its incomplete document
    /// if the build returns an error.
    ///
    /// Portable lowering constructs children before parents, so a failed
    /// callback can leave nodes, callbacks, and state slots partially claimed.
    /// Retaining those claims would make the next safe-point retry fail with a
    /// misleading duplicate-slot error. The completed document, callbacks,
    /// retained state values, and async tasks remain intact; only the failed
    /// document construction is rolled back.
    #[doc(hidden)]
    pub fn with_build_transaction<R, E>(
        &mut self,
        build: impl FnOnce(&mut Self) -> Result<R, E>,
    ) -> Result<R, E> {
        let result = build(self);
        if result.is_err() {
            self.abort_build();
        }
        result
    }

    /// Discards the incomplete document currently being lowered.
    #[doc(hidden)]
    pub fn abort_build(&mut self) {
        self.nodes.clear();
        self.strings.clear();
        self.string_indices.clear();
        self.blobs.clear();
        self.blob_indices.clear();
        self.parented.clear();
        self.depths.clear();
        self.slots.clear();
        self.building_callbacks.clear();
        self.document_bytes = DOCUMENT_HEADER_BYTES;
        self.property_count = 0;
        self.callback_count = 0;
        self.child_count = 0;
        self.animation_slots.clear();
        self.rebuild_requested = false;
        self.frame_requested = false;
        self.live_states.abort_build(&mut self.state_registry);
    }

    /// Publishes the host window metrics used by the next portable build.
    ///
    /// Portable widget code cannot access a native window directly, but
    /// responsive providers such as `MediaQuery` still need the same physical
    /// size and scale factor that native layout sees. The values are retained
    /// by the guest context and shared with every `BuildContext` created while
    /// lowering this generation.
    #[doc(hidden)]
    #[inline]
    pub fn set_window_metrics(&self, width: u32, height: u32, scale_factor: f64) {
        self.portable_window.update_headless_metrics(
            winit::dpi::PhysicalSize::new(width, height),
            scale_factor,
        );
    }

    /// Replaces the generation's async ceilings before the first task starts.
    ///
    /// A context owns its scheduler and task scope, so changing this value is
    /// only valid while the context is still preparing a document. Existing
    /// tasks are deliberately not reconfigured.
    #[inline]
    pub fn with_async_limits(mut self, limits: PortableAsyncLimits) -> Self {
        debug_assert_eq!(self.async_runtime.task_count(), 0);
        self.async_runtime = PortableAsyncRuntime::new(limits);
        self
    }

    /// Creates the `BuildContext` used by one generated guest build while
    /// retaining this portable context's inherited ambient state.
    #[doc(hidden)]
    #[cfg(feature = "portable-guest")]
    #[inline]
    pub fn build_context(&self) -> crate::base::BuildContext<'static> {
        crate::base::BuildContext::portable_with_window(
            self.inherited_states.clone(),
            self.portable_window.clone(),
        )
    }

    /// Temporarily installs one inherited guest value for a nested lowering.
    ///
    /// The previous value is restored even when the child lowering unwinds, so
    /// nested providers shadow deterministically and cannot leak into siblings.
    #[doc(hidden)]
    #[cfg(feature = "portable-guest")]
    pub fn with_state<T: Any, R>(
        &mut self,
        state: T,
        callback: impl FnOnce(&mut Self) -> R,
    ) -> R {
        let type_id = TypeId::of::<T>();
        let previous = self
            .inherited_states
            .borrow_mut()
            .insert(type_id, Rc::new(state));
        let _guard = PortableStateScopeGuard {
            states: self.inherited_states.clone(),
            type_id,
            previous,
        };
        callback(self)
    }

    /// Retains one generation-local animation value under a stable widget
    /// slot while a portable guest is rebuilt for successive frames.
    ///
    /// Animation state is deliberately separate from the serialized retained
    /// state registry: it belongs to the current guest instance and is sampled
    /// again on the next frame rather than migrated across a hot-reload
    /// generation. A type change at an existing slot safely starts that slot
    /// over with `initial`.
    #[doc(hidden)]
    pub fn with_animation_state<T, R>(
        &mut self,
        slot: StableSlotId,
        initial: impl FnOnce() -> T,
        callback: impl FnOnce(&mut T, &mut Self) -> R,
    ) -> R
    where
        T: 'static,
    {
        let mut state = self
            .animation_states
            .remove(&slot)
            .and_then(|state| state.downcast::<T>().ok().map(|state| *state))
            .unwrap_or_else(initial);
        self.animation_slots.insert(slot);
        let result = callback(&mut state, self);
        self.animation_states.insert(slot, Box::new(state));
        result
    }

    /// Requests another portable guest build at the next host frame safe
    /// point.
    #[doc(hidden)]
    #[inline]
    pub fn request_frame(&mut self) {
        self.frame_requested = true;
    }

    /// Takes and clears the pending portable guest frame request.
    #[doc(hidden)]
    #[inline]
    pub fn take_frame_request(&mut self) -> bool {
        std::mem::take(&mut self.frame_requested)
    }

    /// Derives a retained slot from an explicit widget key or source fallback.
    ///
    /// Explicit keys are domain-separated from fallback fingerprints. Key
    /// hashing is deterministic and allocation-free; equality-compatible keys
    /// therefore derive the same slot without converting them to strings.
    pub fn slot_for(
        &self,
        key: Option<&Key>,
        source: SourceFingerprint,
    ) -> StableSlotId {
        match key {
            Some(key) => {
                let mut hasher = StableKeyHasher::new();
                hasher.write(b"aimer.portable.slot.key.v1");
                key.hash(&mut hasher);
                hasher.finish128()
            }
            None => {
                let mut hasher = StableHasher::new();
                hasher.write_str("aimer.portable.slot.source.v1");
                hasher.write_bytes(&source.0.to_bytes());
                hasher.finish()
            }
        }
    }

    /// Derives a callback identity from the widget slot and stable event kind.
    pub fn callback_id_for(
        &self,
        key: Option<&Key>,
        source: SourceFingerprint,
        event_kind: EventId,
    ) -> StableId128 {
        let mut hasher = StableHasher::new();
        hasher.write_str("aimer.portable.callback.v1");
        hasher.write_bytes(&self.slot_for(key, source).to_bytes());
        hasher.write_u64(event_kind.value());
        hasher.finish()
    }

    pub(crate) fn start_async_task(
        &self,
        callback_id: StableId128,
        schema: AsyncCallbackSchemaMetadata,
        future: Pin<Box<dyn Future<Output = ()> + 'static>>,
    ) -> Result<PortableTaskId, PortableCallbackFailure> {
        self.async_runtime.start(callback_id, schema, future)
    }

    /// Registers a host-capability async task without retaining a native
    /// executor handle in the guest document.
    #[doc(hidden)]
    pub fn register_host_async_task(
        &mut self,
        callback_id: StableId128,
        schema: AsyncCallbackSchemaMetadata,
    ) -> Result<PortableTaskId, PortableCallbackFailure> {
        self.async_runtime.register_host_task(callback_id, schema)
    }

    /// Accepts one validated host-owned completion at the serialized build
    /// boundary. Invalid or stale events leave the current tree untouched.
    #[doc(hidden)]
    pub fn dispatch_async_event(
        &mut self,
        event: &AsyncCallbackEventView<'_>,
    ) -> Result<(), PortableAsyncEventError> {
        self.async_runtime
            .dispatch_host_event(self.generation_id, event)?;
        self.queue_rebuild();
        Ok(())
    }

    /// Accepts a completion authenticated by the host generation.
    ///
    /// Host capability tasks intentionally have no guest future to retain.
    /// Generation validation owns their task table; this method applies only
    /// the bounded event and publishes its rebuild request at the next build.
    #[doc(hidden)]
    pub fn dispatch_external_async_event(
        &mut self,
        event: &AsyncCallbackEventView<'_>,
    ) -> Result<(), PortableAsyncEventError> {
        let callback_id = StableId128::from_bytes(*event.callback_id().as_bytes());
        let Some(PortableCallbackRegistration::Async { schema, .. }) =
            self.active_callbacks.inner.callbacks.get(&callback_id)
        else {
            return Err(PortableAsyncEventError::UnknownCallback { id: callback_id });
        };
        self.async_runtime
            .dispatch_external_event(
                self.generation_id,
                event,
                schema.maximum_completion_bytes() as usize,
            )?;
        self.queue_rebuild();
        Ok(())
    }

    /// Polls guest-owned async callback tasks at the portable build boundary.
    ///
    /// Futures and their wakers remain inside this generation's Venus scope.
    /// Only completion state and bounded diagnostics are retained by the
    /// portable context; no executor object or future is encoded into AWIR.
    pub fn run_async_microtasks(&mut self) -> usize {
        let (polled, completed) = self.async_runtime.run_microtasks();
        if completed > 0 || self.async_runtime.has_failure() {
            self.queue_rebuild();
        }
        polled
    }

    /// Returns whether the generation retains a task or has ready scheduler work.
    #[inline]
    pub fn has_async_work(&self) -> bool {
        self.frame_requested
            || self.async_runtime.task_count() != 0
            || self.async_runtime.has_ready_work()
    }

    /// Returns the number of in-flight guest-owned async callback tasks.
    #[inline]
    pub fn async_task_count(&self) -> usize {
        self.async_runtime.task_count()
    }

    /// Cancels one generation-local async callback task.
    #[inline]
    pub fn cancel_async_task(&self, task_id: PortableTaskId) -> bool {
        self.async_runtime.cancel(task_id)
    }

    /// Takes the next structured async failure, if one occurred.
    #[inline]
    pub fn take_async_failure(&self) -> Option<PortableAsyncFailure> {
        self.async_runtime.take_failure()
    }

    /// Moves a string into the document table after checking all byte ceilings.
    ///
    /// Equal strings reuse their first document-local reference so repeated UI
    /// text occupies one range record and one UTF-8 payload in binary AWIR.
    pub fn push_owned_string(
        &mut self,
        value: String,
    ) -> Result<PropertyValue, PortableBuildError> {
        check_limit(
            PortableWidgetResource::StringBytes,
            self.limits.max_string_bytes,
            value.len(),
        )?;
        let mut hasher = StableHasher::new();
        hasher.write_str("aimer.portable.widget-string.v1");
        hasher.write_str(&value);
        let hash = hasher.finish();
        if let Some(&index) = self.string_indices.get(&hash) {
            if self.strings[index as usize] == value {
                return Ok(PropertyValue::StringRef(index));
            }
            if let Some(index) = self.strings.iter().position(|existing| existing == &value) {
                return Ok(PropertyValue::StringRef(index as u32));
            }
        }
        let next_bytes = checked_add(
            self.document_bytes,
            STRING_RANGE_BYTES,
            PortableWidgetResource::DocumentBytes,
        )?;
        let next_bytes = checked_add(
            next_bytes,
            value.len(),
            PortableWidgetResource::DocumentBytes,
        )?;
        check_limit(
            PortableWidgetResource::DocumentBytes,
            self.limits.max_document_bytes,
            next_bytes,
        )?;
        let index = u32::try_from(self.strings.len()).map_err(|_| {
            PortableBuildError::LengthOverflow {
                resource: PortableWidgetResource::StringBytes,
                actual: self.strings.len(),
            }
        })?;
        self.strings.push(value);
        self.string_indices.entry(hash).or_insert(index);
        self.document_bytes = next_bytes;
        Ok(PropertyValue::StringRef(index))
    }

    /// Copies a borrowed string into the document after checking its limits.
    #[inline]
    pub fn push_string(
        &mut self,
        value: &str,
    ) -> Result<PropertyValue, PortableBuildError> {
        check_limit(
            PortableWidgetResource::StringBytes,
            self.limits.max_string_bytes,
            value.len(),
        )?;
        self.push_owned_string(value.to_owned())
    }

    /// Encodes one property and annotates any failure with its stable schema
    /// field and generated source fingerprint.
    #[inline]
    pub fn encode_property<T>(
        &mut self,
        property: PropertyId,
        source: SourceFingerprint,
        value: T,
    ) -> Result<PropertyValue, PortableBuildError>
    where
        T: super::encoder::PortableEncodeProperty,
    {
        value
            .encode_property(self)
            .map_err(|error| error.with_property_context(property, "", source))
    }

    /// Encodes one reflected property while retaining its canonical schema
    /// name for cross-boundary diagnostics.
    #[inline]
    pub fn encode_property_named<T>(
        &mut self,
        property: PropertyId,
        property_name: &'static str,
        source: SourceFingerprint,
        value: T,
    ) -> Result<PropertyValue, PortableBuildError>
    where
        T: super::encoder::PortableEncodeProperty,
    {
        value
            .encode_property(self)
            .map_err(|error| error.with_property_context(property, property_name, source))
    }

    /// Copies a blob into the document table after checking all byte ceilings.
    ///
    /// Equal blobs reuse their first document-local reference. The input is
    /// checked before the owned copy is allocated, so an oversized blob cannot
    /// grow the current document.
    pub fn push_blob(&mut self, value: impl AsRef<[u8]>) -> Result<PropertyValue, PortableBuildError> {
        let value = value.as_ref();
        check_limit(
            PortableWidgetResource::BlobBytes,
            self.limits.max_blob_bytes,
            value.len(),
        )?;
        let hash = blob_hash(value);
        if let Some(index) = self.find_blob(hash, value) {
            return Ok(PropertyValue::BlobRef(index));
        }
        self.push_new_blob(hash, value.to_vec())
    }

    /// Moves an owned blob into the document table after checking its limits.
    ///
    /// This is the allocation-free ownership path for codecs that already
    /// produce a `Vec<u8>`. Equal values are dropped and reuse their first
    /// document-local reference.
    pub fn push_owned_blob(&mut self, value: Vec<u8>) -> Result<PropertyValue, PortableBuildError> {
        check_limit(
            PortableWidgetResource::BlobBytes,
            self.limits.max_blob_bytes,
            value.len(),
        )?;
        let hash = blob_hash(&value);
        if let Some(index) = self.find_blob(hash, &value) {
            return Ok(PropertyValue::BlobRef(index));
        }
        self.push_new_blob(hash, value)
    }

    fn find_blob(&self, hash: StableId128, value: &[u8]) -> Option<u32> {
        let Some(&index) = self.blob_indices.get(&hash) else {
            return None;
        };
        if self.blobs[index as usize] == value {
            return Some(index);
        }
        self.blobs
            .iter()
            .position(|existing| existing.as_slice() == value)
            .map(|index| index as u32)
    }

    fn push_new_blob(
        &mut self,
        hash: StableId128,
        value: Vec<u8>,
    ) -> Result<PropertyValue, PortableBuildError> {
        let next_bytes = checked_add(
            self.document_bytes,
            BLOB_RANGE_BYTES,
            PortableWidgetResource::DocumentBytes,
        )?;
        let next_bytes = checked_add(
            next_bytes,
            value.len(),
            PortableWidgetResource::DocumentBytes,
        )?;
        check_limit(
            PortableWidgetResource::DocumentBytes,
            self.limits.max_document_bytes,
            next_bytes,
        )?;
        let index = u32::try_from(self.blobs.len()).map_err(|_| {
            PortableBuildError::LengthOverflow {
                resource: PortableWidgetResource::BlobBytes,
                actual: self.blobs.len(),
            }
        })?;
        self.blobs.push(value);
        self.blob_indices.entry(hash).or_insert(index);
        self.document_bytes = next_bytes;
        Ok(PropertyValue::BlobRef(index))
    }

    /// Appends a node after validating every count, reference, key, and edge.
    ///
    /// `properties` and `children` are copied only after all limits and graph
    /// invariants have passed. Concrete widget lowering should move owned text
    /// and blobs through [`Self::push_owned_string`] and [`Self::push_owned_blob`]
    /// before calling this method.
    pub fn push_node(
        &mut self,
        widget_type: WidgetSchemaId,
        widget_schema: Version,
        key: Option<&Key>,
        source: SourceFingerprint,
        properties: &[WidgetProperty],
        children: &[PortableNodeId],
    ) -> Result<PortableNodeId, PortableBuildError> {
        self.push_node_with_callbacks(
            widget_type,
            widget_schema,
            key,
            source,
            properties,
            Vec::new(),
            children,
        )
    }

    /// Appends one node and atomically stages all callback bindings it owns.
    pub fn push_node_with_callbacks(
        &mut self,
        widget_type: WidgetSchemaId,
        widget_schema: Version,
        key: Option<&Key>,
        source: SourceFingerprint,
        properties: &[WidgetProperty],
        callbacks: Vec<PortableCallback>,
        children: &[PortableNodeId],
    ) -> Result<PortableNodeId, PortableBuildError> {
        let node_count = self.nodes.len().saturating_add(1);
        check_limit(
            PortableWidgetResource::Nodes,
            self.limits.max_nodes,
            node_count,
        )?;
        let property_count = self.property_count.saturating_add(properties.len());
        check_limit(
            PortableWidgetResource::Properties,
            self.limits.max_properties,
            property_count,
        )?;
        let mut callback_ids = BTreeSet::new();
        for callback in &callbacks {
            if self.building_callbacks.contains_key(&callback.callback_id)
                || !callback_ids.insert(callback.callback_id)
            {
                return Err(PortableCallbackError::Duplicate { id: callback.callback_id }
                .into());
            }
        }
        let callback_count = self.callback_count.saturating_add(callbacks.len());
        if callback_count > self.limits.max_callbacks {
            return Err(PortableCallbackError::Capacity {
                max: self.limits.max_callbacks,
                actual: callback_count,
            }
            .into());
        }
        let child_count = self.child_count.saturating_add(children.len());
        check_limit(
            PortableWidgetResource::Children,
            self.limits.max_children,
            child_count,
        )?;
        let key_count = self.slots.len().saturating_add(1);
        check_limit(
            PortableWidgetResource::Keys,
            self.limits.max_keys,
            key_count,
        )?;
        let property_bytes = properties.len().checked_mul(PROPERTY_BYTES).ok_or(
            PortableBuildError::LengthOverflow {
                resource: PortableWidgetResource::DocumentBytes,
                actual: usize::MAX,
            },
        )?;
        let callback_bytes = callbacks.len().checked_mul(CALLBACK_BYTES).ok_or(
            PortableBuildError::LengthOverflow {
                resource: PortableWidgetResource::DocumentBytes,
                actual: usize::MAX,
            },
        )?;
        let child_bytes = children.len().checked_mul(CHILD_BYTES).ok_or(
            PortableBuildError::LengthOverflow {
                resource: PortableWidgetResource::DocumentBytes,
                actual: usize::MAX,
            },
        )?;
        let node_bytes = NODE_BYTES
            .checked_add(property_bytes)
            .and_then(|bytes| bytes.checked_add(callback_bytes))
            .and_then(|bytes| bytes.checked_add(child_bytes))
            .ok_or(PortableBuildError::LengthOverflow {
                resource: PortableWidgetResource::DocumentBytes,
                actual: usize::MAX,
            })?;
        let document_bytes = checked_add(
            self.document_bytes,
            node_bytes,
            PortableWidgetResource::DocumentBytes,
        )?;
        check_limit(
            PortableWidgetResource::DocumentBytes,
            self.limits.max_document_bytes,
            document_bytes,
        )?;

        for property in properties {
            match property.value() {
                PropertyValue::StringRef(index) if index as usize >= self.strings.len() => {
                    return Err(PortableBuildError::InvalidPropertyReference { index });
                }
                PropertyValue::BlobRef(index) if index as usize >= self.blobs.len() => {
                    return Err(PortableBuildError::InvalidPropertyReference { index });
                }
                PropertyValue::F64(value) if !value.is_finite() => {
                    return Err(PortableBuildError::NonFiniteFloat);
                }
                _ => {}
            }
        }

        let mut depth = 1_usize;
        for (position, child) in children.iter().copied().enumerate() {
            let index = child.0 as usize;
            if index >= self.nodes.len() {
                return Err(PortableBuildError::InvalidChild {
                    child,
                    node_count: self.nodes.len(),
                });
            }
            if children[..position].contains(&child) {
                return Err(PortableBuildError::DuplicateChild { child });
            }
            if self.parented[index] {
                return Err(PortableBuildError::ChildAlreadyAttached { child });
            }
            depth = depth.max(self.depths[index].saturating_add(1));
        }
        check_limit(
            PortableWidgetResource::Nodes,
            self.limits.max_nodes,
            depth,
        )?;

        let slot = self.slot_for(key, source);
        if self.slots.contains(&slot) {
            return Err(PortableBuildError::DuplicateSlot { slot });
        }

        let node_id = PortableNodeId(u32::try_from(self.nodes.len()).map_err(|_| {
            PortableBuildError::LengthOverflow {
                resource: PortableWidgetResource::Nodes,
                actual: self.nodes.len(),
            }
        })?);
        let owned_children = children.iter().map(|child| child.0).collect();
        let owned_properties = properties.to_vec();
        let owned_callbacks = callbacks
            .iter()
            .map(|callback| match callback.async_schema {
                Some(schema) => CallbackBinding::new_async(
                    callback.event_kind,
                    callback.event_schema,
                    schema.contract_version(),
                    AnterosId::from_bytes(callback.callback_id.to_bytes()),
                ),
                None => CallbackBinding::new(
                    callback.event_kind,
                    callback.event_schema,
                    AnterosId::from_bytes(callback.callback_id.to_bytes()),
                ),
            })
            .collect();
        for callback in callbacks {
            self.building_callbacks.insert(callback.callback_id, callback.body);
        }
        self.nodes.push(OwnedNode {
            widget_type,
            widget_schema,
            key: AnterosId::from_bytes(slot.to_bytes()),
            properties: owned_properties,
            callbacks: owned_callbacks,
            children: owned_children,
        });
        self.parented.push(false);
        self.depths.push(depth);
        for child in children {
            self.parented[child.0 as usize] = true;
        }
        self.slots.insert(slot);
        self.property_count = property_count;
        self.callback_count = callback_count;
        self.child_count = child_count;
        self.document_bytes = document_bytes;
        Ok(node_id)
    }

    /// Returns an error suitable for the default [`Widget`](crate::Widget) lowering.
    #[inline]
    pub fn unsupported_widget(
        &self,
        widget: &'static str,
        source: SourceFingerprint,
    ) -> PortableBuildError {
        PortableBuildError::UnsupportedWidget { widget, source }
    }

    /// Completes the current tree and moves it into an immutable semantic graph.
    pub fn finish_graph(
        &mut self,
        root: PortableNodeId,
    ) -> Result<PortableSemanticGraph, PortableBuildError> {
        let root_index = root.0 as usize;
        if root_index >= self.nodes.len() {
            return Err(PortableBuildError::InvalidChild {
                child: root,
                node_count: self.nodes.len(),
            });
        }
        if self.parented[root_index]
            || self
                .parented
                .iter()
                .enumerate()
                .any(|(index, parented)| index != root_index && !parented)
        {
            return Err(PortableBuildError::IncompleteTree);
        }
        let graph = PortableSemanticGraph::new(
            self.generation_id,
            self.document_revision,
            root,
            self.limits,
            std::mem::take(&mut self.nodes),
            std::mem::take(&mut self.strings),
            std::mem::take(&mut self.blobs),
        );
        self.active_callbacks.inner.retired.set(true);
        self.active_callbacks = PortableCallbackRegistry {
            inner: Rc::new(PortableCallbackRegistryInner {
                generation_id: self.generation_id,
                retired: Cell::new(false),
                callbacks: std::mem::take(&mut self.building_callbacks),
            }),
        };
        self.document_revision = self.document_revision.wrapping_add(1);
        self.document_bytes = DOCUMENT_HEADER_BYTES;
        self.property_count = 0;
        self.callback_count = 0;
        self.child_count = 0;
        self.string_indices.clear();
        self.blob_indices.clear();
        self.parented.clear();
        self.depths.clear();
        self.slots.clear();
        self.animation_states
            .retain(|slot, _| self.animation_slots.contains(slot));
        self.animation_slots.clear();
        self.live_states.finish_generation();
        Ok(graph)
    }

    /// Completes the semantic graph and compiles it into an AWIR document.
    #[inline]
    pub fn finish_document(
        &mut self,
        root: PortableNodeId,
    ) -> Result<PortableWidgetDocument, PortableBuildError> {
        self.finish_graph(root).map(PortableSemanticGraph::compile)
    }

    /// Returns a handle to the callback registry for the latest completed document.
    #[inline]
    pub fn callback_registry(&self) -> PortableCallbackRegistry {
        self.active_callbacks.clone()
    }

    /// Returns a handle to the latest completed generation's callback registry.
    #[inline]
    pub fn take_callback_registry(&self) -> PortableCallbackRegistry {
        self.callback_registry()
    }

    fn drain_live_state_mutations(&mut self) -> Result<(), StateRegistryError> {
        self.live_states.drain_all(&mut self.state_registry)
    }

    /// Borrows the typed retained-state registry.
    #[inline]
    pub const fn state_registry(&self) -> &StateRegistry {
        &self.state_registry
    }

    /// Mutably borrows the typed retained-state registry.
    #[inline]
    pub fn state_registry_mut(&mut self) -> &mut StateRegistry {
        &mut self.state_registry
    }

    /// Defers one typed mutation until the portable rebuild boundary.
    pub fn queue_state_mutation<T, F>(
        &mut self,
        slot: StableSlotId,
        mutation: F,
    ) -> Result<(), PortableBuildError>
    where
        T: AimerReflectionType + PortableDecode + PortableEncode + 'static,
        F: FnOnce(&mut T) + 'static,
    {
        let actual = self.mutations.len().saturating_add(1);
        check_limit(
            PortableWidgetResource::Mutations,
            self.limits.max_nodes,
            actual,
        )?;
        self.mutations.push_back(Box::new(move |registry| {
            registry.mutate::<T, _>(slot, mutation)
        }));
        self.rebuild_requested = true;
        Ok(())
    }

    /// Requests a rebuild without changing retained state.
    #[inline]
    pub fn queue_rebuild(&mut self) {
        self.rebuild_requested = true;
    }

    /// Applies deferred mutations in first-in, first-out order.
    pub fn apply_queued_mutations(&mut self) -> Result<(), PortableBuildError> {
        // Async callbacks queue directly on their retained state handle. Drain
        // those mutations at the same serialized rebuild boundary as explicit
        // context mutations so a completed future cannot rebuild stale state.
        self.drain_live_state_mutations()?;
        while let Some(mutation) = self.mutations.pop_front() {
            mutation(&mut self.state_registry)?;
        }
        Ok(())
    }

    /// Returns whether a mutation or explicit request requires rebuilding.
    #[inline]
    pub const fn rebuild_requested(&self) -> bool {
        self.rebuild_requested
    }

    /// Takes and clears the coalesced rebuild request.
    #[inline]
    pub fn take_rebuild_request(&mut self) -> bool {
        std::mem::take(&mut self.rebuild_requested)
    }
}

/// An immutable owned Widget IR snapshot.
///
/// Anteros deliberately models documents as borrowed tables. This owner avoids
/// self-references: [`Self::with_document`] creates the short-lived borrowed
/// node and string tables only for the duration of a callback.
#[doc(hidden)]
pub struct PortableWidgetDocument {
    graph: PortableSemanticGraph,
}

impl PortableWidgetDocument {
    #[inline]
    pub(super) const fn from_graph(graph: PortableSemanticGraph) -> Self { Self { graph } }
    /// Calls `callback` with a valid borrowed Anteros document.
    pub fn with_document<R>(
        &self,
        callback: impl FnOnce(&WidgetDocument<'_>) -> R,
    ) -> R {
        self.graph.with_document(callback)
    }

    /// Encodes a compact snapshot using the limits enforced during construction.
    ///
    /// Equal payloads are interned before AWIR serialization. The semantic
    /// builder already interns strings eagerly; compact encoding additionally
    /// covers payloads supplied by future portable value codecs.
    pub fn encode(&self) -> Result<Vec<u8>, PortableBuildError> {
        self.with_document(|document| document.encode_compact(self.model_limits()))
            .map_err(PortableBuildError::Model)
    }

    /// Returns Anteros limits equivalent to this builder's byte constraints.
    #[inline]
    pub fn model_limits(&self) -> ModelLimits {
        self.graph.model_limits()
    }
}

fn check_wire_limit(
    resource: PortableWidgetResource,
    actual: usize,
) -> Result<(), PortableBuildError> {
    if u32::try_from(actual).is_err() {
        Err(PortableBuildError::LengthOverflow { resource, actual })
    } else {
        Ok(())
    }
}

fn check_limit(
    resource: PortableWidgetResource,
    max: usize,
    actual: usize,
) -> Result<(), PortableBuildError> {
    if actual > max {
        Err(PortableBuildError::LimitExceeded {
            resource,
            max,
            actual,
        })
    } else {
        Ok(())
    }
}

fn checked_add(
    left: usize,
    right: usize,
    resource: PortableWidgetResource,
) -> Result<usize, PortableBuildError> {
    left.checked_add(right)
        .ok_or(PortableBuildError::LengthOverflow {
            resource,
            actual: usize::MAX,
    })
}

fn blob_hash(value: &[u8]) -> StableId128 {
    let mut hasher = StableHasher::new();
    hasher.write_str("aimer.portable.widget-blob.v1");
    hasher.write_bytes(value);
    hasher.finish()
}

struct StableKeyHasher {
    state: u128,
}

impl StableKeyHasher {
    #[inline]
    const fn new() -> Self {
        Self {
            state: 0x6c62_272e_07bb_0142_62b8_2175_6295_c58d,
        }
    }

    #[inline]
    fn finish128(self) -> StableId128 {
        StableId128::from_u128(self.state)
    }
}

impl Hasher for StableKeyHasher {
    #[inline]
    fn finish(&self) -> u64 {
        self.state as u64
    }

    fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.state ^= byte as u128;
            self.state = self
                .state
                .wrapping_mul(0x0000_0000_0100_0000_0000_0000_0000_013b);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use aimer_anteros::{
        AsyncCallbackEvent, AsyncCallbackEventView, AsyncCallbackSchemaMetadata, AsyncTaskId,
        CallbackBinding, EVENT_BUTTON_PRESS, EventId, PROPERTY_TEXT_CONTENT, PropertyValue,
        PropertyId, StableId128 as AnterosId, Version, WIDGET_BUTTON, WIDGET_COLUMN, WIDGET_TEXT,
        WidgetDocumentView, WidgetAssemblyDocument, WidgetProperty,
    };

    use super::*;
    use crate::portable::{
        AimerReflectionType, DecodeError, Decoder, EncodeError, Encoder, FieldDescriptor,
        FieldKind, PortableApply, PortableDecode, PortableEncode, PortableLimits, StableId128,
        TypeSchema,
    };
    use crate::{AnyElement, Key, PortableWidget, Widget};

    const STATE_FIELDS: &[FieldDescriptor] = &[
        FieldDescriptor::new("value", "u32", FieldKind::Retained),
    ];
    const STATE_SCHEMA: TypeSchema = TypeSchema::new(
        "CounterState",
        StableId128::from_path("type", "tests::CounterState"),
        STATE_FIELDS,
    );

    #[derive(Debug, PartialEq)]
    struct CounterState(u32);

    impl AimerReflectionType for CounterState {
        const TYPE_ID: StableId128 = StableId128::from_path("type", "tests::CounterState");

        fn schema() -> &'static TypeSchema {
            &STATE_SCHEMA
        }
    }

    impl PortableEncode for CounterState {
        fn encode(&self, encoder: &mut Encoder<'_>) -> Result<(), EncodeError> {
            encoder.nested(|encoder| {
                encoder.field(&STATE_FIELDS[0], |encoder| self.0.encode(encoder))
            })
        }
    }

    impl PortableDecode for CounterState {
        fn decode(decoder: &mut Decoder<'_>) -> Result<Self, DecodeError> {
            decoder.nested(|decoder| {
                Ok(Self(decoder.field(&STATE_FIELDS[0])?.unwrap()))
            })
        }
    }

    impl PortableApply for CounterState {
        type Retained = u32;

        fn decode_retained(decoder: &mut Decoder<'_>) -> Result<Self::Retained, DecodeError> {
            decoder.nested(|decoder| Ok(decoder.field(&STATE_FIELDS[0])?.unwrap()))
        }

        fn apply_retained(&mut self, retained: Self::Retained) {
            self.0 = retained;
        }
    }

    struct PortableText(String);

    impl Widget for PortableText {
        fn to_element(self, _ctx: &crate::base::BuildContext) -> AnyElement {
            panic!("portable conversion must not enter native element construction")
        }
    }

    impl PortableWidget for PortableText {
        #[cfg(feature = "portable-guest")]
        fn to_portable_node(
            self,
            ctx: &mut PortableBuildContext,
            source: SourceFingerprint,
        ) -> Result<PortableNodeId, PortableBuildError> {
            let content = ctx.push_owned_string(self.0)?;
            ctx.push_node(
                WIDGET_TEXT,
                Version::new(1, 0),
                None,
                source,
                &[WidgetProperty::new(PROPERTY_TEXT_CONTENT, content)],
                &[],
            )
        }
    }

    struct NativeOnly;

    impl Widget for NativeOnly {
        fn to_element(self, _ctx: &crate::base::BuildContext) -> AnyElement {
            panic!("native construction is not part of this test")
        }

        fn debug_name(&self) -> &'static str {
            "NativeOnly"
        }
    }

    impl PortableWidget for NativeOnly {}

    struct PortableCapabilityProbe;

    impl PortableWidget for PortableCapabilityProbe {
        fn to_portable_node(
            self,
            ctx: &mut PortableBuildContext,
            source: SourceFingerprint,
        ) -> Result<PortableNodeId, PortableBuildError> {
            ctx.push_node(
                WIDGET_TEXT,
                Version::new(1, 0),
                None,
                source,
                &[],
                &[],
            )
        }
    }

    impl Widget for PortableCapabilityProbe {
        fn to_element(self, _ctx: &crate::base::BuildContext) -> AnyElement {
            panic!("portable capability probe must not build natively")
        }
    }

    fn source(value: u8) -> SourceFingerprint {
        SourceFingerprint::new(StableId128::from_bytes([value; 16]))
    }

    fn state_limits() -> PortableLimits {
        PortableLimits::new(8, 16, 64, 128, 1_024)
    }

    fn limits() -> PortableWidgetLimits {
        PortableWidgetLimits::new(8, 8, 8, 8, 64, 2_048)
    }

    fn context() -> PortableBuildContext {
        PortableBuildContext::new(7, 11, limits(), state_limits()).unwrap()
    }

    #[test]
    fn host_async_completion_is_bounded_and_rejected_after_consumption() {
        let mut context = context();
        let callback_id = StableId128::from_bytes([0x91; 16]);
        let schema = AsyncCallbackSchemaMetadata::new(Version::new(1, 0), 2, 8);
        let task_id = context
            .register_host_async_task(callback_id, schema)
            .unwrap();
        let event = AsyncCallbackEvent::complete(
            7,
            1,
            AnterosId::from_bytes(callback_id.to_bytes()),
            AsyncTaskId::new(task_id.value()),
            &[1, 2],
        )
        .encode(aimer_anteros::ModelLimits::new(512, 16, 64, 64))
        .unwrap();
        let event = AsyncCallbackEventView::decode(
            &event,
            aimer_anteros::ModelLimits::new(512, 16, 64, 64),
        )
        .unwrap();

        context.dispatch_async_event(&event).unwrap();
        assert!(context.rebuild_requested());
        assert!(matches!(
            context.dispatch_async_event(&event),
            Err(PortableAsyncEventError::UnknownTask { .. })
        ));
    }

    #[test]
    fn host_async_failure_retains_stable_task_and_callback_diagnostics() {
        let mut context = context();
        let callback_id = StableId128::from_bytes([0x95; 16]);
        let task_id = context
            .register_host_async_task(
                callback_id,
                AsyncCallbackSchemaMetadata::new(Version::new(1, 0), 2, 8),
            )
            .unwrap();
        let event = AsyncCallbackEvent::failure(
            7,
            2,
            AnterosId::from_bytes(callback_id.to_bytes()),
            AsyncTaskId::new(task_id.value()),
            b"denied",
        )
        .encode(aimer_anteros::ModelLimits::new(512, 16, 64, 64))
        .unwrap();
        let event = AsyncCallbackEventView::decode(
            &event,
            aimer_anteros::ModelLimits::new(512, 16, 64, 64),
        )
        .unwrap();

        context.dispatch_async_event(&event).unwrap();
        let failure = context.take_async_failure().unwrap();
        assert_eq!(failure.task_id(), task_id);
        assert_eq!(failure.callback_id(), callback_id);
        assert_eq!(failure.message(), "denied");
    }

    #[test]
    fn async_callback_fuel_exhaustion_becomes_a_structured_failure() {
        let mut context = context().with_async_limits(PortableAsyncLimits::new(4, 8, 0, 4));
        let callback_id = StableId128::from_bytes([0x92; 16]);
        context
            .start_async_task(
                callback_id,
                AsyncCallbackSchemaMetadata::new(Version::new(1, 0), 4, 8),
                Box::pin(std::future::pending::<()>()),
            )
            .unwrap();

        context.run_async_microtasks();

        let failure = context.take_async_failure().expect("fuel failure");
        assert_eq!(failure.callback_id(), callback_id);
        assert_eq!(failure.message(), "async callback fuel exhausted");
        assert_eq!(context.async_task_count(), 0);
    }

    #[test]
    fn async_callback_retained_resource_limit_rejects_new_tasks() {
        let context = context().with_async_limits(PortableAsyncLimits::new(4, 8, u32::MAX, 0));
        let callback_id = StableId128::from_bytes([0x93; 16]);

        let error = context
            .start_async_task(
                callback_id,
                AsyncCallbackSchemaMetadata::new(Version::new(1, 0), 4, 8),
                Box::pin(std::future::pending::<()>()),
            )
            .unwrap_err();

        assert_eq!(error.to_string(), "async retained resource capacity exceeded: maximum 0");
    }

    #[test]
    fn pending_guest_task_keeps_the_safe_point_awake_until_completion() {
        let mut context = context();
        context
            .start_async_task(
                StableId128::from_bytes([0x94; 16]),
                AsyncCallbackSchemaMetadata::new(Version::new(1, 0), 4, 8),
                Box::pin(std::future::pending::<()>()),
            )
            .unwrap();

        context.run_async_microtasks();

        assert_eq!(context.async_task_count(), 1);
        assert!(context.has_async_work());
    }

    #[cfg(feature = "portable-guest")]
    #[test]
    fn portable_build_context_shares_and_restores_nested_ambient_state() {
        let mut context = context();
        context.with_state(String::from("outer"), |context| {
            let build = context.build_context();
            assert_eq!(build.get_state::<String>().as_deref().map(String::as_str), Some("outer"));

            context.with_state(String::from("inner"), |context| {
                let build = context.build_context();
                assert_eq!(build.get_state::<String>().as_deref().map(String::as_str), Some("inner"));
            });

            assert_eq!(
                context
                    .build_context()
                    .get_state::<String>()
                    .as_deref()
                    .map(String::as_str),
                Some("outer")
            );
        });

        assert!(context.build_context().get_state::<String>().is_none());
    }

    #[cfg(feature = "portable-guest")]
    #[test]
    fn portable_build_context_exposes_host_window_metrics() {
        let context = context();
        context.set_window_metrics(1_200, 800, 2.0);

        let build = context.build_context();
        let metrics = crate::WindowMetrics::of(&build.window);

        assert_eq!(metrics.physical_size, winit::dpi::PhysicalSize::new(1_200, 800));
        assert_eq!(metrics.scale_factor, 2.0);
        assert_eq!(metrics.logical_size().width, 600.0);
        assert_eq!(metrics.logical_size().height, 400.0);
    }

    fn callback(
        context: &PortableBuildContext,
        source: SourceFingerprint,
        event_kind: EventId,
        calls: Rc<Cell<usize>>,
    ) -> PortableCallback {
        let id = context.callback_id_for(None, source, event_kind);
        PortableCallback::new(event_kind, Version::new(1, 0), id, move || {
            calls.set(calls.get() + 1);
            Ok(())
        })
    }

    #[test]
    fn source_child_derivation_is_stable_distinct_and_const() {
        const PARENT: SourceFingerprint =
            SourceFingerprint::new(StableId128::from_path("source", "tests::parent"));
        const FIRST: SourceFingerprint = PARENT.child(7);
        const SAME: SourceFingerprint = PARENT.child(7);
        const OTHER: SourceFingerprint = PARENT.child(8);

        assert_eq!(FIRST, SAME);
        assert_ne!(FIRST, OTHER);
        assert_ne!(FIRST, PARENT);
    }

    #[test]
    fn builds_exact_simple_document() {
        let mut ctx = context();
        let text = PortableText("hello".into())
            .to_portable_node(&mut ctx, source(1))
            .unwrap();
        let root = ctx
            .push_node(
                WIDGET_COLUMN,
                Version::new(1, 0),
                None,
                source(2),
                &[],
                &[text],
            )
            .unwrap();
        let document = ctx.finish_document(root).unwrap();
        let bytes = document.encode().unwrap();
        let view = WidgetDocumentView::decode(&bytes, document.model_limits()).unwrap();

        assert_eq!(view.generation_id(), 7);
        assert_eq!(view.document_revision(), 11);
        assert_eq!(view.root_node(), 1);
        assert_eq!(view.node_count(), 2);
        assert_eq!(view.string(0), Some("hello"));
        let leaf = view.node(0).unwrap();
        assert_eq!(leaf.widget_type(), WIDGET_TEXT);
        assert_eq!(
            leaf.properties().collect::<Vec<_>>(),
            vec![WidgetProperty::new(
                PROPERTY_TEXT_CONTENT,
                PropertyValue::StringRef(0),
            )]
        );
        assert_eq!(view.node(1).unwrap().children().collect::<Vec<_>>(), vec![0]);
    }

    #[test]
    fn completes_an_inspectable_semantic_graph_before_binary_awir() {
        let mut ctx = context();
        let content = ctx.push_owned_string("Hello".to_owned()).unwrap();
        let text = ctx
            .push_node(
                WIDGET_TEXT,
                Version::new(1, 0),
                None,
                source(1),
                &[WidgetProperty::new(PROPERTY_TEXT_CONTENT, content)],
                &[],
            )
            .unwrap();
        let root = ctx
            .push_node(
                WIDGET_COLUMN,
                Version::new(1, 0),
                None,
                source(2),
                &[],
                &[text],
            )
            .unwrap();

        let graph = ctx.finish_graph(root).unwrap();

        assert_eq!(graph.generation_id(), 7);
        assert_eq!(graph.document_revision(), 11);
        assert_eq!(graph.root(), root);
        assert_eq!(graph.node_count(), 2);
        assert_eq!(graph.string_count(), 1);
        assert_eq!(graph.string(0), Some("Hello"));
        let text_node = graph.node(text).unwrap();
        assert_eq!(text_node.widget_type(), WIDGET_TEXT);
        assert_eq!(text_node.widget_schema(), Version::new(1, 0));
        assert_eq!(
            text_node.properties(),
            &[WidgetProperty::new(
                PROPERTY_TEXT_CONTENT,
                PropertyValue::StringRef(0),
            )],
        );
        assert_eq!(text_node.children().collect::<Vec<_>>(), Vec::new());
        assert_eq!(
            graph.node(root).unwrap().children().collect::<Vec<_>>(),
            vec![text],
        );

        let document = graph.compile();
        let model_limits = document.model_limits();
        let encoded = document.encode().unwrap();
        let view = WidgetDocumentView::decode(&encoded, model_limits).unwrap();
        assert_eq!(view.root_node(), root.index());
        assert_eq!(view.string(0), Some("Hello"));
        assert_eq!(view.node(root.index()).unwrap().children().collect::<Vec<_>>(), vec![text.index()]);
    }

    #[test]
    fn repeated_strings_reuse_the_first_document_reference() {
        let mut ctx = context();
        let first = ctx.push_owned_string("Repeated".to_owned()).unwrap();
        let second = ctx.push_owned_string("Repeated".to_owned()).unwrap();

        assert_eq!(second, first);

        let root = ctx
            .push_node(
                WIDGET_TEXT,
                Version::new(1, 0),
                None,
                source(1),
                &[WidgetProperty::new(PROPERTY_TEXT_CONTENT, second)],
                &[],
            )
            .unwrap();
        let graph = ctx.finish_graph(root).unwrap();

        assert_eq!(graph.string_count(), 1);
        assert_eq!(graph.string(0), Some("Repeated"));
    }

    #[test]
    fn blobs_are_interned_and_resolved_by_the_completed_document() {
        let mut ctx = PortableBuildContext::new(
            7,
            11,
            limits().with_max_blob_bytes(8),
            state_limits(),
        )
        .unwrap();
        let first = ctx.push_blob(vec![1, 2, 3, 4]).unwrap();
        let second = ctx.push_blob(&[1, 2, 3, 4]).unwrap();
        assert_eq!(first, second);

        let root = ctx
            .push_node(
                WIDGET_TEXT,
                Version::new(1, 0),
                None,
                source(1),
                &[WidgetProperty::new(PropertyId::new(1), first)],
                &[],
            )
            .unwrap();
        let graph = ctx.finish_graph(root).unwrap();

        assert_eq!(graph.blob_count(), 1);
        assert_eq!(graph.blob(0), Some([1, 2, 3, 4].as_slice()));

        let document = graph.compile();
        let bytes = document.encode().unwrap();
        let view = WidgetDocumentView::decode(&bytes, document.model_limits()).unwrap();
        assert_eq!(view.blob_count(), 1);
        assert_eq!(view.blob(0), Some([1, 2, 3, 4].as_slice()));
        assert_eq!(
            view.node(0).unwrap().properties().collect::<Vec<_>>(),
            vec![WidgetProperty::new(PropertyId::new(1), PropertyValue::BlobRef(0))]
        );
    }

    #[test]
    fn blob_references_and_blob_budgets_are_checked_before_node_commit() {
        let mut forged = PortableBuildContext::new(
            7,
            11,
            limits().with_max_blob_bytes(8),
            state_limits(),
        )
        .unwrap();
        let error = forged
            .push_node(
                WIDGET_TEXT,
                Version::new(1, 0),
                None,
                source(1),
                &[WidgetProperty::new(PropertyId::new(1), PropertyValue::BlobRef(0))],
                &[],
            )
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "portable widget property references missing table index 0"
        );
        let node = forged
            .push_node(WIDGET_TEXT, Version::new(1, 0), None, source(2), &[], &[])
            .unwrap();
        assert_eq!(node.index(), 0);

        let mut per_blob = PortableBuildContext::new(
            7,
            11,
            limits().with_max_blob_bytes(3),
            state_limits(),
        )
        .unwrap();
        assert_limit(
            per_blob.push_blob(vec![1, 2, 3, 4]),
            PortableWidgetResource::BlobBytes,
        );

        let mut document = PortableBuildContext::new(
            7,
            11,
            limits()
                .with_max_blob_bytes(4)
                .with_max_document_bytes(DOCUMENT_HEADER_BYTES + STRING_RANGE_BYTES - 1),
            state_limits(),
        )
        .unwrap();
        assert_limit(
            document.push_blob(vec![1]),
            PortableWidgetResource::DocumentBytes,
        );
    }

    #[test]
    fn completed_graph_resets_string_interning_for_the_next_rebuild() {
        let mut ctx = context();
        for (revision, value) in ["First", "Second"].into_iter().enumerate() {
            let content = ctx.push_owned_string(value.to_owned()).unwrap();
            assert_eq!(content, PropertyValue::StringRef(0));
            let root = ctx
                .push_node(
                    WIDGET_TEXT,
                    Version::new(1, 0),
                    None,
                    source(revision as u8 + 1),
                    &[WidgetProperty::new(PROPERTY_TEXT_CONTENT, content)],
                    &[],
                )
                .unwrap();
            let graph = ctx.finish_graph(root).unwrap();
            assert_eq!(graph.string_count(), 1);
            assert_eq!(graph.string(0), Some(value));
        }
    }

    #[test]
    fn completed_graph_resets_blob_storage_for_the_next_rebuild() {
        let mut ctx = PortableBuildContext::new(
            7,
            11,
            limits().with_max_blob_bytes(8),
            state_limits(),
        )
        .unwrap();
        for (revision, value) in [[1_u8, 2], [3, 4]].into_iter().enumerate() {
            let blob = ctx.push_blob(value).unwrap();
            assert_eq!(blob, PropertyValue::BlobRef(0));
            let root = ctx
                .push_node(
                    WIDGET_TEXT,
                    Version::new(1, 0),
                    None,
                    source(revision as u8 + 1),
                    &[WidgetProperty::new(PropertyId::new(1), blob)],
                    &[],
                )
                .unwrap();
            let graph = ctx.finish_graph(root).unwrap();
            assert_eq!(graph.blob_count(), 1);
            assert_eq!(graph.blob(0), Some(value.as_slice()));
        }
    }

    #[test]
    fn blob_order_survives_semantic_diagnostics_and_compact_encoding() {
        let mut ctx = PortableBuildContext::new(
            7,
            11,
            limits().with_max_blob_bytes(8),
            state_limits(),
        )
        .unwrap();
        let first = ctx.push_blob([0xaa_u8, 0xbb]).unwrap();
        let second = ctx.push_blob([0xcc_u8]).unwrap();
        let duplicate = ctx.push_blob([0xaa_u8, 0xbb]).unwrap();
        assert_eq!(duplicate, first);

        let root = ctx
            .push_node(
                WIDGET_TEXT,
                Version::new(1, 0),
                None,
                source(1),
                &[
                    WidgetProperty::new(PropertyId::new(1), first),
                    WidgetProperty::new(PropertyId::new(2), second),
                    WidgetProperty::new(PropertyId::new(3), duplicate),
                ],
                &[],
            )
            .unwrap();
        let graph = ctx.finish_graph(root).unwrap();
        let assembly = graph.to_assembly().unwrap();
        assert!(assembly.contains("BLOBREF blob0"));
        assert!(assembly.contains("BLOBREF blob1"));
        assert!(assembly.contains("blob0:\n  BLOB aabb"));
        assert!(assembly.contains("blob1:\n  BLOB cc"));

        let document = graph.compile();
        let bytes = document.encode().unwrap();
        let view = WidgetDocumentView::decode(&bytes, document.model_limits()).unwrap();
        assert_eq!(view.blob(0), Some([0xaa_u8, 0xbb].as_slice()));
        assert_eq!(view.blob(1), Some([0xcc_u8].as_slice()));
        assert_eq!(view.blob_count(), 2);
    }

    #[test]
    fn semantic_graph_assembly_round_trips_to_identical_awir() {
        let mut ctx = context();
        let content = ctx.push_owned_string("Hello".to_owned()).unwrap();
        let text = ctx
            .push_node(
                WIDGET_TEXT,
                Version::new(1, 0),
                None,
                source(1),
                &[WidgetProperty::new(PROPERTY_TEXT_CONTENT, content)],
                &[],
            )
            .unwrap();
        let root = ctx
            .push_node(
                WIDGET_COLUMN,
                Version::new(1, 0),
                None,
                source(2),
                &[],
                &[text],
            )
            .unwrap();
        let graph = ctx.finish_graph(root).unwrap();
        let model_limits = graph.model_limits();

        let assembly = graph.to_assembly().unwrap();
        let assembled = WidgetAssemblyDocument::parse(&assembly, model_limits)
            .unwrap()
            .encode()
            .unwrap();
        let direct = graph.compile().encode().unwrap();

        assert_eq!(assembled, direct);
        assert!(assembly.contains("ROOT node1"));
        assert!(assembly.contains("STRING \"Hello\""));
    }

    #[test]
    fn semantic_graph_rejects_an_invalid_root_and_detached_nodes() {
        let mut empty = context();
        assert!(matches!(
            empty.finish_graph(PortableNodeId::new(0)),
            Err(PortableBuildError::InvalidChild { node_count: 0, .. })
        ));

        let mut detached = context();
        let first = detached
            .push_node(
                WIDGET_TEXT,
                Version::new(1, 0),
                None,
                source(1),
                &[],
                &[],
            )
            .unwrap();
        detached
            .push_node(
                WIDGET_TEXT,
                Version::new(1, 0),
                None,
                source(2),
                &[],
                &[],
            )
            .unwrap();

        assert!(matches!(
            detached.finish_graph(first),
            Err(PortableBuildError::IncompleteTree)
        ));
    }

    #[test]
    fn equal_semantic_graphs_compile_to_identical_awir() {
        fn build() -> Vec<u8> {
            let mut context = context();
            let content = context.push_owned_string("stable".to_owned()).unwrap();
            let root = context
                .push_node(
                    WIDGET_TEXT,
                    Version::new(1, 0),
                    None,
                    source(1),
                    &[WidgetProperty::new(PROPERTY_TEXT_CONTENT, content)],
                    &[],
                )
                .unwrap();
            context.finish_graph(root).unwrap().compile().encode().unwrap()
        }

        assert_eq!(build(), build());
    }

    #[test]
    fn callback_registry_reports_exact_duplicate_capacity_unknown_and_failure_diagnostics() {
        let source = source(41);
        let calls = Rc::new(Cell::new(0));
        let mut duplicate = context();
        let first = callback(&duplicate, source, EventId::new(1), calls.clone());
        let second = callback(&duplicate, source, EventId::new(1), calls.clone());
        let error = duplicate
            .push_node_with_callbacks(
                WIDGET_BUTTON,
                Version::new(1, 0),
                None,
                source,
                &[],
                vec![first, second],
                &[],
            )
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            format!(
                "duplicate portable callback ID {}",
                duplicate.callback_id_for(None, source, EventId::new(1))
            )
        );

        let mut capacity = PortableBuildContext::new(
            7,
            11,
            limits().with_max_callbacks(0),
            state_limits(),
        )
        .unwrap();
        let registration = callback(&capacity, source, EventId::new(1), calls.clone());
        let error = capacity
            .push_node_with_callbacks(
                WIDGET_BUTTON,
                Version::new(1, 0),
                None,
                source,
                &[],
                vec![registration],
                &[],
            )
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "portable callback capacity exceeded: maximum 0, got 1"
        );

        let mut context = context();
        let failing_id = context.callback_id_for(None, source, EventId::new(2));
        let root = context
            .push_node_with_callbacks(
                WIDGET_BUTTON,
                Version::new(1, 0),
                None,
                source,
                &[],
                vec![PortableCallback::new(
                    EventId::new(2),
                    Version::new(1, 0),
                    failing_id,
                    || Err(PortableCallbackFailure::new("handler rejected event")),
                )],
                &[],
            )
            .unwrap();
        context.finish_document(root).unwrap();
        let registry = context.callback_registry();
        let unknown = StableId128::from_u128(99);
        assert_eq!(
            registry.dispatch(unknown, &mut context).unwrap_err().to_string(),
            format!("unknown portable callback ID {unknown}")
        );
        assert_eq!(
            registry.dispatch(failing_id, &mut context).unwrap_err().to_string(),
            format!("portable callback {failing_id} failed: handler rejected event")
        );
    }

    #[test]
    fn replacement_registry_rebinds_same_id_and_retires_old_registry() {
        let source = source(42);
        let first_calls = Rc::new(Cell::new(0));
        let second_calls = Rc::new(Cell::new(0));
        let mut context = context();
        let callback_id = context.callback_id_for(None, source, EVENT_BUTTON_PRESS);

        let first = callback(&context, source, EVENT_BUTTON_PRESS, first_calls.clone());
        let root = context
            .push_node_with_callbacks(
                WIDGET_BUTTON,
                Version::new(1, 0),
                None,
                source,
                &[],
                vec![first],
                &[],
            )
            .unwrap();
        context.finish_document(root).unwrap();
        let old = context.callback_registry();
        old.dispatch(callback_id, &mut context).unwrap();

        let second = callback(&context, source, EVENT_BUTTON_PRESS, second_calls.clone());
        let root = context
            .push_node_with_callbacks(
                WIDGET_BUTTON,
                Version::new(1, 0),
                None,
                source,
                &[],
                vec![second],
                &[],
            )
            .unwrap();
        context.finish_document(root).unwrap();
        let current = context.callback_registry();

        assert_eq!(
            old.dispatch(callback_id, &mut context).unwrap_err().to_string(),
            "portable callback registry is retired"
        );
        current.dispatch(callback_id, &mut context).unwrap();
        assert_eq!(first_calls.get(), 1);
        assert_eq!(second_calls.get(), 1);
    }

    #[test]
    fn callback_bindings_are_encoded_exactly() {
        let source = source(43);
        let calls = Rc::new(Cell::new(0));
        let mut context = context();
        let callback_id = context.callback_id_for(None, source, EVENT_BUTTON_PRESS);
        let registration = callback(&context, source, EVENT_BUTTON_PRESS, calls);
        let root = context
            .push_node_with_callbacks(
                WIDGET_BUTTON,
                Version::new(1, 0),
                None,
                source,
                &[],
                vec![registration],
                &[],
            )
            .unwrap();
        let graph = context.finish_graph(root).unwrap();
        assert_eq!(
            graph.node(root).unwrap().callbacks(),
            &[CallbackBinding::new(
                EVENT_BUTTON_PRESS,
                Version::new(1, 0),
                aimer_anteros::StableId128::from_bytes(callback_id.to_bytes()),
            )],
        );
        let document = graph.compile();
        let bytes = document.encode().unwrap();
        let view = WidgetDocumentView::decode(&bytes, document.model_limits()).unwrap();

        assert_eq!(
            view.node(0).unwrap().callbacks().collect::<Vec<_>>(),
            vec![CallbackBinding::new(
                EVENT_BUTTON_PRESS,
                Version::new(1, 0),
                aimer_anteros::StableId128::from_bytes(callback_id.to_bytes()),
            )]
        );
    }

    #[test]
    fn erased_widget_uses_portable_conversion_without_native_construction() {
        let mut ctx = context();
        let node = PortableText("erased".into())
            .boxed()
            .into_portable_node(&mut ctx, source(3))
            .unwrap();

        assert_eq!(node.index(), 0);
    }

    #[test]
    fn unsupported_widget_reports_its_native_name_and_source() {
        let mut ctx = context();
        let error = NativeOnly
            .to_portable_node(&mut ctx, source(4))
            .unwrap_err();

        assert!(matches!(
            error,
            PortableBuildError::UnsupportedWidget { widget: "NativeOnly", source: actual }
                if actual == source(4)
        ));
        assert!(error.to_string().contains("NativeOnly"));
    }

    #[test]
    fn portable_lowering_is_owned_by_the_portable_widget_capability() {
        let mut ctx = context();
        let node = PortableWidget::to_portable_node(
            PortableCapabilityProbe,
            &mut ctx,
            source(5),
        )
        .unwrap();

        assert_eq!(node.index(), 0);
    }

    #[test]
    fn every_widget_document_limit_is_enforced_before_growth() {
        assert_limit(
            PortableBuildContext::new(
                0,
                0,
                limits().with_max_document_bytes(63),
                state_limits(),
            ),
            PortableWidgetResource::DocumentBytes,
        );

        let mut ctx = PortableBuildContext::new(
            0,
            0,
            limits().with_max_nodes(0),
            state_limits(),
        )
        .unwrap();
        assert_limit(
            ctx.push_node(WIDGET_TEXT, Version::new(1, 0), None, source(1), &[], &[]),
            PortableWidgetResource::Nodes,
        );

        let mut ctx = PortableBuildContext::new(
            0,
            0,
            limits().with_max_properties(0),
            state_limits(),
        )
        .unwrap();
        assert_limit(
            ctx.push_node(
                WIDGET_TEXT,
                Version::new(1, 0),
                None,
                source(1),
                &[WidgetProperty::new(PropertyId::new(1), PropertyValue::Bool(true))],
                &[],
            ),
            PortableWidgetResource::Properties,
        );

        let mut ctx = PortableBuildContext::new(
            0,
            0,
            limits().with_max_children(0),
            state_limits(),
        )
        .unwrap();
        let child = ctx
            .push_node(WIDGET_TEXT, Version::new(1, 0), None, source(1), &[], &[])
            .unwrap();
        assert_limit(
            ctx.push_node(
                WIDGET_COLUMN,
                Version::new(1, 0),
                None,
                source(2),
                &[],
                &[child],
            ),
            PortableWidgetResource::Children,
        );

        let mut ctx = PortableBuildContext::new(
            0,
            0,
            limits().with_max_keys(0),
            state_limits(),
        )
        .unwrap();
        assert_limit(
            ctx.push_node(WIDGET_TEXT, Version::new(1, 0), None, source(1), &[], &[]),
            PortableWidgetResource::Keys,
        );

        let mut ctx = PortableBuildContext::new(
            0,
            0,
            limits().with_max_string_bytes(2),
            state_limits(),
        )
        .unwrap();
        assert_limit(
            ctx.push_owned_string("abc".into()),
            PortableWidgetResource::StringBytes,
        );
    }

    fn assert_limit<T>(
        result: Result<T, PortableBuildError>,
        expected: PortableWidgetResource,
    ) {
        assert!(matches!(
            result,
            Err(PortableBuildError::LimitExceeded { resource, .. }) if resource == expected
        ));
    }

    #[test]
    fn explicit_key_precedes_stable_source_fallback() {
        let ctx = context();
        let key = Key::Value("item-7".into());

        assert_eq!(ctx.slot_for(Some(&key), source(1)), ctx.slot_for(Some(&key), source(9)));
        assert_eq!(ctx.slot_for(None, source(1)), ctx.slot_for(None, source(1)));
        assert_ne!(ctx.slot_for(None, source(1)), ctx.slot_for(None, source(2)));
        assert_ne!(ctx.slot_for(Some(&key), source(1)), ctx.slot_for(None, source(1)));
    }

    #[test]
    fn queued_mutations_are_ordered_and_request_one_rebuild() {
        let mut ctx = context();
        let slot = ctx.slot_for(None, source(8));
        ctx.state_registry_mut()
            .insert(slot, 0, &CounterState(1))
            .unwrap();

        ctx.queue_state_mutation::<CounterState, _>(slot, |state| state.0 += 1)
            .unwrap();
        ctx.queue_state_mutation::<CounterState, _>(slot, |state| state.0 *= 3)
            .unwrap();
        assert_eq!(ctx.state_registry().restore::<CounterState>(slot).unwrap(), CounterState(1));
        assert!(ctx.rebuild_requested());

        ctx.apply_queued_mutations().unwrap();

        assert_eq!(ctx.state_registry().restore::<CounterState>(slot).unwrap(), CounterState(6));
        assert_eq!(ctx.state_registry().revision(slot), Some(2));
        assert!(ctx.take_rebuild_request());
        assert!(!ctx.take_rebuild_request());
    }

    #[test]
    fn explicit_rebuild_without_mutation_is_queued() {
        let mut ctx = context();
        ctx.queue_rebuild();
        assert!(ctx.rebuild_requested());
        assert!(ctx.take_rebuild_request());
    }

    #[test]
    fn callback_registry_reports_duplicate_capacity_unknown_and_callback_failure() {
        let callback_id = StableId128::from_u128(91);
        let duplicate = PortableCallback::new(
            EventId::new(1),
            Version::new(1, 0),
            callback_id,
            || Ok(()),
        );
        let mut ctx = PortableBuildContext::new(
            7,
            0,
            limits().with_max_callbacks(1),
            state_limits(),
        )
        .unwrap();
        let error = ctx
            .push_node_with_callbacks(
                WIDGET_TEXT,
                Version::new(1, 0),
                None,
                source(1),
                &[],
                vec![
                    duplicate,
                    PortableCallback::new(
                        EventId::new(2),
                        Version::new(1, 0),
                        callback_id,
                        || Ok(()),
                    ),
                ],
                &[],
            )
            .unwrap_err();
        assert!(matches!(
            error,
            PortableBuildError::Callback(PortableCallbackError::Duplicate { id })
                if id == callback_id
        ));

        let first_id = StableId128::from_u128(92);
        let second_id = StableId128::from_u128(93);
        let error = ctx
            .push_node_with_callbacks(
                WIDGET_TEXT,
                Version::new(1, 0),
                None,
                source(1),
                &[],
                vec![
                    PortableCallback::new(
                        EventId::new(1),
                        Version::new(1, 0),
                        first_id,
                        || Ok(()),
                    ),
                    PortableCallback::new(
                        EventId::new(2),
                        Version::new(1, 0),
                        second_id,
                        || Ok(()),
                    ),
                ],
                &[],
            )
            .unwrap_err();
        assert!(matches!(
            error,
            PortableBuildError::Callback(PortableCallbackError::Capacity { max: 1, actual: 2 })
        ));

        let failed_id = StableId128::from_u128(94);
        let node = ctx
            .push_node_with_callbacks(
                WIDGET_TEXT,
                Version::new(1, 0),
                None,
                source(1),
                &[],
                vec![PortableCallback::new(
                    EventId::new(1),
                    Version::new(1, 0),
                    failed_id,
                    || Err(PortableCallbackFailure::new("failed deliberately")),
                )],
                &[],
            )
            .unwrap();
        ctx.finish_document(node).unwrap();
        let registry = ctx.take_callback_registry();
        let unknown = StableId128::from_u128(95);
        assert!(matches!(
            registry.dispatch(unknown, &mut ctx),
            Err(PortableCallbackError::Unknown { id }) if id == unknown
        ));
        assert!(matches!(
            registry.dispatch(failed_id, &mut ctx),
            Err(PortableCallbackError::CallbackFailed { id, message })
                if id == failed_id && message == "failed deliberately"
        ));
    }

    #[test]
    fn replacement_registry_rebinds_stable_id_and_retires_old_behavior() {
        use std::cell::Cell;
        use std::rc::Rc;

        let calls = Rc::new(Cell::new(0));
        let callback_id = StableId128::from_u128(101);
        let mut ctx = context();
        let old_calls = calls.clone();
        let old_node = ctx
            .push_node_with_callbacks(
                WIDGET_TEXT,
                Version::new(1, 0),
                None,
                source(1),
                &[],
                vec![PortableCallback::new(
                    EventId::new(1),
                    Version::new(1, 0),
                    callback_id,
                    move || {
                        old_calls.set(old_calls.get() + 1);
                        Ok(())
                    },
                )],
                &[],
            )
            .unwrap();
        ctx.finish_document(old_node).unwrap();
        let old_registry = ctx.take_callback_registry();

        let new_calls = calls.clone();
        let new_node = ctx
            .push_node_with_callbacks(
                WIDGET_TEXT,
                Version::new(1, 0),
                None,
                source(1),
                &[],
                vec![PortableCallback::new(
                    EventId::new(1),
                    Version::new(1, 0),
                    callback_id,
                    move || {
                        new_calls.set(new_calls.get() + 10);
                        Ok(())
                    },
                )],
                &[],
            )
            .unwrap();
        ctx.finish_document(new_node).unwrap();
        let new_registry = ctx.take_callback_registry();

        assert!(matches!(
            old_registry.dispatch(callback_id, &mut ctx),
            Err(PortableCallbackError::Retired)
        ));
        new_registry.dispatch(callback_id, &mut ctx).unwrap();
        assert_eq!(calls.get(), 10);

        let mut newer_context = context();
        assert!(matches!(
            new_registry.dispatch(callback_id, &mut newer_context),
            Err(PortableCallbackError::Retired)
        ));
    }
}
