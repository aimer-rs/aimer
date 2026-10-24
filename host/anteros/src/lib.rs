//! Portable application contracts and the development-only interpreted runtime.

#[path = "protocol/abi.rs"]
mod abi;
#[path = "portable/adapter.rs"]
mod adapter;
#[path = "protocol/async_callback.rs"]
mod async_callback;
#[path = "capability/capability.rs"]
mod capability;
#[path = "protocol/codec.rs"]
mod codec;
#[path = "protocol/diagnostic.rs"]
mod diagnostic;
#[path = "protocol/event.rs"]
mod event;
#[cfg(feature = "host-runtime")]
#[path = "runtime/generation.rs"]
mod generation;
#[path = "protocol/guest_panic.rs"]
mod guest_panic;
#[path = "protocol/identity.rs"]
mod identity;
#[path = "protocol/manifest.rs"]
mod manifest;
#[path = "portable/materializer.rs"]
mod materializer;
#[path = "protocol/model.rs"]
mod model;
#[path = "portable/portable_schema.rs"]
mod portable_schema;
#[cfg(feature = "host-runtime")]
#[path = "runtime/reload.rs"]
mod reload;
#[cfg(feature = "wasm-hot-reload")]
#[path = "runtime/runtime.rs"]
mod runtime;
#[path = "protocol/schema_identity.rs"]
mod schema_identity;
#[path = "protocol/state.rs"]
mod state;
#[cfg(feature = "host-runtime")]
#[path = "runtime/state_transfer.rs"]
mod state_transfer;
#[path = "portable/widget_assembly.rs"]
mod widget_assembly;
#[path = "portable/widget_ir.rs"]
mod widget_ir;
#[path = "portable/widget_schema.rs"]
mod widget_schema;

pub use abi::{AbiResult, AbiStatus, AbiVersion, CURRENT_ABI_VERSION, UnknownAbiStatus};
pub use adapter::{AdapterError, NativeAdapter, WasmAdapter};
pub use async_callback::{
    ASYNC_CALLBACK_EVENT_FORMAT_VERSION, AsyncCallbackEvent, AsyncCallbackEventKind,
    AsyncCallbackEventView, AsyncTaskId,
};
#[cfg(target_arch = "wasm32")]
pub use capability::WasmCapabilityTransport;
pub use capability::{
    CapabilityBindings, CapabilityCall, CapabilityCompletionToken, CapabilityDecoder,
    CapabilityDescriptor, CapabilityEncoder, CapabilityError, CapabilityGeneration,
    CapabilityLimits, CapabilityProvider, CapabilityRegistry, CapabilityRegistryError,
    CapabilityResult, CapabilityStagingClass, CapabilityTransport, GenerationId,
    PORTABLE_HTTP_CAPABILITY_ABI_MAJOR, PORTABLE_HTTP_CAPABILITY_NAME,
    PORTABLE_HTTP_MAX_REQUEST_BYTES, PORTABLE_HTTP_MAX_RESPONSE_BYTES, PORTABLE_HTTP_METHOD_GET,
    PORTABLE_HTTP_RESPONSE_OVERHEAD_BYTES, StagedCapability, capability_contract_fingerprint,
    portable_http_capability_id, portable_http_contract_fingerprint, portable_http_max_body_bytes,
};
pub use codec::{
    CanonicalDecoder, CanonicalEncoder, DecodeError, DecodeLimits, EncodeError, EncodeLimits,
    Envelope, Version,
};
pub use diagnostic::{
    GuestDiagnostic, GuestDiagnosticCategory, GuestDiagnosticDecodeError,
    GuestDiagnosticEncodeError, GuestOperation, GuestSourceLocation, MAX_GUEST_DIAGNOSTIC_BYTES,
};
pub use event::{CALLBACK_EVENT_FORMAT_VERSION, CallbackEvent, CallbackEventView};
#[cfg(feature = "host-runtime")]
pub use generation::{
    AsyncCallbackError, CallbackBindingError, CallbackBindingSnapshot, Generation,
    GenerationAsyncLimits, GenerationCompletionToken, GenerationHandle, GenerationLimits,
    GenerationResource, GenerationResourceError, GenerationResourceKind,
};
pub use guest_panic::{GuestPanicContext, GuestPanicRecord, GuestPanicScope, capture_guest_panic};
pub use identity::{IdentityKind, StableId128};
pub use manifest::{
    ApplicationManifest, CapabilityPolicy, CapabilityRequirement, CapabilityRequirementView,
    CapabilityRequirements, ManifestView,
};
pub use materializer::{WidgetFactory, WidgetMaterializeError, materialize_widget_tree};
pub use model::{ModelError, ModelLimits};
pub use portable_schema::{
    AsyncCallbackSchemaMetadata, CallbackSchemaMetadata, ChildCardinality,
    PortableWidgetSchemaMetadata, PortableWidgetSchemaMetadataError, PortableWidgetSchemaValidator,
    PropertyPresence, PropertySchemaMetadata, PropertyValueKind, ValueSchemaMetadata,
    validate_portable_widget_schema_metadata,
};
#[cfg(feature = "host-runtime")]
pub use reload::{
    ReloadCommit, ReloadCommitError, ReloadCoordinator, ReloadEventDisposition,
    ReloadEventOverflow, ReloadGuest, ReloadRejection, ReloadReplay, ReloadReplayFailure,
    ReloadReplayReport, ReloadSnapshot, ReloadStage, ReloadTransactionError, ReloadTransactionId,
};

#[cfg(feature = "wasm-hot-reload")]
pub use runtime::{
    GuestInstance, ManifestImage, Runtime, RuntimeConfig, RuntimeError, RuntimeErrorKind,
    StateImage, WidgetImage,
};
pub use schema_identity::{
    EventId, PropertyId, ValueTypeId, WidgetSchemaId, WidgetSchemaMetadata,
    WidgetSchemaMetadataError, stable_schema_hash64, validate_widget_schema_metadata,
};
pub use state::{
    STATE_FORMAT_VERSION, StateBundle, StateBundleView, StateEntries, StateEntry, StateEntryView,
    StatePolicy,
};
#[cfg(feature = "host-runtime")]
pub use state_transfer::{
    PreparedStateTransfer, StateMigration, StateMigrationFailure, StateTransferCoordinator,
    StateTransferError, StateTransferReport, StateTransferStage,
};
pub use widget_assembly::{
    AssemblyErrorKind, WidgetAssemblyDocument, WidgetAssemblyError, disassemble_widget_document,
};
pub use widget_ir::{
    CallbackBinding, CallbackBindings, ChildIndices, PropertyValue, WIDGET_IR_FORMAT_VERSION,
    WidgetDocument, WidgetDocumentView, WidgetNode, WidgetNodeView, WidgetProperties,
    WidgetProperty, WidgetSchemaSupport,
};
pub use widget_schema::{
    BOX_DECORATION_VALUE_MAXIMUM_ENCODED_BYTES, BOX_DECORATION_VALUE_NAME,
    BOX_DECORATION_VALUE_VERSION, BUILTIN_PORTABLE_WIDGET_SCHEMAS, BUILTIN_WIDGET_SCHEMA_VERSION,
    EVENT_BUTTON_DOUBLE_PRESS, EVENT_BUTTON_DOUBLE_PRESS_NAME, EVENT_BUTTON_LONG_PRESS,
    EVENT_BUTTON_LONG_PRESS_NAME, EVENT_BUTTON_PRESS, EVENT_BUTTON_PRESS_NAME,
    EVENT_BUTTON_RIGHT_PRESS, EVENT_BUTTON_RIGHT_PRESS_NAME, EVENT_GESTURE_DETECTOR_DOUBLE_PRESS,
    EVENT_GESTURE_DETECTOR_DOUBLE_PRESS_NAME, EVENT_GESTURE_DETECTOR_DRAG_END,
    EVENT_GESTURE_DETECTOR_DRAG_END_NAME, EVENT_GESTURE_DETECTOR_LONG_PRESS,
    EVENT_GESTURE_DETECTOR_LONG_PRESS_NAME, EVENT_GESTURE_DETECTOR_RIGHT_TAP,
    EVENT_GESTURE_DETECTOR_RIGHT_TAP_NAME, EVENT_GESTURE_DETECTOR_TAP,
    EVENT_GESTURE_DETECTOR_TAP_NAME, EVENT_MOUSE_REGION_HOVER_ENTER,
    EVENT_MOUSE_REGION_HOVER_ENTER_NAME, EVENT_MOUSE_REGION_HOVER_EXIT,
    EVENT_MOUSE_REGION_HOVER_EXIT_NAME, LAYOUT_SPACING_VALUE_MAXIMUM_ENCODED_BYTES,
    LAYOUT_SPACING_VALUE_NAME, LAYOUT_SPACING_VALUE_VERSION,
    LINE_HEIGHT_VALUE_MAXIMUM_ENCODED_BYTES, LINE_HEIGHT_VALUE_NAME, LINE_HEIGHT_VALUE_VERSION,
    PROPERTY_ANIMATED_THEME_CURVE, PROPERTY_ANIMATED_THEME_CURVE_NAME,
    PROPERTY_ANIMATED_THEME_CURVE_X1, PROPERTY_ANIMATED_THEME_CURVE_X1_NAME,
    PROPERTY_ANIMATED_THEME_CURVE_X2, PROPERTY_ANIMATED_THEME_CURVE_X2_NAME,
    PROPERTY_ANIMATED_THEME_CURVE_Y1, PROPERTY_ANIMATED_THEME_CURVE_Y1_NAME,
    PROPERTY_ANIMATED_THEME_CURVE_Y2, PROPERTY_ANIMATED_THEME_CURVE_Y2_NAME,
    PROPERTY_ANIMATED_THEME_DURATION_MILLIS, PROPERTY_ANIMATED_THEME_DURATION_MILLIS_NAME,
    PROPERTY_ANIMATED_THEME_MODE, PROPERTY_ANIMATED_THEME_MODE_NAME,
    PROPERTY_ANIMATED_THEME_SCHEMA_VERSION, PROPERTY_ANIMATED_THEME_SCHEMA_VERSION_NAME,
    PROPERTY_ANIMATED_THEME_TYPE, PROPERTY_ANIMATED_THEME_TYPE_NAME, PROPERTY_ANIMATED_THEME_VALUE,
    PROPERTY_ANIMATED_THEME_VALUE_NAME, PROPERTY_BUTTON_DECORATION,
    PROPERTY_BUTTON_DECORATION_NAME, PROPERTY_COLUMN_GAPS, PROPERTY_COLUMN_GAPS_NAME,
    PROPERTY_COLUMN_HORIZONTAL_ALIGNMENT, PROPERTY_COLUMN_HORIZONTAL_ALIGNMENT_NAME,
    PROPERTY_COLUMN_JUSTIFY_CONTENT, PROPERTY_COLUMN_JUSTIFY_CONTENT_NAME,
    PROPERTY_COLUMN_OVERFLOW, PROPERTY_COLUMN_OVERFLOW_NAME, PROPERTY_COLUMN_VERTICAL_ALIGNMENT,
    PROPERTY_COLUMN_VERTICAL_ALIGNMENT_NAME, PROPERTY_CONTAINER_BOX_DECORATION,
    PROPERTY_CONTAINER_BOX_DECORATION_NAME, PROPERTY_CONTAINER_COLOR,
    PROPERTY_CONTAINER_COLOR_NAME, PROPERTY_CONTAINER_HEIGHT, PROPERTY_CONTAINER_HEIGHT_NAME,
    PROPERTY_CONTAINER_MARGIN, PROPERTY_CONTAINER_MARGIN_NAME, PROPERTY_CONTAINER_PADDING,
    PROPERTY_CONTAINER_PADDING_NAME, PROPERTY_CONTAINER_WIDTH, PROPERTY_CONTAINER_WIDTH_NAME,
    PROPERTY_PROVIDER_SCHEMA_VERSION, PROPERTY_PROVIDER_SCHEMA_VERSION_NAME,
    PROPERTY_PROVIDER_TYPE, PROPERTY_PROVIDER_TYPE_NAME, PROPERTY_PROVIDER_VALUE,
    PROPERTY_PROVIDER_VALUE_NAME, PROPERTY_ROW_GAPS, PROPERTY_ROW_GAPS_NAME,
    PROPERTY_ROW_HORIZONTAL_ALIGNMENT, PROPERTY_ROW_HORIZONTAL_ALIGNMENT_NAME,
    PROPERTY_ROW_JUSTIFY_CONTENT, PROPERTY_ROW_JUSTIFY_CONTENT_NAME, PROPERTY_ROW_OVERFLOW,
    PROPERTY_ROW_OVERFLOW_NAME, PROPERTY_ROW_VERTICAL_ALIGNMENT,
    PROPERTY_ROW_VERTICAL_ALIGNMENT_NAME, PROPERTY_SIZED_BOX_HEIGHT,
    PROPERTY_SIZED_BOX_HEIGHT_NAME, PROPERTY_SIZED_BOX_WIDTH, PROPERTY_SIZED_BOX_WIDTH_NAME,
    PROPERTY_TEXT_ALIGN, PROPERTY_TEXT_ALIGN_NAME, PROPERTY_TEXT_CONTENT,
    PROPERTY_TEXT_CONTENT_NAME, PROPERTY_TEXT_INDENT, PROPERTY_TEXT_INDENT_NAME,
    PROPERTY_TEXT_LINE_HEIGHT, PROPERTY_TEXT_LINE_HEIGHT_NAME, PROPERTY_TEXT_STYLE,
    PROPERTY_TEXT_STYLE_NAME, PROVIDER_VALUE_MAXIMUM_ENCODED_BYTES,
    TEXT_STYLE_VALUE_MAXIMUM_ENCODED_BYTES, TEXT_STYLE_VALUE_NAME, TEXT_STYLE_VALUE_VERSION,
    THEME_DATA_VALUE_MAXIMUM_ENCODED_BYTES, THEME_DATA_VALUE_NAME, THEME_DATA_VALUE_VERSION,
    THEME_VALUE_MAXIMUM_ENCODED_BYTES, WIDGET_ANIMATED_THEME, WIDGET_ANIMATED_THEME_NAME,
    WIDGET_BUTTON, WIDGET_BUTTON_NAME, WIDGET_COLUMN, WIDGET_COLUMN_NAME, WIDGET_CONTAINER,
    WIDGET_CONTAINER_NAME, WIDGET_GESTURE_DETECTOR, WIDGET_GESTURE_DETECTOR_NAME,
    WIDGET_MOUSE_REGION, WIDGET_MOUSE_REGION_NAME, WIDGET_PROVIDER, WIDGET_PROVIDER_NAME,
    WIDGET_ROW, WIDGET_ROW_NAME, WIDGET_SIZED_BOX, WIDGET_SIZED_BOX_NAME, WIDGET_TEXT,
    WIDGET_TEXT_NAME
};

#[cfg(test)]
mod adapter_parity_tests {
    use crate::{
        AdapterError, CallbackBinding, CallbackEvent, EVENT_BUTTON_PRESS, EventId, ModelLimits,
        NativeAdapter, PROPERTY_TEXT_CONTENT, PropertyId, PropertyValue, StableId128, StateBundle,
        StateEntry, StatePolicy, Version, WIDGET_BUTTON, WIDGET_TEXT, WasmAdapter, WidgetDocument,
        WidgetDocumentView, WidgetNode, WidgetProperty, WidgetSchemaId,
    };

    const LIMITS: ModelLimits = ModelLimits::new(4_096, 32, 256, 256);
    const CALLBACK_ID: StableId128 = StableId128::from_bytes([0x31; 16]);
    const WIDGET_KEY: StableId128 = StableId128::from_bytes([0x41; 16]);
    const APPLICATION_ID: StableId128 = StableId128::from_bytes([0x51; 16]);
    const STATE_ID: StableId128 = StableId128::from_bytes([0x61; 16]);
    const SCHEMA_ID: StableId128 = StableId128::from_bytes([0x71; 16]);

    #[test]
    fn native_and_wasm_adapters_emit_identical_initial_images() {
        let native = NativeAdapter::new(LIMITS);
        let wasm = WasmAdapter::new(LIMITS);
        let properties = [WidgetProperty::new(
            PropertyId::new(1),
            PropertyValue::StringRef(0),
        )];
        let callbacks = [CallbackBinding::new(
            EventId::new(1),
            Version::new(1, 0),
            CALLBACK_ID,
        )];
        let nodes = [WidgetNode::new(WidgetSchemaId::new(7), Version::new(1, 0))
            .key(WIDGET_KEY)
            .properties(&properties)
            .callbacks(&callbacks)];
        let strings = ["Count: 0"];
        let document = WidgetDocument::new(1, 0, 0, &nodes, &strings, &[]);
        let event = CallbackEvent::new(
            1,
            1,
            CALLBACK_ID,
            EventId::new(1),
            Version::new(1, 0),
            10,
            &[0],
        )
        .widget_key(WIDGET_KEY);
        let entries = [StateEntry::new(
            STATE_ID,
            SCHEMA_ID,
            Version::new(1, 0),
            StatePolicy::Required,
            &[0],
        )];
        let state = StateBundle::new(APPLICATION_ID, 1, &entries);

        let native_widget = native.encode_widget_document(&document).unwrap();
        let native_event = native.encode_callback_event(event).unwrap();
        let native_state = native.encode_state_bundle(&state).unwrap();

        assert_eq!(write_widget_to_guest(&wasm, &document), native_widget,);
        assert_eq!(write_event_to_guest(&wasm, event), native_event);
        assert_eq!(write_state_to_guest(&wasm, &state), native_state);
        assert_eq!(&native_widget[..8], b"AWIR\x02\x00\x00\x00");
        assert_eq!(&native_event[..8], b"AEVT\x02\x00\x00\x00");
        assert_eq!(&native_state[..8], b"ASTA\x01\x00\x00\x00");
    }

    #[test]
    fn native_and_wasm_adapters_remain_equal_for_an_ordered_update_trace() {
        let native = NativeAdapter::new(LIMITS);
        let wasm = WasmAdapter::new(LIMITS);

        for (sequence, count) in [(2_u64, 1_u8), (3, 2), (4, 3)] {
            let label = format!("Count: {count}");
            let payload = [count];
            let strings = [label.as_str()];
            let properties = [
                WidgetProperty::new(PropertyId::new(1), PropertyValue::StringRef(0)),
                WidgetProperty::new(PropertyId::new(2), PropertyValue::I64(i64::from(count))),
            ];
            let callbacks = [CallbackBinding::new(
                EventId::new(1),
                Version::new(1, 0),
                CALLBACK_ID,
            )];
            let nodes = [WidgetNode::new(WidgetSchemaId::new(7), Version::new(1, 0))
                .key(WIDGET_KEY)
                .properties(&properties)
                .callbacks(&callbacks)];
            let document = WidgetDocument::new(1, sequence, 0, &nodes, &strings, &[]);
            let event = CallbackEvent::new(
                1,
                sequence,
                CALLBACK_ID,
                EventId::new(1),
                Version::new(1, 0),
                sequence * 10,
                &payload,
            )
            .widget_key(WIDGET_KEY);
            let entries = [StateEntry::new(
                STATE_ID,
                SCHEMA_ID,
                Version::new(1, 0),
                StatePolicy::Required,
                &payload,
            )];
            let state = StateBundle::new(APPLICATION_ID, 1, &entries);

            assert_eq!(
                write_widget_to_guest(&wasm, &document),
                native.encode_widget_document(&document).unwrap(),
            );
            assert_eq!(
                write_event_to_guest(&wasm, event),
                native.encode_callback_event(event).unwrap(),
            );
            assert_eq!(
                write_state_to_guest(&wasm, &state),
                native.encode_state_bundle(&state).unwrap(),
            );
        }
    }

    #[test]
    fn shared_counter_application_produces_equal_native_and_wasm_event_traces() {
        let native_adapter = NativeAdapter::new(LIMITS);
        let wasm_adapter = WasmAdapter::new(LIMITS);
        let mut native_application = CounterApplication::default();
        let mut wasm_application = CounterApplication::default();

        for (event, expected_revision, expected_label) in [
            (CounterEvent::Increment, 1, "Count: 1"),
            (CounterEvent::Increment, 2, "Count: 2"),
            (CounterEvent::Decrement, 3, "Count: 1"),
            (CounterEvent::Reset, 4, "Count: 0"),
        ] {
            native_application.dispatch(event);
            wasm_application.dispatch(event);

            let native_image = native_application
                .with_document(|document| native_adapter.encode_widget_document(document).unwrap());
            let wasm_image = wasm_application
                .with_document(|document| write_widget_to_guest(&wasm_adapter, document));

            assert_eq!(native_image, wasm_image);
            let view = WidgetDocumentView::decode(&native_image, LIMITS).unwrap();
            assert_eq!(view.document_revision(), expected_revision);
            assert_eq!(view.string(0), Some(expected_label));
            assert_eq!(view.node(0).unwrap().key(), Some(WIDGET_KEY));
            assert_eq!(
                view.node(0)
                    .unwrap()
                    .callbacks()
                    .next()
                    .unwrap()
                    .callback_id(),
                CALLBACK_ID
            );
        }
    }

    #[derive(Clone, Copy)]
    enum CounterEvent {
        Increment,
        Decrement,
        Reset,
    }

    #[derive(Default)]
    struct CounterApplication {
        count: u8,
        revision: u64,
    }

    impl CounterApplication {
        fn dispatch(&mut self, event: CounterEvent) {
            self.count = match event {
                CounterEvent::Increment => self.count.saturating_add(1),
                CounterEvent::Decrement => self.count.saturating_sub(1),
                CounterEvent::Reset => 0,
            };
            self.revision += 1;
        }

        fn with_document<R>(&self, use_document: impl FnOnce(&WidgetDocument<'_>) -> R) -> R {
            let label = format!("Count: {}", self.count);
            let strings = [label.as_str()];
            let children = [1];
            let properties = [WidgetProperty::new(
                PROPERTY_TEXT_CONTENT,
                PropertyValue::StringRef(0),
            )];
            let callbacks = [CallbackBinding::new(
                EVENT_BUTTON_PRESS,
                Version::new(1, 0),
                CALLBACK_ID,
            )];
            let nodes = [
                WidgetNode::new(WIDGET_BUTTON, Version::new(1, 0))
                    .key(WIDGET_KEY)
                    .children(&children)
                    .callbacks(&callbacks),
                WidgetNode::new(WIDGET_TEXT, Version::new(1, 0)).properties(&properties),
            ];
            let document = WidgetDocument::new(1, self.revision, 0, &nodes, &strings, &[]);
            use_document(&document)
        }
    }

    #[test]
    fn wasm_adapter_reports_required_capacity_without_partial_output() {
        let wasm = WasmAdapter::new(LIMITS);
        let nodes = [WidgetNode::new(WidgetSchemaId::new(1), Version::new(1, 0))];
        let document = WidgetDocument::new(1, 0, 0, &nodes, &[], &[]);
        let mut output = [0xA5; 8];

        let error = wasm
            .write_widget_document(&document, &mut output)
            .unwrap_err();

        assert_eq!(
            error,
            AdapterError::OutputTooSmall {
                required: 128,
                available: 8,
            }
        );
        assert_eq!(output, [0xA5; 8]);
    }

    fn write_widget_to_guest(adapter: &WasmAdapter, document: &WidgetDocument<'_>) -> Vec<u8> {
        let mut memory = vec![0xA5; LIMITS_BUFFER_SIZE];
        let written = adapter
            .write_widget_document(document, &mut memory)
            .unwrap();
        assert!(memory[written..].iter().all(|byte| *byte == 0xA5));
        memory.truncate(written);
        memory
    }

    fn write_event_to_guest(adapter: &WasmAdapter, event: CallbackEvent<'_>) -> Vec<u8> {
        let mut memory = vec![0xA5; LIMITS_BUFFER_SIZE];
        let written = adapter.write_callback_event(event, &mut memory).unwrap();
        assert!(memory[written..].iter().all(|byte| *byte == 0xA5));
        memory.truncate(written);
        memory
    }

    fn write_state_to_guest(adapter: &WasmAdapter, state: &StateBundle<'_>) -> Vec<u8> {
        let mut memory = vec![0xA5; LIMITS_BUFFER_SIZE];
        let written = adapter.write_state_bundle(state, &mut memory).unwrap();
        assert!(memory[written..].iter().all(|byte| *byte == 0xA5));
        memory.truncate(written);
        memory
    }

    const LIMITS_BUFFER_SIZE: usize = 4_096;
}
#[cfg(test)]
mod callback_generation_tests {
    use crate::{
        CallbackBinding, CallbackBindingError, CallbackBindingSnapshot, CallbackEvent,
        CallbackEventView, EventId, Generation, GenerationId, ModelLimits, StableId128, Version,
        WidgetDocument, WidgetDocumentView, WidgetNode, WidgetSchemaId,
    };

    const LIMITS: ModelLimits = ModelLimits::new(4_096, 64, 256, 256).max_widget_depth(16);
    const EVENT_KIND: EventId = EventId::new(7);
    const WRONG_EVENT_KIND: EventId = EventId::new(8);
    const EVENT_SCHEMA: Version = Version::new(1, 0);
    const CALLBACK_ID: StableId128 = StableId128::from_bytes([0x11; 16]);
    const WIDGET_KEY: StableId128 = StableId128::from_bytes([0x22; 16]);

    #[test]
    fn stable_callback_rebinds_to_the_replacement_generation() {
        let old_image = widget_image(41, Some((WIDGET_KEY, CALLBACK_ID)));
        let replacement_image = widget_image(42, Some((WIDGET_KEY, CALLBACK_ID)));
        let old_view = WidgetDocumentView::decode(&old_image, LIMITS).unwrap();
        let replacement_view = WidgetDocumentView::decode(&replacement_image, LIMITS).unwrap();
        let mut old_generation = Generation::new(
            GenerationId::new(41),
            CallbackBindingSnapshot::from_document(&old_view, 16).unwrap(),
        );
        let mut replacement_generation = Generation::new(
            GenerationId::new(42),
            CallbackBindingSnapshot::from_document(&replacement_view, 16).unwrap(),
        );
        let old_event = callback_event(41, 1, CALLBACK_ID, WIDGET_KEY, EVENT_KIND, EVENT_SCHEMA);
        let replacement_event =
            callback_event(42, 1, CALLBACK_ID, WIDGET_KEY, EVENT_KIND, EVENT_SCHEMA);

        let old_event = old_generation.validate_event(&old_event, LIMITS).unwrap();
        let replacement_event = replacement_generation
            .validate_event(&replacement_event, LIMITS)
            .unwrap();

        assert_eq!(old_event.callback_id(), CALLBACK_ID);
        assert_eq!(replacement_event.callback_id(), CALLBACK_ID);
    }

    #[test]
    fn active_generation_encodes_callback_metadata_from_its_binding() {
        let generation = generation(41);

        let event = generation
            .encode_callback_event(CALLBACK_ID, 9, 17, &[0xA5], LIMITS)
            .unwrap();
        let event = CallbackEventView::decode(&event, LIMITS).unwrap();

        assert_eq!(event.generation_id(), 41);
        assert_eq!(event.event_sequence(), 9);
        assert_eq!(event.callback_id(), CALLBACK_ID);
        assert_eq!(event.widget_key(), Some(WIDGET_KEY));
        assert_eq!(event.event_kind(), EVENT_KIND);
        assert_eq!(event.event_schema(), EVENT_SCHEMA);
        assert_eq!(event.monotonic_timestamp(), 17);
        assert_eq!(event.payload(), [0xA5]);
    }

    #[test]
    fn duplicate_callback_identity_rejects_the_complete_snapshot() {
        let child_indices = [1];
        let root_callbacks = [CallbackBinding::new(EVENT_KIND, EVENT_SCHEMA, CALLBACK_ID)];
        let child_callbacks = [CallbackBinding::new(EVENT_KIND, EVENT_SCHEMA, CALLBACK_ID)];
        let nodes = [
            WidgetNode::new(WidgetSchemaId::new(1), Version::new(1, 0))
                .key(WIDGET_KEY)
                .callbacks(&root_callbacks)
                .children(&child_indices),
            WidgetNode::new(WidgetSchemaId::new(2), Version::new(1, 0))
                .key(StableId128::from_bytes([0x33; 16]))
                .callbacks(&child_callbacks),
        ];
        let image = WidgetDocument::new(41, 1, 0, &nodes, &[], &[])
            .encode(LIMITS)
            .unwrap();
        let view = WidgetDocumentView::decode(&image, LIMITS).unwrap();

        let error = CallbackBindingSnapshot::from_document(&view, 16).unwrap_err();

        assert_eq!(
            error,
            CallbackBindingError::DuplicateCallbackId {
                callback_id: CALLBACK_ID,
            }
        );
    }

    #[test]
    fn replacement_rejects_an_event_for_a_removed_callback() {
        let image = widget_image(42, None);
        let view = WidgetDocumentView::decode(&image, LIMITS).unwrap();
        let mut generation = Generation::new(
            GenerationId::new(42),
            CallbackBindingSnapshot::from_document(&view, 16).unwrap(),
        );
        let event = callback_event(42, 1, CALLBACK_ID, WIDGET_KEY, EVENT_KIND, EVENT_SCHEMA);

        let error = generation.validate_event(&event, LIMITS).unwrap_err();

        assert_eq!(
            error,
            CallbackBindingError::UnknownCallback {
                callback_id: CALLBACK_ID,
            }
        );
    }

    #[test]
    fn callback_event_rejects_a_mismatched_widget_key() {
        let mut generation = generation(41);
        let wrong_key = StableId128::from_bytes([0x44; 16]);
        let event = callback_event(41, 1, CALLBACK_ID, wrong_key, EVENT_KIND, EVENT_SCHEMA);

        let error = generation.validate_event(&event, LIMITS).unwrap_err();

        assert_eq!(
            error,
            CallbackBindingError::WidgetKeyMismatch {
                callback_id: CALLBACK_ID,
                expected: Some(WIDGET_KEY),
                actual: Some(wrong_key),
            }
        );
    }

    #[test]
    fn callback_event_rejects_a_wrong_event_kind() {
        let mut generation = generation(41);
        let event = callback_event(
            41,
            1,
            CALLBACK_ID,
            WIDGET_KEY,
            WRONG_EVENT_KIND,
            EVENT_SCHEMA,
        );

        let error = generation.validate_event(&event, LIMITS).unwrap_err();

        assert_eq!(
            error,
            CallbackBindingError::EventKindMismatch {
                callback_id: CALLBACK_ID,
                expected: EVENT_KIND,
                actual: WRONG_EVENT_KIND,
            }
        );
    }

    #[test]
    fn callback_event_rejects_a_wrong_event_schema() {
        let mut generation = generation(41);
        let wrong_schema = Version::new(2, 0);
        let event = callback_event(41, 1, CALLBACK_ID, WIDGET_KEY, EVENT_KIND, wrong_schema);

        let error = generation.validate_event(&event, LIMITS).unwrap_err();

        assert_eq!(
            error,
            CallbackBindingError::EventSchemaMismatch {
                callback_id: CALLBACK_ID,
                expected: EVENT_SCHEMA,
                actual: wrong_schema,
            }
        );
    }

    #[test]
    fn callback_event_sequence_cannot_be_consumed_twice() {
        let mut generation = generation(41);
        let event = callback_event(41, 7, CALLBACK_ID, WIDGET_KEY, EVENT_KIND, EVENT_SCHEMA);
        generation.validate_event(&event, LIMITS).unwrap();

        let error = generation.validate_event(&event, LIMITS).unwrap_err();

        assert_eq!(
            error,
            CallbackBindingError::EventSequenceNotMonotonic {
                previous: 7,
                actual: 7,
            }
        );
    }

    #[test]
    fn event_from_another_generation_is_rejected() {
        let mut generation = generation(41);
        let event = callback_event(40, 1, CALLBACK_ID, WIDGET_KEY, EVENT_KIND, EVENT_SCHEMA);

        let error = generation.validate_event(&event, LIMITS).unwrap_err();

        assert_eq!(
            error,
            CallbackBindingError::GenerationMismatch {
                expected: GenerationId::new(41),
                actual: GenerationId::new(40),
            }
        );
    }

    #[test]
    fn retired_generation_rejects_new_callback_events() {
        let mut generation = generation(41);
        generation.retire();
        let event = callback_event(41, 1, CALLBACK_ID, WIDGET_KEY, EVENT_KIND, EVENT_SCHEMA);

        let error = generation.validate_event(&event, LIMITS).unwrap_err();

        assert_eq!(
            error,
            CallbackBindingError::RetiredGeneration {
                generation_id: GenerationId::new(41),
            }
        );
    }

    fn generation(generation_id: u64) -> Generation {
        let image = widget_image(generation_id, Some((WIDGET_KEY, CALLBACK_ID)));
        let view = WidgetDocumentView::decode(&image, LIMITS).unwrap();
        Generation::new(
            GenerationId::new(generation_id),
            CallbackBindingSnapshot::from_document(&view, 16).unwrap(),
        )
    }

    fn widget_image(generation_id: u64, binding: Option<(StableId128, StableId128)>) -> Vec<u8> {
        let callbacks = binding
            .map(|(_, callback_id)| [CallbackBinding::new(EVENT_KIND, EVENT_SCHEMA, callback_id)]);
        let node = binding.map_or_else(
            || WidgetNode::new(WidgetSchemaId::new(1), Version::new(1, 0)),
            |(key, _)| {
                WidgetNode::new(WidgetSchemaId::new(1), Version::new(1, 0))
                    .key(key)
                    .callbacks(callbacks.as_ref().unwrap())
            },
        );
        WidgetDocument::new(generation_id, 1, 0, &[node], &[], &[])
            .encode(LIMITS)
            .unwrap()
    }

    fn callback_event(
        generation_id: u64,
        sequence: u64,
        callback_id: StableId128,
        widget_key: StableId128,
        event_kind: EventId,
        event_schema: Version,
    ) -> Vec<u8> {
        CallbackEvent::new(
            generation_id,
            sequence,
            callback_id,
            event_kind,
            event_schema,
            123,
            &[],
        )
        .widget_key(widget_key)
        .encode(LIMITS)
        .unwrap()
    }
}
#[cfg(test)]
mod capability_registry_tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use crate::{
        AbiVersion, ApplicationManifest, CALLBACK_EVENT_FORMAT_VERSION, CapabilityCall,
        CapabilityCompletionToken, CapabilityDescriptor, CapabilityError, CapabilityGeneration,
        CapabilityLimits, CapabilityPolicy, CapabilityProvider, CapabilityRegistry,
        CapabilityRegistryError, CapabilityRequirement, CapabilityResult, CapabilityStagingClass,
        CapabilityTransport, GenerationId, ManifestView, ModelLimits, STATE_FORMAT_VERSION,
        StableId128, WIDGET_IR_FORMAT_VERSION,
    };

    const MODEL_LIMITS: ModelLimits = ModelLimits::new(512, 16, 64, 64);
    const CAPABILITY_ID: StableId128 = StableId128::from_bytes([0x20; 16]);
    const OTHER_CAPABILITY_ID: StableId128 = StableId128::from_bytes([0x21; 16]);
    const FINGERPRINT: [u8; 32] = [0x30; 32];
    const OTHER_FINGERPRINT: [u8; 32] = [0x31; 32];
    const PROVIDER_LIMITS: CapabilityLimits = CapabilityLimits::new(4, 8);

    #[test]
    fn registry_owns_and_dispatches_a_negotiated_provider() {
        let invocations = Rc::new(Cell::new(0));
        let provider = FixtureProvider::new(
            descriptor(CAPABILITY_ID, 1, FINGERPRINT),
            Rc::clone(&invocations),
            Ok(vec![9, 8]),
        );
        let mut registry = CapabilityRegistry::new(4);
        registry.register(provider).unwrap();
        let manifest = manifest(&[requirement(
            CAPABILITY_ID,
            1,
            CapabilityPolicy::Required,
            FINGERPRINT,
        )]);

        let bindings = registry.negotiate(&manifest).unwrap();
        let response = bindings
            .invoke(CapabilityCall::new(CAPABILITY_ID, 1, 7, &[1, 2], 8))
            .unwrap();

        assert_eq!(response, [9, 8]);
        assert_eq!(invocations.get(), 1);
    }

    #[test]
    fn registry_rejects_duplicate_providers_and_its_provider_limit() {
        let mut registry = CapabilityRegistry::new(1);
        registry.register(provider(CAPABILITY_ID)).unwrap();

        assert_eq!(
            registry.register(provider(CAPABILITY_ID)),
            Err(CapabilityRegistryError::DuplicateProvider {
                capability_id: CAPABILITY_ID,
            })
        );
        assert_eq!(
            registry.register(provider(OTHER_CAPABILITY_ID)),
            Err(CapabilityRegistryError::ProviderLimitExceeded { limit: 1 })
        );
    }

    #[test]
    fn required_provider_must_exist_and_match_abi_and_fingerprint() {
        let missing_registry = CapabilityRegistry::new(1);
        let missing_manifest = manifest(&[requirement(
            CAPABILITY_ID,
            1,
            CapabilityPolicy::Required,
            FINGERPRINT,
        )]);
        assert_eq!(
            missing_registry.negotiate(&missing_manifest).unwrap_err(),
            CapabilityRegistryError::MissingRequiredProvider {
                capability_id: CAPABILITY_ID,
            }
        );

        let mut registry = CapabilityRegistry::new(1);
        registry.register(provider(CAPABILITY_ID)).unwrap();
        let abi_manifest = manifest(&[requirement(
            CAPABILITY_ID,
            2,
            CapabilityPolicy::Required,
            FINGERPRINT,
        )]);
        assert_eq!(
            registry.negotiate(&abi_manifest).unwrap_err(),
            CapabilityRegistryError::AbiMismatch {
                capability_id: CAPABILITY_ID,
                required: 2,
                provided: 1,
            }
        );

        let fingerprint_manifest = manifest(&[requirement(
            CAPABILITY_ID,
            1,
            CapabilityPolicy::Required,
            OTHER_FINGERPRINT,
        )]);
        assert_eq!(
            registry.negotiate(&fingerprint_manifest).unwrap_err(),
            CapabilityRegistryError::ContractMismatch {
                capability_id: CAPABILITY_ID,
            }
        );
    }

    #[test]
    fn optional_unavailable_or_incompatible_providers_bind_as_unsupported() {
        let mut registry = CapabilityRegistry::new(1);
        registry.register(provider(CAPABILITY_ID)).unwrap();
        let manifest = manifest(&[
            requirement(CAPABILITY_ID, 2, CapabilityPolicy::Optional, FINGERPRINT),
            requirement(
                OTHER_CAPABILITY_ID,
                1,
                CapabilityPolicy::Optional,
                OTHER_FINGERPRINT,
            ),
        ]);

        let bindings = registry.negotiate(&manifest).unwrap();

        assert_eq!(
            bindings.invoke(CapabilityCall::new(CAPABILITY_ID, 2, 0, &[], 8)),
            Err(CapabilityError::Unsupported)
        );
        assert_eq!(
            bindings.invoke(CapabilityCall::new(OTHER_CAPABILITY_ID, 1, 0, &[], 8,)),
            Err(CapabilityError::Unsupported)
        );
    }

    #[test]
    fn bindings_enforce_request_and_response_limits_around_provider_dispatch() {
        let invocations = Rc::new(Cell::new(0));
        let provider = FixtureProvider::new(
            descriptor(CAPABILITY_ID, 1, FINGERPRINT),
            Rc::clone(&invocations),
            Ok(vec![0; 5]),
        );
        let mut registry = CapabilityRegistry::new(1);
        registry.register(provider).unwrap();
        let manifest = manifest(&[requirement(
            CAPABILITY_ID,
            1,
            CapabilityPolicy::Required,
            FINGERPRINT,
        )]);
        let bindings = registry.negotiate(&manifest).unwrap();

        assert_eq!(
            bindings.invoke(CapabilityCall::new(CAPABILITY_ID, 1, 0, &[0; 5], 8)),
            Err(CapabilityError::LimitExceeded)
        );
        assert_eq!(invocations.get(), 0);
        assert_eq!(
            bindings.invoke(CapabilityCall::new(CAPABILITY_ID, 1, 0, &[], 4)),
            Err(CapabilityError::LimitExceeded)
        );
        assert_eq!(invocations.get(), 1);
    }

    #[test]
    fn retired_generation_rejects_new_calls_and_late_completions() {
        let completion = Rc::new(RefCell::new(None));
        let invocations = Rc::new(Cell::new(0));
        let mut registry = CapabilityRegistry::new(1);
        registry
            .register_with_staging(
                GenerationProvider {
                    completion: Rc::clone(&completion),
                    invocations: Rc::clone(&invocations),
                },
                CapabilityStagingClass::PureQuery,
            )
            .unwrap();
        let manifest = manifest(&[requirement(
            CAPABILITY_ID,
            1,
            CapabilityPolicy::Required,
            FINGERPRINT,
        )]);
        let bindings = registry
            .negotiate_generation(&manifest, GenerationId::new(41))
            .unwrap();

        assert_eq!(bindings.generation_id(), GenerationId::new(41));
        assert_eq!(
            bindings.invoke(CapabilityCall::new(CAPABILITY_ID, 1, 0, &[], 8)),
            Ok(Vec::new())
        );
        let completion = completion.borrow_mut().take().unwrap();
        bindings.retire();

        assert_eq!(
            bindings.invoke(CapabilityCall::new(CAPABILITY_ID, 1, 0, &[], 8)),
            Err(CapabilityError::RetiredGeneration)
        );
        assert_eq!(invocations.get(), 1);
        assert_eq!(
            completion.complete(vec![1]),
            Err(CapabilityError::RetiredGeneration)
        );
    }

    #[test]
    fn dropping_bindings_rejects_late_capability_completion() {
        let completion = Rc::new(RefCell::new(None));
        let mut registry = CapabilityRegistry::new(1);
        registry
            .register_with_staging(
                GenerationProvider {
                    completion: Rc::clone(&completion),
                    invocations: Rc::new(Cell::new(0)),
                },
                CapabilityStagingClass::PureQuery,
            )
            .unwrap();
        let manifest = manifest(&[requirement(
            CAPABILITY_ID,
            1,
            CapabilityPolicy::Required,
            FINGERPRINT,
        )]);
        let bindings = registry
            .negotiate_generation(&manifest, GenerationId::new(41))
            .unwrap();
        bindings
            .invoke(CapabilityCall::new(CAPABILITY_ID, 1, 0, &[], 8))
            .unwrap();
        let completion = completion.borrow_mut().take().unwrap();

        drop(bindings);

        assert_eq!(
            completion.complete(Vec::<u8>::new()),
            Err(CapabilityError::RetiredGeneration)
        );
    }

    #[test]
    fn default_registration_is_committed_only_for_candidate_generations() {
        let mut registry = CapabilityRegistry::new(1);
        registry.register(provider(CAPABILITY_ID)).unwrap();
        let manifest = manifest(&[requirement(
            CAPABILITY_ID,
            1,
            CapabilityPolicy::Required,
            FINGERPRINT,
        )]);
        let bindings = registry
            .negotiate_generation(&manifest, GenerationId::new(41))
            .unwrap();

        assert_eq!(
            bindings.invoke(CapabilityCall::new(CAPABILITY_ID, 1, 0, &[], 8)),
            Err(CapabilityError::NotActive)
        );
        bindings.activate();
        assert_eq!(
            bindings.invoke(CapabilityCall::new(CAPABILITY_ID, 1, 0, &[], 8)),
            Ok(Vec::new())
        );
    }

    fn provider(capability_id: StableId128) -> FixtureProvider {
        FixtureProvider::new(
            descriptor(capability_id, 1, FINGERPRINT),
            Rc::new(Cell::new(0)),
            Ok(Vec::new()),
        )
    }

    fn descriptor(
        capability_id: StableId128,
        abi_major: u32,
        fingerprint: [u8; 32],
    ) -> CapabilityDescriptor {
        CapabilityDescriptor::new(capability_id, abi_major, fingerprint, PROVIDER_LIMITS)
    }

    fn requirement(
        capability_id: StableId128,
        abi_major: u32,
        policy: CapabilityPolicy,
        fingerprint: [u8; 32],
    ) -> CapabilityRequirement {
        CapabilityRequirement::new(capability_id, abi_major, policy, fingerprint)
    }

    fn manifest(requirements: &[CapabilityRequirement]) -> ManifestView<'static> {
        let bytes = ApplicationManifest::new(
            AbiVersion::new(1, 0),
            AbiVersion::new(1, 0),
            WIDGET_IR_FORMAT_VERSION,
            CALLBACK_EVENT_FORMAT_VERSION,
            STATE_FORMAT_VERSION,
            StableId128::from_bytes([0x10; 16]),
            requirements,
        )
        .encode(MODEL_LIMITS)
        .unwrap()
        .into_boxed_slice();
        ManifestView::decode(Box::leak(bytes), MODEL_LIMITS).unwrap()
    }

    struct FixtureProvider {
        descriptor: CapabilityDescriptor,
        invocations: Rc<Cell<u32>>,
        response: CapabilityResult<Vec<u8>>,
    }

    impl FixtureProvider {
        fn new(
            descriptor: CapabilityDescriptor,
            invocations: Rc<Cell<u32>>,
            response: CapabilityResult<Vec<u8>>,
        ) -> Self {
            Self {
                descriptor,
                invocations,
                response,
            }
        }
    }

    impl CapabilityProvider for FixtureProvider {
        fn descriptor(&self) -> CapabilityDescriptor {
            self.descriptor
        }

        fn invoke(
            &self,
            _generation: CapabilityGeneration,
            method_id: u32,
            request: &[u8],
            response_limit: u32,
        ) -> CapabilityResult<Vec<u8>> {
            assert!(method_id == 0 || method_id == 7);
            assert!(request == [1, 2] || request.is_empty());
            assert!(response_limit <= PROVIDER_LIMITS.max_response_bytes());
            self.invocations.set(self.invocations.get() + 1);
            self.response.clone()
        }
    }

    struct GenerationProvider {
        completion: Rc<RefCell<Option<CapabilityCompletionToken>>>,
        invocations: Rc<Cell<u32>>,
    }

    impl CapabilityProvider for GenerationProvider {
        fn descriptor(&self) -> CapabilityDescriptor {
            descriptor(CAPABILITY_ID, 1, FINGERPRINT)
        }

        fn invoke(
            &self,
            generation: CapabilityGeneration,
            _method_id: u32,
            _request: &[u8],
            _response_limit: u32,
        ) -> CapabilityResult<Vec<u8>> {
            self.invocations.set(self.invocations.get() + 1);
            self.completion.replace(Some(generation.completion_token()));
            Ok(Vec::new())
        }
    }
}
#[cfg(test)]
mod capability_staging_tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use crate::{
        AbiVersion, ApplicationManifest, CALLBACK_EVENT_FORMAT_VERSION, CapabilityCall,
        CapabilityDescriptor, CapabilityError, CapabilityGeneration, CapabilityLimits,
        CapabilityPolicy, CapabilityProvider, CapabilityRegistry, CapabilityRequirement,
        CapabilityResult, CapabilityStagingClass, CapabilityTransport, GenerationId, ManifestView,
        ModelLimits, STATE_FORMAT_VERSION, StableId128, StagedCapability, WIDGET_IR_FORMAT_VERSION,
    };

    const MODEL_LIMITS: ModelLimits = ModelLimits::new(512, 16, 64, 64);
    const CAPABILITY_ID: StableId128 = StableId128::from_bytes([0x20; 16]);
    const FINGERPRINT: [u8; 32] = [0x30; 32];
    const PROVIDER_LIMITS: CapabilityLimits = CapabilityLimits::new(8, 8);

    #[test]
    fn candidate_executes_pure_and_read_only_capabilities_immediately() {
        for staging in [
            CapabilityStagingClass::PureQuery,
            CapabilityStagingClass::ReadOnly,
        ] {
            let counters = Counters::default();
            let bindings = candidate_bindings(staging, counters.clone(), vec![1, 2]);

            let response = bindings.invoke(call()).unwrap();

            assert_eq!(response, [1, 2]);
            assert_eq!(counters.invoked.get(), 1);
            assert_eq!(counters.staged.get(), 0);
            assert_eq!(counters.activated.get(), 0);
        }
    }

    #[test]
    fn candidate_defers_registrations_and_external_requests_until_activation() {
        for staging in [
            CapabilityStagingClass::RegistrableResource,
            CapabilityStagingClass::ExternalRequest,
        ] {
            let counters = Counters::default();
            let bindings = candidate_bindings(staging, counters.clone(), vec![3, 4]);

            let response = bindings.invoke(call()).unwrap();

            assert_eq!(response, [3, 4]);
            assert_eq!(counters.invoked.get(), 0);
            assert_eq!(counters.staged.get(), 1);
            assert_eq!(counters.activated.get(), 0);

            bindings.activate();

            assert_eq!(counters.activated.get(), 1);
        }
    }

    #[test]
    fn candidate_rejects_irreversible_effects_until_activation() {
        let counters = Counters::default();
        let bindings = candidate_bindings(
            CapabilityStagingClass::IrreversibleEffect,
            counters.clone(),
            Vec::new(),
        );

        assert_eq!(bindings.invoke(call()), Err(CapabilityError::NotActive));
        assert_eq!(counters.invoked.get(), 0);

        bindings.activate();

        assert_eq!(bindings.invoke(call()), Ok(Vec::new()));
        assert_eq!(counters.invoked.get(), 1);
    }

    #[test]
    fn retiring_a_candidate_discards_staged_effects() {
        let counters = Counters::default();
        let bindings = candidate_bindings(
            CapabilityStagingClass::ExternalRequest,
            counters.clone(),
            Vec::new(),
        );
        bindings.invoke(call()).unwrap();

        bindings.retire();

        assert_eq!(counters.activated.get(), 0);
        assert_eq!(
            bindings.invoke(call()),
            Err(CapabilityError::RetiredGeneration)
        );
    }

    #[test]
    fn staged_response_limits_are_enforced_before_effect_publication() {
        let counters = Counters::default();
        let bindings = candidate_bindings(
            CapabilityStagingClass::RegistrableResource,
            counters.clone(),
            vec![0; 9],
        );

        assert_eq!(bindings.invoke(call()), Err(CapabilityError::LimitExceeded));
        bindings.activate();

        assert_eq!(counters.staged.get(), 1);
        assert_eq!(counters.activated.get(), 0);
    }

    fn candidate_bindings(
        staging: CapabilityStagingClass,
        counters: Counters,
        response: Vec<u8>,
    ) -> crate::CapabilityBindings {
        let mut registry = CapabilityRegistry::new(1);
        registry
            .register_with_staging(StagingProvider { counters, response }, staging)
            .unwrap();
        registry
            .negotiate_generation(&manifest(), GenerationId::new(41))
            .unwrap()
    }

    fn call() -> CapabilityCall<'static> {
        CapabilityCall::new(CAPABILITY_ID, 1, 7, &[], 8)
    }

    fn manifest() -> ManifestView<'static> {
        let requirements = [CapabilityRequirement::new(
            CAPABILITY_ID,
            1,
            CapabilityPolicy::Required,
            FINGERPRINT,
        )];
        let bytes = ApplicationManifest::new(
            AbiVersion::new(1, 0),
            AbiVersion::new(1, 0),
            WIDGET_IR_FORMAT_VERSION,
            CALLBACK_EVENT_FORMAT_VERSION,
            STATE_FORMAT_VERSION,
            StableId128::from_bytes([0x10; 16]),
            &requirements,
        )
        .encode(MODEL_LIMITS)
        .unwrap()
        .into_boxed_slice();
        ManifestView::decode(Box::leak(bytes), MODEL_LIMITS).unwrap()
    }

    #[derive(Clone, Default)]
    struct Counters {
        invoked: Rc<Cell<u32>>,
        staged: Rc<Cell<u32>>,
        activated: Rc<Cell<u32>>,
    }

    struct StagingProvider {
        counters: Counters,
        response: Vec<u8>,
    }

    impl CapabilityProvider for StagingProvider {
        fn descriptor(&self) -> CapabilityDescriptor {
            CapabilityDescriptor::new(CAPABILITY_ID, 1, FINGERPRINT, PROVIDER_LIMITS)
        }

        fn invoke(
            &self,
            _generation: CapabilityGeneration,
            _method_id: u32,
            _request: &[u8],
            _response_limit: u32,
        ) -> CapabilityResult<Vec<u8>> {
            self.counters.invoked.set(self.counters.invoked.get() + 1);
            Ok(self.response.clone())
        }

        fn stage(
            &self,
            _generation: CapabilityGeneration,
            _method_id: u32,
            _request: &[u8],
            _response_limit: u32,
        ) -> CapabilityResult<StagedCapability> {
            self.counters.staged.set(self.counters.staged.get() + 1);
            let activated = self.counters.activated.clone();
            Ok(StagedCapability::new(self.response.clone(), move || {
                activated.set(activated.get() + 1);
            }))
        }
    }
}
#[cfg(test)]
mod compact_encoding_tests {
    use crate::{
        ModelError, ModelLimits, PropertyId, PropertyValue, Version, WIDGET_TEXT, WidgetDocument,
        WidgetDocumentView, WidgetNode, WidgetProperty,
    };

    const LIMITS: ModelLimits = ModelLimits::new(16 * 1024, 64, 1024, 1024);

    #[test]
    fn compact_encoding_interns_duplicate_strings_by_first_occurrence() {
        let properties = [
            WidgetProperty::new(PropertyId::new(1), PropertyValue::StringRef(2)),
            WidgetProperty::new(PropertyId::new(2), PropertyValue::StringRef(1)),
        ];
        let nodes = [WidgetNode::new(WIDGET_TEXT, Version::new(1, 0)).properties(&properties)];
        let strings = ["alpha", "beta", "alpha"];
        let document = WidgetDocument::new(7, 11, 0, &nodes, &strings, &[]);

        let encoded = document.encode_compact(LIMITS).unwrap();
        let decoded = WidgetDocumentView::decode(&encoded, LIMITS).unwrap();
        let values: Vec<_> = decoded
            .node(0)
            .unwrap()
            .properties()
            .map(|property| property.value())
            .collect();

        assert_eq!(decoded.string(0), Some("alpha"));
        assert_eq!(decoded.string(1), Some("beta"));
        assert_eq!(decoded.string(2), None);
        assert_eq!(
            values,
            [PropertyValue::StringRef(0), PropertyValue::StringRef(1)]
        );
    }

    #[test]
    fn compact_encoding_interns_duplicate_blobs_and_remaps_mixed_references() {
        let properties = [
            WidgetProperty::new(PropertyId::new(1), PropertyValue::StringRef(2)),
            WidgetProperty::new(PropertyId::new(2), PropertyValue::BlobRef(2)),
            WidgetProperty::new(PropertyId::new(3), PropertyValue::StringRef(1)),
            WidgetProperty::new(PropertyId::new(4), PropertyValue::BlobRef(1)),
            WidgetProperty::new(PropertyId::new(5), PropertyValue::I64(-9)),
        ];
        let nodes = [WidgetNode::new(WIDGET_TEXT, Version::new(1, 0)).properties(&properties)];
        let strings = ["first", "second", "first"];
        let blobs: [&[u8]; 3] = [b"one", b"two", b"one"];
        let document = WidgetDocument::new(8, 12, 0, &nodes, &strings, &blobs);

        let encoded = document.encode_compact(LIMITS).unwrap();
        let decoded = WidgetDocumentView::decode(&encoded, LIMITS).unwrap();
        let values: Vec<_> = decoded
            .node(0)
            .unwrap()
            .properties()
            .map(|property| property.value())
            .collect();

        assert_eq!(decoded.blob(0), Some(b"one".as_slice()));
        assert_eq!(decoded.blob(1), Some(b"two".as_slice()));
        assert_eq!(decoded.blob(2), None);
        assert_eq!(
            values,
            [
                PropertyValue::StringRef(0),
                PropertyValue::BlobRef(0),
                PropertyValue::StringRef(1),
                PropertyValue::BlobRef(1),
                PropertyValue::I64(-9),
            ]
        );
    }

    #[test]
    fn compact_encoding_is_deterministic_across_repeated_calls() {
        let properties = [
            WidgetProperty::new(PropertyId::new(1), PropertyValue::StringRef(1)),
            WidgetProperty::new(PropertyId::new(2), PropertyValue::BlobRef(1)),
        ];
        let nodes = [WidgetNode::new(WIDGET_TEXT, Version::new(1, 0)).properties(&properties)];
        let strings = ["same", "same"];
        let blobs: [&[u8]; 2] = [b"bytes", b"bytes"];
        let document = WidgetDocument::new(13, 21, 0, &nodes, &strings, &blobs);

        let first = document.encode_compact(LIMITS).unwrap();
        let second = document.encode_compact(LIMITS).unwrap();

        assert_eq!(first, second);
    }

    #[test]
    fn compact_encoding_matches_ordinary_encoding_without_duplicate_payloads() {
        let properties = [
            WidgetProperty::new(PropertyId::new(1), PropertyValue::StringRef(1)),
            WidgetProperty::new(PropertyId::new(2), PropertyValue::BlobRef(0)).optional(),
            WidgetProperty::new(PropertyId::new(3), PropertyValue::Rgba(0x1020_30ff)),
        ];
        let children = [1];
        let nodes = [
            WidgetNode::new(WIDGET_TEXT, Version::new(1, 0))
                .properties(&properties)
                .children(&children),
            WidgetNode::new(WIDGET_TEXT, Version::new(1, 0)),
        ];
        let strings = ["first", "second"];
        let blobs: [&[u8]; 2] = [b"alpha", b"beta"];
        let document = WidgetDocument::new(34, 55, 0, &nodes, &strings, &blobs);

        assert_eq!(
            document.encode_compact(LIMITS).unwrap(),
            document.encode(LIMITS).unwrap()
        );
    }

    #[test]
    fn compact_encoding_rejects_references_outside_original_tables() {
        let string_properties = [WidgetProperty::new(
            PropertyId::new(1),
            PropertyValue::StringRef(2),
        )];
        let string_nodes =
            [WidgetNode::new(WIDGET_TEXT, Version::new(1, 0)).properties(&string_properties)];
        let strings = ["duplicate", "duplicate"];
        let string_document = WidgetDocument::new(1, 1, 0, &string_nodes, &strings, &[]);

        assert_eq!(
            string_document.encode_compact(LIMITS).unwrap_err(),
            ModelError::PropertyReferenceOutOfBounds { index: 2, count: 2 }
        );

        let blob_properties = [WidgetProperty::new(
            PropertyId::new(1),
            PropertyValue::BlobRef(2),
        )];
        let blob_nodes =
            [WidgetNode::new(WIDGET_TEXT, Version::new(1, 0)).properties(&blob_properties)];
        let blobs: [&[u8]; 2] = [b"duplicate", b"duplicate"];
        let blob_document = WidgetDocument::new(1, 1, 0, &blob_nodes, &[], &blobs);

        assert_eq!(
            blob_document.encode_compact(LIMITS).unwrap_err(),
            ModelError::PropertyReferenceOutOfBounds { index: 2, count: 2 }
        );
    }

    #[test]
    fn compact_encoding_applies_document_limit_after_deduplication() {
        let nodes = [WidgetNode::new(WIDGET_TEXT, Version::new(1, 0))];
        let strings = ["0123456789abcdef", "0123456789abcdef"];
        let document = WidgetDocument::new(1, 1, 0, &nodes, &strings, &[]);
        let limits = ModelLimits::new(160, 64, 32, 32);

        assert_eq!(
            document.encode(limits).unwrap_err(),
            ModelError::DocumentTooLarge {
                length: 176,
                limit: 160,
            }
        );

        let compact = document.encode_compact(limits).unwrap();
        let decoded = WidgetDocumentView::decode(&compact, limits).unwrap();

        assert_eq!(compact.len(), 152);
        assert_eq!(decoded.string(0), Some("0123456789abcdef"));
        assert_eq!(decoded.string(1), None);
    }
}
#[cfg(test)]
mod generation_resources_tests {
    use std::cell::Cell;
    use std::cell::RefCell;
    use std::future::pending;
    use std::rc::Rc;

    use crate::{
        AsyncCallbackError, AsyncCallbackEvent, AsyncCallbackEventKind, CallbackBinding,
        CallbackBindingSnapshot, Generation, GenerationAsyncLimits, GenerationId, GenerationLimits,
        GenerationResource, GenerationResourceError, GenerationResourceKind, ModelLimits,
        StableId128, Version, WidgetDocument, WidgetDocumentView, WidgetNode, WidgetSchemaId,
    };
    use aimer_venus::LocalScheduler;

    const MODEL_LIMITS: ModelLimits = ModelLimits::new(1_024, 16, 64, 64);

    #[test]
    fn retirement_cancels_pending_generation_tasks() {
        let scheduler = LocalScheduler::new();
        let mut generation = generation(41, scheduler.clone(), GenerationLimits::new(4));
        generation.spawn_task(pending()).unwrap();
        assert_eq!(scheduler.task_count(), 1);

        generation.retire();

        assert_eq!(scheduler.task_count(), 0);
    }

    #[test]
    fn async_completion_consumes_one_generation_owned_task_once() {
        let mut generation = async_generation(41, LocalScheduler::new());
        let task_id = generation.register_async_task(CALLBACK_ID).unwrap();
        let event = AsyncCallbackEvent::new(
            41,
            1,
            CALLBACK_ID,
            task_id,
            AsyncCallbackEventKind::Complete,
            &[7, 8],
        )
        .encode(MODEL_LIMITS)
        .unwrap();

        generation
            .validate_async_event(&event, MODEL_LIMITS)
            .unwrap();
        assert_eq!(generation.async_task_count(), 0);
        assert!(
            generation
                .validate_async_event(&event, MODEL_LIMITS)
                .is_err()
        );
    }

    #[test]
    fn generation_async_resource_limit_bounds_host_tasks() {
        let mut generation = async_generation(41, LocalScheduler::new())
            .with_async_limits(GenerationAsyncLimits::new(4, 8).with_max_retained_resources(0));

        assert_eq!(
            generation.register_async_task(CALLBACK_ID),
            Err(AsyncCallbackError::TaskLimitExceeded { limit: 0 })
        );
        assert_eq!(generation.async_task_count(), 0);
    }

    #[test]
    fn retired_generation_rejects_late_async_completion_and_cancels_task() {
        let scheduler = LocalScheduler::new();
        let mut generation = async_generation(41, scheduler.clone());
        let task_id = generation.register_async_task(CALLBACK_ID).unwrap();
        generation.retire();

        let event = AsyncCallbackEvent::complete(41, 1, CALLBACK_ID, task_id, &[])
            .encode(MODEL_LIMITS)
            .unwrap();
        assert!(
            generation
                .validate_async_event(&event, MODEL_LIMITS)
                .is_err()
        );
        assert_eq!(generation.async_task_count(), 0);
        assert_eq!(scheduler.task_count(), 0);
    }

    #[test]
    fn retirement_releases_every_resource_kind_once() {
        let releases = Rc::new(Cell::new(0));
        let mut generation = generation(41, LocalScheduler::new(), GenerationLimits::new(4));
        for kind in [
            GenerationResourceKind::Timer,
            GenerationResourceKind::Subscription,
            GenerationResourceKind::Request,
            GenerationResourceKind::Capability,
        ] {
            generation
                .register_resource(kind, TrackedResource(releases.clone()))
                .unwrap();
        }

        generation.retire();
        generation.retire();

        assert_eq!(releases.get(), 4);
    }

    #[test]
    fn generation_resources_can_be_released_explicitly() {
        let releases = Rc::new(Cell::new(0));
        let mut generation = generation(41, LocalScheduler::new(), GenerationLimits::new(4));
        let handle = generation
            .register_resource(
                GenerationResourceKind::Timer,
                TrackedResource(releases.clone()),
            )
            .unwrap();

        generation.release_resource(handle).unwrap();
        let error = generation.release_resource(handle).unwrap_err();

        assert_eq!(releases.get(), 1);
        assert_eq!(error, GenerationResourceError::InvalidHandle { handle });
    }

    #[test]
    fn resource_registration_obeys_each_kind_limit() {
        let mut generation = generation(41, LocalScheduler::new(), GenerationLimits::new(1));
        generation
            .register_resource(GenerationResourceKind::Request, TrackedResource::default())
            .unwrap();

        let error = generation
            .register_resource(GenerationResourceKind::Request, TrackedResource::default())
            .unwrap_err();

        assert_eq!(
            error,
            GenerationResourceError::LimitExceeded {
                kind: GenerationResourceKind::Request,
                limit: 1,
            }
        );
    }

    #[test]
    fn exhausted_handle_serial_releases_the_rejected_resource() {
        let releases = Rc::new(Cell::new(0));
        let mut generation = generation(
            41,
            LocalScheduler::new(),
            GenerationLimits::new(2).max_handle_serial(1),
        );
        generation
            .register_resource(
                GenerationResourceKind::Timer,
                TrackedResource(releases.clone()),
            )
            .unwrap();

        let error = generation
            .register_resource(
                GenerationResourceKind::Timer,
                TrackedResource(releases.clone()),
            )
            .unwrap_err();

        assert_eq!(error, GenerationResourceError::HandleExhausted);
        assert_eq!(releases.get(), 1);
        generation.retire();
        assert_eq!(releases.get(), 2);
    }

    #[test]
    fn a_resource_handle_cannot_cross_generations() {
        let releases = Rc::new(Cell::new(0));
        let mut owner = generation(41, LocalScheduler::new(), GenerationLimits::new(4));
        let mut replacement = generation(42, LocalScheduler::new(), GenerationLimits::new(4));
        let handle = owner
            .register_resource(
                GenerationResourceKind::Subscription,
                TrackedResource(releases.clone()),
            )
            .unwrap();

        let error = replacement.release_resource(handle).unwrap_err();
        owner.retire();

        assert_eq!(
            error,
            GenerationResourceError::GenerationMismatch {
                expected: GenerationId::new(42),
                actual: GenerationId::new(41),
            }
        );
        assert_eq!(releases.get(), 1);
    }

    #[test]
    fn late_completion_is_dropped_after_generation_retirement() {
        let delivered = Rc::new(Cell::new(false));
        let mut generation = generation(41, LocalScheduler::new(), GenerationLimits::new(4));
        let completion = generation.completion_token();
        generation.retire();

        let error = completion
            .deliver({
                let delivered = delivered.clone();
                move || delivered.set(true)
            })
            .unwrap_err();

        assert_eq!(error, GenerationResourceError::RetiredGeneration);
        assert!(!delivered.get());
    }

    #[test]
    fn disposal_failure_does_not_skip_generation_cleanup() {
        let scheduler = LocalScheduler::new();
        let releases = Rc::new(Cell::new(0));
        let mut generation = generation(41, scheduler.clone(), GenerationLimits::new(4));
        generation.spawn_task(pending()).unwrap();
        generation
            .register_resource(
                GenerationResourceKind::Capability,
                TrackedResource(releases.clone()),
            )
            .unwrap();

        let error = generation
            .retire_with_disposal(|| Err(DisposeError))
            .unwrap_err();

        assert_eq!(error, DisposeError);
        assert_eq!(scheduler.task_count(), 0);
        assert_eq!(releases.get(), 1);
    }

    #[test]
    fn dropping_a_generation_releases_its_resources() {
        let releases = Rc::new(Cell::new(0));
        let mut generation = generation(41, LocalScheduler::new(), GenerationLimits::new(4));
        generation
            .register_resource(
                GenerationResourceKind::Timer,
                TrackedResource(releases.clone()),
            )
            .unwrap();

        drop(generation);

        assert_eq!(releases.get(), 1);
    }

    #[test]
    fn generation_owns_the_guest_until_host_resources_are_released() {
        let order = Rc::new(RefCell::new(Vec::new()));
        let nodes = [WidgetNode::new(WidgetSchemaId::new(1), Version::new(1, 0))];
        let image = WidgetDocument::new(41, 1, 0, &nodes, &[], &[])
            .encode(MODEL_LIMITS)
            .unwrap();
        let view = WidgetDocumentView::decode(&image, MODEL_LIMITS).unwrap();
        let mut generation = Generation::with_guest(
            GenerationId::new(41),
            CallbackBindingSnapshot::from_document(&view, 4).unwrap(),
            LocalScheduler::new(),
            GenerationLimits::new(4),
            TrackedGuest(order.clone()),
        );
        assert!(Rc::ptr_eq(&generation.guest().0, &order));
        generation
            .register_resource(
                GenerationResourceKind::Request,
                OrderedResource(order.clone()),
            )
            .unwrap();

        drop(generation);

        assert_eq!(&*order.borrow(), &["resource", "guest"]);
    }

    fn generation(
        generation_id: u64,
        scheduler: Rc<LocalScheduler>,
        limits: GenerationLimits,
    ) -> Generation {
        let nodes = [WidgetNode::new(WidgetSchemaId::new(1), Version::new(1, 0))];
        let image = WidgetDocument::new(generation_id, 1, 0, &nodes, &[], &[])
            .encode(MODEL_LIMITS)
            .unwrap();
        let view = WidgetDocumentView::decode(&image, MODEL_LIMITS).unwrap();
        Generation::with_scheduler(
            GenerationId::new(generation_id),
            CallbackBindingSnapshot::from_document(&view, 4).unwrap(),
            scheduler,
            limits,
        )
    }

    const CALLBACK_ID: StableId128 = StableId128::from_bytes([0x71; 16]);

    fn async_generation(generation_id: u64, scheduler: Rc<LocalScheduler>) -> Generation {
        let callbacks = [CallbackBinding::new_async(
            crate::EVENT_BUTTON_PRESS,
            Version::new(1, 0),
            Version::new(1, 0),
            CALLBACK_ID,
        )];
        let nodes =
            [WidgetNode::new(WidgetSchemaId::new(1), Version::new(1, 0)).callbacks(&callbacks)];
        let image = WidgetDocument::new(generation_id, 1, 0, &nodes, &[], &[])
            .encode(MODEL_LIMITS)
            .unwrap();
        let view = WidgetDocumentView::decode(&image, MODEL_LIMITS).unwrap();
        Generation::with_scheduler(
            GenerationId::new(generation_id),
            CallbackBindingSnapshot::from_document(&view, 4).unwrap(),
            scheduler,
            GenerationLimits::new(4),
        )
    }

    #[derive(Debug, Default)]
    struct TrackedResource(Rc<Cell<usize>>);

    impl GenerationResource for TrackedResource {
        fn release(self: Box<Self>) {
            self.0.set(self.0.get() + 1);
        }
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct DisposeError;

    struct OrderedResource(Rc<RefCell<Vec<&'static str>>>);

    impl GenerationResource for OrderedResource {
        fn release(self: Box<Self>) {
            self.0.borrow_mut().push("resource");
        }
    }

    struct TrackedGuest(Rc<RefCell<Vec<&'static str>>>);

    impl Drop for TrackedGuest {
        fn drop(&mut self) {
            self.0.borrow_mut().push("guest");
        }
    }
}
#[cfg(all(test, feature = "wasm-hot-reload"))]
mod guest_abi_tests {
    mod guest_module {
        use crate as aimer_anteros;

        include!("../tests/support/guest_module.rs");
    }

    use crate::{
        AbiVersion, CALLBACK_EVENT_FORMAT_VERSION, CapabilityDescriptor, CapabilityLimits,
        CapabilityProvider, CapabilityRegistry, CapabilityResult, CapabilityStagingClass,
        GenerationId, GuestDiagnosticCategory, GuestOperation, ModelLimits, Runtime, RuntimeConfig,
        RuntimeErrorKind, STATE_FORMAT_VERSION, StableId128, StateTransferCoordinator,
        StateTransferError, StateTransferStage, Version, WIDGET_IR_FORMAT_VERSION,
    };

    const LIMITS: ModelLimits = ModelLimits::new(512, 16, 64, 64);
    #[rustfmt::skip]
    const EXPECTED_AWIR: &[u8] = &[
        b'A', b'W', b'I', b'R',
        2, 0, 0, 0,
        11, 0, 0, 0, 0, 0, 0, 0,
        13, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0,
        1, 0, 0, 0,
        0, 0, 0, 0,
        0, 0, 0, 0,
        0, 0, 0, 0,
        0, 0, 0, 0,
        0, 0, 0, 0,
        0, 0, 0, 0,
        0, 0, 0, 0,
        128, 0, 0, 0,
        7, 0, 0, 0,
        0, 0, 0, 0,
        1, 0, 2, 0,
        0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0,
        0, 0, 0, 0,
        0, 0, 0, 0,
        0, 0, 0, 0,
        0, 0, 0, 0,
        0, 0, 0, 0,
        0, 0, 0, 0,
        0, 0, 0, 0,
    ];
    #[rustfmt::skip]
    const CALLBACK_EVENT: &[u8] = &[
        b'A', b'E', b'V', b'T',
        2, 0, 0, 0,
        3, 0, 0, 0, 0, 0, 0, 0,
        5, 0, 0, 0, 0, 0, 0, 0,
        0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11,
        0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11,
        0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0,
        11, 0, 0, 0, 0, 0, 0, 0,
        7, 0, 0, 0,
        0, 0, 0, 0,
        2, 0, 1, 0,
        0, 0, 0, 0,
        1, 0, 0, 0,
        97, 0, 0, 0,
        0, 0, 0, 0,
        0, 0, 0, 0,
        0xA5,
    ];
    #[rustfmt::skip]
    const EXPECTED_ASTA: &[u8] = &[
        b'A', b'S', b'T', b'A',
        1, 0, 0, 0,
        0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10,
        0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10,
        7, 0, 0, 0, 0, 0, 0, 0,
        1, 0, 0, 0,
        1, 0, 0, 0,
        97, 0, 0, 0,
        0, 0, 0, 0,
        0x20, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20,
        0x20, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20,
        0x30, 0x30, 0x30, 0x30, 0x30, 0x30, 0x30, 0x30,
        0x30, 0x30, 0x30, 0x30, 0x30, 0x30, 0x30, 0x30,
        2, 0, 1, 0,
        1, 0, 0, 0,
        0, 0, 0, 0,
        1, 0, 0, 0,
        0xA5,
    ];
    #[rustfmt::skip]
    const EXPECTED_AMNF: &[u8] = &[
        b'A', b'M', b'N', b'F',
        1, 0, 0, 0,
        1, 0, 0, 0, 0, 0, 0, 0,
        1, 0, 0, 0, 0, 0, 0, 0,
        2, 0, 0, 0,
        2, 0, 0, 0,
        1, 0, 0, 0,
        0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10,
        0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10,
        0, 0, 0, 0,
        64, 0, 0, 0,
        0, 0, 0, 0,
    ];

    #[test]
    fn runtime_build_returns_the_fixture_guest_widget_image() {
        let runtime = test_runtime(10_000);
        let module = guest_module::build_guest();

        let output = runtime.build(&module, LIMITS).unwrap();

        assert_eq!(output.as_bytes(), EXPECTED_AWIR);
        assert_eq!(output.view().generation_id(), 11);
        assert_eq!(output.view().document_revision(), 13);
    }

    #[test]
    fn runtime_surfaces_structured_build_diagnostic() {
        let runtime = test_runtime(10_000);
        let error = runtime
            .build(&guest_module::diagnostic_build_guest(), LIMITS)
            .unwrap_err();

        assert_eq!(error.kind(), RuntimeErrorKind::GuestStatus);
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

    #[test]
    fn runtime_falls_back_to_status_only_for_malformed_diagnostic() {
        let runtime = test_runtime(10_000);
        let error = runtime
            .build(&guest_module::malformed_diagnostic_build_guest(), LIMITS)
            .unwrap_err();

        assert_eq!(error.kind(), RuntimeErrorKind::GuestStatus);
        assert!(error.diagnostic().is_none());
        assert!(
            error
                .to_string()
                .contains("aimer_build probe returned ApplicationError with length 0")
        );
    }

    #[test]
    fn runtime_falls_back_to_status_only_for_oversized_diagnostic() {
        let runtime = test_runtime(10_000);
        let error = runtime
            .build(&guest_module::oversized_diagnostic_build_guest(), LIMITS)
            .unwrap_err();

        assert_eq!(error.kind(), RuntimeErrorKind::GuestStatus);
        assert!(error.diagnostic().is_none());
        assert!(
            error
                .to_string()
                .contains("aimer_build probe returned ApplicationError with length 0")
        );
    }

    #[test]
    fn persistent_guest_build_dispatches_a_negotiated_capability_import() {
        let runtime = test_runtime(10_000);
        let module = guest_module::capability_build_guest();
        let mut registry = CapabilityRegistry::new(1);
        registry
            .register_with_staging(WidgetProvider, CapabilityStagingClass::PureQuery)
            .unwrap();

        let mut guest = runtime
            .instantiate_with_capabilities(&module, &registry, LIMITS, GenerationId::new(41))
            .unwrap();
        let output = guest.build(LIMITS).unwrap();

        assert_eq!(output.as_bytes(), EXPECTED_AWIR);
    }

    #[test]
    fn runtime_guest_activates_committed_only_capabilities_explicitly() {
        let runtime = test_runtime(10_000);
        let module = guest_module::capability_build_guest();
        let mut registry = CapabilityRegistry::new(1);
        registry.register(WidgetProvider).unwrap();
        let mut guest = runtime
            .instantiate_with_capabilities(&module, &registry, LIMITS, GenerationId::new(41))
            .unwrap();

        assert_eq!(
            guest.build(LIMITS).unwrap_err().kind(),
            RuntimeErrorKind::GuestStatus
        );

        guest.activate();

        assert_eq!(guest.build(LIMITS).unwrap().as_bytes(), EXPECTED_AWIR);
    }

    #[test]
    fn retired_guest_generation_rejects_new_capability_builds() {
        let runtime = test_runtime(10_000);
        let mut registry = CapabilityRegistry::new(1);
        registry
            .register_with_staging(WidgetProvider, CapabilityStagingClass::PureQuery)
            .unwrap();
        let mut guest = runtime
            .instantiate_with_capabilities(
                &guest_module::capability_build_guest(),
                &registry,
                LIMITS,
                GenerationId::new(41),
            )
            .unwrap();

        assert_eq!(guest.generation_id(), Some(GenerationId::new(41)));
        guest.retire();
        let error = guest.build(LIMITS).unwrap_err();

        assert_eq!(error.kind(), RuntimeErrorKind::RetiredGeneration);
    }

    #[test]
    fn capability_guest_rejects_missing_providers_and_incompatible_imports() {
        let runtime = test_runtime(10_000);
        let registry = CapabilityRegistry::new(0);

        let missing = runtime
            .instantiate_with_capabilities(
                &guest_module::capability_build_guest(),
                &registry,
                LIMITS,
                GenerationId::new(41),
            )
            .unwrap_err();
        assert_eq!(missing.kind(), RuntimeErrorKind::CapabilityNegotiation);

        let wrong_signature = runtime
            .instantiate_with_capabilities(
                &guest_module::wrong_capability_import_signature_guest(),
                &registry,
                LIMITS,
                GenerationId::new(41),
            )
            .unwrap_err();
        assert_eq!(wrong_signature.kind(), RuntimeErrorKind::Instantiation);

        let unrelated = runtime
            .instantiate_with_capabilities(
                &guest_module::imported_memory_guest(),
                &registry,
                LIMITS,
                GenerationId::new(41),
            )
            .unwrap_err();
        assert_eq!(unrelated.kind(), RuntimeErrorKind::Module);
    }

    #[test]
    fn capability_guest_maps_provider_failures_to_stable_abi_statuses() {
        let runtime = test_runtime(10_000);
        let mut registry = CapabilityRegistry::new(1);
        registry
            .register_with_staging(DeniedWidgetProvider, CapabilityStagingClass::PureQuery)
            .unwrap();
        let mut guest = runtime
            .instantiate_with_capabilities(
                &guest_module::capability_build_guest(),
                &registry,
                LIMITS,
                GenerationId::new(41),
            )
            .unwrap();

        let error = guest.build(LIMITS).unwrap_err();

        assert_eq!(error.kind(), RuntimeErrorKind::GuestStatus);
        assert!(error.to_string().contains("CapabilityDenied"));
    }

    #[test]
    fn capability_guest_rejects_oversized_requests_before_copying_guest_memory() {
        let runtime = test_runtime(10_000);
        let mut registry = CapabilityRegistry::new(1);
        registry
            .register_with_staging(WidgetProvider, CapabilityStagingClass::PureQuery)
            .unwrap();
        let mut guest = runtime
            .instantiate_with_capabilities(
                &guest_module::oversized_capability_request_guest(),
                &registry,
                LIMITS,
                GenerationId::new(42),
            )
            .unwrap();

        let error = guest.build(LIMITS).unwrap_err();

        assert_eq!(error.kind(), RuntimeErrorKind::GuestStatus);
        assert!(error.to_string().contains("ResourceExhausted"));
    }

    struct WidgetProvider;

    impl CapabilityProvider for WidgetProvider {
        fn descriptor(&self) -> CapabilityDescriptor {
            CapabilityDescriptor::new(
                StableId128::from_bytes([0x20; 16]),
                1,
                [0x30; 32],
                CapabilityLimits::new(0, 512),
            )
        }

        fn invoke(
            &self,
            _generation: crate::CapabilityGeneration,
            method_id: u32,
            request: &[u8],
            response_limit: u32,
        ) -> CapabilityResult<Vec<u8>> {
            assert_eq!(method_id, 0);
            assert!(request.is_empty());
            assert_eq!(response_limit, 512);
            Ok(EXPECTED_AWIR.to_vec())
        }
    }

    struct DeniedWidgetProvider;

    impl CapabilityProvider for DeniedWidgetProvider {
        fn descriptor(&self) -> CapabilityDescriptor {
            WidgetProvider.descriptor()
        }

        fn invoke(
            &self,
            _generation: crate::CapabilityGeneration,
            _method_id: u32,
            _request: &[u8],
            _response_limit: u32,
        ) -> CapabilityResult<Vec<u8>> {
            Err(crate::CapabilityError::Denied)
        }
    }

    #[test]
    fn runtime_round_trips_callback_input_into_exported_guest_state() {
        let runtime = test_runtime(10_000);
        let module = guest_module::callback_state_guest();

        let state = runtime
            .dispatch_event_and_export_state(&module, CALLBACK_EVENT, LIMITS)
            .unwrap();

        assert_eq!(state.as_bytes(), EXPECTED_ASTA);
        assert_eq!(state.view().source_generation(), 7);
        assert_eq!(state.view().entry(0).unwrap().payload(), &[0xA5]);
    }

    #[test]
    fn guest_instance_preserves_state_between_abi_calls() {
        let runtime = test_runtime(10_000);
        let module = guest_module::callback_state_guest();
        let mut guest = runtime.instantiate(&module).unwrap();

        assert!(
            guest
                .dispatch_event(CALLBACK_EVENT, LIMITS)
                .unwrap()
                .is_none()
        );
        let state = guest.export_state(LIMITS).unwrap();

        assert_eq!(state.as_bytes(), EXPECTED_ASTA);
        assert_eq!(state.view().entry(0).unwrap().payload(), &[0xA5]);
    }

    #[test]
    fn guest_instance_imports_and_exports_identical_state() {
        let runtime = test_runtime(10_000);
        let module = guest_module::callback_state_guest();
        let mut guest = runtime.instantiate(&module).unwrap();

        guest.import_state(EXPECTED_ASTA, LIMITS).unwrap();
        let state = guest.export_state(LIMITS).unwrap();

        assert_eq!(state.as_bytes(), EXPECTED_ASTA);
    }

    #[test]
    fn coordinator_exports_imports_and_verifies_persistent_guest_state() {
        let runtime = test_runtime(10_000);
        let module = guest_module::callback_state_guest();
        let mut previous = runtime.instantiate(&module).unwrap();
        previous.dispatch_event(CALLBACK_EVENT, LIMITS).unwrap();
        let old_before = previous.export_state(LIMITS).unwrap().as_bytes().to_vec();
        let mut candidate = runtime.instantiate(&module).unwrap();
        let coordinator = StateTransferCoordinator::new()
            .model_limits(LIMITS)
            .migration_fuel(8);

        let report = coordinator
            .transfer_guest_state(&mut previous, &mut candidate)
            .unwrap();

        assert_eq!(report.preserved_entries(), 1);
        assert_eq!(
            candidate.export_state(LIMITS).unwrap().as_bytes(),
            old_before
        );
        assert_eq!(
            previous.export_state(LIMITS).unwrap().as_bytes(),
            old_before
        );
    }

    #[test]
    fn coordinator_keeps_the_old_guest_unchanged_when_candidate_import_fails() {
        let runtime = test_runtime(10_000);
        let mut previous = runtime
            .instantiate(&guest_module::callback_state_guest())
            .unwrap();
        previous.dispatch_event(CALLBACK_EVENT, LIMITS).unwrap();
        let old_before = previous.export_state(LIMITS).unwrap().as_bytes().to_vec();
        let mut candidate = runtime
            .instantiate(&guest_module::incompatible_import_status_guest())
            .unwrap();
        let coordinator = StateTransferCoordinator::new()
            .model_limits(LIMITS)
            .migration_fuel(8);

        let error = coordinator
            .transfer_guest_state(&mut previous, &mut candidate)
            .unwrap_err();

        assert!(matches!(
            error,
            StateTransferError::Runtime {
                stage: StateTransferStage::ImportCandidate,
                ..
            }
        ));
        assert_eq!(
            previous.export_state(LIMITS).unwrap().as_bytes(),
            old_before
        );
    }

    #[test]
    fn coordinator_rejects_a_candidate_that_silently_ignores_imported_state() {
        let runtime = test_runtime(10_000);
        let mut previous = runtime
            .instantiate(&guest_module::callback_state_guest())
            .unwrap();
        previous.dispatch_event(CALLBACK_EVENT, LIMITS).unwrap();
        let old_before = previous.export_state(LIMITS).unwrap().as_bytes().to_vec();
        let mut candidate = runtime
            .instantiate(&guest_module::ignoring_import_guest())
            .unwrap();
        let coordinator = StateTransferCoordinator::new()
            .model_limits(LIMITS)
            .migration_fuel(8);

        let error = coordinator
            .transfer_guest_state(&mut previous, &mut candidate)
            .unwrap_err();

        assert!(matches!(error, StateTransferError::VerificationMismatch));
        assert_eq!(
            previous.export_state(LIMITS).unwrap().as_bytes(),
            old_before
        );
    }

    #[test]
    fn coordinator_executes_schema_migration_inside_the_candidate_guest() {
        let runtime = test_runtime(10_000);
        let mut previous = runtime
            .instantiate(&guest_module::callback_state_guest())
            .unwrap();
        previous.dispatch_event(CALLBACK_EVENT, LIMITS).unwrap();
        let mut candidate = runtime
            .instantiate(&guest_module::candidate_migration_guest())
            .unwrap();
        let coordinator = StateTransferCoordinator::new()
            .model_limits(LIMITS)
            .migration_fuel(8);

        let report = coordinator
            .transfer_guest_state(&mut previous, &mut candidate)
            .unwrap();

        let state = candidate.export_state(LIMITS).unwrap();
        assert_eq!(
            state.view().entry(0).unwrap().schema_version(),
            Version::new(3, 0)
        );
        assert_eq!(state.view().entry(0).unwrap().payload(), &[0xA5]);
        assert_eq!(
            report.migrated_state_ids(),
            &[StableId128::from_bytes([0x20; 16])]
        );
        assert!(report.migration_fuel_consumed() > 0);
    }

    #[test]
    fn coordinator_reports_a_candidate_migration_trap() {
        assert_migration_runtime_failure(
            &test_runtime(10_000),
            &test_runtime(10_000),
            guest_module::trapping_migration_guest(),
            RuntimeErrorKind::Execution,
        );
    }

    #[test]
    fn coordinator_skips_candidate_migration_when_schemas_are_compatible() {
        let runtime = test_runtime(10_000);
        let mut previous = runtime
            .instantiate(&guest_module::callback_state_guest())
            .unwrap();
        previous.dispatch_event(CALLBACK_EVENT, LIMITS).unwrap();
        let mut candidate = runtime
            .instantiate(&guest_module::unneeded_trapping_migration_guest())
            .unwrap();
        let coordinator = StateTransferCoordinator::new().model_limits(LIMITS);

        let report = coordinator
            .transfer_guest_state(&mut previous, &mut candidate)
            .unwrap();

        assert_eq!(report.preserved_entries(), 1);
        assert_eq!(report.migration_fuel_consumed(), 0);
    }

    #[test]
    fn coordinator_reports_candidate_migration_fuel_exhaustion() {
        assert_migration_runtime_failure(
            &test_runtime(10_000),
            &test_runtime(1_000),
            guest_module::infinite_migration_guest(),
            RuntimeErrorKind::FuelExhausted,
        );
    }

    #[test]
    fn coordinator_rejects_malformed_candidate_migration_output() {
        assert_migration_runtime_failure(
            &test_runtime(10_000),
            &test_runtime(10_000),
            guest_module::malformed_migration_guest(),
            RuntimeErrorKind::StateDocument,
        );
    }

    #[test]
    fn coordinator_rejects_silent_required_state_substitution() {
        let runtime = test_runtime(10_000);
        let mut previous = runtime
            .instantiate(&guest_module::callback_state_guest())
            .unwrap();
        previous.dispatch_event(CALLBACK_EVENT, LIMITS).unwrap();
        let old_before = previous.export_state(LIMITS).unwrap().as_bytes().to_vec();
        let mut candidate = runtime
            .instantiate(&guest_module::substituted_migration_state_guest())
            .unwrap();
        let coordinator = StateTransferCoordinator::new()
            .model_limits(LIMITS)
            .migration_fuel(8);

        let error = coordinator
            .transfer_guest_state(&mut previous, &mut candidate)
            .unwrap_err();

        assert!(matches!(
            error,
            StateTransferError::StateIncompatible {
                policy: crate::StatePolicy::Required,
                ..
            }
        ));
        assert_eq!(
            previous.export_state(LIMITS).unwrap().as_bytes(),
            old_before
        );
    }

    fn assert_migration_runtime_failure(
        previous_runtime: &Runtime,
        candidate_runtime: &Runtime,
        candidate_module: Vec<u8>,
        expected_kind: RuntimeErrorKind,
    ) {
        let mut previous = previous_runtime
            .instantiate(&guest_module::callback_state_guest())
            .unwrap();
        previous.dispatch_event(CALLBACK_EVENT, LIMITS).unwrap();
        let old_before = previous.export_state(LIMITS).unwrap().as_bytes().to_vec();
        let mut candidate = candidate_runtime.instantiate(&candidate_module).unwrap();
        let coordinator = StateTransferCoordinator::new()
            .model_limits(LIMITS)
            .migration_fuel(8);

        let error = coordinator
            .transfer_guest_state(&mut previous, &mut candidate)
            .unwrap_err();

        match error {
            StateTransferError::Runtime {
                stage: StateTransferStage::MigrateCandidate,
                source,
            } => assert_eq!(source.kind(), expected_kind),
            other => panic!("unexpected state transfer error: {other}"),
        }
        assert_eq!(
            previous.export_state(LIMITS).unwrap().as_bytes(),
            old_before
        );
    }

    #[test]
    fn guest_instance_queries_a_canonical_manifest() {
        let runtime = test_runtime(10_000);
        let module = guest_module::callback_state_guest();
        let mut guest = runtime.instantiate(&module).unwrap();

        let manifest = guest.manifest(LIMITS).unwrap();

        assert_eq!(manifest.as_bytes(), EXPECTED_AMNF);
        assert_eq!(manifest.view().minimum_abi(), AbiVersion::new(1, 0));
        assert_eq!(manifest.view().maximum_abi(), AbiVersion::new(1, 0));
        assert_eq!(
            manifest.view().widget_ir_version(),
            WIDGET_IR_FORMAT_VERSION
        );
        assert_eq!(
            manifest.view().callback_event_version(),
            CALLBACK_EVENT_FORMAT_VERSION
        );
        assert_eq!(manifest.view().state_version(), STATE_FORMAT_VERSION);
        assert_eq!(
            manifest.view().program_id(),
            StableId128::from_bytes([0x10; 16])
        );
    }

    #[test]
    fn guest_instance_rejects_a_malformed_manifest() {
        let error = query_manifest_error(guest_module::malformed_manifest_guest(), 10_000);

        assert_eq!(error.kind(), RuntimeErrorKind::ManifestDocument);
    }

    #[test]
    fn guest_instance_rejects_repeated_manifest_undersizing() {
        let error =
            query_manifest_error(guest_module::repeated_undersized_manifest_guest(), 10_000);

        assert_eq!(error.kind(), RuntimeErrorKind::GuestStatus);
    }

    #[test]
    fn guest_instance_rejects_an_invalid_manifest_output_pointer() {
        let error = query_manifest_error(guest_module::invalid_manifest_pointer_guest(), 10_000);

        assert_eq!(error.kind(), RuntimeErrorKind::GuestMemory);
    }

    #[test]
    fn guest_instance_reports_a_manifest_trap() {
        let error = query_manifest_error(guest_module::trapping_manifest_guest(), 10_000);

        assert_eq!(error.kind(), RuntimeErrorKind::Execution);
    }

    #[test]
    fn guest_instance_reports_manifest_fuel_exhaustion() {
        let error = query_manifest_error(guest_module::infinite_manifest_guest(), 100);

        assert_eq!(error.kind(), RuntimeErrorKind::FuelExhausted);
    }

    #[test]
    fn guest_instance_reports_manifest_cleanup_failure() {
        let error = query_manifest_error(guest_module::manifest_cleanup_failure_guest(), 10_000);

        assert_eq!(error.kind(), RuntimeErrorKind::GuestStatus);
        assert!(error.to_string().contains("aimer_dealloc"));
    }

    #[test]
    fn guest_instance_preserves_manifest_validation_failure_over_cleanup_failure() {
        let error = query_manifest_error(
            guest_module::malformed_manifest_and_cleanup_failure_guest(),
            10_000,
        );

        assert_eq!(error.kind(), RuntimeErrorKind::ManifestDocument);
    }

    fn query_manifest_error(module: Vec<u8>, fuel_per_call: u64) -> crate::RuntimeError {
        let runtime = test_runtime(fuel_per_call);
        let mut guest = runtime.instantiate(&module).unwrap();
        guest.manifest(LIMITS).unwrap_err()
    }

    #[test]
    fn runtime_rejects_a_guest_missing_the_manifest_export() {
        let runtime = test_runtime(10_000);
        let module = guest_module::missing_manifest_guest();

        let error = runtime.instantiate(&module).unwrap_err();

        assert_eq!(error.kind(), RuntimeErrorKind::Export);
    }

    #[test]
    fn runtime_rejects_an_unsupported_module_import_before_instantiation() {
        let runtime = test_runtime(10_000);
        let module = guest_module::unsupported_import_guest();

        let error = runtime.instantiate(&module).unwrap_err();

        assert_eq!(error.kind(), RuntimeErrorKind::Module);
    }

    #[test]
    fn runtime_rejects_a_wrongly_typed_manifest_export() {
        let error = instantiate_error(guest_module::wrong_manifest_signature_guest());

        assert_eq!(error.kind(), RuntimeErrorKind::Export);
    }

    #[test]
    fn runtime_rejects_an_undeclared_export() {
        let error = instantiate_error(guest_module::unexpected_export_guest());

        assert_eq!(error.kind(), RuntimeErrorKind::Export);
    }

    #[test]
    fn runtime_rejects_a_start_function_before_it_executes() {
        let error = instantiate_error(guest_module::start_function_guest());

        assert_eq!(error.kind(), RuntimeErrorKind::Module);
    }

    #[test]
    fn runtime_rejects_imported_memory() {
        let error = instantiate_error(guest_module::imported_memory_guest());

        assert_eq!(error.kind(), RuntimeErrorKind::Module);
    }

    #[test]
    fn runtime_rejects_a_module_larger_than_its_configured_limit() {
        let module = guest_module::memory_growth_guest();
        let runtime = Runtime::new(sandbox_config().max_module_bytes(module.len() - 1));

        let error = runtime.invoke_i32(&module, "grow").unwrap_err();

        assert_eq!(error.kind(), RuntimeErrorKind::ResourceLimit);
    }

    #[test]
    fn runtime_rejects_memory_growth_beyond_its_configured_page_limit() {
        let runtime = Runtime::new(sandbox_config().max_memory_pages(1));

        let error = runtime
            .invoke_i32(&guest_module::memory_growth_guest(), "grow")
            .unwrap_err();

        assert_eq!(error.kind(), RuntimeErrorKind::ResourceLimit);
    }

    #[test]
    fn runtime_rejects_table_growth_beyond_its_configured_element_limit() {
        let runtime = Runtime::new(sandbox_config().max_table_elements(1));

        let error = runtime
            .invoke_i32(&guest_module::table_growth_guest(), "grow")
            .unwrap_err();

        assert_eq!(error.kind(), RuntimeErrorKind::ResourceLimit);
    }

    #[test]
    fn runtime_rejects_guest_calls_beyond_its_configured_depth_limit() {
        let runtime = Runtime::new(sandbox_config().max_call_depth(8));

        let error = runtime
            .invoke_i32(&guest_module::recursive_guest(), "recurse")
            .unwrap_err();

        assert_eq!(error.kind(), RuntimeErrorKind::ResourceLimit);
    }

    #[test]
    fn runtime_rejects_unsupported_simd_before_guest_execution() {
        let runtime = Runtime::new(sandbox_config());

        let error = runtime
            .invoke_i32(&guest_module::simd_guest(), "simd")
            .unwrap_err();

        assert_eq!(error.kind(), RuntimeErrorKind::Module);
    }

    #[test]
    fn copied_widget_output_is_isolated_from_later_guest_memory_changes() {
        let runtime = Runtime::new(sandbox_config());
        let mut guest = runtime
            .instantiate(&guest_module::widget_source_mutation_guest())
            .unwrap();
        let image = guest.build(LIMITS).unwrap();

        guest.dispatch_event(CALLBACK_EVENT, LIMITS).unwrap();
        let next_build = guest.build(LIMITS).unwrap_err();

        assert_eq!(image.as_bytes(), EXPECTED_AWIR);
        assert_eq!(next_build.kind(), RuntimeErrorKind::WidgetDocument);
    }

    #[test]
    fn callback_dispatch_returns_a_validated_complete_widget_snapshot() {
        let runtime = test_runtime(10_000);
        let mut guest = runtime
            .instantiate(&guest_module::callback_widget_output_guest())
            .unwrap();

        let image = guest
            .dispatch_event(CALLBACK_EVENT, LIMITS)
            .unwrap()
            .unwrap();

        assert_eq!(image.as_bytes(), EXPECTED_AWIR);
        assert_eq!(image.view().generation_id(), 11);
    }

    #[test]
    fn callback_dispatch_rejects_a_partial_widget_snapshot() {
        let runtime = test_runtime(10_000);
        let mut guest = runtime
            .instantiate(&guest_module::callback_partial_widget_output_guest())
            .unwrap();

        let error = guest.dispatch_event(CALLBACK_EVENT, LIMITS).unwrap_err();

        assert_eq!(error.kind(), RuntimeErrorKind::WidgetDocument);
    }

    #[test]
    fn module_validation_is_deterministic_for_truncations_and_byte_mutations() {
        let module = guest_module::memory_growth_guest();
        let runtime = test_runtime(10_000);

        for end in 0..module.len() {
            let first = runtime
                .invoke_i32(&module[..end], "grow")
                .map_err(|error| error.kind());
            let second = runtime
                .invoke_i32(&module[..end], "grow")
                .map_err(|error| error.kind());
            assert!(first.is_err(), "prefix length {end}");
            assert_eq!(first, second, "prefix length {end}");
        }

        for index in 0..module.len() {
            let mut mutated = module.clone();
            mutated[index] ^= 0xA5;
            let first = runtime
                .invoke_i32(&mutated, "grow")
                .map_err(|error| error.kind());
            let second = runtime
                .invoke_i32(&mutated, "grow")
                .map_err(|error| error.kind());
            assert_eq!(first, second, "mutated byte {index}");
        }
    }

    fn sandbox_config() -> RuntimeConfig {
        RuntimeConfig::new()
            .fuel_per_call(10_000)
            .max_module_bytes(64 * 1_024)
            .max_memory_pages(2)
            .max_table_elements(16)
            .max_call_depth(64)
    }

    fn test_runtime(fuel_per_call: u64) -> Runtime {
        Runtime::new(sandbox_config().fuel_per_call(fuel_per_call))
    }

    fn instantiate_error(module: Vec<u8>) -> crate::RuntimeError {
        let runtime = test_runtime(10_000);
        runtime.instantiate(&module).unwrap_err()
    }

    #[test]
    fn guest_instance_rejects_malformed_imported_state() {
        let runtime = test_runtime(10_000);
        let module = guest_module::callback_state_guest();
        let mut guest = runtime.instantiate(&module).unwrap();

        let error = guest
            .import_state(&EXPECTED_ASTA[..EXPECTED_ASTA.len() - 1], LIMITS)
            .unwrap_err();

        assert_eq!(error.kind(), RuntimeErrorKind::StateDocument);
    }

    #[test]
    fn guest_instance_rejects_an_incompatible_import_status() {
        let runtime = test_runtime(10_000);
        let module = guest_module::incompatible_import_status_guest();
        let mut guest = runtime.instantiate(&module).unwrap();

        let error = guest.import_state(EXPECTED_ASTA, LIMITS).unwrap_err();

        assert_eq!(error.kind(), RuntimeErrorKind::GuestStatus);
    }

    #[test]
    fn guest_instance_rejects_an_import_allocation_outside_guest_memory() {
        let runtime = test_runtime(10_000);
        let module = guest_module::invalid_import_pointer_guest();
        let mut guest = runtime.instantiate(&module).unwrap();

        let error = guest.import_state(EXPECTED_ASTA, LIMITS).unwrap_err();

        assert_eq!(error.kind(), RuntimeErrorKind::GuestMemory);
        guest.import_state(EXPECTED_ASTA, LIMITS).unwrap();
    }

    #[test]
    fn guest_instance_reports_a_state_import_trap() {
        let runtime = test_runtime(10_000);
        let module = guest_module::trapping_import_guest();
        let mut guest = runtime.instantiate(&module).unwrap();

        let error = guest.import_state(EXPECTED_ASTA, LIMITS).unwrap_err();

        assert_eq!(error.kind(), RuntimeErrorKind::Execution);
    }

    #[test]
    fn guest_instance_reports_state_import_fuel_exhaustion() {
        let runtime = test_runtime(100);
        let module = guest_module::infinite_import_guest();
        let mut guest = runtime.instantiate(&module).unwrap();

        let error = guest.import_state(EXPECTED_ASTA, LIMITS).unwrap_err();

        assert_eq!(error.kind(), RuntimeErrorKind::FuelExhausted);
    }

    #[test]
    fn guest_instance_reports_state_import_cleanup_failure() {
        let runtime = test_runtime(10_000);
        let module = guest_module::import_cleanup_failure_guest();
        let mut guest = runtime.instantiate(&module).unwrap();

        let error = guest.import_state(EXPECTED_ASTA, LIMITS).unwrap_err();

        assert_eq!(error.kind(), RuntimeErrorKind::GuestStatus);
        assert!(error.to_string().contains("aimer_dealloc"));
    }

    #[test]
    fn guest_instance_preserves_import_failure_over_cleanup_failure() {
        let runtime = test_runtime(10_000);
        let module = guest_module::import_and_cleanup_failure_guest();
        let mut guest = runtime.instantiate(&module).unwrap();

        let error = guest.import_state(EXPECTED_ASTA, LIMITS).unwrap_err();

        assert_eq!(error.kind(), RuntimeErrorKind::GuestStatus);
        assert!(error.to_string().contains("aimer_import_state"));

        let cleanup_error = guest.import_state(EXPECTED_ASTA, LIMITS).unwrap_err();

        assert_eq!(cleanup_error.kind(), RuntimeErrorKind::GuestStatus);
        assert!(cleanup_error.to_string().contains("aimer_dealloc"));
    }

    #[test]
    fn runtime_rejects_an_incompatible_guest_abi_version() {
        let runtime = test_runtime(10_000);
        let module = guest_module::build_guest_with_abi(2_u64 << 32);

        let error = runtime.build(&module, LIMITS).unwrap_err();

        assert_eq!(error.kind(), RuntimeErrorKind::AbiVersion);
    }

    #[test]
    fn runtime_rejects_a_guest_that_repeats_buffer_too_small() {
        let runtime = test_runtime(10_000);
        let module = guest_module::repeated_undersized_guest();

        let error = runtime.build(&module, LIMITS).unwrap_err();

        assert_eq!(error.kind(), RuntimeErrorKind::GuestStatus);
    }

    #[test]
    fn runtime_rejects_an_allocator_pointer_outside_guest_memory() {
        let runtime = test_runtime(10_000);
        let module = guest_module::invalid_pointer_guest();

        let error = runtime.build(&module, LIMITS).unwrap_err();

        assert_eq!(error.kind(), RuntimeErrorKind::GuestMemory);
    }

    #[test]
    fn runtime_reports_a_guest_build_trap() {
        let runtime = test_runtime(10_000);
        let module = guest_module::trapping_build_guest();

        let error = runtime.build(&module, LIMITS).unwrap_err();

        assert_eq!(error.kind(), RuntimeErrorKind::Execution);
    }

    #[test]
    fn runtime_reports_build_fuel_exhaustion_separately_from_traps() {
        let runtime = test_runtime(100);
        let module = guest_module::infinite_build_guest();

        let error = runtime.build(&module, LIMITS).unwrap_err();

        assert_eq!(error.kind(), RuntimeErrorKind::FuelExhausted);
    }
}
#[cfg(test)]
mod guest_diagnostics_tests {
    use crate::{
        GuestDiagnostic, GuestDiagnosticCategory, GuestOperation, GuestSourceLocation,
        MAX_GUEST_DIAGNOSTIC_BYTES, StableId128,
    };

    #[test]
    fn structured_guest_diagnostics_are_bounded_and_canonical() {
        let diagnostic = GuestDiagnostic::new(
            GuestOperation::Build,
            GuestDiagnosticCategory::PropertyEncoding,
            "custom property codec rejected the value",
        )
        .with_widget("Container")
        .with_property("aimer.property:app::Container:decoration")
        .with_source(StableId128::from_bytes([0xAB; 16]))
        .with_limits(64, 65);

        let encoded = diagnostic.encode().unwrap();
        let decoded = GuestDiagnostic::decode(&encoded, MAX_GUEST_DIAGNOSTIC_BYTES).unwrap();

        assert_eq!(decoded, diagnostic);
        assert!(
            decoded
                .to_string()
                .contains("aimer_build: property encoding Container")
        );
        assert!(
            decoded
                .to_string()
                .contains("aimer.property:app::Container:decoration")
        );

        let mut malformed = encoded.clone();
        malformed[0] = b'X';
        assert!(GuestDiagnostic::decode(&malformed, MAX_GUEST_DIAGNOSTIC_BYTES).is_err());
        assert!(matches!(
            diagnostic.encode_with_limit(0),
            Err(crate::GuestDiagnosticEncodeError::InvalidLimit { maximum: 0 })
        ));
        assert!(
            GuestDiagnostic::decode(&encoded[..encoded.len() - 1], MAX_GUEST_DIAGNOSTIC_BYTES,)
                .is_err()
        );
        assert!(GuestDiagnostic::decode(&encoded, encoded.len() - 1).is_err());

        let oversized = GuestDiagnostic::new(
            GuestOperation::Build,
            GuestDiagnosticCategory::Application,
            "x".repeat(MAX_GUEST_DIAGNOSTIC_BYTES),
        );
        assert!(oversized.encode().is_err());
    }

    #[test]
    fn panic_diagnostic_round_trips_guest_source_location() {
        let diagnostic = GuestDiagnostic::new(
            GuestOperation::Build,
            GuestDiagnosticCategory::Panic,
            "called `Option::unwrap()` on a `None` value",
        )
        .with_widget("HttpRequestButton")
        .with_location(GuestSourceLocation::new(
            "target/aimer-hot-reload/application/src/http_request_button.rs",
            117,
            67,
        ));

        let encoded = diagnostic.encode().unwrap();
        let decoded = GuestDiagnostic::decode(&encoded, MAX_GUEST_DIAGNOSTIC_BYTES).unwrap();

        assert_eq!(decoded.category(), GuestDiagnosticCategory::Panic);
        assert_eq!(decoded.widget(), Some("HttpRequestButton"));
        assert_eq!(
            decoded.location().map(GuestSourceLocation::file),
            Some("target/aimer-hot-reload/application/src/http_request_button.rs"),
        );
        assert_eq!(decoded.location().map(GuestSourceLocation::line), Some(117));
        assert_eq!(
            decoded.location().map(GuestSourceLocation::column),
            Some(67)
        );
        assert!(
            decoded
                .to_string()
                .contains("target/aimer-hot-reload/application/src/http_request_button.rs:117:67")
        );
    }
}
#[cfg(test)]
mod manifest_tests {
    use crate::{
        AbiVersion, ApplicationManifest, CALLBACK_EVENT_FORMAT_VERSION, CapabilityPolicy,
        CapabilityRequirement, ManifestView, ModelError, ModelLimits, STATE_FORMAT_VERSION,
        StableId128, Version, WIDGET_IR_FORMAT_VERSION,
    };

    const LIMITS: ModelLimits = ModelLimits::new(512, 16, 64, 64);
    const PROGRAM_ID: StableId128 = StableId128::from_bytes([0x10; 16]);
    const CAPABILITY_ID: StableId128 = StableId128::from_bytes([0x20; 16]);
    const FINGERPRINT: [u8; 32] = [0x30; 32];
    #[rustfmt::skip]
    const EXPECTED_AMNF: &[u8] = &[
        b'A', b'M', b'N', b'F',
        1, 0, 0, 0,
        1, 0, 0, 0, 0, 0, 0, 0,
        1, 0, 0, 0, 0, 0, 0, 0,
        2, 0, 0, 0,
        2, 0, 0, 0,
        1, 0, 0, 0,
        0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10,
        0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10,
        1, 0, 0, 0,
        120, 0, 0, 0,
        0, 0, 0, 0,
        0x20, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20,
        0x20, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20,
        1, 0, 0, 0,
        1, 0, 0, 0,
        0x30, 0x30, 0x30, 0x30, 0x30, 0x30, 0x30, 0x30,
        0x30, 0x30, 0x30, 0x30, 0x30, 0x30, 0x30, 0x30,
        0x30, 0x30, 0x30, 0x30, 0x30, 0x30, 0x30, 0x30,
        0x30, 0x30, 0x30, 0x30, 0x30, 0x30, 0x30, 0x30,
    ];

    #[test]
    fn application_manifest_matches_the_version_one_golden_image() {
        let capabilities = [CapabilityRequirement::new(
            CAPABILITY_ID,
            1,
            CapabilityPolicy::Required,
            FINGERPRINT,
        )];
        let manifest = ApplicationManifest::new(
            AbiVersion::new(1, 0),
            AbiVersion::new(1, 0),
            WIDGET_IR_FORMAT_VERSION,
            CALLBACK_EVENT_FORMAT_VERSION,
            STATE_FORMAT_VERSION,
            PROGRAM_ID,
            &capabilities,
        );

        let bytes = manifest.encode(LIMITS).unwrap();
        let view = ManifestView::decode(&bytes, LIMITS).unwrap();

        assert_eq!(bytes, EXPECTED_AMNF);
        assert_eq!(view.minimum_abi(), AbiVersion::new(1, 0));
        assert_eq!(view.maximum_abi(), AbiVersion::new(1, 0));
        assert_eq!(view.widget_ir_version(), WIDGET_IR_FORMAT_VERSION);
        assert_eq!(view.callback_event_version(), CALLBACK_EVENT_FORMAT_VERSION);
        assert_eq!(view.state_version(), STATE_FORMAT_VERSION);
        assert_eq!(view.program_id(), PROGRAM_ID);
        assert_eq!(view.capability_count(), 1);
        let capability = view.capability(0).unwrap();
        assert_eq!(capability.capability_id(), CAPABILITY_ID);
        assert_eq!(capability.abi_major(), 1);
        assert_eq!(capability.policy(), CapabilityPolicy::Required);
        assert_eq!(capability.contract_fingerprint(), &FINGERPRINT);
    }

    #[test]
    fn application_manifest_rejects_an_inverted_abi_range() {
        let manifest = ApplicationManifest::new(
            AbiVersion::new(2, 0),
            AbiVersion::new(1, 0),
            Version::new(1, 0),
            Version::new(1, 0),
            Version::new(1, 0),
            PROGRAM_ID,
            &[],
        );

        let error = manifest.encode(LIMITS).unwrap_err();

        assert_eq!(error, ModelError::InvalidAbiRange);
    }

    #[test]
    fn application_manifest_rejects_duplicate_capability_ids() {
        let capabilities = [
            CapabilityRequirement::new(CAPABILITY_ID, 1, CapabilityPolicy::Required, FINGERPRINT),
            CapabilityRequirement::new(CAPABILITY_ID, 2, CapabilityPolicy::Optional, [0x40; 32]),
        ];
        let manifest = manifest_with_capabilities(&capabilities);

        let error = manifest.encode(LIMITS).unwrap_err();

        assert_eq!(
            error,
            ModelError::DuplicateCapabilityId {
                capability_id: CAPABILITY_ID,
            }
        );
    }

    #[test]
    fn application_manifest_rejects_noncanonical_capability_order() {
        let capabilities = [
            CapabilityRequirement::new(
                StableId128::from_bytes([0x21; 16]),
                1,
                CapabilityPolicy::Required,
                FINGERPRINT,
            ),
            CapabilityRequirement::new(CAPABILITY_ID, 1, CapabilityPolicy::Optional, [0x40; 32]),
        ];
        let manifest = manifest_with_capabilities(&capabilities);

        let error = manifest.encode(LIMITS).unwrap_err();

        assert_eq!(error, ModelError::NonCanonicalCapabilityOrder);
    }

    #[test]
    fn manifest_view_rejects_malformed_and_over_limit_images() {
        let truncated = ManifestView::decode(&EXPECTED_AMNF[..EXPECTED_AMNF.len() - 1], LIMITS);
        assert!(matches!(truncated, Err(ModelError::LengthMismatch { .. })));

        let mut reserved = EXPECTED_AMNF.to_vec();
        reserved[60] = 1;
        assert!(matches!(
            ManifestView::decode(&reserved, LIMITS),
            Err(ModelError::NonCanonicalReserved)
        ));

        let mut invalid_policy = EXPECTED_AMNF.to_vec();
        invalid_policy[84] = 3;
        assert!(matches!(
            ManifestView::decode(&invalid_policy, LIMITS),
            Err(ModelError::InvalidCapabilityPolicy { value: 3 })
        ));

        let tight_limits = ModelLimits::new(119, 16, 64, 64);
        assert!(matches!(
            ManifestView::decode(EXPECTED_AMNF, tight_limits),
            Err(ModelError::DocumentTooLarge {
                length: 120,
                limit: 119,
            })
        ));
    }

    fn manifest_with_capabilities<'a>(
        capabilities: &'a [CapabilityRequirement],
    ) -> ApplicationManifest<'a> {
        ApplicationManifest::new(
            AbiVersion::new(1, 0),
            AbiVersion::new(1, 0),
            WIDGET_IR_FORMAT_VERSION,
            CALLBACK_EVENT_FORMAT_VERSION,
            STATE_FORMAT_VERSION,
            PROGRAM_ID,
            capabilities,
        )
    }
}
#[cfg(test)]
mod portable_schema_metadata_tests {
    use crate::{
        BUILTIN_PORTABLE_WIDGET_SCHEMAS, BUILTIN_WIDGET_SCHEMA_VERSION, CallbackSchemaMetadata,
        ChildCardinality, EventId, ModelLimits, PROPERTY_CONTAINER_BOX_DECORATION,
        PROPERTY_CONTAINER_COLOR, PROPERTY_CONTAINER_HEIGHT, PROPERTY_CONTAINER_MARGIN,
        PROPERTY_CONTAINER_PADDING, PROPERTY_CONTAINER_WIDTH, PROPERTY_SIZED_BOX_HEIGHT,
        PROPERTY_SIZED_BOX_WIDTH, PROPERTY_TEXT_ALIGN, PROPERTY_TEXT_CONTENT, PROPERTY_TEXT_INDENT,
        PROPERTY_TEXT_LINE_HEIGHT, PROPERTY_TEXT_STYLE, PortableWidgetSchemaMetadata, PropertyId,
        PropertyPresence, PropertySchemaMetadata, PropertyValue, PropertyValueKind,
        ValueSchemaMetadata, ValueTypeId, Version, WIDGET_CONTAINER, WIDGET_SIZED_BOX, WIDGET_TEXT,
        WidgetDocument, WidgetNode, WidgetProperty, WidgetSchemaId, WidgetSchemaMetadata,
        stable_schema_hash64, validate_portable_widget_schema_metadata,
    };

    #[test]
    fn canonical_metadata_constructors_generate_all_permanent_identity_kinds() {
        const WIDGET_NAME: &str = "aimer.widget:test::IdentityCard";
        const PROPERTY_NAME: &str = "aimer.property:test::IdentityCard:title";
        const EVENT_NAME: &str = "aimer.event:test::IdentityCard:on_press";
        const VALUE_NAME: &str = "aimer.value:test::Title";
        let version = Version::new(1, 0);

        let widget = WidgetSchemaMetadata::from_canonical_name(WIDGET_NAME, version, version);
        let property = PropertySchemaMetadata::from_canonical_name(
            PROPERTY_NAME,
            PropertyValueKind::StringRef,
        );
        let callback = CallbackSchemaMetadata::from_canonical_name(EVENT_NAME, version, 1);
        let value = ValueSchemaMetadata::from_canonical_name(VALUE_NAME, version, 256);

        assert_eq!(widget.id(), WidgetSchemaId::new(0x775e_f3fd_0433_57ad));
        assert_eq!(property.id(), PropertyId::new(0x5d11_c1d1_0249_867a));
        assert_eq!(callback.id(), EventId::new(0x6e45_52c6_121c_bc9e));
        assert_eq!(value.id(), ValueTypeId::new(0xac04_62a2_64b7_68ee));
    }

    #[test]
    fn built_in_container_and_text_publish_complete_portable_schema_metadata() {
        validate_portable_widget_schema_metadata(&BUILTIN_PORTABLE_WIDGET_SCHEMAS).unwrap();

        let container = schema(WIDGET_CONTAINER);
        assert_eq!(container.children(), ChildCardinality::exactly(1));
        assert_eq!(container.properties().len(), 6);
        assert_property(
            container,
            PROPERTY_CONTAINER_WIDTH,
            PropertyValueKind::BlobRef,
            PropertyPresence::Optional,
        );
        assert_property(
            container,
            PROPERTY_CONTAINER_HEIGHT,
            PropertyValueKind::BlobRef,
            PropertyPresence::Optional,
        );
        assert_property(
            container,
            PROPERTY_CONTAINER_COLOR,
            PropertyValueKind::Rgba,
            PropertyPresence::Optional,
        );
        assert_property(
            container,
            PROPERTY_CONTAINER_PADDING,
            PropertyValueKind::BlobRef,
            PropertyPresence::Optional,
        );
        assert_property(
            container,
            PROPERTY_CONTAINER_MARGIN,
            PropertyValueKind::BlobRef,
            PropertyPresence::Optional,
        );
        assert_property(
            container,
            PROPERTY_CONTAINER_BOX_DECORATION,
            PropertyValueKind::BlobRef,
            PropertyPresence::Optional,
        );

        let sized_box = schema(WIDGET_SIZED_BOX);
        assert_property(
            sized_box,
            PROPERTY_SIZED_BOX_WIDTH,
            PropertyValueKind::BlobRef,
            PropertyPresence::Optional,
        );
        assert_property(
            sized_box,
            PROPERTY_SIZED_BOX_HEIGHT,
            PropertyValueKind::BlobRef,
            PropertyPresence::Optional,
        );

        let text = schema(WIDGET_TEXT);
        assert_eq!(text.children(), ChildCardinality::none());
        assert_eq!(text.properties().len(), 5);
        assert_property(
            text,
            PROPERTY_TEXT_CONTENT,
            PropertyValueKind::StringRef,
            PropertyPresence::Required,
        );
        assert_property(
            text,
            PROPERTY_TEXT_ALIGN,
            PropertyValueKind::I64,
            PropertyPresence::Optional,
        );
        assert_property(
            text,
            PROPERTY_TEXT_STYLE,
            PropertyValueKind::BlobRef,
            PropertyPresence::Optional,
        );
        assert_property(
            text,
            PROPERTY_TEXT_LINE_HEIGHT,
            PropertyValueKind::BlobRef,
            PropertyPresence::Optional,
        );
        assert_property(
            text,
            PROPERTY_TEXT_INDENT,
            PropertyValueKind::F64,
            PropertyPresence::Optional,
        );
    }

    #[test]
    fn identical_linked_schema_metadata_is_idempotent() {
        let schema = *schema(WIDGET_TEXT);

        assert!(validate_portable_widget_schema_metadata(&[schema, schema]).is_ok());
    }

    #[test]
    fn newer_guest_can_target_the_oldest_permanent_host_schema_version() {
        let validator =
            crate::PortableWidgetSchemaValidator::new(&BUILTIN_PORTABLE_WIDGET_SCHEMAS).unwrap();
        let future_property =
            PropertyId::from_canonical_name("aimer.property:aimer_text::Text:future_optional_hint");
        let properties = [
            WidgetProperty::new(PROPERTY_TEXT_CONTENT, PropertyValue::StringRef(0)),
            WidgetProperty::new(future_property, PropertyValue::Bool(true)).optional(),
        ];
        let nodes =
            [WidgetNode::new(WIDGET_TEXT, BUILTIN_WIDGET_SCHEMA_VERSION).properties(&properties)];
        let image = WidgetDocument::new(1, 1, 0, &nodes, &["newer guest"], &[])
            .encode(ModelLimits::new(4_096, 4, 16, 8))
            .unwrap();
        let document =
            crate::WidgetDocumentView::decode(&image, ModelLimits::new(4_096, 4, 16, 8)).unwrap();

        assert_eq!(BUILTIN_WIDGET_SCHEMA_VERSION, Version::new(1, 0));
        document.validate_schemas(&validator).unwrap();
    }

    #[test]
    fn portable_registry_rejects_a_property_identity_reused_by_another_widget() {
        const PROPERTY_NAME: &str = "aimer.property:test::First:value";
        let property = PropertySchemaMetadata::new(
            PropertyId::new(stable_schema_hash64(PROPERTY_NAME)),
            PROPERTY_NAME,
            PropertyValueKind::I64,
        );
        let properties = [property];
        let first = widget_schema("aimer.widget:test::First", &properties);
        let second = widget_schema("aimer.widget:test::Second", &properties);

        assert!(validate_portable_widget_schema_metadata(&[first, second]).is_err());
    }

    #[test]
    fn custom_blob_property_carries_bounded_versioned_value_metadata() {
        const PROPERTY_NAME: &str = "aimer.property:test::Chart:points";
        const VALUE_NAME: &str = "aimer.value:test::PointList";
        let value = ValueSchemaMetadata::new(
            ValueTypeId::new(stable_schema_hash64(VALUE_NAME)),
            VALUE_NAME,
            Version::new(2, 1),
            4_096,
        );
        let properties = [PropertySchemaMetadata::new(
            PropertyId::new(stable_schema_hash64(PROPERTY_NAME)),
            PROPERTY_NAME,
            PropertyValueKind::BlobRef,
        )
        .with_value_schema(value)];
        let schema = widget_schema("aimer.widget:test::Chart", &properties);

        validate_portable_widget_schema_metadata(&[schema]).unwrap();
        assert_eq!(schema.properties()[0].value_schema(), Some(value));
    }

    #[test]
    fn disjoint_versions_of_one_widget_may_reuse_unchanged_properties() {
        const WIDGET_NAME: &str = "aimer.widget:test::Versioned";
        const PROPERTY_NAME: &str = "aimer.property:test::Versioned:value";
        let widget_id = WidgetSchemaId::new(stable_schema_hash64(WIDGET_NAME));
        let properties = [PropertySchemaMetadata::new(
            PropertyId::new(stable_schema_hash64(PROPERTY_NAME)),
            PROPERTY_NAME,
            PropertyValueKind::I64,
        )];
        let first = PortableWidgetSchemaMetadata::new(
            WidgetSchemaMetadata::new(
                widget_id,
                WIDGET_NAME,
                Version::new(1, 0),
                Version::new(1, 0),
            ),
            &properties,
            &[],
            ChildCardinality::none(),
        );
        let second = PortableWidgetSchemaMetadata::new(
            WidgetSchemaMetadata::new(
                widget_id,
                WIDGET_NAME,
                Version::new(2, 0),
                Version::new(2, 0),
            ),
            &properties,
            &[],
            ChildCardinality::none(),
        );

        validate_portable_widget_schema_metadata(&[first, second]).unwrap();
    }

    fn schema(id: crate::WidgetSchemaId) -> &'static PortableWidgetSchemaMetadata<'static> {
        BUILTIN_PORTABLE_WIDGET_SCHEMAS
            .iter()
            .find(|schema| schema.widget().id() == id)
            .expect("built-in portable schema must exist")
    }

    fn assert_property(
        schema: &PortableWidgetSchemaMetadata<'_>,
        id: crate::PropertyId,
        kind: PropertyValueKind,
        presence: PropertyPresence,
    ) {
        let property = schema
            .properties()
            .iter()
            .find(|property| property.id() == id)
            .expect("portable property metadata must exist");
        assert_eq!(property.value_kind(), kind);
        assert_eq!(property.presence(), presence);
    }

    fn widget_schema<'a>(
        canonical_name: &'a str,
        properties: &'a [PropertySchemaMetadata<'a>],
    ) -> PortableWidgetSchemaMetadata<'a> {
        PortableWidgetSchemaMetadata::new(
            WidgetSchemaMetadata::new(
                WidgetSchemaId::new(stable_schema_hash64(canonical_name)),
                canonical_name,
                Version::new(1, 0),
                Version::new(1, 0),
            ),
            properties,
            &[],
            ChildCardinality::none(),
        )
    }
}
#[cfg(test)]
mod portable_schema_validation_tests {
    use crate::{
        AsyncCallbackSchemaMetadata, BUILTIN_PORTABLE_WIDGET_SCHEMAS, CallbackBinding,
        CallbackSchemaMetadata, ChildCardinality, EventId, ModelError, ModelLimits,
        PROPERTY_TEXT_CONTENT, PortableWidgetSchemaMetadata, PortableWidgetSchemaMetadataError,
        PortableWidgetSchemaValidator, PropertyId, PropertyValue, PropertyValueKind, StableId128,
        ValueSchemaMetadata, Version, WIDGET_CONTAINER, WIDGET_TEXT, WidgetDocument,
        WidgetDocumentView, WidgetFactory, WidgetMaterializeError, WidgetNode, WidgetNodeView,
        WidgetProperty, WidgetSchemaId, WidgetSchemaMetadata, WidgetSchemaSupport,
        materialize_widget_tree,
    };

    const LIMITS: ModelLimits = ModelLimits::new(8_192, 64, 256, 256).max_widget_depth(16);
    const VERSION: Version = Version::new(1, 0);

    #[test]
    fn validates_a_container_and_text_graph_from_builtin_metadata() {
        let text_properties = [WidgetProperty::new(
            PROPERTY_TEXT_CONTENT,
            PropertyValue::StringRef(0),
        )];
        let children = [1];
        let nodes = [
            WidgetNode::new(WIDGET_CONTAINER, VERSION).children(&children),
            WidgetNode::new(WIDGET_TEXT, VERSION).properties(&text_properties),
        ];

        validate(&nodes, &["hello"], &[], builtin_validator()).unwrap();
    }

    #[test]
    fn rejects_missing_required_property_and_wrong_wire_kind() {
        assert_eq!(
            validate(
                &[WidgetNode::new(WIDGET_TEXT, VERSION)],
                &[],
                &[],
                builtin_validator()
            ),
            Err(ModelError::MissingWidgetProperty {
                node: 0,
                widget_type: WIDGET_TEXT,
                property_id: PROPERTY_TEXT_CONTENT,
            })
        );

        let properties = [WidgetProperty::new(
            PROPERTY_TEXT_CONTENT,
            PropertyValue::I64(1),
        )];
        assert_eq!(
            validate(
                &[WidgetNode::new(WIDGET_TEXT, VERSION).properties(&properties)],
                &[],
                &[],
                builtin_validator(),
            ),
            Err(ModelError::InvalidWidgetPropertyType {
                node: 0,
                widget_type: WIDGET_TEXT,
                property_id: PROPERTY_TEXT_CONTENT,
            })
        );
    }

    #[test]
    fn rejects_duplicate_properties() {
        let properties = [
            WidgetProperty::new(PROPERTY_TEXT_CONTENT, PropertyValue::StringRef(0)),
            WidgetProperty::new(PROPERTY_TEXT_CONTENT, PropertyValue::StringRef(0)),
        ];

        assert_eq!(
            validate(
                &[WidgetNode::new(WIDGET_TEXT, VERSION).properties(&properties)],
                &["hello"],
                &[],
                builtin_validator(),
            ),
            Err(ModelError::DuplicateWidgetProperty {
                node: 0,
                widget_type: WIDGET_TEXT,
                property_id: PROPERTY_TEXT_CONTENT,
            })
        );
    }

    #[test]
    fn accepts_unknown_optional_property_and_rejects_unknown_required_property() {
        let unknown = PropertyId::new(99);
        let optional = [
            WidgetProperty::new(PROPERTY_TEXT_CONTENT, PropertyValue::StringRef(0)),
            WidgetProperty::new(unknown, PropertyValue::Bool(true)).optional(),
        ];
        validate(
            &[WidgetNode::new(WIDGET_TEXT, VERSION).properties(&optional)],
            &["hello"],
            &[],
            builtin_validator(),
        )
        .unwrap();

        let required = [
            WidgetProperty::new(PROPERTY_TEXT_CONTENT, PropertyValue::StringRef(0)),
            WidgetProperty::new(unknown, PropertyValue::Bool(true)),
        ];
        assert_eq!(
            validate(
                &[WidgetNode::new(WIDGET_TEXT, VERSION).properties(&required)],
                &["hello"],
                &[],
                builtin_validator(),
            ),
            Err(ModelError::UnsupportedWidgetProperty {
                node: 0,
                widget_type: WIDGET_TEXT,
                property_id: unknown,
            })
        );
    }

    #[test]
    fn enforces_child_cardinality() {
        assert_eq!(
            validate(
                &[WidgetNode::new(WIDGET_CONTAINER, VERSION)],
                &[],
                &[],
                builtin_validator(),
            ),
            Err(ModelError::InvalidWidgetChildCount {
                node: 0,
                widget_type: WIDGET_CONTAINER,
                count: 0,
                minimum: 1,
                maximum: 1,
            })
        );
    }

    #[test]
    fn enforces_callback_schema_count_and_unique_stable_identity() {
        const EVENT_NAME: &str = "aimer.event:test::Action:on_run";
        const WIDGET_NAME: &str = "aimer.widget:test::Action";
        let event = EventId::from_canonical_name(EVENT_NAME);
        let callbacks = [CallbackSchemaMetadata::from_canonical_name(
            EVENT_NAME, VERSION, 1,
        )];
        let schemas = [PortableWidgetSchemaMetadata::new(
            WidgetSchemaMetadata::from_canonical_name(WIDGET_NAME, VERSION, VERSION),
            &[],
            &callbacks,
            ChildCardinality::none(),
        )];
        let validator = PortableWidgetSchemaValidator::new(&schemas).unwrap();
        let widget = schemas[0].widget().id();
        let first_id = StableId128::from_bytes([1; 16]);
        let second_id = StableId128::from_bytes([2; 16]);

        let wrong_version = [CallbackBinding::new(event, Version::new(1, 1), first_id)];
        assert_eq!(
            validate(
                &[WidgetNode::new(widget, VERSION).callbacks(&wrong_version)],
                &[],
                &[],
                validator,
            ),
            Err(ModelError::UnsupportedWidgetCallback {
                node: 0,
                widget_type: widget,
                event_kind: event,
            })
        );

        let unknown_event = EventId::new(99);
        let wrong_identity = [CallbackBinding::new(unknown_event, VERSION, first_id)];
        assert_eq!(
            validate(
                &[WidgetNode::new(widget, VERSION).callbacks(&wrong_identity)],
                &[],
                &[],
                validator,
            ),
            Err(ModelError::UnsupportedWidgetCallback {
                node: 0,
                widget_type: widget,
                event_kind: unknown_event,
            })
        );

        let too_many = [
            CallbackBinding::new(event, VERSION, first_id),
            CallbackBinding::new(event, VERSION, second_id),
        ];
        assert_eq!(
            validate(
                &[WidgetNode::new(widget, VERSION).callbacks(&too_many)],
                &[],
                &[],
                validator,
            ),
            Err(ModelError::InvalidWidgetCallbackCount {
                node: 0,
                widget_type: widget,
                count: 2,
                maximum: 1,
            })
        );

        let duplicate = [
            CallbackBinding::new(event, VERSION, first_id),
            CallbackBinding::new(event, VERSION, first_id),
        ];
        assert_eq!(
            validate(
                &[WidgetNode::new(widget, VERSION).callbacks(&duplicate)],
                &[],
                &[],
                validator,
            ),
            Err(ModelError::DuplicateWidgetCallback {
                node: 0,
                widget_type: widget,
                callback_id: first_id,
            })
        );
    }

    #[test]
    fn async_callback_contract_is_explicit_and_older_schema_rejects_it() {
        const EVENT_NAME: &str = "aimer.event:test::Action:on_run_async";
        const WIDGET_NAME: &str = "aimer.widget:test::ActionAsync";
        let event = EventId::from_canonical_name(EVENT_NAME);
        let async_schema = AsyncCallbackSchemaMetadata::new(VERSION, 2, 8);
        let callbacks = [
            CallbackSchemaMetadata::from_canonical_name(EVENT_NAME, VERSION, 1)
                .with_async_schema(async_schema),
        ];
        let schemas = [PortableWidgetSchemaMetadata::new(
            WidgetSchemaMetadata::from_canonical_name(WIDGET_NAME, VERSION, VERSION),
            &[],
            &callbacks,
            ChildCardinality::none(),
        )];
        let widget = schemas[0].widget().id();
        let callback_id = StableId128::from_bytes([8; 16]);
        let binding = [CallbackBinding::new_async(
            event,
            VERSION,
            VERSION,
            callback_id,
        )];

        validate(
            &[WidgetNode::new(widget, VERSION).callbacks(&binding)],
            &[],
            &[],
            PortableWidgetSchemaValidator::new(&schemas).unwrap(),
        )
        .unwrap();

        let old_callbacks = [CallbackSchemaMetadata::from_canonical_name(
            EVENT_NAME, VERSION, 1,
        )];
        let old_schemas = [PortableWidgetSchemaMetadata::new(
            WidgetSchemaMetadata::from_canonical_name(WIDGET_NAME, VERSION, VERSION),
            &[],
            &old_callbacks,
            ChildCardinality::none(),
        )];
        assert_eq!(
            validate(
                &[WidgetNode::new(widget, VERSION).callbacks(&binding)],
                &[],
                &[],
                PortableWidgetSchemaValidator::new(&old_schemas).unwrap(),
            ),
            Err(ModelError::UnsupportedAsyncCallback {
                node: 0,
                widget_type: widget,
                event_kind: event,
                version: VERSION,
            })
        );
    }

    #[test]
    fn bounds_custom_blob_payload_through_the_document_view() {
        const PROPERTY_NAME: &str = "aimer.property:test::Chart:points";
        const VALUE_NAME: &str = "aimer.value:test::Points";
        const WIDGET_NAME: &str = "aimer.widget:test::Chart";
        let property = PropertyId::from_canonical_name(PROPERTY_NAME);
        let properties = [crate::PropertySchemaMetadata::from_canonical_name(
            PROPERTY_NAME,
            PropertyValueKind::BlobRef,
        )
        .with_value_schema(ValueSchemaMetadata::from_canonical_name(
            VALUE_NAME, VERSION, 3,
        ))];
        let schemas = [PortableWidgetSchemaMetadata::new(
            WidgetSchemaMetadata::from_canonical_name(WIDGET_NAME, VERSION, VERSION),
            &properties,
            &[],
            ChildCardinality::none(),
        )];
        let widget = schemas[0].widget().id();
        let node_properties = [WidgetProperty::new(property, PropertyValue::BlobRef(0))];
        let nodes = [WidgetNode::new(widget, VERSION).properties(&node_properties)];

        assert_eq!(
            validate(
                &nodes,
                &[],
                &[&[1, 2, 3, 4]],
                PortableWidgetSchemaValidator::new(&schemas).unwrap(),
            ),
            Err(ModelError::InvalidWidgetPropertyValue {
                node: 0,
                widget_type: widget,
                property_id: property,
            })
        );
    }

    #[test]
    fn exact_lookup_rejects_unknown_widgets_and_versions_outside_inclusive_range() {
        const WIDGET_NAME: &str = "aimer.widget:test::Versioned";
        let schemas = [PortableWidgetSchemaMetadata::new(
            WidgetSchemaMetadata::from_canonical_name(
                WIDGET_NAME,
                Version::new(1, 0),
                Version::new(1, 2),
            ),
            &[],
            &[],
            ChildCardinality::none(),
        )];
        let validator = PortableWidgetSchemaValidator::new(&schemas).unwrap();
        let widget = schemas[0].widget().id();
        let unknown = WidgetSchemaId::new(99);
        assert!(!validator.supports(unknown, VERSION));
        assert!(!validator.supports(widget, Version::new(0, 9)));
        assert!(!validator.supports(widget, Version::new(1, 3)));
        assert!(validator.supports(widget, Version::new(1, 0)));
        assert!(validator.supports(widget, Version::new(1, 1)));
        assert!(validator.supports(widget, Version::new(1, 2)));
    }

    #[test]
    fn constructor_rejects_invalid_metadata() {
        const WIDGET_NAME: &str = "aimer.widget:test::Broken";
        let schemas = [PortableWidgetSchemaMetadata::new(
            WidgetSchemaMetadata::from_canonical_name(WIDGET_NAME, VERSION, VERSION),
            &[],
            &[],
            ChildCardinality::new(2, 1),
        )];

        assert!(matches!(
            PortableWidgetSchemaValidator::new(&schemas),
            Err(PortableWidgetSchemaMetadataError::InvalidChildCardinality { .. })
        ));
    }

    #[test]
    fn schema_failure_prevents_factory_builds() {
        let nodes = [WidgetNode::new(WIDGET_TEXT, VERSION)];
        let image = encode(&nodes, &[], &[]);
        let validator = builtin_validator();
        let mut factory = RecordingFactory {
            validator: &validator,
            builds: 0,
        };

        let error = materialize_widget_tree(&image, LIMITS, &mut factory).unwrap_err();

        assert!(matches!(
            error,
            WidgetMaterializeError::Model(ModelError::MissingWidgetProperty { .. })
        ));
        assert_eq!(factory.builds, 0);
    }

    fn builtin_validator() -> PortableWidgetSchemaValidator<'static> {
        PortableWidgetSchemaValidator::new(&BUILTIN_PORTABLE_WIDGET_SCHEMAS).unwrap()
    }

    fn validate(
        nodes: &[WidgetNode<'_>],
        strings: &[&str],
        blobs: &[&[u8]],
        validator: PortableWidgetSchemaValidator<'_>,
    ) -> Result<(), ModelError> {
        let image = encode(nodes, strings, blobs);
        WidgetDocumentView::decode(&image, LIMITS)
            .unwrap()
            .validate_schemas(&validator)
    }

    fn encode(nodes: &[WidgetNode<'_>], strings: &[&str], blobs: &[&[u8]]) -> Vec<u8> {
        WidgetDocument::new(1, 1, 0, nodes, strings, blobs)
            .encode(LIMITS)
            .unwrap()
    }

    struct RecordingFactory<'a> {
        validator: &'a PortableWidgetSchemaValidator<'a>,
        builds: usize,
    }

    impl WidgetSchemaSupport for RecordingFactory<'_> {
        fn supports(&self, widget_type: WidgetSchemaId, schema: Version) -> bool {
            self.validator.supports(widget_type, schema)
        }

        fn validate_node(
            &self,
            document: &WidgetDocumentView<'_>,
            node_index: u32,
            node: WidgetNodeView<'_>,
        ) -> Result<(), ModelError> {
            self.validator.validate_node(document, node_index, node)
        }
    }

    impl WidgetFactory for RecordingFactory<'_> {
        type Node = ();
        type Error = ();

        fn build(
            &mut self,
            _document: &WidgetDocumentView<'_>,
            _node_index: u32,
            _node: WidgetNodeView<'_>,
            _children: Vec<Self::Node>,
        ) -> Result<Self::Node, Self::Error> {
            self.builds += 1;
            Ok(())
        }
    }
}
#[cfg(test)]
mod reload_transaction_tests {
    use std::cell::Cell;
    use std::cell::RefCell;
    use std::rc::Rc;

    use crate::{
        CallbackBindingSnapshot, Generation, GenerationId, GenerationLimits, GenerationResource,
        GenerationResourceError, GenerationResourceKind, ReloadCoordinator, ReloadEventDisposition,
        ReloadGuest, ReloadSnapshot, ReloadStage, ReloadTransactionError,
    };

    #[test]
    fn rejected_candidate_preserves_the_active_snapshot_and_replays_fifo_once() {
        let lifecycle = Rc::new(RefCell::new(Vec::new()));
        let active = snapshot(1, "old-root", lifecycle.clone());
        let mut coordinator = ReloadCoordinator::new(active).max_queued_events(3);
        let transaction = coordinator.begin_reload();
        coordinator
            .stage_candidate(
                transaction,
                snapshot(2, "candidate-root", lifecycle.clone()),
            )
            .unwrap();

        assert_eq!(
            coordinator.route_event("first").unwrap(),
            ReloadEventDisposition::Queued
        );
        assert_eq!(
            coordinator.route_event("second").unwrap(),
            ReloadEventDisposition::Queued
        );
        let rejection = coordinator
            .commit(
                transaction,
                |_old, _candidate| Err("prepare failed"),
                |_old, _candidate| unreachable!("a rejected candidate cannot mutate native state"),
            )
            .unwrap_err();

        assert_eq!(rejection.preflight_error(), Some(&"prepare failed"));
        assert_eq!(rejection.replay().unwrap().as_slice(), &["first", "second"]);
        assert_eq!(coordinator.active().generation_id(), GenerationId::new(1));
        assert_eq!(coordinator.active().root(), &"old-root");
        assert_eq!(lifecycle.borrow().as_slice(), ["candidate-retired"]);
        assert_eq!(
            rejection
                .into_replay()
                .unwrap()
                .into_iter()
                .collect::<Vec<_>>(),
            ["first", "second"]
        );
        assert_eq!(
            coordinator.route_event("after").unwrap(),
            ReloadEventDisposition::Dispatch("after")
        );
    }

    #[test]
    fn successful_commit_installs_one_coherent_snapshot_and_retires_the_old_generation() {
        let lifecycle = Rc::new(RefCell::new(Vec::new()));
        let active = snapshot(1, "old-root", lifecycle.clone());
        let mut coordinator = ReloadCoordinator::new(active).max_queued_events(4);
        let transaction = coordinator.begin_reload();
        coordinator
            .stage_candidate(transaction, snapshot(2, "new-root", lifecycle.clone()))
            .unwrap();
        coordinator.route_event(10).unwrap();
        coordinator.route_event(20).unwrap();
        let commit_observations = Rc::new(RefCell::new(Vec::new()));
        let observations = commit_observations.clone();

        let committed = coordinator
            .commit(
                transaction,
                |old, candidate| {
                    assert_eq!(old.root(), &"old-root");
                    assert_eq!(candidate.root(), &"new-root");
                    Ok::<(), ()>(())
                },
                move |old, candidate| {
                    observations
                        .borrow_mut()
                        .push((old.generation_id().get(), candidate.generation_id().get()));
                },
            )
            .unwrap();

        assert_eq!(commit_observations.borrow().as_slice(), [(1, 2)]);
        assert_eq!(coordinator.active().generation_id(), GenerationId::new(2));
        assert_eq!(coordinator.active().root(), &"new-root");
        assert_eq!(committed.replay().as_slice(), &[10, 20]);
        assert_eq!(
            lifecycle.borrow().as_slice(),
            ["candidate-activated", "old-retired"]
        );
    }

    #[test]
    fn prepared_commit_carries_the_single_preflight_artifact_into_the_infallible_swap() {
        let lifecycle = Rc::new(RefCell::new(Vec::new()));
        let active = snapshot(1, "old-root", lifecycle);
        let mut coordinator: ReloadCoordinator<RecordingGuest, _, ()> =
            ReloadCoordinator::new(active);
        let transaction = coordinator.begin_reload();
        coordinator
            .stage_candidate(
                transaction,
                snapshot(2, "new-root", Rc::new(RefCell::new(Vec::new()))),
            )
            .unwrap();
        let preflight_calls = Cell::new(0_u32);

        coordinator
            .commit_prepared(
                transaction,
                |old, candidate| {
                    preflight_calls.set(preflight_calls.get() + 1);
                    Ok::<_, ()>((old.generation_id(), candidate.generation_id()))
                },
                |old, candidate, prepared| {
                    assert_eq!(prepared, (old.generation_id(), candidate.generation_id()));
                },
            )
            .unwrap();

        assert_eq!(preflight_calls.get(), 1);
        assert_eq!(coordinator.active().generation_id(), GenerationId::new(2));
    }

    #[test]
    fn newer_candidate_supersedes_and_destroys_the_staged_candidate() {
        let lifecycle = Rc::new(RefCell::new(Vec::new()));
        let active = snapshot(1, "old-root", lifecycle.clone());
        let mut coordinator = ReloadCoordinator::new(active).max_queued_events(2);
        let first = coordinator.begin_reload();
        coordinator
            .stage_candidate(first, snapshot(2, "stale-root", lifecycle.clone()))
            .unwrap();
        coordinator.route_event(7).unwrap();

        let second = coordinator.begin_reload();

        assert_ne!(first, second);
        assert_eq!(lifecycle.borrow().as_slice(), ["candidate-retired"]);
        assert_eq!(coordinator.queued_event_count(), 1);
        let stale =
            coordinator.stage_candidate(first, snapshot(3, "also-stale", lifecycle.clone()));
        assert!(matches!(
            stale,
            Err(ReloadTransactionError::Superseded { .. })
        ));
        assert_eq!(
            lifecycle.borrow().as_slice(),
            ["candidate-retired", "also-stale-retired"]
        );
    }

    #[test]
    fn event_barrier_is_bounded_without_dropping_the_overflowing_event() {
        let lifecycle = Rc::new(RefCell::new(Vec::new()));
        let active = snapshot(1, "old-root", lifecycle);
        let mut coordinator = ReloadCoordinator::new(active).max_queued_events(1);
        coordinator.begin_reload();
        coordinator.route_event("queued").unwrap();

        let overflow = coordinator.route_event("overflow").unwrap_err();

        assert_eq!(overflow.event(), &"overflow");
        assert_eq!(overflow.limit(), 1);
        assert_eq!(coordinator.queued_event_count(), 1);
    }

    #[test]
    fn replay_attempts_every_event_once_and_reports_removed_callback_failures() {
        let lifecycle = Rc::new(RefCell::new(Vec::new()));
        let active = snapshot(1, "old-root", lifecycle);
        let mut coordinator = ReloadCoordinator::new(active).max_queued_events(3);
        let transaction = coordinator.begin_reload();
        coordinator.route_event("retained").unwrap();
        coordinator.route_event("removed").unwrap();
        coordinator.route_event("later").unwrap();
        let replay = coordinator.rollback(transaction).unwrap();
        let attempted = Rc::new(RefCell::new(Vec::new()));
        let observed = attempted.clone();

        let report = replay.dispatch(move |event| {
            observed.borrow_mut().push(event);
            if event == "removed" {
                Err("callback was removed")
            } else {
                Ok(())
            }
        });

        assert_eq!(
            attempted.borrow().as_slice(),
            ["retained", "removed", "later"]
        );
        assert_eq!(report.attempted_events(), 3);
        assert_eq!(report.delivered_events(), 2);
        assert_eq!(report.failures().len(), 1);
        assert_eq!(report.failures()[0].event_index(), 1);
        assert_eq!(report.failures()[0].error(), &"callback was removed");
    }

    #[test]
    fn every_fallible_precommit_stage_rolls_back_to_the_exact_old_snapshot() {
        let stages = [
            ReloadStage::Preflight,
            ReloadStage::Instantiate,
            ReloadStage::Initialize,
            ReloadStage::ExportState,
            ReloadStage::MigrateState,
            ReloadStage::ImportState,
            ReloadStage::Build,
            ReloadStage::Validate,
            ReloadStage::Materialize,
            ReloadStage::PrepareReconciliation,
            ReloadStage::PreCommitCancellation,
        ];

        for stage in stages {
            let lifecycle = Rc::new(RefCell::new(Vec::new()));
            let active = snapshot(1, "old-root", lifecycle.clone());
            let mut coordinator = ReloadCoordinator::new(active).max_queued_events(1);
            let transaction = coordinator.begin_reload();
            coordinator.route_event("queued").unwrap();
            if stage != ReloadStage::Preflight && stage != ReloadStage::Instantiate {
                coordinator
                    .stage_candidate(
                        transaction,
                        snapshot(2, "candidate-root", lifecycle.clone()),
                    )
                    .unwrap();
            }

            let rejection = coordinator
                .reject(transaction, stage, "injected failure")
                .unwrap();

            assert_eq!(rejection.stage(), stage);
            assert_eq!(rejection.error(), &"injected failure");
            assert_eq!(rejection.replay().as_slice(), &["queued"]);
            assert_eq!(coordinator.active().generation_id(), GenerationId::new(1));
            assert_eq!(coordinator.active().root(), &"old-root");
            if stage != ReloadStage::Preflight && stage != ReloadStage::Instantiate {
                assert_eq!(lifecycle.borrow().as_slice(), ["candidate-retired"]);
            } else {
                assert!(lifecycle.borrow().is_empty());
            }
        }
    }

    #[test]
    fn commit_retires_old_resources_and_rejects_their_late_completions() {
        let lifecycle = Rc::new(RefCell::new(Vec::new()));
        let releases = Rc::new(Cell::new(0));
        let active = ReloadSnapshot::new(
            Generation::with_guest(
                GenerationId::new(1),
                CallbackBindingSnapshot::empty(),
                aimer_venus::LocalScheduler::new(),
                GenerationLimits::new(1),
                RecordingGuest {
                    name: "old",
                    lifecycle: lifecycle.clone(),
                },
            ),
            "old-root",
        );
        let mut coordinator: ReloadCoordinator<_, _, ()> =
            ReloadCoordinator::new(active).max_queued_events(0);
        coordinator
            .active_mut()
            .generation_mut()
            .register_resource(
                GenerationResourceKind::Timer,
                RecordingResource(releases.clone()),
            )
            .unwrap();
        let old_completion = coordinator.active().generation().completion_token();
        let transaction = coordinator.begin_reload();
        coordinator
            .stage_candidate(transaction, snapshot(2, "new-root", lifecycle))
            .unwrap();

        coordinator
            .commit(
                transaction,
                |_old, _candidate| Ok::<(), ()>(()),
                |_old, _candidate| {},
            )
            .unwrap();

        assert_eq!(releases.get(), 1);
        let delivered = Rc::new(Cell::new(0));
        let observed = delivered.clone();
        assert_eq!(
            old_completion.deliver(move || observed.set(1)),
            Err(GenerationResourceError::RetiredGeneration)
        );
        assert_eq!(delivered.get(), 0);
    }

    struct RecordingResource(Rc<Cell<u32>>);

    impl GenerationResource for RecordingResource {
        fn release(self: Box<Self>) {
            self.0.set(self.0.get() + 1);
        }
    }

    fn snapshot(
        generation_id: u64,
        root: &'static str,
        lifecycle: Rc<RefCell<Vec<&'static str>>>,
    ) -> ReloadSnapshot<RecordingGuest, &'static str> {
        let callbacks = CallbackBindingSnapshot::empty();
        let guest = RecordingGuest {
            name: if generation_id == 1 {
                "old"
            } else if generation_id == 2 {
                "candidate"
            } else {
                "also-stale"
            },
            lifecycle,
        };
        ReloadSnapshot::new(
            Generation::with_guest(
                GenerationId::new(generation_id),
                callbacks,
                aimer_venus::LocalScheduler::new(),
                crate::GenerationLimits::new(0),
                guest,
            ),
            root,
        )
    }

    struct RecordingGuest {
        name: &'static str,
        lifecycle: Rc<RefCell<Vec<&'static str>>>,
    }

    impl ReloadGuest for RecordingGuest {
        fn activate(&mut self) {
            self.lifecycle.borrow_mut().push(match self.name {
                "candidate" => "candidate-activated",
                _ => "unexpected-activated",
            });
        }

        fn retire(&mut self) {
            self.lifecycle.borrow_mut().push(match self.name {
                "old" => "old-retired",
                "candidate" => "candidate-retired",
                "also-stale" => "also-stale-retired",
                _ => "unexpected-retired",
            });
        }
    }
}
#[cfg(test)]
mod state_transfer_tests {
    use crate::{
        ModelLimits, StableId128, StateBundle, StateBundleView, StateEntry, StateMigration,
        StateMigrationFailure, StatePolicy, StateTransferCoordinator, Version,
    };

    const LIMITS: ModelLimits = ModelLimits::new(1_024, 8, 64, 128);
    const APPLICATION_ID: StableId128 = StableId128::from_bytes([0x10; 16]);
    const SESSION_ID: StableId128 = StableId128::from_bytes([0x20; 16]);
    const SETTINGS_ID: StableId128 = StableId128::from_bytes([0x30; 16]);
    const SESSION_SCHEMA: StableId128 = StableId128::from_bytes([0x40; 16]);
    const SETTINGS_SCHEMA: StableId128 = StableId128::from_bytes([0x50; 16]);

    #[test]
    fn transfer_preserves_matching_state_and_keeps_new_state_defaults() {
        let previous_payload = [7];
        let previous_entries = [StateEntry::new(
            SESSION_ID,
            SESSION_SCHEMA,
            Version::new(1, 0),
            StatePolicy::Required,
            &previous_payload,
        )];
        let previous = StateBundle::new(APPLICATION_ID, 3, &previous_entries)
            .encode(LIMITS)
            .unwrap();
        let stale_payload = [0];
        let settings_default = [9];
        let candidate_entries = [
            StateEntry::new(
                SESSION_ID,
                SESSION_SCHEMA,
                Version::new(1, 0),
                StatePolicy::Required,
                &stale_payload,
            ),
            StateEntry::new(
                SETTINGS_ID,
                SETTINGS_SCHEMA,
                Version::new(1, 0),
                StatePolicy::ResetSafe,
                &settings_default,
            ),
        ];
        let candidate_defaults = StateBundle::new(APPLICATION_ID, 4, &candidate_entries)
            .encode(LIMITS)
            .unwrap();
        let coordinator = StateTransferCoordinator::new()
            .model_limits(LIMITS)
            .migration_fuel(8);

        let prepared = coordinator.prepare(&previous, &candidate_defaults).unwrap();

        let transferred = StateBundleView::decode(prepared.as_bytes(), LIMITS).unwrap();
        assert_eq!(transferred.source_generation(), 4);
        assert_eq!(transferred.entry(0).unwrap().payload(), previous_payload);
        assert_eq!(transferred.entry(1).unwrap().payload(), settings_default);
        assert_eq!(prepared.report().preserved_entries(), 1);
        assert_eq!(prepared.report().defaulted_entries(), 1);
        assert_eq!(prepared.report().state_bytes(), prepared.as_bytes().len());
        assert!(prepared.report().migrated_state_ids().is_empty());
        assert!(prepared.report().reset_state_ids().is_empty());
    }

    #[test]
    fn transfer_explicitly_reports_a_removed_reset_safe_entry() {
        let payload = [7];
        let previous_entries = [StateEntry::new(
            SESSION_ID,
            SESSION_SCHEMA,
            Version::new(1, 0),
            StatePolicy::ResetSafe,
            &payload,
        )];
        let previous = StateBundle::new(APPLICATION_ID, 3, &previous_entries)
            .encode(LIMITS)
            .unwrap();
        let candidate_defaults = StateBundle::new(APPLICATION_ID, 4, &[])
            .encode(LIMITS)
            .unwrap();
        let coordinator = StateTransferCoordinator::new()
            .model_limits(LIMITS)
            .migration_fuel(8);

        let prepared = coordinator.prepare(&previous, &candidate_defaults).unwrap();

        let transferred = StateBundleView::decode(prepared.as_bytes(), LIMITS).unwrap();
        assert_eq!(transferred.entry_count(), 0);
        assert_eq!(prepared.report().reset_state_ids(), &[SESSION_ID]);
    }

    #[test]
    fn transfer_applies_a_bounded_multi_version_migration() {
        let previous_payload = [1];
        let previous_entries = [StateEntry::new(
            SESSION_ID,
            SESSION_SCHEMA,
            Version::new(1, 0),
            StatePolicy::Required,
            &previous_payload,
        )];
        let previous = StateBundle::new(APPLICATION_ID, 3, &previous_entries)
            .encode(LIMITS)
            .unwrap();
        let candidate_default = [0, 0, 0];
        let candidate_entries = [StateEntry::new(
            SESSION_ID,
            SESSION_SCHEMA,
            Version::new(3, 0),
            StatePolicy::Required,
            &candidate_default,
        )];
        let candidate_defaults = StateBundle::new(APPLICATION_ID, 4, &candidate_entries)
            .encode(LIMITS)
            .unwrap();
        let mut coordinator = StateTransferCoordinator::new()
            .model_limits(LIMITS)
            .migration_fuel(5);
        coordinator
            .register_migration(StateMigration::new(
                SESSION_ID,
                SESSION_SCHEMA,
                Version::new(1, 0),
                SESSION_SCHEMA,
                Version::new(2, 0),
                2,
                migrate_v1_to_v2,
            ))
            .unwrap();
        coordinator
            .register_migration(StateMigration::new(
                SESSION_ID,
                SESSION_SCHEMA,
                Version::new(2, 0),
                SESSION_SCHEMA,
                Version::new(3, 0),
                3,
                migrate_v2_to_v3,
            ))
            .unwrap();

        let prepared = coordinator.prepare(&previous, &candidate_defaults).unwrap();

        let transferred = StateBundleView::decode(prepared.as_bytes(), LIMITS).unwrap();
        assert_eq!(transferred.entry(0).unwrap().payload(), &[1, 2, 3]);
        assert_eq!(prepared.report().migrated_state_ids(), &[SESSION_ID]);
        assert_eq!(prepared.report().migration_fuel_consumed(), 5);
    }

    fn migrate_v1_to_v2(payload: &[u8]) -> Result<Vec<u8>, StateMigrationFailure> {
        let mut migrated = payload.to_vec();
        migrated.push(2);
        Ok(migrated)
    }

    fn migrate_v2_to_v3(payload: &[u8]) -> Result<Vec<u8>, StateMigrationFailure> {
        let mut migrated = payload.to_vec();
        migrated.push(3);
        Ok(migrated)
    }

    #[test]
    fn transfer_rejects_a_post_import_verification_export_mismatch() {
        let previous_payload = [7];
        let previous_entries = [StateEntry::new(
            SESSION_ID,
            SESSION_SCHEMA,
            Version::new(1, 0),
            StatePolicy::Required,
            &previous_payload,
        )];
        let previous = StateBundle::new(APPLICATION_ID, 3, &previous_entries)
            .encode(LIMITS)
            .unwrap();
        let candidate_default = [0];
        let candidate_entries = [StateEntry::new(
            SESSION_ID,
            SESSION_SCHEMA,
            Version::new(1, 0),
            StatePolicy::Required,
            &candidate_default,
        )];
        let candidate_defaults = StateBundle::new(APPLICATION_ID, 4, &candidate_entries)
            .encode(LIMITS)
            .unwrap();
        let coordinator = StateTransferCoordinator::new()
            .model_limits(LIMITS)
            .migration_fuel(8);
        let prepared = coordinator.prepare(&previous, &candidate_defaults).unwrap();
        let incorrect_payload = [8];
        let incorrect_entries = [StateEntry::new(
            SESSION_ID,
            SESSION_SCHEMA,
            Version::new(1, 0),
            StatePolicy::Required,
            &incorrect_payload,
        )];
        let verification_export = StateBundle::new(APPLICATION_ID, 4, &incorrect_entries)
            .encode(LIMITS)
            .unwrap();

        let error = coordinator
            .verify(&prepared, &verification_export)
            .unwrap_err();

        assert!(matches!(
            error,
            crate::StateTransferError::VerificationMismatch
        ));
    }

    #[test]
    fn transfer_rejects_removed_required_state() {
        let payload = [7];
        let entries = [StateEntry::new(
            SESSION_ID,
            SESSION_SCHEMA,
            Version::new(1, 0),
            StatePolicy::Required,
            &payload,
        )];
        let previous = StateBundle::new(APPLICATION_ID, 3, &entries)
            .encode(LIMITS)
            .unwrap();
        let candidate = StateBundle::new(APPLICATION_ID, 4, &[])
            .encode(LIMITS)
            .unwrap();
        let coordinator = StateTransferCoordinator::new().model_limits(LIMITS);

        let error = coordinator.prepare(&previous, &candidate).unwrap_err();

        assert!(matches!(
            error,
            crate::StateTransferError::StateIncompatible {
                state_id: SESSION_ID,
                policy: StatePolicy::Required,
            }
        ));
    }

    #[test]
    fn transfer_uses_the_candidate_default_for_incompatible_reset_safe_state() {
        let previous_payload = [7];
        let previous_entries = [StateEntry::new(
            SESSION_ID,
            SESSION_SCHEMA,
            Version::new(1, 0),
            StatePolicy::ResetSafe,
            &previous_payload,
        )];
        let previous = StateBundle::new(APPLICATION_ID, 3, &previous_entries)
            .encode(LIMITS)
            .unwrap();
        let default_payload = [9];
        let candidate_entries = [StateEntry::new(
            SESSION_ID,
            SESSION_SCHEMA,
            Version::new(2, 0),
            StatePolicy::ResetSafe,
            &default_payload,
        )];
        let candidate = StateBundle::new(APPLICATION_ID, 4, &candidate_entries)
            .encode(LIMITS)
            .unwrap();
        let coordinator = StateTransferCoordinator::new().model_limits(LIMITS);

        let prepared = coordinator.prepare(&previous, &candidate).unwrap();

        let state = StateBundleView::decode(prepared.as_bytes(), LIMITS).unwrap();
        assert_eq!(state.entry(0).unwrap().payload(), default_payload);
        assert_eq!(prepared.report().reset_state_ids(), &[SESSION_ID]);
    }

    #[test]
    fn transfer_rejects_a_migration_that_exceeds_its_candidate_fuel() {
        let (previous, candidate) = schema_upgrade_images();
        let mut coordinator = StateTransferCoordinator::new()
            .model_limits(LIMITS)
            .migration_fuel(1);
        coordinator
            .register_migration(StateMigration::new(
                SESSION_ID,
                SESSION_SCHEMA,
                Version::new(1, 0),
                SESSION_SCHEMA,
                Version::new(3, 0),
                2,
                migrate_v1_to_v2,
            ))
            .unwrap();

        let error = coordinator.prepare(&previous, &candidate).unwrap_err();

        assert!(matches!(
            error,
            crate::StateTransferError::MigrationFuelExhausted {
                state_id: SESSION_ID,
                required: 2,
                remaining: 1,
            }
        ));
    }

    #[test]
    fn transfer_propagates_an_application_migration_failure() {
        let (previous, candidate) = schema_upgrade_images();
        let mut coordinator = StateTransferCoordinator::new()
            .model_limits(LIMITS)
            .migration_fuel(2);
        coordinator
            .register_migration(StateMigration::new(
                SESSION_ID,
                SESSION_SCHEMA,
                Version::new(1, 0),
                SESSION_SCHEMA,
                Version::new(3, 0),
                2,
                reject_migration,
            ))
            .unwrap();

        let error = coordinator.prepare(&previous, &candidate).unwrap_err();

        assert!(matches!(
            error,
            crate::StateTransferError::MigrationFailed {
                state_id: SESSION_ID,
                ..
            }
        ));
    }

    #[test]
    fn candidate_migration_preserves_multiple_independent_state_entries() {
        let session_payload = [1];
        let settings_payload = [4];
        let previous_entries = [
            StateEntry::new(
                SESSION_ID,
                SESSION_SCHEMA,
                Version::new(1, 0),
                StatePolicy::Required,
                &session_payload,
            ),
            StateEntry::new(
                SETTINGS_ID,
                SETTINGS_SCHEMA,
                Version::new(1, 0),
                StatePolicy::Required,
                &settings_payload,
            ),
        ];
        let previous = StateBundle::new(APPLICATION_ID, 3, &previous_entries)
            .encode(LIMITS)
            .unwrap();
        let session_default = [0, 0];
        let settings_default = [0];
        let new_id = StableId128::from_bytes([0x60; 16]);
        let new_schema = StableId128::from_bytes([0x70; 16]);
        let new_default = [8];
        let candidate_entries = [
            StateEntry::new(
                SESSION_ID,
                SESSION_SCHEMA,
                Version::new(2, 0),
                StatePolicy::Required,
                &session_default,
            ),
            StateEntry::new(
                SETTINGS_ID,
                SETTINGS_SCHEMA,
                Version::new(1, 0),
                StatePolicy::Required,
                &settings_default,
            ),
            StateEntry::new(
                new_id,
                new_schema,
                Version::new(1, 0),
                StatePolicy::ResetSafe,
                &new_default,
            ),
        ];
        let candidate = StateBundle::new(APPLICATION_ID, 4, &candidate_entries)
            .encode(LIMITS)
            .unwrap();
        let migrated_session = [1, 2];
        let migrated_entries = [StateEntry::new(
            SESSION_ID,
            SESSION_SCHEMA,
            Version::new(2, 0),
            StatePolicy::Required,
            &migrated_session,
        )];
        let migrated = StateBundle::new(APPLICATION_ID, 4, &migrated_entries)
            .encode(LIMITS)
            .unwrap();
        let coordinator = StateTransferCoordinator::new().model_limits(LIMITS);

        let prepared = coordinator
            .prepare_candidate_migration(&previous, &candidate, &migrated)
            .unwrap();

        let state = StateBundleView::decode(prepared.as_bytes(), LIMITS).unwrap();
        assert_eq!(state.entry(0).unwrap().payload(), migrated_session);
        assert_eq!(state.entry(1).unwrap().payload(), settings_payload);
        assert_eq!(state.entry(2).unwrap().payload(), new_default);
        assert_eq!(prepared.report().migrated_state_ids(), &[SESSION_ID]);
        assert_eq!(prepared.report().preserved_entries(), 1);
        assert_eq!(prepared.report().defaulted_entries(), 1);
    }

    fn schema_upgrade_images() -> (Vec<u8>, Vec<u8>) {
        let previous_payload = [1];
        let previous_entries = [StateEntry::new(
            SESSION_ID,
            SESSION_SCHEMA,
            Version::new(1, 0),
            StatePolicy::Required,
            &previous_payload,
        )];
        let previous = StateBundle::new(APPLICATION_ID, 3, &previous_entries)
            .encode(LIMITS)
            .unwrap();
        let candidate_payload = [0];
        let candidate_entries = [StateEntry::new(
            SESSION_ID,
            SESSION_SCHEMA,
            Version::new(3, 0),
            StatePolicy::Required,
            &candidate_payload,
        )];
        let candidate = StateBundle::new(APPLICATION_ID, 4, &candidate_entries)
            .encode(LIMITS)
            .unwrap();
        (previous, candidate)
    }

    fn reject_migration(_payload: &[u8]) -> Result<Vec<u8>, StateMigrationFailure> {
        Err(StateMigrationFailure::new(
            "fixture migration rejected state",
        ))
    }
}
#[cfg(test)]
mod widget_assembly_tests {
    use crate::{
        AssemblyErrorKind, ModelError, ModelLimits, PropertyValue, WidgetAssemblyDocument,
        WidgetDocumentView, disassemble_widget_document, stable_schema_hash64,
    };

    const LIMITS: ModelLimits = ModelLimits::new(16_384, 128, 1_024, 1_024).max_widget_depth(32);

    const SAMPLE: &str = r#"AWIR 2 0

SECTION TEXT
ROOT node0

node0:
  NODE hash64("aimer.widget:aimer_container::single_child::Container") 1 0
  PROP hash64("aimer.property:aimer_container::single_child::Container:width") F64 320.0
  PROP hash64("aimer.property:aimer_container::single_child::Container:height") F64 180.0
  PROP hash64("aimer.property:aimer_container::single_child::Container:color") RGBA 0x112233FF
  CHILD node1
  END

node1:
  NODE hash64("aimer.widget:aimer_text::Text") 1 0
  PROP hash64("aimer.property:aimer_text::Text:text") STRREF hello
  END

SECTION DATA
hello:
  STRING "Hello"
"#;

    #[test]
    fn assembles_the_documented_container_text_sample() {
        let assembly = WidgetAssemblyDocument::parse(SAMPLE, LIMITS).unwrap();
        let image = assembly.encode().unwrap();
        let view = WidgetDocumentView::decode(&image, LIMITS).unwrap();

        assert_eq!(view.node_count(), 2);
        assert_eq!(view.root_node(), 0);
        assert_eq!(
            view.node(0).unwrap().widget_type().value(),
            stable_schema_hash64("aimer.widget:aimer_container::single_child::Container")
        );
        assert_eq!(
            view.node(0)
                .unwrap()
                .properties()
                .map(|property| property.value())
                .collect::<Vec<_>>(),
            [
                PropertyValue::F64(320.0),
                PropertyValue::F64(180.0),
                PropertyValue::Rgba(0x112233FF),
            ]
        );
        assert_eq!(view.node(0).unwrap().children().collect::<Vec<_>>(), [1]);
        assert_eq!(view.string(0), Some("Hello"));
    }

    #[test]
    fn deterministic_disassembly_round_trips_every_value_and_binding() {
        let source = r#"AWIR 2 0
GENERATION 41
REVISION 9
SECTION TEXT
ROOT root
root:
  NODE 0x0000000000000010 2 3
  KEY 00112233445566778899aabbccddeeff
  PROP OPTIONAL 0x0000000000000100 BOOL true
  PROP 0x0000000000000101 I64 -9223372036854775808
  PROP 0x0000000000000102 F64 -0.25
  PROP 0x0000000000000103 RGBA 0x01020304
  PROP 0x0000000000000104 STRREF greeting
  PROP 0x0000000000000105 BLOBREF payload
  CALLBACK hash64("aimer.event:test::press") 1 2 ffeeddccbbaa99887766554433221100
  CHILD leaf
  END
leaf:
  NODE hash64("aimer.widget:test::Leaf") 1 0
  END
SECTION DATA
greeting:
  STRING "Hello, \u{4e16}\u{754c}!\n"
payload:
  BLOB 0x0001aaff
"#;
        let first = WidgetAssemblyDocument::parse(source, LIMITS)
            .unwrap()
            .encode()
            .unwrap();
        let first_view = WidgetDocumentView::decode(&first, LIMITS).unwrap();

        let text = disassemble_widget_document(&first_view);
        let second = WidgetAssemblyDocument::parse(&text, LIMITS)
            .unwrap()
            .encode()
            .unwrap();

        assert_eq!(second, first);
        assert!(text.contains("GENERATION 41\nREVISION 9"));
        assert!(text.contains("PROP OPTIONAL 0x0000000000000100 BOOL true"));
        assert!(text.contains("STRING \"Hello, 世界!\\n\""));
    }

    #[test]
    fn rejects_duplicate_and_missing_labels_with_source_context() {
        let duplicate = SAMPLE.replace("hello:\n  STRING", "node0:\n  STRING");
        let error = WidgetAssemblyDocument::parse(&duplicate, LIMITS).unwrap_err();
        assert_eq!(error.kind(), &AssemblyErrorKind::DuplicateLabel);
        assert!(error.line().is_some());

        let missing = SAMPLE.replace("CHILD node1", "CHILD absent");
        let error = WidgetAssemblyDocument::parse(&missing, LIMITS).unwrap_err();
        assert_eq!(error.kind(), &AssemblyErrorKind::MissingReference);
        assert!(error.context().contains("absent"));
    }

    #[test]
    fn rejects_malformed_syntax_missing_end_and_non_finite_float() {
        for (source, expected) in [
            (
                SAMPLE.replace("NODE hash64", "WIDGET hash64"),
                AssemblyErrorKind::UnknownDirective,
            ),
            (
                SAMPLE.replace("  END\n\nnode1:", "\nnode1:"),
                AssemblyErrorKind::MissingEnd,
            ),
            (
                SAMPLE.replace("F64 320.0", "F64 NaN"),
                AssemblyErrorKind::NonFiniteFloat,
            ),
            (
                SAMPLE.replace("STRING \"Hello\"", "STRING \"bad\\q\""),
                AssemblyErrorKind::InvalidEscape,
            ),
        ] {
            let error = WidgetAssemblyDocument::parse(&source, LIMITS).unwrap_err();
            assert_eq!(error.kind(), &expected);
        }
    }

    #[test]
    fn rejects_unsupported_versions_and_tight_limits() {
        let version = SAMPLE.replacen("AWIR 2 0", "AWIR 3 0", 1);
        let error = WidgetAssemblyDocument::parse(&version, LIMITS).unwrap_err();
        assert_eq!(error.kind(), &AssemblyErrorKind::UnsupportedVersion);

        let error =
            WidgetAssemblyDocument::parse(SAMPLE, ModelLimits::new(64, 1, 4, 1)).unwrap_err();
        assert_eq!(error.kind(), &AssemblyErrorKind::LimitExceeded);
    }

    #[test]
    fn encode_rejects_every_invalid_widget_topology() {
        for (source, expected) in [
            (
                graph_source("node0", &[&[0]]),
                ModelError::WidgetCycle { node: 0 },
            ),
            (
                graph_source("node0", &[&[], &[]]),
                ModelError::UnreachableWidgetNode { node: 1 },
            ),
            (
                graph_source("node0", &[&[1, 1], &[]]),
                ModelError::DuplicateWidgetChild {
                    parent: 0,
                    child: 1,
                },
            ),
            (
                graph_source("node0", &[&[1, 2], &[2], &[]]),
                ModelError::MultipleWidgetParents {
                    node: 2,
                    first_parent: 0,
                    second_parent: 1,
                },
            ),
        ] {
            let error = WidgetAssemblyDocument::parse(&source, LIMITS).unwrap_err();
            assert_eq!(error.kind(), &AssemblyErrorKind::Model(expected));
        }

        let depth_limits = ModelLimits::new(16_384, 8, 32, 32).max_widget_depth(2);
        let error =
            WidgetAssemblyDocument::parse(&graph_source("node0", &[&[1], &[2], &[]]), depth_limits)
                .unwrap_err();
        assert_eq!(error.kind(), &AssemblyErrorKind::LimitExceeded);
        assert!(error.context().contains("depth 3 exceeds limit 2"));
    }

    #[test]
    fn disassembly_round_trips_under_the_same_tight_binary_byte_limit() {
        let image = WidgetAssemblyDocument::parse(SAMPLE, LIMITS)
            .unwrap()
            .encode()
            .unwrap();
        let tight_limits =
            ModelLimits::new(image.len() as u32, 128, 1_024, 1_024).max_widget_depth(32);
        let view = WidgetDocumentView::decode(&image, tight_limits).unwrap();
        let source = disassemble_widget_document(&view);

        assert!(source.len() > image.len());
        assert_eq!(
            WidgetAssemblyDocument::parse(&source, tight_limits)
                .unwrap()
                .encode()
                .unwrap(),
            image
        );
    }

    #[test]
    fn compact_float_bits_preserve_extreme_finite_values_exactly() {
        let source = r#"AWIR 2 0
SECTION TEXT
ROOT node0
node0:
  NODE 0x0000000000000001 1 0
  PROP 0x0000000000000001 F64 0x0000000000000001
  PROP 0x0000000000000002 F64 0x7fefffffffffffff
  PROP 0x0000000000000003 F64 0x8000000000000000
  END
SECTION DATA
"#;
        let image = WidgetAssemblyDocument::parse(source, LIMITS)
            .unwrap()
            .encode()
            .unwrap();
        let view = WidgetDocumentView::decode(&image, LIMITS).unwrap();
        let text = disassemble_widget_document(&view);

        assert!(text.contains("F64 0x0000000000000001"));
        assert!(text.contains("F64 0x7fefffffffffffff"));
        assert!(text.contains("F64 0x8000000000000000"));
        assert_eq!(
            WidgetAssemblyDocument::parse(&text, LIMITS)
                .unwrap()
                .encode()
                .unwrap(),
            image
        );

        for bits in [
            "0x7ff0000000000000",
            "0xfff0000000000000",
            "0x7ff8000000000000",
        ] {
            let error = WidgetAssemblyDocument::parse(
                &source.replace("0x0000000000000001\n  PROP", &format!("{bits}\n  PROP")),
                LIMITS,
            )
            .unwrap_err();
            assert_eq!(error.kind(), &AssemblyErrorKind::NonFiniteFloat);
        }
    }

    #[test]
    fn rejects_missing_structure_invalid_identities_and_bad_references() {
        for (source, expected) in [
            (
                SAMPLE.replace("ROOT node0\n", ""),
                AssemblyErrorKind::MissingRoot,
            ),
            ("AWIR 2 0\n".to_owned(), AssemblyErrorKind::MissingSection),
            (
                SAMPLE.split("SECTION DATA").next().unwrap().to_owned(),
                AssemblyErrorKind::MissingSection,
            ),
            (
                SAMPLE.replace(
                    "hash64(\"aimer.widget:aimer_container::single_child::Container\")",
                    "0xxyz0000000000000",
                ),
                AssemblyErrorKind::InvalidHex,
            ),
            (
                SAMPLE.replace("  PROP hash64", "  KEY 0011\n  PROP hash64"),
                AssemblyErrorKind::InvalidIdentity,
            ),
            (
                SAMPLE.replace(
                    "  PROP hash64",
                    "  KEY 00112233445566778899aabbccddeefg\n  PROP hash64",
                ),
                AssemblyErrorKind::InvalidHex,
            ),
            (
                SAMPLE.replace(
                    "  CHILD node1",
                    "  CALLBACK 0x0000000000000001 1 0 0011\n  CHILD node1",
                ),
                AssemblyErrorKind::InvalidIdentity,
            ),
            (
                SAMPLE
                    .replace("STRREF hello", "STRREF payload")
                    .replace("hello:\n  STRING \"Hello\"", "payload:\n  BLOB 0x00"),
                AssemblyErrorKind::MissingReference,
            ),
            (
                SAMPLE.replace("STRREF hello", "BLOBREF hello"),
                AssemblyErrorKind::MissingReference,
            ),
        ] {
            let error = WidgetAssemblyDocument::parse(&source, LIMITS).unwrap_err();
            assert_eq!(error.kind(), &expected, "source:\n{source}");
        }

        let blob_limits = ModelLimits::new(16_384, 128, 1_024, 1);
        let source = SAMPLE
            .replace("STRING \"Hello\"", "BLOB 0x0001")
            .replace("STRREF hello", "BLOBREF hello");
        let error = WidgetAssemblyDocument::parse(&source, blob_limits).unwrap_err();
        assert_eq!(error.kind(), &AssemblyErrorKind::LimitExceeded);
    }

    fn graph_source(root: &str, children: &[&[usize]]) -> String {
        let mut source = format!("AWIR 2 0\nSECTION TEXT\nROOT {root}\n");
        for (index, children) in children.iter().enumerate() {
            source.push_str(&format!("node{index}:\n  NODE 0x0000000000000001 1 0\n"));
            for child in *children {
                source.push_str(&format!("  CHILD node{child}\n"));
            }
            source.push_str("  END\n");
        }
        source.push_str("SECTION DATA\n");
        source
    }
}
#[cfg(test)]
mod widget_graph_tests {
    use crate::{
        ModelError, ModelLimits, StableId128, Version, WidgetDocument, WidgetDocumentView,
        WidgetNode, WidgetSchemaId, WidgetSchemaSupport,
    };

    const LIMITS: ModelLimits = ModelLimits::new(4_096, 64, 256, 256).max_widget_depth(16);

    #[test]
    fn widget_graph_rejects_a_cycle() {
        let root_children = [1];
        let child_children = [0];
        let nodes = [
            WidgetNode::new(WidgetSchemaId::new(1), Version::new(1, 0)).children(&root_children),
            WidgetNode::new(WidgetSchemaId::new(2), Version::new(1, 0)).children(&child_children),
        ];
        let image = encode(&nodes);

        let error = decode_error(&image, LIMITS);

        assert_eq!(error, ModelError::WidgetCycle { node: 0 });
    }

    #[test]
    fn widget_graph_rejects_a_descendant_cycle_before_multiple_ownership() {
        let root_children = [1];
        let branch_children = [2];
        let descendant_children = [1];
        let nodes = [
            WidgetNode::new(WidgetSchemaId::new(1), Version::new(1, 0)).children(&root_children),
            WidgetNode::new(WidgetSchemaId::new(2), Version::new(1, 0)).children(&branch_children),
            WidgetNode::new(WidgetSchemaId::new(3), Version::new(1, 0))
                .children(&descendant_children),
        ];
        let image = encode(&nodes);

        let error = decode_error(&image, LIMITS);

        assert_eq!(error, ModelError::WidgetCycle { node: 1 });
    }

    #[test]
    fn widget_graph_rejects_a_node_with_multiple_parents() {
        let root_children = [1, 2];
        let branch_children = [1];
        let nodes = [
            WidgetNode::new(WidgetSchemaId::new(1), Version::new(1, 0)).children(&root_children),
            WidgetNode::new(WidgetSchemaId::new(2), Version::new(1, 0)),
            WidgetNode::new(WidgetSchemaId::new(3), Version::new(1, 0)).children(&branch_children),
        ];
        let image = encode(&nodes);

        let error = decode_error(&image, LIMITS);

        assert_eq!(
            error,
            ModelError::MultipleWidgetParents {
                node: 1,
                first_parent: 0,
                second_parent: 2,
            }
        );
    }

    #[test]
    fn widget_graph_rejects_duplicate_children() {
        let root_children = [1, 1];
        let nodes = [
            WidgetNode::new(WidgetSchemaId::new(1), Version::new(1, 0)).children(&root_children),
            WidgetNode::new(WidgetSchemaId::new(2), Version::new(1, 0)),
        ];
        let image = encode(&nodes);

        let error = decode_error(&image, LIMITS);

        assert_eq!(
            error,
            ModelError::DuplicateWidgetChild {
                parent: 0,
                child: 1,
            }
        );
    }

    #[test]
    fn widget_graph_rejects_an_unreachable_second_root() {
        let nodes = [
            WidgetNode::new(WidgetSchemaId::new(1), Version::new(1, 0)),
            WidgetNode::new(WidgetSchemaId::new(2), Version::new(1, 0)),
        ];
        let image = encode(&nodes);

        let error = decode_error(&image, LIMITS);

        assert_eq!(error, ModelError::UnreachableWidgetNode { node: 1 });
    }

    #[test]
    fn widget_graph_rejects_excessive_depth() {
        let root_children = [1];
        let branch_children = [2];
        let nodes = [
            WidgetNode::new(WidgetSchemaId::new(1), Version::new(1, 0)).children(&root_children),
            WidgetNode::new(WidgetSchemaId::new(2), Version::new(1, 0)).children(&branch_children),
            WidgetNode::new(WidgetSchemaId::new(3), Version::new(1, 0)),
        ];
        let image = encode(&nodes);

        let error = decode_error(&image, LIMITS.max_widget_depth(2));

        assert_eq!(
            error,
            ModelError::WidgetDepthExceeded { depth: 3, limit: 2 }
        );
    }

    #[test]
    fn widget_graph_rejects_an_incompatible_widget_schema() {
        let nodes = [WidgetNode::new(WidgetSchemaId::new(7), Version::new(2, 0))];
        let image = encode(&nodes);
        let view = WidgetDocumentView::decode(&image, LIMITS).unwrap();

        let error = view.validate_schemas(&HostSchemas).unwrap_err();

        assert_eq!(
            error,
            ModelError::UnsupportedWidgetSchema {
                node: 0,
                widget_type: WidgetSchemaId::new(7),
                schema: Version::new(2, 0),
            }
        );
    }

    #[test]
    fn widget_graph_accepts_a_complete_supported_tree() {
        let key = StableId128::from_bytes([0xA5; 16]);
        let root_children = [1, 2];
        let nodes = [
            WidgetNode::new(WidgetSchemaId::new(7), Version::new(1, 0)).children(&root_children),
            WidgetNode::new(WidgetSchemaId::new(8), Version::new(1, 1)).key(key),
            WidgetNode::new(WidgetSchemaId::new(9), Version::new(1, 0)),
        ];
        let image = encode(&nodes);

        let view = WidgetDocumentView::decode(&image, LIMITS).unwrap();
        view.validate_schemas(&HostSchemas).unwrap();

        assert_eq!(view.node_count(), 3);
        assert_eq!(view.node(0).unwrap().children().collect::<Vec<_>>(), [1, 2]);
    }

    fn encode(nodes: &[WidgetNode<'_>]) -> Vec<u8> {
        WidgetDocument::new(1, 1, 0, nodes, &[], &[])
            .encode(LIMITS)
            .unwrap()
    }

    fn decode_error(image: &[u8], limits: ModelLimits) -> ModelError {
        match WidgetDocumentView::decode(image, limits) {
            Ok(_) => panic!("invalid widget graph was accepted"),
            Err(error) => error,
        }
    }

    struct HostSchemas;

    impl WidgetSchemaSupport for HostSchemas {
        fn supports(&self, widget_type: WidgetSchemaId, schema: Version) -> bool {
            matches!(
                (widget_type.value(), schema.major(), schema.minor()),
                (1..=3, 1, 0) | (7, 1, 0) | (8, 1, 0..=1) | (9, 1, 0)
            )
        }
    }
}
#[cfg(test)]
mod widget_materializer_tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use crate::{
        ModelError, ModelLimits, PropertyId, PropertyValue, Version, WidgetDocument,
        WidgetDocumentView, WidgetFactory, WidgetMaterializeError, WidgetNode, WidgetNodeView,
        WidgetProperty, WidgetSchemaId, WidgetSchemaSupport, materialize_widget_tree,
    };

    const LIMITS: ModelLimits = ModelLimits::new(4_096, 64, 256, 256).max_widget_depth(16);

    #[test]
    fn invalid_widget_graph_creates_no_headless_nodes() {
        let root_children = [1];
        let child_children = [0];
        let nodes = [
            WidgetNode::new(WidgetSchemaId::new(1), Version::new(1, 0)).children(&root_children),
            WidgetNode::new(WidgetSchemaId::new(2), Version::new(1, 0)).children(&child_children),
        ];
        let image = encode(&nodes);
        let mut factory = RecordingFactory::default();

        let error = materialize_widget_tree(&image, LIMITS, &mut factory).unwrap_err();

        assert_eq!(
            error,
            WidgetMaterializeError::Model(ModelError::WidgetCycle { node: 0 })
        );
        assert!(factory.built_types.is_empty());
    }

    #[test]
    fn unsupported_widget_schema_creates_no_headless_nodes() {
        let nodes = [WidgetNode::new(WidgetSchemaId::new(99), Version::new(1, 0))];
        let image = encode(&nodes);
        let mut factory = RecordingFactory::default();

        let error = materialize_widget_tree(&image, LIMITS, &mut factory).unwrap_err();

        assert_eq!(
            error,
            WidgetMaterializeError::Model(ModelError::UnsupportedWidgetSchema {
                node: 0,
                widget_type: WidgetSchemaId::new(99),
                schema: Version::new(1, 0),
            })
        );
        assert!(factory.built_types.is_empty());
    }

    #[test]
    fn invalid_widget_properties_are_preflighted_before_any_headless_node_is_created() {
        let properties = [WidgetProperty::new(
            PropertyId::new(99),
            PropertyValue::Bool(true),
        )];
        let child_indices = [1];
        let nodes = [
            WidgetNode::new(WidgetSchemaId::new(1), Version::new(1, 0)).children(&child_indices),
            WidgetNode::new(WidgetSchemaId::new(2), Version::new(1, 0)).properties(&properties),
        ];
        let image = WidgetDocument::new(1, 1, 0, &nodes, &[], &[])
            .encode(LIMITS)
            .unwrap();
        let mut factory = PreflightFactory::default();

        let error = materialize_widget_tree(&image, LIMITS, &mut factory).unwrap_err();

        assert_eq!(
            error,
            WidgetMaterializeError::Model(ModelError::UnsupportedWidgetProperty {
                node: 1,
                widget_type: WidgetSchemaId::new(2),
                property_id: PropertyId::new(99),
            })
        );
        assert!(factory.built_types.is_empty());
    }

    #[test]
    fn valid_widget_graph_materializes_children_before_the_disconnected_root() {
        let root_children = [1, 2];
        let nodes = [
            WidgetNode::new(WidgetSchemaId::new(1), Version::new(1, 0)).children(&root_children),
            WidgetNode::new(WidgetSchemaId::new(2), Version::new(1, 0)),
            WidgetNode::new(WidgetSchemaId::new(3), Version::new(1, 0)),
        ];
        let image = encode(&nodes);
        let mut factory = RecordingFactory::default();

        let root = materialize_widget_tree(&image, LIMITS, &mut factory).unwrap();

        assert_eq!(
            factory.built_types,
            [
                WidgetSchemaId::new(2),
                WidgetSchemaId::new(3),
                WidgetSchemaId::new(1),
            ]
        );
        assert_eq!(root.widget_type, WidgetSchemaId::new(1));
        assert_eq!(
            root.children
                .iter()
                .map(|child| child.widget_type)
                .collect::<Vec<_>>(),
            [WidgetSchemaId::new(2), WidgetSchemaId::new(3)]
        );
    }

    #[test]
    fn failed_materialization_drops_the_disconnected_candidate() {
        let root_children = [1, 2];
        let nodes = [
            WidgetNode::new(WidgetSchemaId::new(1), Version::new(1, 0)).children(&root_children),
            WidgetNode::new(WidgetSchemaId::new(2), Version::new(1, 0)),
            WidgetNode::new(WidgetSchemaId::new(3), Version::new(1, 0)),
        ];
        let image = encode(&nodes);
        let drops = Rc::new(Cell::new(0));
        let mut factory = FailingFactory {
            drops: drops.clone(),
            built_types: Vec::new(),
        };

        let error = materialize_widget_tree(&image, LIMITS, &mut factory).unwrap_err();

        assert_eq!(
            error,
            WidgetMaterializeError::Factory {
                node: 2,
                error: FactoryError,
            }
        );
        assert_eq!(factory.built_types, [WidgetSchemaId::new(2)]);
        assert_eq!(drops.get(), 1);
    }

    fn encode(nodes: &[WidgetNode<'_>]) -> Vec<u8> {
        WidgetDocument::new(1, 1, 0, nodes, &[], &[])
            .encode(LIMITS)
            .unwrap()
    }

    #[derive(Debug, Eq, PartialEq)]
    struct HeadlessNode {
        widget_type: WidgetSchemaId,
        children: Vec<HeadlessNode>,
    }

    #[derive(Default)]
    struct RecordingFactory {
        built_types: Vec<WidgetSchemaId>,
    }

    impl WidgetSchemaSupport for RecordingFactory {
        fn supports(&self, widget_type: WidgetSchemaId, schema: Version) -> bool {
            (1..=3).contains(&widget_type.value()) && schema == Version::new(1, 0)
        }
    }

    impl WidgetFactory for RecordingFactory {
        type Error = FactoryError;
        type Node = HeadlessNode;

        fn build(
            &mut self,
            _document: &WidgetDocumentView<'_>,
            _node_index: u32,
            node: WidgetNodeView<'_>,
            children: Vec<Self::Node>,
        ) -> Result<Self::Node, Self::Error> {
            let widget_type = node.widget_type();
            self.built_types.push(widget_type);
            Ok(HeadlessNode {
                widget_type,
                children,
            })
        }
    }

    #[derive(Default)]
    struct PreflightFactory {
        built_types: Vec<WidgetSchemaId>,
    }

    impl WidgetSchemaSupport for PreflightFactory {
        fn supports(&self, widget_type: WidgetSchemaId, schema: Version) -> bool {
            (1..=2).contains(&widget_type.value()) && schema == Version::new(1, 0)
        }

        fn validate_node(
            &self,
            _document: &WidgetDocumentView<'_>,
            node_index: u32,
            node: WidgetNodeView<'_>,
        ) -> Result<(), ModelError> {
            if let Some(property) = node.properties().next() {
                return Err(ModelError::UnsupportedWidgetProperty {
                    node: node_index,
                    widget_type: node.widget_type(),
                    property_id: property.property_id(),
                });
            }
            Ok(())
        }
    }

    impl WidgetFactory for PreflightFactory {
        type Error = FactoryError;
        type Node = HeadlessNode;

        fn build(
            &mut self,
            _document: &WidgetDocumentView<'_>,
            _node_index: u32,
            node: WidgetNodeView<'_>,
            children: Vec<Self::Node>,
        ) -> Result<Self::Node, Self::Error> {
            self.built_types.push(node.widget_type());
            Ok(HeadlessNode {
                widget_type: node.widget_type(),
                children,
            })
        }
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct FactoryError;

    #[derive(Debug)]
    struct TrackedNode(Rc<Cell<usize>>);

    impl Drop for TrackedNode {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }

    struct FailingFactory {
        drops: Rc<Cell<usize>>,
        built_types: Vec<WidgetSchemaId>,
    }

    impl WidgetSchemaSupport for FailingFactory {
        fn supports(&self, widget_type: WidgetSchemaId, schema: Version) -> bool {
            (1..=3).contains(&widget_type.value()) && schema == Version::new(1, 0)
        }
    }

    impl WidgetFactory for FailingFactory {
        type Error = FactoryError;
        type Node = TrackedNode;

        fn build(
            &mut self,
            _document: &WidgetDocumentView<'_>,
            _node_index: u32,
            node: WidgetNodeView<'_>,
            _children: Vec<Self::Node>,
        ) -> Result<Self::Node, Self::Error> {
            if node.widget_type() == WidgetSchemaId::new(3) {
                return Err(FactoryError);
            }
            self.built_types.push(node.widget_type());
            Ok(TrackedNode(self.drops.clone()))
        }
    }
}
#[cfg(all(test, feature = "wasm-hot-reload"))]
mod tests {
    use super::*;
    #[rustfmt::skip]
    const ANSWER_MODULE: &[u8] = &[
        0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00, 0x01, 0x05, 0x01, 0x60, 0x00, 0x01,
        0x7F, 0x03, 0x02, 0x01, 0x00, 0x07, 0x0A, 0x01, 0x06, 0x61, 0x6E, 0x73, 0x77, 0x65, 0x72,
        0x00, 0x00, 0x0A, 0x06, 0x01, 0x04, 0x00, 0x41, 0x2A, 0x0B,
    ];
    #[rustfmt::skip]
    const INFINITE_LOOP_MODULE: &[u8] = &[
        0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00, 0x01, 0x05, 0x01, 0x60, 0x00, 0x01,
        0x7F, 0x03, 0x02, 0x01, 0x00, 0x07, 0x08, 0x01, 0x04, 0x73, 0x70, 0x69, 0x6E, 0x00, 0x00,
        0x0A, 0x0B, 0x01, 0x09, 0x00, 0x03, 0x40, 0x0C, 0x00, 0x0B, 0x41, 0x00, 0x0B,
    ];

    #[test]
    fn runtime_executes_a_deterministic_wasm_function() {
        let runtime = Runtime::new(test_config(1_000));

        let result = runtime.invoke_i32(ANSWER_MODULE, "answer").unwrap();

        assert_eq!(result, 42);
    }

    #[test]
    fn runtime_rejects_an_infinite_function_when_fuel_is_exhausted() {
        let runtime = Runtime::new(test_config(100));

        let error = runtime
            .invoke_i32(INFINITE_LOOP_MODULE, "spin")
            .unwrap_err();

        assert_eq!(error.kind(), RuntimeErrorKind::FuelExhausted);
    }

    fn test_config(fuel_per_call: u64) -> RuntimeConfig {
        RuntimeConfig::new()
            .fuel_per_call(fuel_per_call)
            .max_module_bytes(1_024)
            .max_memory_pages(1)
            .max_table_elements(1)
            .max_call_depth(64)
    }
}
