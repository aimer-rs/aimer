use aimer_anteros::{
    AbiStatus, AsyncCallbackEventView, CallbackEventView, GuestOperation,
    MAX_GUEST_DIAGNOSTIC_BYTES, ManifestView, ModelLimits, StateBundleView, WidgetDocumentView,
    capture_guest_panic,
};
use std::ops::{Deref, DerefMut};
use std::sync::OnceLock;

use crossbeam::queue::SegQueue;

use crate::memory::AllocationLedger;
use crate::{GuestError, GuestProgram};

/// Explicit guest-side document and host-memory ceilings.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GuestLimits {
    model: ModelLimits,
    max_live_allocations: u32,
    max_host_allocation_bytes: u32,
    max_alignment: u32,
    max_diagnostic_bytes: u32,
}

impl GuestLimits {
    /// Creates explicit portable-model and host-owned allocation ceilings.
    #[inline]
    pub const fn new(
        model: ModelLimits,
        max_live_allocations: u32,
        max_host_allocation_bytes: u32,
        max_alignment: u32,
    ) -> Self {
        Self {
            model,
            max_live_allocations,
            max_host_allocation_bytes,
            max_alignment,
            max_diagnostic_bytes: MAX_GUEST_DIAGNOSTIC_BYTES as u32,
        }
    }

    /// Replaces the structured diagnostic output ceiling.
    #[inline]
    pub const fn max_diagnostic_bytes(mut self, maximum: u32) -> Self {
        self.max_diagnostic_bytes = maximum;
        self
    }

    #[inline]
    pub(crate) const fn max_live_allocations(self) -> u32 {
        self.max_live_allocations
    }

    #[inline]
    pub(crate) const fn max_host_allocation_bytes(self) -> u32 {
        self.max_host_allocation_bytes
    }

    #[inline]
    pub(crate) const fn max_alignment(self) -> u32 {
        self.max_alignment
    }

    #[inline]
    pub(crate) const fn diagnostic_limit(self) -> usize {
        self.max_diagnostic_bytes as usize
    }

    fn validate(self) -> Result<(), GuestError> {
        if self.max_live_allocations == 0
            || self.max_host_allocation_bytes == 0
            || self.max_alignment == 0
            || !self.max_alignment.is_power_of_two()
            || self.max_diagnostic_bytes == 0
            || self.max_diagnostic_bytes as usize > MAX_GUEST_DIAGNOSTIC_BYTES
        {
            return Err(GuestError::new(AbiStatus::InvalidArgument));
        }
        Ok(())
    }
}

/// Validating adapter between one portable program and the raw WASM exports.
pub struct GuestAdapter<P> {
    program: P,
    limits: GuestLimits,
}

impl<P: GuestProgram> GuestAdapter<P> {
    /// Creates an adapter after validating every configured resource ceiling.
    pub fn new(program: P, limits: GuestLimits) -> Result<Self, GuestError> {
        limits.validate()?;
        Ok(Self { program, limits })
    }

    /// Produces and validates the canonical application manifest.
    pub fn manifest(&self) -> Result<Vec<u8>, GuestError> {
        let bytes = self
            .program
            .manifest(self.limits.model)
            .map_err(|error| error.with_operation(GuestOperation::Manifest))?;
        ManifestView::decode(&bytes, self.limits.model)
            .map_err(GuestError::from_model)
            .map_err(|error| error.with_operation(GuestOperation::Manifest))?;
        Ok(bytes)
    }

    /// Applies immutable host context before state transfer or application work.
    pub fn initialize(&mut self, generation_id: u64) -> Result<(), GuestError> {
        self.program
            .initialize(generation_id)
            .map_err(|error| error.with_operation(GuestOperation::Initialize))
    }

    /// Publishes the host window metrics before portable application work.
    #[inline]
    pub fn set_window_metrics(
        &mut self,
        width: u32,
        height: u32,
        scale_factor: f64,
    ) -> Result<(), GuestError> {
        self.program
            .set_window_metrics(width, height, scale_factor)
            .map_err(|error| error.with_operation(GuestOperation::Initialize))
    }

    /// Produces and validates the current canonical Widget IR image.
    pub fn build(&mut self) -> Result<Vec<u8>, GuestError> {
        let bytes = self
            .program
            .build(self.limits.model)
            .map_err(|error| error.with_operation(GuestOperation::Build))?;
        WidgetDocumentView::decode(&bytes, self.limits.model)
            .map_err(GuestError::from_model)
            .map_err(|error| error.with_operation(GuestOperation::Build))?;
        Ok(bytes)
    }

    /// Validates and dispatches a callback exactly once.
    pub fn dispatch_event(&mut self, event: &[u8]) -> Result<Option<Vec<u8>>, GuestError> {
        let event = CallbackEventView::decode(event, self.limits.model)
            .map_err(GuestError::from_model)
            .map_err(|error| error.with_operation(GuestOperation::CallbackRebuild))?;
        let output = self
            .program
            .dispatch_event(&event, self.limits.model)
            .map_err(|error| error.with_operation(GuestOperation::CallbackRebuild))?;
        if let Some(bytes) = &output {
            WidgetDocumentView::decode(bytes, self.limits.model)
                .map_err(GuestError::from_model)
                .map_err(|error| error.with_operation(GuestOperation::CallbackRebuild))?;
        }
        Ok(output)
    }

    /// Polls guest-owned async work and validates an optional Widget IR image.
    pub fn poll_async(&mut self) -> Result<Option<Vec<u8>>, GuestError> {
        let output = self
            .program
            .poll_async(self.limits.model)
            .map_err(|error| error.with_operation(GuestOperation::Build))?;
        if let Some(bytes) = &output {
            WidgetDocumentView::decode(bytes, self.limits.model)
                .map_err(GuestError::from_model)
                .map_err(|error| error.with_operation(GuestOperation::Build))?;
        }
        Ok(output)
    }

    /// Returns the guest's bounded async-work wake hint.
    #[inline]
    pub fn has_async_work(&self) -> bool {
        self.program.has_async_work()
    }

    /// Validates and dispatches one host-owned async completion document.
    pub fn dispatch_async_event(&mut self, event: &[u8]) -> Result<Option<Vec<u8>>, GuestError> {
        let event = AsyncCallbackEventView::decode(event, self.limits.model)
            .map_err(GuestError::from_model)
            .map_err(|error| error.with_operation(GuestOperation::CallbackRebuild))?;
        let output = self
            .program
            .dispatch_async_event(&event, self.limits.model)
            .map_err(|error| error.with_operation(GuestOperation::CallbackRebuild))?;
        if let Some(bytes) = &output {
            WidgetDocumentView::decode(bytes, self.limits.model)
                .map_err(GuestError::from_model)
                .map_err(|error| error.with_operation(GuestOperation::CallbackRebuild))?;
        }
        Ok(output)
    }

    /// Produces and validates the complete canonical state image.
    pub fn export_state(&self) -> Result<Vec<u8>, GuestError> {
        let bytes = self
            .program
            .export_state(self.limits.model)
            .map_err(|error| error.with_operation(GuestOperation::ExportState))?;
        StateBundleView::decode(&bytes, self.limits.model)
            .map_err(GuestError::from_model)
            .map_err(|error| error.with_operation(GuestOperation::ExportState))?;
        Ok(bytes)
    }

    /// Validates and synchronously imports a complete state image.
    pub fn import_state(&mut self, state: &[u8]) -> Result<(), GuestError> {
        let state = StateBundleView::decode(state, self.limits.model)
            .map_err(GuestError::from_model)
            .map_err(|error| error.with_operation(GuestOperation::Import))?;
        self.program
            .import_state(&state)
            .map_err(|error| error.with_operation(GuestOperation::Import))
    }

    /// Validates an old image and validates the program's migrated state image.
    pub fn migrate_state(&mut self, state: &[u8]) -> Result<Vec<u8>, GuestError> {
        let state = StateBundleView::decode(state, self.limits.model)
            .map_err(GuestError::from_model)
            .map_err(|error| error.with_operation(GuestOperation::Migration))?;
        let output = self
            .program
            .migrate_state(&state, self.limits.model)
            .map_err(|error| error.with_operation(GuestOperation::Migration))?;
        StateBundleView::decode(&output, self.limits.model)
            .map_err(GuestError::from_model)
            .map_err(|error| error.with_operation(GuestOperation::Migration))?;
        Ok(output)
    }
}

struct RawGuest<P> {
    adapter: GuestAdapter<P>,
    memory: AllocationLedger,
    pending_manifest: Option<Vec<u8>>,
    pending_build: Option<Vec<u8>>,
    pending_async: Option<Vec<u8>>,
    pending_state: Option<Vec<u8>>,
    pending_migration: Option<(Vec<u8>, Vec<u8>)>,
    pending_diagnostic: Option<Vec<u8>>,
}

impl<P: GuestProgram> RawGuest<P> {
    fn new(program: P, limits: GuestLimits) -> Result<Self, GuestError> {
        Ok(Self {
            adapter: GuestAdapter::new(program, limits)?,
            memory: AllocationLedger::new(
                limits.max_live_allocations(),
                limits.max_host_allocation_bytes(),
                limits.max_alignment(),
            ),
            pending_manifest: None,
            pending_build: None,
            pending_async: None,
            pending_state: None,
            pending_migration: None,
            pending_diagnostic: None,
        })
    }

    fn allocate(&mut self, length: i32, alignment: i32) -> Result<u32, GuestError> {
        let length =
            usize::try_from(length).map_err(|_| GuestError::new(AbiStatus::InvalidArgument))?;
        let alignment =
            usize::try_from(alignment).map_err(|_| GuestError::new(AbiStatus::InvalidArgument))?;
        let pointer = self.memory.allocate(length, alignment)?;
        match u32::try_from(pointer) {
            Ok(pointer) => Ok(pointer),
            Err(_) => {
                self.memory.deallocate(pointer, length, alignment)?;
                Err(GuestError::new(AbiStatus::ResourceExhausted))
            }
        }
    }

    fn deallocate(&mut self, pointer: i32, length: i32, alignment: i32) -> Result<(), GuestError> {
        let pointer = pointer as u32 as usize;
        let length =
            usize::try_from(length).map_err(|_| GuestError::new(AbiStatus::InvalidArgument))?;
        let alignment =
            usize::try_from(alignment).map_err(|_| GuestError::new(AbiStatus::InvalidArgument))?;
        self.memory.deallocate(pointer, length, alignment)
    }

    fn initialize(&mut self, generation_id: i64) -> Result<u32, GuestError> {
        self.reject_other_pending_outputs(true, true, true, true, false)?;
        self.adapter.initialize(generation_id as u64)?;
        Ok(0)
    }

    fn set_window_metrics(
        &mut self,
        width: i32,
        height: i32,
        scale_factor_bits: i64,
    ) -> Result<u32, GuestError> {
        self.reject_other_pending_outputs(true, true, true, true, false)?;
        let width =
            u32::try_from(width).map_err(|_| GuestError::new(AbiStatus::InvalidArgument))?;
        let height =
            u32::try_from(height).map_err(|_| GuestError::new(AbiStatus::InvalidArgument))?;
        let scale_factor = f64::from_bits(scale_factor_bits as u64);
        if !scale_factor.is_finite() || scale_factor <= 0.0 {
            return Err(GuestError::new(AbiStatus::InvalidArgument));
        }
        self.adapter
            .set_window_metrics(width, height, scale_factor)?;
        Ok(0)
    }

    fn manifest(&mut self, pointer: i32, capacity: i32) -> Result<u32, GuestError> {
        if self.pending_manifest.is_none() {
            self.reject_other_pending_outputs(false, true, true, true, false)?;
            self.pending_manifest = Some(self.adapter.manifest()?);
        }
        write_cached_output(
            &mut self.memory,
            &mut self.pending_manifest,
            pointer,
            capacity,
        )
    }

    fn build(&mut self, pointer: i32, capacity: i32) -> Result<u32, GuestError> {
        if self.pending_build.is_none() {
            self.reject_other_pending_outputs(true, false, true, true, false)?;
            self.pending_build = Some(self.adapter.build()?);
        }
        write_cached_output(&mut self.memory, &mut self.pending_build, pointer, capacity)
    }

    fn diagnostic(&mut self, pointer: i32, capacity: i32) -> Result<u32, GuestError> {
        write_cached_output(
            &mut self.memory,
            &mut self.pending_diagnostic,
            pointer,
            capacity,
        )
    }

    fn dispatch_event(
        &mut self,
        event_pointer: i32,
        event_length: i32,
        output_pointer: i32,
        output_capacity: i32,
    ) -> Result<u32, GuestError> {
        self.reject_other_pending_outputs(true, true, true, true, false)?;
        let output_capacity = usize::try_from(output_capacity)
            .map_err(|_| GuestError::new(AbiStatus::InvalidArgument))?;
        let output_pointer = output_pointer as u32 as usize;
        self.memory.write(output_pointer, output_capacity, &[])?;
        let event_pointer = event_pointer as u32 as usize;
        let event_length = usize::try_from(event_length)
            .map_err(|_| GuestError::new(AbiStatus::InvalidArgument))?;
        let (memory, adapter) = (&self.memory, &mut self.adapter);
        let event = memory.read(event_pointer, event_length)?;
        let output = adapter.dispatch_event(event)?;
        match output {
            Some(output) => {
                self.memory
                    .write(output_pointer, output_capacity, &output)?;
                u32::try_from(output.len())
                    .map_err(|_| GuestError::new(AbiStatus::ResourceExhausted))
            }
            None => Ok(0),
        }
    }

    fn poll_async(&mut self, pointer: i32, capacity: i32) -> Result<u32, GuestError> {
        if self.pending_async.is_none() {
            self.reject_other_pending_outputs(true, true, true, true, true)?;
            if let Some(output) = self.adapter.poll_async()? {
                self.pending_async = Some(output);
            } else {
                return Ok(0);
            }
        }
        write_cached_output(&mut self.memory, &mut self.pending_async, pointer, capacity)
    }

    fn async_ready(&self) -> Result<u32, GuestError> {
        Ok(u32::from(self.adapter.has_async_work()))
    }

    fn dispatch_async_event(
        &mut self,
        event_pointer: i32,
        event_length: i32,
        output_pointer: i32,
        output_capacity: i32,
    ) -> Result<u32, GuestError> {
        self.reject_other_pending_outputs(true, true, true, true, false)?;
        let output_capacity = usize::try_from(output_capacity)
            .map_err(|_| GuestError::new(AbiStatus::InvalidArgument))?;
        let output_pointer = output_pointer as u32 as usize;
        self.memory.write(output_pointer, output_capacity, &[])?;
        let event_pointer = event_pointer as u32 as usize;
        let event_length = usize::try_from(event_length)
            .map_err(|_| GuestError::new(AbiStatus::InvalidArgument))?;
        let (memory, adapter) = (&self.memory, &mut self.adapter);
        let event = memory.read(event_pointer, event_length)?;
        let output = adapter.dispatch_async_event(event)?;
        match output {
            Some(output) => {
                self.memory
                    .write(output_pointer, output_capacity, &output)?;
                u32::try_from(output.len())
                    .map_err(|_| GuestError::new(AbiStatus::ResourceExhausted))
            }
            None => Ok(0),
        }
    }

    fn export_state(&mut self, pointer: i32, capacity: i32) -> Result<u32, GuestError> {
        if self.pending_state.is_none() {
            self.reject_other_pending_outputs(true, true, false, true, false)?;
            self.pending_state = Some(self.adapter.export_state()?);
        }
        write_cached_output(&mut self.memory, &mut self.pending_state, pointer, capacity)
    }

    fn import_state(&mut self, pointer: i32, length: i32) -> Result<u32, GuestError> {
        self.reject_other_pending_outputs(true, true, true, true, false)?;
        let pointer = pointer as u32 as usize;
        let length =
            usize::try_from(length).map_err(|_| GuestError::new(AbiStatus::InvalidArgument))?;
        let (memory, adapter) = (&self.memory, &mut self.adapter);
        let state = memory.read(pointer, length)?;
        adapter.import_state(state)?;
        Ok(0)
    }

    fn migrate_state(
        &mut self,
        state_pointer: i32,
        state_length: i32,
        output_pointer: i32,
        output_capacity: i32,
    ) -> Result<u32, GuestError> {
        let state_pointer = state_pointer as u32 as usize;
        let state_length = usize::try_from(state_length)
            .map_err(|_| GuestError::new(AbiStatus::InvalidArgument))?;
        let state = self.memory.read(state_pointer, state_length)?;
        if self.pending_migration.is_none() {
            self.reject_other_pending_outputs(true, true, true, false, false)?;
            let input = state.to_vec();
            let output = self.adapter.migrate_state(state)?;
            self.pending_migration = Some((input, output));
        } else if self
            .pending_migration
            .as_ref()
            .is_some_and(|(input, _)| input.as_slice() != state)
        {
            return Err(GuestError::new(AbiStatus::InvalidArgument));
        }
        let Some((_, output)) = self.pending_migration.as_ref() else {
            return Err(GuestError::new(AbiStatus::InternalError));
        };
        let required = u32::try_from(output.len())
            .map_err(|_| GuestError::new(AbiStatus::ResourceExhausted))?;
        let capacity = usize::try_from(output_capacity)
            .map_err(|_| GuestError::new(AbiStatus::InvalidArgument))?;
        if capacity < output.len() {
            return Err(GuestError::with_value(AbiStatus::BufferTooSmall, required));
        }
        self.memory
            .write(output_pointer as u32 as usize, capacity, output)?;
        self.pending_migration = None;
        Ok(required)
    }

    fn reject_other_pending_outputs(
        &self,
        manifest: bool,
        build: bool,
        state: bool,
        migration: bool,
        async_poll: bool,
    ) -> Result<(), GuestError> {
        let other_pending = (manifest && self.pending_manifest.is_some())
            || (build && self.pending_build.is_some())
            || (state && self.pending_state.is_some())
            || (migration && self.pending_migration.is_some())
            || (!async_poll && self.pending_async.is_some());
        if other_pending {
            return Err(GuestError::new(AbiStatus::InvalidArgument));
        }
        Ok(())
    }
}

fn write_cached_output(
    memory: &mut AllocationLedger,
    pending: &mut Option<Vec<u8>>,
    pointer: i32,
    capacity: i32,
) -> Result<u32, GuestError> {
    let Some(output) = pending.as_ref() else {
        return Err(GuestError::new(AbiStatus::InternalError));
    };
    let required =
        u32::try_from(output.len()).map_err(|_| GuestError::new(AbiStatus::ResourceExhausted))?;
    let capacity =
        usize::try_from(capacity).map_err(|_| GuestError::new(AbiStatus::InvalidArgument))?;
    if capacity < output.len() {
        return Err(GuestError::with_value(AbiStatus::BufferTooSmall, required));
    }
    memory.write(pointer as u32 as usize, capacity, output)?;
    *pending = None;
    Ok(required)
}

/// Lazily initialized implementation behind [`crate::export_guest!`].
///
/// The guest instance is checked out from a lock-free queue for each export
/// call. A concurrent or reentrant call returns `InternalError` immediately
/// instead of waiting for the single guest instance to become available.
#[doc(hidden)]
pub struct ExportedGuest<P> {
    inner: OnceLock<Result<SegQueue<RawGuest<P>>, GuestError>>,
    limits: GuestLimits,
}

struct GuestSlot<'a, P> {
    queue: &'a SegQueue<RawGuest<P>>,
    guest: Option<RawGuest<P>>,
}

impl<P> Deref for GuestSlot<'_, P> {
    type Target = RawGuest<P>;

    fn deref(&self) -> &Self::Target {
        self.guest
            .as_ref()
            .expect("a checked-out guest slot always contains a guest")
    }
}

impl<P> DerefMut for GuestSlot<'_, P> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.guest
            .as_mut()
            .expect("a checked-out guest slot always contains a guest")
    }
}

impl<P> Drop for GuestSlot<'_, P> {
    fn drop(&mut self) {
        if let Some(guest) = self.guest.take() {
            self.queue.push(guest);
        }
    }
}

impl<P: GuestProgram + Default> ExportedGuest<P> {
    /// Creates an uninitialized export bridge without guest allocations.
    #[doc(hidden)]
    #[inline]
    pub const fn new(limits: GuestLimits) -> Self {
        Self {
            inner: OnceLock::new(),
            limits,
        }
    }

    fn with_guest(
        &self,
        operation_kind: GuestOperation,
        operation: impl FnOnce(&mut RawGuest<P>) -> Result<u32, GuestError>,
    ) -> i64 {
        let guest = self.inner.get_or_init(|| {
            RawGuest::new(P::default(), self.limits).map(|guest| {
                let queue = SegQueue::new();
                queue.push(guest);
                queue
            })
        });
        match guest {
            Ok(queue) => {
                let Some(raw_guest) = queue.pop() else {
                    return pack(AbiStatus::InternalError, 0);
                };
                let mut guest = GuestSlot {
                    queue,
                    guest: Some(raw_guest),
                };
                guest.pending_diagnostic = None;
                match capture_guest_panic(|| operation(&mut guest)) {
                    Err(panic) => {
                        let error = GuestError::from_panic(operation_kind, panic);
                        guest.pending_diagnostic =
                            error.encode_diagnostic(self.limits.diagnostic_limit());
                        pack(error.status(), error.value())
                    }
                    Ok(Ok(value)) => pack(AbiStatus::Ok, value),
                    Ok(Err(error)) => {
                        guest.pending_diagnostic =
                            error.encode_diagnostic(self.limits.diagnostic_limit());
                        pack(error.status(), error.value())
                    }
                }
            }
            Err(error) => pack(error.status(), 0),
        }
    }

    fn with_diagnostic(
        &self,
        operation: impl FnOnce(&mut RawGuest<P>) -> Result<u32, GuestError>,
    ) -> i64 {
        let guest = self.inner.get_or_init(|| {
            RawGuest::new(P::default(), self.limits).map(|guest| {
                let queue = SegQueue::new();
                queue.push(guest);
                queue
            })
        });
        match guest {
            Ok(queue) => {
                let Some(raw_guest) = queue.pop() else {
                    return pack(AbiStatus::InternalError, 0);
                };
                let mut guest = GuestSlot {
                    queue,
                    guest: Some(raw_guest),
                };
                pack_result(operation(&mut guest))
            }
            Err(error) => pack(error.status(), 0),
        }
    }

    fn with_pending_diagnostic(
        &self,
        operation: impl FnOnce(&mut RawGuest<P>) -> Result<u32, GuestError>,
    ) -> i64 {
        let guest = self.inner.get_or_init(|| {
            RawGuest::new(P::default(), self.limits).map(|guest| {
                let queue = SegQueue::new();
                queue.push(guest);
                queue
            })
        });
        match guest {
            Ok(queue) => {
                let Some(raw_guest) = queue.pop() else {
                    return pack(AbiStatus::InternalError, 0);
                };
                let mut guest = GuestSlot {
                    queue,
                    guest: Some(raw_guest),
                };
                pack_result(operation(&mut guest))
            }
            Err(error) => pack(error.status(), 0),
        }
    }

    #[doc(hidden)]
    #[inline]
    pub fn allocate(&self, length: i32, alignment: i32) -> i64 {
        self.with_pending_diagnostic(|guest| guest.allocate(length, alignment))
    }

    #[doc(hidden)]
    #[inline]
    pub fn initialize(&self, generation_id: i64) -> i64 {
        self.with_guest(GuestOperation::Initialize, |guest| {
            guest.initialize(generation_id)
        })
    }

    #[doc(hidden)]
    #[inline]
    pub fn set_window_metrics(&self, width: i32, height: i32, scale_factor_bits: i64) -> i32 {
        let packed = self.with_guest(GuestOperation::Initialize, |guest| {
            guest.set_window_metrics(width, height, scale_factor_bits)
        });
        (packed as u64 >> 32) as i32
    }

    #[doc(hidden)]
    #[inline]
    pub fn deallocate(&self, pointer: i32, length: i32, alignment: i32) -> i32 {
        let packed = self.with_pending_diagnostic(|guest| {
            guest.deallocate(pointer, length, alignment)?;
            Ok(0)
        });
        (packed as u64 >> 32) as i32
    }

    #[doc(hidden)]
    #[inline]
    pub fn manifest(&self, pointer: i32, capacity: i32) -> i64 {
        self.with_guest(GuestOperation::Manifest, |guest| {
            guest.manifest(pointer, capacity)
        })
    }

    #[doc(hidden)]
    #[inline]
    pub fn build(&self, pointer: i32, capacity: i32) -> i64 {
        self.with_guest(GuestOperation::Build, |guest| {
            guest.build(pointer, capacity)
        })
    }

    #[doc(hidden)]
    #[inline]
    pub fn diagnostic(&self, pointer: i32, capacity: i32) -> i64 {
        self.with_diagnostic(|guest| guest.diagnostic(pointer, capacity))
    }

    #[doc(hidden)]
    #[inline]
    pub fn dispatch_event(
        &self,
        event_pointer: i32,
        event_length: i32,
        output_pointer: i32,
        output_capacity: i32,
    ) -> i64 {
        self.with_guest(GuestOperation::CallbackRebuild, |guest| {
            guest.dispatch_event(event_pointer, event_length, output_pointer, output_capacity)
        })
    }

    #[doc(hidden)]
    #[inline]
    pub fn poll_async(&self, pointer: i32, capacity: i32) -> i64 {
        self.with_guest(GuestOperation::Build, |guest| {
            guest.poll_async(pointer, capacity)
        })
    }

    #[doc(hidden)]
    #[inline]
    pub fn async_ready(&self) -> i64 {
        self.with_pending_diagnostic(|guest| guest.async_ready())
    }

    #[doc(hidden)]
    #[inline]
    pub fn dispatch_async_event(
        &self,
        event_pointer: i32,
        event_length: i32,
        output_pointer: i32,
        output_capacity: i32,
    ) -> i64 {
        self.with_guest(GuestOperation::CallbackRebuild, |guest| {
            guest.dispatch_async_event(event_pointer, event_length, output_pointer, output_capacity)
        })
    }

    #[doc(hidden)]
    #[inline]
    pub fn export_state(&self, pointer: i32, capacity: i32) -> i64 {
        self.with_guest(GuestOperation::ExportState, |guest| {
            guest.export_state(pointer, capacity)
        })
    }

    #[doc(hidden)]
    #[inline]
    pub fn import_state(&self, pointer: i32, length: i32) -> i64 {
        self.with_guest(GuestOperation::Import, |guest| {
            guest.import_state(pointer, length)
        })
    }

    #[doc(hidden)]
    #[inline]
    pub fn migrate_state(
        &self,
        state_pointer: i32,
        state_length: i32,
        output_pointer: i32,
        output_capacity: i32,
    ) -> i64 {
        self.with_guest(GuestOperation::Migration, |guest| {
            guest.migrate_state(state_pointer, state_length, output_pointer, output_capacity)
        })
    }
}

#[inline]
const fn pack(status: AbiStatus, value: u32) -> i64 {
    (((status as u64) << 32) | value as u64) as i64
}

#[inline]
fn pack_result(result: Result<u32, GuestError>) -> i64 {
    match result {
        Ok(value) => pack(AbiStatus::Ok, value),
        Err(error) => pack(error.status(), error.value()),
    }
}

#[cfg(test)]
mod tests {
    use aimer_anteros::{
        AbiResult, CallbackEventView, GuestDiagnostic, GuestDiagnosticCategory, GuestOperation,
        GuestPanicScope, MAX_GUEST_DIAGNOSTIC_BYTES, ModelLimits, StateBundleView, Version,
        WidgetDocument, WidgetNode, WidgetSchemaId,
    };

    use super::*;

    #[derive(Default)]
    struct PanickingGuest;

    impl GuestProgram for PanickingGuest {
        fn manifest(&self, _limits: ModelLimits) -> Result<Vec<u8>, GuestError> {
            Err(GuestError::new(AbiStatus::InternalError))
        }

        fn build(&mut self, _limits: ModelLimits) -> Result<Vec<u8>, GuestError> {
            panic!("guest build exploded")
        }

        fn dispatch_event(
            &mut self,
            _event: &CallbackEventView<'_>,
            _limits: ModelLimits,
        ) -> Result<Option<Vec<u8>>, GuestError> {
            Err(GuestError::new(AbiStatus::InternalError))
        }

        fn export_state(&self, _limits: ModelLimits) -> Result<Vec<u8>, GuestError> {
            Err(GuestError::new(AbiStatus::InternalError))
        }

        fn import_state(&mut self, _state: &StateBundleView<'_>) -> Result<(), GuestError> {
            Err(GuestError::new(AbiStatus::InternalError))
        }
    }

    #[test]
    fn raw_build_converts_a_guest_panic_into_a_structured_diagnostic() {
        let limits = GuestLimits::new(ModelLimits::new(4_096, 16, 64, 64), 4, 4_096, 16);
        let guest = ExportedGuest::<PanickingGuest>::new(limits);

        let result = AbiResult::from_packed(guest.with_guest(GuestOperation::Build, |guest| {
            let _scope = GuestPanicScope::new("TestWidget", "build");
            guest.build(0, 0)
        }))
        .unwrap();

        assert_eq!(result.status(), AbiStatus::ApplicationError);
        let queue = guest.inner.get().unwrap().as_ref().unwrap();
        let raw_guest = queue.pop().expect("the guest should be available");
        let diagnostic = raw_guest
            .pending_diagnostic
            .as_deref()
            .map(|bytes| GuestDiagnostic::decode(bytes, MAX_GUEST_DIAGNOSTIC_BYTES).unwrap())
            .expect("the panic diagnostic should be pending");
        queue.push(raw_guest);

        assert_eq!(diagnostic.operation(), GuestOperation::Build);
        assert_eq!(diagnostic.category(), GuestDiagnosticCategory::Panic);
        assert_eq!(diagnostic.widget(), Some("TestWidget"));
        assert!(
            diagnostic
                .message()
                .contains("during build: guest build exploded")
        );
        assert!(diagnostic.location().is_some());
    }

    #[derive(Default)]
    struct PollGuest;

    impl GuestProgram for PollGuest {
        fn manifest(&self, _limits: ModelLimits) -> Result<Vec<u8>, GuestError> {
            Ok(Vec::new())
        }

        fn build(&mut self, limits: ModelLimits) -> Result<Vec<u8>, GuestError> {
            Ok(widget_image(limits))
        }

        fn dispatch_event(
            &mut self,
            _event: &CallbackEventView<'_>,
            _limits: ModelLimits,
        ) -> Result<Option<Vec<u8>>, GuestError> {
            Ok(None)
        }

        fn poll_async(&mut self, limits: ModelLimits) -> Result<Option<Vec<u8>>, GuestError> {
            Ok(Some(widget_image(limits)))
        }

        fn export_state(&self, _limits: ModelLimits) -> Result<Vec<u8>, GuestError> {
            Err(GuestError::new(AbiStatus::InternalError))
        }

        fn import_state(&mut self, _state: &StateBundleView<'_>) -> Result<(), GuestError> {
            Err(GuestError::new(AbiStatus::InternalError))
        }
    }

    fn widget_image(limits: ModelLimits) -> Vec<u8> {
        WidgetDocument::new(
            7,
            1,
            0,
            &[WidgetNode::new(WidgetSchemaId::new(1), Version::new(1, 0))],
            &[],
            &[],
        )
        .encode(limits)
        .unwrap()
    }

    #[test]
    fn raw_async_poll_negotiates_and_reuses_one_completion_image() {
        let model_limits = ModelLimits::new(4_096, 16, 64, 64);
        let limits = GuestLimits::new(model_limits, 4, 4_096, 16);
        let mut guest = RawGuest::new(PollGuest, limits).unwrap();

        let probe = AbiResult::from_packed(pack_result(guest.poll_async(0, 0))).unwrap();
        assert_eq!(probe.status(), AbiStatus::BufferTooSmall);
        assert!(probe.value() > 0);
        assert_eq!(
            guest.pending_async.as_ref().map(Vec::len),
            Some(probe.value() as usize)
        );

        let retry_probe = AbiResult::from_packed(pack_result(guest.poll_async(0, 0))).unwrap();
        assert_eq!(retry_probe.status(), AbiStatus::BufferTooSmall);
        assert_eq!(retry_probe.value(), probe.value());
    }
}
