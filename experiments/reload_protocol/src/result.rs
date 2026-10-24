use super::ProtocolError;

const COMMITTED: u16 = 1;
const REJECTED: u16 = 2;
const CANCELLED: u16 = 3;

/// A stable reload pipeline stage suitable for protocol diagnostics.
///
/// Values are explicitly encoded rather than relying on Rust enum layout, so
/// hosts and clients built from different source revisions agree on the wire.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u16)]
pub enum ReloadStage {
    /// Validate the module envelope and host compatibility.
    Preflight = 1,
    /// Instantiate the isolated candidate runtime.
    Instantiate = 2,
    /// Initialize candidate-owned application state.
    Initialize = 3,
    /// Export the active generation's portable state.
    StateExport = 4,
    /// Migrate state into the candidate's schemas.
    Migration = 5,
    /// Import migrated state into the candidate.
    StateImport = 6,
    /// Build the candidate Widget IR.
    Build = 7,
    /// Validate the candidate Widget IR and callbacks.
    Validation = 8,
    /// Materialize a disconnected native element tree.
    Materialization = 9,
    /// Prepare side-effect-free reconciliation.
    Reconciliation = 10,
    /// Wait for the window host's commit safe point.
    CommitWait = 11,
    /// Cancel the candidate before commit.
    Cancellation = 12,
}

impl ReloadStage {
    fn decode(value: u16) -> Result<Self, ProtocolError> {
        match value {
            1 => Ok(Self::Preflight),
            2 => Ok(Self::Instantiate),
            3 => Ok(Self::Initialize),
            4 => Ok(Self::StateExport),
            5 => Ok(Self::Migration),
            6 => Ok(Self::StateImport),
            7 => Ok(Self::Build),
            8 => Ok(Self::Validation),
            9 => Ok(Self::Materialization),
            10 => Ok(Self::Reconciliation),
            11 => Ok(Self::CommitWait),
            12 => Ok(Self::Cancellation),
            _ => Err(ProtocolError::InvalidFrame("unknown reload stage")),
        }
    }
}

/// The terminal, idempotently recoverable outcome of one reload request.
///
/// A committed result is emitted only after the host safe point installs the
/// coherent generation, callback, and root snapshot. A rejected or cancelled
/// result always names the generation that remained active.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReloadResult {
    /// The candidate became the active generation.
    Committed {
        /// Generation installed by the host safe point.
        active_generation: u64,
        /// Number of reset-safe state entries restored to defaults.
        reset_state_entries: u32,
        /// Number of non-fatal cleanup diagnostics.
        cleanup_warnings: u32,
    },
    /// The candidate was destroyed and the prior generation remained active.
    Rejected {
        /// Last pipeline stage reached by the candidate.
        stage: ReloadStage,
        /// Stable stage-specific error code.
        error_code: u32,
        /// Generation retained by rollback.
        active_generation: u64,
        /// Bounded, secret-free diagnostic text.
        diagnostic: String,
    },
    /// Cancellation won the race before commit began.
    Cancelled {
        /// Generation retained after cancellation.
        active_generation: u64,
    },
}

impl ReloadResult {
    /// Encodes this result into its canonical little-endian payload.
    ///
    /// `max_diagnostic_bytes` applies only to rejected-result text. The method
    /// rejects oversized text before allocating the output payload.
    pub fn encode(&self, max_diagnostic_bytes: usize) -> Result<Vec<u8>, ProtocolError> {
        match self {
            Self::Committed {
                active_generation,
                reset_state_entries,
                cleanup_warnings,
            } => {
                let mut bytes = Vec::with_capacity(20);
                bytes.extend_from_slice(&COMMITTED.to_le_bytes());
                bytes.extend_from_slice(&0_u16.to_le_bytes());
                bytes.extend_from_slice(&active_generation.to_le_bytes());
                bytes.extend_from_slice(&reset_state_entries.to_le_bytes());
                bytes.extend_from_slice(&cleanup_warnings.to_le_bytes());
                Ok(bytes)
            }
            Self::Rejected {
                stage,
                error_code,
                active_generation,
                diagnostic,
            } => {
                if diagnostic.len() > max_diagnostic_bytes {
                    return Err(ProtocolError::InvalidFrame(
                        "reload diagnostic exceeds message limit",
                    ));
                }
                let diagnostic_len: u32 = diagnostic
                    .len()
                    .try_into()
                    .map_err(|_| ProtocolError::InvalidFrame("reload diagnostic is too long"))?;
                let capacity = 20_usize
                    .checked_add(diagnostic.len())
                    .ok_or(ProtocolError::InvalidFrame("reload result length overflow"))?;
                let mut bytes = Vec::with_capacity(capacity);
                bytes.extend_from_slice(&REJECTED.to_le_bytes());
                bytes.extend_from_slice(&(*stage as u16).to_le_bytes());
                bytes.extend_from_slice(&error_code.to_le_bytes());
                bytes.extend_from_slice(&active_generation.to_le_bytes());
                bytes.extend_from_slice(&diagnostic_len.to_le_bytes());
                bytes.extend_from_slice(diagnostic.as_bytes());
                Ok(bytes)
            }
            Self::Cancelled { active_generation } => {
                let mut bytes = Vec::with_capacity(12);
                bytes.extend_from_slice(&CANCELLED.to_le_bytes());
                bytes.extend_from_slice(&0_u16.to_le_bytes());
                bytes.extend_from_slice(&active_generation.to_le_bytes());
                Ok(bytes)
            }
        }
    }

    /// Decodes one complete canonical terminal-result payload.
    pub fn decode(bytes: &[u8], max_diagnostic_bytes: usize) -> Result<Self, ProtocolError> {
        let kind = read_u16(bytes, 0)?;
        match kind {
            COMMITTED => {
                if bytes.len() != 20 || read_u16(bytes, 2)? != 0 {
                    return Err(ProtocolError::InvalidFrame(
                        "invalid committed-result payload",
                    ));
                }
                Ok(Self::Committed {
                    active_generation: read_u64(bytes, 4)?,
                    reset_state_entries: read_u32(bytes, 12)?,
                    cleanup_warnings: read_u32(bytes, 16)?,
                })
            }
            REJECTED => {
                if bytes.len() < 20 {
                    return Err(ProtocolError::InvalidFrame(
                        "truncated rejected-result payload",
                    ));
                }
                let diagnostic_len = usize::try_from(read_u32(bytes, 16)?).map_err(|_| {
                    ProtocolError::InvalidFrame("reload diagnostic length does not fit target")
                })?;
                if diagnostic_len > max_diagnostic_bytes
                    || bytes.len().checked_sub(20) != Some(diagnostic_len)
                {
                    return Err(ProtocolError::InvalidFrame(
                        "invalid rejected-result diagnostic length",
                    ));
                }
                let diagnostic = std::str::from_utf8(&bytes[20..])
                    .map_err(|_| ProtocolError::InvalidFrame("reload diagnostic is not UTF-8"))?
                    .to_owned();
                Ok(Self::Rejected {
                    stage: ReloadStage::decode(read_u16(bytes, 2)?)?,
                    error_code: read_u32(bytes, 4)?,
                    active_generation: read_u64(bytes, 8)?,
                    diagnostic,
                })
            }
            CANCELLED => {
                if bytes.len() != 12 || read_u16(bytes, 2)? != 0 {
                    return Err(ProtocolError::InvalidFrame(
                        "invalid cancelled-result payload",
                    ));
                }
                Ok(Self::Cancelled {
                    active_generation: read_u64(bytes, 4)?,
                })
            }
            _ => Err(ProtocolError::InvalidFrame(
                "unknown terminal-result discriminator",
            )),
        }
    }
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, ProtocolError> {
    let value = bytes
        .get(offset..offset + 2)
        .ok_or(ProtocolError::InvalidFrame("truncated terminal-result u16"))?;
    Ok(u16::from_le_bytes(value.try_into().unwrap()))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, ProtocolError> {
    let value = bytes
        .get(offset..offset + 4)
        .ok_or(ProtocolError::InvalidFrame("truncated terminal-result u32"))?;
    Ok(u32::from_le_bytes(value.try_into().unwrap()))
}

fn read_u64(bytes: &[u8], offset: usize) -> Result<u64, ProtocolError> {
    let value = bytes
        .get(offset..offset + 8)
        .ok_or(ProtocolError::InvalidFrame("truncated terminal-result u64"))?;
    Ok(u64::from_le_bytes(value.try_into().unwrap()))
}
