use crate::{ModelError, ModelLimits, StableId128, Version};

const MAGIC: [u8; 4] = *b"AASY";
/// Current version of the bounded async callback event document.
pub const ASYNC_CALLBACK_EVENT_FORMAT_VERSION: Version = Version::new(1, 0);
const FORMAT_VERSION: Version = ASYNC_CALLBACK_EVENT_FORMAT_VERSION;
const HEADER_LEN: usize = 72;

/// A generation-local identity assigned when an async callback starts.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct AsyncTaskId(u64);

impl AsyncTaskId {
    /// Creates an explicit task identity.
    #[inline]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the numeric task identity.
    #[inline]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// The terminal outcome reported for one async callback task.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum AsyncCallbackEventKind {
    /// The task completed successfully and carries an optional bounded result.
    Complete = 1,
    /// The task failed and carries a bounded diagnostic payload.
    Failure = 2,
    /// The task was cancelled by its owner or generation retirement.
    Cancelled = 3,
}

impl AsyncCallbackEventKind {
    fn from_wire(value: u32) -> Result<Self, ModelError> {
        match value {
            1 => Ok(Self::Complete),
            2 => Ok(Self::Failure),
            3 => Ok(Self::Cancelled),
            _ => Err(ModelError::NonCanonicalReserved),
        }
    }
}

/// One bounded async callback completion or cancellation message.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AsyncCallbackEvent<'a> {
    generation_id: u64,
    event_sequence: u64,
    callback_id: StableId128,
    task_id: AsyncTaskId,
    kind: AsyncCallbackEventKind,
    payload: &'a [u8],
}

impl<'a> AsyncCallbackEvent<'a> {
    /// Creates a successful task completion.
    #[inline]
    pub const fn complete(
        generation_id: u64,
        event_sequence: u64,
        callback_id: StableId128,
        task_id: AsyncTaskId,
        payload: &'a [u8],
    ) -> Self {
        Self::new(
            generation_id,
            event_sequence,
            callback_id,
            task_id,
            AsyncCallbackEventKind::Complete,
            payload,
        )
    }

    /// Creates a bounded task failure diagnostic.
    #[inline]
    pub const fn failure(
        generation_id: u64,
        event_sequence: u64,
        callback_id: StableId128,
        task_id: AsyncTaskId,
        payload: &'a [u8],
    ) -> Self {
        Self::new(
            generation_id,
            event_sequence,
            callback_id,
            task_id,
            AsyncCallbackEventKind::Failure,
            payload,
        )
    }

    /// Creates a task cancellation notification.
    #[inline]
    pub const fn cancelled(
        generation_id: u64,
        event_sequence: u64,
        callback_id: StableId128,
        task_id: AsyncTaskId,
    ) -> Self {
        Self::new(
            generation_id,
            event_sequence,
            callback_id,
            task_id,
            AsyncCallbackEventKind::Cancelled,
            &[],
        )
    }

    /// Creates an event with an explicit terminal outcome kind.
    #[inline]
    pub const fn new(
        generation_id: u64,
        event_sequence: u64,
        callback_id: StableId128,
        task_id: AsyncTaskId,
        kind: AsyncCallbackEventKind,
        payload: &'a [u8],
    ) -> Self {
        Self {
            generation_id,
            event_sequence,
            callback_id,
            task_id,
            kind,
            payload,
        }
    }

    /// Encodes one canonical bounded async event.
    pub fn encode(self, limits: ModelLimits) -> Result<Vec<u8>, ModelError> {
        if self.task_id.get() == 0 {
            return Err(ModelError::NonCanonicalReserved);
        }
        if self.kind == AsyncCallbackEventKind::Cancelled && !self.payload.is_empty() {
            return Err(ModelError::NonCanonicalReserved);
        }
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
        write_u64(&mut output, self.task_id.get());
        write_u32(&mut output, self.kind as u32);
        write_u32(&mut output, 0);
        write_u32(&mut output, self.payload.len() as u32);
        write_u32(&mut output, total_len as u32);
        write_u64(&mut output, 0);
        output.extend_from_slice(self.payload);
        Ok(output)
    }
}

/// A validated allocation-free view over one async callback event.
#[derive(Debug)]
pub struct AsyncCallbackEventView<'a> {
    bytes: &'a [u8],
}

impl<'a> AsyncCallbackEventView<'a> {
    /// Validates the event header, terminal kind, lengths, and payload bounds.
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
        if read_version(bytes, 4) != FORMAT_VERSION {
            return Err(ModelError::UnsupportedVersion {
                version: read_version(bytes, 4),
            });
        }
        if read_u32(bytes, 52) != 0 || read_u64(bytes, 64) != 0 {
            return Err(ModelError::NonCanonicalReserved);
        }
        if read_u64(bytes, 40) == 0 {
            return Err(ModelError::NonCanonicalReserved);
        }
        let kind = AsyncCallbackEventKind::from_wire(read_u32(bytes, 48))?;
        let payload_len = read_u32(bytes, 56) as usize;
        if kind == AsyncCallbackEventKind::Cancelled && payload_len != 0 {
            return Err(ModelError::NonCanonicalReserved);
        }
        if payload_len > limits.max_blob_bytes as usize {
            return Err(ModelError::BlobTooLarge {
                length: payload_len,
                limit: limits.max_blob_bytes,
            });
        }
        let declared_len = read_u32(bytes, 60) as usize;
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

    /// Returns the owning generation identity.
    #[inline]
    pub fn generation_id(&self) -> u64 {
        read_u64(self.bytes, 8)
    }

    /// Returns the generation-local replay-protection sequence.
    #[inline]
    pub fn event_sequence(&self) -> u64 {
        read_u64(self.bytes, 16)
    }

    /// Returns the stable callback identity.
    #[inline]
    pub fn callback_id(&self) -> StableId128 {
        let mut bytes = [0_u8; 16];
        bytes.copy_from_slice(&self.bytes[24..40]);
        StableId128::from_bytes(bytes)
    }

    /// Returns the generation-local task identity.
    #[inline]
    pub fn task_id(&self) -> AsyncTaskId {
        AsyncTaskId::new(read_u64(self.bytes, 40))
    }

    /// Returns the terminal outcome kind.
    #[inline]
    pub fn kind(&self) -> AsyncCallbackEventKind {
        AsyncCallbackEventKind::from_wire(read_u32(self.bytes, 48))
            .expect("validated async callback kind")
    }

    /// Borrows the validated bounded outcome payload.
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
    Version::new(read_u16(bytes, offset), read_u16(bytes, offset + 2))
}

#[inline]
fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ModelLimits, StableId128};

    const CALLBACK_ID: StableId128 = StableId128::from_bytes([0x31; 16]);
    const LIMITS: ModelLimits = ModelLimits::new(512, 16, 64, 128);

    #[test]
    fn async_completion_round_trips_its_stable_task_identity_and_payload() {
        let event =
            AsyncCallbackEvent::complete(7, 3, CALLBACK_ID, AsyncTaskId::new(11), &[1, 2, 3]);
        let bytes = event.encode(LIMITS).unwrap();
        let view = AsyncCallbackEventView::decode(&bytes, LIMITS).unwrap();

        assert_eq!(view.generation_id(), 7);
        assert_eq!(view.event_sequence(), 3);
        assert_eq!(view.callback_id(), CALLBACK_ID);
        assert_eq!(view.task_id(), AsyncTaskId::new(11));
        assert_eq!(view.kind(), AsyncCallbackEventKind::Complete);
        assert_eq!(view.payload(), &[1, 2, 3]);
    }

    #[test]
    fn malformed_async_events_are_rejected_without_relaxing_the_payload_limit() {
        let event =
            AsyncCallbackEvent::complete(7, 3, CALLBACK_ID, AsyncTaskId::new(11), &[1, 2, 3]);
        let mut bytes = event.encode(LIMITS).unwrap();

        bytes[52] = 1;
        assert!(matches!(
            AsyncCallbackEventView::decode(&bytes, LIMITS),
            Err(ModelError::NonCanonicalReserved)
        ));

        let oversized =
            AsyncCallbackEvent::complete(7, 4, CALLBACK_ID, AsyncTaskId::new(12), &[0; 129]);
        assert_eq!(
            oversized.encode(LIMITS),
            Err(ModelError::BlobTooLarge {
                length: 129,
                limit: 128,
            })
        );
    }

    #[test]
    fn cancelled_async_events_carry_no_completion_payload() {
        let event = AsyncCallbackEvent::cancelled(7, 5, CALLBACK_ID, AsyncTaskId::new(13));
        let bytes = event.encode(LIMITS).unwrap();
        let view = AsyncCallbackEventView::decode(&bytes, LIMITS).unwrap();

        assert_eq!(view.kind(), AsyncCallbackEventKind::Cancelled);
        assert!(view.payload().is_empty());
    }

    #[test]
    fn malformed_cancelled_event_with_payload_is_rejected() {
        let event = AsyncCallbackEvent::cancelled(7, 6, CALLBACK_ID, AsyncTaskId::new(14));
        let mut bytes = event.encode(LIMITS).unwrap();
        bytes[56..60].copy_from_slice(&1_u32.to_le_bytes());
        bytes[60..64].copy_from_slice(&73_u32.to_le_bytes());
        bytes.push(9);

        assert!(matches!(
            AsyncCallbackEventView::decode(&bytes, LIMITS),
            Err(ModelError::NonCanonicalReserved)
        ));
    }
}
