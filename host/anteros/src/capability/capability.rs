use std::cell::{Cell, RefCell};
use std::fmt;
use std::rc::Rc;

use sha2::{Digest, Sha256};

use crate::{
    AsyncCallbackEvent, AsyncTaskId, CapabilityPolicy, ManifestView, ModelError, ModelLimits,
    StableId128,
};

const CONTRACT_DERIVATION_DOMAIN: &[u8] = b"aimer.capability-contract.v1\0";

/// The canonical capability identity used by generated hot-reload guests for
/// bounded HTTP GET requests.
pub const PORTABLE_HTTP_CAPABILITY_NAME: &str = "aimer::portable_http";

/// The wire-contract major version used by the portable HTTP capability.
pub const PORTABLE_HTTP_CAPABILITY_ABI_MAJOR: u32 = 1;

/// The only method currently exposed by the portable HTTP capability.
pub const PORTABLE_HTTP_METHOD_GET: u32 = 0;

/// Maximum canonical URL request size accepted by the portable HTTP bridge.
pub const PORTABLE_HTTP_MAX_REQUEST_BYTES: u32 = 16 * 1024;

/// Maximum complete response image accepted by the portable HTTP bridge.
///
/// The generated guest materializes the response before decoding it, so this
/// ceiling is intentionally kept below the CLI safe profile's 4 MiB linear
/// memory limit. Keeping the response bounded also leaves room for the guest
/// runtime, request bytes, and the decoded body to coexist.
pub const PORTABLE_HTTP_MAX_RESPONSE_BYTES: u32 = 256 * 1024;

/// Bytes occupied by the status and length fields around an HTTP body.
pub const PORTABLE_HTTP_RESPONSE_OVERHEAD_BYTES: u32 = 2 + 4;

/// Returns the stable identity of the built-in portable HTTP capability.
#[inline]
pub fn portable_http_capability_id() -> StableId128 {
    StableId128::derive_capability(PORTABLE_HTTP_CAPABILITY_NAME)
}

/// Returns the fingerprint of the built-in portable HTTP wire contract.
#[inline]
pub fn portable_http_contract_fingerprint() -> [u8; 32] {
    capability_contract_fingerprint(
        b"aimer.portable_http.v1|get(url:string)->(status:u16,body:bytes)",
    )
}

/// Returns the maximum body size that fits in a portable HTTP response image.
#[inline]
pub const fn portable_http_max_body_bytes() -> u32 {
    PORTABLE_HTTP_MAX_RESPONSE_BYTES - PORTABLE_HTTP_RESPONSE_OVERHEAD_BYTES
}

/// Derives a capability contract fingerprint from its canonical wire schema.
///
/// The input is the versioned canonical declaration image produced by Aimer's
/// capability tooling. This function hashes the
/// `aimer.capability-contract.v1` domain, the image's little-endian `u64` byte
/// length, and the complete image. Documentation, SDK release metadata,
/// package versions, source locations, and implementation bodies do not belong
/// in that image.
#[inline]
pub fn capability_contract_fingerprint(canonical_contract: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(CONTRACT_DERIVATION_DOMAIN);
    hasher.update((canonical_contract.len() as u64).to_le_bytes());
    hasher.update(canonical_contract);
    hasher.finalize().into()
}

/// A stable failure reported by capability negotiation, transport, or codecs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CapabilityError {
    /// The permanent host does not provide the requested capability contract.
    Unsupported,
    /// Host policy rejected the operation.
    Denied,
    /// The provider exists but cannot currently serve the operation.
    Unavailable,
    /// The guest produced a malformed request.
    InvalidRequest,
    /// The host returned a malformed or non-canonical response.
    InvalidResponse,
    /// A request or response exceeded its declared byte limit.
    LimitExceeded,
    /// The owning guest generation has retired and cannot accept work.
    RetiredGeneration,
    /// The operation requires an active generation and cannot run while staging.
    NotActive,
}

/// The uniform result returned by native providers and generated guest proxies.
pub type CapabilityResult<T> = Result<T, CapabilityError>;

/// A reload-coordinator-assigned identity for one guest generation.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct GenerationId(u64);

impl GenerationId {
    /// Creates an explicit generation identity.
    #[inline]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the coordinator-assigned numeric identity.
    #[inline]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// The activity scope passed to native capability providers.
#[derive(Clone, Debug)]
pub struct CapabilityGeneration {
    id: GenerationId,
    lifecycle: Rc<Cell<CapabilityLifecycle>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CapabilityLifecycle {
    Candidate,
    Active,
    Retired,
}

impl CapabilityGeneration {
    fn candidate(id: GenerationId) -> Self {
        Self {
            id,
            lifecycle: Rc::new(Cell::new(CapabilityLifecycle::Candidate)),
        }
    }

    fn active(id: GenerationId) -> Self {
        Self {
            id,
            lifecycle: Rc::new(Cell::new(CapabilityLifecycle::Active)),
        }
    }

    /// Returns the reload-coordinator-assigned generation identity.
    #[inline]
    pub const fn id(&self) -> GenerationId {
        self.id
    }

    /// Creates a token that rejects a completion after this generation retires.
    #[inline]
    pub fn completion_token(&self) -> CapabilityCompletionToken {
        CapabilityCompletionToken {
            generation: self.clone(),
        }
    }

    fn ensure_active(&self) -> CapabilityResult<()> {
        match self.lifecycle.get() {
            CapabilityLifecycle::Candidate | CapabilityLifecycle::Active => Ok(()),
            CapabilityLifecycle::Retired => Err(CapabilityError::RetiredGeneration),
        }
    }

    #[inline]
    fn is_candidate(&self) -> bool {
        self.lifecycle.get() == CapabilityLifecycle::Candidate
    }

    #[inline]
    fn activate(&self) {
        if self.is_candidate() {
            self.lifecycle.set(CapabilityLifecycle::Active);
        }
    }

    fn retire(&self) {
        self.lifecycle.set(CapabilityLifecycle::Retired);
    }
}

/// A generation-tagged guard for asynchronous provider completion delivery.
#[derive(Clone, Debug)]
pub struct CapabilityCompletionToken {
    generation: CapabilityGeneration,
}

impl CapabilityCompletionToken {
    /// Returns the generation that initiated the asynchronous provider work.
    #[inline]
    pub const fn generation_id(&self) -> GenerationId {
        self.generation.id()
    }

    /// Accepts a completion until its initiating generation retires.
    #[inline]
    pub fn complete<T>(&self, value: T) -> CapabilityResult<T> {
        self.generation.ensure_active()?;
        Ok(value)
    }

    /// Encodes one generation-owned async callback completion.
    ///
    /// The capability token supplies the trusted generation identity. The
    /// active hot-reload host still validates the callback and task identities
    /// against its [`Generation`](crate::Generation) before dispatching the
    /// event, so a provider cannot complete a task belonging to another
    /// callback or generation by forging the `AASY` header.
    pub fn encode_async_completion(
        &self,
        callback_id: StableId128,
        task_id: AsyncTaskId,
        event_sequence: u64,
        payload: &[u8],
        limits: ModelLimits,
    ) -> CapabilityResult<Vec<u8>> {
        self.generation.ensure_active()?;
        AsyncCallbackEvent::complete(
            self.generation.id().get(),
            event_sequence,
            callback_id,
            task_id,
            payload,
        )
        .encode(limits)
        .map_err(capability_completion_encode_error)
    }
}

fn capability_completion_encode_error(error: ModelError) -> CapabilityError {
    match error {
        ModelError::BlobTooLarge { .. }
        | ModelError::DocumentTooLarge { .. }
        | ModelError::LengthOverflow => CapabilityError::LimitExceeded,
        _ => CapabilityError::InvalidResponse,
    }
}

/// The immutable contract and byte limits exposed by one native provider.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapabilityDescriptor {
    capability_id: StableId128,
    abi_major: u32,
    contract_fingerprint: [u8; 32],
    limits: CapabilityLimits,
}

/// Host policy controlling capability behavior while a generation is staging.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CapabilityStagingClass {
    /// A bounded query with no externally visible side effect.
    PureQuery,
    /// Read-only access that cannot mutate external state.
    ReadOnly,
    /// A dormant registration activated only when the generation commits.
    RegistrableResource,
    /// A validated asynchronous request queued until generation commit.
    ExternalRequest,
    /// A transient effect that is forbidden until the generation is active.
    IrreversibleEffect,
}

/// A validated dormant capability operation prepared by a native provider.
///
/// Dropping this value rolls the operation back by discarding its activation
/// closure. Activation is intentionally infallible: providers must complete all
/// fallible validation and reservation before returning this value.
pub struct StagedCapability {
    response: Vec<u8>,
    activation: Option<Box<dyn FnOnce()>>,
}

impl StagedCapability {
    /// Creates a dormant operation and its canonical immediate guest response.
    pub fn new(response: Vec<u8>, activation: impl FnOnce() + 'static) -> Self {
        Self {
            response,
            activation: Some(Box::new(activation)),
        }
    }

    fn response(&self) -> &[u8] {
        &self.response
    }

    fn activate(mut self) {
        if let Some(activation) = self.activation.take() {
            activation();
        }
    }
}

impl fmt::Debug for StagedCapability {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StagedCapability")
            .field("response_len", &self.response.len())
            .finish_non_exhaustive()
    }
}

impl CapabilityDescriptor {
    /// Creates a provider descriptor from generated contract metadata.
    #[inline]
    pub const fn new(
        capability_id: StableId128,
        abi_major: u32,
        contract_fingerprint: [u8; 32],
        limits: CapabilityLimits,
    ) -> Self {
        Self {
            capability_id,
            abi_major,
            contract_fingerprint,
            limits,
        }
    }

    /// Returns the stable package-scoped capability identity.
    #[inline]
    pub const fn capability_id(self) -> StableId128 {
        self.capability_id
    }

    /// Returns the incompatible wire-contract major version.
    #[inline]
    pub const fn abi_major(self) -> u32 {
        self.abi_major
    }

    /// Returns the deterministic wire-contract fingerprint.
    #[inline]
    pub const fn contract_fingerprint(self) -> [u8; 32] {
        self.contract_fingerprint
    }

    /// Returns the provider's request and response byte limits.
    #[inline]
    pub const fn limits(self) -> CapabilityLimits {
        self.limits
    }
}

/// A type-erased native implementation of one generated capability contract.
pub trait CapabilityProvider {
    /// Returns immutable metadata used for registration and negotiation.
    fn descriptor(&self) -> CapabilityDescriptor;

    /// Invokes one canonical method request and returns canonical response bytes.
    fn invoke(
        &self,
        generation: CapabilityGeneration,
        method_id: u32,
        request: &[u8],
        response_limit: u32,
    ) -> CapabilityResult<Vec<u8>>;

    /// Validates and prepares a dormant candidate operation.
    ///
    /// Providers registered as [`CapabilityStagingClass::RegistrableResource`]
    /// or [`CapabilityStagingClass::ExternalRequest`] must override this method.
    /// The default fails closed without invoking the active operation.
    fn stage(
        &self,
        _generation: CapabilityGeneration,
        _method_id: u32,
        _request: &[u8],
        _response_limit: u32,
    ) -> CapabilityResult<StagedCapability> {
        Err(CapabilityError::NotActive)
    }
}

/// A deterministic provider registration or manifest-negotiation failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CapabilityRegistryError {
    /// The registry already contains this stable capability identity.
    DuplicateProvider {
        /// The duplicated package-scoped capability identity.
        capability_id: StableId128,
    },
    /// Registering another provider would exceed the configured capacity.
    ProviderLimitExceeded {
        /// The maximum number of owned providers.
        limit: u32,
    },
    /// A required manifest capability has no registered provider.
    MissingRequiredProvider {
        /// The unavailable package-scoped capability identity.
        capability_id: StableId128,
    },
    /// A required provider uses a different incompatible ABI major.
    AbiMismatch {
        /// The package-scoped capability identity.
        capability_id: StableId128,
        /// The ABI major required by the guest manifest.
        required: u32,
        /// The ABI major implemented by the native provider.
        provided: u32,
    },
    /// A required provider's canonical wire contract differs from the guest.
    ContractMismatch {
        /// The package-scoped capability identity.
        capability_id: StableId128,
    },
}

/// An owned collection of permanent native capability providers.
pub struct CapabilityRegistry {
    providers: Vec<RegisteredProvider>,
    max_providers: u32,
}

#[derive(Clone)]
struct RegisteredProvider {
    provider: Rc<dyn CapabilityProvider>,
    staging: CapabilityStagingClass,
}

impl RegisteredProvider {
    fn descriptor(&self) -> CapabilityDescriptor {
        self.provider.descriptor()
    }
}

impl CapabilityRegistry {
    /// Creates an empty registry with an explicit provider-count limit.
    #[inline]
    pub const fn new(max_providers: u32) -> Self {
        Self {
            providers: Vec::new(),
            max_providers,
        }
    }

    /// Registers one provider as committed-only under its stable identity.
    ///
    /// The conservative default prevents an unclassified provider from causing
    /// side effects while a candidate generation is staging. Side-effect-free
    /// providers should use [`Self::register_with_staging`] explicitly.
    pub fn register<P>(&mut self, provider: P) -> Result<(), CapabilityRegistryError>
    where
        P: CapabilityProvider + 'static,
    {
        self.register_with_staging(provider, CapabilityStagingClass::IrreversibleEffect)
    }

    /// Registers a provider with explicit candidate-generation staging policy.
    pub fn register_with_staging<P>(
        &mut self,
        provider: P,
        staging: CapabilityStagingClass,
    ) -> Result<(), CapabilityRegistryError>
    where
        P: CapabilityProvider + 'static,
    {
        let descriptor = provider.descriptor();
        let index = self
            .providers
            .binary_search_by_key(&descriptor.capability_id(), |provider| {
                provider.descriptor().capability_id()
            });
        let index = match index {
            Ok(_) => {
                return Err(CapabilityRegistryError::DuplicateProvider {
                    capability_id: descriptor.capability_id(),
                });
            }
            Err(index) => index,
        };
        if self.providers.len() >= self.max_providers as usize {
            return Err(CapabilityRegistryError::ProviderLimitExceeded {
                limit: self.max_providers,
            });
        }
        self.providers.insert(
            index,
            RegisteredProvider {
                provider: Rc::new(provider),
                staging,
            },
        );
        Ok(())
    }

    /// Negotiates a validated guest manifest into generation-owned bindings.
    pub fn negotiate(
        &self,
        manifest: &ManifestView<'_>,
    ) -> Result<CapabilityBindings, CapabilityRegistryError> {
        self.negotiate_with_lifecycle(manifest, CapabilityGeneration::active(GenerationId::new(0)))
    }

    /// Negotiates exact providers for one reload-coordinator-owned generation.
    pub fn negotiate_generation(
        &self,
        manifest: &ManifestView<'_>,
        generation_id: GenerationId,
    ) -> Result<CapabilityBindings, CapabilityRegistryError> {
        self.negotiate_with_lifecycle(manifest, CapabilityGeneration::candidate(generation_id))
    }

    fn negotiate_with_lifecycle(
        &self,
        manifest: &ManifestView<'_>,
        generation: CapabilityGeneration,
    ) -> Result<CapabilityBindings, CapabilityRegistryError> {
        let mut bindings = Vec::with_capacity(manifest.capability_count() as usize);
        for requirement in manifest.capabilities() {
            let capability_id = requirement.capability_id();
            let provider = self
                .providers
                .binary_search_by_key(&capability_id, |provider| {
                    provider.descriptor().capability_id()
                })
                .ok()
                .map(|index| self.providers[index].clone());
            let provider = match provider {
                None if requirement.policy() == CapabilityPolicy::Required => {
                    return Err(CapabilityRegistryError::MissingRequiredProvider { capability_id });
                }
                None => None,
                Some(provider) => {
                    let descriptor = provider.descriptor();
                    if descriptor.abi_major() != requirement.abi_major() {
                        if requirement.policy() == CapabilityPolicy::Required {
                            return Err(CapabilityRegistryError::AbiMismatch {
                                capability_id,
                                required: requirement.abi_major(),
                                provided: descriptor.abi_major(),
                            });
                        }
                        None
                    } else if &descriptor.contract_fingerprint()
                        != requirement.contract_fingerprint()
                    {
                        if requirement.policy() == CapabilityPolicy::Required {
                            return Err(CapabilityRegistryError::ContractMismatch {
                                capability_id,
                            });
                        }
                        None
                    } else {
                        Some(provider)
                    }
                }
            };
            bindings.push(CapabilityBinding {
                capability_id,
                abi_major: requirement.abi_major(),
                staging: provider
                    .as_ref()
                    .map(|provider| provider.staging)
                    .unwrap_or(CapabilityStagingClass::PureQuery),
                provider: provider.map(|provider| provider.provider),
            });
        }
        Ok(CapabilityBindings {
            generation,
            bindings,
            staged: RefCell::new(Vec::new()),
        })
    }
}

impl fmt::Display for CapabilityRegistryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateProvider { capability_id } => {
                write!(formatter, "duplicate capability provider {capability_id:?}")
            }
            Self::ProviderLimitExceeded { limit } => {
                write!(formatter, "capability provider limit {limit} exceeded")
            }
            Self::MissingRequiredProvider { capability_id } => {
                write!(
                    formatter,
                    "missing required capability provider {capability_id:?}"
                )
            }
            Self::AbiMismatch {
                capability_id,
                required,
                provided,
            } => write!(
                formatter,
                "capability {capability_id:?} requires ABI {required} but host provides {provided}"
            ),
            Self::ContractMismatch { capability_id } => {
                write!(
                    formatter,
                    "capability {capability_id:?} contract fingerprint mismatch"
                )
            }
        }
    }
}

impl std::error::Error for CapabilityRegistryError {}

struct CapabilityBinding {
    capability_id: StableId128,
    abi_major: u32,
    staging: CapabilityStagingClass,
    provider: Option<Rc<dyn CapabilityProvider>>,
}

/// Generation-owned exact provider bindings produced by manifest negotiation.
pub struct CapabilityBindings {
    generation: CapabilityGeneration,
    bindings: Vec<CapabilityBinding>,
    staged: RefCell<Vec<StagedCapability>>,
}

impl CapabilityBindings {
    /// Returns the reload-coordinator-assigned owner of these bindings.
    #[inline]
    pub const fn generation_id(&self) -> GenerationId {
        self.generation.id()
    }

    /// Permanently rejects new calls and pending completions for this generation.
    #[inline]
    pub fn retire(&self) {
        self.staged.borrow_mut().clear();
        self.generation.retire();
    }

    /// Returns whether this binding set has not retired.
    #[inline]
    pub fn is_active(&self) -> bool {
        self.generation.ensure_active().is_ok()
    }

    /// Atomically publishes every prevalidated dormant operation.
    ///
    /// This transition is idempotent. Operations are removed from staging
    /// before their infallible activation closures run.
    pub fn activate(&self) {
        if !self.generation.is_candidate() {
            return;
        }
        let staged = self.staged.take();
        self.generation.activate();
        for operation in staged {
            operation.activate();
        }
    }

    #[cfg(feature = "wasm-hot-reload")]
    pub(crate) fn request_limit(
        &self,
        capability_id: StableId128,
        abi_major: u32,
    ) -> CapabilityResult<u32> {
        Ok(self
            .binding(capability_id, abi_major)?
            .provider
            .as_ref()
            .expect("checked provider binding")
            .descriptor()
            .limits()
            .max_request_bytes())
    }

    fn binding(
        &self,
        capability_id: StableId128,
        abi_major: u32,
    ) -> CapabilityResult<&CapabilityBinding> {
        self.generation.ensure_active()?;
        self.bindings
            .binary_search_by_key(&capability_id, |binding| binding.capability_id)
            .ok()
            .map(|index| &self.bindings[index])
            .filter(|binding| binding.abi_major == abi_major)
            .filter(|binding| binding.provider.is_some())
            .ok_or(CapabilityError::Unsupported)
    }
}

impl fmt::Debug for CapabilityBindings {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CapabilityBindings")
            .field("generation_id", &self.generation.id())
            .field("binding_count", &self.bindings.len())
            .finish()
    }
}

impl Drop for CapabilityBindings {
    fn drop(&mut self) {
        self.retire();
    }
}

impl CapabilityTransport for CapabilityBindings {
    fn invoke(&self, call: CapabilityCall<'_>) -> CapabilityResult<Vec<u8>> {
        let binding = self.binding(call.capability_id(), call.abi_major())?;
        let provider = binding.provider.as_ref().expect("checked provider binding");
        let limits = provider.descriptor().limits();
        if call.request().len() > limits.max_request_bytes() as usize {
            return Err(CapabilityError::LimitExceeded);
        }
        let response_limit = call.response_limit().min(limits.max_response_bytes());
        if self.generation.is_candidate() {
            match binding.staging {
                CapabilityStagingClass::PureQuery | CapabilityStagingClass::ReadOnly => {}
                CapabilityStagingClass::RegistrableResource
                | CapabilityStagingClass::ExternalRequest => {
                    let operation = provider.stage(
                        self.generation.clone(),
                        call.method_id(),
                        call.request(),
                        response_limit,
                    )?;
                    if operation.response().len() > response_limit as usize {
                        return Err(CapabilityError::LimitExceeded);
                    }
                    let response = operation.response().to_vec();
                    self.staged.borrow_mut().push(operation);
                    return Ok(response);
                }
                CapabilityStagingClass::IrreversibleEffect => {
                    return Err(CapabilityError::NotActive);
                }
            }
        }
        let response = provider.invoke(
            self.generation.clone(),
            call.method_id(),
            call.request(),
            response_limit,
        )?;
        if response.len() > response_limit as usize {
            return Err(CapabilityError::LimitExceeded);
        }
        Ok(response)
    }
}

/// Per-call byte limits applied before a capability crosses a trust boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapabilityLimits {
    max_request_bytes: u32,
    max_response_bytes: u32,
}

impl CapabilityLimits {
    /// Creates explicit request and response byte limits.
    #[inline]
    pub const fn new(max_request_bytes: u32, max_response_bytes: u32) -> Self {
        Self {
            max_request_bytes,
            max_response_bytes,
        }
    }

    /// Returns the maximum canonical request size.
    #[inline]
    pub const fn max_request_bytes(self) -> u32 {
        self.max_request_bytes
    }

    /// Returns the maximum canonical response size.
    #[inline]
    pub const fn max_response_bytes(self) -> u32 {
        self.max_response_bytes
    }
}

/// One bounded invocation submitted by a generated guest proxy.
#[derive(Clone, Copy, Debug)]
pub struct CapabilityCall<'a> {
    capability_id: StableId128,
    abi_major: u32,
    method_id: u32,
    request: &'a [u8],
    response_limit: u32,
}

impl<'a> CapabilityCall<'a> {
    /// Creates a call from generated contract metadata and canonical bytes.
    #[inline]
    pub const fn new(
        capability_id: StableId128,
        abi_major: u32,
        method_id: u32,
        request: &'a [u8],
        response_limit: u32,
    ) -> Self {
        Self {
            capability_id,
            abi_major,
            method_id,
            request,
            response_limit,
        }
    }

    /// Returns the stable manifest capability identity.
    #[inline]
    pub const fn capability_id(&self) -> StableId128 {
        self.capability_id
    }

    /// Returns the incompatible capability ABI major.
    #[inline]
    pub const fn abi_major(&self) -> u32 {
        self.abi_major
    }

    /// Returns the canonical method index.
    #[inline]
    pub const fn method_id(&self) -> u32 {
        self.method_id
    }

    /// Borrows the complete canonical request image.
    #[inline]
    pub const fn request(&self) -> &'a [u8] {
        self.request
    }

    /// Returns the maximum response size accepted by the guest.
    #[inline]
    pub const fn response_limit(&self) -> u32 {
        self.response_limit
    }
}

/// The target-specific bridge used by generated WASM guest proxies.
///
/// Implementations may call a WebAssembly host import, an in-process test
/// provider, or another bounded transport. They must return complete response
/// bytes and must reject responses larger than [`CapabilityCall::response_limit`].
pub trait CapabilityTransport {
    /// Invokes one canonical capability operation.
    fn invoke(&self, call: CapabilityCall<'_>) -> CapabilityResult<Vec<u8>>;
}

/// The native-interpreter import transport used by generated WASM guest proxies.
#[cfg(target_arch = "wasm32")]
#[derive(Clone, Copy, Debug, Default)]
pub struct WasmCapabilityTransport;

#[cfg(target_arch = "wasm32")]
impl WasmCapabilityTransport {
    /// Creates the zero-sized transport for the current guest generation.
    #[inline]
    pub const fn new() -> Self {
        Self
    }
}

#[cfg(target_arch = "wasm32")]
impl CapabilityTransport for WasmCapabilityTransport {
    fn invoke(&self, call: CapabilityCall<'_>) -> CapabilityResult<Vec<u8>> {
        let request_length =
            i32::try_from(call.request().len()).map_err(|_| CapabilityError::LimitExceeded)?;
        let output_capacity =
            i32::try_from(call.response_limit()).map_err(|_| CapabilityError::LimitExceeded)?;
        let capability_id_pointer =
            i32::try_from(call.capability_id().as_bytes().as_ptr() as usize)
                .map_err(|_| CapabilityError::InvalidRequest)?;
        let request_pointer = i32::try_from(call.request().as_ptr() as usize)
            .map_err(|_| CapabilityError::InvalidRequest)?;
        let mut response = vec![0_u8; call.response_limit() as usize];
        let output_pointer = i32::try_from(response.as_mut_ptr() as usize)
            .map_err(|_| CapabilityError::LimitExceeded)?;

        // SAFETY: All pointers refer to live guest-owned slices for the full
        // synchronous call. The host validates every range and never retains a
        // pointer after the import returns.
        let packed = unsafe {
            aimer_capability_call(
                capability_id_pointer,
                call.abi_major() as i32,
                call.method_id() as i32,
                request_pointer,
                request_length,
                output_pointer,
                output_capacity,
            )
        };
        let result =
            crate::AbiResult::from_packed(packed).map_err(|_| CapabilityError::InvalidResponse)?;
        match result.status() {
            crate::AbiStatus::Ok if result.value() <= call.response_limit() => {
                response.truncate(result.value() as usize);
                Ok(response)
            }
            crate::AbiStatus::BufferTooSmall | crate::AbiStatus::ResourceExhausted => {
                Err(CapabilityError::LimitExceeded)
            }
            crate::AbiStatus::UnknownId | crate::AbiStatus::UnsupportedVersion => {
                Err(CapabilityError::Unsupported)
            }
            crate::AbiStatus::CapabilityDenied => Err(CapabilityError::Denied),
            crate::AbiStatus::NotActive => Err(CapabilityError::NotActive),
            crate::AbiStatus::InvalidArgument | crate::AbiStatus::MalformedMessage => {
                Err(CapabilityError::InvalidRequest)
            }
            crate::AbiStatus::RetiredGeneration => Err(CapabilityError::RetiredGeneration),
            _ => Err(CapabilityError::InvalidResponse),
        }
    }
}

#[cfg(target_arch = "wasm32")]
#[link(wasm_import_module = "aimer")]
unsafe extern "C" {
    #[link_name = "capability_call"]
    fn aimer_capability_call(
        capability_id_pointer: i32,
        abi_major: i32,
        method_id: i32,
        request_pointer: i32,
        request_length: i32,
        output_pointer: i32,
        output_capacity: i32,
    ) -> i64;
}

/// A bounded canonical encoder used by generated capability proxies.
pub struct CapabilityEncoder {
    bytes: Vec<u8>,
    limit: u32,
}

impl CapabilityEncoder {
    /// Creates an empty encoder with an explicit byte limit.
    #[inline]
    pub const fn new(limit: u32) -> Self {
        Self {
            bytes: Vec::new(),
            limit,
        }
    }

    /// Appends one canonical boolean.
    #[inline]
    pub fn write_bool(&mut self, value: bool) -> CapabilityResult<()> {
        self.write_u8(u8::from(value))
    }

    /// Appends one byte.
    #[inline]
    pub fn write_u8(&mut self, value: u8) -> CapabilityResult<()> {
        self.extend(&[value])
    }

    /// Appends one signed byte.
    #[inline]
    pub fn write_i8(&mut self, value: i8) -> CapabilityResult<()> {
        self.extend(&value.to_le_bytes())
    }

    /// Appends one little-endian `u16`.
    #[inline]
    pub fn write_u16(&mut self, value: u16) -> CapabilityResult<()> {
        self.extend(&value.to_le_bytes())
    }

    /// Appends one little-endian `i16`.
    #[inline]
    pub fn write_i16(&mut self, value: i16) -> CapabilityResult<()> {
        self.extend(&value.to_le_bytes())
    }

    /// Appends one little-endian `u32`.
    #[inline]
    pub fn write_u32(&mut self, value: u32) -> CapabilityResult<()> {
        self.extend(&value.to_le_bytes())
    }

    /// Appends one little-endian `i32`.
    #[inline]
    pub fn write_i32(&mut self, value: i32) -> CapabilityResult<()> {
        self.extend(&value.to_le_bytes())
    }

    /// Appends one little-endian `u64`.
    #[inline]
    pub fn write_u64(&mut self, value: u64) -> CapabilityResult<()> {
        self.extend(&value.to_le_bytes())
    }

    /// Appends one little-endian `i64`.
    #[inline]
    pub fn write_i64(&mut self, value: i64) -> CapabilityResult<()> {
        self.extend(&value.to_le_bytes())
    }

    /// Appends one canonical `f32` bit pattern.
    #[inline]
    pub fn write_f32(&mut self, value: f32) -> CapabilityResult<()> {
        self.write_u32(value.to_bits())
    }

    /// Appends one canonical `f64` bit pattern.
    #[inline]
    pub fn write_f64(&mut self, value: f64) -> CapabilityResult<()> {
        self.write_u64(value.to_bits())
    }

    /// Appends one length-prefixed UTF-8 string.
    pub fn write_string(&mut self, value: &str) -> CapabilityResult<()> {
        self.write_length(value.len())?;
        self.extend(value.as_bytes())
    }

    /// Appends one length-prefixed byte string.
    pub fn write_bytes(&mut self, value: &[u8]) -> CapabilityResult<()> {
        self.write_length(value.len())?;
        self.extend(value)
    }

    /// Finishes the request without another allocation.
    #[inline]
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    fn write_length(&mut self, length: usize) -> CapabilityResult<()> {
        let length = u32::try_from(length).map_err(|_| CapabilityError::LimitExceeded)?;
        self.write_u32(length)
    }

    fn extend(&mut self, value: &[u8]) -> CapabilityResult<()> {
        let length = self
            .bytes
            .len()
            .checked_add(value.len())
            .ok_or(CapabilityError::LimitExceeded)?;
        if length > self.limit as usize {
            return Err(CapabilityError::LimitExceeded);
        }
        self.bytes.extend_from_slice(value);
        Ok(())
    }
}

/// A bounded zero-copy decoder used by generated capability proxies.
pub struct CapabilityDecoder<'a> {
    bytes: &'a [u8],
    offset: usize,
    invalid_error: CapabilityError,
}

impl<'a> CapabilityDecoder<'a> {
    /// Validates the complete response size before exposing values.
    #[inline]
    pub fn new(bytes: &'a [u8], limit: u32) -> CapabilityResult<Self> {
        if bytes.len() > limit as usize {
            return Err(CapabilityError::LimitExceeded);
        }
        Ok(Self {
            bytes,
            offset: 0,
            invalid_error: CapabilityError::InvalidResponse,
        })
    }

    /// Validates a complete provider request before exposing values.
    #[inline]
    pub fn new_request(bytes: &'a [u8], limit: u32) -> CapabilityResult<Self> {
        if bytes.len() > limit as usize {
            return Err(CapabilityError::LimitExceeded);
        }
        Ok(Self {
            bytes,
            offset: 0,
            invalid_error: CapabilityError::InvalidRequest,
        })
    }

    /// Reads one canonical boolean.
    pub fn read_bool(&mut self) -> CapabilityResult<bool> {
        match self.read_u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(self.invalid_error),
        }
    }

    /// Reads one byte.
    #[inline]
    pub fn read_u8(&mut self) -> CapabilityResult<u8> {
        Ok(self.take(1)?[0])
    }

    /// Reads one signed byte.
    #[inline]
    pub fn read_i8(&mut self) -> CapabilityResult<i8> {
        Ok(i8::from_le_bytes([self.read_u8()?]))
    }

    /// Reads one little-endian `u16`.
    #[inline]
    pub fn read_u16(&mut self) -> CapabilityResult<u16> {
        Ok(u16::from_le_bytes(self.take_array()?))
    }

    /// Reads one little-endian `i16`.
    #[inline]
    pub fn read_i16(&mut self) -> CapabilityResult<i16> {
        Ok(i16::from_le_bytes(self.take_array()?))
    }

    /// Reads one little-endian `u32`.
    #[inline]
    pub fn read_u32(&mut self) -> CapabilityResult<u32> {
        Ok(u32::from_le_bytes(self.take_array()?))
    }

    /// Reads one little-endian `i32`.
    #[inline]
    pub fn read_i32(&mut self) -> CapabilityResult<i32> {
        Ok(i32::from_le_bytes(self.take_array()?))
    }

    /// Reads one little-endian `u64`.
    #[inline]
    pub fn read_u64(&mut self) -> CapabilityResult<u64> {
        Ok(u64::from_le_bytes(self.take_array()?))
    }

    /// Reads one little-endian `i64`.
    #[inline]
    pub fn read_i64(&mut self) -> CapabilityResult<i64> {
        Ok(i64::from_le_bytes(self.take_array()?))
    }

    /// Reads one canonical `f32` bit pattern.
    #[inline]
    pub fn read_f32(&mut self) -> CapabilityResult<f32> {
        Ok(f32::from_bits(self.read_u32()?))
    }

    /// Reads one canonical `f64` bit pattern.
    #[inline]
    pub fn read_f64(&mut self) -> CapabilityResult<f64> {
        Ok(f64::from_bits(self.read_u64()?))
    }

    /// Copies one validated UTF-8 string into an owned value.
    pub fn read_string(&mut self) -> CapabilityResult<String> {
        let length = self.read_u32()? as usize;
        let bytes = self.take(length)?;
        let value = std::str::from_utf8(bytes).map_err(|_| self.invalid_error)?;
        Ok(value.to_owned())
    }

    /// Copies one bounded byte string into an owned value.
    pub fn read_bytes(&mut self) -> CapabilityResult<Vec<u8>> {
        let length = self.read_u32()? as usize;
        Ok(self.take(length)?.to_vec())
    }

    /// Rejects trailing response bytes.
    #[inline]
    pub fn finish(self) -> CapabilityResult<()> {
        if self.offset == self.bytes.len() {
            Ok(())
        } else {
            Err(self.invalid_error)
        }
    }

    fn take(&mut self, length: usize) -> CapabilityResult<&'a [u8]> {
        let end = self.offset.checked_add(length).ok_or(self.invalid_error)?;
        let value = self.bytes.get(self.offset..end).ok_or(self.invalid_error)?;
        self.offset = end;
        Ok(value)
    }

    fn take_array<const LENGTH: usize>(&mut self) -> CapabilityResult<[u8; LENGTH]> {
        self.take(LENGTH)?
            .try_into()
            .map_err(|_| self.invalid_error)
    }
}
