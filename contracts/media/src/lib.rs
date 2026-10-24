#![deny(missing_docs)]

//! Platform-neutral media lifecycle and capability contracts.
//!
//! This first slice intentionally contains no native playback, browser, or
//! camera dependency. Platform adapters report typed unsupported, denied,
//! unavailable, cancellation, and resource-limit outcomes through these
//! seams.

use std::fmt;

/// A platform capability consumed by media and capture adapters.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum MediaCapability {
    /// Audio playback.
    AudioPlayback,
    /// Video playback.
    VideoPlayback,
    /// Embedded web content.
    WebView,
    /// User-selected local files.
    FilePicker,
    /// Camera or media capture.
    Camera,
}

/// A capability set supplied by a platform adapter.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CapabilitySet {
    supported: std::collections::BTreeSet<MediaCapability>,
}

impl CapabilitySet {
    /// Creates a set with no capabilities, making unsupported behavior explicit.
    pub fn new() -> Self {
        Self::default()
    }

    /// Marks a capability as supported.
    pub fn support(mut self, capability: MediaCapability) -> Self {
        self.supported.insert(capability);
        self
    }

    /// Returns whether a capability is supported.
    pub fn supports(&self, capability: MediaCapability) -> bool {
        self.supported.contains(&capability)
    }

    /// Returns all supported capabilities in deterministic order.
    pub fn iter(&self) -> impl Iterator<Item = MediaCapability> + '_ {
        self.supported.iter().copied()
    }
}

/// A media element family.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum MediaKind {
    /// Audio playback.
    Audio,
    /// Video playback.
    Video,
    /// Web content.
    WebView,
}

impl MediaKind {
    fn capability(self) -> MediaCapability {
        match self {
            Self::Audio => MediaCapability::AudioPlayback,
            Self::Video => MediaCapability::VideoPlayback,
            Self::WebView => MediaCapability::WebView,
        }
    }
}

/// A source accepted by a platform media element.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MediaSource {
    /// A network or local URL understood by the adapter.
    Url(String),
    /// Inline HTML for a WebView adapter.
    Html(String),
}

impl MediaSource {
    /// Creates a URL source.
    pub fn url(url: impl Into<String>) -> Self {
        Self::Url(url.into())
    }

    /// Creates inline HTML content.
    pub fn html(html: impl Into<String>) -> Self {
        Self::Html(html.into())
    }

    fn is_empty(&self) -> bool {
        match self {
            Self::Url(value) | Self::Html(value) => value.trim().is_empty(),
        }
    }
}

/// A deterministic media element identity.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct MediaId(u64);

impl MediaId {
    /// Creates an application-owned media identity.
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the numeric identity.
    pub const fn value(self) -> u64 {
        self.0
    }
}

/// A bounded logical media size.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MediaSize {
    width: u32,
    height: u32,
}

impl MediaSize {
    /// Creates a size; zero is allowed for intrinsic or audio-only layout.
    pub fn new(width: u32, height: u32) -> Result<Self, MediaError> {
        if width > 32_768 || height > 32_768 {
            return Err(MediaError::ResourceLimit {
                message: "media dimensions exceed the bounded maximum".to_owned(),
            });
        }
        Ok(Self { width, height })
    }

    /// Returns the logical width.
    pub const fn width(self) -> u32 {
        self.width
    }

    /// Returns the logical height.
    pub const fn height(self) -> u32 {
        self.height
    }
}

/// A lifecycle error with explicit unsupported-platform outcomes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MediaError {
    /// The platform does not expose the requested capability.
    Unsupported {
        /// Missing capability.
        capability: MediaCapability,
        /// Platform-specific explanation.
        reason: String,
    },
    /// The caller disposed the element.
    Disposed,
    /// The command does not apply to the current lifecycle state.
    InvalidState {
        /// Human-readable transition explanation.
        message: String,
    },
    /// A source was empty or incompatible with the element kind.
    InvalidSource {
        /// Human-readable source explanation.
        message: String,
    },
    /// A configured resource bound was exceeded.
    ResourceLimit {
        /// Human-readable limit explanation.
        message: String,
    },
}

impl fmt::Display for MediaError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported { reason, .. } => write!(formatter, "unsupported media: {reason}"),
            Self::Disposed => formatter.write_str("media element is disposed"),
            Self::InvalidState { message }
            | Self::InvalidSource { message }
            | Self::ResourceLimit { message } => formatter.write_str(message),
        }
    }
}

impl std::error::Error for MediaError {}

/// The observable lifecycle of a media element.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MediaState {
    /// Constructed but not loaded.
    Created,
    /// The adapter reported that the capability is unsupported.
    Unsupported {
        /// Missing capability.
        capability: MediaCapability,
        /// Platform-specific explanation.
        reason: String,
    },
    /// Ready for playback or display.
    Ready,
    /// Actively playing audio or video.
    Playing,
    /// Paused after playback began.
    Paused,
    /// Stopped and retained for reuse.
    Stopped,
    /// Resources have been released and no further commands are accepted.
    Disposed,
}

/// A capability-gated audio/video/WebView model.
#[derive(Clone, Debug)]
pub struct MediaElement {
    id: MediaId,
    kind: MediaKind,
    source: MediaSource,
    capabilities: CapabilitySet,
    size: MediaSize,
    state: MediaState,
    focused: bool,
}

impl MediaElement {
    /// Creates an element in the `Created` state.
    pub fn new(
        id: MediaId,
        kind: MediaKind,
        source: MediaSource,
        capabilities: CapabilitySet,
    ) -> Result<Self, MediaError> {
        if source.is_empty() {
            return Err(MediaError::InvalidSource {
                message: "media source cannot be empty".to_owned(),
            });
        }
        Ok(Self {
            id,
            kind,
            source,
            capabilities,
            size: MediaSize::new(0, 0)?,
            state: MediaState::Created,
            focused: false,
        })
    }

    /// Creates an audio element.
    pub fn audio(
        id: MediaId,
        source: MediaSource,
        capabilities: CapabilitySet,
    ) -> Result<Self, MediaError> {
        Self::new(id, MediaKind::Audio, source, capabilities)
    }

    /// Creates a video element.
    pub fn video(
        id: MediaId,
        source: MediaSource,
        capabilities: CapabilitySet,
    ) -> Result<Self, MediaError> {
        Self::new(id, MediaKind::Video, source, capabilities)
    }

    /// Creates a WebView element.
    pub fn web_view(
        id: MediaId,
        source: MediaSource,
        capabilities: CapabilitySet,
    ) -> Result<Self, MediaError> {
        Self::new(id, MediaKind::WebView, source, capabilities)
    }

    /// Returns the application-owned identity.
    pub const fn id(&self) -> MediaId {
        self.id
    }

    /// Returns the element family.
    pub const fn kind(&self) -> MediaKind {
        self.kind
    }

    /// Returns the source.
    pub fn source(&self) -> &MediaSource {
        &self.source
    }

    /// Returns the current logical size.
    pub const fn size(&self) -> MediaSize {
        self.size
    }

    /// Returns the current lifecycle state.
    pub fn state(&self) -> MediaState {
        self.state.clone()
    }

    /// Sets a bounded layout size before or after loading.
    pub fn resize(&mut self, size: MediaSize) -> Result<(), MediaError> {
        self.ensure_live()?;
        self.size = size;
        Ok(())
    }

    /// Requests focus for keyboard/accessibility integration.
    pub fn focus(&mut self) -> Result<(), MediaError> {
        self.ensure_live()?;
        self.focused = true;
        Ok(())
    }

    /// Clears the element's focus ownership.
    pub fn blur(&mut self) -> Result<(), MediaError> {
        self.ensure_live()?;
        self.focused = false;
        Ok(())
    }

    /// Returns whether this element owns focus.
    pub const fn is_focused(&self) -> bool {
        self.focused
    }

    /// Starts loading and reports unsupported capability explicitly.
    pub fn load(&mut self) -> Result<(), MediaError> {
        self.ensure_live()?;
        let capability = self.kind.capability();
        if !self.capabilities.supports(capability) {
            let reason = format!("platform does not provide {capability:?}");
            self.state = MediaState::Unsupported {
                capability,
                reason: reason.clone(),
            };
            return Err(MediaError::Unsupported { capability, reason });
        }
        self.state = MediaState::Ready;
        Ok(())
    }

    /// Starts or resumes audio/video playback.
    pub fn play(&mut self) -> Result<(), MediaError> {
        self.ensure_live()?;
        if self.kind == MediaKind::WebView {
            return Err(MediaError::InvalidState {
                message: "WebView elements do not expose playback commands".to_owned(),
            });
        }
        self.ensure_supported()?;
        if matches!(
            self.state,
            MediaState::Ready | MediaState::Paused | MediaState::Stopped
        ) {
            self.state = MediaState::Playing;
            Ok(())
        } else {
            Err(MediaError::InvalidState {
                message: "media must be loaded before playback".to_owned(),
            })
        }
    }

    /// Pauses active playback.
    pub fn pause(&mut self) -> Result<(), MediaError> {
        self.ensure_live()?;
        if self.state == MediaState::Playing {
            self.state = MediaState::Paused;
            Ok(())
        } else {
            Err(MediaError::InvalidState {
                message: "only playing media can be paused".to_owned(),
            })
        }
    }

    /// Stops playback while retaining the loaded element.
    pub fn stop(&mut self) -> Result<(), MediaError> {
        self.ensure_live()?;
        if matches!(self.state, MediaState::Playing | MediaState::Paused) {
            self.state = MediaState::Stopped;
            Ok(())
        } else {
            Err(MediaError::InvalidState {
                message: "only active media can be stopped".to_owned(),
            })
        }
    }

    /// Releases the adapter resource; disposal is idempotent.
    pub fn dispose(&mut self) -> Result<(), MediaError> {
        if self.state != MediaState::Disposed {
            self.state = MediaState::Disposed;
            self.focused = false;
        }
        Ok(())
    }

    fn ensure_live(&self) -> Result<(), MediaError> {
        if self.state == MediaState::Disposed {
            Err(MediaError::Disposed)
        } else {
            Ok(())
        }
    }

    fn ensure_supported(&self) -> Result<(), MediaError> {
        if let MediaState::Unsupported { capability, reason } = &self.state {
            return Err(MediaError::Unsupported {
                capability: *capability,
                reason: reason.clone(),
            });
        }
        Ok(())
    }
}

/// A file-picker or camera request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CaptureRequest {
    capability: MediaCapability,
    accepted_mime_types: Vec<String>,
    max_bytes: u64,
}

impl CaptureRequest {
    /// Creates a bounded local file-picker request.
    pub fn file_picker<I, S>(accepted_mime_types: I, max_bytes: u64) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            capability: MediaCapability::FilePicker,
            accepted_mime_types: accepted_mime_types.into_iter().map(Into::into).collect(),
            max_bytes,
        }
    }

    /// Creates a bounded camera capture request.
    pub fn camera<I, S>(accepted_mime_types: I, max_bytes: u64) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            capability: MediaCapability::Camera,
            accepted_mime_types: accepted_mime_types.into_iter().map(Into::into).collect(),
            max_bytes,
        }
    }

    /// Returns the capability needed to fulfill the request.
    pub const fn capability(&self) -> MediaCapability {
        self.capability
    }
}

/// Metadata for a selected or captured file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MediaFile {
    name: String,
    mime_type: String,
    bytes: u64,
}

impl MediaFile {
    /// Creates file metadata and rejects empty names/types.
    pub fn new(
        name: impl Into<String>,
        mime_type: impl Into<String>,
        bytes: u64,
    ) -> Result<Self, MediaError> {
        let name = name.into();
        let mime_type = mime_type.into();
        if name.trim().is_empty() || mime_type.trim().is_empty() {
            return Err(MediaError::InvalidSource {
                message: "captured files need a name and MIME type".to_owned(),
            });
        }
        if name.contains('/') || name.contains('\\') || name.contains('\0') {
            return Err(MediaError::InvalidSource {
                message: "captured file names cannot contain path separators".to_owned(),
            });
        }
        Ok(Self {
            name,
            mime_type,
            bytes,
        })
    }

    /// Returns the display name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the declared MIME type.
    pub fn mime_type(&self) -> &str {
        &self.mime_type
    }

    /// Returns the byte size.
    pub const fn bytes(&self) -> u64 {
        self.bytes
    }
}

/// A bounded outcome from a picker or capture adapter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CaptureOutcome {
    /// A user-selected file.
    Selected(MediaFile),
    /// A camera or capture adapter produced a file.
    Captured(MediaFile),
    /// The user dismissed the picker/capture flow.
    Cancelled,
    /// Permission was denied.
    Denied,
    /// The adapter exists but is temporarily unavailable.
    Unavailable,
    /// The target cannot provide this capability.
    Unsupported {
        /// Missing capability.
        capability: MediaCapability,
        /// Platform-specific explanation.
        reason: String,
    },
    /// The result violated a caller-provided security limit.
    Rejected(CaptureRejection),
}

/// A platform adapter seam for file selection and media capture.
pub trait CaptureAdapter {
    /// Starts a user file-picker flow.
    fn pick(&self, request: &CaptureRequest) -> CaptureOutcome;

    /// Starts a camera/media capture flow.
    fn capture(&self, request: &CaptureRequest) -> CaptureOutcome;
}

/// A file result rejected by type or size policy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CaptureRejection {
    /// The file is larger than the request maximum.
    SizeLimit {
        /// Maximum permitted bytes.
        max_bytes: u64,
        /// Actual file bytes.
        actual_bytes: u64,
    },
    /// The MIME type was not requested.
    TypeNotAllowed {
        /// Actual MIME type.
        mime_type: String,
    },
}

/// Validates a selected/captured file against a request.
pub fn validate_capture(request: &CaptureRequest, file: &MediaFile) -> CaptureOutcome {
    if file.bytes() > request.max_bytes {
        return CaptureOutcome::Rejected(CaptureRejection::SizeLimit {
            max_bytes: request.max_bytes,
            actual_bytes: file.bytes(),
        });
    }
    if !request.accepted_mime_types.is_empty()
        && !request
            .accepted_mime_types
            .iter()
            .any(|mime| mime.eq_ignore_ascii_case(file.mime_type()))
    {
        return CaptureOutcome::Rejected(CaptureRejection::TypeNotAllowed {
            mime_type: file.mime_type().to_owned(),
        });
    }
    CaptureOutcome::Selected(file.clone())
}

/// A deterministic adapter that reports unsupported capabilities.
#[derive(Clone, Debug)]
pub struct UnsupportedCaptureAdapter {
    reason: String,
}

impl UnsupportedCaptureAdapter {
    /// Creates an adapter with a visible platform-specific explanation.
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }

    /// Returns the platform explanation.
    pub fn reason(&self) -> &str {
        &self.reason
    }
}

impl CaptureAdapter for UnsupportedCaptureAdapter {
    /// Reports that the request capability is unsupported.
    fn pick(&self, request: &CaptureRequest) -> CaptureOutcome {
        CaptureOutcome::Unsupported {
            capability: request.capability,
            reason: self.reason.clone(),
        }
    }

    /// Reports that the camera capability is unsupported.
    fn capture(&self, request: &CaptureRequest) -> CaptureOutcome {
        CaptureOutcome::Unsupported {
            capability: request.capability,
            reason: self.reason.clone(),
        }
    }
}

#[cfg(test)]
mod tests;
