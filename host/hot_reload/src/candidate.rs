use std::error::Error;
use std::fmt;
use std::fs;
use std::path::Path;
use std::rc::Rc;

use aimer_anteros::{
    AbiVersion, CALLBACK_EVENT_FORMAT_VERSION, CURRENT_ABI_VERSION, CallbackBindingError,
    CallbackBindingSnapshot, CapabilityRegistry, Generation, GenerationId, GenerationLimits,
    GuestDiagnosticCategory, GuestInstance, ModelLimits, ReloadSnapshot, ReloadStage, Runtime,
    RuntimeError, STATE_FORMAT_VERSION, StateTransferCoordinator, StateTransferError,
    StateTransferReport, StateTransferStage, Version, WIDGET_IR_FORMAT_VERSION,
    WidgetMaterializeError,
};
use aimer_venus::LocalScheduler;
use aimer_widget::base::BuildContext;
use aimer_widget::{AnyElement, ReconciliationPlanError, plan_element_reconciliation};

use super::{MissingNativeMaterializer, WidgetIrStageDiagnostics, materialize_aimer_widget_tree};

/// Resource ceilings applied while preparing one reload candidate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReloadCandidateLimits {
    model: ModelLimits,
    generation: GenerationLimits,
    max_callback_bindings: u32,
}

impl ReloadCandidateLimits {
    /// Creates explicit portable-model, generation-resource, and callback limits.
    #[inline]
    pub const fn new(
        model: ModelLimits,
        generation: GenerationLimits,
        max_callback_bindings: u32,
    ) -> Self {
        Self {
            model,
            generation,
            max_callback_bindings,
        }
    }

    /// Returns the portable document limits shared by preparation and callbacks.
    #[inline]
    pub const fn model(self) -> ModelLimits {
        self.model
    }

    /// Returns the maximum callback table size for one generation.
    #[inline]
    pub const fn max_callback_bindings(self) -> u32 {
        self.max_callback_bindings
    }
}

/// A fully prepared candidate that has not been exposed to the application tree.
///
/// The snapshot owns its isolated interpreter, callback table, and disconnected
/// native root. Dropping this value retires the candidate without touching the
/// active generation. Commit authority remains exclusively with the Quiver
/// safe-point host.
pub struct PreparedReloadCandidate {
    snapshot: ReloadSnapshot<GuestInstance, AnyElement>,
    state_transfer: StateTransferReport,
}

impl PreparedReloadCandidate {
    /// Borrows the verified state-transfer diagnostics for terminal reporting.
    #[inline]
    pub const fn state_transfer_report(&self) -> &StateTransferReport {
        &self.state_transfer
    }

    /// Moves the coherent candidate snapshot into the safe-point command queue.
    #[inline]
    pub fn into_snapshot(self) -> ReloadSnapshot<GuestInstance, AnyElement> {
        self.snapshot
    }
}

/// Permanent host services used to prepare isolated reload candidates.
///
/// The preparer borrows immutable runtime policy and capability registrations.
/// It performs guest execution only while preparing; the returned snapshot is
/// later committed without interpreter or network work at the safe point.
pub struct ReloadCandidatePreparer<'a> {
    runtime: &'a Runtime,
    capabilities: &'a CapabilityRegistry,
    state_transfer: &'a StateTransferCoordinator,
    scheduler: Rc<LocalScheduler>,
    limits: ReloadCandidateLimits,
    widget_ir_diagnostics: bool,
}

impl<'a> ReloadCandidatePreparer<'a> {
    /// Creates a preparer from permanent host policy and generation services.
    #[inline]
    pub const fn new(
        runtime: &'a Runtime,
        capabilities: &'a CapabilityRegistry,
        state_transfer: &'a StateTransferCoordinator,
        scheduler: Rc<LocalScheduler>,
        limits: ReloadCandidateLimits,
    ) -> Self {
        Self {
            runtime,
            capabilities,
            state_transfer,
            scheduler,
            limits,
            widget_ir_diagnostics: false,
        }
    }

    /// Enables deterministic Widget IR stage output after native construction succeeds.
    #[inline]
    pub const fn widget_ir_diagnostics(mut self, enabled: bool) -> Self {
        self.widget_ir_diagnostics = enabled;
        self
    }

    /// Converts authenticated module bytes into one staged coherent snapshot.
    ///
    /// Preparation validates and instantiates the module, checks its manifest,
    /// transfers and verifies complete guest state, builds Widget IR, copies the
    /// callback table, materializes a disconnected native tree, and validates a
    /// side-effect-free reconciliation plan. The active snapshot is used only
    /// for state export and immutable reconciliation planning.
    pub fn prepare(
        &self,
        module: &[u8],
        generation_id: GenerationId,
        active: &mut ReloadSnapshot<GuestInstance, AnyElement>,
        ctx: &BuildContext,
        dispatch_callback: impl Fn(aimer_anteros::StableId128) + 'static,
    ) -> Result<PreparedReloadCandidate, ReloadCandidatePreparationError> {
        if generation_id <= active.generation_id() {
            return Err(ReloadCandidatePreparationError::GenerationNotNewer {
                active: active.generation_id(),
                candidate: generation_id,
            });
        }

        let mut candidate = self.instantiate(module, generation_id)?;
        let state_transfer = self
            .state_transfer
            .transfer_guest_state(active.generation_mut().guest_mut(), &mut candidate)
            .map_err(ReloadCandidatePreparationError::StateTransfer)?;
        let snapshot = self.finish(
            candidate,
            generation_id,
            Some(active.root()),
            ctx,
            dispatch_callback,
        )?;
        Ok(PreparedReloadCandidate {
            snapshot,
            state_transfer,
        })
    }

    /// Prepares the first guest generation before any guest state exists.
    ///
    /// The initial module still validates its default state image, manifest,
    /// callback table, Widget IR, and disconnected native tree. State migration
    /// and reconciliation against a previous guest are intentionally absent.
    pub fn prepare_initial(
        &self,
        module: &[u8],
        generation_id: GenerationId,
        ctx: &BuildContext,
        dispatch_callback: impl Fn(aimer_anteros::StableId128) + 'static,
    ) -> Result<ReloadSnapshot<GuestInstance, AnyElement>, ReloadCandidatePreparationError> {
        let mut candidate = self.instantiate(module, generation_id)?;
        candidate
            .export_state(self.limits.model)
            .map_err(ReloadCandidatePreparationError::InitialState)?;
        self.finish(candidate, generation_id, None, ctx, dispatch_callback)
    }

    fn instantiate(
        &self,
        module: &[u8],
        generation_id: GenerationId,
    ) -> Result<GuestInstance, ReloadCandidatePreparationError> {
        let mut candidate = self
            .runtime
            .instantiate_with_capabilities(
                module,
                self.capabilities,
                self.limits.model,
                generation_id,
            )
            .map_err(|error| {
                if error.kind() == aimer_anteros::RuntimeErrorKind::Initialization {
                    ReloadCandidatePreparationError::Initialize(error)
                } else {
                    ReloadCandidatePreparationError::Instantiate(error)
                }
            })?;
        let manifest = candidate
            .manifest(self.limits.model)
            .map_err(ReloadCandidatePreparationError::Manifest)?;
        validate_manifest(&manifest.view())?;
        Ok(candidate)
    }

    fn finish(
        &self,
        mut candidate: GuestInstance,
        generation_id: GenerationId,
        active_root: Option<&AnyElement>,
        ctx: &BuildContext,
        dispatch_callback: impl Fn(aimer_anteros::StableId128) + 'static,
    ) -> Result<ReloadSnapshot<GuestInstance, AnyElement>, ReloadCandidatePreparationError> {
        let window_size = ctx.window.inner_size();
        candidate
            .set_window_metrics(
                window_size.width,
                window_size.height,
                ctx.window.scale_factor(),
            )
            .map_err(ReloadCandidatePreparationError::Build)?;
        let widget_image = candidate
            .build(self.limits.model)
            .map_err(ReloadCandidatePreparationError::Build)?;
        let document = widget_image.view();
        if document.generation_id() != generation_id.get() {
            return Err(ReloadCandidatePreparationError::WidgetGenerationMismatch {
                expected: generation_id,
                actual: document.generation_id(),
            });
        }
        let callbacks =
            CallbackBindingSnapshot::from_document(&document, self.limits.max_callback_bindings)
                .map_err(ReloadCandidatePreparationError::Callbacks)?;
        let root = materialize_aimer_widget_tree(
            widget_image.as_bytes(),
            self.limits.model,
            ctx,
            dispatch_callback,
        )
        .map_err(ReloadCandidatePreparationError::Materialize)?;
        if let Some(diagnostics) = WidgetIrStageDiagnostics::new(self.widget_ir_diagnostics)
            .render(widget_image.as_bytes(), self.limits.model)
            .map_err(|error| {
                ReloadCandidatePreparationError::Materialize(WidgetMaterializeError::Model(error))
            })?
        {
            eprintln!("{diagnostics}");
        }
        if let Some(active_root) = active_root {
            plan_element_reconciliation(active_root.as_ref(), root.as_ref())
                .validate()
                .map_err(ReloadCandidatePreparationError::Reconciliation)?;
        }
        let generation = Generation::with_guest(
            generation_id,
            callbacks,
            Rc::clone(&self.scheduler),
            self.limits.generation,
            candidate,
        );
        Ok(ReloadSnapshot::new(generation, root))
    }
}

/// Incompatibility between one canonical manifest and permanent host formats.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManifestCompatibilityError {
    /// The host core ABI is outside the guest's declared inclusive range.
    CoreAbi {
        minimum: AbiVersion,
        maximum: AbiVersion,
        host: AbiVersion,
    },
    /// The guest declared an unsupported Widget IR format.
    WidgetIr {
        declared: Version,
        supported: Version,
    },
    /// The guest declared an unsupported callback-event format.
    CallbackEvent {
        declared: Version,
        supported: Version,
    },
    /// The guest declared an unsupported state-bundle format.
    State {
        declared: Version,
        supported: Version,
    },
}

impl fmt::Display for ManifestCompatibilityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CoreAbi {
                minimum,
                maximum,
                host,
            } => write!(
                formatter,
                "guest core ABI range {minimum:?}..={maximum:?} does not include host {host:?}"
            ),
            Self::WidgetIr {
                declared,
                supported,
            } => write!(
                formatter,
                "guest Widget IR format {declared:?} is not supported; host requires {supported:?}"
            ),
            Self::CallbackEvent {
                declared,
                supported,
            } => write!(
                formatter,
                "guest callback-event format {declared:?} is not supported; host requires {supported:?}"
            ),
            Self::State {
                declared,
                supported,
            } => write!(
                formatter,
                "guest state-bundle format {declared:?} is not supported; host requires {supported:?}"
            ),
        }
    }
}

impl Error for ManifestCompatibilityError {}

/// A named failure while converting module bytes into a staged snapshot.
#[derive(Debug)]
pub enum ReloadCandidatePreparationError {
    /// Candidate identities must advance monotonically beyond the active one.
    GenerationNotNewer {
        active: GenerationId,
        candidate: GenerationId,
    },
    /// Module validation, import linking, or instantiation failed.
    Instantiate(RuntimeError),
    /// Reading the canonical manifest failed after instantiation.
    Manifest(RuntimeError),
    /// Controlled guest initialization rejected immutable host context.
    Initialize(RuntimeError),
    /// The manifest declared formats unavailable in this permanent host.
    ManifestCompatibility(ManifestCompatibilityError),
    /// Complete state export, migration, import, or verification failed.
    StateTransfer(StateTransferError),
    /// The first generation's canonical default state could not be exported.
    InitialState(RuntimeError),
    /// Candidate Widget IR execution or canonical decoding failed.
    Build(RuntimeError),
    /// Widget IR generation identity did not match its candidate owner.
    WidgetGenerationMismatch { expected: GenerationId, actual: u64 },
    /// Copying the immutable callback table failed.
    Callbacks(CallbackBindingError),
    /// Concrete disconnected native materialization failed.
    Materialize(WidgetMaterializeError<MissingNativeMaterializer>),
    /// Side-effect-free reconciliation planning failed.
    Reconciliation(ReconciliationPlanError),
}

impl ReloadCandidatePreparationError {
    /// Returns the stable transactional stage for protocol diagnostics.
    pub const fn stage(&self) -> ReloadStage {
        match self {
            Self::GenerationNotNewer { .. } => ReloadStage::Preflight,
            Self::Instantiate(_) | Self::Manifest(_) | Self::ManifestCompatibility(_) => {
                ReloadStage::Instantiate
            }
            Self::Initialize(_) => ReloadStage::Initialize,
            Self::StateTransfer(error) => state_transfer_reload_stage(error),
            Self::InitialState(_) => ReloadStage::ExportState,
            Self::Build(_) => ReloadStage::Build,
            Self::WidgetGenerationMismatch { .. } | Self::Callbacks(_) => ReloadStage::Validate,
            Self::Materialize(_) => ReloadStage::Materialize,
            Self::Reconciliation(_) => ReloadStage::PrepareReconciliation,
        }
    }
}

impl fmt::Display for ReloadCandidatePreparationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GenerationNotNewer { active, candidate } => write!(
                formatter,
                "candidate generation {} does not advance active generation {}",
                candidate.get(),
                active.get()
            ),
            Self::Instantiate(error) => {
                write!(formatter, "candidate instantiation failed: {error}")
            }
            Self::Manifest(error) => write!(formatter, "candidate manifest failed: {error}"),
            Self::Initialize(error) => {
                write!(formatter, "candidate initialization failed: {error}")
            }
            Self::ManifestCompatibility(error) => error.fmt(formatter),
            Self::StateTransfer(error) => {
                write!(formatter, "candidate state transfer failed: {error}")
            }
            Self::InitialState(error) => {
                write!(formatter, "candidate default state failed: {error}")
            }
            Self::Build(error) => format_build_error(formatter, error),
            Self::WidgetGenerationMismatch { expected, actual } => write!(
                formatter,
                "candidate Widget IR generation {actual} does not match assigned generation {}",
                expected.get()
            ),
            Self::Callbacks(error) => write!(formatter, "candidate callbacks failed: {error}"),
            Self::Materialize(error) => {
                write!(formatter, "candidate materialization failed: {error}")
            }
            Self::Reconciliation(error) => {
                write!(formatter, "candidate reconciliation failed: {error}")
            }
        }
    }
}

fn format_build_error(formatter: &mut fmt::Formatter<'_>, error: &RuntimeError) -> fmt::Result {
    let Some(diagnostic) = error
        .diagnostic()
        .filter(|diagnostic| diagnostic.category() == GuestDiagnosticCategory::Panic)
    else {
        return write!(formatter, "candidate build failed: {error}");
    };

    let (phase, payload) = diagnostic
        .message()
        .strip_prefix("during ")
        .and_then(|message| message.split_once(": "))
        .unwrap_or(("build", diagnostic.message()));
    if let Some(widget) = diagnostic.widget() {
        write!(
            formatter,
            "candidate build failed: Widget `{widget}` panicked during {phase}: {payload}"
        )?;
    } else {
        write!(
            formatter,
            "candidate build failed: guest panicked during {phase}: {payload}"
        )?;
    }

    let Some(location) = diagnostic.location() else {
        return Ok(());
    };
    write!(formatter, "\n\n")?;
    if let Some(site) = safe_guest_panic_site(location) {
        write!(formatter, "{site}")
    } else {
        write!(
            formatter,
            "at {}:{}:{}",
            location.file(),
            location.line(),
            location.column()
        )
    }
}

fn safe_guest_panic_site(
    location: &aimer_anteros::GuestSourceLocation,
) -> Option<aimer_utils::PanicSite> {
    let path = Path::new(location.file());
    let canonical = fs::canonicalize(path).ok()?;
    let current = fs::canonicalize(std::env::current_dir().ok()?).ok()?;
    if !canonical.starts_with(current) {
        return None;
    }
    Some(aimer_utils::PanicSite::new(
        canonical.to_string_lossy().into_owned(),
        location.line(),
        location.column(),
    ))
}

impl Error for ReloadCandidatePreparationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::GenerationNotNewer { .. } | Self::WidgetGenerationMismatch { .. } => None,
            Self::Instantiate(error)
            | Self::Manifest(error)
            | Self::Initialize(error)
            | Self::InitialState(error)
            | Self::Build(error) => Some(error),
            Self::ManifestCompatibility(error) => Some(error),
            Self::StateTransfer(error) => Some(error),
            Self::Callbacks(error) => Some(error),
            Self::Materialize(error) => Some(error),
            Self::Reconciliation(error) => Some(error),
        }
    }
}

impl From<ManifestCompatibilityError> for ReloadCandidatePreparationError {
    #[inline]
    fn from(error: ManifestCompatibilityError) -> Self {
        Self::ManifestCompatibility(error)
    }
}

fn validate_manifest(
    manifest: &aimer_anteros::ManifestView<'_>,
) -> Result<(), ManifestCompatibilityError> {
    if (CURRENT_ABI_VERSION.to_packed() as u64) < (manifest.minimum_abi().to_packed() as u64)
        || (CURRENT_ABI_VERSION.to_packed() as u64) > (manifest.maximum_abi().to_packed() as u64)
    {
        return Err(ManifestCompatibilityError::CoreAbi {
            minimum: manifest.minimum_abi(),
            maximum: manifest.maximum_abi(),
            host: CURRENT_ABI_VERSION,
        });
    }
    if manifest.widget_ir_version() != WIDGET_IR_FORMAT_VERSION {
        return Err(ManifestCompatibilityError::WidgetIr {
            declared: manifest.widget_ir_version(),
            supported: WIDGET_IR_FORMAT_VERSION,
        });
    }
    if manifest.callback_event_version() != CALLBACK_EVENT_FORMAT_VERSION {
        return Err(ManifestCompatibilityError::CallbackEvent {
            declared: manifest.callback_event_version(),
            supported: CALLBACK_EVENT_FORMAT_VERSION,
        });
    }
    if manifest.state_version() != STATE_FORMAT_VERSION {
        return Err(ManifestCompatibilityError::State {
            declared: manifest.state_version(),
            supported: STATE_FORMAT_VERSION,
        });
    }
    Ok(())
}

const fn state_transfer_reload_stage(error: &StateTransferError) -> ReloadStage {
    match error {
        StateTransferError::Runtime { stage, .. } => match stage {
            StateTransferStage::ExportPrevious => ReloadStage::ExportState,
            StateTransferStage::ExportCandidateDefaults => ReloadStage::Initialize,
            StateTransferStage::MigrateCandidate => ReloadStage::MigrateState,
            StateTransferStage::ImportCandidate | StateTransferStage::ExportVerification => {
                ReloadStage::ImportState
            }
        },
        StateTransferError::MissingModelLimits => ReloadStage::Preflight,
        StateTransferError::MigrationFuelExhausted { .. }
        | StateTransferError::MigrationFailed { .. }
        | StateTransferError::MigrationOutputMismatch
        | StateTransferError::ZeroMigrationFuel { .. }
        | StateTransferError::DuplicateMigration { .. } => ReloadStage::MigrateState,
        _ => ReloadStage::ImportState,
    }
}
