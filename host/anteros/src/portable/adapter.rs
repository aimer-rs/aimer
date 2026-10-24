use std::error::Error;
use std::fmt;

use crate::{CallbackEvent, ModelError, ModelLimits, StateBundle, WidgetDocument};

/// A failure while adapting a portable application model to a runtime output.
///
/// Model failures preserve the same validation error in native and WebAssembly
/// builds. An undersized WebAssembly output region reports the complete required
/// length and leaves the caller's memory unchanged, allowing a bounded retry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdapterError {
    /// Canonical model validation or encoding failed.
    Model(ModelError),
    /// The host-provided guest-memory region cannot hold the complete image.
    OutputTooSmall {
        /// Number of bytes required by the complete canonical image.
        required: usize,
        /// Number of writable bytes supplied by the caller.
        available: usize,
    },
}

impl fmt::Display for AdapterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Model(error) => error.fmt(formatter),
            Self::OutputTooSmall {
                required,
                available,
            } => write!(
                formatter,
                "adapter output requires {required} bytes but has {available}"
            ),
        }
    }
}

impl Error for AdapterError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Model(error) => Some(error),
            Self::OutputTooSmall { .. } => None,
        }
    }
}

impl From<ModelError> for AdapterError {
    #[inline]
    fn from(error: ModelError) -> Self {
        Self::Model(error)
    }
}

/// Native-AOT output adapter for canonical portable application images.
///
/// This adapter owns the resulting byte vector. Native hosts may later select a
/// validated direct representation for materialization, but conformance paths
/// use these bytes to prove that native behavior does not depend on Rust layout,
/// pointer width, target endianness, or enum discriminants.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeAdapter {
    limits: ModelLimits,
}

impl NativeAdapter {
    /// Creates an adapter with explicit portable-model resource ceilings.
    #[inline]
    pub const fn new(limits: ModelLimits) -> Self {
        Self { limits }
    }

    /// Encodes one complete immutable Widget IR snapshot.
    pub fn encode_widget_document(
        &self,
        document: &WidgetDocument<'_>,
    ) -> Result<Vec<u8>, AdapterError> {
        document.encode(self.limits).map_err(Into::into)
    }

    /// Encodes one callback invocation using the canonical event image.
    pub fn encode_callback_event(&self, event: CallbackEvent<'_>) -> Result<Vec<u8>, AdapterError> {
        event.encode(self.limits).map_err(Into::into)
    }

    /// Encodes one complete versioned guest-state bundle.
    pub fn encode_state_bundle(&self, state: &StateBundle<'_>) -> Result<Vec<u8>, AdapterError> {
        state.encode(self.limits).map_err(Into::into)
    }
}

/// WebAssembly-oriented output adapter for host-supplied guest memory.
///
/// The adapter never exposes Rust vectors, slices, or layouts through the ABI.
/// It first produces the complete canonical image, verifies the destination
/// capacity, and then performs one copy. A failed call cannot expose a partial
/// document to the host.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WasmAdapter {
    limits: ModelLimits,
}

impl WasmAdapter {
    /// Creates an adapter with the same explicit ceilings used by the host.
    #[inline]
    pub const fn new(limits: ModelLimits) -> Self {
        Self { limits }
    }

    /// Writes one complete immutable Widget IR image into guest memory.
    pub fn write_widget_document(
        &self,
        document: &WidgetDocument<'_>,
        output: &mut [u8],
    ) -> Result<usize, AdapterError> {
        write_output(document.encode(self.limits)?, output)
    }

    /// Writes one complete callback-event image into guest memory.
    pub fn write_callback_event(
        &self,
        event: CallbackEvent<'_>,
        output: &mut [u8],
    ) -> Result<usize, AdapterError> {
        write_output(event.encode(self.limits)?, output)
    }

    /// Writes one complete versioned state-bundle image into guest memory.
    pub fn write_state_bundle(
        &self,
        state: &StateBundle<'_>,
        output: &mut [u8],
    ) -> Result<usize, AdapterError> {
        write_output(state.encode(self.limits)?, output)
    }
}

fn write_output(encoded: Vec<u8>, output: &mut [u8]) -> Result<usize, AdapterError> {
    let required = encoded.len();
    if output.len() < required {
        return Err(AdapterError::OutputTooSmall {
            required,
            available: output.len(),
        });
    }
    output[..required].copy_from_slice(&encoded);
    Ok(required)
}
