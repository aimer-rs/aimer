use std::error::Error;
use std::fmt;
use std::fmt::Write as _;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{
    Receiver, RecvTimeoutError, SendError, SyncSender, TryRecvError, TrySendError, sync_channel,
};
use std::time::Duration;

use aimer_animation::AnyCurve;
use aimer_anteros::{WIDGET_GESTURE_DETECTOR, WIDGET_MOUSE_REGION};
use aimer_anteros::{
    BUILTIN_PORTABLE_WIDGET_SCHEMAS, BUILTIN_WIDGET_SCHEMA_VERSION, CallbackBindings, EventId,
    ModelError, ModelLimits, PortableWidgetSchemaValidator, PropertyId, PropertyValue, StableId128,
    Version, WidgetDocumentView, WidgetFactory, WidgetMaterializeError, WidgetNodeView,
    WidgetSchemaId, WidgetSchemaSupport, disassemble_widget_document, materialize_widget_tree,
};
use aimer_anteros::{ReloadCommit, ReloadStage as RuntimeReloadStage};
use aimer_attribute::Dimension;
use aimer_container::SizedBox;
use aimer_flex::{BoxAlignment, Column, JustifyContent, OverflowBehavior, Row};
use aimer_grid::{Grid, GridItem, GridPortableConfig};
use aimer_input::button::Button;
use aimer_input::gesture::gesture_detector::GestureDetector;
use aimer_input::mouse_region::MouseRegion;
use aimer_provider::{Provider, ProviderHandle};
use aimer_reload_protocol::{ReloadCommand, ReloadResult, ReloadStage};
use aimer_reload_server::ReloadCommandSink;
use aimer_scroll::{ScrollBar, ScrollPortableConfig, Scrollable};
use aimer_space::Stack;
use aimer_style::{AnimatedTheme, BoxDecoration, LayoutSpacing, TextStyle, ThemeData, ThemeMode};
use aimer_text::{TextButton, TextSource};
use aimer_widget::base::BuildContext;
use aimer_widget::portable::{
    PortableMaterializeError, PortableMaterializeProperty, PortableNativeMaterializer,
    PortableNativeWidgetRegistration, linked_portable_native_widget_registrations,
    linked_portable_native_widget_schemas, optional_materialized_property,
    required_materialized_property,
};
use aimer_widget::{AnyElement, Element, ErrorWidget, Key, StatelessElement, Widget};

mod bootstrap;
mod candidate;
mod live;
mod portable_http;

pub use bootstrap::{initialize_hot_reload_host, take_hot_reload_config};
pub use candidate::{
    ManifestCompatibilityError, PreparedReloadCandidate, ReloadCandidateLimits,
    ReloadCandidatePreparationError, ReloadCandidatePreparer,
};
pub use live::{
    LiveReloadCommit, LiveReloadConfig, LiveReloadError, LiveReloadHost, LiveReloadStartError,
};

pub use aimer_anteros::{
    EVENT_BUTTON_DOUBLE_PRESS, EVENT_BUTTON_LONG_PRESS, EVENT_BUTTON_PRESS,
    EVENT_BUTTON_RIGHT_PRESS,
    PROPERTY_ANIMATED_THEME_CURVE, PROPERTY_ANIMATED_THEME_CURVE_X1,
    PROPERTY_ANIMATED_THEME_CURVE_X2, PROPERTY_ANIMATED_THEME_CURVE_Y1,
    PROPERTY_ANIMATED_THEME_CURVE_Y2, PROPERTY_ANIMATED_THEME_DURATION_MILLIS,
    PROPERTY_ANIMATED_THEME_MODE, PROPERTY_ANIMATED_THEME_SCHEMA_VERSION,
    PROPERTY_ANIMATED_THEME_TYPE, PROPERTY_ANIMATED_THEME_VALUE, PROPERTY_BUTTON_DECORATION,
    PROPERTY_COLUMN_GAPS, PROPERTY_COLUMN_HORIZONTAL_ALIGNMENT, PROPERTY_COLUMN_JUSTIFY_CONTENT,
    PROPERTY_COLUMN_OVERFLOW, PROPERTY_COLUMN_VERTICAL_ALIGNMENT,
    PROPERTY_CONTAINER_BOX_DECORATION, PROPERTY_CONTAINER_COLOR, PROPERTY_CONTAINER_HEIGHT,
    PROPERTY_CONTAINER_MARGIN, PROPERTY_CONTAINER_PADDING, PROPERTY_CONTAINER_WIDTH,
    PROPERTY_PROVIDER_SCHEMA_VERSION, PROPERTY_PROVIDER_TYPE, PROPERTY_PROVIDER_VALUE,
    PROPERTY_ROW_GAPS, PROPERTY_ROW_HORIZONTAL_ALIGNMENT, PROPERTY_ROW_JUSTIFY_CONTENT,
    PROPERTY_ROW_OVERFLOW, PROPERTY_ROW_VERTICAL_ALIGNMENT, PROPERTY_SIZED_BOX_HEIGHT,
    PROPERTY_SIZED_BOX_WIDTH, PROPERTY_TEXT_CONTENT, THEME_DATA_VALUE_VERSION,
    WIDGET_ANIMATED_THEME, WIDGET_BUTTON, WIDGET_COLUMN, WIDGET_CONTAINER, WIDGET_PROVIDER, WIDGET_ROW, WIDGET_SIZED_BOX, WIDGET_TEXT,

};

const BRIDGE_QUEUE_FULL: u32 = 0x7001;
const BRIDGE_HOST_DISCONNECTED: u32 = 0x7003;
const BRIDGE_COMMAND_DROPPED: u32 = 0x7004;

/// Creates a bounded handoff from the authenticated listener thread to Quiver.
///
/// The sink implements [`ReloadCommandSink`] and may be moved into
/// `aimer_reload_server`. The inbox is owned by the application thread and is
/// the only side allowed to prepare or commit a candidate. Listener execution
/// waits for [`PendingProtocolReload::complete`], so upload acceptance cannot
/// be mistaken for a host safe-point commit.
pub fn reload_command_bridge(
    capacity: usize,
    active_generation: u64,
) -> (ProtocolReloadSink, ProtocolReloadInbox) {
    reload_command_bridge_with_wake(capacity, active_generation, || {})
}

/// Creates a bounded listener handoff that wakes the application event loop.
///
/// `wake` runs only after a complete authenticated command enters the bounded
/// queue. It must not inspect or mutate the widget tree; its sole purpose is to
/// arrange the application-thread safe point that consumes the command.
pub fn reload_command_bridge_with_wake(
    capacity: usize,
    active_generation: u64,
    wake: impl Fn() + Send + Sync + 'static,
) -> (ProtocolReloadSink, ProtocolReloadInbox) {
    let (sender, receiver) = sync_channel(capacity);
    let active_generation = Arc::new(AtomicU64::new(active_generation));
    (
        ProtocolReloadSink {
            sender,
            active_generation: Arc::clone(&active_generation),
            wake: Arc::new(wake),
        },
        ProtocolReloadInbox {
            receiver,
            active_generation,
        },
    )
}

/// Authenticated listener-side adapter for the Quiver command handoff.
pub struct ProtocolReloadSink {
    sender: SyncSender<PendingProtocolReload>,
    active_generation: Arc<AtomicU64>,
    wake: Arc<dyn Fn() + Send + Sync>,
}

impl ReloadCommandSink for ProtocolReloadSink {
    fn execute(&self, command: ReloadCommand) -> ReloadResult {
        let (response, result) = sync_channel(1);
        let pending = PendingProtocolReload {
            command,
            response: Some(response),
            active_generation: Arc::clone(&self.active_generation),
        };
        match self.sender.try_send(pending) {
            Ok(()) => (self.wake)(),
            Err(TrySendError::Full(_)) => {
                return bridge_rejection(
                    ReloadStage::Preflight,
                    BRIDGE_QUEUE_FULL,
                    self.active_generation.load(Ordering::Acquire),
                    "reload command queue is full",
                );
            }
            Err(TrySendError::Disconnected(_)) => {
                return bridge_rejection(
                    ReloadStage::CommitWait,
                    BRIDGE_HOST_DISCONNECTED,
                    self.active_generation.load(Ordering::Acquire),
                    "reload command host is unavailable",
                );
            }
        }
        match result.recv() {
            Ok(result) => result,
            Err(_) => bridge_rejection(
                ReloadStage::CommitWait,
                BRIDGE_HOST_DISCONNECTED,
                self.active_generation.load(Ordering::Acquire),
                "reload command host disconnected",
            ),
        }
    }
}

/// Application-thread receiver for authenticated complete module commands.
pub struct ProtocolReloadInbox {
    receiver: Receiver<PendingProtocolReload>,
    active_generation: Arc<AtomicU64>,
}

impl ProtocolReloadInbox {
    /// Waits up to `timeout` for one complete authenticated command.
    pub fn recv_timeout(
        &self,
        timeout: Duration,
    ) -> Result<PendingProtocolReload, RecvTimeoutError> {
        self.receiver.recv_timeout(timeout)
    }

    /// Attempts to take one command without blocking the event loop.
    #[inline]
    pub fn try_recv(&self) -> Result<PendingProtocolReload, TryRecvError> {
        self.receiver.try_recv()
    }

    /// Returns the generation used for fail-closed bridge diagnostics.
    #[inline]
    pub fn active_generation(&self) -> u64 {
        self.active_generation.load(Ordering::Acquire)
    }
}

/// One authenticated command owned by the application thread.
///
/// Dropping this value without completing it reports a stable cancellation to
/// the listener, preventing a blocked connection or an ambiguous success.
pub struct PendingProtocolReload {
    command: ReloadCommand,
    response: Option<SyncSender<ReloadResult>>,
    active_generation: Arc<AtomicU64>,
}

impl PendingProtocolReload {
    /// Borrows the complete command for compatibility checks and preparation.
    #[inline]
    pub const fn command(&self) -> &ReloadCommand {
        &self.command
    }

    /// Reports the authoritative host result to the authenticated listener.
    pub fn complete(mut self, result: ReloadResult) -> Result<(), SendError<ReloadResult>> {
        self.active_generation
            .store(result_active_generation(&result), Ordering::Release);
        self.response.take().unwrap().send(result)
    }

    /// Reports a successful coherent snapshot installation.
    pub fn complete_commit<E>(
        self,
        commit: &ReloadCommit<E>,
        reset_state_entries: u32,
        cleanup_warnings: u32,
    ) -> Result<(), SendError<ReloadResult>> {
        self.complete(ReloadResult::Committed {
            active_generation: commit.generation_id().get(),
            reset_state_entries,
            cleanup_warnings,
        })
    }

    /// Reports a structured preparation or safe-point rejection.
    pub fn complete_rejection(
        self,
        stage: RuntimeReloadStage,
        error_code: u32,
        active_generation: u64,
        diagnostic: String,
    ) -> Result<(), SendError<ReloadResult>> {
        self.complete(ReloadResult::Rejected {
            stage: protocol_reload_stage(stage),
            error_code,
            active_generation,
            diagnostic,
        })
    }
}

impl Drop for PendingProtocolReload {
    fn drop(&mut self) {
        let Some(response) = self.response.take() else {
            return;
        };
        let active_generation = self.active_generation.load(Ordering::Acquire);
        let _ = response.send(bridge_rejection(
            ReloadStage::Cancellation,
            BRIDGE_COMMAND_DROPPED,
            active_generation,
            "reload command was dropped before host completion",
        ));
    }
}

fn bridge_rejection(
    stage: ReloadStage,
    error_code: u32,
    active_generation: u64,
    diagnostic: &'static str,
) -> ReloadResult {
    ReloadResult::Rejected {
        stage,
        error_code,
        active_generation,
        diagnostic: diagnostic.to_owned(),
    }
}

fn result_active_generation(result: &ReloadResult) -> u64 {
    match result {
        ReloadResult::Committed {
            active_generation, ..
        }
        | ReloadResult::Rejected {
            active_generation, ..
        }
        | ReloadResult::Cancelled { active_generation } => *active_generation,
    }
}

/// Maps every transactional Anteros boundary to its stable wire stage.
#[inline]
pub const fn protocol_reload_stage(stage: RuntimeReloadStage) -> ReloadStage {
    match stage {
        RuntimeReloadStage::Preflight => ReloadStage::Preflight,
        RuntimeReloadStage::Instantiate => ReloadStage::Instantiate,
        RuntimeReloadStage::Initialize => ReloadStage::Initialize,
        RuntimeReloadStage::ExportState => ReloadStage::StateExport,
        RuntimeReloadStage::MigrateState => ReloadStage::Migration,
        RuntimeReloadStage::ImportState => ReloadStage::StateImport,
        RuntimeReloadStage::Build => ReloadStage::Build,
        RuntimeReloadStage::Validate => ReloadStage::Validation,
        RuntimeReloadStage::Materialize => ReloadStage::Materialization,
        RuntimeReloadStage::PrepareReconciliation => ReloadStage::Reconciliation,
        RuntimeReloadStage::PreCommitCancellation => ReloadStage::Cancellation,
    }
}

/// Materializes one validated Widget IR image into a disconnected Aimer tree.
///
/// The complete portable graph and every concrete widget schema are validated
/// before any element is created. The returned root is disconnected from the
/// live Quiver tree and may be dropped on any later candidate failure without
/// publishing native state or platform resources.
pub fn materialize_aimer_widget_tree(
    image: &[u8],
    limits: ModelLimits,
    ctx: &BuildContext,
    dispatch_callback: impl Fn(StableId128) + 'static,
) -> Result<AnyElement, WidgetMaterializeError<MissingNativeMaterializer>> {
    let linked_schemas = linked_portable_native_widget_schemas();
    let schemas = PortableWidgetSchemaValidator::new_with_additional(
        &BUILTIN_PORTABLE_WIDGET_SCHEMAS,
        linked_schemas,
    )
    .map_err(|error| {
        WidgetMaterializeError::FactorySetup(MissingNativeMaterializer::InvalidRegistry(
            error.to_string(),
        ))
    })?;
    let registry =
        NativeWidgetMaterializerRegistry::new(&BUILTIN_NATIVE_MATERIALIZERS).map_err(|error| {
            WidgetMaterializeError::FactorySetup(MissingNativeMaterializer::InvalidRegistry(
                format!("{error:?}"),
            ))
        })?;
    let mut factory = AimerWidgetFactory {
        ctx,
        dispatch_callback: Rc::new(dispatch_callback),
        schemas,
        registry,
    };
    materialize_widget_tree(image, limits, &mut factory)
}

type NativeWidgetMaterializer = for<'factory, 'document> fn(
    &AimerWidgetFactory<'factory>,
    &WidgetDocumentView<'document>,
    WidgetNodeView<'document>,
    Vec<AnyElement>,
) -> AnyElement;

#[derive(Clone, Copy)]
struct NativeWidgetMaterializerRegistration {
    widget_type: WidgetSchemaId,
    minimum: Version,
    maximum: Version,
    materialize: NativeWidgetMaterializer,
}

impl NativeWidgetMaterializerRegistration {
    #[inline]
    const fn new(
        widget_type: WidgetSchemaId,
        minimum: Version,
        maximum: Version,
        materialize: NativeWidgetMaterializer,
    ) -> Self {
        Self {
            widget_type,
            minimum,
            maximum,
            materialize,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NativeMaterializerRegistryError {
    InvalidVersionRange {
        widget_type: WidgetSchemaId,
        minimum: Version,
        maximum: Version,
    },
    OverlappingVersions {
        widget_type: WidgetSchemaId,
        first_minimum: Version,
        first_maximum: Version,
        second_minimum: Version,
        second_maximum: Version,
    },
}

#[derive(Clone, Copy)]
struct NativeWidgetMaterializerRegistry<'a> {
    registrations: &'a [NativeWidgetMaterializerRegistration],
    derived_registrations: &'a [PortableNativeWidgetRegistration],
}

#[derive(Clone, Copy)]
enum ResolvedNativeWidgetMaterializer {
    Handwritten(NativeWidgetMaterializer),
    Derived(PortableNativeMaterializer),
}

impl<'a> NativeWidgetMaterializerRegistry<'a> {
    fn new(
        registrations: &'a [NativeWidgetMaterializerRegistration],
    ) -> Result<Self, NativeMaterializerRegistryError> {
        aimer_animation::ensure_portable_native_registrations();
        for (index, registration) in registrations.iter().enumerate() {
            if !version_at_least(registration.maximum, registration.minimum) {
                return Err(NativeMaterializerRegistryError::InvalidVersionRange {
                    widget_type: registration.widget_type,
                    minimum: registration.minimum,
                    maximum: registration.maximum,
                });
            }
            for other in &registrations[index + 1..] {
                if registration.widget_type == other.widget_type
                    && version_ranges_overlap(
                        registration.minimum,
                        registration.maximum,
                        other.minimum,
                        other.maximum,
                    )
                {
                    return Err(NativeMaterializerRegistryError::OverlappingVersions {
                        widget_type: registration.widget_type,
                        first_minimum: registration.minimum,
                        first_maximum: registration.maximum,
                        second_minimum: other.minimum,
                        second_maximum: other.maximum,
                    });
                }
            }
        }
        let derived_registrations = linked_portable_native_widget_registrations();
        validate_derived_native_materializer_registrations(derived_registrations)?;
        for derived in derived_registrations.iter().copied() {
            let widget = derived.schema().widget();
            for registration in registrations {
                if registration.widget_type == widget.id()
                    && version_ranges_overlap(
                        registration.minimum,
                        registration.maximum,
                        widget.min_version(),
                        widget.max_version(),
                    )
                {
                    return Err(NativeMaterializerRegistryError::OverlappingVersions {
                        widget_type: widget.id(),
                        first_minimum: registration.minimum,
                        first_maximum: registration.maximum,
                        second_minimum: widget.min_version(),
                        second_maximum: widget.max_version(),
                    });
                }
            }
        }
        Ok(Self {
            registrations,
            derived_registrations,
        })
    }

    #[inline]
    fn resolve(
        self,
        widget_type: WidgetSchemaId,
        version: Version,
    ) -> Option<ResolvedNativeWidgetMaterializer> {
        self.registrations
            .iter()
            .find(|registration| {
                registration.widget_type == widget_type
                    && version_at_least(version, registration.minimum)
                    && version_at_least(registration.maximum, version)
            })
            .map(|registration| {
                ResolvedNativeWidgetMaterializer::Handwritten(registration.materialize)
            })
            .or_else(|| {
                self.derived_registrations
                    .iter()
                    .copied()
                    .find(|registration| registration.supports(widget_type, version))
                    .map(|registration| {
                        ResolvedNativeWidgetMaterializer::Derived(registration.materialize())
                    })
            })
    }
}

#[inline]
fn identical_derived_native_materializers(
    first: PortableNativeWidgetRegistration,
    second: PortableNativeWidgetRegistration,
) -> bool {
    first.schema() == second.schema()
        && std::ptr::fn_addr_eq(first.materialize(), second.materialize())
}

fn validate_derived_native_materializer_registrations(
    derived_registrations: &[PortableNativeWidgetRegistration],
) -> Result<(), NativeMaterializerRegistryError> {
    for (index, registration) in derived_registrations.iter().copied().enumerate() {
        let widget = registration.schema().widget();
        if !version_at_least(widget.max_version(), widget.min_version()) {
            return Err(NativeMaterializerRegistryError::InvalidVersionRange {
                widget_type: widget.id(),
                minimum: widget.min_version(),
                maximum: widget.max_version(),
            });
        }
        for other in &derived_registrations[index + 1..] {
            if identical_derived_native_materializers(registration, *other) {
                continue;
            }
            let other_widget = other.schema().widget();
            if widget.id() == other_widget.id()
                && version_ranges_overlap(
                    widget.min_version(),
                    widget.max_version(),
                    other_widget.min_version(),
                    other_widget.max_version(),
                )
            {
                return Err(NativeMaterializerRegistryError::OverlappingVersions {
                    widget_type: widget.id(),
                    first_minimum: widget.min_version(),
                    first_maximum: widget.max_version(),
                    second_minimum: other_widget.min_version(),
                    second_maximum: other_widget.max_version(),
                });
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NativeMaterializerKind {
    Handwritten,
    Derived,
}

#[cfg(test)]
impl ResolvedNativeWidgetMaterializer {
    #[inline]
    const fn kind(self) -> NativeMaterializerKind {
        match self {
            Self::Handwritten(_) => NativeMaterializerKind::Handwritten,
            Self::Derived(_) => NativeMaterializerKind::Derived,
        }
    }
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GuestLoweringKind {
    Generated,
    Manual,
}

#[cfg(test)]
#[derive(Clone, Copy)]
struct BuiltinPortableCoverageSpec {
    widget_type: WidgetSchemaId,
    schema_only: bool,
    guest_lowering: GuestLoweringKind,
    focused_round_trip_test: &'static str,
}

#[cfg(test)]
const WIDGET_SELECTION_AREA: WidgetSchemaId =
    WidgetSchemaId::from_canonical_name("aimer.widget:aimer_text::SelectionArea");
#[cfg(test)]
const WIDGET_EXPANDED: WidgetSchemaId =
    WidgetSchemaId::from_canonical_name("aimer.widget:aimer_flex::flex::Expanded");
#[cfg(test)]
const WIDGET_POSITIONED: WidgetSchemaId =
    WidgetSchemaId::from_canonical_name("aimer.widget:aimer_space::space::Positioned");
const WIDGET_STACK: WidgetSchemaId =
    WidgetSchemaId::from_canonical_name("aimer.widget:aimer_space::space::Stack");
const WIDGET_GRID: WidgetSchemaId =
    WidgetSchemaId::from_canonical_name("aimer.widget:aimer_grid::grid::Grid");
const PROPERTY_GRID_CONFIG: PropertyId =
    PropertyId::from_canonical_name("aimer.property:aimer_grid::grid::Grid:config");
const WIDGET_SCROLLABLE: WidgetSchemaId =
    WidgetSchemaId::from_canonical_name("aimer.widget:aimer_scroll::scrollable::Scrollable");
const PROPERTY_SCROLLABLE_CONFIG: PropertyId =
    PropertyId::from_canonical_name("aimer.property:aimer_scroll::scrollable::Scrollable:config");
#[cfg(test)]
const WIDGET_SCROLL_BAR: WidgetSchemaId =
    WidgetSchemaId::from_canonical_name("aimer.widget:aimer_scroll::scrollable::ScrollBar");
#[cfg(test)]
const WIDGET_ASPECT_RATIO: WidgetSchemaId =
    WidgetSchemaId::from_canonical_name("aimer.widget:aimer_container::single_child::AspectRatio");
#[cfg(test)]
const WIDGET_ZERO_SIZED_BOX: WidgetSchemaId =
    WidgetSchemaId::from_canonical_name("aimer.widget:aimer_container::single_child::ZeroSizedBox");
#[cfg(test)]
const WIDGET_OPACITY: WidgetSchemaId =
    WidgetSchemaId::from_canonical_name("aimer.widget:aimer_container::single_child::Opacity");
#[cfg(test)]
const WIDGET_FOCUS_SCOPE: WidgetSchemaId =
    WidgetSchemaId::from_canonical_name("aimer.widget:aimer_widget::FocusScope");
#[cfg(test)]
const WIDGET_TEXT_FIELD: WidgetSchemaId =
    WidgetSchemaId::from_canonical_name("aimer.widget:aimer_input::TextField");
#[cfg(test)]
const WIDGET_TEXT_AREA: WidgetSchemaId =
    WidgetSchemaId::from_canonical_name("aimer.widget:aimer_input::TextArea");
#[cfg(test)]
const WIDGET_RICH_TEXT: WidgetSchemaId =
    WidgetSchemaId::from_canonical_name("aimer.widget:aimer_text::RichText");
#[cfg(test)]
const WIDGET_CONTEXT_MENU_ROWS: WidgetSchemaId =
    WidgetSchemaId::from_canonical_name("aimer.widget:aimer_ctxmenu::ContextMenuRows");
#[cfg(test)]
const WIDGET_CONTEXT_MENU: WidgetSchemaId =
    WidgetSchemaId::from_canonical_name("aimer.widget:aimer_ctxmenu::ContextMenu");
#[cfg(test)]
const WIDGET_SCALABLE: WidgetSchemaId =
    WidgetSchemaId::from_canonical_name("aimer.widget:aimer_container::single_child::Scalable");
#[cfg(test)]
const WIDGET_RESIZABLE: WidgetSchemaId =
    WidgetSchemaId::from_canonical_name("aimer.widget:aimer_container::single_child::Resizable");
#[cfg(test)]
const WIDGET_ANIMATED_BUILDER: WidgetSchemaId =
    WidgetSchemaId::from_canonical_name("aimer.widget:aimer_animation::AnimatedBuilder");
#[cfg(test)]
const WIDGET_NAMED_WIDGET: WidgetSchemaId = WidgetSchemaId::from_canonical_name(
    "aimer.widget:aimer_widget::widget::stateless::NamedWidget",
);
#[cfg(test)]
const WIDGET_CHILD_BUILDER: WidgetSchemaId =
    WidgetSchemaId::from_canonical_name("aimer.widget:aimer_widget::ChildBuilder");

// This is deliberately a checked manifest rather than a claim that every
// linked derive is portable. In particular, a derived schema stays absent when
// it skips native behavior without rejecting non-default values, or when its
// native Widget implementation is unfinished. The audit below resolves each
// constructor independently from the linked registry and reports its kind.
#[cfg(test)]
const BUILTIN_PORTABLE_COVERAGE: [BuiltinPortableCoverageSpec; 33] = [
    BuiltinPortableCoverageSpec {
        widget_type: WIDGET_COLUMN,
        schema_only: true,
        guest_lowering: GuestLoweringKind::Generated,
        focused_round_trip_test: "portable_builtin_showcase_round_trips_through_host",
    },
    BuiltinPortableCoverageSpec {
        widget_type: WIDGET_ROW,
        schema_only: true,
        guest_lowering: GuestLoweringKind::Generated,
        focused_round_trip_test: "portable_builtin_showcase_round_trips_through_host",
    },
    BuiltinPortableCoverageSpec {
        widget_type: WIDGET_CONTAINER,
        schema_only: false,
        guest_lowering: GuestLoweringKind::Generated,
        focused_round_trip_test: "portable_builtin_showcase_round_trips_through_host",
    },
    BuiltinPortableCoverageSpec {
        widget_type: WIDGET_SIZED_BOX,
        schema_only: true,
        guest_lowering: GuestLoweringKind::Generated,
        focused_round_trip_test: "portable_builtin_showcase_round_trips_through_host",
    },
    BuiltinPortableCoverageSpec {
        widget_type: WIDGET_TEXT,
        schema_only: false,
        guest_lowering: GuestLoweringKind::Generated,
        focused_round_trip_test: "portable_builtin_showcase_round_trips_through_host",
    },
    BuiltinPortableCoverageSpec {
        widget_type: WIDGET_EXPANDED,
        schema_only: false,
        guest_lowering: GuestLoweringKind::Generated,
        focused_round_trip_test: "safe_derived_portable_widgets_round_trip_through_linked_host_materializers",
    },
    BuiltinPortableCoverageSpec {
        widget_type: WIDGET_POSITIONED,
        schema_only: false,
        guest_lowering: GuestLoweringKind::Generated,
        focused_round_trip_test: "safe_derived_portable_widgets_round_trip_through_linked_host_materializers",
    },
    BuiltinPortableCoverageSpec {
        widget_type: WIDGET_STACK,
        schema_only: true,
        guest_lowering: GuestLoweringKind::Generated,
        focused_round_trip_test: "stack_round_trips_through_its_host_materializer",
    },
    BuiltinPortableCoverageSpec {
        widget_type: WIDGET_GRID,
        schema_only: true,
        guest_lowering: GuestLoweringKind::Manual,
        focused_round_trip_test: "grid_round_trips_layout_configuration_and_children",
    },
    BuiltinPortableCoverageSpec {
        widget_type: WIDGET_SCROLLABLE,
        schema_only: true,
        guest_lowering: GuestLoweringKind::Manual,
        focused_round_trip_test: "scrollable_round_trips_axis_and_scrollbars_through_host_materializer",
    },
    BuiltinPortableCoverageSpec {
        widget_type: WIDGET_SCROLL_BAR,
        schema_only: true,
        guest_lowering: GuestLoweringKind::Generated,
        focused_round_trip_test: "scroll_bar_round_trips_through_host_materializer",
    },
    BuiltinPortableCoverageSpec {
        widget_type: WIDGET_BUTTON,
        schema_only: true,
        guest_lowering: GuestLoweringKind::Generated,
        focused_round_trip_test: "portable_builtin_showcase_round_trips_through_host",
    },
    BuiltinPortableCoverageSpec {
        widget_type: WIDGET_TEXT_BUTTON,
        schema_only: true,
        guest_lowering: GuestLoweringKind::Generated,
        focused_round_trip_test: "text_button_materialization_routes_properties_and_callbacks",
    },
    BuiltinPortableCoverageSpec {
        widget_type: WIDGET_PROVIDER,
        schema_only: true,
        guest_lowering: GuestLoweringKind::Manual,
        focused_round_trip_test: "portable_builtin_showcase_round_trips_through_host",
    },
    BuiltinPortableCoverageSpec {
        widget_type: WIDGET_ANIMATED_THEME,
        schema_only: true,
        guest_lowering: GuestLoweringKind::Manual,
        focused_round_trip_test: "portable_builtin_showcase_round_trips_through_host",
    },
    BuiltinPortableCoverageSpec {
        widget_type: WIDGET_ERROR,
        schema_only: true,
        guest_lowering: GuestLoweringKind::Generated,
        focused_round_trip_test: "portable_builtin_showcase_round_trips_through_host",
    },
    BuiltinPortableCoverageSpec {
        widget_type: WIDGET_GESTURE_DETECTOR,
        schema_only: true,
        guest_lowering: GuestLoweringKind::Generated,
        focused_round_trip_test: "gesture_detector_materializes_its_child_and_callback_binding",
    },
    BuiltinPortableCoverageSpec {
        widget_type: WIDGET_MOUSE_REGION,
        schema_only: true,
        guest_lowering: GuestLoweringKind::Generated,
        focused_round_trip_test: "mouse_region_materializes_its_child_and_callback_bindings",
    },
    BuiltinPortableCoverageSpec {
        widget_type: WIDGET_SELECTION_AREA,
        schema_only: false,
        guest_lowering: GuestLoweringKind::Generated,
        focused_round_trip_test: "safe_derived_portable_widgets_round_trip_through_linked_host_materializers",
    },
    BuiltinPortableCoverageSpec {
        widget_type: WIDGET_ASPECT_RATIO,
        schema_only: false,
        guest_lowering: GuestLoweringKind::Generated,
        focused_round_trip_test: "safe_derived_portable_widgets_round_trip_through_linked_host_materializers",
    },
    BuiltinPortableCoverageSpec {
        widget_type: WIDGET_ZERO_SIZED_BOX,
        schema_only: false,
        guest_lowering: GuestLoweringKind::Generated,
        focused_round_trip_test: "safe_derived_portable_widgets_round_trip_through_linked_host_materializers",
    },
    BuiltinPortableCoverageSpec {
        widget_type: WIDGET_OPACITY,
        schema_only: false,
        guest_lowering: GuestLoweringKind::Generated,
        focused_round_trip_test: "safe_derived_portable_widgets_round_trip_through_linked_host_materializers",
    },
    BuiltinPortableCoverageSpec {
        widget_type: WIDGET_FOCUS_SCOPE,
        schema_only: false,
        guest_lowering: GuestLoweringKind::Generated,
        focused_round_trip_test: "safe_derived_portable_widgets_round_trip_through_linked_host_materializers",
    },
    BuiltinPortableCoverageSpec {
        widget_type: WIDGET_TEXT_FIELD,
        schema_only: false,
        guest_lowering: GuestLoweringKind::Generated,
        focused_round_trip_test: "text_input_contract_round_trips_bounded_configuration_without_native_handles",
    },
    BuiltinPortableCoverageSpec {
        widget_type: WIDGET_TEXT_AREA,
        schema_only: false,
        guest_lowering: GuestLoweringKind::Generated,
        focused_round_trip_test: "text_input_contract_round_trips_bounded_configuration_without_native_handles",
    },
    BuiltinPortableCoverageSpec {
        widget_type: WIDGET_RICH_TEXT,
        schema_only: false,
        guest_lowering: GuestLoweringKind::Generated,
        focused_round_trip_test: "rich_text_content_round_trips_through_guest_ir_and_native_materializer",
    },
    BuiltinPortableCoverageSpec {
        widget_type: WIDGET_CONTEXT_MENU_ROWS,
        schema_only: false,
        guest_lowering: GuestLoweringKind::Generated,
        focused_round_trip_test: "context_menu_rows_round_trip_through_guest_ir_and_host_materializer",
    },
    BuiltinPortableCoverageSpec {
        widget_type: WIDGET_CONTEXT_MENU,
        schema_only: false,
        guest_lowering: GuestLoweringKind::Generated,
        focused_round_trip_test: "context_menu_round_trip_through_guest_ir_and_host_materializer",
    },
    BuiltinPortableCoverageSpec {
        widget_type: WIDGET_SCALABLE,
        schema_only: false,
        guest_lowering: GuestLoweringKind::Generated,
        focused_round_trip_test: "scalable_lowering_preserves_scale_and_required_child",
    },
    BuiltinPortableCoverageSpec {
        widget_type: WIDGET_RESIZABLE,
        schema_only: false,
        guest_lowering: GuestLoweringKind::Generated,
        focused_round_trip_test: "resizable_lowering_preserves_every_supported_property",
    },
    BuiltinPortableCoverageSpec {
        widget_type: WIDGET_ANIMATED_BUILDER,
        schema_only: false,
        guest_lowering: GuestLoweringKind::Generated,
        focused_round_trip_test: "animated_builder_round_trip_through_guest_ir_and_host_materializer",
    },
    BuiltinPortableCoverageSpec {
        widget_type: WIDGET_NAMED_WIDGET,
        schema_only: false,
        guest_lowering: GuestLoweringKind::Generated,
        focused_round_trip_test: "named_widget_guest_lowering_is_transparent_to_its_inner_widget",
    },
    BuiltinPortableCoverageSpec {
        widget_type: WIDGET_CHILD_BUILDER,
        schema_only: false,
        guest_lowering: GuestLoweringKind::Generated,
        focused_round_trip_test: "child_builder_guest_lowering_consumes_only_a_unique_source",
    },
];

#[cfg(test)]
#[derive(Clone, Debug, Eq, PartialEq)]
struct PortableBuiltinCoverageEntry {
    widget_type: WidgetSchemaId,
    canonical_name: String,
    schema_only: Option<bool>,
    schema_validation: bool,
    guest_lowering: Option<GuestLoweringKind>,
    host_materializer: Option<NativeMaterializerKind>,
    focused_round_trip_test: Option<&'static str>,
}

#[cfg(test)]
impl PortableBuiltinCoverageEntry {
    fn missing_contracts(&self) -> Vec<&'static str> {
        let mut missing = Vec::new();
        if !self.schema_validation {
            missing.push("schema validation");
        }
        if self.guest_lowering.is_none() {
            missing.push("guest lowering");
        }
        if self.host_materializer.is_none() {
            missing.push("host materializer");
        }
        if self.focused_round_trip_test.is_none() {
            missing.push("focused round-trip coverage");
        }
        missing
    }
}

#[cfg(test)]
#[derive(Clone, Debug, Eq, PartialEq)]
struct PortableBuiltinCoverageReport {
    registry_errors: Vec<String>,
    entries: Vec<PortableBuiltinCoverageEntry>,
}

#[cfg(test)]
impl PortableBuiltinCoverageReport {
    fn is_complete(&self) -> bool {
        self.registry_errors.is_empty()
            && self
                .entries
                .iter()
                .all(|entry| entry.missing_contracts().is_empty())
    }
}

#[cfg(test)]
impl fmt::Display for PortableBuiltinCoverageReport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.registry_errors.is_empty() && self.is_complete() {
            return write!(
                formatter,
                "portable built-in registry coverage complete ({} schemas)",
                self.entries.len(),
            );
        }

        for error in &self.registry_errors {
            writeln!(formatter, "registry error: {error}")?;
        }
        for entry in &self.entries {
            let missing = entry.missing_contracts();
            if missing.is_empty() {
                continue;
            }
            writeln!(
                formatter,
                "{} ({}) missing {}{}",
                entry.canonical_name,
                entry.widget_type,
                missing.join(", "),
                if entry.schema_only == Some(true) {
                    " [schema_only]"
                } else {
                    ""
                },
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
fn audit_portable_builtin_registry() -> PortableBuiltinCoverageReport {
    let linked_schemas = linked_portable_native_widget_schemas();
    let mut inventory = BUILTIN_PORTABLE_WIDGET_SCHEMAS.to_vec();
    inventory.extend(
        linked_schemas
            .iter()
            .copied()
            .filter(|schema| !is_test_fixture_schema(schema)),
    );
    let mut registry_errors = Vec::new();
    let validator = match PortableWidgetSchemaValidator::new_with_additional(
        &BUILTIN_PORTABLE_WIDGET_SCHEMAS,
        linked_schemas,
    ) {
        Ok(validator) => Some(validator),
        Err(error) => {
            registry_errors.push(error.to_string());
            None
        }
    };
    let native_registry = match NativeWidgetMaterializerRegistry::new(&BUILTIN_NATIVE_MATERIALIZERS)
    {
        Ok(registry) => Some(registry),
        Err(error) => {
            registry_errors.push(format!("{error:?}"));
            None
        }
    };

    let mut entries = Vec::with_capacity(inventory.len());
    for schema in inventory {
        let widget = schema.widget();
        let spec = BUILTIN_PORTABLE_COVERAGE
            .iter()
            .find(|spec| spec.widget_type == widget.id());
        let host_materializer = native_registry.and_then(|registry| {
            registry
                .resolve(widget.id(), widget.min_version())
                .map(ResolvedNativeWidgetMaterializer::kind)
        });
        entries.push(PortableBuiltinCoverageEntry {
            widget_type: widget.id(),
            canonical_name: widget.canonical_name().to_owned(),
            schema_only: spec.map(|spec| spec.schema_only),
            schema_validation: validator
                .map(|validator| validator.supports(widget.id(), widget.min_version()))
                .unwrap_or(false),
            guest_lowering: spec.map(|spec| spec.guest_lowering),
            host_materializer,
            focused_round_trip_test: spec.map(|spec| spec.focused_round_trip_test),
        });
    }

    for spec in BUILTIN_PORTABLE_COVERAGE {
        if entries
            .iter()
            .all(|entry| entry.widget_type != spec.widget_type)
        {
            entries.push(PortableBuiltinCoverageEntry {
                widget_type: spec.widget_type,
                canonical_name: format!("unregistered widget {}", spec.widget_type),
                schema_only: Some(spec.schema_only),
                schema_validation: false,
                guest_lowering: Some(spec.guest_lowering),
                host_materializer: None,
                focused_round_trip_test: Some(spec.focused_round_trip_test),
            });
        }
    }

    PortableBuiltinCoverageReport {
        registry_errors,
        entries,
    }
}

#[cfg(test)]
#[inline]
fn is_test_fixture_schema(schema: &aimer_anteros::PortableWidgetSchemaMetadata<'_>) -> bool {
    schema
        .widget()
        .canonical_name()
        .starts_with("aimer.widget:aimer_quiver.tests.")
}

#[inline]
const fn version_at_least(version: Version, minimum: Version) -> bool {
    version.major() > minimum.major()
        || (version.major() == minimum.major() && version.minor() >= minimum.minor())
}

#[inline]
const fn version_ranges_overlap(
    first_minimum: Version,
    first_maximum: Version,
    second_minimum: Version,
    second_maximum: Version,
) -> bool {
    version_at_least(first_maximum, second_minimum)
        && version_at_least(second_maximum, first_minimum)
}

/// A validated widget schema has no native constructor in the permanent host.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MissingNativeMaterializer {
    /// Linked schemas or constructors conflict and cannot be selected safely.
    InvalidRegistry(String),
    /// No constructor was linked for the validated widget schema.
    Missing {
        widget_type: WidgetSchemaId,
        schema: Version,
    },
    /// A derived constructor rejected a final checked Rust conversion.
    Derived(PortableMaterializeError),
}

impl fmt::Display for MissingNativeMaterializer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRegistry(error) => {
                write!(formatter, "invalid native materializer registry: {error}")
            }
            Self::Missing {
                widget_type,
                schema,
            } => write!(
                formatter,
                "no native materializer is registered for widget {} schema {}.{}; rebuild and relaunch the application",
                widget_type,
                schema.major(),
                schema.minor(),
            ),
            Self::Derived(error) => {
                write!(formatter, "derived native materializer failed: {error}")
            }
        }
    }
}

impl Error for MissingNativeMaterializer {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidRegistry(_) | Self::Missing { .. } => None,
            Self::Derived(error) => Some(error),
        }
    }
}

struct AimerWidgetFactory<'a> {
    ctx: &'a BuildContext<'a>,
    dispatch_callback: Rc<dyn Fn(StableId128)>,
    schemas: PortableWidgetSchemaValidator<'static>,
    registry: NativeWidgetMaterializerRegistry<'static>,
}

impl WidgetSchemaSupport for AimerWidgetFactory<'_> {
    fn supports(&self, widget_type: WidgetSchemaId, schema: Version) -> bool {
        self.schemas.supports(widget_type, schema)
    }

    fn validate_node(
        &self,
        document: &WidgetDocumentView<'_>,
        node_index: u32,
        node: WidgetNodeView<'_>,
    ) -> Result<(), ModelError> {
        self.schemas.validate_node(document, node_index, node)?;
        let node = document
            .node(node_index)
            .ok_or(ModelError::NodeIndexOutOfBounds {
                index: node_index,
                node_count: document.node_count(),
            })?;
        for property in node.properties() {
            validate_property_value(document, node_index, node.widget_type(), property)?;
        }
        match node.widget_type() {
            WIDGET_PROVIDER => validate_provider_value(document, node_index, node)?,
            WIDGET_ANIMATED_THEME => validate_animated_theme_value(document, node_index, node)?,
            WIDGET_GRID => validate_grid_value(document, node_index, node)?,
            WIDGET_SCROLLABLE => validate_scrollable_value(document, node_index, node)?,
            _ => {}
        }
        Ok(())
    }
}

impl WidgetFactory for AimerWidgetFactory<'_> {
    type Error = MissingNativeMaterializer;
    type Node = AnyElement;

    fn build(
        &mut self,
        document: &WidgetDocumentView<'_>,
        _node_index: u32,
        node: WidgetNodeView<'_>,
        children: Vec<Self::Node>,
    ) -> Result<Self::Node, Self::Error> {
        let key = node.key();
        let widget_type = node.widget_type();
        let materialize = self
            .registry
            .resolve(widget_type, node.widget_schema())
            .ok_or(MissingNativeMaterializer::Missing {
                widget_type,
                schema: node.widget_schema(),
            })?;
        let element = match materialize {
            ResolvedNativeWidgetMaterializer::Handwritten(materialize) => {
                materialize(self, document, node, children)
            }
            ResolvedNativeWidgetMaterializer::Derived(materialize) => {
                let children = children
                    .into_iter()
                    .map(|child| MaterializedElementWidget(child).boxed())
                    .collect();
                materialize(document, node, children)
                    .map_err(MissingNativeMaterializer::Derived)?
                    .to_element(self.ctx)
            }
        };
        Ok(match key {
            Some(key) => StatelessElement::wrapper(
                element,
                Some(Key::fixed(*key.as_bytes())),
                widget_debug_name(widget_type),
            )
            .boxed(),
            None => element,
        })
    }
}

struct MaterializedElementWidget(AnyElement);

impl Widget for MaterializedElementWidget {
    #[inline]
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        self.0
    }
}

impl aimer_widget::PortableWidget for MaterializedElementWidget {}

const SCHEMA_V1: Version = BUILTIN_WIDGET_SCHEMA_VERSION;
const WIDGET_ERROR: WidgetSchemaId =
    WidgetSchemaId::from_canonical_name("aimer.widget:aimer_widget::ErrorWidget");
const PROPERTY_ERROR_MESSAGE: PropertyId =
    PropertyId::from_canonical_name("aimer.property:aimer_widget::ErrorWidget:message");
const WIDGET_TEXT_BUTTON: WidgetSchemaId =
    WidgetSchemaId::from_canonical_name("aimer.widget:aimer_text::TextButton");
const PROPERTY_TEXT_BUTTON_DISABLED: PropertyId =
    PropertyId::from_canonical_name("aimer.property:aimer_text::TextButton:disabled");
const PROPERTY_TEXT_BUTTON_LABEL: PropertyId =
    PropertyId::from_canonical_name("aimer.property:aimer_text::TextButton:label");
const PROPERTY_TEXT_BUTTON_COLOR: PropertyId =
    PropertyId::from_canonical_name("aimer.property:aimer_text::TextButton:color");
const PROPERTY_TEXT_BUTTON_HOVER_COLOR: PropertyId =
    PropertyId::from_canonical_name("aimer.property:aimer_text::TextButton:hover_color");
const PROPERTY_TEXT_BUTTON_DISABLED_COLOR: PropertyId =
    PropertyId::from_canonical_name("aimer.property:aimer_text::TextButton:disabled_color");
const PROPERTY_TEXT_BUTTON_STYLE: PropertyId =
    PropertyId::from_canonical_name("aimer.property:aimer_text::TextButton:style");
const PROPERTY_TEXT_BUTTON_HOVER_STYLE: PropertyId =
    PropertyId::from_canonical_name("aimer.property:aimer_text::TextButton:hover_style");
const PROPERTY_TEXT_BUTTON_DISABLED_STYLE: PropertyId =
    PropertyId::from_canonical_name("aimer.property:aimer_text::TextButton:disabled_style");
const EVENT_TEXT_BUTTON_PRESS: EventId =
    EventId::from_canonical_name("aimer.event:aimer_text::TextButton:on_press");
const EVENT_TEXT_BUTTON_DOUBLE_PRESS: EventId =
    EventId::from_canonical_name("aimer.event:aimer_text::TextButton:on_double_press");

static BUILTIN_NATIVE_MATERIALIZERS: [NativeWidgetMaterializerRegistration; 13] = [
    NativeWidgetMaterializerRegistration::new(
        WIDGET_COLUMN,
        SCHEMA_V1,
        SCHEMA_V1,
        materialize_column,
    ),
    NativeWidgetMaterializerRegistration::new(WIDGET_ROW, SCHEMA_V1, SCHEMA_V1, materialize_row),
    NativeWidgetMaterializerRegistration::new(
        WIDGET_SIZED_BOX,
        SCHEMA_V1,
        SCHEMA_V1,
        materialize_sized_box,
    ),
    NativeWidgetMaterializerRegistration::new(
        WIDGET_STACK,
        SCHEMA_V1,
        SCHEMA_V1,
        materialize_stack,
    ),
    NativeWidgetMaterializerRegistration::new(WIDGET_GRID, SCHEMA_V1, SCHEMA_V1, materialize_grid),
    NativeWidgetMaterializerRegistration::new(
        WIDGET_SCROLLABLE,
        SCHEMA_V1,
        SCHEMA_V1,
        materialize_scrollable,
    ),
    NativeWidgetMaterializerRegistration::new(
        WIDGET_BUTTON,
        SCHEMA_V1,
        SCHEMA_V1,
        materialize_button,
    ),
    NativeWidgetMaterializerRegistration::new(
        WIDGET_PROVIDER,
        SCHEMA_V1,
        SCHEMA_V1,
        materialize_provider,
    ),
    NativeWidgetMaterializerRegistration::new(
        WIDGET_ANIMATED_THEME,
        SCHEMA_V1,
        SCHEMA_V1,
        materialize_animated_theme,
    ),
    NativeWidgetMaterializerRegistration::new(
        WIDGET_ERROR,
        SCHEMA_V1,
        SCHEMA_V1,
        materialize_error_widget,
    ),
    NativeWidgetMaterializerRegistration::new(
        WIDGET_TEXT_BUTTON,
        SCHEMA_V1,
        SCHEMA_V1,
        materialize_text_button,
    ),
    NativeWidgetMaterializerRegistration::new(
        WIDGET_GESTURE_DETECTOR,
        SCHEMA_V1,
        SCHEMA_V1,
        materialize_gesture_detector,
    ),
    NativeWidgetMaterializerRegistration::new(
        WIDGET_MOUSE_REGION,
        SCHEMA_V1,
        SCHEMA_V1,
        materialize_mouse_region,
    ),
];

fn materialize_column(
    factory: &AimerWidgetFactory<'_>,
    document: &WidgetDocumentView<'_>,
    node: WidgetNodeView<'_>,
    children: Vec<AnyElement>,
) -> AnyElement {
    let vertical_alignment = optional_materialized_property::<BoxAlignment>(
        document,
        &node,
        PROPERTY_COLUMN_VERTICAL_ALIGNMENT,
    )
    .expect("validated Column vertical alignment is decodable")
    .unwrap_or_default();
    let horizontal_alignment = optional_materialized_property::<BoxAlignment>(
        document,
        &node,
        PROPERTY_COLUMN_HORIZONTAL_ALIGNMENT,
    )
    .expect("validated Column horizontal alignment is decodable")
    .unwrap_or_default();
    let justify_content = optional_materialized_property::<JustifyContent>(
        document,
        &node,
        PROPERTY_COLUMN_JUSTIFY_CONTENT,
    )
    .expect("validated Column justify content is decodable");
    let gaps =
        optional_materialized_property::<LayoutSpacing>(document, &node, PROPERTY_COLUMN_GAPS)
            .expect("validated Column gaps are decodable")
            .unwrap_or_default();
    let overflow = optional_materialized_property::<OverflowBehavior>(
        document,
        &node,
        PROPERTY_COLUMN_OVERFLOW,
    )
    .expect("validated Column overflow is decodable")
    .unwrap_or_default();

    let mut column = Column::new()
        .vertical_alignment(vertical_alignment)
        .horizontal_alignment(horizontal_alignment)
        .gaps(gaps)
        .overflow(overflow);
    if let Some(justify_content) = justify_content {
        column = column.justify_content(justify_content);
    }
    column
        .children(children.into_iter().map(RetainedElementWidget))
        .to_element(factory.ctx)
}

fn materialize_row(
    factory: &AimerWidgetFactory<'_>,
    document: &WidgetDocumentView<'_>,
    node: WidgetNodeView<'_>,
    children: Vec<AnyElement>,
) -> AnyElement {
    let vertical_alignment = optional_materialized_property::<BoxAlignment>(
        document,
        &node,
        PROPERTY_ROW_VERTICAL_ALIGNMENT,
    )
    .expect("validated Row vertical alignment is decodable")
    .unwrap_or_default();
    let horizontal_alignment = optional_materialized_property::<BoxAlignment>(
        document,
        &node,
        PROPERTY_ROW_HORIZONTAL_ALIGNMENT,
    )
    .expect("validated Row horizontal alignment is decodable")
    .unwrap_or_default();
    let justify_content = optional_materialized_property::<JustifyContent>(
        document,
        &node,
        PROPERTY_ROW_JUSTIFY_CONTENT,
    )
    .expect("validated Row justify content is decodable");
    let gaps = optional_materialized_property::<LayoutSpacing>(document, &node, PROPERTY_ROW_GAPS)
        .expect("validated Row gaps are decodable")
        .unwrap_or_default();
    let overflow =
        optional_materialized_property::<OverflowBehavior>(document, &node, PROPERTY_ROW_OVERFLOW)
            .expect("validated Row overflow is decodable")
            .unwrap_or_default();

    let mut row = Row::new()
        .vertical_alignment(vertical_alignment)
        .horizontal_alignment(horizontal_alignment)
        .gaps(gaps)
        .overflow(overflow);
    if let Some(justify_content) = justify_content {
        row = row.justify_content(justify_content);
    }
    row.children(children.into_iter().map(RetainedElementWidget))
        .to_element(factory.ctx)
}

fn materialize_sized_box(
    factory: &AimerWidgetFactory<'_>,
    document: &WidgetDocumentView<'_>,
    node: WidgetNodeView<'_>,
    _children: Vec<AnyElement>,
) -> AnyElement {
    build_sized_box(document, &node, factory.ctx)
}

fn materialize_stack(
    factory: &AimerWidgetFactory<'_>,
    _document: &WidgetDocumentView<'_>,
    _node: WidgetNodeView<'_>,
    children: Vec<AnyElement>,
) -> AnyElement {
    let stack = children.into_iter().fold(Stack::new(), |stack, child| {
        stack.add_child(RetainedElementWidget(child))
    });
    stack.to_element(factory.ctx)
}

fn materialize_grid(
    factory: &AimerWidgetFactory<'_>,
    document: &WidgetDocumentView<'_>,
    node: WidgetNodeView<'_>,
    children: Vec<AnyElement>,
) -> AnyElement {
    let config = match required_materialized_property::<GridPortableConfig>(
        document,
        &node,
        PROPERTY_GRID_CONFIG,
    ) {
        Ok(config) => config,
        Err(error) => {
            return ErrorWidget::new(format!("Grid portable configuration error: {error}"))
                .to_element(factory.ctx);
        }
    };
    if config.items.len() != children.len() {
        return ErrorWidget::new(format!(
            "Grid portable configuration has {} items for {} children",
            config.items.len(),
            children.len(),
        ))
        .to_element(factory.ctx);
    }

    let GridPortableConfig {
        columns,
        rows,
        column_gap,
        row_gap,
        horizontal_alignment,
        vertical_alignment,
        overflow,
        items,
    } = config;
    let grid = Grid::new()
        .columns(columns)
        .rows(rows)
        .column_gap(column_gap)
        .row_gap(row_gap)
        .horizontal_alignment(horizontal_alignment)
        .vertical_alignment(vertical_alignment)
        .overflow(overflow)
        .children(items.into_iter().zip(children).map(|(item, child)| {
            let placement = item.placement;
            let mut grid_item = GridItem::new(RetainedElementWidget(child));
            if let Some(row) = placement.row {
                grid_item = grid_item.row(row);
            }
            if let Some(column) = placement.column {
                grid_item = grid_item.column(column);
            }
            grid_item = grid_item
                .row_span(placement.row_span)
                .column_span(placement.column_span);
            if let Some(alignment) = item.horizontal_alignment {
                grid_item = grid_item.horizontal_alignment(alignment);
            }
            if let Some(alignment) = item.vertical_alignment {
                grid_item = grid_item.vertical_alignment(alignment);
            }
            grid_item
        }));
    grid.to_element(factory.ctx)
}

fn materialize_scrollable(
    factory: &AimerWidgetFactory<'_>,
    document: &WidgetDocumentView<'_>,
    node: WidgetNodeView<'_>,
    mut children: Vec<AnyElement>,
) -> AnyElement {
    let config = match required_materialized_property::<ScrollPortableConfig>(
        document,
        &node,
        PROPERTY_SCROLLABLE_CONFIG,
    ) {
        Ok(config) => config,
        Err(error) => {
            return ErrorWidget::new(format!("Scrollable portable configuration error: {error}"))
                .to_element(factory.ctx);
        }
    };
    let Some(child) = children.pop() else {
        return ErrorWidget::new("Scrollable portable node is missing its child")
            .to_element(factory.ctx);
    };
    if !children.is_empty() {
        return ErrorWidget::new("Scrollable portable node has too many children")
            .to_element(factory.ctx);
    }

    Scrollable::new()
        .axis(config.axis)
        .scroll_bar_placement(config.scroll_bar_placement)
        .vertical_scroll_bar(config.vertical_scroll_bar.then(ScrollBar::default))
        .horizontal_scroll_bar(config.horizontal_scroll_bar.then(ScrollBar::default))
        .child(RetainedElementWidget(child))
        .to_element(factory.ctx)
}

fn materialize_error_widget(
    factory: &AimerWidgetFactory<'_>,
    document: &WidgetDocumentView<'_>,
    node: WidgetNodeView<'_>,
    children: Vec<AnyElement>,
) -> AnyElement {
    debug_assert!(children.is_empty());
    let message = required_materialized_property::<String>(document, &node, PROPERTY_ERROR_MESSAGE)
        .expect("validated ErrorWidget message property is present and decodable");
    ErrorWidget::new(message).to_element(factory.ctx)
}

fn materialize_text_button(
    factory: &AimerWidgetFactory<'_>,
    document: &WidgetDocumentView<'_>,
    node: WidgetNodeView<'_>,
    children: Vec<AnyElement>,
) -> AnyElement {
    debug_assert!(children.is_empty());
    let disabled =
        required_materialized_property::<bool>(document, &node, PROPERTY_TEXT_BUTTON_DISABLED)
            .expect("validated TextButton disabled property is present and decodable");
    let label =
        required_materialized_property::<TextSource>(document, &node, PROPERTY_TEXT_BUTTON_LABEL)
            .expect("validated TextButton label property is present and decodable");
    let color: Option<aimer_widget::base::Color> =
        aimer_widget::portable::optional_materialized_property(
            document,
            &node,
            PROPERTY_TEXT_BUTTON_COLOR,
        )
        .expect("validated TextButton color property is decodable");
    let hover_color: Option<aimer_widget::base::Color> =
        aimer_widget::portable::optional_materialized_property(
            document,
            &node,
            PROPERTY_TEXT_BUTTON_HOVER_COLOR,
        )
        .expect("validated TextButton hover color property is decodable");
    let disabled_color: Option<aimer_widget::base::Color> =
        aimer_widget::portable::optional_materialized_property(
            document,
            &node,
            PROPERTY_TEXT_BUTTON_DISABLED_COLOR,
        )
        .expect("validated TextButton disabled color property is decodable");
    let style: Option<TextStyle> =
        optional_materialized_property(document, &node, PROPERTY_TEXT_BUTTON_STYLE)
            .expect("validated TextButton style property is decodable");
    let hover_style: Option<TextStyle> =
        optional_materialized_property(document, &node, PROPERTY_TEXT_BUTTON_HOVER_STYLE)
            .expect("validated TextButton hover style property is decodable");
    let disabled_style: Option<TextStyle> =
        optional_materialized_property(document, &node, PROPERTY_TEXT_BUTTON_DISABLED_STYLE)
            .expect("validated TextButton disabled style property is decodable");

    let mut button = TextButton::new(label).disabled(disabled);
    if let Some(style) = style {
        button = button.style(style);
    }
    if let Some(style) = hover_style {
        button = button.hover_style(style);
    }
    if let Some(style) = disabled_style {
        button = button.disabled_style(style);
    }
    if let Some(color) = color {
        button = button.color(color);
    }
    if let Some(color) = hover_color {
        button = button.hover_color(color);
    }
    if let Some(color) = disabled_color {
        button = button.disabled_color(color);
    }
    bind_text_button_callbacks(button, node.callbacks(), &factory.dispatch_callback)
        .to_element(factory.ctx)
}

fn materialize_gesture_detector(
    factory: &AimerWidgetFactory<'_>,
    _document: &WidgetDocumentView<'_>,
    node: WidgetNodeView<'_>,
    mut children: Vec<AnyElement>,
) -> AnyElement {
    let child = children
        .pop()
        .expect("validated GestureDetector has exactly one child");
    debug_assert!(children.is_empty());
    bind_gesture_detector_callbacks(
        GestureDetector::new(),
        node.callbacks(),
        &factory.dispatch_callback,
    )
    .child(RetainedElementWidget(child))
    .to_element(factory.ctx)
}

fn materialize_mouse_region(
    factory: &AimerWidgetFactory<'_>,
    _document: &WidgetDocumentView<'_>,
    node: WidgetNodeView<'_>,
    mut children: Vec<AnyElement>,
) -> AnyElement {
    let child = children
        .pop()
        .expect("validated MouseRegion has exactly one child");
    debug_assert!(children.is_empty());
    bind_mouse_region_callbacks(
        MouseRegion::new(),
        node.callbacks(),
        &factory.dispatch_callback,
    )
    .child(RetainedElementWidget(child))
    .to_element(factory.ctx)
}



fn materialize_button(
    factory: &AimerWidgetFactory<'_>,
    document: &WidgetDocumentView<'_>,
    node: WidgetNodeView<'_>,
    mut children: Vec<AnyElement>,
) -> AnyElement {
    factory.build_button(document, &node, &mut children)
}

fn materialize_provider(
    factory: &AimerWidgetFactory<'_>,
    document: &WidgetDocumentView<'_>,
    node: WidgetNodeView<'_>,
    mut children: Vec<AnyElement>,
) -> AnyElement {
    let version = unpack_provider_version(
        provider_i64_property(&node, PROPERTY_PROVIDER_SCHEMA_VERSION)
            .expect("validated Provider schema version property is present"),
    )
    .expect("validated Provider schema version is representable");
    let blob_index = match provider_property(&node, PROPERTY_PROVIDER_VALUE) {
        Some(PropertyValue::BlobRef(index)) => index,
        _ => unreachable!("validated Provider value property changed during materialization"),
    };
    let bytes = document
        .blob(blob_index)
        .expect("validated Provider blob reference is present");
    let codec = ThemeData::portable_codec();
    debug_assert_eq!(version, codec.schema().version());
    let theme = codec
        .decode(bytes, version)
        .expect("validated ThemeData provider payload is decodable");
    Provider::new()
        .handle(ProviderHandle::new(theme))
        .child(RetainedElementWidget(
            children
                .pop()
                .expect("validated Provider has exactly one child"),
        ))
        .to_element(factory.ctx)
}

fn materialize_animated_theme(
    factory: &AimerWidgetFactory<'_>,
    document: &WidgetDocumentView<'_>,
    node: WidgetNodeView<'_>,
    mut children: Vec<AnyElement>,
) -> AnyElement {
    let version = unpack_provider_version(
        animated_theme_i64_property(&node, PROPERTY_ANIMATED_THEME_SCHEMA_VERSION)
            .expect("validated AnimatedTheme schema version property is present"),
    )
    .expect("validated AnimatedTheme schema version is representable");
    let blob_index = match provider_property(&node, PROPERTY_ANIMATED_THEME_VALUE) {
        Some(PropertyValue::BlobRef(index)) => index,
        _ => unreachable!("validated AnimatedTheme value property changed during materialization"),
    };
    let bytes = document
        .blob(blob_index)
        .expect("validated AnimatedTheme blob reference is present");
    let codec = ThemeData::portable_codec();
    debug_assert_eq!(version, codec.schema().version());
    let theme = codec
        .decode(bytes, version)
        .expect("validated ThemeData animated-theme payload is decodable");
    let mode = decode_animated_theme_mode(
        animated_theme_i64_property(&node, PROPERTY_ANIMATED_THEME_MODE)
            .expect("validated AnimatedTheme mode property is present"),
    )
    .expect("validated AnimatedTheme mode is supported");
    let duration_millis =
        animated_theme_i64_property(&node, PROPERTY_ANIMATED_THEME_DURATION_MILLIS)
            .expect("validated AnimatedTheme duration property is present") as u64;
    let curve =
        decode_animated_theme_curve(&node).expect("validated AnimatedTheme curve is supported");

    AnimatedTheme::new()
        .data(theme)
        .mode(mode)
        .duration(Duration::from_millis(duration_millis))
        .curve(curve)
        .child(RetainedElementWidget(
            children
                .pop()
                .expect("validated AnimatedTheme has exactly one child"),
        ))
        .to_element(factory.ctx)
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct WidgetIrStageDiagnostics {
    enabled: bool,
}

impl WidgetIrStageDiagnostics {
    #[inline]
    pub(crate) const fn new(enabled: bool) -> Self {
        Self { enabled }
    }

    pub(crate) fn render(
        self,
        image: &[u8],
        limits: ModelLimits,
    ) -> Result<Option<String>, ModelError> {
        if !self.enabled {
            return Ok(None);
        }
        let document = WidgetDocumentView::decode(image, limits)?;
        let assembly = disassemble_widget_document(&document);
        let validator = PortableWidgetSchemaValidator::new_with_additional(
            &BUILTIN_PORTABLE_WIDGET_SCHEMAS,
            linked_portable_native_widget_schemas(),
        )
        .expect("linked portable widget metadata must be valid");
        document.validate_schemas(&validator)?;
        let mut property_count = 0;
        let mut callback_count = 0;
        let mut child_count = 0;
        for node_index in 0..document.node_count() {
            let node = document
                .node(node_index)
                .ok_or(ModelError::NodeIndexOutOfBounds {
                    index: node_index,
                    node_count: document.node_count(),
                })?;
            property_count += node.properties().len();
            callback_count += node.callbacks().len();
            child_count += node.children().len();
        }
        let mut output = String::new();
        writeln!(output, "[Widget IR: semantic graph]").expect("writing to String cannot fail");
        writeln!(
            output,
            "root=node{} generation={} revision={}",
            document.root_node(),
            document.generation_id(),
            document.document_revision(),
        )
        .expect("writing to String cannot fail");
        write_node_diagnostics(&mut output, &document, true);
        writeln!(output, "[Widget IR: textual assembly]").expect("writing to String cannot fail");
        writeln!(output, "{assembly}").expect("writing to String cannot fail");
        writeln!(
            output,
            "[Widget IR: compact binary AWIR] bytes={} nodes={} properties={} callbacks={} children={}",
            image.len(),
            document.node_count(),
            property_count,
            callback_count,
            child_count,
        )
        .expect("writing to String cannot fail");
        for (offset, bytes) in image.chunks(32).enumerate() {
            write!(output, "{:08x}:", offset * 32).expect("writing to String cannot fail");
            for byte in bytes {
                write!(output, " {byte:02x}").expect("writing to String cannot fail");
            }
            output.push('\n');
        }
        writeln!(output, "[Widget IR: decoded AWIR]").expect("writing to String cannot fail");
        write_node_diagnostics(&mut output, &document, false);
        writeln!(
            output,
            "data strings={} blobs={}",
            document.string_count(),
            document.blob_count(),
        )
        .expect("writing to String cannot fail");
        writeln!(output, "[Widget IR: schema validation] accepted")
            .expect("writing to String cannot fail");
        writeln!(output, "[Widget IR: native materialization] ready")
            .expect("writing to String cannot fail");
        Ok(Some(output))
    }
}

fn write_node_diagnostics(
    output: &mut String,
    document: &WidgetDocumentView<'_>,
    resolve_payloads: bool,
) {
    for node_index in 0..document.node_count() {
        let Some(node) = document.node(node_index) else {
            continue;
        };
        writeln!(
            output,
            "node{node_index}: widget={} schema={}.{} key={:?}",
            node.widget_type(),
            node.widget_schema().major(),
            node.widget_schema().minor(),
            node.key(),
        )
        .expect("writing to String cannot fail");
        for property in node.properties() {
            let resolved = match property.value() {
                PropertyValue::StringRef(index) if resolve_payloads => document
                    .string(index)
                    .map(|value| format!(" string={value:?}")),
                PropertyValue::BlobRef(index) if resolve_payloads => document
                    .blob(index)
                    .map(|value| format!(" blob_bytes={}", value.len())),
                _ => None,
            }
            .unwrap_or_default();
            writeln!(
                output,
                "  property={} optional={} value={:?}{}",
                property.property_id(),
                property.is_optional(),
                property.value(),
                resolved,
            )
            .expect("writing to String cannot fail");
        }
        for callback in node.callbacks() {
            writeln!(
                output,
                "  callback={} schema={}.{} id={:02x?}",
                callback.event_kind(),
                callback.event_schema().major(),
                callback.event_schema().minor(),
                callback.callback_id().as_bytes(),
            )
            .expect("writing to String cannot fail");
        }
        for child in node.children() {
            writeln!(output, "  child=node{child}").expect("writing to String cannot fail");
        }
    }
}

impl AimerWidgetFactory<'_> {
    fn build_button(
        &self,
        document: &WidgetDocumentView<'_>,
        node: &WidgetNodeView<'_>,
        children: &mut Vec<AnyElement>,
    ) -> AnyElement {
        let decoration = optional_materialized_property::<BoxDecoration>(
            document,
            node,
            PROPERTY_BUTTON_DECORATION,
        )
        .expect("validated Button decoration is decodable");
        let mut button =
            bind_button_callbacks(Button::new(), node.callbacks(), &self.dispatch_callback);
        if let Some(decoration) = decoration {
            button = button.decoration(decoration);
        }
        button
            .child(RetainedElementWidget(children.pop().unwrap()))
            .to_element(self.ctx)
    }
}

fn bind_button_callbacks(
    mut button: Button,
    callbacks: CallbackBindings<'_>,
    dispatch_callback: &Rc<dyn Fn(StableId128)>,
) -> Button {
    for callback in callbacks {
        let callback_id = callback.callback_id();
        match callback.event_kind() {
            EVENT_BUTTON_PRESS => {
                let dispatch_callback = Rc::clone(dispatch_callback);
                button = button.on_press(move || dispatch_callback(callback_id));
            }
            EVENT_BUTTON_LONG_PRESS => {
                let dispatch_callback = Rc::clone(dispatch_callback);
                button = button.on_long_press(move || dispatch_callback(callback_id));
            }
            EVENT_BUTTON_DOUBLE_PRESS => {
                let dispatch_callback = Rc::clone(dispatch_callback);
                button = button.on_double_press(move || dispatch_callback(callback_id));
            }
            EVENT_BUTTON_RIGHT_PRESS => {
                let dispatch_callback = Rc::clone(dispatch_callback);
                button = button.on_right_press(move || dispatch_callback(callback_id));
            }
            _ => unreachable!("validated Button callback changed during materialization"),
        }
    }
    button
}

fn bind_gesture_detector_callbacks(
    mut detector: GestureDetector,
    callbacks: CallbackBindings<'_>,
    dispatch_callback: &Rc<dyn Fn(StableId128)>,
) -> GestureDetector {
    for callback in callbacks {
        let callback_id = callback.callback_id();
        match callback.event_kind() {
            EVENT_GESTURE_DETECTOR_TAP => {
                let dispatch_callback = Rc::clone(dispatch_callback);
                detector = detector.on_tap(move || dispatch_callback(callback_id));
            }
            EVENT_GESTURE_DETECTOR_DOUBLE_PRESS => {
                let dispatch_callback = Rc::clone(dispatch_callback);
                detector = detector.on_double_press(move || dispatch_callback(callback_id));
            }
            EVENT_GESTURE_DETECTOR_LONG_PRESS => {
                let dispatch_callback = Rc::clone(dispatch_callback);
                detector = detector.on_long_press(move || dispatch_callback(callback_id));
            }
            EVENT_GESTURE_DETECTOR_DRAG_END => {
                let dispatch_callback = Rc::clone(dispatch_callback);
                detector = detector.on_drag_end(move || dispatch_callback(callback_id));
            }
            EVENT_GESTURE_DETECTOR_RIGHT_TAP => {
                let dispatch_callback = Rc::clone(dispatch_callback);
                detector = detector.on_right_tap(move || dispatch_callback(callback_id));
            }
            _ => unreachable!("validated GestureDetector callback changed during materialization"),
        }
    }
    detector
}

fn bind_mouse_region_callbacks(
    mut region: MouseRegion,
    callbacks: CallbackBindings<'_>,
    dispatch_callback: &Rc<dyn Fn(StableId128)>,
) -> MouseRegion {
    for callback in callbacks {
        let callback_id = callback.callback_id();
        match callback.event_kind() {
            EVENT_MOUSE_REGION_HOVER_ENTER => {
                let dispatch_callback = Rc::clone(dispatch_callback);
                region = region.on_hover_enter(move || dispatch_callback(callback_id));
            }
            EVENT_MOUSE_REGION_HOVER_EXIT => {
                let dispatch_callback = Rc::clone(dispatch_callback);
                region = region.on_hover_exit(move || dispatch_callback(callback_id));
            }
            _ => unreachable!("validated MouseRegion callback changed during materialization"),
        }
    }
    region
}

fn bind_text_button_callbacks(
    mut button: TextButton,
    callbacks: CallbackBindings<'_>,
    dispatch_callback: &Rc<dyn Fn(StableId128)>,
) -> TextButton {
    for callback in callbacks {
        let callback_id = callback.callback_id();
        match callback.event_kind() {
            EVENT_TEXT_BUTTON_PRESS => {
                let dispatch_callback = Rc::clone(dispatch_callback);
                button = button.on_press(move || dispatch_callback(callback_id));
            }
            EVENT_TEXT_BUTTON_DOUBLE_PRESS => {
                let dispatch_callback = Rc::clone(dispatch_callback);
                button = button.on_double_press(move || dispatch_callback(callback_id));
            }
            _ => unreachable!("validated TextButton callback changed during materialization"),
        }
    }
    button
}

struct RetainedElementWidget(AnyElement);

impl Widget for RetainedElementWidget {
    #[inline]
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        self.0
    }
}

impl aimer_widget::PortableWidget for RetainedElementWidget {}

fn validate_property_value(
    document: &WidgetDocumentView<'_>,
    node_index: u32,
    widget_type: WidgetSchemaId,
    property: aimer_anteros::WidgetProperty,
) -> Result<(), ModelError> {
    let is_dimension = match widget_type {
        WIDGET_CONTAINER => matches!(
            property.property_id(),
            PROPERTY_CONTAINER_WIDTH | PROPERTY_CONTAINER_HEIGHT
        ),
        WIDGET_SIZED_BOX => matches!(
            property.property_id(),
            PROPERTY_SIZED_BOX_WIDTH | PROPERTY_SIZED_BOX_HEIGHT
        ),
        _ => false,
    };
    if !is_dimension {
        return Ok(());
    }

    let invalid = || ModelError::InvalidWidgetPropertyValue {
        node: node_index,
        widget_type,
        property_id: property.property_id(),
    };
    let dimension = Dimension::from_awir(document, property.property_id(), property.value())
        .map_err(|_| invalid())?;
    if dimension == Dimension::Auto
        || matches!(dimension, Dimension::Px(value) if value.is_finite() && value >= 0.0)
    {
        Ok(())
    } else {
        Err(invalid())
    }
}

fn validate_grid_value(
    document: &WidgetDocumentView<'_>,
    node_index: u32,
    node: WidgetNodeView<'_>,
) -> Result<(), ModelError> {
    let config =
        required_materialized_property::<GridPortableConfig>(document, &node, PROPERTY_GRID_CONFIG)
            .map_err(|_| ModelError::InvalidWidgetPropertyValue {
                node: node_index,
                widget_type: WIDGET_GRID,
                property_id: PROPERTY_GRID_CONFIG,
            })?;
    let count = node.children().len() as u32;
    let expected = config.items.len() as u32;
    if count != expected {
        return Err(ModelError::InvalidWidgetChildCount {
            node: node_index,
            widget_type: WIDGET_GRID,
            count,
            minimum: expected,
            maximum: expected,
        });
    }
    Ok(())
}

fn validate_scrollable_value(
    document: &WidgetDocumentView<'_>,
    node_index: u32,
    node: WidgetNodeView<'_>,
) -> Result<(), ModelError> {
    required_materialized_property::<ScrollPortableConfig>(
        document,
        &node,
        PROPERTY_SCROLLABLE_CONFIG,
    )
    .map(|_| ())
    .map_err(|_| ModelError::InvalidWidgetPropertyValue {
        node: node_index,
        widget_type: WIDGET_SCROLLABLE,
        property_id: PROPERTY_SCROLLABLE_CONFIG,
    })
}

fn validate_provider_value(
    document: &WidgetDocumentView<'_>,
    node_index: u32,
    node: WidgetNodeView<'_>,
) -> Result<(), ModelError> {
    let invalid = |property_id| ModelError::InvalidWidgetPropertyValue {
        node: node_index,
        widget_type: WIDGET_PROVIDER,
        property_id,
    };
    let Some(PropertyValue::I64(type_id)) = provider_property(&node, PROPERTY_PROVIDER_TYPE) else {
        return Err(invalid(PROPERTY_PROVIDER_TYPE));
    };
    let Some(PropertyValue::I64(version)) =
        provider_property(&node, PROPERTY_PROVIDER_SCHEMA_VERSION)
    else {
        return Err(invalid(PROPERTY_PROVIDER_SCHEMA_VERSION));
    };
    let Some(PropertyValue::BlobRef(blob_index)) =
        provider_property(&node, PROPERTY_PROVIDER_VALUE)
    else {
        return Err(invalid(PROPERTY_PROVIDER_VALUE));
    };
    let codec = ThemeData::portable_codec();
    if type_id as u64 != codec.schema().id().value() {
        return Err(invalid(PROPERTY_PROVIDER_TYPE));
    }
    let Some(version) = unpack_provider_version(version) else {
        return Err(invalid(PROPERTY_PROVIDER_SCHEMA_VERSION));
    };
    if version != THEME_DATA_VALUE_VERSION {
        return Err(invalid(PROPERTY_PROVIDER_SCHEMA_VERSION));
    }
    let Some(bytes) = document.blob(blob_index) else {
        return Err(invalid(PROPERTY_PROVIDER_VALUE));
    };
    codec
        .decode(bytes, version)
        .map(|_| ())
        .map_err(|_| invalid(PROPERTY_PROVIDER_VALUE))
}

fn validate_animated_theme_value(
    document: &WidgetDocumentView<'_>,
    node_index: u32,
    node: WidgetNodeView<'_>,
) -> Result<(), ModelError> {
    let invalid = |property_id| ModelError::InvalidWidgetPropertyValue {
        node: node_index,
        widget_type: WIDGET_ANIMATED_THEME,
        property_id,
    };
    let Some(PropertyValue::I64(type_id)) = provider_property(&node, PROPERTY_ANIMATED_THEME_TYPE)
    else {
        return Err(invalid(PROPERTY_ANIMATED_THEME_TYPE));
    };
    let Some(PropertyValue::I64(version)) =
        provider_property(&node, PROPERTY_ANIMATED_THEME_SCHEMA_VERSION)
    else {
        return Err(invalid(PROPERTY_ANIMATED_THEME_SCHEMA_VERSION));
    };
    let Some(PropertyValue::BlobRef(blob_index)) =
        provider_property(&node, PROPERTY_ANIMATED_THEME_VALUE)
    else {
        return Err(invalid(PROPERTY_ANIMATED_THEME_VALUE));
    };
    let Some(PropertyValue::I64(mode)) = provider_property(&node, PROPERTY_ANIMATED_THEME_MODE)
    else {
        return Err(invalid(PROPERTY_ANIMATED_THEME_MODE));
    };
    let Some(PropertyValue::I64(duration)) =
        provider_property(&node, PROPERTY_ANIMATED_THEME_DURATION_MILLIS)
    else {
        return Err(invalid(PROPERTY_ANIMATED_THEME_DURATION_MILLIS));
    };
    if type_id as u64 != ThemeData::portable_codec().schema().id().value() {
        return Err(invalid(PROPERTY_ANIMATED_THEME_TYPE));
    }
    let Some(version) = unpack_provider_version(version) else {
        return Err(invalid(PROPERTY_ANIMATED_THEME_SCHEMA_VERSION));
    };
    if version != THEME_DATA_VALUE_VERSION {
        return Err(invalid(PROPERTY_ANIMATED_THEME_SCHEMA_VERSION));
    }
    let Some(bytes) = document.blob(blob_index) else {
        return Err(invalid(PROPERTY_ANIMATED_THEME_VALUE));
    };
    ThemeData::portable_codec()
        .decode(bytes, version)
        .map_err(|_| invalid(PROPERTY_ANIMATED_THEME_VALUE))?;
    if decode_animated_theme_mode(mode).is_none() {
        return Err(invalid(PROPERTY_ANIMATED_THEME_MODE));
    }
    if duration < 0 {
        return Err(invalid(PROPERTY_ANIMATED_THEME_DURATION_MILLIS));
    }
    if decode_animated_theme_curve(&node).is_none() {
        return Err(invalid(PROPERTY_ANIMATED_THEME_CURVE));
    }
    Ok(())
}

fn provider_property(node: &WidgetNodeView<'_>, property_id: PropertyId) -> Option<PropertyValue> {
    node.properties()
        .find(|property| property.property_id() == property_id)
        .map(|property| property.value())
}

fn provider_i64_property(node: &WidgetNodeView<'_>, property_id: PropertyId) -> Option<i64> {
    match provider_property(node, property_id) {
        Some(PropertyValue::I64(value)) => Some(value),
        _ => None,
    }
}

fn animated_theme_i64_property(node: &WidgetNodeView<'_>, property_id: PropertyId) -> Option<i64> {
    provider_i64_property(node, property_id)
}

fn animated_theme_f64_property(node: &WidgetNodeView<'_>, property_id: PropertyId) -> Option<f64> {
    match provider_property(node, property_id) {
        Some(PropertyValue::F64(value)) => Some(value),
        _ => None,
    }
}

fn decode_animated_theme_mode(value: i64) -> Option<ThemeMode> {
    match value {
        0 => Some(ThemeMode::System),
        1 => Some(ThemeMode::Light),
        2 => Some(ThemeMode::Dark),
        _ => None,
    }
}

fn decode_animated_theme_curve(node: &WidgetNodeView<'_>) -> Option<AnyCurve> {
    let tag = animated_theme_i64_property(node, PROPERTY_ANIMATED_THEME_CURVE)?;
    let controls = [
        animated_theme_f64_property(node, PROPERTY_ANIMATED_THEME_CURVE_X1),
        animated_theme_f64_property(node, PROPERTY_ANIMATED_THEME_CURVE_Y1),
        animated_theme_f64_property(node, PROPERTY_ANIMATED_THEME_CURVE_X2),
        animated_theme_f64_property(node, PROPERTY_ANIMATED_THEME_CURVE_Y2),
    ];
    // All four control points or none; the descriptor decides which tags take
    // them and rejects non-finite ones.
    let points = match controls {
        [None, None, None, None] => None,
        [Some(x1), Some(y1), Some(x2), Some(y2)] => Some([x1, y1, x2, y2]),
        _ => return None,
    };
    AnyCurve::from_descriptor(tag, points)
}

fn unpack_provider_version(value: i64) -> Option<Version> {
    if value < 0 {
        return None;
    }
    let value = value as u64;
    let major = value >> 32;
    let minor = value & u32::MAX as u64;
    (major <= u16::MAX as u64 && minor <= u16::MAX as u64)
        .then(|| Version::new(major as u16, minor as u16))
}

#[cfg(test)]
fn pack_provider_version(version: Version) -> i64 {
    (((version.major() as u64) << 32) | version.minor() as u64) as i64
}

fn build_sized_box(
    document: &WidgetDocumentView<'_>,
    node: &WidgetNodeView<'_>,
    ctx: &BuildContext,
) -> AnyElement {
    let mut sized_box = SizedBox::new();
    if let Some(width) =
        optional_materialized_property::<Dimension>(document, node, PROPERTY_SIZED_BOX_WIDTH)
            .expect("validated SizedBox width is decodable")
    {
        sized_box = sized_box.width(width);
    }
    if let Some(height) =
        optional_materialized_property::<Dimension>(document, node, PROPERTY_SIZED_BOX_HEIGHT)
            .expect("validated SizedBox height is decodable")
    {
        sized_box = sized_box.height(height);
    }
    sized_box.to_element(ctx)
}

const fn widget_debug_name(widget_type: WidgetSchemaId) -> &'static str {
    match widget_type {
        WIDGET_COLUMN => "Column",
        WIDGET_ROW => "Row",
        WIDGET_CONTAINER => "Container",
        WIDGET_SIZED_BOX => "SizedBox",
        WIDGET_TEXT => "RawTextWidget",
        WIDGET_GRID => "Grid",
        WIDGET_SCROLLABLE => "Scrollable",
        WIDGET_BUTTON => "Button",
        WIDGET_TEXT_BUTTON => "TextButton",
        WIDGET_PROVIDER => "Provider",
        WIDGET_ANIMATED_THEME => "AnimatedTheme",
        _ => "Unknown",
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use aimer_anteros::{
        CallbackBinding, ChildCardinality, EventId, PortableWidgetSchemaMetadata, WidgetDocument,
        WidgetNode, WidgetProperty, WidgetSchemaMetadata,
    };
    use aimer_events::element::ElementEvent;
    use aimer_events::pointer::{PointerButton, PointerInfo};
    use aimer_utils::callback::CallbackExecutor;
    use aimer_widget::AnyWidget;

    use super::*;

    const LIMITS: ModelLimits = ModelLimits::new(4_096, 16, 64, 64);
    const SCHEMA_V1: Version = BUILTIN_WIDGET_SCHEMA_VERSION;
    const SCHEMA_V1_1: Version = Version::new(1, 1);
    const PRESS_ID: StableId128 = StableId128::from_bytes([1; 16]);
    const LONG_PRESS_ID: StableId128 = StableId128::from_bytes([2; 16]);
    const DOUBLE_PRESS_ID: StableId128 = StableId128::from_bytes([3; 16]);
    const RIGHT_PRESS_ID: StableId128 = StableId128::from_bytes([4; 16]);

    fn derived_materializer(
        _document: &WidgetDocumentView<'_>,
        _node: WidgetNodeView<'_>,
        _children: Vec<AnyWidget>,
    ) -> Result<AnyWidget, PortableMaterializeError> {
        Err(PortableMaterializeError::InvalidChildCount {
            expected: 0,
            actual: 1,
        })
    }

    fn alternate_derived_materializer(
        _document: &WidgetDocumentView<'_>,
        _node: WidgetNodeView<'_>,
        _children: Vec<AnyWidget>,
    ) -> Result<AnyWidget, PortableMaterializeError> {
        Err(PortableMaterializeError::InvalidChildCount {
            expected: 0,
            actual: 2,
        })
    }

    fn derived_schema() -> PortableWidgetSchemaMetadata<'static> {
        PortableWidgetSchemaMetadata::new(
            WidgetSchemaMetadata::from_canonical_name(
                "aimer.widget:test::DerivedMaterializer",
                SCHEMA_V1,
                SCHEMA_V1,
            ),
            &[],
            &[],
            ChildCardinality::none(),
        )
    }

    #[test]
    fn derived_materializer_registry_ignores_identical_linker_duplicates() {
        let schema = derived_schema();
        let registrations = [
            PortableNativeWidgetRegistration::new(schema, derived_materializer),
            PortableNativeWidgetRegistration::new(schema, derived_materializer),
        ];

        assert!(validate_derived_native_materializer_registrations(&registrations).is_ok());
    }

    #[test]
    fn derived_materializer_registry_rejects_conflicting_linker_duplicates() {
        let schema = derived_schema();
        let registrations = [
            PortableNativeWidgetRegistration::new(schema, derived_materializer),
            PortableNativeWidgetRegistration::new(schema, alternate_derived_materializer),
        ];

        assert!(matches!(
            validate_derived_native_materializer_registrations(&registrations),
            Err(NativeMaterializerRegistryError::OverlappingVersions {
                widget_type,
                first_minimum: SCHEMA_V1,
                first_maximum: SCHEMA_V1,
                second_minimum: SCHEMA_V1,
                second_maximum: SCHEMA_V1,
            }) if widget_type == schema.widget().id()
        ));
    }

    #[test]
    fn native_materializer_registry_rejects_overlapping_duplicate_registrations() {
        let registrations = [
            NativeWidgetMaterializerRegistration::new(
                WIDGET_TEXT,
                SCHEMA_V1,
                SCHEMA_V1,
                materialize_sized_box,
            ),
            NativeWidgetMaterializerRegistration::new(
                WIDGET_TEXT,
                SCHEMA_V1,
                SCHEMA_V1_1,
                materialize_sized_box,
            ),
        ];

        assert!(matches!(
            NativeWidgetMaterializerRegistry::new(&registrations),
            Err(NativeMaterializerRegistryError::OverlappingVersions {
                widget_type: WIDGET_TEXT,
                first_minimum: SCHEMA_V1,
                first_maximum: SCHEMA_V1,
                second_minimum: SCHEMA_V1,
                second_maximum: SCHEMA_V1_1,
            })
        ));
    }

    #[test]
    fn native_materializer_registry_resolves_versions_and_reports_missing_entries() {
        let test_widget = WidgetSchemaId::new(0xfeed);
        let registrations = [NativeWidgetMaterializerRegistration::new(
            test_widget,
            SCHEMA_V1,
            SCHEMA_V1_1,
            materialize_sized_box,
        )];
        let registry = NativeWidgetMaterializerRegistry::new(&registrations).unwrap();

        assert!(registry.resolve(test_widget, SCHEMA_V1).is_some());
        assert!(registry.resolve(test_widget, SCHEMA_V1_1).is_some());
        assert!(registry.resolve(test_widget, Version::new(1, 2)).is_none());
        assert!(
            registry
                .resolve(WidgetSchemaId::new(0xbeef), SCHEMA_V1)
                .is_none()
        );
    }

    #[cfg(feature = "portable-guest")]
    #[test]
    fn portable_builtin_registry_has_a_complete_contract() {
        let coverage = audit_portable_builtin_registry();
        eprintln!(
            "linked schemas: {:?}",
            linked_portable_native_widget_schemas()
                .iter()
                .map(|schema| schema.widget().canonical_name())
                .collect::<Vec<_>>()
        );
        assert!(coverage.is_complete(), "{coverage}");
    }

    #[test]
    fn portable_builtin_audit_includes_shipped_linked_schemas_but_not_test_fixtures() {
        let coverage = audit_portable_builtin_registry();
        assert!(
            coverage
                .entries
                .iter()
                .any(|entry| entry.canonical_name == "aimer.widget:aimer_text::RichText")
        );
        assert!(coverage.entries.iter().all(|entry| {
            !entry
                .canonical_name
                .starts_with("aimer.widget:aimer_quiver.tests.")
        }));
    }

    #[test]
    fn schema_only_builtins_require_an_explicit_host_materializer() {
        let coverage = audit_portable_builtin_registry();
        for entry in coverage
            .entries
            .iter()
            .filter(|entry| entry.schema_only == Some(true))
        {
            assert!(
                entry.host_materializer.is_some(),
                "{} is schema_only and has no permanent host materializer",
                entry.canonical_name,
            );
        }
    }

    #[test]
    fn safe_derived_portable_widgets_are_classified_by_the_audit() {
        let coverage = audit_portable_builtin_registry();
        for (widget_type, canonical_name) in [
            (
                WIDGET_SELECTION_AREA,
                "aimer.widget:aimer_text::SelectionArea",
            ),
            (WIDGET_EXPANDED, "aimer.widget:aimer_flex::flex::Expanded"),
            (
                WIDGET_POSITIONED,
                "aimer.widget:aimer_space::space::Positioned",
            ),
            (
                WIDGET_ASPECT_RATIO,
                "aimer.widget:aimer_container::single_child::AspectRatio",
            ),
            (
                WIDGET_ZERO_SIZED_BOX,
                "aimer.widget:aimer_container::single_child::ZeroSizedBox",
            ),
            (
                WIDGET_OPACITY,
                "aimer.widget:aimer_container::single_child::Opacity",
            ),
            (WIDGET_FOCUS_SCOPE, "aimer.widget:aimer_widget::FocusScope"),
        ] {
            let entry = coverage
                .entries
                .iter()
                .find(|entry| entry.widget_type == widget_type)
                .unwrap_or_else(|| panic!("missing audit entry for {canonical_name}"));
            assert_eq!(entry.schema_only, Some(false), "{canonical_name}");
            assert_eq!(
                entry.guest_lowering,
                Some(GuestLoweringKind::Generated),
                "{canonical_name}",
            );
            assert_eq!(
                entry.host_materializer,
                Some(NativeMaterializerKind::Derived),
                "{canonical_name}",
            );
            assert_eq!(
                entry.focused_round_trip_test,
                Some("safe_derived_portable_widgets_round_trip_through_linked_host_materializers"),
                "{canonical_name}",
            );
        }
    }

    #[cfg(feature = "portable-guest")]
    #[test]
    fn safe_derived_portable_widgets_round_trip_through_linked_host_materializers() {
        use aimer_container::{AspectRatio, Opacity, RatioOption, ZeroSizedBox};
        use aimer_flex::Expanded;
        use aimer_space::Positioned;
        use aimer_text::SelectionArea;
        use aimer_widget::base::Color;
        use aimer_widget::portable::{
            PortableBuildContext, PortableLimits, PortableWidgetLimits, SourceFingerprint,
        };
        use aimer_widget::{FocusScope, PortableWidget};

        let mut guest = PortableBuildContext::new(
            9,
            3,
            PortableWidgetLimits::new(16, 16, 16, 16, 256, 8_192).with_max_blob_bytes(1_024),
            PortableLimits::new(16, 32, 256, 512, 8_192),
        )
        .unwrap();
        let source =
            SourceFingerprint::new(aimer_widget::portable::StableId128::from_bytes([29; 16]));
        let root = SelectionArea::new()
            .selection_color(Color::Rgba(12, 34, 56, 78))
            .child(
                AspectRatio::new()
                    .aspect_ratio(16.0 / 9.0)
                    .ratio_option(RatioOption::Height)
                    .child(
                        Opacity::new().opacity(0.625).child(
                            FocusScope::new().traps(false).child(
                                Positioned::new()
                                    .left(12.0)
                                    .child(Expanded::new().flex(2.5).child(ZeroSizedBox::new())),
                            ),
                        ),
                    ),
            )
            .to_portable_node(&mut guest, source)
            .unwrap();
        let document = guest.finish_document(root).unwrap();
        let limits = document.model_limits();
        let image = document.encode().unwrap();

        let _root = materialize_aimer_widget_tree(&image, limits, &host_context(), |_| {})
            .expect("safe derived widgets must use their linked generated materializers");

        let mut flex_guest = PortableBuildContext::new(
            9,
            3,
            PortableWidgetLimits::new(16, 16, 16, 16, 256, 8_192).with_max_blob_bytes(1_024),
            PortableLimits::new(16, 32, 256, 512, 8_192),
        )
        .unwrap();
        let flex_root = Expanded::new()
            .flex(2.5)
            .child(ZeroSizedBox::new())
            .to_portable_node(
                &mut flex_guest,
                SourceFingerprint::new(aimer_widget::portable::StableId128::from_bytes([30; 16])),
            )
            .unwrap();
        let flex_document = flex_guest.finish_document(flex_root).unwrap();
        let flex_limits = flex_document.model_limits();
        let flex_image = flex_document.encode().unwrap();
        let flex_element =
            materialize_aimer_widget_tree(&flex_image, flex_limits, &host_context(), |_| {})
                .expect("Expanded must materialize through the linked host constructor");
        assert_eq!(
            flex_element.flex(),
            Some(2.5),
            "transparent host wrappers must preserve Expanded's flex factor"
        );
    }

    #[cfg(feature = "portable-guest")]
    #[test]
    fn stack_round_trips_through_its_host_materializer() {
        use aimer_container::ZeroSizedBox;
        use aimer_space::Stack;
        use aimer_widget::PortableWidget;
        use aimer_widget::portable::{
            PortableBuildContext, PortableLimits, PortableWidgetLimits, SourceFingerprint,
        };

        let mut guest = PortableBuildContext::new(
            1,
            1,
            PortableWidgetLimits::new(8, 8, 8, 8, 64, 1_024),
            PortableLimits::new(8, 16, 64, 128, 1_024),
        )
        .unwrap();
        let source =
            SourceFingerprint::new(aimer_widget::portable::StableId128::from_bytes([31; 16]));
        let root = Stack::new()
            .add_child(ZeroSizedBox::new())
            .add_child(ZeroSizedBox::new())
            .to_portable_node(&mut guest, source)
            .unwrap();
        let document = guest.finish_document(root).unwrap();
        let image = document.encode().unwrap();

        materialize_aimer_widget_tree(&image, document.model_limits(), &host_context(), |_| {})
            .expect("Stack must have a permanent host materializer");
    }

    #[cfg(feature = "portable-guest")]
    #[test]
    fn sized_box_round_trips_explicit_dimensions_into_native_layout() {
        use aimer_widget::portable::{
            PortableBuildContext, PortableLimits, PortableWidgetLimits, SourceFingerprint,
        };
        use aimer_widget::{LayoutElement, PortableWidget};

        let mut guest = PortableBuildContext::new(
            1,
            1,
            PortableWidgetLimits::new(8, 8, 8, 8, 64, 1_024).with_max_blob_bytes(128),
            PortableLimits::new(8, 16, 64, 128, 1_024),
        )
        .unwrap();
        let source =
            SourceFingerprint::new(aimer_widget::portable::StableId128::from_bytes([34; 16]));
        let root = SizedBox::new()
            .width(48)
            .height(24)
            .to_portable_node(&mut guest, source)
            .unwrap();
        let document = guest.finish_document(root).unwrap();
        let image = document.encode().unwrap();

        let element =
            materialize_aimer_widget_tree(&image, document.model_limits(), &host_context(), |_| {})
                .expect("SizedBox must materialize through the permanent host");
        let size = element.computed_size(&host_context());

        assert_eq!(size.width, 48.0);
        assert_eq!(size.height, 24.0);
    }

    #[cfg(feature = "portable-guest")]
    #[test]
    fn sized_box_spacers_contribute_to_a_materialized_column() {
        use aimer_widget::portable::{
            PortableBuildContext, PortableLimits, PortableWidgetLimits, SourceFingerprint,
        };
        use aimer_widget::{LayoutElement, PortableWidget};

        let mut guest = PortableBuildContext::new(
            1,
            1,
            PortableWidgetLimits::new(8, 8, 8, 8, 64, 1_024).with_max_blob_bytes(128),
            PortableLimits::new(8, 16, 64, 128, 1_024),
        )
        .unwrap();
        let source =
            SourceFingerprint::new(aimer_widget::portable::StableId128::from_bytes([35; 16]));
        let root = Column::new()
            .children([SizedBox::new().height(24), SizedBox::new().height(40)])
            .to_portable_node(&mut guest, source)
            .unwrap();
        let document = guest.finish_document(root).unwrap();
        let image = document.encode().unwrap();

        let element =
            materialize_aimer_widget_tree(&image, document.model_limits(), &host_context(), |_| {})
                .expect("Column spacers must materialize through the permanent host");
        let size = element.computed_size(&host_context());

        assert_eq!(size.height, 64.0);
    }

    #[cfg(feature = "portable-guest")]
    #[test]
    fn sized_box_spacers_survive_scrollable_content_measurement() {
        use aimer_attribute::{BoxConstraint, ResolvedSize};
        use aimer_container::{Container, ZeroSizedBox};
        use aimer_scroll::{ScrollAxis, Scrollable};
        use aimer_style::LayoutSpacing;
        use aimer_text::Text;
        use aimer_widget::portable::{
            PortableBuildContext, PortableLimits, PortableWidgetLimits, SourceFingerprint,
        };
        use aimer_widget::{LayoutElement, PortableWidget};

        let section = || {
            Scrollable::new().axis(ScrollAxis::Vertical).child(
                Container::new()
                    .padding(LayoutSpacing::new().top(20).bottom(20))
                    .child(
                        Column::new().children([
                            Container::new()
                                .height(100)
                                .child(Text::new("Consistence Looking"))
                                .boxed(),
                            SizedBox::new().height(24).boxed(),
                            Container::new()
                                .height(450)
                                .child(ZeroSizedBox::new())
                                .boxed(),
                            SizedBox::new().height(40).boxed(),
                            SizedBox::new().height(44).boxed(),
                            SizedBox::new().height(40).boxed(),
                        ]),
                    ),
            )
        };

        let mut context = host_context();
        context.box_constraint = BoxConstraint::new().max_width(800.0).max_height(600.0);
        context.parent_size = ResolvedSize {
            width: 800.0,
            height: 600.0,
        };
        let native = section().to_element(&context);

        let mut guest = PortableBuildContext::new(
            1,
            1,
            PortableWidgetLimits::new(16, 16, 16, 16, 64, 4_096).with_max_blob_bytes(128),
            PortableLimits::new(16, 32, 128, 256, 4_096),
        )
        .unwrap();
        let source =
            SourceFingerprint::new(aimer_widget::portable::StableId128::from_bytes([37; 16]));
        let root = section().to_portable_node(&mut guest, source).unwrap();
        let document = guest.finish_document(root).unwrap();
        let image = document.encode().unwrap();
        let hot = materialize_aimer_widget_tree(&image, document.model_limits(), &context, |_| {})
            .expect("scrollable section must materialize through the permanent host");

        assert_eq!(native.content_size(&context).height, 738.0);
        assert_eq!(hot.content_size(&context), native.content_size(&context));
    }

    #[cfg(feature = "portable-guest")]
    #[test]
    fn grid_round_trips_layout_configuration_and_children() {
        use aimer_container::ZeroSizedBox;
        use aimer_grid::{Grid, GridItem, GridTrack};
        use aimer_widget::PortableWidget;
        use aimer_widget::portable::{
            PortableBuildContext, PortableLimits, PortableWidgetLimits, SourceFingerprint,
        };

        let mut guest = PortableBuildContext::new(
            1,
            1,
            PortableWidgetLimits::new(8, 8, 8, 8, 128, 4_096).with_max_blob_bytes(256),
            PortableLimits::new(8, 16, 64, 128, 4_096),
        )
        .unwrap();
        let source =
            SourceFingerprint::new(aimer_widget::portable::StableId128::from_bytes([32; 16]));
        let root = Grid::new()
            .columns([GridTrack::Px(120.0)])
            .rows([GridTrack::Px(40.0)])
            .gap(8.0)
            .children([GridItem::new(ZeroSizedBox::new()).at(0, 0)])
            .to_portable_node(&mut guest, source)
            .unwrap();
        let document = guest.finish_document(root).unwrap();
        let image = document.encode().unwrap();

        materialize_aimer_widget_tree(&image, document.model_limits(), &host_context(), |_| {})
            .expect("Grid must preserve enough layout metadata for host materialization");
    }

    #[cfg(feature = "portable-guest")]
    #[test]
    fn scrollable_round_trips_axis_and_scrollbars_through_host_materializer() {
        use aimer_container::ZeroSizedBox;
        use aimer_scroll::{ScrollAxis, Scrollable};
        use aimer_widget::PortableWidget;
        use aimer_widget::portable::{
            PortableBuildContext, PortableLimits, PortableWidgetLimits, SourceFingerprint,
        };

        let mut guest = PortableBuildContext::new(
            1,
            1,
            PortableWidgetLimits::new(8, 8, 8, 8, 128, 4_096).with_max_blob_bytes(256),
            PortableLimits::new(8, 16, 64, 128, 4_096),
        )
        .unwrap();
        let source =
            SourceFingerprint::new(aimer_widget::portable::StableId128::from_bytes([33; 16]));
        let root = Scrollable::new()
            .axis(ScrollAxis::Horizontal)
            .vertical_scroll_bar(None)
            .horizontal_scroll_bar(None)
            .child(ZeroSizedBox::new())
            .to_portable_node(&mut guest, source)
            .unwrap();
        let document = guest.finish_document(root).unwrap();
        let image = document.encode().unwrap();

        materialize_aimer_widget_tree(&image, document.model_limits(), &host_context(), |_| {})
            .expect("Scrollable must have a permanent host materializer");
    }

    #[cfg(feature = "portable-guest")]
    #[test]
    fn scroll_bar_round_trips_through_host_materializer() {
        use aimer_scroll::ScrollBar;
        use aimer_widget::PortableWidget;
        use aimer_widget::portable::{
            PortableBuildContext, PortableLimits, PortableWidgetLimits, SourceFingerprint,
        };

        let mut guest = PortableBuildContext::new(
            1,
            1,
            PortableWidgetLimits::new(8, 8, 8, 8, 64, 1_024),
            PortableLimits::new(8, 16, 64, 128, 1_024),
        )
        .unwrap();
        let source =
            SourceFingerprint::new(aimer_widget::portable::StableId128::from_bytes([35; 16]));
        let root = ScrollBar::default()
            .to_portable_node(&mut guest, source)
            .unwrap();
        let document = guest.finish_document(root).unwrap();
        let image = document.encode().unwrap();

        materialize_aimer_widget_tree(&image, document.model_limits(), &host_context(), |_| {})
            .expect("ScrollBar must have a permanent host materializer");
    }

    #[test]
    fn portable_builtin_showcase_round_trips_through_host() {
        let codec = ThemeData::portable_codec();
        let snapshot = codec.encode(&ThemeData::dark()).unwrap();
        let root_children = [1, 2, 3, 4, 5, 6, 7, 8];
        let container_child = [9];
        let button_child = [10];
        let provider_child = [11];
        let animated_theme_child = [12];
        let text_properties = [WidgetProperty::new(
            PROPERTY_TEXT_CONTENT,
            PropertyValue::StringRef(0),
        )];
        let provider_properties = [
            WidgetProperty::new(
                PROPERTY_PROVIDER_TYPE,
                PropertyValue::I64(codec.schema().id().value() as i64),
            ),
            WidgetProperty::new(
                PROPERTY_PROVIDER_SCHEMA_VERSION,
                PropertyValue::I64(pack_provider_version(codec.schema().version())),
            ),
            WidgetProperty::new(PROPERTY_PROVIDER_VALUE, PropertyValue::BlobRef(0)),
        ];
        let animated_theme_properties = [
            WidgetProperty::new(
                PROPERTY_ANIMATED_THEME_TYPE,
                PropertyValue::I64(codec.schema().id().value() as i64),
            ),
            WidgetProperty::new(
                PROPERTY_ANIMATED_THEME_SCHEMA_VERSION,
                PropertyValue::I64(pack_provider_version(codec.schema().version())),
            ),
            WidgetProperty::new(PROPERTY_ANIMATED_THEME_VALUE, PropertyValue::BlobRef(1)),
            WidgetProperty::new(PROPERTY_ANIMATED_THEME_MODE, PropertyValue::I64(2)),
            WidgetProperty::new(
                PROPERTY_ANIMATED_THEME_DURATION_MILLIS,
                PropertyValue::I64(1),
            ),
            WidgetProperty::new(PROPERTY_ANIMATED_THEME_CURVE, PropertyValue::I64(0)),
        ];
        let error_properties = [WidgetProperty::new(
            PROPERTY_ERROR_MESSAGE,
            PropertyValue::StringRef(1),
        )];
        let nodes = [
            WidgetNode::new(WIDGET_COLUMN, SCHEMA_V1).children(&root_children),
            WidgetNode::new(WIDGET_ROW, SCHEMA_V1),
            WidgetNode::new(WIDGET_CONTAINER, SCHEMA_V1).children(&container_child),
            WidgetNode::new(WIDGET_SIZED_BOX, SCHEMA_V1),
            WidgetNode::new(WIDGET_TEXT, SCHEMA_V1).properties(&text_properties),
            WidgetNode::new(WIDGET_BUTTON, SCHEMA_V1).children(&button_child),
            WidgetNode::new(WIDGET_PROVIDER, SCHEMA_V1)
                .properties(&provider_properties)
                .children(&provider_child),
            WidgetNode::new(WIDGET_ANIMATED_THEME, SCHEMA_V1)
                .properties(&animated_theme_properties)
                .children(&animated_theme_child),
            WidgetNode::new(WIDGET_ERROR, SCHEMA_V1).properties(&error_properties),
            WidgetNode::new(WIDGET_TEXT, SCHEMA_V1).properties(&text_properties),
            WidgetNode::new(WIDGET_TEXT, SCHEMA_V1).properties(&text_properties),
            WidgetNode::new(WIDGET_TEXT, SCHEMA_V1).properties(&text_properties),
            WidgetNode::new(WIDGET_TEXT, SCHEMA_V1).properties(&text_properties),
        ];
        let image = WidgetDocument::new(
            1,
            1,
            0,
            &nodes,
            &["portable showcase", "portable error"],
            &[&snapshot, &snapshot],
        )
        .encode(LIMITS)
        .unwrap();

        let _root = materialize_aimer_widget_tree(&image, LIMITS, &host_context(), |_| {})
            .expect("every built-in showcase schema must materialize through the permanent host");
    }

    #[test]
    fn widget_ir_stage_diagnostics_are_quiet_by_default_and_deterministic_when_enabled() {
        let image = encode_single(
            WidgetNode::new(WIDGET_TEXT, SCHEMA_V1).properties(&[WidgetProperty::new(
                PROPERTY_TEXT_CONTENT,
                PropertyValue::StringRef(0),
            )]),
            &["Hello"],
        );

        let quiet = WidgetIrStageDiagnostics::new(false);
        assert_eq!(quiet.render(&image, LIMITS).unwrap(), None);

        let verbose = WidgetIrStageDiagnostics::new(true);
        let first = verbose.render(&image, LIMITS).unwrap().unwrap();
        let second = verbose.render(&image, LIMITS).unwrap().unwrap();
        assert_eq!(first, second);
        let semantic = stage_body(&first, "semantic graph", "textual assembly");
        let decoded = stage_body(&first, "decoded AWIR", "schema validation");
        assert_ne!(semantic, decoded);
        assert!(first.contains("00000000: 41 57 49 52"));
        for stage in [
            "semantic graph",
            "textual assembly",
            "compact binary AWIR",
            "decoded AWIR",
            "schema validation",
            "native materialization",
        ] {
            assert!(first.contains(stage), "missing {stage} in {first}");
        }
    }

    fn stage_body<'a>(report: &'a str, stage: &str, next_stage: &str) -> &'a str {
        report
            .split_once(&format!("[Widget IR: {stage}]"))
            .unwrap()
            .1
            .split_once(&format!("[Widget IR: {next_stage}]"))
            .unwrap()
            .0
    }

    #[test]
    fn native_dimensions_reject_negative_sized_box_dimensions() {
        let property = WidgetProperty::new(PROPERTY_SIZED_BOX_WIDTH, PropertyValue::BlobRef(0));
        let properties = [property];
        let nodes = [WidgetNode::new(WIDGET_SIZED_BOX, SCHEMA_V1).properties(&properties)];
        let negative_px = [1, 1, 0, 0, 128, 191];
        let image = WidgetDocument::new(1, 1, 0, &nodes, &[], &[&negative_px])
            .encode(LIMITS)
            .unwrap();
        let document = WidgetDocumentView::decode(&image, LIMITS).unwrap();
        let property = document.node(0).unwrap().properties().next().unwrap();

        assert_eq!(
            validate_property_value(&document, 0, WIDGET_SIZED_BOX, property),
            Err(ModelError::InvalidWidgetPropertyValue {
                node: 0,
                widget_type: WIDGET_SIZED_BOX,
                property_id: PROPERTY_SIZED_BOX_WIDTH,
            })
        );
    }

    #[test]
    fn provider_validation_accepts_the_built_in_theme_data_codec() {
        let codec = ThemeData::portable_codec();
        let snapshot = codec.encode(&ThemeData::dark()).unwrap();
        let properties = [
            WidgetProperty::new(
                PROPERTY_PROVIDER_TYPE,
                PropertyValue::I64(codec.schema().id().value() as i64),
            ),
            WidgetProperty::new(
                PROPERTY_PROVIDER_SCHEMA_VERSION,
                PropertyValue::I64(pack_provider_version(codec.schema().version())),
            ),
            WidgetProperty::new(PROPERTY_PROVIDER_VALUE, PropertyValue::BlobRef(0)),
        ];
        let children = [1];
        let nodes = [
            WidgetNode::new(WIDGET_PROVIDER, SCHEMA_V1)
                .properties(&properties)
                .children(&children),
            WidgetNode::new(WIDGET_SIZED_BOX, SCHEMA_V1),
        ];
        let image = WidgetDocument::new(1, 1, 0, &nodes, &[], &[&snapshot])
            .encode(LIMITS)
            .unwrap();
        let document = WidgetDocumentView::decode(&image, LIMITS).unwrap();

        assert_eq!(
            validate_provider_value(&document, 0, document.node(0).unwrap()),
            Ok(())
        );
    }

    #[test]
    fn provider_validation_rejects_unknown_identity_and_malformed_theme_payload() {
        let cases = [
            (PropertyValue::I64(0), vec![0; 24], PROPERTY_PROVIDER_TYPE),
            (
                PropertyValue::I64(ThemeData::portable_codec().schema().id().value() as i64),
                vec![0; 23],
                PROPERTY_PROVIDER_VALUE,
            ),
        ];
        for (type_value, snapshot, invalid_property) in cases {
            let properties = [
                WidgetProperty::new(PROPERTY_PROVIDER_TYPE, type_value),
                WidgetProperty::new(
                    PROPERTY_PROVIDER_SCHEMA_VERSION,
                    PropertyValue::I64(pack_provider_version(THEME_DATA_VALUE_VERSION)),
                ),
                WidgetProperty::new(PROPERTY_PROVIDER_VALUE, PropertyValue::BlobRef(0)),
            ];
            let children = [1];
            let nodes = [
                WidgetNode::new(WIDGET_PROVIDER, SCHEMA_V1)
                    .properties(&properties)
                    .children(&children),
                WidgetNode::new(WIDGET_SIZED_BOX, SCHEMA_V1),
            ];
            let image = WidgetDocument::new(1, 1, 0, &nodes, &[], &[&snapshot])
                .encode(LIMITS)
                .unwrap();
            let document = WidgetDocumentView::decode(&image, LIMITS).unwrap();

            assert_eq!(
                validate_provider_value(&document, 0, document.node(0).unwrap()),
                Err(ModelError::InvalidWidgetPropertyValue {
                    node: 0,
                    widget_type: WIDGET_PROVIDER,
                    property_id: invalid_property,
                })
            );
        }
    }

    #[test]
    fn animated_theme_validation_accepts_theme_data_and_cubic_animation() {
        let codec = ThemeData::portable_codec();
        let snapshot = codec.encode(&ThemeData::dark()).unwrap();
        let properties = [
            WidgetProperty::new(
                PROPERTY_ANIMATED_THEME_TYPE,
                PropertyValue::I64(codec.schema().id().value() as i64),
            ),
            WidgetProperty::new(
                PROPERTY_ANIMATED_THEME_SCHEMA_VERSION,
                PropertyValue::I64(pack_provider_version(codec.schema().version())),
            ),
            WidgetProperty::new(PROPERTY_ANIMATED_THEME_VALUE, PropertyValue::BlobRef(0)),
            WidgetProperty::new(PROPERTY_ANIMATED_THEME_MODE, PropertyValue::I64(2)),
            WidgetProperty::new(
                PROPERTY_ANIMATED_THEME_DURATION_MILLIS,
                PropertyValue::I64(321),
            ),
            WidgetProperty::new(PROPERTY_ANIMATED_THEME_CURVE, PropertyValue::I64(4)),
            WidgetProperty::new(PROPERTY_ANIMATED_THEME_CURVE_X1, PropertyValue::F64(0.1))
                .optional(),
            WidgetProperty::new(PROPERTY_ANIMATED_THEME_CURVE_Y1, PropertyValue::F64(0.2))
                .optional(),
            WidgetProperty::new(PROPERTY_ANIMATED_THEME_CURVE_X2, PropertyValue::F64(0.8))
                .optional(),
            WidgetProperty::new(PROPERTY_ANIMATED_THEME_CURVE_Y2, PropertyValue::F64(0.9))
                .optional(),
        ];

        with_animated_theme_node(&properties, &snapshot, |document, node| {
            let curve = decode_animated_theme_curve(&node);
            assert_eq!(validate_animated_theme_value(document, 0, node), Ok(()));
            assert_eq!(decode_animated_theme_mode(2), Some(ThemeMode::Dark));
            assert_eq!(
                curve,
                Some(AnyCurve::new(aimer_animation::curves::cubic_bezier(
                    0.1, 0.2, 0.8, 0.9
                )))
            );
        });
    }

    #[test]
    fn animated_theme_validation_rejects_invalid_mode_and_incomplete_curve() {
        let codec = ThemeData::portable_codec();
        let snapshot = codec.encode(&ThemeData::dark()).unwrap();
        let base = [
            WidgetProperty::new(
                PROPERTY_ANIMATED_THEME_TYPE,
                PropertyValue::I64(codec.schema().id().value() as i64),
            ),
            WidgetProperty::new(
                PROPERTY_ANIMATED_THEME_SCHEMA_VERSION,
                PropertyValue::I64(pack_provider_version(codec.schema().version())),
            ),
            WidgetProperty::new(PROPERTY_ANIMATED_THEME_VALUE, PropertyValue::BlobRef(0)),
            WidgetProperty::new(PROPERTY_ANIMATED_THEME_MODE, PropertyValue::I64(7)),
            WidgetProperty::new(
                PROPERTY_ANIMATED_THEME_DURATION_MILLIS,
                PropertyValue::I64(321),
            ),
            WidgetProperty::new(PROPERTY_ANIMATED_THEME_CURVE, PropertyValue::I64(4)),
        ];

        with_animated_theme_node(&base, &snapshot, |document, node| {
            assert_eq!(
                validate_animated_theme_value(document, 0, node),
                Err(ModelError::InvalidWidgetPropertyValue {
                    node: 0,
                    widget_type: WIDGET_ANIMATED_THEME,
                    property_id: PROPERTY_ANIMATED_THEME_MODE,
                })
            );
        });

        let valid_mode = [
            base[0],
            base[1],
            base[2],
            WidgetProperty::new(PROPERTY_ANIMATED_THEME_MODE, PropertyValue::I64(1)),
            base[4],
            base[5],
        ];
        with_animated_theme_node(&valid_mode, &snapshot, |document, node| {
            assert_eq!(
                validate_animated_theme_value(document, 0, node),
                Err(ModelError::InvalidWidgetPropertyValue {
                    node: 0,
                    widget_type: WIDGET_ANIMATED_THEME,
                    property_id: PROPERTY_ANIMATED_THEME_CURVE,
                })
            );
        });
    }

    #[test]
    fn animated_theme_materializer_accepts_the_validated_native_scope() {
        let codec = ThemeData::portable_codec();
        let snapshot = codec.encode(&ThemeData::dark()).unwrap();
        let properties = [
            WidgetProperty::new(
                PROPERTY_ANIMATED_THEME_TYPE,
                PropertyValue::I64(codec.schema().id().value() as i64),
            ),
            WidgetProperty::new(
                PROPERTY_ANIMATED_THEME_SCHEMA_VERSION,
                PropertyValue::I64(pack_provider_version(codec.schema().version())),
            ),
            WidgetProperty::new(PROPERTY_ANIMATED_THEME_VALUE, PropertyValue::BlobRef(0)),
            WidgetProperty::new(PROPERTY_ANIMATED_THEME_MODE, PropertyValue::I64(2)),
            WidgetProperty::new(
                PROPERTY_ANIMATED_THEME_DURATION_MILLIS,
                PropertyValue::I64(321),
            ),
            WidgetProperty::new(PROPERTY_ANIMATED_THEME_CURVE, PropertyValue::I64(3)),
        ];
        let children = [1];
        let nodes = [
            WidgetNode::new(WIDGET_ANIMATED_THEME, SCHEMA_V1)
                .properties(&properties)
                .children(&children),
            WidgetNode::new(WIDGET_SIZED_BOX, SCHEMA_V1),
        ];
        let image = WidgetDocument::new(1, 1, 0, &nodes, &[], &[&snapshot])
            .encode(LIMITS)
            .unwrap();

        let _element = materialize_aimer_widget_tree(&image, LIMITS, &host_context(), |_| {})
            .expect("a validated AnimatedTheme must have a native materializer");
    }

    fn encode_single(node: WidgetNode<'_>, strings: &[&str]) -> Vec<u8> {
        WidgetDocument::new(1, 0, 0, std::slice::from_ref(&node), strings, &[])
            .encode(LIMITS)
            .unwrap()
    }

    fn with_animated_theme_node<R>(
        properties: &[WidgetProperty],
        snapshot: &[u8],
        inspect: impl FnOnce(&WidgetDocumentView<'_>, WidgetNodeView<'_>) -> R,
    ) -> R {
        let children = [1];
        let nodes = [
            WidgetNode::new(WIDGET_ANIMATED_THEME, SCHEMA_V1)
                .properties(properties)
                .children(&children),
            WidgetNode::new(WIDGET_SIZED_BOX, SCHEMA_V1),
        ];
        let image = WidgetDocument::new(1, 1, 0, &nodes, &[], &[snapshot])
            .encode(LIMITS)
            .unwrap();
        let document = WidgetDocumentView::decode(&image, LIMITS).unwrap();
        inspect(&document, document.node(0).unwrap())
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn dummy_async_handle() -> tokio::runtime::Handle {
        use std::sync::OnceLock;

        static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
        let runtime = RUNTIME.get_or_init(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
        });
        let _guard = runtime.enter();
        tokio::runtime::Handle::current()
    }

    fn host_context() -> BuildContext<'static> {
        let inner = Box::leak(Box::new(aimer_canvas::InnerCanvas::new()));
        BuildContext::new(
            aimer_canvas::FrameCanvas::new(inner),
            Default::default(),
            1.0,
            Default::default(),
            Default::default(),
            aimer_widget::base::WindowHandle::headless(Default::default(), 1.0),
            #[cfg(not(target_arch = "wasm32"))]
            dummy_async_handle(),
        )
    }

    #[test]
    fn button_materialization_routes_every_event_to_its_own_callback_id() {
        let callbacks = [
            binding(EVENT_BUTTON_RIGHT_PRESS, RIGHT_PRESS_ID),
            binding(EVENT_BUTTON_PRESS, PRESS_ID),
            binding(EVENT_BUTTON_DOUBLE_PRESS, DOUBLE_PRESS_ID),
            binding(EVENT_BUTTON_LONG_PRESS, LONG_PRESS_ID),
        ];

        with_button_node(&callbacks, |_document, node| {
            let dispatched = Rc::new(RefCell::new(Vec::new()));
            let captured = Rc::clone(&dispatched);
            let dispatch: Rc<dyn Fn(StableId128)> =
                Rc::new(move |callback_id| captured.borrow_mut().push(callback_id));
            let button = bind_button_callbacks(Button::new(), node.callbacks(), &dispatch);

            button.on_double_press.execute(());
            button.on_press.execute(());
            button.on_right_press.execute(());
            button.on_long_press.execute(());

            assert_eq!(
                *dispatched.borrow(),
                [DOUBLE_PRESS_ID, PRESS_ID, RIGHT_PRESS_ID, LONG_PRESS_ID]
            );
        });
    }

    #[test]
    fn button_press_binding_remains_compatible() {
        with_button_node(
            &[binding(EVENT_BUTTON_PRESS, PRESS_ID)],
            |_document, node| {
                let dispatched = Rc::new(RefCell::new(Vec::new()));
                let captured = Rc::clone(&dispatched);
                let dispatch: Rc<dyn Fn(StableId128)> =
                    Rc::new(move |callback_id| captured.borrow_mut().push(callback_id));
                let button = bind_button_callbacks(Button::new(), node.callbacks(), &dispatch);

                assert_eq!(button.on_press.execute(()), Some(()));
                assert_eq!(button.on_long_press.execute(()), None);
                assert_eq!(button.on_double_press.execute(()), None);
                assert_eq!(button.on_right_press.execute(()), None);
                assert_eq!(*dispatched.borrow(), [PRESS_ID]);
            },
        );
    }

    #[test]
    fn button_without_callbacks_remains_valid_and_unbound() {
        with_button_node(&[], |document, node| {
            let dispatch: Rc<dyn Fn(StableId128)> = Rc::new(|_| panic!("unexpected dispatch"));
            let button = bind_button_callbacks(Button::new(), node.callbacks(), &dispatch);
            assert_eq!(validate_button_callbacks(document, node), Ok(()));

            assert_eq!(button.on_press.execute(()), None);
            assert_eq!(button.on_long_press.execute(()), None);
            assert_eq!(button.on_double_press.execute(()), None);
            assert_eq!(button.on_right_press.execute(()), None);
        });
    }

    #[test]
    fn text_button_materialization_routes_properties_and_callbacks() {
        let properties = [
            WidgetProperty::new(PROPERTY_TEXT_BUTTON_DISABLED, PropertyValue::Bool(false)),
            WidgetProperty::new(PROPERTY_TEXT_BUTTON_LABEL, PropertyValue::StringRef(0)),
            WidgetProperty::new(PROPERTY_TEXT_BUTTON_COLOR, PropertyValue::Rgba(0x10203040))
                .optional(),
            WidgetProperty::new(
                PROPERTY_TEXT_BUTTON_HOVER_COLOR,
                PropertyValue::Rgba(0x50607080),
            )
            .optional(),
            WidgetProperty::new(
                PROPERTY_TEXT_BUTTON_DISABLED_COLOR,
                PropertyValue::Rgba(0x90a0b0c0),
            )
            .optional(),
        ];
        let callbacks = [
            CallbackBinding::new_async(EVENT_TEXT_BUTTON_PRESS, SCHEMA_V1, SCHEMA_V1, PRESS_ID),
            CallbackBinding::new_async(
                EVENT_TEXT_BUTTON_DOUBLE_PRESS,
                SCHEMA_V1,
                SCHEMA_V1,
                DOUBLE_PRESS_ID,
            ),
        ];
        let nodes = [WidgetNode::new(WIDGET_TEXT_BUTTON, SCHEMA_V1)
            .properties(&properties)
            .callbacks(&callbacks)];
        let image = WidgetDocument::new(1, 0, 0, &nodes, &["Open"], &[])
            .encode(LIMITS)
            .unwrap();
        let dispatched = Rc::new(RefCell::new(Vec::new()));
        let captured = Rc::clone(&dispatched);
        let element = materialize_aimer_widget_tree(&image, LIMITS, &host_context(), move |id| {
            captured.borrow_mut().push(id)
        })
        .expect("validated TextButton data must materialize");
        let ctx = host_context();
        element.layout(&ctx);
        let pointer = |event: PointerButton| {
            ElementEvent::PointerDown(PointerInfo::mouse(
                aimer_attribute::Vec2d { x: 1.0, y: 1.0 },
                event,
            ))
        };
        let up = || {
            ElementEvent::PointerUp(PointerInfo::mouse(
                aimer_attribute::Vec2d { x: 1.0, y: 1.0 },
                PointerButton::Primary,
            ))
        };

        let _ = element.on_event(&pointer(PointerButton::Primary));
        let _ = element.on_event(&up());
        let _ = element.on_event(&pointer(PointerButton::Primary));
        let _ = element.on_event(&up());

        assert_eq!(*dispatched.borrow(), [PRESS_ID, PRESS_ID, DOUBLE_PRESS_ID]);
    }

    #[cfg(feature = "portable-guest")]
    #[test]
    fn text_button_materialization_preserves_explicit_style_layout() {
        use aimer_style::TextStyle;
        use aimer_widget::portable::{
            PortableBuildContext, PortableLimits, PortableWidgetLimits, SourceFingerprint,
        };
        use aimer_widget::{LayoutElement, PortableWidget};

        let style = TextStyle::new().font_size(31);
        let mut guest = PortableBuildContext::new(
            1,
            1,
            PortableWidgetLimits::new(8, 8, 8, 8, 64, 4_096)
                .with_max_callbacks(4)
                .with_max_blob_bytes(128),
            PortableLimits::new(8, 16, 64, 128, 4_096),
        )
        .unwrap();
        let source =
            SourceFingerprint::new(aimer_widget::portable::StableId128::from_bytes([36; 16]));
        let root = TextButton::new("Open")
            .style(style)
            .to_portable_node(&mut guest, source)
            .unwrap();
        let document = guest.finish_document(root).unwrap();
        let image = document.encode().unwrap();

        let native = TextButton::new("Open")
            .style(style)
            .to_element(&host_context());
        let hot =
            materialize_aimer_widget_tree(&image, document.model_limits(), &host_context(), |_| {})
                .expect("TextButton must materialize through the permanent host");

        assert_eq!(
            hot.computed_size(&host_context()),
            native.computed_size(&host_context())
        );
    }

    #[test]
    fn button_validation_accepts_four_distinct_event_bindings() {
        let callbacks = [
            binding(EVENT_BUTTON_LONG_PRESS, LONG_PRESS_ID),
            binding(EVENT_BUTTON_RIGHT_PRESS, RIGHT_PRESS_ID),
            binding(EVENT_BUTTON_PRESS, PRESS_ID),
            binding(EVENT_BUTTON_DOUBLE_PRESS, DOUBLE_PRESS_ID),
        ];

        with_button_node(&callbacks, |document, node| {
            assert_eq!(validate_button_callbacks(document, node), Ok(()));
        });
    }

    #[test]
    fn button_validation_rejects_duplicate_event_kinds() {
        let callbacks = [
            binding(EVENT_BUTTON_PRESS, PRESS_ID),
            binding(EVENT_BUTTON_PRESS, LONG_PRESS_ID),
        ];

        with_button_node(&callbacks, |document, node| {
            assert_eq!(
                validate_button_callbacks(document, node),
                Err(ModelError::InvalidWidgetCallbackCount {
                    node: 0,
                    widget_type: WIDGET_BUTTON,
                    count: 2,
                    maximum: 1,
                })
            );
        });
    }

    #[test]
    fn button_validation_rejects_duplicate_callback_ids() {
        let callbacks = [
            binding(EVENT_BUTTON_PRESS, PRESS_ID),
            binding(EVENT_BUTTON_LONG_PRESS, PRESS_ID),
        ];

        with_button_node(&callbacks, |document, node| {
            assert_eq!(
                validate_button_callbacks(document, node),
                Err(ModelError::DuplicateWidgetCallback {
                    node: 0,
                    widget_type: WIDGET_BUTTON,
                    callback_id: PRESS_ID,
                })
            );
        });
    }

    #[test]
    fn button_validation_rejects_unsupported_event_kind_and_schema() {
        let cases = [
            CallbackBinding::new(EventId::new(99), SCHEMA_V1, PRESS_ID),
            CallbackBinding::new(EVENT_BUTTON_PRESS, Version::new(2, 0), PRESS_ID),
        ];

        for callback in cases {
            with_button_node(&[callback], |document, node| {
                assert_eq!(
                    validate_button_callbacks(document, node),
                    Err(ModelError::UnsupportedWidgetCallback {
                        node: 0,
                        widget_type: WIDGET_BUTTON,
                        event_kind: callback.event_kind(),
                    })
                );
            });
        }
    }

    #[test]
    fn button_validation_rejects_more_than_four_bindings() {
        let callbacks = [
            binding(EVENT_BUTTON_PRESS, PRESS_ID),
            binding(EVENT_BUTTON_LONG_PRESS, LONG_PRESS_ID),
            binding(EVENT_BUTTON_DOUBLE_PRESS, DOUBLE_PRESS_ID),
            binding(EVENT_BUTTON_RIGHT_PRESS, RIGHT_PRESS_ID),
            binding(EVENT_BUTTON_PRESS, StableId128::from_bytes([5; 16])),
        ];

        with_button_node(&callbacks, |document, node| {
            assert_eq!(
                validate_button_callbacks(document, node),
                Err(ModelError::InvalidWidgetCallbackCount {
                    node: 0,
                    widget_type: WIDGET_BUTTON,
                    count: 2,
                    maximum: 1,
                })
            );
        });
    }

    fn binding(event_kind: EventId, callback_id: StableId128) -> CallbackBinding {
        CallbackBinding::new(event_kind, SCHEMA_V1, callback_id)
    }

    fn validate_button_callbacks(
        document: &WidgetDocumentView<'_>,
        node: WidgetNodeView<'_>,
    ) -> Result<(), ModelError> {
        PortableWidgetSchemaValidator::new(&BUILTIN_PORTABLE_WIDGET_SCHEMAS)
            .unwrap()
            .validate_node(document, 0, node)
    }

    fn with_button_node<R>(
        callbacks: &[CallbackBinding],
        inspect: impl FnOnce(&WidgetDocumentView<'_>, WidgetNodeView<'_>) -> R,
    ) -> R {
        let children = [1];
        let text_properties = [WidgetProperty::new(
            PROPERTY_TEXT_CONTENT,
            PropertyValue::StringRef(0),
        )];
        let nodes = [
            WidgetNode::new(WIDGET_BUTTON, SCHEMA_V1)
                .callbacks(callbacks)
                .children(&children),
            WidgetNode::new(WIDGET_TEXT, SCHEMA_V1).properties(&text_properties),
        ];
        let image = WidgetDocument::new(1, 1, 0, &nodes, &["button"], &[])
            .encode(LIMITS)
            .unwrap();
        let document = WidgetDocumentView::decode(&image, LIMITS).unwrap();
        inspect(&document, document.node(0).unwrap())
    }
}
