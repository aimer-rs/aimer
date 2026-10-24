use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use super::codec::{
    DecodeError, Decoder, EncodeError, Encoder, PortableApply, PortableDecode, PortableEncode,
    PortableLimits, decode_from_slice, encode_to_vec,
};
use super::identity::{StableId128, StableSchemaId, StableSlotId, StableTypeId};
use super::schema::AimerReflectionType;

const STATE_MAGIC: &[u8; 8] = b"AIMSTATE";

#[derive(Clone)]
struct StateEntry {
    type_id: StableTypeId,
    schema_id: StableSchemaId,
    revision: u64,
    payload: Vec<u8>,
}

/// A failure to insert, transfer, or restore portable state.
#[doc(hidden)]
#[derive(Debug)]
pub enum StateRegistryError {
    /// More than one entry used the same stable state slot.
    DuplicateSlot { slot_id: StableSlotId },
    /// An imported or restored slot is not registered by the active program.
    UnknownSlot { slot_id: StableSlotId },
    /// The source and active reflected Rust types differ.
    TypeMismatch {
        slot_id: StableSlotId,
        expected: StableTypeId,
        actual: StableTypeId,
    },
    /// The source and active structural schemas differ.
    SchemaMismatch {
        slot_id: StableSlotId,
        expected: StableSchemaId,
        actual: StableSchemaId,
    },
    /// An import attempted to move a slot's revision backwards.
    RevisionRegression { slot_id: StableSlotId, active: u64, imported: u64 },
    /// A mutation cannot advance a slot beyond the maximum revision.
    RevisionOverflow { slot_id: StableSlotId },
    /// The state document has an unknown preamble.
    InvalidMagic,
    /// Encoding one value or the state document failed.
    Encode(EncodeError),
    /// Decoding one value or the state document failed.
    Decode(DecodeError),
}

impl fmt::Display for StateRegistryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateSlot { slot_id } => write!(formatter, "duplicate portable state slot {slot_id}"),
            Self::UnknownSlot { slot_id } => write!(formatter, "unknown portable state slot {slot_id}"),
            Self::TypeMismatch { slot_id, expected, actual } => write!(
                formatter,
                "portable state slot {slot_id} has type {actual}, expected {expected}"
            ),
            Self::SchemaMismatch { slot_id, expected, actual } => write!(
                formatter,
                "portable state slot {slot_id} has schema {actual}, expected {expected}"
            ),
            Self::RevisionRegression { slot_id, active, imported } => write!(
                formatter,
                "portable state slot {slot_id} revision regressed from {active} to {imported}"
            ),
            Self::RevisionOverflow { slot_id } => {
                write!(formatter, "portable state slot {slot_id} revision overflowed")
            }
            Self::InvalidMagic => formatter.write_str("invalid portable state document preamble"),
            Self::Encode(error) => error.fmt(formatter),
            Self::Decode(error) => error.fmt(formatter),
        }
    }
}

impl Error for StateRegistryError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Encode(error) => Some(error),
            Self::Decode(error) => Some(error),
            _ => None,
        }
    }
}

impl From<EncodeError> for StateRegistryError {
    #[inline]
    fn from(error: EncodeError) -> Self { Self::Encode(error) }
}

impl From<DecodeError> for StateRegistryError {
    #[inline]
    fn from(error: DecodeError) -> Self { Self::Decode(error) }
}

/// A deterministic registry of typed, revisioned portable state slots.
///
/// Import validates and owns the complete candidate image before changing an
/// active entry. Missing active slots retain their current defaults, allowing a
/// new generation to introduce fresh state independently of old snapshots.
#[doc(hidden)]
pub struct StateRegistry {
    limits: PortableLimits,
    entries: BTreeMap<StableSlotId, StateEntry>,
}

impl StateRegistry {
    /// Creates an empty registry using the supplied transfer and codec limits.
    #[inline]
    pub const fn new(limits: PortableLimits) -> Self {
        Self { limits, entries: BTreeMap::new() }
    }

    /// Encodes and registers one typed state slot.
    ///
    /// Duplicate slot IDs are rejected even when all metadata is identical.
    pub fn insert<T>(
        &mut self,
        slot_id: StableSlotId,
        revision: u64,
        value: &T,
    ) -> Result<(), StateRegistryError>
    where
        T: AimerReflectionType + PortableEncode,
    {
        if self.entries.contains_key(&slot_id) {
            return Err(StateRegistryError::DuplicateSlot { slot_id });
        }
        let payload = encode_to_vec(value, self.limits)?;
        if payload.len() > self.limits.max_blob_bytes() {
            return Err(StateRegistryError::Encode(EncodeError::LimitExceeded {
                limit: super::codec::LimitKind::BlobBytes,
                max: self.limits.max_blob_bytes(),
                actual: payload.len(),
            }));
        }
        self.entries.insert(slot_id, StateEntry {
            type_id: T::TYPE_ID,
            schema_id: T::schema_id(),
            revision,
            payload,
        });
        Ok(())
    }

    /// Returns the current revision of a registered slot.
    #[inline]
    pub fn revision(&self, slot_id: StableSlotId) -> Option<u64> {
        self.entries.get(&slot_id).map(|entry| entry.revision)
    }

    #[inline]
    #[cfg(feature = "portable-guest")]
    pub(crate) fn remove(&mut self, slot_id: StableSlotId) -> bool {
        self.entries.remove(&slot_id).is_some()
    }

    /// Encodes all entries in stable slot-ID order.
    pub fn export(&self) -> Result<Vec<u8>, StateRegistryError> {
        let mut output = Vec::new();
        let mut encoder = Encoder::new(&mut output, self.limits);
        encoder.write(STATE_MAGIC)?;
        encoder.claim_entries(self.entries.len())?;
        encoder.write_length(self.entries.len())?;
        for (slot_id, entry) in &self.entries {
            encoder.write(&slot_id.to_bytes())?;
            encoder.write(&entry.type_id.to_bytes())?;
            encoder.write(&entry.schema_id.to_bytes())?;
            entry.revision.encode(&mut encoder)?;
            encoder.blob(&entry.payload)?;
        }
        Ok(output)
    }

    /// Transactionally imports compatible state into registered active slots.
    ///
    /// Duplicate, malformed, trailing, unknown, incompatible, or regressing
    /// input returns an error without modifying any active payload or revision.
    pub fn import(&mut self, input: &[u8]) -> Result<(), StateRegistryError> {
        let imported = Self::decode_entries(input, self.limits)?;
        for (slot_id, candidate) in &imported {
            let active = self.entries.get(slot_id)
                .ok_or(StateRegistryError::UnknownSlot { slot_id: *slot_id })?;
            if candidate.type_id != active.type_id {
                return Err(StateRegistryError::TypeMismatch {
                    slot_id: *slot_id,
                    expected: active.type_id,
                    actual: candidate.type_id,
                });
            }
            if candidate.schema_id != active.schema_id {
                return Err(StateRegistryError::SchemaMismatch {
                    slot_id: *slot_id,
                    expected: active.schema_id,
                    actual: candidate.schema_id,
                });
            }
            if candidate.revision < active.revision {
                return Err(StateRegistryError::RevisionRegression {
                    slot_id: *slot_id,
                    active: active.revision,
                    imported: candidate.revision,
                });
            }
        }
        for (slot_id, candidate) in imported {
            *self.entries.get_mut(&slot_id).expect("all imported slots were validated") = candidate;
        }
        Ok(())
    }

    /// Decodes one slot after exact reflected type and schema validation.
    pub fn restore<T>(&self, slot_id: StableSlotId) -> Result<T, StateRegistryError>
    where
        T: AimerReflectionType + PortableDecode,
    {
        let entry = self.entries.get(&slot_id)
            .ok_or(StateRegistryError::UnknownSlot { slot_id })?;
        if entry.type_id != T::TYPE_ID {
            return Err(StateRegistryError::TypeMismatch {
                slot_id,
                expected: T::TYPE_ID,
                actual: entry.type_id,
            });
        }
        let schema_id = T::schema_id();
        if entry.schema_id != schema_id {
            return Err(StateRegistryError::SchemaMismatch {
                slot_id,
                expected: schema_id,
                actual: entry.schema_id,
            });
        }
        decode_from_slice(&entry.payload, self.limits).map_err(StateRegistryError::Decode)
    }

    /// Applies one mutation transactionally and advances the slot revision.
    ///
    /// The active entry changes only after decoding, user mutation, and bounded
    /// re-encoding have all completed successfully. Generated stateful wrappers
    /// use this operation at their queued rebuild boundary.
    pub fn mutate<T, F>(
        &mut self,
        slot_id: StableSlotId,
        mutation: F,
    ) -> Result<(), StateRegistryError>
    where
        T: AimerReflectionType + PortableDecode + PortableEncode,
        F: FnOnce(&mut T),
    {
        let entry = self.compatible_entry::<T>(slot_id)?;
        let revision = entry
            .revision
            .checked_add(1)
            .ok_or(StateRegistryError::RevisionOverflow { slot_id })?;
        let mut value = decode_from_slice::<T>(&entry.payload, self.limits)?;
        mutation(&mut value);
        let payload = encode_to_vec(&value, self.limits)?;
        if payload.len() > self.limits.max_blob_bytes() {
            return Err(StateRegistryError::Encode(EncodeError::LimitExceeded {
                limit: super::codec::LimitKind::BlobBytes,
                max: self.limits.max_blob_bytes(),
                actual: payload.len(),
            }));
        }
        let entry = self
            .entries
            .get_mut(&slot_id)
            .expect("compatible state entry remains registered");
        entry.payload = payload;
        entry.revision = revision;
        Ok(())
    }

    /// Re-encodes one already-mutated live value and advances its revision.
    ///
    /// Encoding completes before the active entry changes, so a limit or codec
    /// failure leaves both payload and revision untouched.
    pub fn refresh<T>(
        &mut self,
        slot_id: StableSlotId,
        value: &T,
    ) -> Result<(), StateRegistryError>
    where
        T: AimerReflectionType + PortableEncode,
    {
        let entry = self.compatible_entry::<T>(slot_id)?;
        let revision = entry
            .revision
            .checked_add(1)
            .ok_or(StateRegistryError::RevisionOverflow { slot_id })?;
        let payload = encode_to_vec(value, self.limits)?;
        if payload.len() > self.limits.max_blob_bytes() {
            return Err(StateRegistryError::Encode(EncodeError::LimitExceeded {
                limit: super::codec::LimitKind::BlobBytes,
                max: self.limits.max_blob_bytes(),
                actual: payload.len(),
            }));
        }
        let entry = self
            .entries
            .get_mut(&slot_id)
            .expect("compatible state entry remains registered");
        entry.payload = payload;
        entry.revision = revision;
        Ok(())
    }

    /// Applies one compatible snapshot to a freshly configured candidate.
    ///
    /// The complete payload is decoded before the candidate is changed. The
    /// generated [`PortableApply`] implementation then moves only retained
    /// fields, preserving callbacks, runtime handles, and configuration built
    /// by the new generation.
    pub fn restore_into<T>(
        &self,
        slot_id: StableSlotId,
        candidate: &mut T,
    ) -> Result<(), StateRegistryError>
    where
        T: AimerReflectionType + PortableApply,
    {
        let entry = self.compatible_entry::<T>(slot_id)?;
        let mut decoder = Decoder::new(&entry.payload, self.limits)?;
        let retained = T::decode_retained(&mut decoder)?;
        decoder.finish()?;
        candidate.apply_retained(retained);
        Ok(())
    }

    fn compatible_entry<T>(&self, slot_id: StableSlotId) -> Result<&StateEntry, StateRegistryError>
    where
        T: AimerReflectionType,
    {
        let entry = self.entries.get(&slot_id)
            .ok_or(StateRegistryError::UnknownSlot { slot_id })?;
        if entry.type_id != T::TYPE_ID {
            return Err(StateRegistryError::TypeMismatch {
                slot_id,
                expected: T::TYPE_ID,
                actual: entry.type_id,
            });
        }
        let expected_schema = T::schema_id();
        if entry.schema_id != expected_schema {
            return Err(StateRegistryError::SchemaMismatch {
                slot_id,
                expected: expected_schema,
                actual: entry.schema_id,
            });
        }
        Ok(entry)
    }

    fn decode_entries(
        input: &[u8],
        limits: PortableLimits,
    ) -> Result<BTreeMap<StableSlotId, StateEntry>, StateRegistryError> {
        let mut decoder = Decoder::new(input, limits)?;
        if decoder.read(STATE_MAGIC.len())? != STATE_MAGIC {
            return Err(StateRegistryError::InvalidMagic);
        }
        let count = decoder.read_length()?;
        decoder.claim_entries(count)?;
        let mut entries = BTreeMap::new();
        for _ in 0..count {
            let slot_id = read_id(&mut decoder)?;
            let type_id = read_id(&mut decoder)?;
            let schema_id = read_id(&mut decoder)?;
            let revision = u64::decode(&mut decoder)?;
            let payload = decoder.blob()?.to_vec();
            if entries.insert(slot_id, StateEntry {
                type_id,
                schema_id,
                revision,
                payload,
            }).is_some() {
                return Err(StateRegistryError::DuplicateSlot { slot_id });
            }
        }
        decoder.finish()?;
        Ok(entries)
    }
}

fn read_id(decoder: &mut Decoder<'_>) -> Result<StableId128, DecodeError> {
    let bytes: [u8; 16] = decoder.read(16)?.try_into().expect("identity width was checked");
    Ok(StableId128::from_bytes(bytes))
}

#[cfg(test)]
mod tests {
    use super::{StateRegistry, StateRegistryError};
    use super::super::codec::{
        DecodeError, Decoder, EncodeError, Encoder, LimitKind, PortableApply, PortableDecode,
        PortableEncode, PortableLimits,
    };
    use super::super::identity::StableId128;
    use super::super::schema::{AimerReflectionType, FieldDescriptor, FieldKind, TypeSchema};

    const VALUE_FIELDS: &[FieldDescriptor] = &[
        FieldDescriptor::new("value", "u32", FieldKind::Retained),
        FieldDescriptor::new("configuration", "String", FieldKind::Fresh),
    ];
    const VALUE_SCHEMA: TypeSchema = TypeSchema::new(
        "Value",
        StableId128::from_path("type", "tests::Value"),
        VALUE_FIELDS,
    );

    #[derive(Debug, PartialEq)]
    struct Value {
        value: u32,
        configuration: String,
    }

    impl AimerReflectionType for Value {
        const TYPE_ID: StableId128 = StableId128::from_path("type", "tests::Value");

        fn schema() -> &'static TypeSchema {
            &VALUE_SCHEMA
        }
    }

    impl PortableEncode for Value {
        fn encode(&self, encoder: &mut Encoder<'_>) -> Result<(), EncodeError> {
            encoder.nested(|encoder| {
                encoder.field(&VALUE_FIELDS[0], |encoder| self.value.encode(encoder))?;
                encoder.field(&VALUE_FIELDS[1], |encoder| self.configuration.encode(encoder))
            })
        }
    }

    impl PortableDecode for Value {
        fn decode(decoder: &mut Decoder<'_>) -> Result<Self, DecodeError> {
            decoder.nested(|decoder| {
                Ok(Self {
                    value: decoder.field(&VALUE_FIELDS[0])?.unwrap(),
                    configuration: decoder
                        .field(&VALUE_FIELDS[1])?
                        .unwrap_or_else(|| "fresh".into()),
                })
            })
        }
    }

    impl PortableApply for Value {
        type Retained = u32;

        fn decode_retained(decoder: &mut Decoder<'_>) -> Result<Self::Retained, DecodeError> {
            decoder.nested(|decoder| {
                let value = decoder.field(&VALUE_FIELDS[0])?.unwrap();
                let _ = decoder.field::<u8>(&VALUE_FIELDS[1])?;
                Ok(value)
            })
        }

        fn apply_retained(&mut self, retained: Self::Retained) {
            self.value = retained;
        }
    }

    fn limits() -> PortableLimits {
        PortableLimits::new(8, 4, 64, 128, 512)
    }

    fn slot(byte: u8) -> StableId128 {
        StableId128::from_bytes([byte; 16])
    }

    fn registry(value: u32, revision: u64) -> StateRegistry {
        let mut registry = StateRegistry::new(limits());
        registry
            .insert(
                slot(1),
                revision,
                &Value {
                    value,
                    configuration: "not retained".into(),
                },
            )
            .unwrap();
        registry
    }

    #[test]
    fn registry_exports_imports_and_restores_typed_state() {
        let bytes = registry(7, 3).export().unwrap();
        let mut candidate = registry(0, 2);
        candidate.import(&bytes).unwrap();
        assert_eq!(
            candidate.restore::<Value>(slot(1)).unwrap(),
            Value {
                value: 7,
                configuration: "fresh".into(),
            }
        );
        assert_eq!(candidate.revision(slot(1)), Some(3));
    }

    #[test]
    fn applying_retained_state_preserves_candidate_configuration() {
        let bytes = registry(7, 3).export().unwrap();
        let mut candidate_registry = registry(0, 2);
        candidate_registry.import(&bytes).unwrap();
        let mut candidate = Value {
            value: 0,
            configuration: "new generation".into(),
        };

        candidate_registry.restore_into(slot(1), &mut candidate).unwrap();

        assert_eq!(candidate.value, 7);
        assert_eq!(candidate.configuration, "new generation");
    }

    #[test]
    fn duplicate_slots_are_rejected_on_insert_and_import() {
        let mut registry = registry(1, 1);
        assert!(matches!(
            registry.insert(slot(1), 2, &Value { value: 2, configuration: String::new() }),
            Err(StateRegistryError::DuplicateSlot { slot_id }) if slot_id == slot(1)
        ));

        let one = registry.export().unwrap();
        let duplicated = duplicate_first_entry(&one);
        let before = registry.restore::<Value>(slot(1)).unwrap();
        assert!(matches!(
            registry.import(&duplicated),
            Err(StateRegistryError::DuplicateSlot { .. })
        ));
        assert_eq!(registry.restore::<Value>(slot(1)).unwrap(), before);
    }

    #[test]
    fn malformed_trailing_and_limit_violations_are_rejected_transactionally() {
        let valid = registry(9, 2).export().unwrap();
        let mut active = registry(1, 1);
        for invalid in [&valid[..valid.len() - 1], &[valid.as_slice(), &[0]].concat()] {
            assert!(active.import(invalid).is_err());
            assert_eq!(active.restore::<Value>(slot(1)).unwrap().value, 1);
        }

        let mut count_too_large = valid.clone();
        count_too_large[8..12].copy_from_slice(&5_u32.to_le_bytes());
        assert!(matches!(
            active.import(&count_too_large),
            Err(StateRegistryError::Decode(DecodeError::LimitExceeded {
                limit: LimitKind::Entries,
                ..
            }))
        ));
        assert_eq!(active.restore::<Value>(slot(1)).unwrap().value, 1);
    }

    #[test]
    fn incompatible_type_schema_and_revision_retain_active_contents() {
        let exported = registry(9, 3).export().unwrap();
        for offset in [28_usize, 44] {
            let mut incompatible = exported.clone();
            incompatible[offset] ^= 1;
            let mut active = registry(1, 1);
            assert!(active.import(&incompatible).is_err());
            assert_eq!(active.restore::<Value>(slot(1)).unwrap().value, 1);
        }

        let old = registry(9, 1).export().unwrap();
        let mut active = registry(1, 2);
        assert!(matches!(
            active.import(&old),
            Err(StateRegistryError::RevisionRegression { .. })
        ));
        assert_eq!(active.restore::<Value>(slot(1)).unwrap().value, 1);
    }

    #[test]
    fn unknown_imported_slot_retains_active_contents() {
        let mut exported = registry(9, 3).export().unwrap();
        exported[12..28].copy_from_slice(&slot(2).to_bytes());
        let mut active = registry(1, 1);
        assert!(matches!(
            active.import(&exported),
            Err(StateRegistryError::UnknownSlot { slot_id }) if slot_id == slot(2)
        ));
        assert_eq!(active.restore::<Value>(slot(1)).unwrap().value, 1);
    }

    fn duplicate_first_entry(bytes: &[u8]) -> Vec<u8> {
        let mut duplicated = bytes.to_vec();
        duplicated[8..12].copy_from_slice(&2_u32.to_le_bytes());
        duplicated.extend_from_slice(&bytes[12..]);
        duplicated
    }
}
