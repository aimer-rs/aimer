use std::collections::{BTreeMap, BTreeSet, BinaryHeap, HashMap, HashSet, LinkedList, VecDeque};
use std::error::Error;
use std::fmt;

use aimer_anteros::{ValueSchemaMetadata, Version};

use super::schema::{FieldDescriptor, FieldKind};

/// Identifies a bounded portable resource.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LimitKind {
    /// Structural nesting depth.
    Depth,
    /// Aggregate collection and structural entries.
    Entries,
    /// UTF-8 bytes in one string.
    StringBytes,
    /// Bytes in one explicit blob.
    BlobBytes,
    /// Bytes in the complete encoded payload.
    PayloadBytes,
    /// Bytes in one canonical map key or set element.
    KeyBytes,
    /// Bytes in one map value or sequence element.
    ValueBytes,
    /// Bounded work units used to reconstruct collection entries.
    ReconstructionWork,
}

/// Ceilings applied before portable state allocates or writes data.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PortableLimits {
    max_depth: usize,
    max_entries: usize,
    max_string_bytes: usize,
    max_blob_bytes: usize,
    max_payload_bytes: usize,
    max_key_bytes: usize,
    max_value_bytes: usize,
    max_reconstruction_work: usize,
}

impl PortableLimits {
    /// Creates a complete set of fail-closed portable limits.
    ///
    /// Key/value budgets default to the payload budget and reconstruction work
    /// defaults to four units per entry. Callers that know the collection
    /// shape should tighten those independent ceilings with the `with_max_*`
    /// methods.
    #[inline]
    pub const fn new(
        max_depth: usize,
        max_entries: usize,
        max_string_bytes: usize,
        max_blob_bytes: usize,
        max_payload_bytes: usize,
    ) -> Self {
        Self {
            max_depth,
            max_entries,
            max_string_bytes,
            max_blob_bytes,
            max_payload_bytes,
            max_key_bytes: max_payload_bytes,
            max_value_bytes: max_payload_bytes,
            max_reconstruction_work: max_entries.saturating_mul(4),
        }
    }

    /// Creates the structural limits used by one derived value schema.
    ///
    /// The value codec reserves four bytes for its little-endian major/minor
    /// version prefix. The remaining schema budget is applied to the
    /// structural payload, strings, nested blobs, and collection elements.
    #[inline]
    pub const fn for_value(
        maximum_encoded_bytes: u32,
        max_depth: usize,
        max_entries: usize,
        max_string_bytes: usize,
    ) -> Self {
        let payload_bytes = maximum_encoded_bytes.saturating_sub(4) as usize;
        Self::new(
            max_depth,
            max_entries,
            max_string_bytes,
            payload_bytes,
            payload_bytes,
        )
    }

    /// Replaces the structural nesting ceiling.
    #[inline]
    pub const fn with_max_depth(mut self, max_depth: usize) -> Self {
        self.max_depth = max_depth;
        self
    }

    /// Replaces the aggregate entry ceiling.
    #[inline]
    pub const fn with_max_entries(mut self, max_entries: usize) -> Self {
        self.max_entries = max_entries;
        self
    }

    /// Replaces the per-string UTF-8 byte ceiling.
    #[inline]
    pub const fn with_max_string_bytes(mut self, max_string_bytes: usize) -> Self {
        self.max_string_bytes = max_string_bytes;
        self
    }

    /// Replaces the per-blob byte ceiling.
    #[inline]
    pub const fn with_max_blob_bytes(mut self, max_blob_bytes: usize) -> Self {
        self.max_blob_bytes = max_blob_bytes;
        self
    }

    /// Replaces the complete payload byte ceiling.
    #[inline]
    pub const fn with_max_payload_bytes(mut self, max_payload_bytes: usize) -> Self {
        self.max_payload_bytes = max_payload_bytes;
        self
    }

    /// Replaces the encoded-byte ceiling for one map key or set element.
    #[inline]
    pub const fn with_max_key_bytes(mut self, max_key_bytes: usize) -> Self {
        self.max_key_bytes = max_key_bytes;
        self
    }

    /// Replaces the encoded-byte ceiling for one map value or sequence element.
    #[inline]
    pub const fn with_max_value_bytes(mut self, max_value_bytes: usize) -> Self {
        self.max_value_bytes = max_value_bytes;
        self
    }

    /// Replaces the collection reconstruction-work ceiling.
    #[inline]
    pub const fn with_max_reconstruction_work(mut self, max_reconstruction_work: usize) -> Self {
        self.max_reconstruction_work = max_reconstruction_work;
        self
    }

    /// Returns the structural nesting ceiling.
    #[inline]
    pub const fn max_depth(self) -> usize { self.max_depth }
    /// Returns the aggregate entry ceiling.
    #[inline]
    pub const fn max_entries(self) -> usize { self.max_entries }
    /// Returns the per-string UTF-8 byte ceiling.
    #[inline]
    pub const fn max_string_bytes(self) -> usize { self.max_string_bytes }
    /// Returns the per-blob byte ceiling.
    #[inline]
    pub const fn max_blob_bytes(self) -> usize { self.max_blob_bytes }
    /// Returns the complete payload byte ceiling.
    #[inline]
    pub const fn max_payload_bytes(self) -> usize { self.max_payload_bytes }
    /// Returns the encoded-byte ceiling for one map key or set element.
    #[inline]
    pub const fn max_key_bytes(self) -> usize { self.max_key_bytes }
    /// Returns the encoded-byte ceiling for one map value or sequence element.
    #[inline]
    pub const fn max_value_bytes(self) -> usize { self.max_value_bytes }
    /// Returns the collection reconstruction-work ceiling.
    #[inline]
    pub const fn max_reconstruction_work(self) -> usize { self.max_reconstruction_work }
}

/// An error produced while encoding bounded portable state.
#[doc(hidden)]
#[derive(Debug, Eq, PartialEq)]
pub enum EncodeError {
    /// A configured resource ceiling would be exceeded.
    LimitExceeded { limit: LimitKind, max: usize, actual: usize },
    /// A structural length cannot be represented by the version-one format.
    LengthOverflow { actual: usize },
    /// Active state reached a source field that has no portable representation.
    UnsupportedField { field: &'static str, rust_type: &'static str },
    /// Two distinct hash keys produced the same canonical encoded key.
    CanonicalKeyCollision,
    /// A floating-point value is not finite and therefore has no AWIR value.
    NonFiniteFloat,
}

impl fmt::Display for EncodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LimitExceeded { limit, max, actual } => {
                write!(formatter, "portable {limit:?} limit exceeded: maximum {max}, got {actual}")
            }
            Self::LengthOverflow { actual } => {
                write!(formatter, "portable length {actual} exceeds the version-one wire range")
            }
            Self::UnsupportedField { field, rust_type } => write!(
                formatter,
                "active field `{field}` of type `{rust_type}` is not portable; use a hot restart or mark the field fresh"
            ),
            Self::CanonicalKeyCollision => formatter.write_str(
                "portable canonical hash collection contains keys with identical encoded forms",
            ),
            Self::NonFiniteFloat => {
                formatter.write_str("portable floating-point value is not finite")
            }
        }
    }
}

impl Error for EncodeError {}

/// An error produced while decoding bounded portable state.
#[doc(hidden)]
#[derive(Debug, Eq, PartialEq)]
pub enum DecodeError {
    /// The input ended before the requested value was complete.
    UnexpectedEnd { needed: usize, remaining: usize },
    /// A complete value left unconsumed bytes behind.
    TrailingBytes { remaining: usize },
    /// A boolean used a byte other than zero or one.
    InvalidBool(u8),
    /// An option used an unknown discriminant.
    InvalidOptionTag(u8),
    /// A result used an unknown discriminant.
    InvalidResultTag(u8),
    /// A `u32` was not a Unicode scalar value.
    InvalidChar(u32),
    /// A string payload was not UTF-8.
    InvalidUtf8,
    /// An encoded integer is outside the target's pointer-sized range.
    IntegerOutOfRange,
    /// A configured resource ceiling was exceeded.
    LimitExceeded { limit: LimitKind, max: usize, actual: usize },
    /// Active state requested a source field that has no portable representation.
    UnsupportedField { field: &'static str, rust_type: &'static str },
    /// A collection's entries were not encoded in canonical order.
    NonCanonicalOrder,
    /// A canonical collection contained a duplicate entry.
    DuplicateEntry,
    /// An enum payload used a tag that is not declared by its schema.
    InvalidEnumTag(u32),
    /// A floating-point payload is not finite and therefore has no AWIR value.
    NonFiniteFloat,
}

impl fmt::Display for DecodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedEnd { needed, remaining } => write!(
                formatter,
                "portable input ended early: needed {needed} bytes, {remaining} remain"
            ),
            Self::TrailingBytes { remaining } => {
                write!(formatter, "portable input has {remaining} trailing bytes")
            }
            Self::InvalidBool(value) => write!(formatter, "invalid portable bool byte {value}"),
            Self::InvalidOptionTag(value) => write!(formatter, "invalid portable option tag {value}"),
            Self::InvalidResultTag(value) => write!(formatter, "invalid portable result tag {value}"),
            Self::InvalidChar(value) => write!(formatter, "invalid portable char value {value:#x}"),
            Self::InvalidUtf8 => formatter.write_str("portable string is not valid UTF-8"),
            Self::IntegerOutOfRange => formatter.write_str("portable integer is out of range"),
            Self::LimitExceeded { limit, max, actual } => {
                write!(formatter, "portable {limit:?} limit exceeded: maximum {max}, got {actual}")
            }
            Self::UnsupportedField { field, rust_type } => write!(
                formatter,
                "active field `{field}` of type `{rust_type}` is not portable; use a hot restart or mark the field fresh"
            ),
            Self::NonCanonicalOrder => {
                formatter.write_str("portable ordered collection is not in canonical order")
            }
            Self::DuplicateEntry => formatter.write_str("portable collection contains a duplicate entry"),
            Self::InvalidEnumTag(value) => {
                write!(formatter, "portable enum uses unknown tag {value}")
            }
            Self::NonFiniteFloat => {
                formatter.write_str("portable floating-point value is not finite")
            }
        }
    }
}

impl Error for DecodeError {}

/// Encodes a value into the deterministic portable structural format.
#[doc(hidden)]
pub trait PortableEncode {
    /// Appends this value to `encoder` without an intermediate payload copy.
    fn encode(&self, encoder: &mut Encoder<'_>) -> Result<(), EncodeError>;
}

/// Decodes an owned value from the deterministic portable structural format.
#[doc(hidden)]
pub trait PortableDecode: Sized {
    /// Reads one value from `decoder`.
    fn decode(decoder: &mut Decoder<'_>) -> Result<Self, DecodeError>;
}

/// Failure while encoding or decoding a derived, versioned portable value.
#[derive(Debug, Eq, PartialEq)]
pub enum PortableValueError {
    /// The declared schema budget cannot contain the four-byte wire header.
    InvalidSchemaLimit { maximum_encoded_bytes: u32 },
    /// The caller supplied a schema version that this value does not support.
    UnsupportedVersion { expected: Version, actual: Version },
    /// The payload's explicit wire version disagrees with the schema version.
    InvalidWireVersion { expected: Version, actual: Version },
    /// The structural encoder rejected the value or one of its limits.
    Encode(EncodeError),
    /// The structural decoder rejected the payload or one of its limits.
    Decode(DecodeError),
}

impl fmt::Display for PortableValueError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSchemaLimit { maximum_encoded_bytes } => write!(
                formatter,
                "portable value schema limit {maximum_encoded_bytes} is smaller than its wire header",
            ),
            Self::UnsupportedVersion { expected, actual } => write!(
                formatter,
                "portable value requires schema version {}.{}, got {}.{}",
                expected.major(),
                expected.minor(),
                actual.major(),
                actual.minor(),
            ),
            Self::InvalidWireVersion { expected, actual } => write!(
                formatter,
                "portable value payload declares wire version {}.{}, expected {}.{}",
                actual.major(),
                actual.minor(),
                expected.major(),
                expected.minor(),
            ),
            Self::Encode(error) => write!(formatter, "portable value encode failed: {error}"),
            Self::Decode(error) => write!(formatter, "portable value decode failed: {error}"),
        }
    }
}

impl Error for PortableValueError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Encode(error) => Some(error),
            Self::Decode(error) => Some(error),
            _ => None,
        }
    }
}

impl From<EncodeError> for PortableValueError {
    #[inline]
    fn from(error: EncodeError) -> Self {
        Self::Encode(error)
    }
}

impl From<DecodeError> for PortableValueError {
    #[inline]
    fn from(error: DecodeError) -> Self {
        Self::Decode(error)
    }
}

/// Static field metadata emitted by [`PortableValue`] derives.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PortableValueField {
    name: &'static str,
    order: u32,
}

impl PortableValueField {
    /// Creates one canonical field descriptor.
    #[doc(hidden)]
    #[inline]
    pub const fn new(name: &'static str, order: u32) -> Self {
        Self { name, order }
    }

    /// Returns the stable field name.
    #[inline]
    pub const fn name(self) -> &'static str {
        self.name
    }

    /// Returns the canonical field order.
    #[inline]
    pub const fn order(self) -> u32 {
        self.order
    }
}

/// Static enum-variant metadata emitted by [`PortableValue`] derives.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PortableValueVariant {
    name: &'static str,
    tag: u32,
}

impl PortableValueVariant {
    /// Creates one canonical variant descriptor.
    #[doc(hidden)]
    #[inline]
    pub const fn new(name: &'static str, tag: u32) -> Self {
        Self { name, tag }
    }

    /// Returns the stable variant name.
    #[inline]
    pub const fn name(self) -> &'static str {
        self.name
    }

    /// Returns the explicit wire tag.
    #[inline]
    pub const fn tag(self) -> u32 {
        self.tag
    }
}

const VALUE_WIRE_HEADER_BYTES: usize = 4;

/// A deterministic, bounded, versioned value contract.
///
/// `PortableValue` values are encoded into one owned BLOBREF payload. The
/// payload starts with little-endian `u16` major and minor version fields,
/// followed by the structural [`PortableEncode`] representation. Derives also
/// implement [`PortableProperty`], [`PortableEncodeProperty`] on portable
/// guests, and [`PortableMaterializeProperty`] on native hosts.
pub trait PortableValue: PortableEncode + PortableDecode + Sized {
    /// Stable value identity and schema version used by AWIR.
    const SCHEMA: ValueSchemaMetadata<'static>;
    /// Alias retained for callers that name the value schema explicitly.
    const VALUE_SCHEMA: ValueSchemaMetadata<'static> = Self::SCHEMA;
    /// Canonical field metadata, in wire order.
    const FIELDS: &'static [PortableValueField] = &[];
    /// Canonical enum metadata, in stable tag order.
    const VARIANTS: &'static [PortableValueVariant] = &[];
    /// Maximum structural nesting depth for this value.
    const MAX_DEPTH: usize = 32;
    /// Maximum aggregate collection/structural entries for this value.
    const MAX_ENTRIES: usize = 4_096;
    /// Maximum UTF-8 string bytes in this value.
    const MAX_STRING_BYTES: usize = 4_096;
    /// Maximum encoded bytes in one map key or set element.
    const MAX_KEY_BYTES: usize = 4_096;
    /// Maximum encoded bytes in one map value or sequence element.
    const MAX_VALUE_BYTES: usize = 4_096;
    /// Maximum collection reconstruction work for this value.
    const MAX_RECONSTRUCTION_WORK: usize = 16_384;

    /// Returns the stable value schema.
    #[inline]
    fn schema() -> ValueSchemaMetadata<'static> {
        Self::SCHEMA
    }

    /// Encodes the value with its version header and schema limits.
    fn encode_value(&self) -> Result<Vec<u8>, PortableValueError> {
        let maximum = Self::SCHEMA.maximum_encoded_bytes();
        if (maximum as usize) < VALUE_WIRE_HEADER_BYTES {
            return Err(PortableValueError::InvalidSchemaLimit {
                maximum_encoded_bytes: maximum,
            });
        }
        let version = Self::SCHEMA.version();
        let mut output = Vec::new();
        output.extend_from_slice(&version.major().to_le_bytes());
        output.extend_from_slice(&version.minor().to_le_bytes());
        let limits = PortableLimits::for_value(
            maximum,
            Self::MAX_DEPTH,
            Self::MAX_ENTRIES,
            Self::MAX_STRING_BYTES,
        )
        .with_max_key_bytes(Self::MAX_KEY_BYTES)
        .with_max_value_bytes(Self::MAX_VALUE_BYTES)
        .with_max_reconstruction_work(Self::MAX_RECONSTRUCTION_WORK);
        Self::encode(self, &mut Encoder::new(&mut output, limits))?;
        if output.len() > maximum as usize {
            return Err(PortableValueError::Encode(EncodeError::LimitExceeded {
                limit: LimitKind::PayloadBytes,
                max: maximum as usize,
                actual: output.len(),
            }));
        }
        Ok(output)
    }

    /// Decodes one complete payload after checking its external schema version.
    fn decode_value(bytes: &[u8], version: Version) -> Result<Self, PortableValueError> {
        let expected = Self::SCHEMA.version();
        if version != expected {
            return Err(PortableValueError::UnsupportedVersion {
                expected,
                actual: version,
            });
        }
        let maximum = Self::SCHEMA.maximum_encoded_bytes();
        if (maximum as usize) < VALUE_WIRE_HEADER_BYTES {
            return Err(PortableValueError::InvalidSchemaLimit {
                maximum_encoded_bytes: maximum,
            });
        }
        if bytes.len() > maximum as usize {
            return Err(PortableValueError::Decode(DecodeError::LimitExceeded {
                limit: LimitKind::PayloadBytes,
                max: maximum as usize,
                actual: bytes.len(),
            }));
        }
        if bytes.len() < VALUE_WIRE_HEADER_BYTES {
            return Err(PortableValueError::Decode(DecodeError::UnexpectedEnd {
                needed: VALUE_WIRE_HEADER_BYTES,
                remaining: bytes.len(),
            }));
        }
        let wire_version = Version::new(
            u16::from_le_bytes([bytes[0], bytes[1]]),
            u16::from_le_bytes([bytes[2], bytes[3]]),
        );
        if wire_version != expected {
            return Err(PortableValueError::InvalidWireVersion {
                expected,
                actual: wire_version,
            });
        }
        let limits = PortableLimits::for_value(
            maximum,
            Self::MAX_DEPTH,
            Self::MAX_ENTRIES,
            Self::MAX_STRING_BYTES,
        )
        .with_max_key_bytes(Self::MAX_KEY_BYTES)
        .with_max_value_bytes(Self::MAX_VALUE_BYTES)
        .with_max_reconstruction_work(Self::MAX_RECONSTRUCTION_WORK);
        let mut decoder = Decoder::new(&bytes[VALUE_WIRE_HEADER_BYTES..], limits)?;
        let value = Self::decode(&mut decoder)?;
        decoder.finish()?;
        Ok(value)
    }

    /// Alias for callers that prefer an explicitly portable method name.
    #[inline]
    fn encode_portable_value(&self) -> Result<Vec<u8>, PortableValueError> {
        self.encode_value()
    }

    /// Alias for callers that prefer an explicitly portable method name.
    #[inline]
    fn decode_portable_value(bytes: &[u8], version: Version) -> Result<Self, PortableValueError> {
        Self::decode_value(bytes, version)
    }
}

/// Decodes and applies retained state to a freshly configured value.
///
/// Generated implementations decode retained fields into `Retained` without
/// touching the candidate. The registry validates the complete payload before
/// calling `apply_retained`, so malformed state cannot partially modify the
/// candidate and fresh fields remain those built by the new generation.
#[doc(hidden)]
pub trait PortableApply {
    /// Temporary owned image containing only retained fields.
    type Retained;

    /// Decodes retained fields and validates omitted or unsupported fields.
    fn decode_retained(decoder: &mut Decoder<'_>) -> Result<Self::Retained, DecodeError>;

    /// Moves a validated retained image into the candidate value.
    fn apply_retained(&mut self, retained: Self::Retained);
}

/// A bounded encoder that writes directly into caller-owned storage.
#[doc(hidden)]
pub struct Encoder<'a> {
    output: &'a mut Vec<u8>,
    start: usize,
    limits: PortableLimits,
    depth: usize,
    entries: usize,
    reconstruction_work: usize,
}

impl<'a> Encoder<'a> {
    /// Creates an encoder that appends to `output`.
    #[inline]
    pub fn new(output: &'a mut Vec<u8>, limits: PortableLimits) -> Self {
        let start = output.len();
        Self {
            output,
            start,
            limits,
            depth: 0,
            entries: 0,
            reconstruction_work: 0,
        }
    }

    /// Encodes a generated field according to its static retention kind.
    ///
    /// Fresh fields do not evaluate `encode`; unsupported fields return a
    /// secret-free diagnostic naming only source metadata.
    pub fn field<F>(&mut self, field: &FieldDescriptor, encode: F) -> Result<(), EncodeError>
    where
        F: FnOnce(&mut Self) -> Result<(), EncodeError>,
    {
        match field.kind() {
            FieldKind::Retained => encode(self),
            FieldKind::Fresh => Ok(()),
            FieldKind::Unsupported => Err(EncodeError::UnsupportedField {
                field: field.name(),
                rust_type: field.rust_type(),
            }),
        }
    }

    /// Encodes one generated or structural nested value.
    pub fn nested<T, F>(&mut self, encode: F) -> Result<T, EncodeError>
    where
        F: FnOnce(&mut Self) -> Result<T, EncodeError>,
    {
        let actual = self.depth.saturating_add(1);
        if actual > self.limits.max_depth {
            return Err(EncodeError::LimitExceeded {
                limit: LimitKind::Depth,
                max: self.limits.max_depth,
                actual,
            });
        }
        self.depth = actual;
        let result = encode(self);
        self.depth -= 1;
        result
    }

    /// Writes a length-prefixed opaque blob.
    pub fn blob(&mut self, bytes: &[u8]) -> Result<(), EncodeError> {
        self.check_limit(LimitKind::BlobBytes, self.limits.max_blob_bytes, bytes.len())?;
        self.write_length(bytes.len())?;
        self.write(bytes)
    }

    #[doc(hidden)]
    pub fn claim_entries(&mut self, amount: usize) -> Result<(), EncodeError> {
        let actual = self.entries.saturating_add(amount);
        self.check_limit(LimitKind::Entries, self.limits.max_entries, actual)?;
        self.entries = actual;
        Ok(())
    }

    fn claim_reconstruction_work(&mut self, amount: usize) -> Result<(), EncodeError> {
        let actual = self.reconstruction_work.saturating_add(amount);
        self.check_limit(
            LimitKind::ReconstructionWork,
            self.limits.max_reconstruction_work,
            actual,
        )?;
        self.reconstruction_work = actual;
        Ok(())
    }

    fn scoped<T, F>(
        &mut self,
        limit: LimitKind,
        max: usize,
        encode: F,
    ) -> Result<T, EncodeError>
    where
        F: FnOnce(&mut Self) -> Result<T, EncodeError>,
    {
        let start = self.output.len();
        let value = encode(self)?;
        let actual = self.output.len().saturating_sub(start);
        self.check_limit(limit, max, actual)?;
        Ok(value)
    }

    pub(crate) fn write_length(&mut self, length: usize) -> Result<(), EncodeError> {
        let length = u32::try_from(length).map_err(|_| EncodeError::LengthOverflow { actual: length })?;
        self.write(&length.to_le_bytes())
    }

    pub(crate) fn write(&mut self, bytes: &[u8]) -> Result<(), EncodeError> {
        let written = self.output.len().saturating_sub(self.start);
        let actual = written.saturating_add(bytes.len());
        self.check_limit(LimitKind::PayloadBytes, self.limits.max_payload_bytes, actual)?;
        self.output.extend_from_slice(bytes);
        Ok(())
    }

    fn check_limit(&self, limit: LimitKind, max: usize, actual: usize) -> Result<(), EncodeError> {
        if actual > max {
            Err(EncodeError::LimitExceeded { limit, max, actual })
        } else {
            Ok(())
        }
    }

    fn limits_for_nested_value(&self) -> PortableLimits {
        let written = self.output.len().saturating_sub(self.start);
        PortableLimits {
            max_depth: self.limits.max_depth.saturating_sub(self.depth),
            max_entries: self.limits.max_entries.saturating_sub(self.entries),
            max_string_bytes: self.limits.max_string_bytes,
            max_blob_bytes: self.limits.max_blob_bytes,
            max_payload_bytes: self.limits.max_payload_bytes.saturating_sub(written),
            max_key_bytes: self.limits.max_key_bytes,
            max_value_bytes: self.limits.max_value_bytes,
            max_reconstruction_work: self.limits.max_reconstruction_work,
        }
    }
}

/// A bounded decoder borrowing its input payload.
#[doc(hidden)]
pub struct Decoder<'a> {
    input: &'a [u8],
    cursor: usize,
    limits: PortableLimits,
    depth: usize,
    entries: usize,
    reconstruction_work: usize,
}

impl<'a> Decoder<'a> {
    /// Creates a decoder after validating the complete payload byte ceiling.
    #[inline]
    pub fn new(input: &'a [u8], limits: PortableLimits) -> Result<Self, DecodeError> {
        if input.len() > limits.max_payload_bytes {
            return Err(DecodeError::LimitExceeded {
                limit: LimitKind::PayloadBytes,
                max: limits.max_payload_bytes,
                actual: input.len(),
            });
        }
        Ok(Self {
            input,
            cursor: 0,
            limits,
            depth: 0,
            entries: 0,
            reconstruction_work: 0,
        })
    }

    /// Decodes a generated field, returning `None` for fresh configuration.
    pub fn field<T: PortableDecode>(
        &mut self,
        field: &FieldDescriptor,
    ) -> Result<Option<T>, DecodeError> {
        match field.kind() {
            FieldKind::Retained => T::decode(self).map(Some),
            FieldKind::Fresh => Ok(None),
            FieldKind::Unsupported => Err(DecodeError::UnsupportedField {
                field: field.name(),
                rust_type: field.rust_type(),
            }),
        }
    }

    /// Decodes one generated or structural nested value.
    pub fn nested<T, F>(&mut self, decode: F) -> Result<T, DecodeError>
    where
        F: FnOnce(&mut Self) -> Result<T, DecodeError>,
    {
        let actual = self.depth.saturating_add(1);
        if actual > self.limits.max_depth {
            return Err(DecodeError::LimitExceeded {
                limit: LimitKind::Depth,
                max: self.limits.max_depth,
                actual,
            });
        }
        self.depth = actual;
        let result = decode(self);
        self.depth -= 1;
        result
    }

    /// Borrows one length-prefixed opaque blob without copying it.
    pub fn blob(&mut self) -> Result<&'a [u8], DecodeError> {
        let length = self.read_length()?;
        self.check_limit(LimitKind::BlobBytes, self.limits.max_blob_bytes, length)?;
        self.read(length)
    }

    /// Rejects any bytes left after the expected value.
    pub fn finish(self) -> Result<(), DecodeError> {
        let remaining = self.input.len() - self.cursor;
        if remaining == 0 { Ok(()) } else { Err(DecodeError::TrailingBytes { remaining }) }
    }

    #[doc(hidden)]
    pub fn claim_entries(&mut self, amount: usize) -> Result<(), DecodeError> {
        let actual = self.entries.saturating_add(amount);
        self.check_limit(LimitKind::Entries, self.limits.max_entries, actual)?;
        self.entries = actual;
        Ok(())
    }

    fn scoped<T, F>(&mut self, limit: LimitKind, max: usize, decode: F) -> Result<T, DecodeError>
    where
        F: FnOnce(&mut Self) -> Result<T, DecodeError>,
    {
        let start = self.cursor;
        let value = decode(self)?;
        let actual = self.cursor.saturating_sub(start);
        self.check_limit(limit, max, actual)?;
        Ok(value)
    }

    fn claim_reconstruction_work(&mut self, amount: usize) -> Result<(), DecodeError> {
        let actual = self
            .reconstruction_work()
            .saturating_add(amount);
        self.check_limit(
            LimitKind::ReconstructionWork,
            self.limits.max_reconstruction_work,
            actual,
        )?;
        self.set_reconstruction_work(actual);
        Ok(())
    }

    #[inline]
    fn reconstruction_work(&self) -> usize {
        self.reconstruction_work
    }

    #[inline]
    fn set_reconstruction_work(&mut self, value: usize) {
        self.reconstruction_work = value;
    }

    pub(crate) fn read_length(&mut self) -> Result<usize, DecodeError> {
        let bytes: [u8; 4] = self.read(4)?.try_into().expect("length was checked");
        Ok(u32::from_le_bytes(bytes) as usize)
    }

    pub(crate) fn read(&mut self, length: usize) -> Result<&'a [u8], DecodeError> {
        let remaining = self.input.len() - self.cursor;
        if length > remaining {
            return Err(DecodeError::UnexpectedEnd { needed: length, remaining });
        }
        let start = self.cursor;
        self.cursor += length;
        Ok(&self.input[start..self.cursor])
    }

    fn check_limit(&self, limit: LimitKind, max: usize, actual: usize) -> Result<(), DecodeError> {
        if actual > max {
            Err(DecodeError::LimitExceeded { limit, max, actual })
        } else {
            Ok(())
        }
    }
}

/// Encodes one value into newly allocated bounded storage.
pub fn encode_to_vec<T: PortableEncode + ?Sized>(
    value: &T,
    limits: PortableLimits,
) -> Result<Vec<u8>, EncodeError> {
    let mut output = Vec::new();
    value.encode(&mut Encoder::new(&mut output, limits))?;
    Ok(output)
}

/// Decodes exactly one value and rejects trailing input.
pub fn decode_from_slice<T: PortableDecode>(
    input: &[u8],
    limits: PortableLimits,
) -> Result<T, DecodeError> {
    let mut decoder = Decoder::new(input, limits)?;
    let value = T::decode(&mut decoder)?;
    decoder.finish()?;
    Ok(value)
}

macro_rules! fixed_codec {
    ($($type:ty),+ $(,)?) => {$(
        impl PortableEncode for $type {
            #[inline]
            fn encode(&self, encoder: &mut Encoder<'_>) -> Result<(), EncodeError> {
                encoder.write(&self.to_le_bytes())
            }
        }

        impl PortableDecode for $type {
            #[inline]
            fn decode(decoder: &mut Decoder<'_>) -> Result<Self, DecodeError> {
                let bytes = decoder.read(std::mem::size_of::<Self>())?;
                Ok(Self::from_le_bytes(bytes.try_into().expect("fixed width was checked")))
            }
        }
    )+};
}

fixed_codec!(u8, u16, u32, u64, u128, i8, i16, i32, i64, i128);

macro_rules! finite_float_codec {
    ($type:ty) => {
        impl PortableEncode for $type {
            #[inline]
            fn encode(&self, encoder: &mut Encoder<'_>) -> Result<(), EncodeError> {
                if !self.is_finite() {
                    return Err(EncodeError::NonFiniteFloat);
                }
                encoder.write(&self.to_le_bytes())
            }
        }

        impl PortableDecode for $type {
            #[inline]
            fn decode(decoder: &mut Decoder<'_>) -> Result<Self, DecodeError> {
                let bytes = decoder.read(std::mem::size_of::<Self>())?;
                let value = Self::from_le_bytes(bytes.try_into().expect("fixed width was checked"));
                if value.is_finite() {
                    Ok(value)
                } else {
                    Err(DecodeError::NonFiniteFloat)
                }
            }
        }
    };
}

finite_float_codec!(f32);
finite_float_codec!(f64);

impl PortableEncode for bool {
    #[inline]
    fn encode(&self, encoder: &mut Encoder<'_>) -> Result<(), EncodeError> {
        encoder.write(&[u8::from(*self)])
    }
}

impl PortableDecode for bool {
    fn decode(decoder: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        match u8::decode(decoder)? {
            0 => Ok(false),
            1 => Ok(true),
            value => Err(DecodeError::InvalidBool(value)),
        }
    }
}

impl PortableEncode for char {
    #[inline]
    fn encode(&self, encoder: &mut Encoder<'_>) -> Result<(), EncodeError> {
        (*self as u32).encode(encoder)
    }
}

impl PortableDecode for char {
    fn decode(decoder: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        let value = u32::decode(decoder)?;
        char::from_u32(value).ok_or(DecodeError::InvalidChar(value))
    }
}

impl PortableEncode for usize {
    #[inline]
    fn encode(&self, encoder: &mut Encoder<'_>) -> Result<(), EncodeError> {
        (*self as u64).encode(encoder)
    }
}

impl PortableDecode for usize {
    fn decode(decoder: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        usize::try_from(u64::decode(decoder)?).map_err(|_| DecodeError::IntegerOutOfRange)
    }
}

impl PortableEncode for isize {
    #[inline]
    fn encode(&self, encoder: &mut Encoder<'_>) -> Result<(), EncodeError> {
        (*self as i64).encode(encoder)
    }
}

impl PortableDecode for isize {
    fn decode(decoder: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        isize::try_from(i64::decode(decoder)?).map_err(|_| DecodeError::IntegerOutOfRange)
    }
}

impl PortableEncode for str {
    fn encode(&self, encoder: &mut Encoder<'_>) -> Result<(), EncodeError> {
        encoder.check_limit(LimitKind::StringBytes, encoder.limits.max_string_bytes, self.len())?;
        encoder.write_length(self.len())?;
        encoder.write(self.as_bytes())
    }
}

impl PortableEncode for String {
    #[inline]
    fn encode(&self, encoder: &mut Encoder<'_>) -> Result<(), EncodeError> {
        self.as_str().encode(encoder)
    }
}

impl PortableDecode for String {
    fn decode(decoder: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        let length = decoder.read_length()?;
        decoder.check_limit(LimitKind::StringBytes, decoder.limits.max_string_bytes, length)?;
        let bytes = decoder.read(length)?;
        let value = std::str::from_utf8(bytes).map_err(|_| DecodeError::InvalidUtf8)?;
        Ok(value.to_owned())
    }
}

impl<T: PortableEncode> PortableEncode for Option<T> {
    fn encode(&self, encoder: &mut Encoder<'_>) -> Result<(), EncodeError> {
        encoder.nested(|encoder| match self {
            None => 0_u8.encode(encoder),
            Some(value) => {
                encoder.claim_entries(1)?;
                1_u8.encode(encoder)?;
                value.encode(encoder)
            }
        })
    }
}

impl<T: PortableDecode> PortableDecode for Option<T> {
    fn decode(decoder: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        decoder.nested(|decoder| match u8::decode(decoder)? {
            0 => Ok(None),
            1 => {
                decoder.claim_entries(1)?;
                T::decode(decoder).map(Some)
            }
            value => Err(DecodeError::InvalidOptionTag(value)),
        })
    }
}

/// Encodes `Ok` with tag `0` and `Err` with tag `1` inside one bounded nested
/// value. The explicit tag keeps the two result arms unambiguous while using
/// the same depth and entry accounting as [`Option`].
impl<T: PortableEncode, E: PortableEncode> PortableEncode for Result<T, E> {
    fn encode(&self, encoder: &mut Encoder<'_>) -> Result<(), EncodeError> {
        encoder.nested(|encoder| match self {
            Ok(value) => {
                encoder.claim_entries(1)?;
                0_u8.encode(encoder)?;
                value.encode(encoder)
            }
            Err(error) => {
                encoder.claim_entries(1)?;
                1_u8.encode(encoder)?;
                error.encode(encoder)
            }
        })
    }
}

impl<T: PortableDecode, E: PortableDecode> PortableDecode for Result<T, E> {
    fn decode(decoder: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        decoder.nested(|decoder| match u8::decode(decoder)? {
            0 => {
                decoder.claim_entries(1)?;
                T::decode(decoder).map(Ok)
            }
            1 => {
                decoder.claim_entries(1)?;
                E::decode(decoder).map(Err)
            }
            value => Err(DecodeError::InvalidResultTag(value)),
        })
    }
}

impl<T: PortableEncode> PortableEncode for Vec<T> {
    fn encode(&self, encoder: &mut Encoder<'_>) -> Result<(), EncodeError> {
        encoder.nested(|encoder| {
            encoder.claim_entries(self.len())?;
            encoder.claim_reconstruction_work(self.len())?;
            encoder.write_length(self.len())?;
            let max_value_bytes = encoder.limits.max_value_bytes;
            for value in self {
                encoder.scoped(LimitKind::ValueBytes, max_value_bytes, |encoder| {
                    value.encode(encoder)
                })?;
            }
            Ok(())
        })
    }
}

impl<T: PortableDecode> PortableDecode for Vec<T> {
    fn decode(decoder: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        decoder.nested(|decoder| {
            let length = decoder.read_length()?;
            decoder.claim_entries(length)?;
            decoder.claim_reconstruction_work(length)?;
            let mut values = Vec::with_capacity(length);
            for _ in 0..length {
                let value = decoder.scoped(LimitKind::ValueBytes, decoder.limits.max_value_bytes, T::decode)?;
                values.push(value);
            }
            Ok(values)
        })
    }
}

impl<T: PortableEncode + ?Sized> PortableEncode for Box<T> {
    fn encode(&self, encoder: &mut Encoder<'_>) -> Result<(), EncodeError> {
        encoder.nested(|encoder| (**self).encode(encoder))
    }
}

impl<T: PortableDecode> PortableDecode for Box<T> {
    fn decode(decoder: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        decoder.nested(|decoder| T::decode(decoder).map(Box::new))
    }
}

impl<T: PortableEncode, const N: usize> PortableEncode for [T; N] {
    fn encode(&self, encoder: &mut Encoder<'_>) -> Result<(), EncodeError> {
        encoder.nested(|encoder| {
            encoder.claim_entries(N)?;
            encoder.claim_reconstruction_work(N)?;
            let max_value_bytes = encoder.limits.max_value_bytes;
            for value in self {
                encoder.scoped(LimitKind::ValueBytes, max_value_bytes, |encoder| {
                    value.encode(encoder)
                })?;
            }
            Ok(())
        })
    }
}

impl<T: PortableDecode, const N: usize> PortableDecode for [T; N] {
    fn decode(decoder: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        decoder.nested(|decoder| {
            decoder.claim_entries(N)?;
            decoder.claim_reconstruction_work(N)?;
            let mut values = Vec::with_capacity(N);
            for _ in 0..N {
                let max_value_bytes = decoder.limits.max_value_bytes;
                let value = decoder.scoped(LimitKind::ValueBytes, max_value_bytes, T::decode)?;
                values.push(value);
            }
            values.try_into().map_err(|_| unreachable!("array length is fixed"))
        })
    }
}

impl<T: PortableEncode> PortableEncode for VecDeque<T> {
    fn encode(&self, encoder: &mut Encoder<'_>) -> Result<(), EncodeError> {
        encoder.nested(|encoder| {
            encoder.claim_entries(self.len())?;
            encoder.claim_reconstruction_work(self.len())?;
            encoder.write_length(self.len())?;
            let max_value_bytes = encoder.limits.max_value_bytes;
            for value in self {
                encoder.scoped(LimitKind::ValueBytes, max_value_bytes, |encoder| {
                    value.encode(encoder)
                })?;
            }
            Ok(())
        })
    }
}

impl<T: PortableDecode> PortableDecode for VecDeque<T> {
    fn decode(decoder: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        decoder.nested(|decoder| {
            let length = decoder.read_length()?;
            decoder.claim_entries(length)?;
            decoder.claim_reconstruction_work(length)?;
            let mut values = VecDeque::with_capacity(length);
            for _ in 0..length {
                let value = decoder.scoped(LimitKind::ValueBytes, decoder.limits.max_value_bytes, T::decode)?;
                values.push_back(value);
            }
            Ok(values)
        })
    }
}

impl<T: PortableEncode> PortableEncode for LinkedList<T> {
    fn encode(&self, encoder: &mut Encoder<'_>) -> Result<(), EncodeError> {
        encoder.nested(|encoder| {
            encoder.claim_entries(self.len())?;
            encoder.claim_reconstruction_work(self.len())?;
            encoder.write_length(self.len())?;
            let max_value_bytes = encoder.limits.max_value_bytes;
            for value in self {
                encoder.scoped(LimitKind::ValueBytes, max_value_bytes, |encoder| {
                    value.encode(encoder)
                })?;
            }
            Ok(())
        })
    }
}

impl<T: PortableDecode> PortableDecode for LinkedList<T> {
    fn decode(decoder: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        decoder.nested(|decoder| {
            let length = decoder.read_length()?;
            decoder.claim_entries(length)?;
            decoder.claim_reconstruction_work(length)?;
            let mut values = LinkedList::new();
            for _ in 0..length {
                let value = decoder.scoped(LimitKind::ValueBytes, decoder.limits.max_value_bytes, T::decode)?;
                values.push_back(value);
            }
            Ok(values)
        })
    }
}

impl<K: Ord + PortableEncode, V: PortableEncode> PortableEncode for BTreeMap<K, V> {
    fn encode(&self, encoder: &mut Encoder<'_>) -> Result<(), EncodeError> {
        encoder.nested(|encoder| {
            encoder.claim_entries(self.len())?;
            encoder.claim_reconstruction_work(self.len())?;
            encoder.write_length(self.len())?;
            let max_key_bytes = encoder.limits.max_key_bytes;
            let max_value_bytes = encoder.limits.max_value_bytes;
            for (key, value) in self {
                encoder.scoped(LimitKind::KeyBytes, max_key_bytes, |encoder| key.encode(encoder))?;
                encoder.scoped(LimitKind::ValueBytes, max_value_bytes, |encoder| value.encode(encoder))?;
            }
            Ok(())
        })
    }
}

impl<K: Ord + PortableDecode, V: PortableDecode> PortableDecode for BTreeMap<K, V> {
    fn decode(decoder: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        decoder.nested(|decoder| {
            let length = decoder.read_length()?;
            decoder.claim_entries(length)?;
            decoder.claim_reconstruction_work(length)?;
            let mut values = BTreeMap::new();
            for _ in 0..length {
                let max_key_bytes = decoder.limits.max_key_bytes;
                let key = decoder.scoped(LimitKind::KeyBytes, max_key_bytes, K::decode)?;
                if values
                    .keys()
                    .next_back()
                    .is_some_and(|previous| previous >= &key)
                {
                    return Err(DecodeError::NonCanonicalOrder);
                }
                let max_value_bytes = decoder.limits.max_value_bytes;
                let value = decoder.scoped(LimitKind::ValueBytes, max_value_bytes, V::decode)?;
                if values.insert(key, value).is_some() {
                    return Err(DecodeError::DuplicateEntry);
                }
            }
            Ok(values)
        })
    }
}

impl<T: Ord + PortableEncode> PortableEncode for BTreeSet<T> {
    fn encode(&self, encoder: &mut Encoder<'_>) -> Result<(), EncodeError> {
        encoder.nested(|encoder| {
            encoder.claim_entries(self.len())?;
            encoder.write_length(self.len())?;
            let max_key_bytes = encoder.limits.max_key_bytes;
            for value in self {
                encoder.scoped(LimitKind::KeyBytes, max_key_bytes, |encoder| {
                    value.encode(encoder)
                })?;
            }
            Ok(())
        })
    }
}

impl<T: Ord + PortableDecode> PortableDecode for BTreeSet<T> {
    fn decode(decoder: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        decoder.nested(|decoder| {
            let length = decoder.read_length()?;
            decoder.claim_entries(length)?;
            decoder.claim_reconstruction_work(length)?;
            let mut values = BTreeSet::new();
            for _ in 0..length {
                let max_key_bytes = decoder.limits.max_key_bytes;
                let value = decoder.scoped(LimitKind::KeyBytes, max_key_bytes, T::decode)?;
                if values.last().is_some_and(|previous| previous >= &value) {
                    return Err(DecodeError::NonCanonicalOrder);
                }
                if !values.insert(value) {
                    return Err(DecodeError::DuplicateEntry);
                }
            }
            Ok(values)
        })
    }
}

/// Encodes a binary heap as its ascending `into_sorted_vec` sequence.
///
/// This sequence is independent of the heap's internal array layout; decoding
/// inserts each bounded element back into a fresh heap.
impl<T: Clone + Ord + PortableEncode> PortableEncode for BinaryHeap<T> {
    fn encode(&self, encoder: &mut Encoder<'_>) -> Result<(), EncodeError> {
        encoder.nested(|encoder| {
            let values = self.clone().into_sorted_vec();
            encoder.claim_entries(values.len())?;
            encoder.claim_reconstruction_work(values.len())?;
            encoder.write_length(values.len())?;
            let max_value_bytes = encoder.limits.max_value_bytes;
            for value in values {
                encoder.scoped(LimitKind::ValueBytes, max_value_bytes, |encoder| {
                    value.encode(encoder)
                })?;
            }
            Ok(())
        })
    }
}

impl<T: Ord + PortableDecode> PortableDecode for BinaryHeap<T> {
    fn decode(decoder: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        decoder.nested(|decoder| {
            let length = decoder.read_length()?;
            decoder.claim_entries(length)?;
            decoder.claim_reconstruction_work(length)?;
            let mut sorted = Vec::with_capacity(length);
            for _ in 0..length {
                let max_value_bytes = decoder.limits.max_value_bytes;
                let value = decoder.scoped(LimitKind::ValueBytes, max_value_bytes, T::decode)?;
                sorted.push(value);
            }
            if sorted.windows(2).any(|pair| pair[0] >= pair[1]) {
                return Err(DecodeError::NonCanonicalOrder);
            }
            let mut values = BinaryHeap::with_capacity(length);
            for value in sorted {
                values.push(value);
            }
            Ok(values)
        })
    }
}

/// A hash map with an explicit canonical key-order wire adapter.
///
/// Keys are sorted by their encoded deterministic bytes before serialization.
/// Equal encoded keys are rejected rather than resolved by hash-table order,
/// making a key codec change an explicit compatibility decision. A value that
/// changes its key encoding must advance its enclosing schema version rather
/// than relying on raw hash-table iteration or silently accepting collisions.
#[derive(Clone, Debug)]
pub struct CanonicalHashMap<K, V>(HashMap<K, V>);

impl<K: Eq + std::hash::Hash, V: PartialEq> PartialEq for CanonicalHashMap<K, V> {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl<K: Eq + std::hash::Hash, V: Eq> Eq for CanonicalHashMap<K, V> {}

impl<K, V> CanonicalHashMap<K, V> {
    /// Wraps a hash map in the canonical-order adapter.
    #[inline]
    pub fn new(values: HashMap<K, V>) -> Self {
        Self(values)
    }

    /// Returns the wrapped map by value.
    #[inline]
    pub fn into_inner(self) -> HashMap<K, V> {
        self.0
    }

    /// Borrows the wrapped map.
    #[inline]
    pub fn as_inner(&self) -> &HashMap<K, V> {
        &self.0
    }
}

impl<K, V> From<HashMap<K, V>> for CanonicalHashMap<K, V> {
    #[inline]
    fn from(values: HashMap<K, V>) -> Self {
        Self::new(values)
    }
}

impl<K, V> PortableEncode for CanonicalHashMap<K, V>
where
    K: Eq + std::hash::Hash + PortableEncode,
    V: PortableEncode,
{
    fn encode(&self, encoder: &mut Encoder<'_>) -> Result<(), EncodeError> {
        encoder.nested(|encoder| {
            encoder.claim_entries(self.0.len())?;
            encoder.claim_reconstruction_work(self.0.len())?;
            let mut entries = Vec::with_capacity(self.0.len());
            for (key, value) in &self.0 {
                let mut encoded_key = Vec::new();
                key.encode(&mut Encoder::new(
                    &mut encoded_key,
                    encoder.limits_for_nested_value(),
                ))?;
                encoder.check_limit(
                    LimitKind::KeyBytes,
                    encoder.limits.max_key_bytes,
                    encoded_key.len(),
                )?;
                entries.push((encoded_key, key, value));
            }
            entries.sort_unstable_by(|left, right| left.0.cmp(&right.0));
            if entries.windows(2).any(|pair| pair[0].0 == pair[1].0) {
                return Err(EncodeError::CanonicalKeyCollision);
            }
            encoder.write_length(entries.len())?;
            let max_key_bytes = encoder.limits.max_key_bytes;
            let max_value_bytes = encoder.limits.max_value_bytes;
            for (_, key, value) in entries {
                encoder.scoped(LimitKind::KeyBytes, max_key_bytes, |encoder| key.encode(encoder))?;
                encoder.scoped(LimitKind::ValueBytes, max_value_bytes, |encoder| value.encode(encoder))?;
            }
            Ok(())
        })
    }
}

impl<K, V> PortableDecode for CanonicalHashMap<K, V>
where
    K: Eq + std::hash::Hash + PortableDecode,
    V: PortableDecode,
{
    fn decode(decoder: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        decoder.nested(|decoder| {
            let length = decoder.read_length()?;
            decoder.claim_entries(length)?;
            decoder.claim_reconstruction_work(length)?;
            let mut values = HashMap::with_capacity(length);
            let mut previous = None;
            for _ in 0..length {
                let key_start = decoder.cursor;
                let max_key_bytes = decoder.limits.max_key_bytes;
                let key = decoder.scoped(LimitKind::KeyBytes, max_key_bytes, K::decode)?;
                let key_end = decoder.cursor;
                if let Some((previous_start, previous_end)) = previous
                    && decoder.input[previous_start..previous_end]
                        >= decoder.input[key_start..key_end]
                    {
                        return Err(DecodeError::NonCanonicalOrder);
                    }
                let max_value_bytes = decoder.limits.max_value_bytes;
                let value = decoder.scoped(LimitKind::ValueBytes, max_value_bytes, V::decode)?;
                if values.insert(key, value).is_some() {
                    return Err(DecodeError::DuplicateEntry);
                }
                previous = Some((key_start, key_end));
            }
            Ok(Self(values))
        })
    }
}

/// A hash set with an explicit canonical element-order wire adapter.
///
/// Elements are sorted by their encoded bytes, duplicate encodings are
/// rejected, and decoders reject input that is not strictly ordered.
#[derive(Clone, Debug)]
pub struct CanonicalHashSet<T>(HashSet<T>);

impl<T: Eq + std::hash::Hash> PartialEq for CanonicalHashSet<T> {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl<T: Eq + std::hash::Hash> Eq for CanonicalHashSet<T> {}

impl<T> CanonicalHashSet<T> {
    /// Wraps a hash set in the canonical-order adapter.
    #[inline]
    pub fn new(values: HashSet<T>) -> Self {
        Self(values)
    }

    /// Returns the wrapped set by value.
    #[inline]
    pub fn into_inner(self) -> HashSet<T> {
        self.0
    }

    /// Borrows the wrapped set.
    #[inline]
    pub fn as_inner(&self) -> &HashSet<T> {
        &self.0
    }
}

impl<T> From<HashSet<T>> for CanonicalHashSet<T> {
    #[inline]
    fn from(values: HashSet<T>) -> Self {
        Self::new(values)
    }
}

impl<T> PortableEncode for CanonicalHashSet<T>
where
    T: Eq + std::hash::Hash + PortableEncode,
{
    fn encode(&self, encoder: &mut Encoder<'_>) -> Result<(), EncodeError> {
        encoder.nested(|encoder| {
            encoder.claim_entries(self.0.len())?;
            encoder.claim_reconstruction_work(self.0.len())?;
            let mut entries = Vec::with_capacity(self.0.len());
            for value in &self.0 {
                let mut encoded = Vec::new();
                value.encode(&mut Encoder::new(
                    &mut encoded,
                    encoder.limits_for_nested_value(),
                ))?;
                encoder.check_limit(
                    LimitKind::KeyBytes,
                    encoder.limits.max_key_bytes,
                    encoded.len(),
                )?;
                entries.push((encoded, value));
            }
            entries.sort_unstable_by(|left, right| left.0.cmp(&right.0));
            if entries.windows(2).any(|pair| pair[0].0 == pair[1].0) {
                return Err(EncodeError::CanonicalKeyCollision);
            }
            encoder.write_length(entries.len())?;
            let max_key_bytes = encoder.limits.max_key_bytes;
            for (_, value) in entries {
                encoder.scoped(LimitKind::KeyBytes, max_key_bytes, |encoder| {
                    value.encode(encoder)
                })?;
            }
            Ok(())
        })
    }
}

impl<T> PortableDecode for CanonicalHashSet<T>
where
    T: Eq + std::hash::Hash + PortableDecode,
{
    fn decode(decoder: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        decoder.nested(|decoder| {
            let length = decoder.read_length()?;
            decoder.claim_entries(length)?;
            decoder.claim_reconstruction_work(length)?;
            let mut values = HashSet::with_capacity(length);
            let mut previous = None;
            for _ in 0..length {
                let entry_start = decoder.cursor;
                let max_key_bytes = decoder.limits.max_key_bytes;
                let value = decoder.scoped(LimitKind::KeyBytes, max_key_bytes, T::decode)?;
                let entry_end = decoder.cursor;
                if let Some((previous_start, previous_end)) = previous
                    && decoder.input[previous_start..previous_end]
                        >= decoder.input[entry_start..entry_end]
                    {
                        return Err(DecodeError::NonCanonicalOrder);
                    }
                if !values.insert(value) {
                    return Err(DecodeError::DuplicateEntry);
                }
                previous = Some((entry_start, entry_end));
            }
            Ok(Self(values))
        })
    }
}

macro_rules! tuple_codec {
    ($count:expr; $($type:ident:$index:tt),+ $(,)?) => {
        impl<$($type: PortableEncode),+> PortableEncode for ($($type,)+) {
            fn encode(&self, encoder: &mut Encoder<'_>) -> Result<(), EncodeError> {
                encoder.nested(|encoder| {
                    encoder.claim_entries($count)?;
                    encoder.claim_reconstruction_work($count)?;
                    let max_value_bytes = encoder.limits.max_value_bytes;
                    $(encoder.scoped(LimitKind::ValueBytes, max_value_bytes, |encoder| {
                        self.$index.encode(encoder)
                    })?;)+
                    Ok(())
                })
            }
        }

        impl<$($type: PortableDecode),+> PortableDecode for ($($type,)+) {
            fn decode(decoder: &mut Decoder<'_>) -> Result<Self, DecodeError> {
                decoder.nested(|decoder| {
                    decoder.claim_entries($count)?;
                    decoder.claim_reconstruction_work($count)?;
                    let max_value_bytes = decoder.limits.max_value_bytes;
                    Ok(($(
                        decoder.scoped(LimitKind::ValueBytes, max_value_bytes, $type::decode)?,
                    )+))
                })
            }
        }
    };
}

tuple_codec!(1; A:0);
tuple_codec!(2; A:0, B:1);
tuple_codec!(3; A:0, B:1, C:2);
tuple_codec!(4; A:0, B:1, C:2, D:3);
tuple_codec!(5; A:0, B:1, C:2, D:3, E:4);
tuple_codec!(6; A:0, B:1, C:2, D:3, E:4, F:5);
tuple_codec!(7; A:0, B:1, C:2, D:3, E:4, F:5, G:6);
tuple_codec!(8; A:0, B:1, C:2, D:3, E:4, F:5, G:6, H:7);
tuple_codec!(9; A:0, B:1, C:2, D:3, E:4, F:5, G:6, H:7, I:8);
tuple_codec!(10; A:0, B:1, C:2, D:3, E:4, F:5, G:6, H:7, I:8, J:9);
tuple_codec!(11; A:0, B:1, C:2, D:3, E:4, F:5, G:6, H:7, I:8, J:9, K:10);
tuple_codec!(12; A:0, B:1, C:2, D:3, E:4, F:5, G:6, H:7, I:8, J:9, K:10, L:11);

#[cfg(test)]
mod tests {
    use super::{
        CanonicalHashMap, CanonicalHashSet, DecodeError, Decoder, EncodeError, Encoder, LimitKind,
        PortableDecode, PortableEncode, PortableLimits, decode_from_slice, encode_to_vec,
    };
    use super::super::identity::StableId128;
    use super::super::schema::{AimerReflectionType, FieldDescriptor, FieldKind, TypeSchema};
    use std::collections::{BinaryHeap, HashMap, HashSet};

    #[derive(Hash, PartialEq, Eq)]
    struct CollidingKey(u8);

    impl PortableEncode for CollidingKey {
        fn encode(&self, encoder: &mut Encoder<'_>) -> Result<(), EncodeError> {
            0_u8.encode(encoder)
        }
    }

    #[derive(Hash, PartialEq, Eq)]
    struct AliasKey;

    impl PortableEncode for AliasKey {
        fn encode(&self, encoder: &mut Encoder<'_>) -> Result<(), EncodeError> {
            0_u8.encode(encoder)
        }
    }

    impl PortableDecode for AliasKey {
        fn decode(decoder: &mut Decoder<'_>) -> Result<Self, DecodeError> {
            let _ = u8::decode(decoder)?;
            Ok(Self)
        }
    }

    const NESTED_FIELDS: &[FieldDescriptor] = &[
        FieldDescriptor::new("enabled", "bool", FieldKind::Retained),
        FieldDescriptor::new("labels", "Vec<String>", FieldKind::Retained),
    ];
    const NESTED_SCHEMA: TypeSchema = TypeSchema::new(
        "Nested",
        StableId128::from_path("type", "tests::Nested"),
        NESTED_FIELDS,
    );

    #[derive(Debug, PartialEq)]
    struct Nested {
        enabled: bool,
        labels: Vec<String>,
    }

    impl AimerReflectionType for Nested {
        const TYPE_ID: StableId128 = StableId128::from_path("type", "tests::Nested");

        fn schema() -> &'static TypeSchema {
            &NESTED_SCHEMA
        }
    }

    impl PortableEncode for Nested {
        fn encode(&self, encoder: &mut Encoder<'_>) -> Result<(), EncodeError> {
            encoder.nested(|encoder| {
                encoder.field(&NESTED_FIELDS[0], |encoder| self.enabled.encode(encoder))?;
                encoder.field(&NESTED_FIELDS[1], |encoder| self.labels.encode(encoder))
            })
        }
    }

    impl PortableDecode for Nested {
        fn decode(decoder: &mut Decoder<'_>) -> Result<Self, DecodeError> {
            decoder.nested(|decoder| {
                Ok(Self {
                    enabled: decoder.field(&NESTED_FIELDS[0])?.unwrap_or(false),
                    labels: decoder.field(&NESTED_FIELDS[1])?.unwrap_or_default(),
                })
            })
        }
    }

    fn limits() -> PortableLimits {
        PortableLimits::new(8, 64, 64, 64, 1_024)
    }

    fn round_trip<T>(value: T)
    where
        T: PortableEncode + PortableDecode + PartialEq + std::fmt::Debug,
    {
        let bytes = encode_to_vec(&value, limits()).unwrap();
        assert_eq!(decode_from_slice::<T>(&bytes, limits()).unwrap(), value);
    }

    #[test]
    fn scalar_string_container_and_nested_values_round_trip() {
        round_trip((true, -7_i64, 9_u128, 2.5_f32, -8.25_f64, '🦀'));
        round_trip("portable".to_owned());
        round_trip(Some(vec![Box::new([1_u16, 2, 3]), Box::new([4, 5, 6])]));
        round_trip(Nested {
            enabled: true,
            labels: vec!["one".into(), "two".into()],
        });
    }

    #[test]
    fn unordered_collections_require_and_preserve_explicit_canonical_order() {
        let mut first_map = HashMap::new();
        first_map.insert("b".to_owned(), 2_u8);
        first_map.insert("a".to_owned(), 1_u8);
        let mut second_map = HashMap::new();
        second_map.insert("a".to_owned(), 1_u8);
        second_map.insert("b".to_owned(), 2_u8);
        let first = encode_to_vec(&CanonicalHashMap::new(first_map.clone()), limits()).unwrap();
        let second = encode_to_vec(&CanonicalHashMap::new(second_map), limits()).unwrap();
        assert_eq!(first, second);
        assert_eq!(
            decode_from_slice::<CanonicalHashMap<String, u8>>(&first, limits())
                .unwrap()
                .into_inner(),
            first_map,
        );

        let mut values = HashSet::new();
        values.insert(4_u8);
        values.insert(1_u8);
        let encoded = encode_to_vec(&CanonicalHashSet::new(values.clone()), limits()).unwrap();
        assert_eq!(
            decode_from_slice::<CanonicalHashSet<u8>>(&encoded, limits())
                .unwrap()
                .into_inner(),
            values,
        );
        assert!(matches!(
            decode_from_slice::<CanonicalHashSet<u8>>(&[2, 0, 0, 0, 4, 1], limits()),
            Err(DecodeError::NonCanonicalOrder)
        ));
        assert!(matches!(
            decode_from_slice::<BinaryHeap<u8>>(&[3, 0, 0, 0, 2, 1, 3], limits()),
            Err(DecodeError::NonCanonicalOrder)
        ));

        let mut colliding = HashMap::new();
        colliding.insert(CollidingKey(1), 1_u8);
        colliding.insert(CollidingKey(2), 2_u8);
        assert!(matches!(
            encode_to_vec(&CanonicalHashMap::new(colliding), limits()),
            Err(EncodeError::CanonicalKeyCollision)
        ));
        let mut colliding_set = HashSet::new();
        colliding_set.insert(CollidingKey(1));
        colliding_set.insert(CollidingKey(2));
        assert!(matches!(
            encode_to_vec(&CanonicalHashSet::new(colliding_set), limits()),
            Err(EncodeError::CanonicalKeyCollision)
        ));
    }

    #[test]
    fn collection_wire_fixture_is_explicit_and_target_independent() {
        assert_eq!(
            encode_to_vec(&Result::<u8, String>::Ok(7), limits()).unwrap(),
            vec![0, 7],
        );
        assert_eq!(
            encode_to_vec(&Result::<u8, String>::Err("x".to_owned()), limits()).unwrap(),
            vec![1, 1, 0, 0, 0, b'x'],
        );

        let mut map = HashMap::new();
        map.insert("b".to_owned(), 2_u8);
        map.insert("a".to_owned(), 1_u8);
        assert_eq!(
            encode_to_vec(&CanonicalHashMap::new(map), limits()).unwrap(),
            vec![
                2, 0, 0, 0,
                1, 0, 0, 0, b'a', 1,
                1, 0, 0, 0, b'b', 2,
            ],
        );

        let set = HashSet::from([2_u8, 1]);
        assert_eq!(
            encode_to_vec(&CanonicalHashSet::new(set), limits()).unwrap(),
            vec![2, 0, 0, 0, 1, 2],
        );
        assert_eq!(
            encode_to_vec(
                &CanonicalHashMap::<u8, u8>::new(HashMap::new()),
                limits(),
            )
            .unwrap(),
            vec![0, 0, 0, 0],
        );
        assert_eq!(
            encode_to_vec(&CanonicalHashSet::<u8>::new(HashSet::new()), limits()).unwrap(),
            vec![0, 0, 0, 0],
        );
        assert_eq!(encode_to_vec(&Vec::<u8>::new(), limits()).unwrap(), vec![0, 0, 0, 0]);
        assert_eq!(encode_to_vec(&BinaryHeap::<u8>::new(), limits()).unwrap(), vec![0, 0, 0, 0]);
    }

    #[test]
    fn independent_collection_limits_bound_key_value_and_reconstruction_work() {
        let mut map = HashMap::new();
        map.insert("key".to_owned(), 7_u8);
        let map = CanonicalHashMap::new(map);

        assert!(matches!(
            encode_to_vec(&map, limits().with_max_key_bytes(6)),
            Err(EncodeError::LimitExceeded { limit: LimitKind::KeyBytes, .. })
        ));
        assert!(matches!(
            encode_to_vec(&map, limits().with_max_value_bytes(0)),
            Err(EncodeError::LimitExceeded { limit: LimitKind::ValueBytes, .. })
        ));
        assert!(matches!(
            encode_to_vec(&map, limits().with_max_reconstruction_work(0)),
            Err(EncodeError::LimitExceeded {
                limit: LimitKind::ReconstructionWork,
                ..
            })
        ));

        let bytes = encode_to_vec(&map, limits()).unwrap();
        assert!(matches!(
            decode_from_slice::<CanonicalHashMap<String, u8>>(
                &bytes,
                limits().with_max_key_bytes(6),
            ),
            Err(DecodeError::LimitExceeded { limit: LimitKind::KeyBytes, .. })
        ));
        assert!(matches!(
            decode_from_slice::<CanonicalHashMap<String, u8>>(
                &bytes,
                limits().with_max_value_bytes(0),
            ),
            Err(DecodeError::LimitExceeded { limit: LimitKind::ValueBytes, .. })
        ));

        assert!(matches!(
            decode_from_slice::<CanonicalHashMap<String, u8>>(
                &bytes,
                limits().with_max_reconstruction_work(0),
            ),
            Err(DecodeError::LimitExceeded {
                limit: LimitKind::ReconstructionWork,
                ..
            })
        ));
    }

    #[test]
    fn malformed_and_trailing_input_are_rejected() {
        let mut output = Vec::new();
        assert!(matches!(
            f32::NAN.encode(&mut Encoder::new(&mut output, limits())),
            Err(EncodeError::NonFiniteFloat)
        ));
        assert!(matches!(
            decode_from_slice::<bool>(&[2], limits()),
            Err(DecodeError::InvalidBool(2))
        ));
        assert!(matches!(
            decode_from_slice::<u64>(&[1, 2], limits()),
            Err(DecodeError::UnexpectedEnd { .. })
        ));
        assert!(matches!(
            decode_from_slice::<bool>(&[1, 0], limits()),
            Err(DecodeError::TrailingBytes { remaining: 1 })
        ));
        assert!(matches!(
            decode_from_slice::<char>(&0xd800_u32.to_le_bytes(), limits()),
            Err(DecodeError::InvalidChar(0xd800))
        ));
        assert!(matches!(
            decode_from_slice::<Option<u8>>(&[2], limits()),
            Err(DecodeError::InvalidOptionTag(2))
        ));
        assert!(matches!(
            decode_from_slice::<Result<u8, u8>>(&[2], limits()),
            Err(DecodeError::InvalidResultTag(2))
        ));
        assert!(matches!(
            decode_from_slice::<Result<u8, u8>>(&[0], limits()),
            Err(DecodeError::UnexpectedEnd { .. })
        ));
        assert!(matches!(
            decode_from_slice::<String>(&[1, 0, 0, 0, 0xff], limits()),
            Err(DecodeError::InvalidUtf8)
        ));
        assert!(matches!(
            decode_from_slice::<f64>(&f64::INFINITY.to_le_bytes(), limits()),
            Err(DecodeError::NonFiniteFloat)
        ));
    }

    #[test]
    fn canonical_hash_decoders_reject_malformed_order_and_duplicate_values() {
        let noncanonical_map = [
            2, 0, 0, 0,
            1, 0, 0, 0, b'b', 2,
            1, 0, 0, 0, b'a', 1,
        ];
        assert!(matches!(
            decode_from_slice::<CanonicalHashMap<String, u8>>(&noncanonical_map, limits()),
            Err(DecodeError::NonCanonicalOrder)
        ));
        assert!(matches!(
            decode_from_slice::<CanonicalHashMap<String, u8>>(
                &[1, 0, 0, 0, 1, 0, 0, 0],
                limits(),
            ),
            Err(DecodeError::UnexpectedEnd { .. })
        ));

        let aliased_map = [2, 0, 0, 0, 1, 1, 2, 2];
        assert!(matches!(
            decode_from_slice::<CanonicalHashMap<AliasKey, u8>>(&aliased_map, limits()),
            Err(DecodeError::DuplicateEntry)
        ));
        let aliased_set = [2, 0, 0, 0, 1, 2];
        assert!(matches!(
            decode_from_slice::<CanonicalHashSet<AliasKey>>(&aliased_set, limits()),
            Err(DecodeError::DuplicateEntry)
        ));
    }

    #[test]
    fn every_codec_limit_is_enforced() {
        let depth = PortableLimits::new(0, 64, 64, 64, 1_024);
        assert_limit(encode_to_vec(&Some(1_u8), depth), LimitKind::Depth);

        let entries = PortableLimits::new(8, 1, 64, 64, 1_024);
        assert_limit(encode_to_vec(&vec![1_u8, 2], entries), LimitKind::Entries);

        let strings = PortableLimits::new(8, 64, 2, 64, 1_024);
        assert_limit(encode_to_vec(&"abc".to_owned(), strings), LimitKind::StringBytes);

        let blobs = PortableLimits::new(8, 64, 64, 2, 1_024);
        let mut bytes = Vec::new();
        let mut encoder = Encoder::new(&mut bytes, blobs);
        assert_limit(encoder.blob(&[1, 2, 3]), LimitKind::BlobBytes);

        let payload = PortableLimits::new(8, 64, 64, 64, 2);
        assert_limit(encode_to_vec(&1_u64, payload), LimitKind::PayloadBytes);
    }

    fn assert_limit<T>(result: Result<T, EncodeError>, expected: LimitKind) {
        assert!(matches!(
            result,
            Err(EncodeError::LimitExceeded { limit, .. }) if limit == expected
        ));
    }

    #[test]
    fn decoder_enforces_advertised_lengths_before_allocating() {
        let limits = PortableLimits::new(8, 1, 2, 2, 64);
        let three = 3_u32.to_le_bytes();
        assert!(matches!(
            decode_from_slice::<String>(&three, limits),
            Err(DecodeError::LimitExceeded { limit: LimitKind::StringBytes, .. })
        ));
        assert!(matches!(
            decode_from_slice::<Vec<u8>>(&three, limits),
            Err(DecodeError::LimitExceeded { limit: LimitKind::Entries, .. })
        ));
        let mut decoder = Decoder::new(&three, limits).unwrap();
        assert!(matches!(
            decoder.blob(),
            Err(DecodeError::LimitExceeded { limit: LimitKind::BlobBytes, .. })
        ));

        let depth = PortableLimits::new(0, 64, 64, 64, 64);
        assert!(matches!(
            decode_from_slice::<Option<u8>>(&[0], depth),
            Err(DecodeError::LimitExceeded { limit: LimitKind::Depth, .. })
        ));
        let payload = PortableLimits::new(8, 64, 64, 64, 0);
        assert!(matches!(
            decode_from_slice::<bool>(&[1], payload),
            Err(DecodeError::LimitExceeded { limit: LimitKind::PayloadBytes, .. })
        ));
    }

    const SEPARATED_FIELDS: &[FieldDescriptor] = &[
        FieldDescriptor::new("retained", "u32", FieldKind::Retained),
        FieldDescriptor::new("configuration", "String", FieldKind::Fresh),
    ];

    #[test]
    fn generated_code_can_separate_retained_state_from_fresh_configuration() {
        let mut bytes = Vec::new();
        let mut encoder = Encoder::new(&mut bytes, limits());
        encoder
            .field(&SEPARATED_FIELDS[0], |encoder| 42_u32.encode(encoder))
            .unwrap();
        encoder
            .field(&SEPARATED_FIELDS[1], |encoder| "secret config".to_owned().encode(encoder))
            .unwrap();
        assert_eq!(bytes.len(), 4);

        let mut decoder = Decoder::new(&bytes, limits()).unwrap();
        let retained: u32 = decoder.field(&SEPARATED_FIELDS[0]).unwrap().unwrap();
        let configuration: Option<String> = decoder.field(&SEPARATED_FIELDS[1]).unwrap();
        decoder.finish().unwrap();
        assert_eq!(retained, 42);
        assert_eq!(configuration, None);
    }

    #[test]
    fn active_unsupported_field_has_precise_secret_free_diagnostic() {
        let field = FieldDescriptor::new("socket", "NativeSocket", FieldKind::Unsupported);
        let mut bytes = Vec::new();
        let mut encoder = Encoder::new(&mut bytes, limits());
        let error = encoder.field(&field, |_| Ok(())).unwrap_err();
        let message = error.to_string();
        assert!(message.contains("socket"));
        assert!(message.contains("NativeSocket"));
        assert!(message.contains("hot restart"));
        assert!(!message.contains("payload"));
        assert!(bytes.is_empty());
    }
}
