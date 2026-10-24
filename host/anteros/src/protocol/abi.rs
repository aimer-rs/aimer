use std::error::Error;
use std::fmt;

/// The host/guest ABI version implemented by this Anteros release.
pub const CURRENT_ABI_VERSION: AbiVersion = AbiVersion::new(1, 0);

/// A stable version for the core WebAssembly host/guest calling convention.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AbiVersion {
    major: u32,
    minor: u32,
}

impl AbiVersion {
    /// Creates an explicit host/guest ABI version.
    #[inline]
    pub const fn new(major: u32, minor: u32) -> Self {
        Self { major, minor }
    }

    /// Decodes the major and minor components packed into one WebAssembly `i64`.
    #[inline]
    pub const fn from_packed(packed: i64) -> Self {
        let packed = packed as u64;
        Self::new((packed >> 32) as u32, packed as u32)
    }

    /// Returns the breaking ABI version component.
    #[inline]
    pub const fn major(self) -> u32 {
        self.major
    }

    /// Returns the backward-compatible ABI version component.
    #[inline]
    pub const fn minor(self) -> u32 {
        self.minor
    }

    /// Packs the version into the core WebAssembly `i64` representation.
    #[inline]
    pub const fn to_packed(self) -> i64 {
        (((self.major as u64) << 32) | self.minor as u64) as i64
    }
}

impl fmt::Display for AbiVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}", self.major, self.minor)
    }
}

/// Stable status codes returned by guest ABI operations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
#[non_exhaustive]
pub enum AbiStatus {
    /// The operation completed and the low word is the written length or value.
    Ok = 0,
    /// No output was written and the low word is the exact required capacity.
    BufferTooSmall = 1,
    /// One or more scalar arguments were invalid.
    InvalidArgument = 2,
    /// A required ABI or portable document version is unsupported.
    UnsupportedVersion = 3,
    /// An input document is structurally or canonically invalid.
    MalformedMessage = 4,
    /// A required stable identity is unknown.
    UnknownId = 5,
    /// An identity that must be unique was repeated.
    DuplicateId = 6,
    /// Required state cannot be imported without loss.
    StateIncompatible = 7,
    /// A capability was not declared or authorized.
    CapabilityDenied = 8,
    /// The operation requires a generation state that is not active.
    NotActive = 9,
    /// A configured guest or host resource ceiling was reached.
    ResourceExhausted = 10,
    /// An event targets a generation that no longer accepts events.
    RetiredGeneration = 11,
    /// Portable application logic rejected the operation.
    ApplicationError = 12,
    /// The guest adapter failed without a more specific portable status.
    InternalError = 13,
}

impl TryFrom<u32> for AbiStatus {
    type Error = UnknownAbiStatus;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Ok),
            1 => Ok(Self::BufferTooSmall),
            2 => Ok(Self::InvalidArgument),
            3 => Ok(Self::UnsupportedVersion),
            4 => Ok(Self::MalformedMessage),
            5 => Ok(Self::UnknownId),
            6 => Ok(Self::DuplicateId),
            7 => Ok(Self::StateIncompatible),
            8 => Ok(Self::CapabilityDenied),
            9 => Ok(Self::NotActive),
            10 => Ok(Self::ResourceExhausted),
            11 => Ok(Self::RetiredGeneration),
            12 => Ok(Self::ApplicationError),
            13 => Ok(Self::InternalError),
            code => Err(UnknownAbiStatus { code }),
        }
    }
}

/// One operation status and unsigned value packed into a WebAssembly `i64`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AbiResult {
    status: AbiStatus,
    value: u32,
}

impl AbiResult {
    /// Decodes an operation result and rejects unknown status codes.
    #[inline]
    pub fn from_packed(packed: i64) -> Result<Self, UnknownAbiStatus> {
        let packed = packed as u64;
        Ok(Self {
            status: AbiStatus::try_from((packed >> 32) as u32)?,
            value: packed as u32,
        })
    }

    /// Returns the stable operation status.
    #[inline]
    pub const fn status(self) -> AbiStatus {
        self.status
    }

    /// Returns the unsigned length, pointer, or operation-specific value.
    #[inline]
    pub const fn value(self) -> u32 {
        self.value
    }
}

/// An unknown status in a packed guest ABI result.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UnknownAbiStatus {
    code: u32,
}

impl UnknownAbiStatus {
    /// Returns the unrecognized numeric status code.
    #[inline]
    pub const fn code(self) -> u32 {
        self.code
    }
}

impl fmt::Display for UnknownAbiStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "guest returned unknown ABI status {}", self.code)
    }
}

impl Error for UnknownAbiStatus {}
