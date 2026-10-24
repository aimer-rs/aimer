//! Authenticated, bounded wire protocol for development-only module reloads.

use std::fmt;
use std::io;
use std::time::Duration;

use ring::rand::{SecureRandom, SystemRandom};
use zeroize::{Zeroize, Zeroizing};

mod config;
mod result;
mod wire;

pub use config::{
    DEVELOPMENT_HOST_CONFIG_VERSION, DevelopmentHostConfig, DevelopmentHostConfigError,
    MAX_DEVELOPMENT_HOST_CONFIG_TEXT_BYTES,
};
pub use result::{ReloadResult, ReloadStage};
pub use wire::{
    ReloadConnectionOutcome, query_reload_result, receive_module_and_acknowledge,
    receive_reload_command, receive_reload_connection, send_module, send_reload_command,
};

/// Protocol and resource limits shared by the CLI and debug application.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProtocolLimits {
    max_module_bytes: usize,
    max_chunk_bytes: usize,
    max_diagnostic_bytes: usize,
    max_terminal_results: usize,
    io_timeout: Duration,
}

impl ProtocolLimits {
    /// Creates limits with explicit module and connection-time bounds.
    #[inline]
    pub const fn new(max_module_bytes: usize, io_timeout: Duration) -> Self {
        Self {
            max_module_bytes,
            max_chunk_bytes: max_module_bytes,
            max_diagnostic_bytes: max_module_bytes,
            max_terminal_results: 1,
            io_timeout,
        }
    }

    /// Sets the largest plaintext module chunk carried by one frame.
    ///
    /// A smaller ceiling keeps authenticated frame allocation independent of
    /// the complete artifact size. Non-empty transfers reject a zero ceiling.
    #[inline]
    pub const fn max_chunk_bytes(mut self, max_chunk_bytes: usize) -> Self {
        self.max_chunk_bytes = max_chunk_bytes;
        self
    }

    /// Sets the largest UTF-8 terminal diagnostic accepted on the wire.
    #[inline]
    pub const fn max_diagnostic_bytes(mut self, max_diagnostic_bytes: usize) -> Self {
        self.max_diagnostic_bytes = max_diagnostic_bytes;
        self
    }

    /// Sets the bounded number of terminal results retained for reconnects.
    #[inline]
    pub const fn max_terminal_results(mut self, max_terminal_results: usize) -> Self {
        self.max_terminal_results = max_terminal_results;
        self
    }

    /// Returns the largest module accepted by this protocol session.
    #[inline]
    pub const fn max_module_bytes(self) -> usize {
        self.max_module_bytes
    }

    /// Returns the largest plaintext module chunk carried by one frame.
    #[inline]
    pub const fn chunk_bytes_limit(self) -> usize {
        self.max_chunk_bytes
    }

    /// Returns the largest UTF-8 terminal diagnostic accepted on the wire.
    #[inline]
    pub const fn diagnostic_bytes_limit(self) -> usize {
        self.max_diagnostic_bytes
    }

    /// Returns the number of session terminal results retained for reconnects.
    #[inline]
    pub const fn terminal_result_limit(self) -> usize {
        self.max_terminal_results
    }

    /// Returns the read/write timeout applied to each accepted connection.
    #[inline]
    pub const fn io_timeout(self) -> Duration {
        self.io_timeout
    }
}

/// Ephemeral credentials shared only by one CLI launch and debug app process.
#[derive(Clone)]
pub struct SessionCredentials {
    session_id: [u8; 16],
    token: Zeroizing<[u8; 32]>,
}

impl SessionCredentials {
    /// Generates a fresh session identifier and 256-bit token using the OS CSPRNG.
    pub fn generate() -> Result<Self, ProtocolError> {
        let random = SystemRandom::new();
        let mut session_id = [0_u8; 16];
        let mut token = [0_u8; 32];
        random
            .fill(&mut session_id)
            .map_err(|_| ProtocolError::Cryptography)?;
        random
            .fill(&mut token)
            .map_err(|_| ProtocolError::Cryptography)?;
        Ok(Self::from_parts(session_id, token))
    }
    /// Creates deterministic credentials for private launch injection or tests.
    #[inline]
    pub fn from_parts(session_id: [u8; 16], token: [u8; 32]) -> Self {
        Self {
            session_id,
            token: Zeroizing::new(token),
        }
    }

    /// Returns the public session identifier.
    #[inline]
    pub const fn session_id(&self) -> &[u8; 16] {
        &self.session_id
    }

    /// Encodes credentials for a private development-launch environment.
    ///
    /// Both returned strings zeroize their allocations on drop. Callers must
    /// pass them only through a target adapter's proven private launch channel
    /// and must never include them in command arguments or diagnostics.
    pub fn launch_environment_hex(&self) -> (Zeroizing<String>, Zeroizing<String>) {
        (
            Zeroizing::new(hex::encode(self.session_id)),
            Zeroizing::new(hex::encode(self.token.as_ref())),
        )
    }
}

impl fmt::Debug for SessionCredentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SessionCredentials")
            .field("session_id", &"[REDACTED]")
            .field("token", &"[REDACTED]")
            .finish()
    }
}

impl Drop for SessionCredentials {
    fn drop(&mut self) {
        self.session_id.zeroize();
    }
}

/// Acknowledgement for a complete module accepted by the app-side sink.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransferAcknowledgement {
    /// Request identifier supplied by the CLI for this upload.
    pub request_id: u64,
    /// Number of module bytes accepted by the app-side sink.
    pub module_len: usize,
    /// SHA-256 digest of the accepted module.
    pub module_digest: [u8; 32],
}

/// Compatibility metadata authenticated with a complete reload module.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ModuleMetadata {
    application_id: [u8; 16],
    build_id: [u8; 16],
    abi_major: u16,
    abi_minor: u16,
    capability_manifest_digest: [u8; 32],
}

impl ModuleMetadata {
    /// Creates metadata used by the host to reject incompatible artifacts
    /// before candidate instantiation.
    #[inline]
    pub const fn new(
        application_id: [u8; 16],
        build_id: [u8; 16],
        abi_major: u16,
        abi_minor: u16,
        capability_manifest_digest: [u8; 32],
    ) -> Self {
        Self {
            application_id,
            build_id,
            abi_major,
            abi_minor,
            capability_manifest_digest,
        }
    }

    /// Returns the stable application identity.
    #[inline]
    pub const fn application_id(self) -> [u8; 16] {
        self.application_id
    }

    /// Returns the guest build identity.
    #[inline]
    pub const fn build_id(self) -> [u8; 16] {
        self.build_id
    }

    /// Returns the requested ABI major and minor version.
    #[inline]
    pub const fn abi_version(self) -> (u16, u16) {
        (self.abi_major, self.abi_minor)
    }

    /// Returns the canonical capability-manifest digest.
    #[inline]
    pub const fn capability_manifest_digest(self) -> [u8; 32] {
        self.capability_manifest_digest
    }
}

/// One authenticated, complete, bounded module command delivered to the app.
#[derive(Debug, Eq, PartialEq)]
pub struct ReloadCommand {
    request_id: u64,
    metadata: ModuleMetadata,
    module_digest: [u8; 32],
    module: Vec<u8>,
}

impl ReloadCommand {
    pub(crate) fn from_parts(
        request_id: u64,
        metadata: ModuleMetadata,
        module_digest: [u8; 32],
        module: Vec<u8>,
    ) -> Self {
        Self {
            request_id,
            metadata,
            module_digest,
            module,
        }
    }

    /// Returns the session-scoped idempotency key.
    #[inline]
    pub const fn request_id(&self) -> u64 {
        self.request_id
    }

    /// Returns compatibility metadata validated before runtime entry.
    #[inline]
    pub const fn metadata(&self) -> ModuleMetadata {
        self.metadata
    }

    /// Returns the verified SHA-256 digest of the complete module.
    #[inline]
    pub const fn module_digest(&self) -> [u8; 32] {
        self.module_digest
    }

    /// Returns the authenticated module bytes.
    #[inline]
    pub fn module(&self) -> &[u8] {
        &self.module
    }

    /// Moves the authenticated module bytes out of the command.
    #[inline]
    pub fn into_module(self) -> Vec<u8> {
        self.module
    }
}

/// Failure while authenticating or transferring a reload module.
#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error("reload transport I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("reload session authentication failed")]
    Authentication,
    #[error("reload protocol cryptography failed")]
    Cryptography,
    #[error("invalid reload frame: {0}")]
    InvalidFrame(&'static str),
    #[error("reload module is {actual} bytes but the session limit is {maximum}")]
    ModuleTooLarge { actual: usize, maximum: usize },
    #[error("reload module digest did not match its declaration")]
    DigestMismatch,
    #[error("reload module sink rejected the upload: {0}")]
    SinkRejected(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_reload_results_have_stable_canonical_vectors() {
        let committed = ReloadResult::Committed {
            active_generation: 9,
            reset_state_entries: 2,
            cleanup_warnings: 1,
        };
        let rejected = ReloadResult::Rejected {
            stage: ReloadStage::Migration,
            error_code: 0x1020_3040,
            active_generation: 8,
            diagnostic: "schema mismatch".to_owned(),
        };

        assert_eq!(
            committed.encode(64).unwrap(),
            hex::decode("0100000009000000000000000200000001000000").unwrap()
        );
        assert_eq!(
            ReloadResult::decode(&committed.encode(64).unwrap(), 64).unwrap(),
            committed
        );
        assert_eq!(
            ReloadResult::decode(&rejected.encode(64).unwrap(), 64).unwrap(),
            rejected
        );
    }

    #[test]
    fn terminal_result_decoder_rejects_every_truncation_and_noncanonical_field() {
        let result = ReloadResult::Rejected {
            stage: ReloadStage::Validation,
            error_code: 9,
            active_generation: 7,
            diagnostic: "invalid tree".to_owned(),
        };
        let encoded = result.encode(32).unwrap();

        for end in 0..encoded.len() {
            assert!(ReloadResult::decode(&encoded[..end], 32).is_err());
        }
        assert!(ReloadResult::decode(&encoded, 3).is_err());
        let mut unknown_stage = encoded.clone();
        unknown_stage[2..4].copy_from_slice(&99_u16.to_le_bytes());
        assert!(ReloadResult::decode(&unknown_stage, 32).is_err());
        let mut invalid_utf8 = encoded;
        *invalid_utf8.last_mut().unwrap() = 0xFF;
        assert!(ReloadResult::decode(&invalid_utf8, 32).is_err());

        let mut committed = ReloadResult::Committed {
            active_generation: 1,
            reset_state_entries: 0,
            cleanup_warnings: 0,
        }
        .encode(0)
        .unwrap();
        committed[2] = 1;
        assert!(ReloadResult::decode(&committed, 0).is_err());
    }

    #[test]
    fn session_credentials_never_reveal_secret_material_in_debug_output() {
        let credentials = SessionCredentials::from_parts([0x11; 16], [0xA5; 32]);
        let debug = format!("{credentials:?}");

        assert!(!debug.contains(&hex::encode([0x11; 16])));
        assert!(!debug.contains(&hex::encode([0xA5; 32])));
        assert!(debug.contains("[REDACTED]"));
    }
}
