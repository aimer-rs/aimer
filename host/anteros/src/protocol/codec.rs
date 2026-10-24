use std::collections::HashSet;
use std::error::Error;
use std::fmt;

use crate::identity::StableId128;

const ENVELOPE_HEADER_LEN: usize = 24;

/// Resource ceilings applied before a canonical encoder grows its output.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EncodeLimits {
    max_document_bytes: u32,
    max_collection_entries: u32,
    max_string_bytes: u32,
}

impl EncodeLimits {
    /// Creates explicit document, collection, and UTF-8 string ceilings.
    ///
    /// The encoder validates these limits before modifying its output, so a
    /// rejected value never leaves a partial field in the canonical document.
    #[inline]
    pub const fn new(
        max_document_bytes: u32,
        max_collection_entries: u32,
        max_string_bytes: u32,
    ) -> Self {
        Self {
            max_document_bytes,
            max_collection_entries,
            max_string_bytes,
        }
    }
}

/// Resource ceilings applied before a canonical decoder borrows or allocates data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DecodeLimits {
    max_payload_bytes: u32,
    max_collection_entries: u32,
    max_string_bytes: u32,
}

impl DecodeLimits {
    /// Creates explicit payload, collection, and UTF-8 string ceilings.
    ///
    /// Limits are part of the caller's trust policy rather than values supplied
    /// by the encoded document. A decoder checks them before borrowing a
    /// variable-length region or allocating collection storage.
    #[inline]
    pub const fn new(
        max_payload_bytes: u32,
        max_collection_entries: u32,
        max_string_bytes: u32,
    ) -> Self {
        Self {
            max_payload_bytes,
            max_collection_entries,
            max_string_bytes,
        }
    }
}

/// A major/minor version carried by a canonical binary document.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Version {
    major: u16,
    minor: u16,
}

impl Version {
    /// Creates an explicit schema version.
    #[inline]
    pub const fn new(major: u16, minor: u16) -> Self {
        Self { major, minor }
    }

    /// Returns the compatibility-breaking version component.
    #[inline]
    pub const fn major(self) -> u16 {
        self.major
    }

    /// Returns the backward-compatible version component.
    #[inline]
    pub const fn minor(self) -> u16 {
        self.minor
    }
}

/// A borrowed payload and the metadata required by Aimer's binary envelope.
#[derive(Debug, Eq, PartialEq)]
pub struct Envelope<'a> {
    magic: [u8; 4],
    version: Version,
    message_kind: u16,
    flags: u16,
    request_id: u64,
    payload: &'a [u8],
}

/// Failure while encoding a canonical application-core document.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EncodeError {
    /// The payload cannot be represented by the envelope's 32-bit length.
    PayloadTooLarge { length: usize },
    /// A canonical document would exceed the caller's configured ceiling.
    DocumentTooLarge { length: usize, limit: u32 },
    /// A UTF-8 string exceeds the format or caller's configured ceiling.
    StringTooLarge { length: usize, limit: u32 },
    /// A fixed-width collection declared a zero-byte entry width.
    InvalidEntryWidth,
    /// Fixed-width entry bytes do not divide into complete entries.
    CollectionLengthMismatch {
        byte_length: usize,
        entry_width: u32,
    },
    /// A collection exceeds the format or caller's configured ceiling.
    CollectionTooLarge { count: usize, limit: u32 },
    /// A uniqueness-constrained collection contains the same identity twice.
    DuplicateId { id: [u8; 16] },
    /// Checked output-length arithmetic could not represent the requested value.
    LengthOverflow,
}

impl fmt::Display for EncodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PayloadTooLarge { length } => {
                write!(
                    formatter,
                    "payload length {length} exceeds the envelope limit"
                )
            }
            Self::DocumentTooLarge { length, limit } => {
                write!(formatter, "document length {length} exceeds limit {limit}")
            }
            Self::StringTooLarge { length, limit } => {
                write!(formatter, "string length {length} exceeds limit {limit}")
            }
            Self::InvalidEntryWidth => {
                formatter.write_str("fixed collection entry width must be nonzero")
            }
            Self::CollectionLengthMismatch {
                byte_length,
                entry_width,
            } => write!(
                formatter,
                "collection byte length {byte_length} is not divisible by entry width {entry_width}"
            ),
            Self::CollectionTooLarge { count, limit } => {
                write!(formatter, "collection count {count} exceeds limit {limit}")
            }
            Self::DuplicateId { id } => write!(formatter, "duplicate stable ID {id:02x?}"),
            Self::LengthOverflow => formatter.write_str("encoded length arithmetic overflowed"),
        }
    }
}

impl Error for EncodeError {}

/// Failure while decoding a canonical application-core document.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecodeError {
    /// The input ended before a declared fixed-width or length-delimited value.
    Truncated {
        offset: usize,
        needed: usize,
        remaining: usize,
    },
    /// Checked arithmetic could not represent a requested byte range.
    LengthOverflow,
    /// A declared envelope payload exceeds the caller's configured ceiling.
    PayloadTooLarge { length: u32, limit: u32 },
    /// A declared collection count exceeds the caller's configured ceiling.
    CollectionTooLarge { count: u32, limit: u32 },
    /// A declared UTF-8 byte length exceeds the caller's configured ceiling.
    StringTooLarge { length: u32, limit: u32 },
    /// A length-delimited string is not valid UTF-8.
    InvalidUtf8,
    /// A uniqueness-constrained collection declares the same identity twice.
    DuplicateId { id: [u8; 16] },
    /// Bytes remained after the complete canonical value.
    TrailingBytes { count: usize },
}

impl fmt::Display for DecodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated {
                offset,
                needed,
                remaining,
            } => write!(
                formatter,
                "input at offset {offset} needs {needed} bytes but only {remaining} remain"
            ),
            Self::LengthOverflow => formatter.write_str("decoded length arithmetic overflowed"),
            Self::PayloadTooLarge { length, limit } => {
                write!(formatter, "payload length {length} exceeds limit {limit}")
            }
            Self::CollectionTooLarge { count, limit } => {
                write!(formatter, "collection count {count} exceeds limit {limit}")
            }
            Self::StringTooLarge { length, limit } => {
                write!(formatter, "string length {length} exceeds limit {limit}")
            }
            Self::InvalidUtf8 => formatter.write_str("decoded string is not valid UTF-8"),
            Self::DuplicateId { id } => write!(formatter, "duplicate stable ID {id:02x?}"),
            Self::TrailingBytes { count } => {
                write!(formatter, "canonical value has {count} trailing bytes")
            }
        }
    }
}

impl Error for DecodeError {}

/// A bounded writer for canonical, length-delimited application data.
///
/// Every variable-width operation validates its field-specific limit and the
/// complete document limit before appending bytes. Higher-level Widget IR,
/// event, and state codecs compose these primitives to produce one canonical
/// representation for both native and WebAssembly adapters.
pub struct CanonicalEncoder {
    output: Vec<u8>,
    limits: EncodeLimits,
}

impl CanonicalEncoder {
    /// Creates an empty encoder with caller-selected resource ceilings.
    #[inline]
    pub const fn new(limits: EncodeLimits) -> Self {
        Self {
            output: Vec::new(),
            limits,
        }
    }

    /// Writes one `u32`-length-prefixed UTF-8 string.
    ///
    /// Strings are emitted unchanged; the encoder performs no Unicode
    /// normalization. The byte length and complete output are validated before
    /// either the prefix or string bytes are appended.
    pub fn write_str(&mut self, value: &str) -> Result<(), EncodeError> {
        let length = value.len();
        if u32::try_from(length).is_err() || length > self.limits.max_string_bytes as usize {
            return Err(EncodeError::StringTooLarge {
                length,
                limit: self.limits.max_string_bytes,
            });
        }
        self.reserve_field(4, length)?;
        self.output
            .extend_from_slice(&(length as u32).to_le_bytes());
        self.output.extend_from_slice(value.as_bytes());
        Ok(())
    }

    /// Writes a `u32` count followed by canonical fixed-width entry bytes.
    ///
    /// The count is derived from `entries`, preventing callers from declaring
    /// a count that disagrees with the encoded bytes. Zero-width or partial
    /// entries are rejected before the output is modified.
    pub fn write_fixed_collection(
        &mut self,
        entry_width: u32,
        entries: &[u8],
    ) -> Result<(), EncodeError> {
        if entry_width == 0 {
            return Err(EncodeError::InvalidEntryWidth);
        }
        let entry_width_usize = entry_width as usize;
        if !entries.len().is_multiple_of(entry_width_usize) {
            return Err(EncodeError::CollectionLengthMismatch {
                byte_length: entries.len(),
                entry_width,
            });
        }
        let count = entries.len() / entry_width_usize;
        if u32::try_from(count).is_err() || count > self.limits.max_collection_entries as usize {
            return Err(EncodeError::CollectionTooLarge {
                count,
                limit: self.limits.max_collection_entries,
            });
        }
        self.reserve_field(4, entries.len())?;
        self.output.extend_from_slice(&(count as u32).to_le_bytes());
        self.output.extend_from_slice(entries);
        Ok(())
    }

    /// Writes a bounded collection of distinct canonical stable identities.
    ///
    /// The complete collection is checked for count, output size, and duplicate
    /// identities before any bytes are appended. Identities retain their input
    /// order; canonical callers must supply their schema-defined order rather
    /// than relying on a hash-map iteration order.
    pub fn write_unique_stable_ids(
        &mut self,
        identities: &[StableId128],
    ) -> Result<(), EncodeError> {
        let count = identities.len();
        if u32::try_from(count).is_err() || count > self.limits.max_collection_entries as usize {
            return Err(EncodeError::CollectionTooLarge {
                count,
                limit: self.limits.max_collection_entries,
            });
        }
        let byte_length = count.checked_mul(16).ok_or(EncodeError::LengthOverflow)?;
        self.reserve_field(4, byte_length)?;

        let mut seen = HashSet::with_capacity(count);
        for identity in identities {
            if !seen.insert(*identity) {
                return Err(EncodeError::DuplicateId {
                    id: *identity.as_bytes(),
                });
            }
        }

        self.output.extend_from_slice(&(count as u32).to_le_bytes());
        for identity in identities {
            self.output.extend_from_slice(identity.as_bytes());
        }
        Ok(())
    }

    fn reserve_field(&self, prefix_length: usize, value_length: usize) -> Result<(), EncodeError> {
        let field_length = prefix_length
            .checked_add(value_length)
            .ok_or(EncodeError::LengthOverflow)?;
        let document_length = self
            .output
            .len()
            .checked_add(field_length)
            .ok_or(EncodeError::LengthOverflow)?;
        if document_length > self.limits.max_document_bytes as usize {
            return Err(EncodeError::DocumentTooLarge {
                length: document_length,
                limit: self.limits.max_document_bytes,
            });
        }
        Ok(())
    }

    /// Completes encoding and returns the canonical bytes without copying.
    #[inline]
    pub fn finish(self) -> Vec<u8> {
        self.output
    }
}

/// A zero-copy cursor for canonical, length-delimited application data.
///
/// The decoder centralizes checked range arithmetic and caller-selected
/// resource limits. Higher-level Widget IR, event, and state codecs compose
/// these primitives so malformed input is rejected before allocation.
pub struct CanonicalDecoder<'a> {
    input: &'a [u8],
    position: usize,
    limits: DecodeLimits,
}

impl<'a> CanonicalDecoder<'a> {
    /// Creates a decoder over one complete canonical value.
    #[inline]
    pub const fn new(input: &'a [u8], limits: DecodeLimits) -> Self {
        Self {
            input,
            position: 0,
            limits,
        }
    }

    /// Reads a `u32` count followed by fixed-width entries without copying.
    ///
    /// The encoded byte length is computed in the format's 32-bit length
    /// domain before conversion to `usize`. Overflow is rejected before any
    /// range is borrowed. The configured collection ceiling is checked before
    /// either the multiplication or the range access.
    pub fn read_fixed_collection(
        &mut self,
        entry_width: u32,
    ) -> Result<(u32, &'a [u8]), DecodeError> {
        let count = u32::from_le_bytes(self.read_array()?);
        if count > self.limits.max_collection_entries {
            return Err(DecodeError::CollectionTooLarge {
                count,
                limit: self.limits.max_collection_entries,
            });
        }
        let byte_length = count
            .checked_mul(entry_width)
            .ok_or(DecodeError::LengthOverflow)?;
        let bytes = self.read_bytes(byte_length as usize)?;
        Ok((count, bytes))
    }

    /// Reads one `u32`-length-prefixed UTF-8 string without copying.
    ///
    /// The configured string ceiling is checked before reading the declared
    /// byte range. Malformed UTF-8 is rejected rather than normalized or
    /// replaced, preserving canonical byte equality across adapters.
    pub fn read_str(&mut self) -> Result<&'a str, DecodeError> {
        let length = u32::from_le_bytes(self.read_array()?);
        if length > self.limits.max_string_bytes {
            return Err(DecodeError::StringTooLarge {
                length,
                limit: self.limits.max_string_bytes,
            });
        }
        let bytes = self.read_bytes(length as usize)?;
        std::str::from_utf8(bytes).map_err(|_| DecodeError::InvalidUtf8)
    }

    /// Reads a bounded collection of distinct canonical stable identities.
    ///
    /// Entry bytes are borrowed from the input. The returned vector and the
    /// temporary hash set are allocated only after count, width, and input
    /// bounds have all been validated.
    pub fn read_unique_stable_ids(&mut self) -> Result<Vec<StableId128>, DecodeError> {
        let (count, bytes) = self.read_fixed_collection(16)?;
        let mut seen = HashSet::with_capacity(count as usize);
        let mut identities = Vec::with_capacity(count as usize);

        for chunk in bytes.chunks_exact(16) {
            let mut id = [0_u8; 16];
            id.copy_from_slice(chunk);
            if !seen.insert(id) {
                return Err(DecodeError::DuplicateId { id });
            }
            identities.push(StableId128::from_bytes(id));
        }

        Ok(identities)
    }

    fn read_array<const LENGTH: usize>(&mut self) -> Result<[u8; LENGTH], DecodeError> {
        let bytes = self.read_bytes(LENGTH)?;
        let mut value = [0_u8; LENGTH];
        value.copy_from_slice(bytes);
        Ok(value)
    }

    fn read_bytes(&mut self, length: usize) -> Result<&'a [u8], DecodeError> {
        let end = self
            .position
            .checked_add(length)
            .ok_or(DecodeError::LengthOverflow)?;
        let bytes = self
            .input
            .get(self.position..end)
            .ok_or(DecodeError::Truncated {
                offset: self.position,
                needed: length,
                remaining: self.input.len().saturating_sub(self.position),
            })?;
        self.position = end;
        Ok(bytes)
    }

    /// Completes decoding and rejects bytes outside the canonical value.
    ///
    /// Higher-level codecs must call this exactly once after reading their
    /// final field. Accepting unread bytes would permit multiple encodings of
    /// the same value and hide unsupported required fields.
    pub fn finish(self) -> Result<(), DecodeError> {
        let count = self.input.len().saturating_sub(self.position);
        if count == 0 {
            Ok(())
        } else {
            Err(DecodeError::TrailingBytes { count })
        }
    }
}

impl<'a> Envelope<'a> {
    /// Creates an envelope without copying its payload.
    #[inline]
    pub const fn new(
        magic: [u8; 4],
        version: Version,
        message_kind: u16,
        flags: u16,
        request_id: u64,
        payload: &'a [u8],
    ) -> Self {
        Self {
            magic,
            version,
            message_kind,
            flags,
            request_id,
            payload,
        }
    }

    /// Encodes the envelope into its canonical little-endian representation.
    pub fn encode(&self) -> Result<Vec<u8>, EncodeError> {
        let payload_len =
            u32::try_from(self.payload.len()).map_err(|_| EncodeError::PayloadTooLarge {
                length: self.payload.len(),
            })?;
        let mut encoded = Vec::with_capacity(ENVELOPE_HEADER_LEN + self.payload.len());
        encoded.extend_from_slice(&self.magic);
        encoded.extend_from_slice(&self.version.major.to_le_bytes());
        encoded.extend_from_slice(&self.version.minor.to_le_bytes());
        encoded.extend_from_slice(&self.message_kind.to_le_bytes());
        encoded.extend_from_slice(&self.flags.to_le_bytes());
        encoded.extend_from_slice(&payload_len.to_le_bytes());
        encoded.extend_from_slice(&self.request_id.to_le_bytes());
        encoded.extend_from_slice(self.payload);
        Ok(encoded)
    }

    /// Decodes one canonical envelope while borrowing its payload from `input`.
    ///
    /// The decoder performs checked range arithmetic and never allocates for
    /// the payload. Input shorter than any fixed field or the declared payload
    /// is rejected as [`DecodeError::Truncated`].
    pub fn decode(input: &'a [u8], limits: DecodeLimits) -> Result<Self, DecodeError> {
        let mut decoder = CanonicalDecoder::new(input, limits);
        let magic = decoder.read_array()?;
        let version = Version::new(
            u16::from_le_bytes(decoder.read_array()?),
            u16::from_le_bytes(decoder.read_array()?),
        );
        let message_kind = u16::from_le_bytes(decoder.read_array()?);
        let flags = u16::from_le_bytes(decoder.read_array()?);
        let payload_len = u32::from_le_bytes(decoder.read_array()?);
        let request_id = u64::from_le_bytes(decoder.read_array()?);
        if payload_len > limits.max_payload_bytes {
            return Err(DecodeError::PayloadTooLarge {
                length: payload_len,
                limit: limits.max_payload_bytes,
            });
        }
        let payload = decoder.read_bytes(payload_len as usize)?;
        decoder.finish()?;

        Ok(Self::new(
            magic,
            version,
            message_kind,
            flags,
            request_id,
            payload,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_string_encoding_matches_golden_bytes_and_decodes() {
        let mut encoder = crate::CanonicalEncoder::new(crate::EncodeLimits::new(64, 4, 32));

        encoder.write_str("Aimer").unwrap();
        let encoded = encoder.finish();

        assert_eq!(encoded, [5, 0, 0, 0, b'A', b'i', b'm', b'e', b'r']);

        let mut decoder = crate::CanonicalDecoder::new(&encoded, DecodeLimits::new(64, 4, 32));
        assert_eq!(decoder.read_str().unwrap(), "Aimer");
        assert_eq!(decoder.finish(), Ok(()));
    }

    #[test]
    fn canonical_string_encoding_rejects_limits_without_partial_output() {
        let mut encoder = crate::CanonicalEncoder::new(crate::EncodeLimits::new(64, 4, 4));

        encoder.write_str("ok").unwrap();

        assert_eq!(
            encoder.write_str("Aimer"),
            Err(EncodeError::StringTooLarge {
                length: 5,
                limit: 4,
            })
        );
        assert_eq!(encoder.finish(), [2, 0, 0, 0, b'o', b'k']);
    }

    #[test]
    fn canonical_encoding_rejects_document_limit_without_partial_field() {
        let mut encoder = crate::CanonicalEncoder::new(crate::EncodeLimits::new(10, 4, 32));

        encoder.write_str("ok").unwrap();

        assert_eq!(
            encoder.write_str("Aimer"),
            Err(EncodeError::DocumentTooLarge {
                length: 15,
                limit: 10,
            })
        );
        assert_eq!(encoder.finish(), [2, 0, 0, 0, b'o', b'k']);
    }

    #[test]
    fn canonical_fixed_collection_encoding_matches_golden_bytes_and_decodes() {
        let mut encoder = crate::CanonicalEncoder::new(crate::EncodeLimits::new(64, 4, 32));

        encoder
            .write_fixed_collection(2, &[0x11, 0x22, 0x33, 0x44])
            .unwrap();
        let encoded = encoder.finish();

        assert_eq!(encoded, [2, 0, 0, 0, 0x11, 0x22, 0x33, 0x44]);

        let mut decoder = crate::CanonicalDecoder::new(&encoded, DecodeLimits::new(64, 4, 32));
        assert_eq!(
            decoder.read_fixed_collection(2).unwrap(),
            (2, [0x11, 0x22, 0x33, 0x44].as_slice())
        );
        assert_eq!(decoder.finish(), Ok(()));
    }

    #[test]
    fn canonical_fixed_collection_rejects_count_limit_without_partial_output() {
        let mut encoder = crate::CanonicalEncoder::new(crate::EncodeLimits::new(64, 1, 32));

        assert_eq!(
            encoder.write_fixed_collection(2, &[0x11, 0x22, 0x33, 0x44]),
            Err(EncodeError::CollectionTooLarge { count: 2, limit: 1 })
        );
        assert!(encoder.finish().is_empty());
    }

    #[test]
    fn canonical_fixed_collection_rejects_incomplete_entry_without_partial_output() {
        let mut encoder = crate::CanonicalEncoder::new(crate::EncodeLimits::new(64, 4, 32));

        assert_eq!(
            encoder.write_fixed_collection(2, &[0x11, 0x22, 0x33]),
            Err(EncodeError::CollectionLengthMismatch {
                byte_length: 3,
                entry_width: 2,
            })
        );
        assert!(encoder.finish().is_empty());
    }

    #[test]
    fn canonical_fixed_collection_rejects_zero_entry_width_without_partial_output() {
        let mut encoder = crate::CanonicalEncoder::new(crate::EncodeLimits::new(64, 4, 32));

        assert_eq!(
            encoder.write_fixed_collection(0, &[]),
            Err(EncodeError::InvalidEntryWidth)
        );
        assert!(encoder.finish().is_empty());
    }

    #[test]
    fn derived_stable_ids_have_canonical_collection_bytes_and_round_trip() {
        let identities = [
            StableId128::derive(
                "crates.io::counter_app",
                crate::IdentityKind::Widget,
                "counter_button",
            ),
            StableId128::derive(
                "crates.io::counter_app",
                crate::IdentityKind::Callback,
                "counter_button",
            ),
        ];
        let mut encoder = crate::CanonicalEncoder::new(crate::EncodeLimits::new(64, 4, 32));

        encoder.write_unique_stable_ids(&identities).unwrap();
        let encoded = encoder.finish();

        #[rustfmt::skip]
        assert_eq!(
            encoded,
            [
                0x02, 0x00, 0x00, 0x00, 0xD3, 0xD7, 0x3F, 0x7C, 0x91, 0xAD, 0x15, 0x1B,
                0xF7, 0x4C, 0x7D, 0x72, 0x4A, 0x4E, 0xDB, 0x71, 0xEA, 0xFF, 0x7C, 0x1D,
                0x57, 0x21, 0x0F, 0xCB, 0x93, 0x7F, 0x48, 0x32, 0x78, 0xF0, 0xDE, 0xE0,
            ]
        );

        let mut decoder = crate::CanonicalDecoder::new(&encoded, DecodeLimits::new(64, 4, 32));
        assert_eq!(decoder.read_unique_stable_ids().unwrap(), identities);
        assert_eq!(decoder.finish(), Ok(()));
    }

    #[test]
    fn canonical_stable_id_encoding_rejects_duplicates_without_partial_output() {
        let duplicate = StableId128::from_bytes([0xA5; 16]);
        let mut encoder = crate::CanonicalEncoder::new(crate::EncodeLimits::new(64, 4, 32));

        assert_eq!(
            encoder.write_unique_stable_ids(&[duplicate, duplicate]),
            Err(EncodeError::DuplicateId { id: [0xA5; 16] })
        );
        assert!(encoder.finish().is_empty());
    }

    #[test]
    fn envelope_encoding_matches_the_version_one_golden_vector() {
        let envelope = Envelope::new(
            *b"AAPP",
            Version::new(1, 2),
            3,
            4,
            0x0102_0304_0506_0708,
            &[0xDE, 0xAD, 0xBE, 0xEF],
        );

        #[rustfmt::skip]
        assert_eq!(
            envelope.encode().unwrap(),
            [
                0x41, 0x41, 0x50, 0x50, 0x01, 0x00, 0x02, 0x00, 0x03, 0x00, 0x04, 0x00, 0x04,
                0x00, 0x00, 0x00, 0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01, 0xDE, 0xAD,
                0xBE, 0xEF,
            ]
        );
    }

    #[test]
    fn envelope_decoding_rejects_truncation_at_every_byte_boundary() {
        let envelope = Envelope::new(
            *b"AAPP",
            Version::new(1, 2),
            3,
            4,
            0x0102_0304_0506_0708,
            &[0xDE, 0xAD, 0xBE, 0xEF],
        );
        let encoded = envelope.encode().unwrap();
        let limits = DecodeLimits::new(64, 4, 32);

        for boundary in 0..encoded.len() {
            assert!(
                matches!(
                    Envelope::decode(&encoded[..boundary], limits),
                    Err(DecodeError::Truncated { .. })
                ),
                "boundary {boundary} was accepted"
            );
        }
        assert_eq!(Envelope::decode(&encoded, limits).unwrap(), envelope);
    }

    #[test]
    fn envelope_decoding_rejects_trailing_bytes() {
        let envelope = Envelope::new(*b"AAPP", Version::new(1, 0), 1, 0, 7, &[0x2A]);
        let mut encoded = envelope.encode().unwrap();
        encoded.push(0xFF);

        assert_eq!(
            Envelope::decode(&encoded, DecodeLimits::new(64, 4, 32)),
            Err(DecodeError::TrailingBytes { count: 1 })
        );
    }

    #[test]
    fn envelope_decoding_distinguishes_payload_limits_from_truncation() {
        let envelope = Envelope::new(*b"AAPP", Version::new(1, 0), 1, 0, 7, &[0x2A, 0x2B]);
        let encoded = envelope.encode().unwrap();

        assert_eq!(
            Envelope::decode(&encoded, DecodeLimits::new(1, 4, 32)),
            Err(DecodeError::PayloadTooLarge {
                length: 2,
                limit: 1,
            })
        );
    }

    #[test]
    fn fixed_collection_decoding_rejects_byte_length_overflow() {
        let encoded = u32::MAX.to_le_bytes();
        let mut decoder =
            crate::CanonicalDecoder::new(&encoded, DecodeLimits::new(64, u32::MAX, 32));

        assert_eq!(
            decoder.read_fixed_collection(16),
            Err(DecodeError::LengthOverflow)
        );
    }

    #[test]
    fn fixed_collection_decoding_rejects_oversized_counts_before_reading_entries() {
        let encoded = 2_u32.to_le_bytes();
        let mut decoder = crate::CanonicalDecoder::new(&encoded, DecodeLimits::new(64, 1, 32));

        assert_eq!(
            decoder.read_fixed_collection(16),
            Err(DecodeError::CollectionTooLarge { count: 2, limit: 1 })
        );
    }

    #[test]
    fn string_decoding_rejects_invalid_utf8() {
        let encoded = [2, 0, 0, 0, 0xC3, 0x28];
        let mut decoder = crate::CanonicalDecoder::new(&encoded, DecodeLimits::new(64, 4, 32));

        assert_eq!(decoder.read_str(), Err(DecodeError::InvalidUtf8));
    }

    #[test]
    fn stable_id_collection_decoding_rejects_duplicates() {
        let duplicate = [0xA5; 16];
        let mut encoded = Vec::from(2_u32.to_le_bytes());
        encoded.extend_from_slice(&duplicate);
        encoded.extend_from_slice(&duplicate);
        let mut decoder = crate::CanonicalDecoder::new(&encoded, DecodeLimits::new(64, 4, 32));

        assert_eq!(
            decoder.read_unique_stable_ids(),
            Err(DecodeError::DuplicateId { id: duplicate })
        );
    }
}
