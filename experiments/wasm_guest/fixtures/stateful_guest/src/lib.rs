// Each Phase 23 build feature selects a different subset of the reflected fixture.
#[allow(unused_imports)]
use aimer_wasm_guest::anteros::{
    AbiStatus, AbiVersion, ApplicationManifest, CallbackBinding, CallbackEventView, ModelLimits,
    PropertyValue, StableId128, StateBundle, StateBundleView, StateEntry, StatePolicy,
    Version, WidgetDocument, WidgetNode, WidgetProperty, EVENT_BUTTON_PRESS,
    PROPERTY_CONTAINER_BOX_DECORATION, PROPERTY_CONTAINER_COLOR, PROPERTY_CONTAINER_HEIGHT,
    PROPERTY_CONTAINER_MARGIN, PROPERTY_CONTAINER_PADDING, PROPERTY_CONTAINER_WIDTH,
    PROPERTY_TEXT_CONTENT, WIDGET_BUTTON, WIDGET_COLUMN, WIDGET_CONTAINER, WIDGET_TEXT,
    CALLBACK_EVENT_FORMAT_VERSION, STATE_FORMAT_VERSION, WIDGET_IR_FORMAT_VERSION,
};
#[cfg(feature = "phase23-unknown-required-property")]
use aimer_wasm_guest::anteros::PropertyId;
use aimer_wasm_guest::{CallbackRegistry, GuestError, GuestLimits, GuestProgram};

const LIMITS: ModelLimits = ModelLimits::new(4_096, 32, 128, 256).max_widget_depth(16);
const PROGRAM_ID: StableId128 = StableId128::from_bytes([0x11; 16]);
const CALLBACK_ID: StableId128 = StableId128::from_bytes([0x22; 16]);
const STATE_ID: StableId128 = StableId128::from_bytes([0x33; 16]);
const SCHEMA_ID: StableId128 = StableId128::from_bytes([0x44; 16]);
#[cfg(feature = "phase23-missing-materializer")]
const PHASE23_SCHEMA_ONLY_WIDGET: aimer_wasm_guest::anteros::WidgetSchemaId =
    aimer_wasm_guest::anteros::WidgetSchemaId::from_canonical_name(
        "aimer.widget:aimer_quiver.tests.Phase23SchemaOnly",
    );
const VERSION: Version = Version::new(1, 0);

struct StatefulGuest {
    counter: u8,
    callbacks: CallbackRegistry<u8>,
    generation_id: u64,
}

impl Default for StatefulGuest {
    fn default() -> Self {
        let mut callbacks = CallbackRegistry::<u8>::new().max_callbacks(1);
        callbacks
            .register(CALLBACK_ID, |counter, _event| {
                *counter = counter.saturating_add(1);
                Ok(true)
            })
            .expect("static callback registration is valid");
        Self {
            counter: 0,
            callbacks,
            generation_id: 0,
        }
    }
}

impl GuestProgram for StatefulGuest {
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

    fn initialize(&mut self, generation_id: u64) -> Result<(), GuestError> {
        self.generation_id = generation_id;
        Ok(())
    }

    fn build(&mut self, limits: ModelLimits) -> Result<Vec<u8>, GuestError> {
        widget_image(self.generation_id, self.counter, limits)
    }

    fn dispatch_event(
        &mut self,
        event: &CallbackEventView<'_>,
        limits: ModelLimits,
    ) -> Result<Option<Vec<u8>>, GuestError> {
        self.callbacks.dispatch(&mut self.counter, event)?;
        widget_image(self.generation_id, self.counter, limits).map(Some)
    }

    fn export_state(&self, limits: ModelLimits) -> Result<Vec<u8>, GuestError> {
        let entries = [StateEntry::new(
            STATE_ID,
            SCHEMA_ID,
            VERSION,
            StatePolicy::Required,
            std::slice::from_ref(&self.counter),
        )];
        StateBundle::new(PROGRAM_ID, self.generation_id, &entries)
            .encode(limits)
            .map_err(GuestError::from_model)
    }

    fn import_state(&mut self, state: &StateBundleView<'_>) -> Result<(), GuestError> {
        if state.application_id() != PROGRAM_ID || state.entry_count() != 1 {
            return Err(GuestError::new(AbiStatus::StateIncompatible));
        }
        let entry = state.entry(0).unwrap();
        if entry.state_id() != STATE_ID
            || entry.schema_id() != SCHEMA_ID
            || entry.schema_version() != VERSION
            || entry.payload().len() != 1
        {
            return Err(GuestError::new(AbiStatus::StateIncompatible));
        }
        self.counter = entry.payload()[0];
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

fn widget_image(
    generation_id: u64,
    counter: u8,
    limits: ModelLimits,
) -> Result<Vec<u8>, GuestError> {
    #[cfg(any(
        feature = "phase23-incompatible-codec",
        feature = "phase23-unknown-required-property",
        feature = "phase23-oversized-blob",
        feature = "phase23-missing-materializer"
    ))]
    return phase23_invalid_widget_image(generation_id, counter, limits);

    #[cfg(not(any(
        feature = "phase23-incompatible-codec",
        feature = "phase23-unknown-required-property",
        feature = "phase23-oversized-blob",
        feature = "phase23-missing-materializer"
    )))]
    {
    let root_children = [1_u32, 2];
    let sized_button_children = [3_u32];
    let button_children = [4_u32];
    let counter_label = counter.to_string();
    let label_properties = [WidgetProperty::new(
        PROPERTY_TEXT_CONTENT,
        PropertyValue::StringRef(0),
    )];
    let button_properties = [WidgetProperty::new(
        PROPERTY_TEXT_CONTENT,
        PropertyValue::StringRef(1),
    )];
    let width = [1_u8, 1, 0, 0, 32, 67];
    let height = [1_u8, 1, 0, 0, 64, 66];
    let sized_button_properties = [
        WidgetProperty::new(PROPERTY_CONTAINER_WIDTH, PropertyValue::BlobRef(0)),
        WidgetProperty::new(PROPERTY_CONTAINER_HEIGHT, PropertyValue::BlobRef(1)),
        WidgetProperty::new(PROPERTY_CONTAINER_PADDING, PropertyValue::BlobRef(2)),
        WidgetProperty::new(PROPERTY_CONTAINER_MARGIN, PropertyValue::BlobRef(3)),
        WidgetProperty::new(PROPERTY_CONTAINER_BOX_DECORATION, PropertyValue::BlobRef(4)),
        WidgetProperty::new(PROPERTY_CONTAINER_COLOR, PropertyValue::Rgba(0x3366CCFF)),
    ];
    let callbacks = [CallbackBinding::new(
        EVENT_BUTTON_PRESS,
        VERSION,
        CALLBACK_ID,
    )];
    let nodes = [
        WidgetNode::new(WIDGET_COLUMN, VERSION).children(&root_children),
        WidgetNode::new(WIDGET_TEXT, VERSION).properties(&label_properties),
        WidgetNode::new(WIDGET_CONTAINER, VERSION)
            .properties(&sized_button_properties)
            .children(&sized_button_children),
        WidgetNode::new(WIDGET_BUTTON, VERSION)
            .callbacks(&callbacks)
            .children(&button_children),
        WidgetNode::new(WIDGET_TEXT, VERSION).properties(&button_properties),
    ];
    let strings = [counter_label.as_str(), "increment"];
    let mut spacing = [0_u8; 21];
    spacing[0] = 1;
    let mut decoration = [0_u8; 106];
    decoration[0] = 1;
    let blobs: [&[u8]; 5] = [&width, &height, &spacing, &spacing, &decoration];
    WidgetDocument::new(
        generation_id,
        u64::from(counter),
        0,
        &nodes,
        &strings,
        &blobs,
    )
        .encode(limits)
        .map_err(GuestError::from_model)
    }
}

#[cfg(any(
    feature = "phase23-incompatible-codec",
    feature = "phase23-unknown-required-property",
    feature = "phase23-oversized-blob",
    feature = "phase23-missing-materializer"
))]
fn phase23_invalid_widget_image(
    generation_id: u64,
    counter: u8,
    limits: ModelLimits,
) -> Result<Vec<u8>, GuestError> {
    #[cfg(feature = "phase23-missing-materializer")]
    {
        let node = WidgetNode::new(PHASE23_SCHEMA_ONLY_WIDGET, VERSION);
        return WidgetDocument::new(generation_id, u64::from(counter), 0, &[node], &[], &[])
            .encode(limits)
            .map_err(GuestError::from_model);
    }

    #[cfg(not(feature = "phase23-missing-materializer"))]
    {
        let children = [1_u32];
        let text_properties = [WidgetProperty::new(
            PROPERTY_TEXT_CONTENT,
            PropertyValue::StringRef(0),
        )];
        let (container_properties, blobs): (Vec<WidgetProperty>, Vec<Vec<u8>>) = {
        #[cfg(feature = "phase23-incompatible-codec")]
        {
            (
                vec![WidgetProperty::new(
                    PROPERTY_CONTAINER_PADDING,
                    PropertyValue::BlobRef(0),
                )],
                vec![vec![2_u8]],
            )
        }
        #[cfg(feature = "phase23-unknown-required-property")]
        {
            (
                vec![WidgetProperty::new(PropertyId::new(0xDEAD), PropertyValue::Bool(true))],
                Vec::new(),
            )
        }
        #[cfg(feature = "phase23-oversized-blob")]
        {
            (
                vec![WidgetProperty::new(
                    PROPERTY_CONTAINER_BOX_DECORATION,
                    PropertyValue::BlobRef(0),
                )],
                vec![vec![1_u8; 129]],
            )
        }
        };
        let blob_refs: Vec<&[u8]> = blobs.iter().map(Vec::as_slice).collect();
        let nodes = [
            WidgetNode::new(WIDGET_CONTAINER, VERSION)
                .properties(&container_properties)
                .children(&children),
            WidgetNode::new(WIDGET_TEXT, VERSION).properties(&text_properties),
        ];
        WidgetDocument::new(
            generation_id,
            u64::from(counter),
            0,
            &nodes,
            &["invalid"],
            &blob_refs,
        )
        .encode(limits)
        .map_err(GuestError::from_model)
    }
}

aimer_wasm_guest::export_guest!(StatefulGuest, GuestLimits::new(LIMITS, 4, 16_384, 16));
