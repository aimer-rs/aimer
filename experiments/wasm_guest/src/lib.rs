//! Generated application adapter for Aimer's development WASM guest ABI.
//!
//! This crate is linked only into the interpreted development guest. It owns
//! the raw WebAssembly export boundary while application programs operate on
//! validated, borrowed portable documents and owned canonical output images.

mod diagnostic;
mod exports;
mod memory;

use aimer_anteros::{
    AsyncCallbackEventView, CallbackEventView, ModelLimits, StableId128, StateBundleView,
};

pub use aimer_anteros as anteros;
pub use diagnostic::{GuestError, MAX_GUEST_PANIC_PAYLOAD_BYTES};
pub use exports::{ExportedGuest, GuestAdapter, GuestLimits};

/// Portable application behavior invoked by the generated guest exports.
///
/// Implementations produce canonical images with the `aimer_anteros` model
/// encoders. [`GuestAdapter`] validates every image again before exposing it to
/// the host, so malformed application output cannot cross the ABI boundary.
pub trait GuestProgram: Send {
    /// Returns this program's canonical `AMNF` capability manifest.
    fn manifest(&self, limits: ModelLimits) -> Result<Vec<u8>, GuestError>;

    /// Receives the permanent host's monotonically assigned generation ID.
    ///
    /// Generated programs store this value in portable guest state and use it
    /// in every `AWIR`, `ASTA`, and callback document produced by this module.
    /// The default preserves compatibility for programs whose output does not
    /// contain generation-scoped identities.
    fn initialize(&mut self, _generation_id: u64) -> Result<(), GuestError> {
        Ok(())
    }

    /// Publishes the host window metrics used by responsive portable widgets.
    ///
    /// The portable guest cannot borrow a native window, so the host supplies
    /// the physical client size and scale factor before a build or callback.
    /// Guests that do not read window metrics may keep the default no-op.
    fn set_window_metrics(
        &mut self,
        _width: u32,
        _height: u32,
        _scale_factor: f64,
    ) -> Result<(), GuestError> {
        Ok(())
    }

    /// Builds the program's current canonical `AWIR` widget tree.
    fn build(&mut self, limits: ModelLimits) -> Result<Vec<u8>, GuestError>;

    /// Dispatches one validated callback event and optionally rebuilds `AWIR`.
    fn dispatch_event(
        &mut self,
        event: &CallbackEventView<'_>,
        limits: ModelLimits,
    ) -> Result<Option<Vec<u8>>, GuestError>;

    /// Polls guest-owned async callback work at a host safe point.
    ///
    /// The default is an idle guest, preserving compatibility for modules that
    /// do not advertise async callback lowering. A non-empty result is a
    /// complete canonical `AWIR` image and is visible only at this boundary.
    fn poll_async(&mut self, _limits: ModelLimits) -> Result<Option<Vec<u8>>, GuestError> {
        Ok(None)
    }

    /// Reports whether a future or wake is waiting for the next guest safe
    /// point. The result is a bounded hint only; the host still calls
    /// [`Self::poll_async`] and validates any returned image.
    fn has_async_work(&self) -> bool {
        false
    }

    /// Delivers one validated host-owned async completion to the guest.
    ///
    /// Implementations must consume only the stable generation, callback, and
    /// task identities plus the bounded payload. Futures, executor handles, and
    /// native closures remain on their owning side of the protocol.
    fn dispatch_async_event(
        &mut self,
        _event: &AsyncCallbackEventView<'_>,
        _limits: ModelLimits,
    ) -> Result<Option<Vec<u8>>, GuestError> {
        Err(GuestError::new(
            aimer_anteros::AbiStatus::UnsupportedVersion,
        ))
    }

    /// Exports the complete canonical `ASTA` state snapshot.
    fn export_state(&self, limits: ModelLimits) -> Result<Vec<u8>, GuestError>;

    /// Imports one validated canonical `ASTA` state snapshot synchronously.
    fn import_state(&mut self, state: &StateBundleView<'_>) -> Result<(), GuestError>;

    /// Migrates an older validated state snapshot into this program's schema.
    ///
    /// Programs without a migration path reject the operation explicitly. The
    /// host can then apply its registered portable migration policy.
    fn migrate_state(
        &mut self,
        _state: &StateBundleView<'_>,
        _limits: ModelLimits,
    ) -> Result<Vec<u8>, GuestError> {
        Err(GuestError::new(aimer_anteros::AbiStatus::StateIncompatible))
    }
}

/// A registered callback that mutates portable guest state.
pub type CallbackHandler<S> = fn(&mut S, &CallbackEventView<'_>) -> Result<bool, GuestError>;

/// A bounded stable-identity callback table for one guest program.
///
/// The table stores function pointers rather than closures, keeping callback
/// registration deterministic and avoiding captured native addresses or
/// application-owned allocations at dispatch time.
pub struct CallbackRegistry<S> {
    registrations: Vec<(StableId128, CallbackHandler<S>)>,
    max_callbacks: usize,
}

impl<S> CallbackRegistry<S> {
    /// Creates a fail-closed callback table with a zero-entry limit.
    #[inline]
    pub const fn new() -> Self {
        Self {
            registrations: Vec::new(),
            max_callbacks: 0,
        }
    }

    /// Sets the maximum number of callbacks this table can own.
    #[inline]
    pub const fn max_callbacks(mut self, max_callbacks: usize) -> Self {
        self.max_callbacks = max_callbacks;
        self
    }

    /// Registers one stable callback identity.
    pub fn register(
        &mut self,
        callback_id: StableId128,
        handler: CallbackHandler<S>,
    ) -> Result<(), GuestError> {
        match self
            .registrations
            .binary_search_by_key(&callback_id, |(registered_id, _)| *registered_id)
        {
            Ok(_) => Err(GuestError::new(aimer_anteros::AbiStatus::DuplicateId)),
            Err(_) if self.registrations.len() >= self.max_callbacks => {
                Err(GuestError::new(aimer_anteros::AbiStatus::ResourceExhausted))
            }
            Err(index) => {
                self.registrations.insert(index, (callback_id, handler));
                Ok(())
            }
        }
    }

    /// Dispatches a validated event to its registered callback identity.
    pub fn dispatch(
        &self,
        state: &mut S,
        event: &CallbackEventView<'_>,
    ) -> Result<bool, GuestError> {
        let index = self
            .registrations
            .binary_search_by_key(&event.callback_id(), |(callback_id, _)| *callback_id)
            .map_err(|_| GuestError::new(aimer_anteros::AbiStatus::UnknownId))?;
        (self.registrations[index].1)(state, event)
    }
}

impl<S> Default for CallbackRegistry<S> {
    fn default() -> Self {
        Self::new()
    }
}

/// Exports one `Default` program through Aimer's stable WASM guest ABI.
///
/// Invoke this macro exactly once in the generated guest package. Initialization
/// is lazy, so the resulting module has no WebAssembly start function and no
/// application code runs before the host validates and instantiates it.
#[macro_export]
macro_rules! export_guest {
    ($program:ty, $limits:expr $(,)?) => {
        static AIMER_EXPORTED_GUEST: $crate::ExportedGuest<$program> =
            $crate::ExportedGuest::new($limits);

        #[unsafe(no_mangle)]
        pub extern "C" fn aimer_abi_version() -> i64 {
            $crate::anteros::CURRENT_ABI_VERSION.to_packed()
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn aimer_initialize(generation_id: i64) -> i64 {
            AIMER_EXPORTED_GUEST.initialize(generation_id)
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn aimer_set_window_metrics(
            width: i32,
            height: i32,
            scale_factor_bits: i64,
        ) -> i32 {
            AIMER_EXPORTED_GUEST.set_window_metrics(width, height, scale_factor_bits)
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn aimer_alloc(length: i32, alignment: i32) -> i64 {
            AIMER_EXPORTED_GUEST.allocate(length, alignment)
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn aimer_dealloc(pointer: i32, length: i32, alignment: i32) -> i32 {
            AIMER_EXPORTED_GUEST.deallocate(pointer, length, alignment)
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn aimer_manifest(pointer: i32, capacity: i32) -> i64 {
            AIMER_EXPORTED_GUEST.manifest(pointer, capacity)
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn aimer_build(pointer: i32, capacity: i32) -> i64 {
            AIMER_EXPORTED_GUEST.build(pointer, capacity)
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn aimer_diagnostic(pointer: i32, capacity: i32) -> i64 {
            AIMER_EXPORTED_GUEST.diagnostic(pointer, capacity)
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn aimer_dispatch_event(
            event_pointer: i32,
            event_length: i32,
            output_pointer: i32,
            output_capacity: i32,
        ) -> i64 {
            AIMER_EXPORTED_GUEST.dispatch_event(
                event_pointer,
                event_length,
                output_pointer,
                output_capacity,
            )
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn aimer_poll_async(pointer: i32, capacity: i32) -> i64 {
            AIMER_EXPORTED_GUEST.poll_async(pointer, capacity)
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn aimer_async_ready() -> i64 {
            AIMER_EXPORTED_GUEST.async_ready()
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn aimer_dispatch_async_event(
            event_pointer: i32,
            event_length: i32,
            output_pointer: i32,
            output_capacity: i32,
        ) -> i64 {
            AIMER_EXPORTED_GUEST.dispatch_async_event(
                event_pointer,
                event_length,
                output_pointer,
                output_capacity,
            )
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn aimer_export_state(pointer: i32, capacity: i32) -> i64 {
            AIMER_EXPORTED_GUEST.export_state(pointer, capacity)
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn aimer_import_state(pointer: i32, length: i32) -> i64 {
            AIMER_EXPORTED_GUEST.import_state(pointer, length)
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn aimer_migrate_state(
            state_pointer: i32,
            state_length: i32,
            output_pointer: i32,
            output_capacity: i32,
        ) -> i64 {
            AIMER_EXPORTED_GUEST.migrate_state(
                state_pointer,
                state_length,
                output_pointer,
                output_capacity,
            )
        }
    };
}

#[cfg(test)]
mod tests {
    mod diagnostic_exports {
        use crate::{GuestAdapter, GuestError, GuestLimits, GuestProgram};
        use aimer_anteros::{
            AbiResult, AbiStatus, GuestDiagnostic, GuestDiagnosticCategory, GuestOperation,
            ModelLimits,
        };

        #[derive(Default)]
        struct FailingProgram;

        fn unsupported_widget_error() -> GuestError {
            GuestError::with_diagnostic(
                AbiStatus::ApplicationError,
                GuestDiagnostic::new(
                    GuestOperation::Unknown,
                    GuestDiagnosticCategory::UnsupportedWidget,
                    "widget has no guest lowering",
                )
                .with_widget("Container")
                .with_source(aimer_anteros::StableId128::from_bytes([0xCD; 16])),
            )
        }

        impl GuestProgram for FailingProgram {
            fn manifest(&self, _limits: ModelLimits) -> Result<Vec<u8>, GuestError> {
                Err(unsupported_widget_error())
            }

            fn build(&mut self, _limits: ModelLimits) -> Result<Vec<u8>, GuestError> {
                Err(unsupported_widget_error())
            }

            fn dispatch_event(
                &mut self,
                _event: &aimer_anteros::CallbackEventView<'_>,
                _limits: ModelLimits,
            ) -> Result<Option<Vec<u8>>, GuestError> {
                Err(unsupported_widget_error())
            }

            fn export_state(&self, _limits: ModelLimits) -> Result<Vec<u8>, GuestError> {
                Err(unsupported_widget_error())
            }

            fn import_state(
                &mut self,
                _state: &aimer_anteros::StateBundleView<'_>,
            ) -> Result<(), GuestError> {
                Err(unsupported_widget_error())
            }
        }

        #[test]
        fn adapter_adds_the_failing_operation_to_guest_diagnostics() {
            let limits = GuestLimits::new(ModelLimits::new(4_096, 16, 64, 64), 4, 4_096, 16);
            let mut adapter = GuestAdapter::new(FailingProgram, limits).unwrap();

            let error = adapter.build().unwrap_err();

            let diagnostic = error.diagnostic().unwrap();
            assert_eq!(diagnostic.operation(), GuestOperation::Build);
            assert_eq!(
                diagnostic.category(),
                GuestDiagnosticCategory::UnsupportedWidget
            );
            assert_eq!(diagnostic.widget(), Some("Container"));
            assert!(
                error
                    .to_string()
                    .contains("aimer_build: unsupported widget Container at source")
            );
        }

        // The inline unit-test binary contains the guest-contract fixture too,
        // so the export macro's fixed `#[no_mangle]` names would collide. These
        // uniquely named wrappers exercise the same exported guest bridge.
        static AIMER_DIAGNOSTIC_EXPORTED_GUEST: crate::ExportedGuest<FailingProgram> =
            crate::ExportedGuest::new(GuestLimits::new(
                ModelLimits::new(4_096, 16, 64, 64),
                4,
                4_096,
                16,
            ));

        #[unsafe(no_mangle)]
        pub extern "C" fn aimer_diagnostic_build(pointer: i32, capacity: i32) -> i64 {
            AIMER_DIAGNOSTIC_EXPORTED_GUEST.build(pointer, capacity)
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn aimer_diagnostic_output(pointer: i32, capacity: i32) -> i64 {
            AIMER_DIAGNOSTIC_EXPORTED_GUEST.diagnostic(pointer, capacity)
        }

        #[test]
        fn raw_build_error_exposes_a_bounded_diagnostic_output() {
            let result = AbiResult::from_packed(aimer_diagnostic_build(0, 0)).unwrap();
            assert_eq!(result.status(), AbiStatus::ApplicationError);
            assert_eq!(result.value(), 0);

            let probe = AbiResult::from_packed(aimer_diagnostic_output(0, 0)).unwrap();
            assert_eq!(probe.status(), AbiStatus::BufferTooSmall);
            assert!(probe.value() > 0);
        }
    }

    mod guest_contract {
        use crate::{CallbackRegistry, GuestAdapter, GuestError, GuestLimits, GuestProgram};
        use aimer_anteros::{
            AbiStatus, AbiVersion, ApplicationManifest, AsyncCallbackEvent, AsyncCallbackEventView,
            AsyncTaskId, CALLBACK_EVENT_FORMAT_VERSION, CURRENT_ABI_VERSION, CallbackBinding,
            CallbackEvent, CallbackEventView, EventId, ManifestView, ModelLimits,
            STATE_FORMAT_VERSION, StableId128, StateBundle, StateBundleView, StateEntry,
            StatePolicy, Version, WIDGET_IR_FORMAT_VERSION, WidgetDocument, WidgetDocumentView,
            WidgetNode, WidgetSchemaId,
        };

        const LIMITS: ModelLimits = ModelLimits::new(1_024, 16, 64, 64).max_widget_depth(8);
        const PROGRAM_ID: StableId128 = StableId128::from_bytes([0x11; 16]);
        const CALLBACK_ID: StableId128 = StableId128::from_bytes([0x22; 16]);
        const STATE_ID: StableId128 = StableId128::from_bytes([0x33; 16]);
        const SCHEMA_ID: StableId128 = StableId128::from_bytes([0x44; 16]);
        const VERSION: Version = Version::new(1, 0);

        struct CounterProgram {
            value: u8,
            callbacks: CallbackRegistry<u8>,
        }

        impl Default for CounterProgram {
            fn default() -> Self {
                let mut callbacks = CallbackRegistry::<u8>::new().max_callbacks(1);
                callbacks
                    .register(CALLBACK_ID, |value, _event| {
                        *value = value.saturating_add(1);
                        Ok(true)
                    })
                    .unwrap();
                Self {
                    value: 0,
                    callbacks,
                }
            }
        }

        impl GuestProgram for CounterProgram {
            fn manifest(&self, limits: ModelLimits) -> Result<Vec<u8>, GuestError> {
                ApplicationManifest::new(
                    AbiVersion::new(1, 0),
                    AbiVersion::new(1, 0),
                    WIDGET_IR_FORMAT_VERSION,
                    CALLBACK_EVENT_FORMAT_VERSION,
                    STATE_FORMAT_VERSION,
                    PROGRAM_ID,
                    &[],
                )
                .encode(limits)
                .map_err(GuestError::from_model)
            }

            fn build(&mut self, limits: ModelLimits) -> Result<Vec<u8>, GuestError> {
                widget_image(self.value, limits)
            }

            fn dispatch_event(
                &mut self,
                event: &CallbackEventView<'_>,
                limits: ModelLimits,
            ) -> Result<Option<Vec<u8>>, GuestError> {
                let rebuild = self.callbacks.dispatch(&mut self.value, event)?;
                rebuild
                    .then(|| widget_image(self.value, limits))
                    .transpose()
            }

            fn poll_async(&mut self, limits: ModelLimits) -> Result<Option<Vec<u8>>, GuestError> {
                widget_image(self.value, limits).map(Some)
            }

            fn dispatch_async_event(
                &mut self,
                event: &AsyncCallbackEventView<'_>,
                _limits: ModelLimits,
            ) -> Result<Option<Vec<u8>>, GuestError> {
                assert_eq!(event.task_id(), AsyncTaskId::new(11));
                Ok(None)
            }

            fn export_state(&self, limits: ModelLimits) -> Result<Vec<u8>, GuestError> {
                let entries = [StateEntry::new(
                    STATE_ID,
                    SCHEMA_ID,
                    VERSION,
                    StatePolicy::Required,
                    std::slice::from_ref(&self.value),
                )];
                StateBundle::new(PROGRAM_ID, 7, &entries)
                    .encode(limits)
                    .map_err(GuestError::from_model)
            }

            fn import_state(&mut self, state: &StateBundleView<'_>) -> Result<(), GuestError> {
                if state.application_id() != PROGRAM_ID || state.entry_count() != 1 {
                    return Err(GuestError::new(AbiStatus::StateIncompatible));
                }
                let entry = state.entry(0).unwrap();
                if entry.state_id() != STATE_ID || entry.payload().len() != 1 {
                    return Err(GuestError::new(AbiStatus::StateIncompatible));
                }
                self.value = entry.payload()[0];
                Ok(())
            }

            fn migrate_state(
                &mut self,
                state: &StateBundleView<'_>,
                limits: ModelLimits,
            ) -> Result<Vec<u8>, GuestError> {
                let entries = state
                    .entries()
                    .map(|entry| {
                        StateEntry::new(
                            entry.state_id(),
                            entry.schema_id(),
                            entry.schema_version(),
                            entry.policy(),
                            entry.payload(),
                        )
                    })
                    .collect::<Vec<_>>();
                StateBundle::new(state.application_id(), state.source_generation(), &entries)
                    .encode(limits)
                    .map_err(GuestError::from_model)
            }
        }

        #[test]
        fn registered_guest_produces_manifest_widget_callback_and_state_documents() {
            let guest_limits = GuestLimits::new(LIMITS, 2, 2_048, 16);
            let mut guest = GuestAdapter::new(CounterProgram::default(), guest_limits).unwrap();

            let manifest = guest.manifest().unwrap();
            assert_eq!(
                ManifestView::decode(&manifest, LIMITS)
                    .unwrap()
                    .program_id(),
                PROGRAM_ID
            );

            let initial = guest.build().unwrap();
            assert_eq!(
                WidgetDocumentView::decode(&initial, LIMITS)
                    .unwrap()
                    .document_revision(),
                0
            );

            let event = CallbackEvent::new(7, 1, CALLBACK_ID, EventId::new(1), VERSION, 1, &[])
                .encode(LIMITS)
                .unwrap();
            let updated = guest.dispatch_event(&event).unwrap().unwrap();
            assert_eq!(
                WidgetDocumentView::decode(&updated, LIMITS)
                    .unwrap()
                    .document_revision(),
                1
            );

            let state = guest.export_state().unwrap();
            assert_eq!(
                StateBundleView::decode(&state, LIMITS)
                    .unwrap()
                    .entry(0)
                    .unwrap()
                    .payload(),
                [1]
            );
            let migrated = guest.migrate_state(&state).unwrap();
            assert_eq!(migrated, state);
            guest.import_state(&state).unwrap();
        }

        #[test]
        fn adapter_polls_guest_async_work_and_validates_completion_documents() {
            let guest_limits = GuestLimits::new(LIMITS, 2, 2_048, 16);
            let mut guest = GuestAdapter::new(CounterProgram::default(), guest_limits).unwrap();
            let event = AsyncCallbackEvent::complete(7, 1, CALLBACK_ID, AsyncTaskId::new(11), &[])
                .encode(LIMITS)
                .unwrap();

            assert!(guest.dispatch_async_event(&event).unwrap().is_none());
            let image = guest.poll_async().unwrap().unwrap();
            assert_eq!(
                WidgetDocumentView::decode(&image, LIMITS)
                    .unwrap()
                    .generation_id(),
                7
            );
        }

        #[test]
        fn callback_registry_rejects_duplicate_and_unknown_callback_identities() {
            let mut callbacks = CallbackRegistry::<u8>::new().max_callbacks(1);
            callbacks
                .register(CALLBACK_ID, |_state, _event| Ok(false))
                .unwrap();
            assert_eq!(
                callbacks.register(CALLBACK_ID, |_state, _event| Ok(false)),
                Err(GuestError::new(AbiStatus::DuplicateId))
            );

            let unknown = CallbackEvent::new(
                7,
                1,
                StableId128::from_bytes([0xFF; 16]),
                EventId::new(1),
                VERSION,
                1,
                &[],
            )
            .encode(LIMITS)
            .unwrap();
            let unknown = CallbackEventView::decode(&unknown, LIMITS).unwrap();
            assert_eq!(
                callbacks.dispatch(&mut 0, &unknown),
                Err(GuestError::new(AbiStatus::UnknownId))
            );
        }

        #[test]
        fn adapter_rejects_malformed_program_output_and_invalid_limits() {
            struct Malformed;

            impl GuestProgram for Malformed {
                fn manifest(&self, _limits: ModelLimits) -> Result<Vec<u8>, GuestError> {
                    Ok(b"not a manifest".to_vec())
                }

                fn build(&mut self, _limits: ModelLimits) -> Result<Vec<u8>, GuestError> {
                    Ok(b"not widget ir".to_vec())
                }

                fn dispatch_event(
                    &mut self,
                    _event: &CallbackEventView<'_>,
                    _limits: ModelLimits,
                ) -> Result<Option<Vec<u8>>, GuestError> {
                    Ok(None)
                }

                fn export_state(&self, _limits: ModelLimits) -> Result<Vec<u8>, GuestError> {
                    Ok(b"not state".to_vec())
                }

                fn import_state(&mut self, _state: &StateBundleView<'_>) -> Result<(), GuestError> {
                    Ok(())
                }
            }

            let limits = GuestLimits::new(LIMITS, 1, 1_024, 16);
            let mut guest = GuestAdapter::new(Malformed, limits).unwrap();
            let manifest_error = guest.manifest().unwrap_err();
            assert_eq!(manifest_error.status(), AbiStatus::MalformedMessage);
            assert_eq!(
                manifest_error.diagnostic().unwrap().operation(),
                aimer_anteros::GuestOperation::Manifest
            );
            let build_error = guest.build().unwrap_err();
            assert_eq!(build_error.status(), AbiStatus::MalformedMessage);
            assert_eq!(
                build_error.diagnostic().unwrap().operation(),
                aimer_anteros::GuestOperation::Build
            );
            assert!(GuestAdapter::new(Malformed, GuestLimits::new(LIMITS, 0, 0, 3)).is_err());
            assert_eq!(CURRENT_ABI_VERSION.to_packed(), 1_i64 << 32);
        }

        fn widget_image(value: u8, limits: ModelLimits) -> Result<Vec<u8>, GuestError> {
            let callbacks = [CallbackBinding::new(EventId::new(1), VERSION, CALLBACK_ID)];
            let nodes = [WidgetNode::new(WidgetSchemaId::new(1), VERSION).callbacks(&callbacks)];
            WidgetDocument::new(7, u64::from(value), 0, &nodes, &[], &[])
                .encode(limits)
                .map_err(GuestError::from_model)
        }

        crate::export_guest!(CounterProgram, GuestLimits::new(LIMITS, 4, 4_096, 16));

        #[test]
        fn generated_exports_expose_the_exact_version_probe_and_argument_contract() {
            assert_eq!(aimer_abi_version(), CURRENT_ABI_VERSION.to_packed());
            let initialized = aimer_anteros::AbiResult::from_packed(aimer_initialize(9)).unwrap();
            assert_eq!(initialized.status(), AbiStatus::Ok);
            assert_eq!(initialized.value(), 0);

            let manifest_probe =
                aimer_anteros::AbiResult::from_packed(aimer_manifest(0, 0)).unwrap();
            assert_eq!(manifest_probe.status(), AbiStatus::BufferTooSmall);
            assert!(manifest_probe.value() > 0);
            let interleaved_build =
                aimer_anteros::AbiResult::from_packed(aimer_build(0, 0)).unwrap();
            assert_eq!(interleaved_build.status(), AbiStatus::InvalidArgument);

            let allocation = aimer_anteros::AbiResult::from_packed(aimer_alloc(0, 1)).unwrap();
            assert_eq!(allocation.status(), AbiStatus::InvalidArgument);
            assert_eq!(aimer_dealloc(0, 1, 1), AbiStatus::InvalidArgument as i32);
            let state_import =
                aimer_anteros::AbiResult::from_packed(aimer_import_state(0, 0)).unwrap();
            assert_eq!(state_import.status(), AbiStatus::InvalidArgument);
        }
    }

    #[cfg(feature = "wasm-runtime-tests")]
    mod wasm_runtime {
        use std::fs;

        use aimer_anteros::{
            CallbackEvent, EventId, ModelLimits, Runtime, RuntimeConfig, StableId128,
            StateBundleView, Version,
        };

        const LIMITS: ModelLimits = ModelLimits::new(4_096, 32, 128, 128).max_widget_depth(16);
        const PROGRAM_ID: StableId128 = StableId128::from_bytes([0x11; 16]);
        const CALLBACK_ID: StableId128 = StableId128::from_bytes([0x22; 16]);

        #[test]
        fn compiled_guest_runs_manifest_widget_callback_and_state_transfer_through_anteros() {
            let artifact = std::env::var_os("AIMER_WASM_GUEST_FIXTURE").expect(
                "AIMER_WASM_GUEST_FIXTURE must name the compiled stateful_guest.wasm artifact",
            );
            let module = fs::read(artifact).unwrap();
            let runtime = Runtime::new(
                RuntimeConfig::new()
                    .fuel_per_call(10_000_000)
                    .max_module_bytes(16 * 1_024 * 1_024)
                    .max_memory_pages(64)
                    .max_table_elements(1_024)
                    .max_call_depth(256),
            );
            let mut active = runtime.instantiate(&module).unwrap();

            assert_eq!(
                active.manifest(LIMITS).unwrap().view().program_id(),
                PROGRAM_ID
            );
            assert_eq!(active.build(LIMITS).unwrap().view().document_revision(), 0);

            let event = CallbackEvent::new(
                7,
                1,
                CALLBACK_ID,
                EventId::new(1),
                Version::new(1, 0),
                1,
                &[],
            )
            .encode(LIMITS)
            .unwrap();
            let updated = active.dispatch_event(&event, LIMITS).unwrap().unwrap();
            assert_eq!(updated.view().document_revision(), 1);

            let state = active.export_state(LIMITS).unwrap();
            assert_eq!(
                StateBundleView::decode(state.as_bytes(), LIMITS)
                    .unwrap()
                    .entry(0)
                    .unwrap()
                    .payload(),
                [1]
            );
            let mut candidate = runtime.instantiate(&module).unwrap();
            assert!(candidate.supports_state_migration());
            let migrated = candidate.migrate_state(state.as_bytes(), LIMITS).unwrap();
            assert_eq!(migrated.as_bytes(), state.as_bytes());
            candidate.import_state(state.as_bytes(), LIMITS).unwrap();
            assert_eq!(
                candidate.build(LIMITS).unwrap().view().document_revision(),
                1
            );
        }
    }
}
