use crate::{ModelError, ModelLimits, StableId128, Version};

const MAGIC: [u8; 4] = *b"ASTA";
/// Current portable-state document format emitted and accepted by Aimer.
pub const STATE_FORMAT_VERSION: Version = Version::new(1, 0);
const FORMAT_VERSION: Version = STATE_FORMAT_VERSION;
const HEADER_LEN: usize = 48;
const ENTRY_RECORD_LEN: usize = 48;

/// The migration completeness policy for one guest-state entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StatePolicy {
    /// Reload fails rather than silently losing this entry.
    Required,
    /// Candidate migration may explicitly acknowledge resetting this entry.
    ResetSafe,
}

impl StatePolicy {
    #[inline]
    const fn wire_value(self) -> u8 {
        match self {
            Self::Required => 1,
            Self::ResetSafe => 2,
        }
    }

    fn from_wire(value: u8) -> Result<Self, ModelError> {
        match value {
            1 => Ok(Self::Required),
            2 => Ok(Self::ResetSafe),
            _ => Err(ModelError::InvalidStatePolicy { value }),
        }
    }
}

/// One versioned guest-state value with an opaque schema-owned payload.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StateEntry<'a> {
    state_id: StableId128,
    schema_id: StableId128,
    schema_version: Version,
    policy: StatePolicy,
    payload: &'a [u8],
}

impl<'a> StateEntry<'a> {
    /// Creates one state entry from stable state/schema identities.
    #[inline]
    pub const fn new(
        state_id: StableId128,
        schema_id: StableId128,
        schema_version: Version,
        policy: StatePolicy,
        payload: &'a [u8],
    ) -> Self {
        Self {
            state_id,
            schema_id,
            schema_version,
            policy,
            payload,
        }
    }
}

/// A complete canonical guest-state snapshot for one source generation.
pub struct StateBundle<'a> {
    application_id: StableId128,
    source_generation: u64,
    entries: &'a [StateEntry<'a>],
}

impl<'a> StateBundle<'a> {
    /// Creates a bundle whose entries must already be sorted by stable state ID.
    #[inline]
    pub const fn new(
        application_id: StableId128,
        source_generation: u64,
        entries: &'a [StateEntry<'a>],
    ) -> Self {
        Self {
            application_id,
            source_generation,
            entries,
        }
    }

    /// Encodes fixed entry records followed by one contiguous payload section.
    pub fn encode(&self, limits: ModelLimits) -> Result<Vec<u8>, ModelError> {
        if u32::try_from(self.entries.len()).is_err()
            || self.entries.len() > limits.max_collection_entries as usize
        {
            return Err(ModelError::CollectionTooLarge {
                count: self.entries.len(),
                limit: limits.max_collection_entries,
            });
        }
        validate_entry_order(self.entries.iter().map(|entry| entry.state_id))?;

        let mut payload_bytes_len = 0_usize;
        for entry in self.entries {
            if entry.payload.len() > limits.max_blob_bytes as usize {
                return Err(ModelError::BlobTooLarge {
                    length: entry.payload.len(),
                    limit: limits.max_blob_bytes,
                });
            }
            payload_bytes_len = payload_bytes_len
                .checked_add(entry.payload.len())
                .ok_or(ModelError::LengthOverflow)?;
        }
        if u32::try_from(payload_bytes_len).is_err() {
            return Err(ModelError::LengthOverflow);
        }
        let record_bytes = self
            .entries
            .len()
            .checked_mul(ENTRY_RECORD_LEN)
            .ok_or(ModelError::LengthOverflow)?;
        let total_len = HEADER_LEN
            .checked_add(record_bytes)
            .and_then(|length| length.checked_add(payload_bytes_len))
            .ok_or(ModelError::LengthOverflow)?;
        if u32::try_from(total_len).is_err() {
            return Err(ModelError::LengthOverflow);
        }
        if total_len > limits.max_document_bytes as usize {
            return Err(ModelError::DocumentTooLarge {
                length: total_len,
                limit: limits.max_document_bytes,
            });
        }

        let mut output = Vec::with_capacity(total_len);
        output.extend_from_slice(&MAGIC);
        write_version(&mut output, FORMAT_VERSION);
        output.extend_from_slice(self.application_id.as_bytes());
        write_u64(&mut output, self.source_generation);
        write_u32(&mut output, self.entries.len() as u32);
        write_u32(&mut output, payload_bytes_len as u32);
        write_u32(&mut output, total_len as u32);
        write_u32(&mut output, 0);

        let mut payload_start = 0_u32;
        for entry in self.entries {
            output.extend_from_slice(entry.state_id.as_bytes());
            output.extend_from_slice(entry.schema_id.as_bytes());
            write_version(&mut output, entry.schema_version);
            output.push(entry.policy.wire_value());
            output.extend_from_slice(&[0; 3]);
            write_u32(&mut output, payload_start);
            write_u32(&mut output, entry.payload.len() as u32);
            payload_start += entry.payload.len() as u32;
        }
        for entry in self.entries {
            output.extend_from_slice(entry.payload);
        }
        Ok(output)
    }
}

/// A validated, allocation-free view over one canonical guest-state bundle.
pub struct StateBundleView<'a> {
    bytes: &'a [u8],
    entry_count: u32,
    payload_start: usize,
}

#[cfg(feature = "wasm-hot-reload")]
#[derive(Clone, Copy, Debug)]
pub(crate) struct ValidatedStateBundle {
    entry_count: u32,
    payload_start: usize,
}

impl<'a> StateBundleView<'a> {
    /// Validates versioning, canonical identity order, policies, and payload ranges.
    pub fn decode(bytes: &'a [u8], limits: ModelLimits) -> Result<Self, ModelError> {
        if bytes.len() < HEADER_LEN {
            return Err(ModelError::Truncated {
                needed: HEADER_LEN,
                actual: bytes.len(),
            });
        }
        if bytes[..4] != MAGIC {
            return Err(ModelError::InvalidMagic);
        }
        let version = read_version(bytes, 4);
        if version != FORMAT_VERSION {
            return Err(ModelError::UnsupportedVersion { version });
        }
        if read_u32(bytes, 44) != 0 {
            return Err(ModelError::NonCanonicalReserved);
        }
        let entry_count = read_u32(bytes, 32);
        if entry_count > limits.max_collection_entries {
            return Err(ModelError::CollectionTooLarge {
                count: entry_count as usize,
                limit: limits.max_collection_entries,
            });
        }
        let payload_bytes_len = read_u32(bytes, 36);
        let declared_len = read_u32(bytes, 40) as usize;
        let records_len = (entry_count as usize)
            .checked_mul(ENTRY_RECORD_LEN)
            .ok_or(ModelError::LengthOverflow)?;
        let payload_start = HEADER_LEN
            .checked_add(records_len)
            .ok_or(ModelError::LengthOverflow)?;
        let expected_len = payload_start
            .checked_add(payload_bytes_len as usize)
            .ok_or(ModelError::LengthOverflow)?;
        if declared_len != bytes.len() || expected_len != bytes.len() {
            return Err(ModelError::LengthMismatch {
                declared: declared_len,
                actual: bytes.len(),
            });
        }
        if bytes.len() > limits.max_document_bytes as usize {
            return Err(ModelError::DocumentTooLarge {
                length: bytes.len(),
                limit: limits.max_document_bytes,
            });
        }

        let mut previous = None;
        let mut expected_payload_start = 0_u32;
        for index in 0..entry_count {
            let offset = HEADER_LEN + index as usize * ENTRY_RECORD_LEN;
            let state_id = read_id(bytes, offset);
            if let Some(previous) = previous {
                if state_id == previous {
                    return Err(ModelError::DuplicateStateId { state_id });
                }
                if state_id < previous {
                    return Err(ModelError::NonCanonicalStateOrder);
                }
            }
            previous = Some(state_id);
            StatePolicy::from_wire(bytes[offset + 36])?;
            if bytes[offset + 37..offset + 40] != [0; 3] {
                return Err(ModelError::NonCanonicalReserved);
            }
            let start = read_u32(bytes, offset + 40);
            let length = read_u32(bytes, offset + 44);
            if start != expected_payload_start {
                return Err(ModelError::NonCanonicalSectionLayout);
            }
            let end = start
                .checked_add(length)
                .ok_or(ModelError::LengthOverflow)?;
            if end > payload_bytes_len {
                return Err(ModelError::SectionRangeOutOfBounds);
            }
            if length > limits.max_blob_bytes {
                return Err(ModelError::BlobTooLarge {
                    length: length as usize,
                    limit: limits.max_blob_bytes,
                });
            }
            expected_payload_start = end;
        }
        if expected_payload_start != payload_bytes_len {
            return Err(ModelError::NonCanonicalSectionLayout);
        }
        Ok(Self {
            bytes,
            entry_count,
            payload_start,
        })
    }

    #[cfg(feature = "wasm-hot-reload")]
    #[inline]
    pub(crate) const fn into_validated(self) -> ValidatedStateBundle {
        ValidatedStateBundle {
            entry_count: self.entry_count,
            payload_start: self.payload_start,
        }
    }

    #[cfg(feature = "wasm-hot-reload")]
    #[inline]
    pub(crate) const fn from_validated(bytes: &'a [u8], validated: ValidatedStateBundle) -> Self {
        Self {
            bytes,
            entry_count: validated.entry_count,
            payload_start: validated.payload_start,
        }
    }

    /// Returns the stable application identity that owns this state.
    pub fn application_id(&self) -> StableId128 {
        read_id(self.bytes, 8)
    }

    /// Returns the generation that exported this bundle.
    #[inline]
    pub fn source_generation(&self) -> u64 {
        read_u64(self.bytes, 24)
    }

    /// Returns the number of fixed state-entry records.
    #[inline]
    pub const fn entry_count(&self) -> u32 {
        self.entry_count
    }

    /// Borrows one state entry by canonical table index.
    pub fn entry(&self, index: u32) -> Option<StateEntryView<'a>> {
        (index < self.entry_count).then(|| StateEntryView {
            bytes: self.bytes,
            record_offset: HEADER_LEN + index as usize * ENTRY_RECORD_LEN,
            payload_start: self.payload_start,
        })
    }

    /// Iterates canonical state entries without allocating.
    #[inline]
    pub fn entries(&self) -> StateEntries<'a> {
        StateEntries {
            bytes: self.bytes,
            payload_start: self.payload_start,
            index: 0,
            count: self.entry_count,
        }
    }
}

/// A borrowed view over one validated fixed-width state-entry record.
pub struct StateEntryView<'a> {
    bytes: &'a [u8],
    record_offset: usize,
    payload_start: usize,
}

impl<'a> StateEntryView<'a> {
    /// Returns the stable identity of the application state slot.
    pub fn state_id(&self) -> StableId128 {
        read_id(self.bytes, self.record_offset)
    }

    /// Returns the stable payload schema identity.
    pub fn schema_id(&self) -> StableId128 {
        read_id(self.bytes, self.record_offset + 16)
    }

    /// Returns the payload schema version.
    #[inline]
    pub fn schema_version(&self) -> Version {
        read_version(self.bytes, self.record_offset + 32)
    }

    /// Returns the migration completeness policy.
    pub fn policy(&self) -> StatePolicy {
        StatePolicy::from_wire(self.bytes[self.record_offset + 36]).unwrap()
    }

    /// Borrows the opaque schema-owned payload.
    pub fn payload(&self) -> &'a [u8] {
        let start = read_u32(self.bytes, self.record_offset + 40) as usize;
        let length = read_u32(self.bytes, self.record_offset + 44) as usize;
        &self.bytes[self.payload_start + start..self.payload_start + start + length]
    }
}

/// An allocation-free iterator over canonical state-entry views.
pub struct StateEntries<'a> {
    bytes: &'a [u8],
    payload_start: usize,
    index: u32,
    count: u32,
}

impl<'a> Iterator for StateEntries<'a> {
    type Item = StateEntryView<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index == self.count {
            return None;
        }
        let entry = StateEntryView {
            bytes: self.bytes,
            record_offset: HEADER_LEN + self.index as usize * ENTRY_RECORD_LEN,
            payload_start: self.payload_start,
        };
        self.index += 1;
        Some(entry)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = (self.count - self.index) as usize;
        (remaining, Some(remaining))
    }
}

impl ExactSizeIterator for StateEntries<'_> {}

fn validate_entry_order(identities: impl Iterator<Item = StableId128>) -> Result<(), ModelError> {
    let mut previous = None;
    for state_id in identities {
        if let Some(previous) = previous {
            if state_id == previous {
                return Err(ModelError::DuplicateStateId { state_id });
            }
            if state_id < previous {
                return Err(ModelError::NonCanonicalStateOrder);
            }
        }
        previous = Some(state_id);
    }
    Ok(())
}

#[inline]
fn write_u32(output: &mut Vec<u8>, value: u32) {
    output.extend_from_slice(&value.to_le_bytes());
}

#[inline]
fn write_u64(output: &mut Vec<u8>, value: u64) {
    output.extend_from_slice(&value.to_le_bytes());
}

#[inline]
fn write_version(output: &mut Vec<u8>, version: Version) {
    output.extend_from_slice(&version.major().to_le_bytes());
    output.extend_from_slice(&version.minor().to_le_bytes());
}

#[inline]
fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

#[inline]
fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}

#[inline]
fn read_version(bytes: &[u8], offset: usize) -> Version {
    Version::new(
        u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap()),
        u16::from_le_bytes(bytes[offset + 2..offset + 4].try_into().unwrap()),
    )
}

fn read_id(bytes: &[u8], offset: usize) -> StableId128 {
    let mut id = [0_u8; 16];
    id.copy_from_slice(&bytes[offset..offset + 16]);
    StableId128::from_bytes(id)
}

#[rustfmt::skip]
#[cfg(test)]
mod tests {
    use crate::{
        ModelLimits, StableId128, StateBundle, StateBundleView, StateEntry, StatePolicy, Version,
    };

    #[test]
    fn state_bundle_matches_the_version_one_golden_image_and_borrows_payload() {
        let application_id = StableId128::from_bytes([0x10; 16]);
        let state_id = StableId128::from_bytes([0x20; 16]);
        let schema_id = StableId128::from_bytes([0x30; 16]);
        let payload = [0xA5];
        let entries = [StateEntry::new(
            state_id,
            schema_id,
            Version::new(2, 1),
            StatePolicy::Required,
            &payload,
        )];
        let bundle = StateBundle::new(application_id, 7, &entries);

        let encoded = bundle
            .encode(ModelLimits::new(256, 8, 64, 64))
            .unwrap();


        assert_eq!(
            encoded,
            [
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
            ]
        );

        let view = StateBundleView::decode(
            &encoded,
            ModelLimits::new(256, 8, 64, 64),
        )
        .unwrap();
        assert_eq!(view.application_id(), application_id);
        assert_eq!(view.source_generation(), 7);
        assert_eq!(view.entry_count(), 1);
        let entry = view.entry(0).unwrap();
        assert_eq!(entry.state_id(), state_id);
        assert_eq!(entry.schema_id(), schema_id);
        assert_eq!(entry.schema_version(), Version::new(2, 1));
        assert_eq!(entry.policy(), StatePolicy::Required);
        assert_eq!(entry.payload(), payload);
    }

    #[test]
    fn state_bundle_rejects_duplicate_and_noncanonical_state_ids() {
        let application_id = StableId128::from_bytes([0x10; 16]);
        let first_id = StableId128::from_bytes([0x20; 16]);
        let second_id = StableId128::from_bytes([0x30; 16]);
        let schema_id = StableId128::from_bytes([0x40; 16]);
        let duplicate_entries = [
            StateEntry::new(
                first_id,
                schema_id,
                Version::new(1, 0),
                StatePolicy::Required,
                &[],
            ),
            StateEntry::new(
                first_id,
                schema_id,
                Version::new(1, 0),
                StatePolicy::Required,
                &[],
            ),
        ];
        assert_eq!(
            StateBundle::new(application_id, 1, &duplicate_entries)
                .encode(ModelLimits::new(256, 8, 64, 64)),
            Err(crate::ModelError::DuplicateStateId { state_id: first_id })
        );

        let reversed_entries = [
            StateEntry::new(
                second_id,
                schema_id,
                Version::new(1, 0),
                StatePolicy::ResetSafe,
                &[],
            ),
            StateEntry::new(
                first_id,
                schema_id,
                Version::new(1, 0),
                StatePolicy::Required,
                &[],
            ),
        ];
        assert_eq!(
            StateBundle::new(application_id, 1, &reversed_entries)
                .encode(ModelLimits::new(256, 8, 64, 64)),
            Err(crate::ModelError::NonCanonicalStateOrder)
        );
    }

    #[test]
    fn state_bundle_decoder_rejects_overlapping_payload_ranges() {
        let application_id = StableId128::from_bytes([0x10; 16]);
        let schema_id = StableId128::from_bytes([0x40; 16]);
        let first_payload = [1];
        let second_payload = [2];
        let entries = [
            StateEntry::new(
                StableId128::from_bytes([0x20; 16]),
                schema_id,
                Version::new(1, 0),
                StatePolicy::Required,
                &first_payload,
            ),
            StateEntry::new(
                StableId128::from_bytes([0x30; 16]),
                schema_id,
                Version::new(1, 0),
                StatePolicy::Required,
                &second_payload,
            ),
        ];
        let mut encoded = StateBundle::new(application_id, 1, &entries)
            .encode(ModelLimits::new(256, 8, 64, 64))
            .unwrap();
        encoded[136..140].copy_from_slice(&0_u32.to_le_bytes());

        assert!(matches!(
            StateBundleView::decode(&encoded, ModelLimits::new(256, 8, 64, 64)),
            Err(crate::ModelError::NonCanonicalSectionLayout)
        ));
    }
}
