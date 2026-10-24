use crate::{AbiVersion, ModelError, ModelLimits, StableId128, Version};

const MAGIC: [u8; 4] = *b"AMNF";
const FORMAT_VERSION: Version = Version::new(1, 0);
const HEADER_LEN: usize = 64;
const CAPABILITY_RECORD_LEN: usize = 56;

/// Whether a guest can run when one declared capability is unavailable.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CapabilityPolicy {
    /// Manifest negotiation fails unless the host provides an exact match.
    Required,
    /// The host may bind the standard unsupported implementation.
    Optional,
}

impl CapabilityPolicy {
    #[inline]
    const fn wire_value(self) -> u8 {
        match self {
            Self::Required => 1,
            Self::Optional => 2,
        }
    }

    fn from_wire(value: u8) -> Result<Self, ModelError> {
        match value {
            1 => Ok(Self::Required),
            2 => Ok(Self::Optional),
            _ => Err(ModelError::InvalidCapabilityPolicy { value }),
        }
    }
}

/// One exact host capability contract required by an application program.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapabilityRequirement {
    capability_id: StableId128,
    abi_major: u32,
    policy: CapabilityPolicy,
    contract_fingerprint: [u8; 32],
}

impl CapabilityRequirement {
    /// Creates one capability requirement from stable wire-contract metadata.
    #[inline]
    pub const fn new(
        capability_id: StableId128,
        abi_major: u32,
        policy: CapabilityPolicy,
        contract_fingerprint: [u8; 32],
    ) -> Self {
        Self {
            capability_id,
            abi_major,
            policy,
            contract_fingerprint,
        }
    }
}

/// A canonical description of one portable Aimer application program.
pub struct ApplicationManifest<'a> {
    minimum_abi: AbiVersion,
    maximum_abi: AbiVersion,
    widget_ir_version: Version,
    callback_event_version: Version,
    state_version: Version,
    program_id: StableId128,
    capabilities: &'a [CapabilityRequirement],
}

impl<'a> ApplicationManifest<'a> {
    /// Creates a manifest whose capabilities must be sorted by stable identity.
    #[inline]
    pub const fn new(
        minimum_abi: AbiVersion,
        maximum_abi: AbiVersion,
        widget_ir_version: Version,
        callback_event_version: Version,
        state_version: Version,
        program_id: StableId128,
        capabilities: &'a [CapabilityRequirement],
    ) -> Self {
        Self {
            minimum_abi,
            maximum_abi,
            widget_ir_version,
            callback_event_version,
            state_version,
            program_id,
            capabilities,
        }
    }

    /// Encodes the fixed header and fixed-width capability table.
    pub fn encode(&self, limits: ModelLimits) -> Result<Vec<u8>, ModelError> {
        validate_abi_range(self.minimum_abi, self.maximum_abi)?;
        validate_capability_order(
            self.capabilities
                .iter()
                .map(|capability| capability.capability_id),
        )?;
        if u32::try_from(self.capabilities.len()).is_err()
            || self.capabilities.len() > limits.max_collection_entries as usize
        {
            return Err(ModelError::CollectionTooLarge {
                count: self.capabilities.len(),
                limit: limits.max_collection_entries,
            });
        }

        let records_len = self
            .capabilities
            .len()
            .checked_mul(CAPABILITY_RECORD_LEN)
            .ok_or(ModelError::LengthOverflow)?;
        let total_len = HEADER_LEN
            .checked_add(records_len)
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
        write_abi_version(&mut output, self.minimum_abi);
        write_abi_version(&mut output, self.maximum_abi);
        write_version(&mut output, self.widget_ir_version);
        write_version(&mut output, self.callback_event_version);
        write_version(&mut output, self.state_version);
        output.extend_from_slice(self.program_id.as_bytes());
        write_u32(&mut output, self.capabilities.len() as u32);
        write_u32(&mut output, total_len as u32);
        write_u32(&mut output, 0);

        for capability in self.capabilities {
            output.extend_from_slice(capability.capability_id.as_bytes());
            write_u32(&mut output, capability.abi_major);
            output.push(capability.policy.wire_value());
            output.extend_from_slice(&[0; 3]);
            output.extend_from_slice(&capability.contract_fingerprint);
        }
        Ok(output)
    }
}

/// A validated, allocation-free view over one canonical application manifest.
pub struct ManifestView<'a> {
    bytes: &'a [u8],
    capability_count: u32,
}

#[cfg(feature = "wasm-hot-reload")]
#[derive(Clone, Copy, Debug)]
pub(crate) struct ValidatedManifest {
    capability_count: u32,
}

impl<'a> ManifestView<'a> {
    /// Validates the manifest header, ABI range, and canonical capability table.
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
        validate_abi_range(read_abi_version(bytes, 8), read_abi_version(bytes, 16))?;
        if bytes[60..64] != [0; 4] {
            return Err(ModelError::NonCanonicalReserved);
        }

        let capability_count = read_u32(bytes, 52);
        if capability_count > limits.max_collection_entries {
            return Err(ModelError::CollectionTooLarge {
                count: capability_count as usize,
                limit: limits.max_collection_entries,
            });
        }
        let records_len = (capability_count as usize)
            .checked_mul(CAPABILITY_RECORD_LEN)
            .ok_or(ModelError::LengthOverflow)?;
        let expected_len = HEADER_LEN
            .checked_add(records_len)
            .ok_or(ModelError::LengthOverflow)?;
        let declared_len = read_u32(bytes, 56) as usize;
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
        for index in 0..capability_count {
            let offset = HEADER_LEN + index as usize * CAPABILITY_RECORD_LEN;
            let capability_id = read_id(bytes, offset);
            if let Some(previous) = previous {
                if capability_id == previous {
                    return Err(ModelError::DuplicateCapabilityId { capability_id });
                }
                if capability_id < previous {
                    return Err(ModelError::NonCanonicalCapabilityOrder);
                }
            }
            previous = Some(capability_id);
            CapabilityPolicy::from_wire(bytes[offset + 20])?;
            if bytes[offset + 21..offset + 24] != [0; 3] {
                return Err(ModelError::NonCanonicalReserved);
            }
        }

        Ok(Self {
            bytes,
            capability_count,
        })
    }

    #[cfg(feature = "wasm-hot-reload")]
    #[inline]
    pub(crate) const fn into_validated(self) -> ValidatedManifest {
        ValidatedManifest {
            capability_count: self.capability_count,
        }
    }

    #[cfg(feature = "wasm-hot-reload")]
    #[inline]
    pub(crate) const fn from_validated(bytes: &'a [u8], validated: ValidatedManifest) -> Self {
        Self {
            bytes,
            capability_count: validated.capability_count,
        }
    }

    /// Returns the oldest core ABI accepted by this guest.
    #[inline]
    pub fn minimum_abi(&self) -> AbiVersion {
        read_abi_version(self.bytes, 8)
    }

    /// Returns the newest core ABI accepted by this guest.
    #[inline]
    pub fn maximum_abi(&self) -> AbiVersion {
        read_abi_version(self.bytes, 16)
    }

    /// Returns the Widget IR format emitted by this guest.
    #[inline]
    pub fn widget_ir_version(&self) -> Version {
        read_version(self.bytes, 24)
    }

    /// Returns the callback-event format consumed by this guest.
    #[inline]
    pub fn callback_event_version(&self) -> Version {
        read_version(self.bytes, 28)
    }

    /// Returns the state-bundle format consumed and emitted by this guest.
    #[inline]
    pub fn state_version(&self) -> Version {
        read_version(self.bytes, 32)
    }

    /// Returns the stable identity of the application program.
    #[inline]
    pub fn program_id(&self) -> StableId128 {
        read_id(self.bytes, 36)
    }

    /// Returns the number of canonical capability requirement records.
    #[inline]
    pub const fn capability_count(&self) -> u32 {
        self.capability_count
    }

    /// Borrows one capability requirement by canonical table index.
    pub fn capability(&self, index: u32) -> Option<CapabilityRequirementView<'a>> {
        (index < self.capability_count).then(|| CapabilityRequirementView {
            bytes: self.bytes,
            record_offset: HEADER_LEN + index as usize * CAPABILITY_RECORD_LEN,
        })
    }

    /// Iterates capability requirements in canonical stable-identity order.
    #[inline]
    pub fn capabilities(&self) -> CapabilityRequirements<'a> {
        CapabilityRequirements {
            bytes: self.bytes,
            index: 0,
            count: self.capability_count,
        }
    }
}

/// A borrowed view over one validated fixed-width capability record.
pub struct CapabilityRequirementView<'a> {
    bytes: &'a [u8],
    record_offset: usize,
}

impl<'a> CapabilityRequirementView<'a> {
    /// Returns the stable package-scoped capability identity.
    #[inline]
    pub fn capability_id(&self) -> StableId128 {
        read_id(self.bytes, self.record_offset)
    }

    /// Returns the exact capability ABI major requested by the guest.
    #[inline]
    pub fn abi_major(&self) -> u32 {
        read_u32(self.bytes, self.record_offset + 16)
    }

    /// Returns whether absence rejects the guest or binds unsupported behavior.
    #[inline]
    pub fn policy(&self) -> CapabilityPolicy {
        CapabilityPolicy::from_wire(self.bytes[self.record_offset + 20]).unwrap()
    }

    /// Returns the SHA-256 contract fingerprint carried by this requirement.
    #[inline]
    pub fn contract_fingerprint(&self) -> &'a [u8; 32] {
        self.bytes[self.record_offset + 24..self.record_offset + 56]
            .try_into()
            .unwrap()
    }
}

/// An allocation-free iterator over canonical capability requirements.
pub struct CapabilityRequirements<'a> {
    bytes: &'a [u8],
    index: u32,
    count: u32,
}

impl<'a> Iterator for CapabilityRequirements<'a> {
    type Item = CapabilityRequirementView<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let index = self.index;
        if index >= self.count {
            return None;
        }
        self.index += 1;
        Some(CapabilityRequirementView {
            bytes: self.bytes,
            record_offset: HEADER_LEN + index as usize * CAPABILITY_RECORD_LEN,
        })
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = (self.count - self.index) as usize;
        (remaining, Some(remaining))
    }
}

impl ExactSizeIterator for CapabilityRequirements<'_> {}

fn validate_abi_range(minimum: AbiVersion, maximum: AbiVersion) -> Result<(), ModelError> {
    if (minimum.major(), minimum.minor()) > (maximum.major(), maximum.minor()) {
        return Err(ModelError::InvalidAbiRange);
    }
    Ok(())
}

fn validate_capability_order(
    identities: impl Iterator<Item = StableId128>,
) -> Result<(), ModelError> {
    let mut previous = None;
    for capability_id in identities {
        if let Some(previous) = previous {
            if capability_id == previous {
                return Err(ModelError::DuplicateCapabilityId { capability_id });
            }
            if capability_id < previous {
                return Err(ModelError::NonCanonicalCapabilityOrder);
            }
        }
        previous = Some(capability_id);
    }
    Ok(())
}

#[inline]
fn write_u32(output: &mut Vec<u8>, value: u32) {
    output.extend_from_slice(&value.to_le_bytes());
}

#[inline]
fn write_version(output: &mut Vec<u8>, version: Version) {
    output.extend_from_slice(&version.major().to_le_bytes());
    output.extend_from_slice(&version.minor().to_le_bytes());
}

#[inline]
fn write_abi_version(output: &mut Vec<u8>, version: AbiVersion) {
    write_u32(output, version.major());
    write_u32(output, version.minor());
}

#[inline]
fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

#[inline]
fn read_version(bytes: &[u8], offset: usize) -> Version {
    Version::new(
        u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap()),
        u16::from_le_bytes(bytes[offset + 2..offset + 4].try_into().unwrap()),
    )
}

#[inline]
fn read_abi_version(bytes: &[u8], offset: usize) -> AbiVersion {
    AbiVersion::new(read_u32(bytes, offset), read_u32(bytes, offset + 4))
}

fn read_id(bytes: &[u8], offset: usize) -> StableId128 {
    let mut id = [0_u8; 16];
    id.copy_from_slice(&bytes[offset..offset + 16]);
    StableId128::from_bytes(id)
}
