//! Platform-neutral asset identity, loading, and cache contracts.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use crate::img_widget::source::ImageSource;

const DEFAULT_MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
const DEFAULT_MAX_DIMENSION: u32 = 8_192;

/// The operation that produced an [`AssetError`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AssetOperation {
    /// Source or response policy validation.
    Validate,
    /// Manifest registration or lookup.
    Manifest,
    /// Starting a resolver operation.
    Resolve,
    /// Polling or validating a load.
    Load,
    /// Retrying a failed operation.
    Retry,
    /// Cancelling an operation.
    Cancel,
    /// Reading or updating the bounded cache.
    Cache,
}

/// A stable category for an asset failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AssetErrorKind {
    /// An [`AssetId`] or source was empty or malformed.
    InvalidSource,
    /// A path would escape the configured asset boundary.
    UnsafePath,
    /// The source's network origin is not allowed by policy.
    NetworkOriginDenied,
    /// A manifest entry conflicts with an existing source identity.
    ManifestConflict,
    /// A requested manifest entry does not exist.
    NotFound,
    /// A response uses a format not allowed by policy.
    UnsupportedFormat,
    /// A byte, dimension, or cache limit was exceeded.
    ResourceLimit,
    /// The resolver could not start or complete the operation.
    Resolver,
    /// The requested state transition is not valid.
    InvalidState,
    /// The operation timed out.
    Timeout,
    /// The operation was cancelled.
    Cancelled,
}

/// A diagnostic that retains the failed operation and, when known, source.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssetError {
    source: Option<AssetId>,
    operation: AssetOperation,
    kind: AssetErrorKind,
    message: String,
}

impl AssetError {
    /// Creates a diagnostic with an operation, kind, and human-readable message.
    pub fn new(
        operation: AssetOperation,
        kind: AssetErrorKind,
        message: impl Into<String>,
    ) -> Self {
        Self {
            source: None,
            operation,
            kind,
            message: message.into(),
        }
    }

    /// Creates a resolver failure for tests or platform adapters.
    pub fn resolver(message: impl Into<String>) -> Self {
        Self::new(AssetOperation::Resolve, AssetErrorKind::Resolver, message)
    }

    /// Returns the failed source identity, if the operation had one.
    pub fn source(&self) -> Option<&AssetId> {
        self.source.as_ref()
    }

    /// Returns the operation that failed.
    pub const fn operation(&self) -> AssetOperation {
        self.operation
    }

    /// Returns the stable failure category.
    pub const fn kind(&self) -> AssetErrorKind {
        self.kind
    }

    /// Returns the human-readable diagnostic.
    pub fn message(&self) -> &str {
        &self.message
    }

    fn with_source(mut self, source: AssetId) -> Self {
        self.source = Some(source);
        self
    }
}

impl fmt::Display for AssetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(source) = &self.source {
            write!(formatter, "asset '{}': {}", source.as_str(), self.message)
        } else {
            formatter.write_str(&self.message)
        }
    }
}

impl std::error::Error for AssetError {}

/// Why constructing a stable asset identity failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AssetIdError {
    /// The source identity was empty.
    Empty,
    /// The source identity contained a control character.
    ControlCharacter,
}

impl fmt::Display for AssetIdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("asset identity cannot be empty"),
            Self::ControlCharacter => {
                formatter.write_str("asset identity cannot contain control characters")
            }
        }
    }
}

impl std::error::Error for AssetIdError {}

/// A stable identity derived from an asset's canonical source.
///
/// The identity is deterministic and includes network request headers. It is
/// suitable for manifest keys and cache keys, but is not a cryptographic hash.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AssetId(String);

impl AssetId {
    /// Creates an ID from an already canonical identity string.
    pub fn new(value: impl Into<String>) -> Result<Self, AssetIdError> {
        let value = value.into();
        if value.is_empty() {
            return Err(AssetIdError::Empty);
        }
        if value.chars().any(char::is_control) {
            return Err(AssetIdError::ControlCharacter);
        }
        Ok(Self(value))
    }

    /// Creates an ID from an asset source's canonical identity.
    pub fn from_source(source: &AssetSource) -> Result<Self, AssetIdError> {
        Self::new(source.identity_key())
    }

    /// Returns the canonical identity string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AssetId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// A platform-neutral source for an asset.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum AssetSource {
    /// A path registered in an application bundle or asset manifest.
    Bundled(String),
    /// A host file-system path.
    File(PathBuf),
    /// A network URL and canonicalized request headers.
    Network {
        /// The HTTP or HTTPS URL.
        url: String,
        /// Request headers that participate in source identity.
        headers: BTreeMap<String, String>,
    },
    /// A legacy renderer texture ID retained for explicit migration paths.
    LegacyId(u32),
}

impl AssetSource {
    /// Creates a bundled source without performing policy validation.
    pub fn bundled(key: impl Into<String>) -> Self {
        Self::Bundled(key.into())
    }

    /// Creates a file source without performing policy validation.
    pub fn file(path: impl Into<PathBuf>) -> Self {
        Self::File(path.into())
    }

    /// Creates a network source and normalizes header names for stable identity.
    pub fn network(url: impl Into<String>, headers: BTreeMap<String, String>) -> Self {
        Self::Network {
            url: url.into(),
            headers: canonical_headers(headers),
        }
    }

    /// Creates a source for an existing renderer texture ID.
    pub const fn legacy_id(id: u32) -> Self {
        Self::LegacyId(id)
    }

    fn identity_key(&self) -> String {
        match self {
            Self::Bundled(key) => format!("bundle:{key}"),
            Self::File(path) => format!("file:{}", path.to_string_lossy()),
            Self::Network { url, headers } => {
                let mut identity = format!("network:{url}");
                for (name, value) in headers {
                    identity.push('|');
                    identity.push_str(&name.len().to_string());
                    identity.push(':');
                    identity.push_str(name);
                    identity.push_str(&value.len().to_string());
                    identity.push(':');
                    identity.push_str(value);
                }
                identity
            }
            Self::LegacyId(id) => format!("legacy:{id}"),
        }
    }
}

fn canonical_headers(headers: BTreeMap<String, String>) -> BTreeMap<String, String> {
    headers
        .into_iter()
        .map(|(name, value)| (name.to_ascii_lowercase(), value))
        .collect()
}

/// A source paired with its stable identity.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AssetRef {
    id: AssetId,
    source: AssetSource,
}

impl AssetRef {
    /// Creates a reference whose ID is derived from `source`.
    pub fn new(source: AssetSource) -> Result<Self, AssetIdError> {
        let source = match source {
            AssetSource::Network { url, headers } => AssetSource::network(url, headers),
            source => source,
        };
        let id = AssetId::from_source(&source)?;
        Ok(Self { id, source })
    }

    /// Creates a bundled asset reference.
    pub fn bundled(key: impl Into<String>) -> Result<Self, AssetIdError> {
        Self::new(AssetSource::bundled(key))
    }

    /// Converts an existing image source without changing the legacy image API.
    pub fn from_image_source(source: &ImageSource) -> Result<Self, AssetIdError> {
        let source = match source {
            ImageSource::Id(id) => AssetSource::legacy_id(*id),
            ImageSource::Asset(key) => AssetSource::bundled(key.clone()),
            ImageSource::File(path) => AssetSource::file(path.clone()),
            ImageSource::Network(url) => AssetSource::network(url.clone(), BTreeMap::new()),
            ImageSource::NetworkWithHeaders(url, headers) => AssetSource::network(
                url.clone(),
                headers
                    .iter()
                    .map(|(name, value)| (name.clone(), value.clone()))
                    .collect(),
            ),
        };
        Self::new(source)
    }

    /// Returns the stable source identity.
    pub fn id(&self) -> &AssetId {
        &self.id
    }

    /// Returns the source used to resolve the asset.
    pub fn source(&self) -> &AssetSource {
        &self.source
    }
}

/// A validated map of stable IDs to their registered sources.
#[derive(Clone, Debug, Default)]
pub struct AssetManifest {
    entries: BTreeMap<AssetId, AssetRef>,
}

impl AssetManifest {
    /// Creates an empty manifest.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a source under its derived stable identity.
    pub fn register(&mut self, asset: AssetRef) -> Result<(), AssetError> {
        self.register_with_id(asset.id().clone(), asset)
    }

    /// Registers a source under an explicit identity after verifying identity agreement.
    pub fn register_with_id(&mut self, id: AssetId, asset: AssetRef) -> Result<(), AssetError> {
        if &id != asset.id() {
            return Err(AssetError::new(
                AssetOperation::Manifest,
                AssetErrorKind::ManifestConflict,
                "manifest ID does not match the source identity",
            )
            .with_source(id));
        }
        if let Some(existing) = self.entries.get(&id)
            && existing != &asset
        {
            return Err(AssetError::new(
                AssetOperation::Manifest,
                AssetErrorKind::ManifestConflict,
                "manifest ID is already registered to another source",
            )
            .with_source(id));
        }
        self.entries.insert(id, asset);
        Ok(())
    }

    /// Resolves a registered identity.
    pub fn resolve(&self, id: &AssetId) -> Result<&AssetRef, AssetError> {
        self.entries.get(id).ok_or_else(|| {
            AssetError::new(
                AssetOperation::Manifest,
                AssetErrorKind::NotFound,
                "asset is not present in the manifest",
            )
            .with_source(id.clone())
        })
    }

    /// Returns the number of registered sources.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns whether the manifest has no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Iterates over registered IDs and references in deterministic order.
    pub fn iter(&self) -> impl Iterator<Item = (&AssetId, &AssetRef)> {
        self.entries.iter()
    }
}

/// Security and resource limits applied before and after resolution.
#[derive(Clone, Debug)]
pub struct AssetPolicy {
    max_response_bytes: usize,
    max_dimension: u32,
    allowed_network_origins: BTreeSet<String>,
    allowed_mime_types: BTreeSet<String>,
    allow_absolute_files: bool,
}

impl Default for AssetPolicy {
    fn default() -> Self {
        Self {
            max_response_bytes: DEFAULT_MAX_RESPONSE_BYTES,
            max_dimension: DEFAULT_MAX_DIMENSION,
            allowed_network_origins: BTreeSet::new(),
            allowed_mime_types: BTreeSet::new(),
            allow_absolute_files: false,
        }
    }
}

impl AssetPolicy {
    /// Creates the default bounded policy.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the maximum resolved response size.
    pub fn max_response_bytes(mut self, bytes: usize) -> Result<Self, AssetError> {
        if bytes == 0 {
            return Err(AssetError::new(
                AssetOperation::Validate,
                AssetErrorKind::ResourceLimit,
                "maximum response size must be positive",
            ));
        }
        self.max_response_bytes = bytes;
        Ok(self)
    }

    /// Sets the maximum width and height accepted in response metadata.
    pub fn max_dimension(mut self, dimension: u32) -> Result<Self, AssetError> {
        if dimension == 0 {
            return Err(AssetError::new(
                AssetOperation::Validate,
                AssetErrorKind::ResourceLimit,
                "maximum image dimension must be positive",
            ));
        }
        self.max_dimension = dimension;
        Ok(self)
    }

    /// Adds an allowed exact HTTP(S) origin, such as `https://cdn.example`.
    pub fn allow_network_origin(mut self, origin: impl Into<String>) -> Self {
        self.allowed_network_origins
            .insert(origin.into().trim_end_matches('/').to_ascii_lowercase());
        self
    }

    /// Adds a MIME type allowed by the response policy.
    pub fn allow_mime_type(mut self, mime_type: impl Into<String>) -> Self {
        self.allowed_mime_types
            .insert(mime_type.into().to_ascii_lowercase());
        self
    }

    /// Allows absolute file paths when the host explicitly owns that boundary.
    pub fn allow_absolute_files(mut self, allow: bool) -> Self {
        self.allow_absolute_files = allow;
        self
    }

    /// Validates a source before a resolver is started.
    pub fn validate_source(&self, asset: &AssetRef) -> Result<(), AssetError> {
        let error = match asset.source() {
            AssetSource::Bundled(key) => validate_bundle_key(key),
            AssetSource::File(path) => validate_file_path(path, self.allow_absolute_files),
            AssetSource::Network { url, headers } => {
                validate_network_source(url, headers, &self.allowed_network_origins)
            }
            AssetSource::LegacyId(_) => Ok(()),
        };
        error.map_err(|error| error.with_source(asset.id().clone()))
    }

    /// Validates resolved bytes, metadata, and configured response limits.
    pub fn validate_data(&self, asset: &AssetRef, data: &AssetData) -> Result<(), AssetError> {
        if data.len() > self.max_response_bytes {
            return Err(AssetError::new(
                AssetOperation::Load,
                AssetErrorKind::ResourceLimit,
                format!(
                    "response is {} bytes, above the {} byte limit",
                    data.len(),
                    self.max_response_bytes
                ),
            )
            .with_source(asset.id().clone()));
        }
        if let Some(width) = data.metadata().width()
            && width > self.max_dimension
        {
            return Err(AssetError::new(
                AssetOperation::Load,
                AssetErrorKind::ResourceLimit,
                "asset width exceeds the configured dimension limit",
            )
            .with_source(asset.id().clone()));
        }
        if let Some(height) = data.metadata().height()
            && height > self.max_dimension
        {
            return Err(AssetError::new(
                AssetOperation::Load,
                AssetErrorKind::ResourceLimit,
                "asset height exceeds the configured dimension limit",
            )
            .with_source(asset.id().clone()));
        }
        if let Some(mime_type) = data.metadata().mime_type()
            && !self.allowed_mime_types.is_empty()
            && !self
                .allowed_mime_types
                .contains(&mime_type.to_ascii_lowercase())
        {
            return Err(AssetError::new(
                AssetOperation::Load,
                AssetErrorKind::UnsupportedFormat,
                format!("MIME type '{mime_type}' is not allowed"),
            )
            .with_source(asset.id().clone()));
        }
        Ok(())
    }
}

fn validate_bundle_key(key: &str) -> Result<(), AssetError> {
    if key.is_empty() || key.starts_with('/') || key.contains('\\') {
        return Err(AssetError::new(
            AssetOperation::Validate,
            AssetErrorKind::UnsafePath,
            "bundled asset keys must be non-empty relative paths",
        ));
    }
    validate_relative_components(Path::new(key))
}

fn validate_file_path(path: &Path, allow_absolute: bool) -> Result<(), AssetError> {
    if path.as_os_str().is_empty() || path.to_string_lossy().chars().any(char::is_control) {
        return Err(AssetError::new(
            AssetOperation::Validate,
            AssetErrorKind::UnsafePath,
            "file asset paths must be non-empty and free of control characters",
        ));
    }
    if path.is_absolute() && !allow_absolute {
        return Err(AssetError::new(
            AssetOperation::Validate,
            AssetErrorKind::UnsafePath,
            "absolute file paths require explicit policy opt-in",
        ));
    }
    validate_relative_components(path)
}

fn validate_relative_components(path: &Path) -> Result<(), AssetError> {
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir | Component::Prefix(_)))
    {
        return Err(AssetError::new(
            AssetOperation::Validate,
            AssetErrorKind::UnsafePath,
            "asset paths cannot contain parent traversal",
        ));
    }
    Ok(())
}

fn validate_network_source(
    url: &str,
    headers: &BTreeMap<String, String>,
    allowed_origins: &BTreeSet<String>,
) -> Result<(), AssetError> {
    if url.chars().any(char::is_control) {
        return Err(AssetError::new(
            AssetOperation::Validate,
            AssetErrorKind::InvalidSource,
            "network URLs cannot contain control characters",
        ));
    }
    let Some(origin) = network_origin(url) else {
        return Err(AssetError::new(
            AssetOperation::Validate,
            AssetErrorKind::InvalidSource,
            "network assets require an HTTP(S) URL",
        ));
    };
    if !allowed_origins.is_empty() && !allowed_origins.contains(&origin) {
        return Err(AssetError::new(
            AssetOperation::Validate,
            AssetErrorKind::NetworkOriginDenied,
            format!("network origin '{origin}' is not allowed"),
        ));
    }
    if headers.iter().any(|(name, value)| {
        name.is_empty() || name.chars().any(char::is_control) || value.chars().any(char::is_control)
    }) {
        return Err(AssetError::new(
            AssetOperation::Validate,
            AssetErrorKind::InvalidSource,
            "network headers cannot be empty or contain control characters",
        ));
    }
    Ok(())
}

fn network_origin(url: &str) -> Option<String> {
    let lower = url.to_ascii_lowercase();
    let scheme = if lower.starts_with("https://") {
        "https://"
    } else if lower.starts_with("http://") {
        "http://"
    } else {
        return None;
    };
    let authority = url[scheme.len()..]
        .split(['/', '?', '#'])
        .next()
        .filter(|authority| !authority.is_empty())?;
    Some(format!("{}{}", scheme, authority.to_ascii_lowercase()))
}

/// Metadata produced by a resolver alongside decoded or raw bytes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssetMetadata {
    mime_type: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    animated: bool,
}

impl AssetMetadata {
    /// Creates metadata for a response.
    pub fn new(
        mime_type: Option<impl Into<String>>,
        width: Option<u32>,
        height: Option<u32>,
        animated: bool,
    ) -> Self {
        Self {
            mime_type: mime_type.map(Into::into),
            width,
            height,
            animated,
        }
    }

    /// Returns the optional MIME type.
    pub fn mime_type(&self) -> Option<&str> {
        self.mime_type.as_deref()
    }

    /// Returns the optional decoded width.
    pub const fn width(&self) -> Option<u32> {
        self.width
    }

    /// Returns the optional decoded height.
    pub const fn height(&self) -> Option<u32> {
        self.height
    }

    /// Returns whether the source contains animation.
    pub const fn animated(&self) -> bool {
        self.animated
    }
}

/// Resolved bytes and metadata owned through a reference-counted byte slice.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssetData {
    bytes: Arc<[u8]>,
    metadata: AssetMetadata,
}

impl AssetData {
    /// Creates data with empty metadata.
    pub fn new(bytes: Vec<u8>) -> Self {
        Self {
            bytes: Arc::from(bytes),
            metadata: AssetMetadata::new(None::<String>, None, None, false),
        }
    }

    /// Replaces the response metadata.
    pub fn with_metadata(mut self, metadata: AssetMetadata) -> Self {
        self.metadata = metadata;
        self
    }

    /// Returns the resolved bytes without copying.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Returns response metadata.
    pub fn metadata(&self) -> &AssetMetadata {
        &self.metadata
    }

    /// Returns the number of resolved bytes.
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Returns whether no bytes were resolved.
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

/// Progress for a load operation. `None` means that the source has no known total.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AssetProgress(Option<f32>);

impl AssetProgress {
    /// Creates determinate progress in the inclusive range `0..=1`.
    pub fn known(fraction: f32) -> Result<Self, AssetError> {
        if !fraction.is_finite() || !(0.0..=1.0).contains(&fraction) {
            return Err(AssetError::new(
                AssetOperation::Load,
                AssetErrorKind::InvalidSource,
                "asset progress must be finite and between zero and one",
            ));
        }
        Ok(Self(Some(fraction)))
    }

    /// Creates indeterminate progress.
    pub const fn unknown() -> Self {
        Self(None)
    }

    /// Returns the known fraction, or `None` for indeterminate progress.
    pub const fn fraction(self) -> Option<f32> {
        self.0
    }
}

/// The policy for animated image sources.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum AnimationPolicy {
    /// Decode only the first frame of an animated source.
    FirstFrame,
    /// Preserve an animated source when the platform supports it.
    Preserve,
}

/// Decode options that participate in cache identity.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct DecodeProfile {
    target_size: Option<(u32, u32)>,
    normalize_orientation: bool,
    animation: AnimationPolicy,
}

impl Default for DecodeProfile {
    fn default() -> Self {
        Self {
            target_size: None,
            normalize_orientation: true,
            animation: AnimationPolicy::FirstFrame,
        }
    }
}

impl DecodeProfile {
    /// Creates the default bounded decode profile.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a positive target size for decoder downscaling.
    pub fn target_size(mut self, width: u32, height: u32) -> Result<Self, AssetError> {
        if width == 0 || height == 0 {
            return Err(AssetError::new(
                AssetOperation::Validate,
                AssetErrorKind::ResourceLimit,
                "decode target dimensions must be positive",
            ));
        }
        self.target_size = Some((width, height));
        Ok(self)
    }

    /// Selects whether EXIF/image orientation should be normalized.
    pub fn normalize_orientation(mut self, normalize: bool) -> Self {
        self.normalize_orientation = normalize;
        self
    }

    /// Selects the animated-image policy.
    pub fn animation_policy(mut self, animation: AnimationPolicy) -> Self {
        self.animation = animation;
        self
    }

    /// Returns the optional target size.
    pub const fn target_dimensions(&self) -> Option<(u32, u32)> {
        self.target_size
    }

    /// Returns whether orientation normalization is enabled.
    pub const fn orientation_is_normalized(&self) -> bool {
        self.normalize_orientation
    }

    /// Returns the animated-image policy.
    pub const fn animation_policy_value(&self) -> AnimationPolicy {
        self.animation
    }
}

/// A source/version/decode combination used for cache identity.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AssetCacheKey {
    asset_id: AssetId,
    version: u64,
    profile: DecodeProfile,
}

impl AssetCacheKey {
    /// Returns the source identity portion of the key.
    pub fn asset_id(&self) -> &AssetId {
        &self.asset_id
    }

    /// Returns the caller-provided asset version.
    pub const fn version(&self) -> u64 {
        self.version
    }

    /// Returns the decode options portion of the key.
    pub fn profile(&self) -> &DecodeProfile {
        &self.profile
    }
}

/// A load request for a source and its request variants.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssetRequest {
    asset: AssetRef,
    version: u64,
    profile: DecodeProfile,
    allow_stale: bool,
}

impl AssetRequest {
    /// Creates a request for the current version and default decode profile.
    pub fn new(asset: AssetRef) -> Self {
        Self {
            asset,
            version: 0,
            profile: DecodeProfile::default(),
            allow_stale: false,
        }
    }

    /// Sets a version token that invalidates older cache entries.
    pub const fn version(mut self, version: u64) -> Self {
        self.version = version;
        self
    }

    /// Sets decoder options that form part of the cache key.
    pub fn decode_profile(mut self, profile: DecodeProfile) -> Self {
        self.profile = profile;
        self
    }

    /// Allows the newest older cached version to be shown if resolution fails.
    pub const fn allow_stale(mut self, allow: bool) -> Self {
        self.allow_stale = allow;
        self
    }

    /// Returns the referenced source.
    pub fn asset(&self) -> &AssetRef {
        &self.asset
    }

    /// Returns the request version.
    pub const fn version_value(&self) -> u64 {
        self.version
    }

    /// Returns the decode profile.
    pub fn profile(&self) -> &DecodeProfile {
        &self.profile
    }

    /// Returns whether stale fallback is enabled.
    pub const fn stale_fallback_enabled(&self) -> bool {
        self.allow_stale
    }

    /// Returns the complete cache/deduplication key.
    pub fn cache_key(&self) -> AssetCacheKey {
        AssetCacheKey {
            asset_id: self.asset.id().clone(),
            version: self.version,
            profile: self.profile.clone(),
        }
    }
}

/// A poll result returned by a platform resolver.
#[derive(Debug)]
pub enum AssetLoadPoll {
    /// The request remains active with optional progress.
    Pending(AssetProgress),
    /// The request completed with data.
    Ready(AssetData),
    /// The request failed with a retained diagnostic.
    Failed(AssetError),
    /// The platform acknowledged cancellation.
    Cancelled,
}

/// A cancellable, poll-based resolver operation.
pub trait AssetLoadOperation: Send {
    /// Advances the operation without blocking the render thread.
    fn poll(&mut self) -> AssetLoadPoll;

    /// Requests cancellation of the underlying platform operation.
    fn cancel(&mut self);
}

/// Resolves an [`AssetRequest`] into a cancellable load operation.
pub trait AssetResolver: Send + Sync {
    /// Starts a request or returns a typed start failure.
    fn start(&self, request: &AssetRequest) -> Result<Box<dyn AssetLoadOperation>, AssetError>;
}

/// A handle to a deduplicated load operation.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct LoadHandle(u64);

impl LoadHandle {
    /// Returns the process-local handle number.
    pub const fn id(self) -> u64 {
        self.0
    }
}

/// The externally visible lifecycle of a requested asset.
#[derive(Clone, Debug, PartialEq)]
pub enum LoadState {
    /// The resolver is active.
    Loading {
        /// Current known or unknown progress.
        progress: AssetProgress,
        /// One-based attempt number.
        attempt: u32,
    },
    /// The asset is available, optionally from an older cache version.
    Ready {
        /// Loaded bytes and metadata.
        data: AssetData,
        /// Whether the resolver failed and an older version was retained.
        stale: bool,
        /// The failed refresh diagnostic when `stale` is true.
        stale_error: Option<AssetError>,
    },
    /// The resolver or response policy failed.
    Error {
        /// The retained source/operation diagnostic.
        error: AssetError,
        /// One-based attempt number.
        attempt: u32,
    },
    /// The caller cancelled the operation.
    Cancelled {
        /// One-based attempt number.
        attempt: u32,
    },
}

/// Limits for the in-memory decoded/raw asset cache.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AssetCacheConfig {
    max_entries: usize,
    max_bytes: usize,
}

impl AssetCacheConfig {
    /// Creates positive entry and byte limits.
    pub fn new(max_entries: usize, max_bytes: usize) -> Result<Self, AssetError> {
        if max_entries == 0 || max_bytes == 0 {
            return Err(AssetError::new(
                AssetOperation::Cache,
                AssetErrorKind::ResourceLimit,
                "cache limits must be positive",
            ));
        }
        Ok(Self {
            max_entries,
            max_bytes,
        })
    }

    /// Returns the entry limit.
    pub const fn max_entries(self) -> usize {
        self.max_entries
    }

    /// Returns the byte limit.
    pub const fn max_bytes(self) -> usize {
        self.max_bytes
    }
}

impl Default for AssetCacheConfig {
    fn default() -> Self {
        Self {
            max_entries: 256,
            max_bytes: 64 * 1024 * 1024,
        }
    }
}

/// Current bounded-cache usage.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CacheStats {
    entries: usize,
    bytes: usize,
    max_entries: usize,
    max_bytes: usize,
}

impl CacheStats {
    /// Returns the number of retained entries.
    pub const fn entries(self) -> usize {
        self.entries
    }

    /// Returns the number of retained bytes.
    pub const fn bytes(self) -> usize {
        self.bytes
    }

    /// Returns the configured entry limit.
    pub const fn max_entries(self) -> usize {
        self.max_entries
    }

    /// Returns the configured byte limit.
    pub const fn max_bytes(self) -> usize {
        self.max_bytes
    }
}

struct CacheEntry {
    data: AssetData,
    last_used: u64,
}

struct AssetCache {
    config: AssetCacheConfig,
    entries: HashMap<AssetCacheKey, CacheEntry>,
    bytes: usize,
    clock: u64,
}

impl AssetCache {
    fn new(config: AssetCacheConfig) -> Self {
        Self {
            config,
            entries: HashMap::new(),
            bytes: 0,
            clock: 0,
        }
    }

    fn touch(&mut self) -> u64 {
        self.clock = self.clock.wrapping_add(1);
        self.clock
    }

    fn get(&mut self, key: &AssetCacheKey) -> Option<AssetData> {
        let used = self.touch();
        self.entries.get_mut(key).map(|entry| {
            entry.last_used = used;
            entry.data.clone()
        })
    }

    fn stale_for(
        &mut self,
        asset_id: &AssetId,
        profile: &DecodeProfile,
        requested_version: u64,
    ) -> Option<AssetData> {
        let key = self
            .entries
            .keys()
            .filter(|key| {
                key.asset_id.eq(asset_id)
                    && key.profile.eq(profile)
                    && key.version < requested_version
            })
            .max_by_key(|key| key.version)
            .cloned()?;
        self.get(&key)
    }

    fn insert(&mut self, key: AssetCacheKey, data: AssetData) {
        if data.len() > self.config.max_bytes {
            return;
        }
        let used = self.touch();
        if let Some(previous) = self.entries.insert(
            key,
            CacheEntry {
                data: data.clone(),
                last_used: used,
            },
        ) {
            self.bytes = self.bytes.saturating_sub(previous.data.len());
        }
        self.bytes = self.bytes.saturating_add(data.len());
        self.evict();
    }

    fn evict(&mut self) {
        while self.entries.len() > self.config.max_entries || self.bytes > self.config.max_bytes {
            let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            if let Some(entry) = self.entries.remove(&oldest) {
                self.bytes = self.bytes.saturating_sub(entry.data.len());
            }
        }
    }

    fn invalidate(&mut self, asset_id: &AssetId) {
        let keys = self
            .entries
            .keys()
            .filter(|key| key.asset_id.eq(asset_id))
            .cloned()
            .collect::<Vec<_>>();
        for key in keys {
            if let Some(entry) = self.entries.remove(&key) {
                self.bytes = self.bytes.saturating_sub(entry.data.len());
            }
        }
    }

    fn clear(&mut self) {
        self.entries.clear();
        self.bytes = 0;
    }

    fn stats(&self) -> CacheStats {
        CacheStats {
            entries: self.entries.len(),
            bytes: self.bytes,
            max_entries: self.config.max_entries,
            max_bytes: self.config.max_bytes,
        }
    }
}

struct RequestEntry {
    request: AssetRequest,
    state: LoadState,
    operation: Option<Box<dyn AssetLoadOperation>>,
    attempt: u32,
}

/// Coordinates manifest validation, deduplicated loading, and bounded caching.
pub struct AssetManager<R> {
    resolver: R,
    policy: AssetPolicy,
    cache: AssetCache,
    manifest: AssetManifest,
    entries: HashMap<LoadHandle, RequestEntry>,
    inflight: HashMap<AssetCacheKey, LoadHandle>,
    next_handle: u64,
}

impl<R: AssetResolver> AssetManager<R> {
    /// Creates a manager with default policy and cache limits.
    pub fn new(resolver: R) -> Self {
        Self::with_options(
            resolver,
            AssetPolicy::default(),
            AssetCacheConfig::default(),
        )
    }

    /// Creates a manager with an explicit security policy.
    pub fn with_policy(resolver: R, policy: AssetPolicy) -> Self {
        Self::with_options(resolver, policy, AssetCacheConfig::default())
    }

    /// Creates a manager with explicit cache limits.
    pub fn with_cache_config(resolver: R, config: AssetCacheConfig) -> Self {
        Self::with_options(resolver, AssetPolicy::default(), config)
    }

    /// Creates a manager with explicit security and cache policies.
    pub fn with_options(resolver: R, policy: AssetPolicy, config: AssetCacheConfig) -> Self {
        Self {
            resolver,
            policy,
            cache: AssetCache::new(config),
            manifest: AssetManifest::new(),
            entries: HashMap::new(),
            inflight: HashMap::new(),
            next_handle: 0,
        }
    }

    /// Registers a manifest source for later resolution by callers.
    pub fn register(&mut self, asset: AssetRef) -> Result<(), AssetError> {
        self.manifest.register(asset)
    }

    /// Returns the current manifest.
    pub fn manifest(&self) -> &AssetManifest {
        &self.manifest
    }

    /// Starts or joins a request. Equal source/version/profile requests share one operation.
    pub fn request(&mut self, request: AssetRequest) -> Result<LoadHandle, AssetError> {
        self.policy.validate_source(request.asset())?;
        let key = request.cache_key();
        if let Some(handle) = self.inflight.get(&key).copied() {
            return Ok(handle);
        }
        let handle = self.allocate_handle();
        if let Some(data) = self.cache.get(&key) {
            self.entries.insert(
                handle,
                RequestEntry {
                    request,
                    state: LoadState::Ready {
                        data,
                        stale: false,
                        stale_error: None,
                    },
                    operation: None,
                    attempt: 0,
                },
            );
            return Ok(handle);
        }

        let attempt = 1;
        let (operation, state) = match self.resolver.start(&request) {
            Ok(operation) => (
                Some(operation),
                LoadState::Loading {
                    progress: AssetProgress::unknown(),
                    attempt,
                },
            ),
            Err(error) => (
                None,
                LoadState::Error {
                    error: error.with_source(request.asset().id().clone()),
                    attempt,
                },
            ),
        };
        if operation.is_some() {
            self.inflight.insert(key, handle);
        }
        self.entries.insert(
            handle,
            RequestEntry {
                request,
                state,
                operation,
                attempt,
            },
        );
        Ok(handle)
    }

    /// Starts a request intended to warm the cache before a widget needs it.
    pub fn preload(&mut self, request: AssetRequest) -> Result<LoadHandle, AssetError> {
        self.request(request)
    }

    /// Resolves a registered manifest identity and starts its default request.
    pub fn request_registered(&mut self, asset_id: &AssetId) -> Result<LoadHandle, AssetError> {
        let asset = self.manifest.resolve(asset_id)?.clone();
        self.request(AssetRequest::new(asset))
    }

    /// Returns the last published lifecycle state for a handle.
    pub fn status(&self, handle: LoadHandle) -> Result<LoadState, AssetError> {
        self.entries
            .get(&handle)
            .map(|entry| entry.state.clone())
            .ok_or_else(|| Self::missing_handle(handle))
    }

    /// Polls a live resolver operation and publishes its next lifecycle state.
    pub fn poll(&mut self, handle: LoadHandle) -> Result<LoadState, AssetError> {
        let (request, poll) = {
            let entry = self
                .entries
                .get_mut(&handle)
                .ok_or_else(|| Self::missing_handle(handle))?;
            let Some(operation) = entry.operation.as_mut() else {
                return Ok(entry.state.clone());
            };
            (entry.request.clone(), operation.poll())
        };

        let key = request.cache_key();
        let next_state = match poll {
            AssetLoadPoll::Pending(progress) => LoadState::Loading {
                progress,
                attempt: self
                    .entries
                    .get(&handle)
                    .map(|entry| entry.attempt)
                    .unwrap_or(1),
            },
            AssetLoadPoll::Ready(data) => match self.policy.validate_data(request.asset(), &data) {
                Ok(()) => {
                    self.cache.insert(key.clone(), data.clone());
                    LoadState::Ready {
                        data,
                        stale: false,
                        stale_error: None,
                    }
                }
                Err(error) => LoadState::Error {
                    error,
                    attempt: self
                        .entries
                        .get(&handle)
                        .map(|entry| entry.attempt)
                        .unwrap_or(1),
                },
            },
            AssetLoadPoll::Failed(error) => {
                let error = error.with_source(request.asset().id().clone());
                if request.stale_fallback_enabled()
                    && let Some(data) = self.cache.stale_for(
                        request.asset().id(),
                        request.profile(),
                        request.version_value(),
                    )
                {
                    LoadState::Ready {
                        data,
                        stale: true,
                        stale_error: Some(error),
                    }
                } else {
                    LoadState::Error {
                        error,
                        attempt: self
                            .entries
                            .get(&handle)
                            .map(|entry| entry.attempt)
                            .unwrap_or(1),
                    }
                }
            }
            AssetLoadPoll::Cancelled => LoadState::Cancelled {
                attempt: self
                    .entries
                    .get(&handle)
                    .map(|entry| entry.attempt)
                    .unwrap_or(1),
            },
        };
        let terminal = !matches!(&next_state, LoadState::Loading { .. });
        let entry = self
            .entries
            .get_mut(&handle)
            .expect("handle was checked above");
        entry.state = next_state.clone();
        if terminal {
            entry.operation = None;
            self.inflight.remove(&key);
        }
        Ok(next_state)
    }

    /// Cancels a deduplicated operation and publishes `Cancelled` to all joiners.
    pub fn cancel(&mut self, handle: LoadHandle) -> Result<LoadState, AssetError> {
        let (key, attempt) = {
            let entry = self
                .entries
                .get_mut(&handle)
                .ok_or_else(|| Self::missing_handle(handle))?;
            if let Some(operation) = entry.operation.as_mut() {
                operation.cancel();
            }
            entry.operation = None;
            entry.state = LoadState::Cancelled {
                attempt: entry.attempt,
            };
            (entry.request.cache_key(), entry.attempt)
        };
        self.inflight.remove(&key);
        Ok(LoadState::Cancelled { attempt })
    }

    /// Retries a terminal error or cancellation using a fresh resolver operation.
    pub fn retry(&mut self, handle: LoadHandle) -> Result<LoadState, AssetError> {
        let request = {
            let entry = self
                .entries
                .get(&handle)
                .ok_or_else(|| Self::missing_handle(handle))?;
            if !matches!(
                &entry.state,
                LoadState::Error { .. } | LoadState::Cancelled { .. }
            ) {
                return Err(AssetError::new(
                    AssetOperation::Retry,
                    AssetErrorKind::InvalidState,
                    "only an error or cancelled load can be retried",
                )
                .with_source(entry.request.asset().id().clone()));
            }
            entry.request.clone()
        };
        let key = request.cache_key();
        if self.inflight.contains_key(&key) {
            return Err(AssetError::new(
                AssetOperation::Retry,
                AssetErrorKind::InvalidState,
                "another request for this cache key is already active",
            )
            .with_source(request.asset().id().clone()));
        }
        let attempt = self
            .entries
            .get(&handle)
            .map(|entry| entry.attempt.saturating_add(1))
            .unwrap_or(1);
        let (operation, state) = match self.resolver.start(&request) {
            Ok(operation) => (
                Some(operation),
                LoadState::Loading {
                    progress: AssetProgress::unknown(),
                    attempt,
                },
            ),
            Err(error) => (
                None,
                LoadState::Error {
                    error: error.with_source(request.asset().id().clone()),
                    attempt,
                },
            ),
        };
        if operation.is_some() {
            self.inflight.insert(key, handle);
        }
        let entry = self
            .entries
            .get_mut(&handle)
            .expect("handle was checked above");
        entry.request = request;
        entry.operation = operation;
        entry.attempt = attempt;
        entry.state = state.clone();
        Ok(state)
    }

    /// Removes every cached request variant for a source identity.
    pub fn invalidate(&mut self, asset_id: &AssetId) {
        self.cache.invalidate(asset_id);
    }

    /// Removes all cached data while leaving live operations untouched.
    pub fn clear_cache(&mut self) {
        self.cache.clear();
    }

    /// Returns bounded-cache usage.
    pub fn cache_stats(&self) -> CacheStats {
        self.cache.stats()
    }

    fn allocate_handle(&mut self) -> LoadHandle {
        self.next_handle = self.next_handle.wrapping_add(1);
        LoadHandle(self.next_handle)
    }

    fn missing_handle(handle: LoadHandle) -> AssetError {
        AssetError::new(
            AssetOperation::Load,
            AssetErrorKind::NotFound,
            format!("load handle {} is not known", handle.id()),
        )
    }
}
