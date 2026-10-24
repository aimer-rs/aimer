use crate::{EventId, ModelError, ModelLimits, StableId128, Version};

const MAGIC: [u8; 4] = *b"AEVT";
/// Current callback-event document format emitted and accepted by Aimer.
pub const CALLBACK_EVENT_FORMAT_VERSION: Version = Version::new(2, 0);
const FORMAT_VERSION: Version = CALLBACK_EVENT_FORMAT_VERSION;
const HEADER_LEN: usize = 96;
const WIDGET_KEY_PRESENT: u32 = 1;

/// A typed callback invocation sent from the permanent host to a guest generation.
///
/// The payload remains an opaque bounded byte string owned by the event schema.
/// Stable identities cross the seam; native closures and function pointers do not.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CallbackEvent<'a> {
    generation_id: u64,
    event_sequence: u64,
    callback_id: StableId128,
    widget_key: Option<StableId128>,
    event_kind: EventId,
    event_schema: Version,
    monotonic_timestamp: u64,
    payload: &'a [u8],
}

impl<'a> CallbackEvent<'a> {
    /// Creates an event without an associated stable widget key.
    #[inline]
    pub const fn new(
        generation_id: u64,
        event_sequence: u64,
        callback_id: StableId128,
        event_kind: EventId,
        event_schema: Version,
        monotonic_timestamp: u64,
        payload: &'a [u8],
    ) -> Self {
        Self {
            generation_id,
            event_sequence,
            callback_id,
            widget_key: None,
            event_kind,
            event_schema,
            monotonic_timestamp,
            payload,
        }
    }

    /// Associates the event with a stable native widget identity.
    #[inline]
    pub const fn widget_key(mut self, widget_key: StableId128) -> Self {
        self.widget_key = Some(widget_key);
        self
    }

    /// Encodes the event as one fixed header followed by its opaque payload.
    pub fn encode(self, limits: ModelLimits) -> Result<Vec<u8>, ModelError> {
        if self.payload.len() > limits.max_blob_bytes as usize {
            return Err(ModelError::BlobTooLarge {
                length: self.payload.len(),
                limit: limits.max_blob_bytes,
            });
        }
        let total_len = HEADER_LEN
            .checked_add(self.payload.len())
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
        write_u64(&mut output, self.generation_id);
        write_u64(&mut output, self.event_sequence);
        output.extend_from_slice(self.callback_id.as_bytes());
        output.extend_from_slice(
            self.widget_key
                .unwrap_or(StableId128::from_bytes([0; 16]))
                .as_bytes(),
        );
        write_u64(&mut output, self.monotonic_timestamp);
        write_u64(&mut output, self.event_kind.value());
        write_version(&mut output, self.event_schema);
        write_u32(
            &mut output,
            u32::from(self.widget_key.is_some()) * WIDGET_KEY_PRESENT,
        );
        write_u32(&mut output, self.payload.len() as u32);
        write_u32(&mut output, total_len as u32);
        write_u64(&mut output, 0);
        output.extend_from_slice(self.payload);
        Ok(output)
    }
}

/// A validated, allocation-free callback event borrowed from host-owned bytes.
#[derive(Debug)]
pub struct CallbackEventView<'a> {
    bytes: &'a [u8],
}

impl<'a> CallbackEventView<'a> {
    /// Validates the event header, canonical flags, length, and payload limit.
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
        let flags = read_u32(bytes, 76);
        if flags & !WIDGET_KEY_PRESENT != 0 || read_u64(bytes, 88) != 0 {
            return Err(ModelError::NonCanonicalReserved);
        }
        if flags == 0 && bytes[40..56] != [0; 16] {
            return Err(ModelError::NonCanonicalReserved);
        }
        let payload_len = read_u32(bytes, 80) as usize;
        if payload_len > limits.max_blob_bytes as usize {
            return Err(ModelError::BlobTooLarge {
                length: payload_len,
                limit: limits.max_blob_bytes,
            });
        }
        let declared_len = read_u32(bytes, 84) as usize;
        let expected_len = HEADER_LEN
            .checked_add(payload_len)
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
        Ok(Self { bytes })
    }

    /// Returns the generation that may dispatch this event.
    #[inline]
    pub fn generation_id(&self) -> u64 {
        read_u64(self.bytes, 8)
    }

    /// Returns the generation-local replay-protection sequence.
    #[inline]
    pub fn event_sequence(&self) -> u64 {
        read_u64(self.bytes, 16)
    }

    /// Returns the stable callback identity resolved in the active guest.
    pub fn callback_id(&self) -> StableId128 {
        read_id(self.bytes, 24)
    }

    /// Returns the optional stable widget key associated with the event source.
    pub fn widget_key(&self) -> Option<StableId128> {
        (read_u32(self.bytes, 76) & WIDGET_KEY_PRESENT != 0).then(|| read_id(self.bytes, 40))
    }

    /// Returns the stable event kind.
    #[inline]
    pub fn event_kind(&self) -> EventId {
        EventId::new(read_u64(self.bytes, 64))
    }

    /// Returns the opaque payload schema version.
    #[inline]
    pub fn event_schema(&self) -> Version {
        read_version(self.bytes, 72)
    }

    /// Returns the host's monotonic timestamp without exposing wall-clock time.
    #[inline]
    pub fn monotonic_timestamp(&self) -> u64 {
        read_u64(self.bytes, 56)
    }

    /// Borrows the validated opaque event payload.
    #[inline]
    pub fn payload(&self) -> &'a [u8] {
        &self.bytes[HEADER_LEN..]
    }
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
        CallbackEvent, CallbackEventView, EventId, ModelLimits, StableId128, Version,
    };

    #[test]
    fn callback_event_matches_the_version_two_golden_image_and_borrows_payload() {
        let callback_id = StableId128::from_bytes([0x11; 16]);
        let widget_key = StableId128::from_bytes([0x22; 16]);
        let event = CallbackEvent::new(
            3,
            5,
            callback_id,
            EventId::new(7),
            Version::new(2, 1),
            11,
            &[0xA5, 0x5A],
        )
        .widget_key(widget_key);

        let encoded = event
            .encode(ModelLimits::new(256, 16, 64, 64))
            .unwrap();

        assert_eq!(
            encoded,
            [
                b'A', b'E', b'V', b'T',
                2, 0, 0, 0,
                3, 0, 0, 0, 0, 0, 0, 0,
                5, 0, 0, 0, 0, 0, 0, 0,
                0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11,
                0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11,
                0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22,
                0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22,
                11, 0, 0, 0, 0, 0, 0, 0,
                7, 0, 0, 0, 0, 0, 0, 0,
                2, 0, 1, 0,
                1, 0, 0, 0,
                2, 0, 0, 0,
                98, 0, 0, 0,
                0, 0, 0, 0, 0, 0, 0, 0,
                0xA5, 0x5A,
            ]
        );

        let view = CallbackEventView::decode(
            &encoded,
            ModelLimits::new(256, 16, 64, 64),
        )
        .unwrap();
        assert_eq!(view.generation_id(), 3);
        assert_eq!(view.event_sequence(), 5);
        assert_eq!(view.callback_id(), callback_id);
        assert_eq!(view.widget_key(), Some(widget_key));
        assert_eq!(view.event_kind(), EventId::new(7));
        assert_eq!(view.event_schema(), Version::new(2, 1));
        assert_eq!(view.monotonic_timestamp(), 11);
        assert_eq!(view.payload(), &[0xA5, 0x5A]);
    }

    #[test]
    fn callback_event_rejects_oversized_payloads_and_accepts_empty_payloads() {
        let callback_id = StableId128::from_bytes([0x11; 16]);
        let oversized = CallbackEvent::new(
            1,
            1,
            callback_id,
            EventId::new(1),
            Version::new(1, 0),
            1,
            &[1, 2, 3],
        );
        assert_eq!(
            oversized.encode(ModelLimits::new(256, 8, 64, 2)),
            Err(crate::ModelError::BlobTooLarge {
                length: 3,
                limit: 2,
            })
        );

        let empty = CallbackEvent::new(
            1,
            2,
            callback_id,
            EventId::new(1),
            Version::new(1, 0),
            2,
            &[],
        )
        .encode(ModelLimits::new(256, 8, 64, 2))
        .unwrap();
        assert_eq!(
            CallbackEventView::decode(&empty, ModelLimits::new(256, 8, 64, 2))
                .unwrap()
                .payload(),
            []
        );
    }
}
