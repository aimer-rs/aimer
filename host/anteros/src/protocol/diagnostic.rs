use std::error::Error;
use std::fmt;

use crate::identity::StableId128;

/// The maximum number of bytes a guest may return for one structured
/// diagnostic.
pub const MAX_GUEST_DIAGNOSTIC_BYTES: usize = 4_096;

const DIAGNOSTIC_MAGIC: [u8; 4] = *b"AGDI";
const LEGACY_DIAGNOSTIC_FORMAT_VERSION: u16 = 1;
const DIAGNOSTIC_FORMAT_VERSION: u16 = 2;
const HEADER_BYTES_V1: usize = 16;
const HEADER_BYTES_V2: usize = 18;
const SOURCE_FLAG: u16 = 1 << 0;
const LIMITS_FLAG: u16 = 1 << 1;
const WIDGET_FLAG: u16 = 1 << 2;
const PROPERTY_FLAG: u16 = 1 << 3;
const LOCATION_FLAG: u16 = 1 << 4;
const KNOWN_FLAGS_V1: u16 = SOURCE_FLAG | LIMITS_FLAG | WIDGET_FLAG | PROPERTY_FLAG;
const KNOWN_FLAGS_V2: u16 = KNOWN_FLAGS_V1 | LOCATION_FLAG;

/// Identifies the guest operation that produced a diagnostic.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum GuestOperation {
    /// The application manifest was requested.
    Manifest = 1,
    /// The initial or ordinary Widget IR build was requested.
    Build = 2,
    /// A callback dispatched and requested a Widget IR rebuild.
    CallbackRebuild = 3,
    /// A state image was exported.
    ExportState = 4,
    /// A state image was imported.
    Import = 5,
    /// A state image was migrated.
    Migration = 6,
    /// A candidate generation was initialized.
    Initialize = 7,
    /// The guest did not identify the operation.
    Unknown = 255,
}

impl GuestOperation {
    /// Returns the stable human-readable operation name used in diagnostics.
    #[inline]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Manifest => "aimer_manifest",
            Self::Build => "aimer_build",
            Self::CallbackRebuild => "callback rebuild",
            Self::ExportState => "aimer_export_state",
            Self::Import => "import",
            Self::Migration => "migration",
            Self::Initialize => "aimer_initialize",
            Self::Unknown => "guest operation",
        }
    }
}

impl TryFrom<u8> for GuestOperation {
    type Error = GuestDiagnosticDecodeError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::Manifest),
            2 => Ok(Self::Build),
            3 => Ok(Self::CallbackRebuild),
            4 => Ok(Self::ExportState),
            5 => Ok(Self::Import),
            6 => Ok(Self::Migration),
            7 => Ok(Self::Initialize),
            255 => Ok(Self::Unknown),
            value => Err(GuestDiagnosticDecodeError::UnknownOperation { value }),
        }
    }
}

/// Classifies a portable guest build or state-processing failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum GuestDiagnosticCategory {
    /// A generic application failure without a more precise category.
    Application = 0,
    /// A widget has no guest lowering implementation.
    UnsupportedWidget = 1,
    /// A widget property cannot be represented by the guest contract.
    UnsupportedProperty = 2,
    /// A reflected property codec rejected its value.
    PropertyEncoding = 3,
    /// A configured portable resource limit was exceeded.
    LimitExceeded = 4,
    /// A value could not be represented by the wire format's length range.
    LengthOverflow = 5,
    /// A child reference is invalid.
    InvalidChild = 6,
    /// A child was supplied more than once.
    DuplicateChild = 7,
    /// A child is already owned by another parent.
    ChildAlreadyAttached = 8,
    /// Two widgets derived the same retained slot.
    DuplicateSlot = 9,
    /// The document graph is incomplete.
    IncompleteTree = 10,
    /// A property references a missing table entry.
    InvalidPropertyReference = 11,
    /// A floating-point value is not finite.
    NonFiniteFloat = 12,
    /// A Rust value has no valid AWIR representation.
    InvalidPropertyValue = 13,
    /// A callback registration or dispatch failed.
    Callback = 14,
    /// Retained portable state processing failed.
    State = 15,
    /// Canonical Anteros model validation failed.
    Model = 16,
    /// Application code panicked while the guest was executing an operation.
    Panic = 17,
}

impl GuestDiagnosticCategory {
    /// Returns the stable human-readable category name.
    #[inline]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Application => "application",
            Self::UnsupportedWidget => "unsupported widget",
            Self::UnsupportedProperty => "unsupported property",
            Self::PropertyEncoding => "property encoding",
            Self::LimitExceeded => "limit exceeded",
            Self::LengthOverflow => "length overflow",
            Self::InvalidChild => "invalid child",
            Self::DuplicateChild => "duplicate child",
            Self::ChildAlreadyAttached => "child already attached",
            Self::DuplicateSlot => "duplicate slot",
            Self::IncompleteTree => "incomplete tree",
            Self::InvalidPropertyReference => "invalid property reference",
            Self::NonFiniteFloat => "non-finite float",
            Self::InvalidPropertyValue => "invalid property value",
            Self::Callback => "callback",
            Self::State => "state",
            Self::Model => "model",
            Self::Panic => "panic",
        }
    }
}

impl TryFrom<u8> for GuestDiagnosticCategory {
    type Error = GuestDiagnosticDecodeError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Application),
            1 => Ok(Self::UnsupportedWidget),
            2 => Ok(Self::UnsupportedProperty),
            3 => Ok(Self::PropertyEncoding),
            4 => Ok(Self::LimitExceeded),
            5 => Ok(Self::LengthOverflow),
            6 => Ok(Self::InvalidChild),
            7 => Ok(Self::DuplicateChild),
            8 => Ok(Self::ChildAlreadyAttached),
            9 => Ok(Self::DuplicateSlot),
            10 => Ok(Self::IncompleteTree),
            11 => Ok(Self::InvalidPropertyReference),
            12 => Ok(Self::NonFiniteFloat),
            13 => Ok(Self::InvalidPropertyValue),
            14 => Ok(Self::Callback),
            15 => Ok(Self::State),
            16 => Ok(Self::Model),
            17 => Ok(Self::Panic),
            value => Err(GuestDiagnosticDecodeError::UnknownCategory { value }),
        }
    }
}

/// A source coordinate captured inside the guest and transported through the
/// diagnostic envelope after any compiler path remapping.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GuestSourceLocation {
    file: String,
    line: u32,
    column: u32,
}

impl GuestSourceLocation {
    /// Creates a source location using one-based line and column coordinates.
    pub fn new(file: impl Into<String>, line: u32, column: u32) -> Self {
        Self {
            file: file.into(),
            line,
            column,
        }
    }

    /// Returns the guest-reported source file.
    #[inline]
    pub fn file(&self) -> &str {
        &self.file
    }

    /// Returns the one-based source line.
    #[inline]
    pub const fn line(&self) -> u32 {
        self.line
    }

    /// Returns the one-based source column.
    #[inline]
    pub const fn column(&self) -> u32 {
        self.column
    }
}

/// A bounded, owned diagnostic that can safely cross the guest ABI.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GuestDiagnostic {
    operation: GuestOperation,
    category: GuestDiagnosticCategory,
    widget: Option<String>,
    property: Option<String>,
    source: Option<StableId128>,
    location: Option<GuestSourceLocation>,
    limit: Option<u64>,
    actual: Option<u64>,
    message: String,
}

impl GuestDiagnostic {
    /// Creates a diagnostic with the stable operation, category, and message.
    pub fn new(
        operation: GuestOperation,
        category: GuestDiagnosticCategory,
        message: impl Into<String>,
    ) -> Self {
        Self {
            operation,
            category,
            widget: None,
            property: None,
            source: None,
            location: None,
            limit: None,
            actual: None,
            message: message.into(),
        }
    }

    /// Attaches a canonical widget name when the failing operation knows one.
    #[inline]
    pub fn with_widget(mut self, widget: impl Into<String>) -> Self {
        self.widget = Some(widget.into());
        self
    }

    /// Attaches a canonical property name when the failing operation knows one.
    #[inline]
    pub fn with_property(mut self, property: impl Into<String>) -> Self {
        self.property = Some(property.into());
        self
    }

    /// Attaches the stable source fingerprint of the failing lowering site.
    #[inline]
    pub const fn with_source(mut self, source: StableId128) -> Self {
        self.source = Some(source);
        self
    }

    /// Attaches the guest source coordinate of the failing operation.
    #[inline]
    pub fn with_location(mut self, location: GuestSourceLocation) -> Self {
        self.location = Some(location);
        self
    }

    /// Attaches configured and observed resource usage to the diagnostic.
    #[inline]
    pub const fn with_limits(mut self, limit: u64, actual: u64) -> Self {
        self.limit = Some(limit);
        self.actual = Some(actual);
        self
    }

    /// Reassigns the operation while preserving the portable failure detail.
    #[inline]
    pub const fn with_operation(mut self, operation: GuestOperation) -> Self {
        self.operation = operation;
        self
    }

    /// Returns the failing guest operation.
    #[inline]
    pub const fn operation(&self) -> GuestOperation {
        self.operation
    }

    /// Returns the portable diagnostic category.
    #[inline]
    pub const fn category(&self) -> GuestDiagnosticCategory {
        self.category
    }

    /// Returns the canonical widget name, when present.
    #[inline]
    pub fn widget(&self) -> Option<&str> {
        self.widget.as_deref()
    }

    /// Returns the canonical property name, when present.
    #[inline]
    pub fn property(&self) -> Option<&str> {
        self.property.as_deref()
    }

    /// Returns the source fingerprint, when present.
    #[inline]
    pub const fn source(&self) -> Option<StableId128> {
        self.source
    }

    /// Returns the guest source coordinate, when the diagnostic carries one.
    #[inline]
    pub fn location(&self) -> Option<&GuestSourceLocation> {
        self.location.as_ref()
    }

    /// Returns the configured resource limit, when present.
    #[inline]
    pub const fn limit(&self) -> Option<u64> {
        self.limit
    }

    /// Returns the observed resource usage, when present.
    #[inline]
    pub const fn actual(&self) -> Option<u64> {
        self.actual
    }

    /// Returns the bounded explanatory message.
    #[inline]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Encodes this diagnostic using the fixed ABI maximum.
    pub fn encode(&self) -> Result<Vec<u8>, GuestDiagnosticEncodeError> {
        self.encode_with_limit(MAX_GUEST_DIAGNOSTIC_BYTES)
    }

    /// Encodes this diagnostic under a negotiated finite output ceiling.
    pub fn encode_with_limit(&self, maximum: usize) -> Result<Vec<u8>, GuestDiagnosticEncodeError> {
        if maximum == 0 || maximum > MAX_GUEST_DIAGNOSTIC_BYTES {
            return Err(GuestDiagnosticEncodeError::InvalidLimit { maximum });
        }
        let widget = self.widget.as_deref().unwrap_or_default().as_bytes();
        let property = self.property.as_deref().unwrap_or_default().as_bytes();
        let location_file = self
            .location
            .as_ref()
            .map_or(&[][..], |location| location.file.as_bytes());
        let message = self.message.as_bytes();
        for (name, value) in [
            ("widget", widget),
            ("property", property),
            ("location", location_file),
            ("message", message),
        ] {
            if value.len() > u16::MAX as usize {
                return Err(GuestDiagnosticEncodeError::FieldTooLong {
                    field: name,
                    actual: value.len(),
                });
            }
            if std::str::from_utf8(value).is_err() {
                return Err(GuestDiagnosticEncodeError::InvalidText { field: name });
            }
        }

        let mut flags = 0;
        if self.source.is_some() {
            flags |= SOURCE_FLAG;
        }
        if self.limit.is_some() || self.actual.is_some() {
            if self.limit.is_none() || self.actual.is_none() {
                return Err(GuestDiagnosticEncodeError::IncompleteLimits);
            }
            flags |= LIMITS_FLAG;
        }
        if self.widget.is_some() {
            flags |= WIDGET_FLAG;
        }
        if self.property.is_some() {
            flags |= PROPERTY_FLAG;
        }
        if self.location.is_some() {
            flags |= LOCATION_FLAG;
        }

        let version = if flags & LOCATION_FLAG != 0 {
            DIAGNOSTIC_FORMAT_VERSION
        } else {
            LEGACY_DIAGNOSTIC_FORMAT_VERSION
        };
        let header_bytes = if version == DIAGNOSTIC_FORMAT_VERSION {
            HEADER_BYTES_V2
        } else {
            HEADER_BYTES_V1
        };
        let mut encoded = Vec::with_capacity(header_bytes + message.len());
        encoded.extend_from_slice(&DIAGNOSTIC_MAGIC);
        encoded.extend_from_slice(&version.to_le_bytes());
        encoded.push(self.operation as u8);
        encoded.push(self.category as u8);
        encoded.extend_from_slice(&flags.to_le_bytes());
        encoded.extend_from_slice(&(widget.len() as u16).to_le_bytes());
        encoded.extend_from_slice(&(property.len() as u16).to_le_bytes());
        encoded.extend_from_slice(&(message.len() as u16).to_le_bytes());
        if version == DIAGNOSTIC_FORMAT_VERSION {
            encoded.extend_from_slice(&(location_file.len() as u16).to_le_bytes());
        }
        if let Some(source) = self.source {
            encoded.extend_from_slice(source.as_bytes());
        }
        if flags & LIMITS_FLAG != 0 {
            encoded.extend_from_slice(&self.limit.unwrap().to_le_bytes());
            encoded.extend_from_slice(&self.actual.unwrap().to_le_bytes());
        }
        if let Some(location) = &self.location {
            encoded.extend_from_slice(&location.line.to_le_bytes());
            encoded.extend_from_slice(&location.column.to_le_bytes());
        }
        encoded.extend_from_slice(location_file);
        encoded.extend_from_slice(widget);
        encoded.extend_from_slice(property);
        encoded.extend_from_slice(message);
        if encoded.len() > maximum {
            return Err(GuestDiagnosticEncodeError::TooLong {
                maximum,
                actual: encoded.len(),
            });
        }
        Ok(encoded)
    }

    /// Decodes one canonical diagnostic under a negotiated finite ceiling.
    pub fn decode(bytes: &[u8], maximum: usize) -> Result<Self, GuestDiagnosticDecodeError> {
        if maximum > MAX_GUEST_DIAGNOSTIC_BYTES || maximum == 0 {
            return Err(GuestDiagnosticDecodeError::InvalidLimit { maximum });
        }
        if bytes.len() > maximum || bytes.len() > MAX_GUEST_DIAGNOSTIC_BYTES {
            return Err(GuestDiagnosticDecodeError::LimitExceeded {
                maximum,
                actual: bytes.len(),
            });
        }
        if bytes.len() < HEADER_BYTES_V1 || bytes[..4] != DIAGNOSTIC_MAGIC {
            return Err(GuestDiagnosticDecodeError::Malformed);
        }
        let version = u16::from_le_bytes([bytes[4], bytes[5]]);
        let (header_bytes, known_flags) = match version {
            LEGACY_DIAGNOSTIC_FORMAT_VERSION => (HEADER_BYTES_V1, KNOWN_FLAGS_V1),
            DIAGNOSTIC_FORMAT_VERSION => {
                if bytes.len() < HEADER_BYTES_V2 {
                    return Err(GuestDiagnosticDecodeError::Malformed);
                }
                (HEADER_BYTES_V2, KNOWN_FLAGS_V2)
            }
            version => return Err(GuestDiagnosticDecodeError::UnsupportedVersion { version }),
        };
        let operation = GuestOperation::try_from(bytes[6])?;
        let category = GuestDiagnosticCategory::try_from(bytes[7])?;
        let flags = u16::from_le_bytes([bytes[8], bytes[9]]);
        if flags & !known_flags != 0 {
            return Err(GuestDiagnosticDecodeError::Malformed);
        }
        let widget_len = u16::from_le_bytes([bytes[10], bytes[11]]) as usize;
        let property_len = u16::from_le_bytes([bytes[12], bytes[13]]) as usize;
        let message_len = u16::from_le_bytes([bytes[14], bytes[15]]) as usize;
        let location_file_len = if version == DIAGNOSTIC_FORMAT_VERSION {
            u16::from_le_bytes([bytes[16], bytes[17]]) as usize
        } else {
            0
        };
        let mut cursor = header_bytes;
        let source = if flags & SOURCE_FLAG != 0 {
            let bytes = take(bytes, &mut cursor, 16)?;
            Some(StableId128::from_bytes(bytes.try_into().unwrap()))
        } else {
            None
        };
        let (limit, actual) = if flags & LIMITS_FLAG != 0 {
            let limit = u64::from_le_bytes(take(bytes, &mut cursor, 8)?.try_into().unwrap());
            let actual = u64::from_le_bytes(take(bytes, &mut cursor, 8)?.try_into().unwrap());
            (Some(limit), Some(actual))
        } else {
            (None, None)
        };
        let location = if flags & LOCATION_FLAG != 0 {
            let line = u32::from_le_bytes(take(bytes, &mut cursor, 4)?.try_into().unwrap());
            let column = u32::from_le_bytes(take(bytes, &mut cursor, 4)?.try_into().unwrap());
            let file = decode_text(take(bytes, &mut cursor, location_file_len)?, "location")?;
            Some(GuestSourceLocation::new(file, line, column))
        } else if location_file_len == 0 {
            None
        } else {
            return Err(GuestDiagnosticDecodeError::Malformed);
        };
        let widget = if flags & WIDGET_FLAG != 0 {
            Some(decode_text(
                take(bytes, &mut cursor, widget_len)?,
                "widget",
            )?)
        } else if widget_len == 0 {
            None
        } else {
            return Err(GuestDiagnosticDecodeError::Malformed);
        };
        let property = if flags & PROPERTY_FLAG != 0 {
            Some(decode_text(
                take(bytes, &mut cursor, property_len)?,
                "property",
            )?)
        } else if property_len == 0 {
            None
        } else {
            return Err(GuestDiagnosticDecodeError::Malformed);
        };
        let message = decode_text(take(bytes, &mut cursor, message_len)?, "message")?;
        if cursor != bytes.len() {
            return Err(GuestDiagnosticDecodeError::TrailingBytes);
        }
        Ok(Self {
            operation,
            category,
            widget,
            property,
            source,
            location,
            limit,
            actual,
            message,
        })
    }
}

impl fmt::Display for GuestDiagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}: {}",
            self.operation.name(),
            self.category.name()
        )?;
        if let Some(widget) = &self.widget {
            write!(formatter, " {widget}")?;
        }
        if let Some(property) = &self.property {
            write!(formatter, " property {property}")?;
        }
        if let Some(source) = self.source {
            formatter.write_str(" at source ")?;
            for byte in source.as_bytes() {
                write!(formatter, "{byte:02x}")?;
            }
        }
        if let Some(location) = &self.location {
            write!(
                formatter,
                " at {}:{}:{}",
                location.file, location.line, location.column
            )?;
        }
        if let (Some(limit), Some(actual)) = (self.limit, self.actual) {
            write!(formatter, " (limit {limit}, actual {actual})")?;
        }
        if !self.message.is_empty() {
            write!(formatter, ": {}", self.message)?;
        }
        Ok(())
    }
}

/// A failure while encoding a bounded guest diagnostic.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GuestDiagnosticEncodeError {
    /// The negotiated maximum is outside the fixed ABI bound.
    InvalidLimit { maximum: usize },
    /// One text field does not fit in its u16 wire length.
    FieldTooLong { field: &'static str, actual: usize },
    /// A text field was not valid UTF-8.
    InvalidText { field: &'static str },
    /// Only one of the resource limit fields was supplied.
    IncompleteLimits,
    /// The encoded payload exceeds the negotiated maximum.
    TooLong { maximum: usize, actual: usize },
}

impl fmt::Display for GuestDiagnosticEncodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLimit { maximum } => {
                write!(
                    formatter,
                    "diagnostic limit {maximum} is outside the ABI bound"
                )
            }
            Self::FieldTooLong { field, actual } => {
                write!(formatter, "diagnostic {field} field has {actual} bytes")
            }
            Self::InvalidText { field } => write!(formatter, "diagnostic {field} is not UTF-8"),
            Self::IncompleteLimits => {
                formatter.write_str("diagnostic resource limits are incomplete")
            }
            Self::TooLong { maximum, actual } => write!(
                formatter,
                "diagnostic has {actual} bytes but the negotiated maximum is {maximum}"
            ),
        }
    }
}

impl Error for GuestDiagnosticEncodeError {}

/// A malformed or oversized guest diagnostic payload.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GuestDiagnosticDecodeError {
    /// The negotiated maximum is outside the fixed ABI bound.
    InvalidLimit { maximum: usize },
    /// The payload exceeds the negotiated maximum.
    LimitExceeded { maximum: usize, actual: usize },
    /// The payload has an invalid preamble or field layout.
    Malformed,
    /// The payload uses a diagnostic format this host does not understand.
    UnsupportedVersion { version: u16 },
    /// The operation tag is not recognized.
    UnknownOperation { value: u8 },
    /// The category tag is not recognized.
    UnknownCategory { value: u8 },
    /// A text field is not valid UTF-8.
    InvalidText { field: &'static str },
    /// Bytes remained after the declared fields.
    TrailingBytes,
}

impl fmt::Display for GuestDiagnosticDecodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLimit { maximum } => {
                write!(
                    formatter,
                    "diagnostic limit {maximum} is outside the ABI bound"
                )
            }
            Self::LimitExceeded { maximum, actual } => write!(
                formatter,
                "diagnostic has {actual} bytes but the negotiated maximum is {maximum}"
            ),
            Self::Malformed => formatter.write_str("malformed guest diagnostic"),
            Self::UnsupportedVersion { version } => {
                write!(formatter, "unsupported guest diagnostic version {version}")
            }
            Self::UnknownOperation { value } => {
                write!(formatter, "unknown guest diagnostic operation {value}")
            }
            Self::UnknownCategory { value } => {
                write!(formatter, "unknown guest diagnostic category {value}")
            }
            Self::InvalidText { field } => write!(formatter, "diagnostic {field} is not UTF-8"),
            Self::TrailingBytes => formatter.write_str("guest diagnostic has trailing bytes"),
        }
    }
}

impl Error for GuestDiagnosticDecodeError {}

fn take<'a>(
    bytes: &'a [u8],
    cursor: &mut usize,
    length: usize,
) -> Result<&'a [u8], GuestDiagnosticDecodeError> {
    let end = cursor
        .checked_add(length)
        .ok_or(GuestDiagnosticDecodeError::Malformed)?;
    let value = bytes
        .get(*cursor..end)
        .ok_or(GuestDiagnosticDecodeError::Malformed)?;
    *cursor = end;
    Ok(value)
}

fn decode_text(bytes: &[u8], field: &'static str) -> Result<String, GuestDiagnosticDecodeError> {
    String::from_utf8(bytes.to_vec()).map_err(|_| GuestDiagnosticDecodeError::InvalidText { field })
}
