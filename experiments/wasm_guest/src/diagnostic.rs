use std::error::Error;
use std::fmt;

use aimer_anteros::{
    AbiStatus, GuestDiagnostic, GuestDiagnosticCategory, GuestOperation, GuestPanicRecord,
    GuestSourceLocation, ModelError,
};

/// The maximum UTF-8 payload copied from a guest panic into AGDI.
pub const MAX_GUEST_PANIC_PAYLOAD_BYTES: usize = 2_048;

/// A sanitized failure returned across the guest ABI boundary.
///
/// The optional diagnostic contains only bounded, canonical text and stable
/// identities. It cannot contain guest state, callback payloads, capabilities,
/// or host session material. Oversized diagnostics are discarded by the raw
/// export bridge and leave the stable status intact.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GuestError {
    status: AbiStatus,
    value: u32,
    diagnostic: Option<GuestDiagnostic>,
}

impl GuestError {
    /// Creates an application failure with one stable ABI status.
    #[inline]
    pub const fn new(status: AbiStatus) -> Self {
        Self {
            status,
            value: 0,
            diagnostic: None,
        }
    }

    pub(crate) const fn with_value(status: AbiStatus, value: u32) -> Self {
        Self {
            status,
            value,
            diagnostic: None,
        }
    }

    /// Creates an application failure with one bounded structured diagnostic.
    #[inline]
    pub fn with_diagnostic(status: AbiStatus, diagnostic: GuestDiagnostic) -> Self {
        Self {
            status,
            value: 0,
            diagnostic: Some(diagnostic),
        }
    }

    /// Attaches the operation currently invoking the guest program.
    #[inline]
    pub fn with_operation(mut self, operation: GuestOperation) -> Self {
        self.diagnostic = self
            .diagnostic
            .take()
            .map(|diagnostic| diagnostic.with_operation(operation));
        self
    }

    /// Converts canonical model validation into a stable ABI failure.
    #[inline]
    pub fn from_model(error: ModelError) -> Self {
        let status = match &error {
            ModelError::UnsupportedVersion { .. } => AbiStatus::UnsupportedVersion,
            ModelError::DocumentTooLarge { .. }
            | ModelError::CollectionTooLarge { .. }
            | ModelError::StringTooLarge { .. }
            | ModelError::BlobTooLarge { .. }
            | ModelError::WidgetDepthExceeded { .. } => AbiStatus::ResourceExhausted,
            ModelError::DuplicateWidgetKey { .. }
            | ModelError::DuplicateWidgetChild { .. }
            | ModelError::DuplicateCapabilityId { .. }
            | ModelError::DuplicateStateId { .. } => AbiStatus::DuplicateId,
            _ => AbiStatus::MalformedMessage,
        };
        Self::with_diagnostic(
            status,
            GuestDiagnostic::new(
                GuestOperation::Unknown,
                GuestDiagnosticCategory::Model,
                error.to_string(),
            ),
        )
    }

    /// Converts a recovered guest panic into a bounded structured failure.
    pub(crate) fn from_panic(operation: GuestOperation, panic: GuestPanicRecord) -> Self {
        let context = panic.context();
        let payload = bounded_panic_payload(panic.payload());
        let message = match context {
            Some(context) => format!("during {}: {payload}", context.phase()),
            None => payload,
        };
        let mut diagnostic =
            GuestDiagnostic::new(operation, GuestDiagnosticCategory::Panic, message);
        if let Some(context) = context {
            diagnostic = diagnostic.with_widget(context.widget());
        }
        if let Some(location) = panic.location() {
            diagnostic = diagnostic.with_location(GuestSourceLocation::new(
                location.file(),
                location.line(),
                location.column(),
            ));
        }
        Self::with_diagnostic(AbiStatus::ApplicationError, diagnostic)
    }

    /// Returns the stable status exposed to the permanent host.
    #[inline]
    pub const fn status(&self) -> AbiStatus {
        self.status
    }

    /// Returns the optional structured diagnostic carried by this error.
    #[inline]
    pub fn diagnostic(&self) -> Option<&GuestDiagnostic> {
        self.diagnostic.as_ref()
    }

    pub(crate) fn encode_diagnostic(&self, maximum: usize) -> Option<Vec<u8>> {
        self.diagnostic
            .as_ref()
            .and_then(|diagnostic| diagnostic.encode_with_limit(maximum).ok())
    }

    #[inline]
    pub(crate) const fn value(&self) -> u32 {
        self.value
    }
}

fn bounded_panic_payload(payload: &str) -> String {
    if payload.len() <= MAX_GUEST_PANIC_PAYLOAD_BYTES {
        return payload.to_owned();
    }
    const SUFFIX: &str = "… [truncated]";
    let budget = MAX_GUEST_PANIC_PAYLOAD_BYTES.saturating_sub(SUFFIX.len());
    let mut end = budget;
    while end > 0 && !payload.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{}", &payload[..end], SUFFIX)
}

impl fmt::Display for GuestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(diagnostic) = &self.diagnostic {
            write!(
                formatter,
                "guest operation failed with {:?}: {diagnostic}",
                self.status
            )
        } else {
            write!(formatter, "guest operation failed with {:?}", self.status)
        }
    }
}

impl Error for GuestError {}

#[cfg(test)]
mod tests {
    use super::*;
    use aimer_anteros::capture_guest_panic;

    #[cfg(panic = "unwind")]
    #[test]
    fn panic_payloads_are_bounded_without_splitting_utf8() {
        let payload = "é".repeat(MAX_GUEST_PANIC_PAYLOAD_BYTES);
        let panic = capture_guest_panic(|| panic!("{payload}")).expect_err("panic should recover");
        let error = GuestError::from_panic(GuestOperation::Build, panic);
        let diagnostic = error.diagnostic().unwrap().message();

        assert!(diagnostic.len() < MAX_GUEST_PANIC_PAYLOAD_BYTES + 32);
        assert!(diagnostic.ends_with("… [truncated]"));
        assert!(diagnostic.is_char_boundary(diagnostic.len()));
    }
}
