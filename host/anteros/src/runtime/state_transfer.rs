use std::borrow::Cow;
use std::error::Error;
use std::fmt;

#[cfg(feature = "wasm-hot-reload")]
use crate::runtime::{GuestInstance, RuntimeError};
use crate::{
    ModelError, ModelLimits, StableId128, StateBundle, StateBundleView, StateEntry, StatePolicy,
    Version,
};

/// Coordinates bounded state preparation for one candidate generation.
///
/// The coordinator compares the old generation's canonical state snapshot with
/// the candidate's default snapshot. Matching schemas retain their old payload,
/// while state IDs introduced by the candidate retain their declared default.
/// Every other transition fails closed until an explicit migration or reset is
/// selected.
pub struct StateTransferCoordinator {
    model_limits: Option<ModelLimits>,
    migration_fuel: u64,
    migrations: Vec<StateMigration>,
}

impl StateTransferCoordinator {
    /// Creates a fail-closed coordinator with no document or migration budget.
    #[inline]
    pub const fn new() -> Self {
        Self {
            model_limits: None,
            migration_fuel: 0,
            migrations: Vec::new(),
        }
    }

    /// Sets the limits used to decode and encode every state snapshot.
    #[inline]
    pub const fn model_limits(mut self, limits: ModelLimits) -> Self {
        self.model_limits = Some(limits);
        self
    }

    /// Sets the fuel available to explicitly registered migration steps.
    #[inline]
    pub const fn migration_fuel(mut self, fuel: u64) -> Self {
        self.migration_fuel = fuel;
        self
    }

    /// Registers one deterministic directed schema migration.
    ///
    /// A state/schema/version source may have only one outgoing step. This
    /// makes multi-version path selection independent of registration order.
    pub fn register_migration(
        &mut self,
        migration: StateMigration,
    ) -> Result<(), StateTransferError> {
        if migration.fuel_cost == 0 {
            return Err(StateTransferError::ZeroMigrationFuel {
                state_id: migration.state_id,
            });
        }
        if self.migrations.iter().any(|registered| {
            registered.state_id == migration.state_id
                && registered.from_schema_id == migration.from_schema_id
                && registered.from_version == migration.from_version
        }) {
            return Err(StateTransferError::DuplicateMigration {
                state_id: migration.state_id,
                schema_id: migration.from_schema_id,
                version: migration.from_version,
            });
        }
        self.migrations.push(migration);
        Ok(())
    }

    /// Prepares the canonical state image that a candidate must import.
    ///
    /// Both inputs are validated before any output is produced. The returned
    /// image uses the candidate generation and candidate policies, but matching
    /// state payloads are borrowed from the previous image while encoding.
    pub fn prepare(
        &self,
        previous_bytes: &[u8],
        candidate_default_bytes: &[u8],
    ) -> Result<PreparedStateTransfer, StateTransferError> {
        let limits = self
            .model_limits
            .ok_or(StateTransferError::MissingModelLimits)?;
        let previous = StateBundleView::decode(previous_bytes, limits)?;
        let candidate = StateBundleView::decode(candidate_default_bytes, limits)?;
        if previous.application_id() != candidate.application_id() {
            return Err(StateTransferError::ApplicationMismatch {
                previous: previous.application_id(),
                candidate: candidate.application_id(),
            });
        }

        let mut previous_entries = previous.entries().peekable();
        let mut candidate_entries = candidate.entries().peekable();
        let mut prepared = Vec::with_capacity(candidate.entry_count() as usize);
        let mut preserved_entries = 0_u32;
        let mut defaulted_entries = 0_u32;
        let mut reset_state_ids = Vec::new();
        let mut migrated_state_ids = Vec::new();
        let mut migration_fuel_remaining = self.migration_fuel;

        while let Some(candidate_entry) = candidate_entries.next() {
            while previous_entries
                .peek()
                .is_some_and(|entry| entry.state_id() < candidate_entry.state_id())
            {
                let missing = previous_entries.next().unwrap();
                acknowledge_removed_state(
                    missing.state_id(),
                    missing.policy(),
                    &mut reset_state_ids,
                )?;
            }

            let payload = if previous_entries
                .peek()
                .is_some_and(|entry| entry.state_id() == candidate_entry.state_id())
            {
                let previous_entry = previous_entries.next().unwrap();
                if previous_entry.schema_id() == candidate_entry.schema_id()
                    && previous_entry.schema_version() == candidate_entry.schema_version()
                {
                    preserved_entries += 1;
                    Cow::Borrowed(previous_entry.payload())
                } else if let Some(payload) = self.migrate_entry(
                    &previous_entry,
                    &candidate_entry,
                    limits,
                    &mut migration_fuel_remaining,
                )? {
                    migrated_state_ids.push(previous_entry.state_id());
                    Cow::Owned(payload)
                } else if previous_entry.policy() == StatePolicy::ResetSafe {
                    reset_state_ids.push(previous_entry.state_id());
                    Cow::Borrowed(candidate_entry.payload())
                } else {
                    return Err(StateTransferError::StateIncompatible {
                        state_id: previous_entry.state_id(),
                        policy: previous_entry.policy(),
                    });
                }
            } else {
                defaulted_entries += 1;
                Cow::Borrowed(candidate_entry.payload())
            };
            prepared.push(PreparedEntry {
                state_id: candidate_entry.state_id(),
                schema_id: candidate_entry.schema_id(),
                schema_version: candidate_entry.schema_version(),
                policy: candidate_entry.policy(),
                payload,
            });
        }

        for missing in previous_entries {
            acknowledge_removed_state(missing.state_id(), missing.policy(), &mut reset_state_ids)?;
        }

        let entries = prepared
            .iter()
            .map(|entry| {
                StateEntry::new(
                    entry.state_id,
                    entry.schema_id,
                    entry.schema_version,
                    entry.policy,
                    entry.payload.as_ref(),
                )
            })
            .collect::<Vec<_>>();
        let bytes = StateBundle::new(
            candidate.application_id(),
            candidate.source_generation(),
            &entries,
        )
        .encode(limits)?;
        let state_bytes = bytes.len();

        Ok(PreparedStateTransfer {
            bytes,
            report: StateTransferReport {
                preserved_entries,
                defaulted_entries,
                migrated_state_ids,
                reset_state_ids,
                migration_fuel_consumed: self.migration_fuel - migration_fuel_remaining,
                state_bytes,
            },
        })
    }

    /// Verifies that a candidate exported exactly the state it imported.
    ///
    /// The verification image is decoded under the same limits before its
    /// canonical bytes are compared. A mismatch rejects the candidate even if
    /// its import call previously returned success.
    pub fn verify(
        &self,
        prepared: &PreparedStateTransfer,
        verification_bytes: &[u8],
    ) -> Result<(), StateTransferError> {
        let limits = self
            .model_limits
            .ok_or(StateTransferError::MissingModelLimits)?;
        StateBundleView::decode(verification_bytes, limits)?;
        if prepared.as_bytes() != verification_bytes {
            return Err(StateTransferError::VerificationMismatch);
        }
        Ok(())
    }

    /// Validates candidate-produced migration output and prepares final import state.
    ///
    /// Required schema changes must have one migrated output entry. Reset-safe
    /// schema changes may omit their output entry to explicitly select the
    /// candidate default. State that did not exist in the old generation always
    /// uses the candidate default and cannot be invented by migration output.
    pub fn prepare_candidate_migration(
        &self,
        previous_bytes: &[u8],
        candidate_default_bytes: &[u8],
        migrated_bytes: &[u8],
    ) -> Result<PreparedStateTransfer, StateTransferError> {
        let limits = self
            .model_limits
            .ok_or(StateTransferError::MissingModelLimits)?;
        let previous = StateBundleView::decode(previous_bytes, limits)?;
        let candidate = StateBundleView::decode(candidate_default_bytes, limits)?;
        let migrated = StateBundleView::decode(migrated_bytes, limits)?;
        if previous.application_id() != candidate.application_id()
            || migrated.application_id() != candidate.application_id()
            || migrated.source_generation() != candidate.source_generation()
        {
            return Err(StateTransferError::MigrationOutputMismatch);
        }

        let mut previous_entries = previous.entries().peekable();
        let mut migrated_entries = migrated.entries().peekable();
        let mut prepared = Vec::with_capacity(candidate.entry_count() as usize);
        let mut preserved_entries = 0_u32;
        let mut defaulted_entries = 0_u32;
        let mut migrated_state_ids = Vec::new();
        let mut reset_state_ids = Vec::new();

        for candidate_entry in candidate.entries() {
            while previous_entries
                .peek()
                .is_some_and(|entry| entry.state_id() < candidate_entry.state_id())
            {
                let missing = previous_entries.next().unwrap();
                acknowledge_removed_state(
                    missing.state_id(),
                    missing.policy(),
                    &mut reset_state_ids,
                )?;
            }
            if migrated_entries
                .peek()
                .is_some_and(|entry| entry.state_id() < candidate_entry.state_id())
            {
                return Err(StateTransferError::MigrationOutputMismatch);
            }

            let previous_entry = previous_entries
                .peek()
                .filter(|entry| entry.state_id() == candidate_entry.state_id());
            let migrated_entry = migrated_entries
                .peek()
                .filter(|entry| entry.state_id() == candidate_entry.state_id());
            let payload = match (previous_entry, migrated_entry) {
                (Some(previous_entry), Some(migrated_entry)) => {
                    validate_migration_target(&candidate_entry, migrated_entry)?;
                    let schema_matches = previous_entry.schema_id() == candidate_entry.schema_id()
                        && previous_entry.schema_version() == candidate_entry.schema_version();
                    if schema_matches {
                        if previous_entry.payload() != migrated_entry.payload() {
                            return Err(StateTransferError::MigrationOutputMismatch);
                        }
                        preserved_entries += 1;
                    } else {
                        migrated_state_ids.push(candidate_entry.state_id());
                    }
                    Cow::Borrowed(migrated_entry.payload())
                }
                (Some(previous_entry), None) => {
                    let schema_matches = previous_entry.schema_id() == candidate_entry.schema_id()
                        && previous_entry.schema_version() == candidate_entry.schema_version();
                    if schema_matches {
                        preserved_entries += 1;
                        Cow::Borrowed(previous_entry.payload())
                    } else if previous_entry.policy() == StatePolicy::ResetSafe {
                        reset_state_ids.push(previous_entry.state_id());
                        Cow::Borrowed(candidate_entry.payload())
                    } else {
                        return Err(StateTransferError::StateIncompatible {
                            state_id: previous_entry.state_id(),
                            policy: previous_entry.policy(),
                        });
                    }
                }
                (None, Some(_)) => return Err(StateTransferError::MigrationOutputMismatch),
                (None, None) => {
                    defaulted_entries += 1;
                    Cow::Borrowed(candidate_entry.payload())
                }
            };
            if previous_entry.is_some() {
                previous_entries.next();
            }
            if migrated_entry.is_some() {
                migrated_entries.next();
            }
            prepared.push(PreparedEntry {
                state_id: candidate_entry.state_id(),
                schema_id: candidate_entry.schema_id(),
                schema_version: candidate_entry.schema_version(),
                policy: candidate_entry.policy(),
                payload,
            });
        }

        for missing in previous_entries {
            acknowledge_removed_state(missing.state_id(), missing.policy(), &mut reset_state_ids)?;
        }
        if migrated_entries.next().is_some() {
            return Err(StateTransferError::MigrationOutputMismatch);
        }
        self.encode_prepared(
            candidate.application_id(),
            candidate.source_generation(),
            prepared,
            StateTransferReport {
                preserved_entries,
                defaulted_entries,
                migrated_state_ids,
                reset_state_ids,
                migration_fuel_consumed: 0,
                state_bytes: 0,
            },
            limits,
        )
    }

    /// Transfers state between two isolated persistent guest generations.
    ///
    /// The old guest is only exported. All import and verification work occurs
    /// on the candidate, so any failure can be handled by destroying only that
    /// candidate while the old live generation remains unchanged.
    #[cfg(feature = "wasm-hot-reload")]
    pub fn transfer_guest_state(
        &self,
        previous: &mut GuestInstance,
        candidate: &mut GuestInstance,
    ) -> Result<StateTransferReport, StateTransferError> {
        let limits = self
            .model_limits
            .ok_or(StateTransferError::MissingModelLimits)?;
        let previous_state =
            previous
                .export_state(limits)
                .map_err(|source| StateTransferError::Runtime {
                    stage: StateTransferStage::ExportPrevious,
                    source,
                })?;
        let candidate_defaults =
            candidate
                .export_state(limits)
                .map_err(|source| StateTransferError::Runtime {
                    stage: StateTransferStage::ExportCandidateDefaults,
                    source,
                })?;
        let prepared = if candidate.supports_state_migration()
            && schema_migration_required(previous_state.view(), candidate_defaults.view())
        {
            let migrated = candidate
                .migrate_state(previous_state.as_bytes(), limits)
                .map_err(|source| StateTransferError::Runtime {
                    stage: StateTransferStage::MigrateCandidate,
                    source,
                })?;
            let mut prepared = self.prepare_candidate_migration(
                previous_state.as_bytes(),
                candidate_defaults.as_bytes(),
                migrated.as_bytes(),
            )?;
            prepared.report.migration_fuel_consumed = candidate.last_migration_fuel_consumed();
            prepared
        } else {
            self.prepare(previous_state.as_bytes(), candidate_defaults.as_bytes())?
        };
        candidate
            .import_state(prepared.as_bytes(), limits)
            .map_err(|source| StateTransferError::Runtime {
                stage: StateTransferStage::ImportCandidate,
                source,
            })?;
        let verification =
            candidate
                .export_state(limits)
                .map_err(|source| StateTransferError::Runtime {
                    stage: StateTransferStage::ExportVerification,
                    source,
                })?;
        self.verify(&prepared, verification.as_bytes())?;
        Ok(prepared.into_report())
    }

    fn migrate_entry(
        &self,
        previous: &crate::StateEntryView<'_>,
        candidate: &crate::StateEntryView<'_>,
        limits: ModelLimits,
        fuel_remaining: &mut u64,
    ) -> Result<Option<Vec<u8>>, StateTransferError> {
        let mut schema_id = previous.schema_id();
        let mut version = previous.schema_version();
        let mut payload = None;

        for _ in 0..self.migrations.len() {
            if schema_id == candidate.schema_id() && version == candidate.schema_version() {
                return Ok(payload);
            }
            let Some(migration) = self.migrations.iter().find(|migration| {
                migration.state_id == previous.state_id()
                    && migration.from_schema_id == schema_id
                    && migration.from_version == version
            }) else {
                return Ok(None);
            };
            if migration.fuel_cost > *fuel_remaining {
                return Err(StateTransferError::MigrationFuelExhausted {
                    state_id: previous.state_id(),
                    required: migration.fuel_cost,
                    remaining: *fuel_remaining,
                });
            }
            *fuel_remaining -= migration.fuel_cost;
            let input = payload.as_deref().unwrap_or_else(|| previous.payload());
            let output = (migration.migrate)(input).map_err(|source| {
                StateTransferError::MigrationFailed {
                    state_id: previous.state_id(),
                    source,
                }
            })?;
            if output.len() > limits.max_blob_bytes as usize {
                return Err(StateTransferError::Model(ModelError::BlobTooLarge {
                    length: output.len(),
                    limit: limits.max_blob_bytes,
                }));
            }
            payload = Some(output);
            schema_id = migration.to_schema_id;
            version = migration.to_version;
        }

        if schema_id == candidate.schema_id() && version == candidate.schema_version() {
            Ok(payload)
        } else {
            Ok(None)
        }
    }

    fn encode_prepared(
        &self,
        application_id: StableId128,
        source_generation: u64,
        prepared: Vec<PreparedEntry<'_>>,
        mut report: StateTransferReport,
        limits: ModelLimits,
    ) -> Result<PreparedStateTransfer, StateTransferError> {
        let entries = prepared
            .iter()
            .map(|entry| {
                StateEntry::new(
                    entry.state_id,
                    entry.schema_id,
                    entry.schema_version,
                    entry.policy,
                    entry.payload.as_ref(),
                )
            })
            .collect::<Vec<_>>();
        let bytes = StateBundle::new(application_id, source_generation, &entries).encode(limits)?;
        report.state_bytes = bytes.len();
        Ok(PreparedStateTransfer { bytes, report })
    }
}

#[cfg(feature = "wasm-hot-reload")]
fn schema_migration_required(
    previous: StateBundleView<'_>,
    candidate: StateBundleView<'_>,
) -> bool {
    let mut previous_entries = previous.entries().peekable();
    let mut candidate_entries = candidate.entries().peekable();
    while let (Some(previous_entry), Some(candidate_entry)) =
        (previous_entries.peek(), candidate_entries.peek())
    {
        match previous_entry.state_id().cmp(&candidate_entry.state_id()) {
            std::cmp::Ordering::Less => {
                previous_entries.next();
            }
            std::cmp::Ordering::Greater => {
                candidate_entries.next();
            }
            std::cmp::Ordering::Equal => {
                if previous_entry.schema_id() != candidate_entry.schema_id()
                    || previous_entry.schema_version() != candidate_entry.schema_version()
                {
                    return true;
                }
                previous_entries.next();
                candidate_entries.next();
            }
        }
    }
    false
}

fn validate_migration_target(
    candidate: &crate::StateEntryView<'_>,
    migrated: &crate::StateEntryView<'_>,
) -> Result<(), StateTransferError> {
    if migrated.schema_id() != candidate.schema_id()
        || migrated.schema_version() != candidate.schema_version()
        || migrated.policy() != candidate.policy()
    {
        return Err(StateTransferError::MigrationOutputMismatch);
    }
    Ok(())
}

fn acknowledge_removed_state(
    state_id: StableId128,
    policy: StatePolicy,
    reset_state_ids: &mut Vec<StableId128>,
) -> Result<(), StateTransferError> {
    match policy {
        StatePolicy::Required => Err(StateTransferError::StateIncompatible { state_id, policy }),
        StatePolicy::ResetSafe => {
            reset_state_ids.push(state_id);
            Ok(())
        }
    }
}

impl Default for StateTransferCoordinator {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

struct PreparedEntry<'a> {
    state_id: StableId128,
    schema_id: StableId128,
    schema_version: Version,
    policy: StatePolicy,
    payload: Cow<'a, [u8]>,
}

/// One directed migration between two versions of a persistent state schema.
pub struct StateMigration {
    state_id: StableId128,
    from_schema_id: StableId128,
    from_version: Version,
    to_schema_id: StableId128,
    to_version: Version,
    fuel_cost: u64,
    migrate: fn(&[u8]) -> Result<Vec<u8>, StateMigrationFailure>,
}

impl StateMigration {
    /// Declares one migration and its deterministic candidate fuel charge.
    #[inline]
    pub const fn new(
        state_id: StableId128,
        from_schema_id: StableId128,
        from_version: Version,
        to_schema_id: StableId128,
        to_version: Version,
        fuel_cost: u64,
        migrate: fn(&[u8]) -> Result<Vec<u8>, StateMigrationFailure>,
    ) -> Self {
        Self {
            state_id,
            from_schema_id,
            from_version,
            to_schema_id,
            to_version,
            fuel_cost,
            migrate,
        }
    }
}

/// An application-defined failure returned by a state migration function.
#[derive(Debug)]
pub struct StateMigrationFailure {
    message: String,
}

impl StateMigrationFailure {
    /// Creates a sanitized migration failure diagnostic.
    #[inline]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for StateMigrationFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for StateMigrationFailure {}

/// One host-owned canonical state image ready for candidate import.
#[derive(Debug)]
pub struct PreparedStateTransfer {
    bytes: Vec<u8>,
    report: StateTransferReport,
}

impl PreparedStateTransfer {
    /// Returns the canonical bytes to pass to the candidate state import.
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Returns the explicit preservation, default, migration, and reset result.
    #[inline]
    pub const fn report(&self) -> &StateTransferReport {
        &self.report
    }

    #[cfg(feature = "wasm-hot-reload")]
    fn into_report(self) -> StateTransferReport {
        self.report
    }
}

/// The terminal state outcome prepared for one candidate generation.
#[derive(Debug, Eq, PartialEq)]
pub struct StateTransferReport {
    preserved_entries: u32,
    defaulted_entries: u32,
    migrated_state_ids: Vec<StableId128>,
    reset_state_ids: Vec<StableId128>,
    migration_fuel_consumed: u64,
    state_bytes: usize,
}

/// The guest operation at which coordinated state transfer failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StateTransferStage {
    /// Exporting the currently live generation failed.
    ExportPrevious,
    /// Exporting the candidate's state defaults failed.
    ExportCandidateDefaults,
    /// Executing the candidate's migration export failed.
    MigrateCandidate,
    /// Importing the prepared image into the candidate failed.
    ImportCandidate,
    /// Exporting the candidate's verification image failed.
    ExportVerification,
}

impl StateTransferReport {
    /// Returns the number of payloads copied from schema-compatible old state.
    #[inline]
    pub const fn preserved_entries(&self) -> u32 {
        self.preserved_entries
    }

    /// Returns the number of new candidate state defaults retained.
    #[inline]
    pub const fn defaulted_entries(&self) -> u32 {
        self.defaulted_entries
    }

    /// Returns state IDs whose payload passed through one or more migrations.
    #[inline]
    pub fn migrated_state_ids(&self) -> &[StableId128] {
        &self.migrated_state_ids
    }

    /// Returns old reset-safe state IDs explicitly acknowledged as reset.
    #[inline]
    pub fn reset_state_ids(&self) -> &[StableId128] {
        &self.reset_state_ids
    }

    /// Returns migration fuel charged while preparing this image.
    #[inline]
    pub const fn migration_fuel_consumed(&self) -> u64 {
        self.migration_fuel_consumed
    }

    /// Returns the complete canonical candidate state image size in bytes.
    #[inline]
    pub const fn state_bytes(&self) -> usize {
        self.state_bytes
    }
}

/// A failure while preparing or validating guest state transfer.
#[derive(Debug)]
pub enum StateTransferError {
    /// No model limits were configured, so bounded decoding cannot begin.
    MissingModelLimits,
    /// A canonical state document was malformed or exceeded its limits.
    Model(ModelError),
    /// The old and candidate snapshots belong to different applications.
    ApplicationMismatch {
        previous: StableId128,
        candidate: StableId128,
    },
    /// An old entry cannot be represented by the candidate without loss.
    StateIncompatible {
        state_id: StableId128,
        policy: StatePolicy,
    },
    /// One migration source was registered more than once.
    DuplicateMigration {
        state_id: StableId128,
        schema_id: StableId128,
        version: Version,
    },
    /// A migration declared no fuel charge and cannot be accounted.
    ZeroMigrationFuel { state_id: StableId128 },
    /// The next migration step exceeds this candidate's remaining fuel.
    MigrationFuelExhausted {
        state_id: StableId128,
        required: u64,
        remaining: u64,
    },
    /// An application migration rejected its source payload.
    MigrationFailed {
        state_id: StableId128,
        source: StateMigrationFailure,
    },
    /// The candidate's post-import export differs from the prepared image.
    VerificationMismatch,
    /// Candidate migration output did not match the target state declaration.
    MigrationOutputMismatch,
    /// A persistent guest operation failed during coordinated transfer.
    #[cfg(feature = "wasm-hot-reload")]
    Runtime {
        stage: StateTransferStage,
        source: RuntimeError,
    },
}

impl From<ModelError> for StateTransferError {
    #[inline]
    fn from(error: ModelError) -> Self {
        Self::Model(error)
    }
}

impl fmt::Display for StateTransferError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingModelLimits => {
                formatter.write_str("state transfer model limits are not configured")
            }
            Self::Model(error) => write!(formatter, "invalid state document: {error}"),
            Self::ApplicationMismatch {
                previous,
                candidate,
            } => write!(
                formatter,
                "state application identity changed from {previous:?} to {candidate:?}"
            ),
            Self::StateIncompatible { state_id, policy } => write!(
                formatter,
                "state {state_id:?} with {policy:?} policy has no compatible candidate entry"
            ),
            Self::DuplicateMigration {
                state_id,
                schema_id,
                version,
            } => write!(
                formatter,
                "duplicate migration source for state {state_id:?}, schema {schema_id:?}, version {version:?}"
            ),
            Self::ZeroMigrationFuel { state_id } => {
                write!(
                    formatter,
                    "migration for state {state_id:?} has zero fuel cost"
                )
            }
            Self::MigrationFuelExhausted {
                state_id,
                required,
                remaining,
            } => write!(
                formatter,
                "migration for state {state_id:?} requires {required} fuel but {remaining} remains"
            ),
            Self::MigrationFailed { state_id, source } => {
                write!(
                    formatter,
                    "migration for state {state_id:?} failed: {source}"
                )
            }
            Self::VerificationMismatch => formatter
                .write_str("candidate state verification export does not match the imported state"),
            Self::MigrationOutputMismatch => formatter.write_str(
                "candidate migration output does not match the target state declaration",
            ),
            #[cfg(feature = "wasm-hot-reload")]
            Self::Runtime { stage, source } => {
                write!(
                    formatter,
                    "state transfer failed during {stage:?}: {source}"
                )
            }
        }
    }
}

impl Error for StateTransferError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Model(error) => Some(error),
            Self::MigrationFailed { source, .. } => Some(source),
            #[cfg(feature = "wasm-hot-reload")]
            Self::Runtime { source, .. } => Some(source),
            _ => None,
        }
    }
}
