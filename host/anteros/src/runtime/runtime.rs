use std::error::Error;
use std::fmt;

use wasmi::{
    AsContext, AsContextMut, Caller, Config, Engine, Instance, Linker, Memory, Module, Store,
    StoreLimits, StoreLimitsBuilder, TrapCode, TypedFunc,
};
use wasmparser::{ExternalKind, Parser, Payload, TypeRef};

use crate::manifest::ValidatedManifest;
use crate::state::ValidatedStateBundle;
use crate::widget_ir::ValidatedWidgetDocument;
use crate::{
    AbiResult, AbiStatus, AsyncCallbackEventView, CURRENT_ABI_VERSION, CallbackEventView,
    CapabilityBindings, CapabilityCall, CapabilityError, CapabilityRegistry, CapabilityTransport,
    GuestDiagnostic, MAX_GUEST_DIAGNOSTIC_BYTES, ManifestView, ModelLimits, StableId128,
    StateBundleView, WidgetDocumentView,
};

const OUTPUT_ALIGNMENT: i32 = 1;
const WASM_PAGE_BYTES: usize = 64 * 1_024;

/// Configuration for an interpreted Aimer application runtime.
///
/// A configuration defines the execution and resource budgets assigned to a
/// guest generation. Persistent guest instances reset the fuel budget before
/// each call while retaining memory and globals within the configured ceilings.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RuntimeConfig {
    fuel_per_call: u64,
    max_module_bytes: usize,
    max_memory_pages: u32,
    max_table_elements: usize,
    max_call_depth: usize,
}

impl RuntimeConfig {
    /// Creates a fail-closed runtime configuration.
    ///
    /// Every initial budget is zero. Call all builder methods with explicit,
    /// measured ceilings before loading or executing guest code.
    #[inline]
    pub const fn new() -> Self {
        Self {
            fuel_per_call: 0,
            max_module_bytes: 0,
            max_memory_pages: 0,
            max_table_elements: 0,
            max_call_depth: 0,
        }
    }

    /// Sets the maximum fuel available to each guest export invocation.
    ///
    /// A zero budget causes any guest instruction that consumes fuel to trap.
    #[inline]
    pub const fn fuel_per_call(mut self, fuel: u64) -> Self {
        self.fuel_per_call = fuel;
        self
    }

    /// Sets the maximum accepted encoded WebAssembly module size in bytes.
    #[inline]
    pub const fn max_module_bytes(mut self, bytes: usize) -> Self {
        self.max_module_bytes = bytes;
        self
    }

    /// Sets the maximum size of each guest linear memory in 64-KiB pages.
    #[inline]
    pub const fn max_memory_pages(mut self, pages: u32) -> Self {
        self.max_memory_pages = pages;
        self
    }

    /// Sets the maximum number of elements in each guest table.
    #[inline]
    pub const fn max_table_elements(mut self, elements: usize) -> Self {
        self.max_table_elements = elements;
        self
    }

    /// Sets the maximum depth of nested guest function calls.
    #[inline]
    pub const fn max_call_depth(mut self, depth: usize) -> Self {
        self.max_call_depth = depth;
        self
    }
}

/// Identifies the stage at which interpreted execution failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum RuntimeErrorKind {
    /// The module bytes did not contain a valid supported WebAssembly module.
    Module,
    /// The module could not be instantiated or its start function failed.
    Instantiation,
    /// The requested export was absent or had the wrong signature.
    Export,
    /// The guest export trapped or otherwise failed during execution.
    Execution,
    /// The guest exhausted the fuel budget assigned to its invocation.
    FuelExhausted,
    /// The guest's core ABI version is not compatible with this host.
    AbiVersion,
    /// A guest operation returned an invalid or unsuccessful ABI status.
    GuestStatus,
    /// A guest pointer or memory range is invalid.
    GuestMemory,
    /// A guest response exceeds the configured host ceiling.
    OutputLimit,
    /// The guest exceeded a configured module, memory, table, or call limit.
    ResourceLimit,
    /// The guest returned an invalid portable Widget IR document.
    WidgetDocument,
    /// The host callback input is not a valid portable event document.
    EventDocument,
    /// The guest returned an invalid portable state document.
    StateDocument,
    /// The guest returned an invalid application manifest document.
    ManifestDocument,
    /// The guest manifest and permanent host providers are incompatible.
    CapabilityNegotiation,
    /// The guest rejected immutable candidate-generation initialization.
    Initialization,
    /// The reload coordinator retired this guest generation.
    RetiredGeneration,
}

impl fmt::Display for RuntimeErrorKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Module => formatter.write_str("module validation failed"),
            Self::Instantiation => formatter.write_str("module instantiation failed"),
            Self::Export => formatter.write_str("guest export lookup failed"),
            Self::Execution => formatter.write_str("guest execution failed"),
            Self::FuelExhausted => formatter.write_str("guest fuel exhausted"),
            Self::AbiVersion => formatter.write_str("guest ABI version mismatch"),
            Self::GuestStatus => formatter.write_str("guest ABI operation failed"),
            Self::GuestMemory => formatter.write_str("guest memory access failed"),
            Self::OutputLimit => formatter.write_str("guest output limit exceeded"),
            Self::ResourceLimit => formatter.write_str("guest resource limit exceeded"),
            Self::WidgetDocument => formatter.write_str("guest Widget IR validation failed"),
            Self::EventDocument => formatter.write_str("callback event validation failed"),
            Self::StateDocument => formatter.write_str("guest state validation failed"),
            Self::ManifestDocument => formatter.write_str("guest manifest validation failed"),
            Self::CapabilityNegotiation => {
                formatter.write_str("guest capability negotiation failed")
            }
            Self::Initialization => formatter.write_str("guest initialization failed"),
            Self::RetiredGeneration => formatter.write_str("guest generation is retired"),
        }
    }
}

/// An error produced while preparing or invoking an interpreted guest module.
///
/// The public error exposes an Aimer-owned classification while retaining the
/// interpreter error as its source for diagnostics. This prevents callers from
/// depending on `wasmi` error layout or variants.
#[derive(Debug)]
pub struct RuntimeError {
    kind: RuntimeErrorKind,
    source: Box<dyn Error + 'static>,
    diagnostic: Option<GuestDiagnostic>,
}

impl RuntimeError {
    #[inline]
    fn new(source_kind: RuntimeErrorKind, source: impl Error + 'static) -> Self {
        Self {
            kind: source_kind,
            source: Box::new(source),
            diagnostic: None,
        }
    }

    #[inline]
    fn detail(kind: RuntimeErrorKind, detail: impl Into<String>) -> Self {
        Self::new(kind, RuntimeErrorDetail(detail.into()))
    }

    fn guest_status(detail: impl Into<String>, diagnostic: Option<GuestDiagnostic>) -> Self {
        let source = diagnostic
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_else(|| detail.into());
        Self {
            kind: RuntimeErrorKind::GuestStatus,
            source: Box::new(RuntimeErrorDetail(source)),
            diagnostic,
        }
    }

    #[inline]
    fn execution(source: wasmi::Error) -> Self {
        let kind = match source.as_trap_code() {
            Some(TrapCode::OutOfFuel) => RuntimeErrorKind::FuelExhausted,
            Some(TrapCode::GrowthOperationLimited | TrapCode::StackOverflow) => {
                RuntimeErrorKind::ResourceLimit
            }
            _ => RuntimeErrorKind::Execution,
        };
        Self {
            kind,
            source: Box::new(source),
            diagnostic: None,
        }
    }

    /// Returns the stable Aimer classification for this failure.
    #[inline]
    pub const fn kind(&self) -> RuntimeErrorKind {
        self.kind
    }

    /// Returns structured guest failure context when the guest supplied a
    /// valid bounded diagnostic payload.
    #[inline]
    pub fn diagnostic(&self) -> Option<&GuestDiagnostic> {
        self.diagnostic.as_ref()
    }
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.kind, self.source)
    }
}

impl Error for RuntimeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.source.as_ref())
    }
}

#[derive(Debug)]
struct RuntimeErrorDetail(String);

impl fmt::Display for RuntimeErrorDetail {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for RuntimeErrorDetail {}

/// One owned Widget IR image copied and validated from guest linear memory.
#[derive(Debug)]
pub struct WidgetImage {
    bytes: Vec<u8>,
    validated: ValidatedWidgetDocument,
}

/// One owned state image copied and validated from guest linear memory.
///
/// The image owns its canonical `ASTA` bytes after the guest allocation has
/// been released. Its borrowed view reuses validation metadata and never
/// exposes a pointer into WebAssembly memory.
#[derive(Debug)]
pub struct StateImage {
    bytes: Vec<u8>,
    validated: ValidatedStateBundle,
}

/// One owned application manifest copied and validated from guest memory.
///
/// The image owns its canonical `AMNF` bytes after the guest allocation has
/// been released. Its borrowed view reuses validation metadata and never
/// exposes a pointer into WebAssembly memory.
#[derive(Debug)]
pub struct ManifestImage {
    bytes: Vec<u8>,
    validated: ValidatedManifest,
}

impl ManifestImage {
    /// Returns the canonical bytes copied from the guest.
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Returns an allocation-free view over the already validated image.
    #[inline]
    pub fn view(&self) -> ManifestView<'_> {
        ManifestView::from_validated(&self.bytes, self.validated)
    }
}

impl StateImage {
    /// Returns the canonical bytes copied from the guest.
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Returns an allocation-free view over the already validated image.
    #[inline]
    pub fn view(&self) -> StateBundleView<'_> {
        StateBundleView::from_validated(&self.bytes, self.validated)
    }
}

impl WidgetImage {
    /// Returns the canonical bytes copied from the guest.
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Returns an allocation-free view over the already validated image.
    #[inline]
    pub fn view(&self) -> WidgetDocumentView<'_> {
        WidgetDocumentView::from_validated(&self.bytes, self.validated)
    }
}

/// A reusable interpreter for development-only Aimer application code.
///
/// `Runtime` owns a configured `wasmi` engine but does not expose interpreter
/// types through its public interface. It can execute isolated proof calls or
/// instantiate a persistent [`GuestInstance`] with an independent store.
#[derive(Debug)]
pub struct Runtime {
    engine: Engine,
    fuel_per_call: u64,
    max_module_bytes: usize,
    max_memory_bytes: usize,
    max_table_elements: usize,
}

/// One persistent interpreted Aimer application generation.
///
/// A guest instance owns its `wasmi` store, linear memory, and resolved ABI
/// exports. Successive calls therefore observe the same guest state without
/// exposing interpreter-specific types through Aimer's public API. Each guest
/// export still receives an independent fuel budget, and no pointer into guest
/// memory survives a public method call.
#[derive(Debug)]
pub struct GuestInstance {
    store: Store<HostState>,
    exports: CallbackStateExports,
    fuel_per_call: u64,
    last_migration_fuel_consumed: u64,
}

#[derive(Debug)]
struct HostState {
    capabilities: Option<CapabilityBindings>,
    limits: StoreLimits,
}

impl HostState {
    fn new(limits: StoreLimits) -> Self {
        Self {
            capabilities: None,
            limits,
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn capability_call(
    mut caller: Caller<'_, HostState>,
    capability_id_pointer: i32,
    abi_major: i32,
    method_id: i32,
    request_pointer: i32,
    request_length: i32,
    output_pointer: i32,
    output_capacity: i32,
) -> i64 {
    let result = (|| {
        let abi_major = u32::try_from(abi_major).map_err(|_| CapabilityError::InvalidRequest)?;
        let method_id = u32::try_from(method_id).map_err(|_| CapabilityError::InvalidRequest)?;
        let request_length =
            u32::try_from(request_length).map_err(|_| CapabilityError::InvalidRequest)?;
        let output_capacity =
            u32::try_from(output_capacity).map_err(|_| CapabilityError::InvalidRequest)?;
        let memory = caller
            .get_export("memory")
            .and_then(|export| export.into_memory())
            .ok_or(CapabilityError::InvalidRequest)?;

        let mut capability_id = [0_u8; 16];
        memory
            .read(
                caller.as_context(),
                capability_id_pointer as u32 as usize,
                &mut capability_id,
            )
            .map_err(|_| CapabilityError::InvalidRequest)?;
        let capability_id = StableId128::from_bytes(capability_id);
        let bindings = caller
            .data()
            .capabilities
            .as_ref()
            .ok_or(CapabilityError::Unavailable)?;
        let request_limit = bindings.request_limit(capability_id, abi_major)?;
        if request_length > request_limit {
            return Err(CapabilityError::LimitExceeded);
        }
        let mut request = vec![0_u8; request_length as usize];
        memory
            .read(
                caller.as_context(),
                request_pointer as u32 as usize,
                &mut request,
            )
            .map_err(|_| CapabilityError::InvalidRequest)?;

        let response = bindings.invoke(CapabilityCall::new(
            capability_id,
            abi_major,
            method_id,
            &request,
            u32::MAX,
        ))?;
        let required = u32::try_from(response.len()).map_err(|_| CapabilityError::LimitExceeded)?;
        if required > output_capacity {
            return Ok((AbiStatus::BufferTooSmall, required));
        }
        if required != 0 {
            memory
                .write(
                    caller.as_context_mut(),
                    output_pointer as u32 as usize,
                    &response,
                )
                .map_err(|_| CapabilityError::InvalidRequest)?;
        }
        Ok((AbiStatus::Ok, required))
    })();

    match result {
        Ok((status, value)) => pack_abi_result(status, value),
        Err(error) => pack_abi_result(capability_error_status(error), 0),
    }
}

#[inline]
const fn capability_error_status(error: CapabilityError) -> AbiStatus {
    match error {
        CapabilityError::Unsupported => AbiStatus::UnknownId,
        CapabilityError::Denied => AbiStatus::CapabilityDenied,
        CapabilityError::Unavailable => AbiStatus::NotActive,
        CapabilityError::NotActive => AbiStatus::NotActive,
        CapabilityError::InvalidRequest => AbiStatus::MalformedMessage,
        CapabilityError::InvalidResponse => AbiStatus::InternalError,
        CapabilityError::LimitExceeded => AbiStatus::ResourceExhausted,
        CapabilityError::RetiredGeneration => AbiStatus::RetiredGeneration,
    }
}

#[inline]
const fn pack_abi_result(status: AbiStatus, value: u32) -> i64 {
    (((status as u64) << 32) | value as u64) as i64
}

impl Runtime {
    /// Creates an interpreter using the supplied execution configuration.
    #[inline]
    pub fn new(config: RuntimeConfig) -> Self {
        let mut engine_config = Config::default();
        engine_config.consume_fuel(true);
        engine_config.set_max_recursion_depth(config.max_call_depth.max(1));
        engine_config.wasm_memory64(false);
        engine_config.wasm_multi_memory(false);
        engine_config.wasm_tail_call(false);
        engine_config.wasm_extended_const(false);
        engine_config.wasm_custom_page_sizes(false);

        Self {
            engine: Engine::new(&engine_config),
            fuel_per_call: config.fuel_per_call,
            max_module_bytes: config.max_module_bytes,
            max_memory_bytes: (config.max_memory_pages as usize).saturating_mul(WASM_PAGE_BYTES),
            max_table_elements: config.max_table_elements,
        }
    }

    /// Instantiates one persistent callback/state guest generation.
    ///
    /// The module is preflighted without host imports or a start function, its
    /// callback, manifest, and state ABI exports are resolved, and its core ABI
    /// version is checked before the instance is returned. The resulting
    /// [`GuestInstance`] owns the store and memory used by all subsequent calls.
    pub fn instantiate(&self, module_bytes: &[u8]) -> Result<GuestInstance, RuntimeError> {
        self.instantiate_persistent(module_bytes, false)
    }

    /// Instantiates a persistent guest and negotiates its declared capabilities.
    ///
    /// The module may import the single canonical `aimer.capability_call`
    /// function. The guest manifest is copied and validated before provider
    /// bindings become active, and required contract mismatches reject the
    /// generation before any application operation can execute.
    pub fn instantiate_with_capabilities(
        &self,
        module_bytes: &[u8],
        registry: &CapabilityRegistry,
        limits: ModelLimits,
        generation_id: crate::GenerationId,
    ) -> Result<GuestInstance, RuntimeError> {
        let mut guest = self.instantiate_persistent(module_bytes, true)?;
        let manifest = guest.manifest(limits)?;
        let bindings = registry
            .negotiate_generation(&manifest.view(), generation_id)
            .map_err(|error| RuntimeError::new(RuntimeErrorKind::CapabilityNegotiation, error))?;
        guest.store.data_mut().capabilities = Some(bindings);
        guest.initialize_generation(generation_id)?;
        Ok(guest)
    }

    fn instantiate_persistent(
        &self,
        module_bytes: &[u8],
        allow_capability_import: bool,
    ) -> Result<GuestInstance, RuntimeError> {
        self.validate_module_size(module_bytes)?;
        let module = Module::new(&self.engine, module_bytes)
            .map_err(|error| RuntimeError::new(RuntimeErrorKind::Module, error))?;
        validate_persistent_module_shape(module_bytes, allow_capability_import)?;
        let mut store = self.new_store()?;
        let mut linker = Linker::new(&self.engine);
        linker
            .func_wrap("aimer", "capability_call", capability_call)
            .map_err(|error| RuntimeError::new(RuntimeErrorKind::Module, error))?;
        let instance = linker
            .instantiate_and_start(&mut store, &module)
            .map_err(|error| RuntimeError::new(RuntimeErrorKind::Instantiation, error))?;
        let exports = CallbackStateExports::new(&instance, &store)?;

        self.reset_fuel(&mut store)?;
        let version = exports
            .abi_version
            .call(&mut store, ())
            .map_err(RuntimeError::execution)?;
        let version = crate::AbiVersion::from_packed(version);
        if version != CURRENT_ABI_VERSION {
            return Err(RuntimeError::detail(
                RuntimeErrorKind::AbiVersion,
                format!("guest ABI {version} is incompatible with host ABI {CURRENT_ABI_VERSION}"),
            ));
        }

        Ok(GuestInstance {
            store,
            exports,
            fuel_per_call: self.fuel_per_call,
            last_migration_fuel_consumed: 0,
        })
    }

    /// Executes an exported `() -> i32` function from `module_bytes`.
    ///
    /// The module is validated, instantiated without host imports, and invoked
    /// in a fresh store. The call receives exactly the configured fuel budget.
    /// No JIT compiler or executable-memory allocation is used by this API.
    pub fn invoke_i32(&self, module_bytes: &[u8], export_name: &str) -> Result<i32, RuntimeError> {
        self.validate_module_size(module_bytes)?;
        let module = Module::new(&self.engine, module_bytes)
            .map_err(|error| RuntimeError::new(RuntimeErrorKind::Module, error))?;
        let mut store = self.new_store()?;
        let linker = Linker::new(&self.engine);
        let instance = linker
            .instantiate_and_start(&mut store, &module)
            .map_err(|error| RuntimeError::new(RuntimeErrorKind::Instantiation, error))?;
        let function = instance
            .get_typed_func::<(), i32>(&store, export_name)
            .map_err(|error| RuntimeError::new(RuntimeErrorKind::Export, error))?;

        function
            .call(&mut store, ())
            .map_err(RuntimeError::execution)
    }

    /// Executes the guest's initial build operation and validates its Widget IR.
    ///
    /// The runtime verifies ABI version `1.0`, probes the exact output length,
    /// allocates one guest-owned output region, retries once, copies the bytes,
    /// and releases the region. No guest pointer is retained after this method.
    pub fn build(
        &self,
        module_bytes: &[u8],
        limits: ModelLimits,
    ) -> Result<WidgetImage, RuntimeError> {
        self.validate_module_size(module_bytes)?;
        let module = Module::new(&self.engine, module_bytes)
            .map_err(|error| RuntimeError::new(RuntimeErrorKind::Module, error))?;
        let mut store = self.new_store()?;
        let linker = Linker::new(&self.engine);
        let instance = linker
            .instantiate_and_start(&mut store, &module)
            .map_err(|error| RuntimeError::new(RuntimeErrorKind::Instantiation, error))?;
        let exports = GuestExports::new(&instance, &store)?;

        self.reset_fuel(&mut store)?;
        let version = exports
            .abi_version
            .call(&mut store, ())
            .map_err(RuntimeError::execution)?;
        let version = crate::AbiVersion::from_packed(version);
        if version != CURRENT_ABI_VERSION {
            return Err(RuntimeError::detail(
                RuntimeErrorKind::AbiVersion,
                format!("guest ABI {version} is incompatible with host ABI {CURRENT_ABI_VERSION}"),
            ));
        }

        self.reset_fuel(&mut store)?;
        let probe = exports
            .build
            .call(&mut store, (0, 0))
            .map_err(RuntimeError::execution)?;
        let probe = decode_abi_result(probe)?;
        if probe.status() != AbiStatus::BufferTooSmall || probe.value() == 0 {
            return Err(RuntimeError::guest_status(
                format!(
                    "aimer_build probe returned {:?} with length {}",
                    probe.status(),
                    probe.value()
                ),
                self.read_diagnostic(&exports, &mut store),
            ));
        }
        let required = probe.value();
        if required > limits.max_document_bytes {
            return Err(RuntimeError::detail(
                RuntimeErrorKind::OutputLimit,
                format!(
                    "aimer_build requires {required} bytes but the limit is {}",
                    limits.max_document_bytes
                ),
            ));
        }
        let required_i32 = i32::try_from(required).map_err(|_| {
            RuntimeError::detail(
                RuntimeErrorKind::OutputLimit,
                format!("aimer_build length {required} cannot be represented by the guest ABI"),
            )
        })?;

        self.reset_fuel(&mut store)?;
        let allocation = exports
            .alloc
            .call(&mut store, (required_i32, OUTPUT_ALIGNMENT))
            .map_err(RuntimeError::execution)?;
        let allocation = decode_abi_result(allocation)?;
        if allocation.status() != AbiStatus::Ok || allocation.value() == 0 {
            return Err(RuntimeError::detail(
                RuntimeErrorKind::GuestStatus,
                format!(
                    "aimer_alloc returned {:?} with pointer {}",
                    allocation.status(),
                    allocation.value()
                ),
            ));
        }
        let pointer = allocation.value();
        let output =
            validate_memory_range(exports.memory, &store, pointer, required).and_then(|()| {
                self.build_into_allocated_region(
                    &exports,
                    &mut store,
                    pointer,
                    required,
                    required_i32,
                )
            });
        let deallocation = self.deallocate(
            &exports,
            &mut store,
            pointer,
            required_i32,
            OUTPUT_ALIGNMENT,
        );
        let bytes = match (output, deallocation) {
            (Err(error), _) => return Err(error),
            (Ok(_), Err(error)) => return Err(error),
            (Ok(bytes), Ok(())) => bytes,
        };

        let validated = WidgetDocumentView::decode(&bytes, limits)
            .map_err(|error| RuntimeError::new(RuntimeErrorKind::WidgetDocument, error))?
            .into_validated();
        Ok(WidgetImage { bytes, validated })
    }

    /// Dispatches one canonical callback event and exports the resulting state.
    ///
    /// This proof-stage operation creates one isolated guest instance, copies
    /// the complete validated `AEVT` image into guest-owned linear memory,
    /// invokes `aimer_dispatch_event`, and then retrieves one canonical `ASTA`
    /// snapshot through bounded output negotiation. The callback must complete
    /// with empty success; Widget IR output remains the responsibility of the
    /// later retained-generation API. Every guest allocation is released before
    /// this method returns, including error paths.
    pub fn dispatch_event_and_export_state(
        &self,
        module_bytes: &[u8],
        event_bytes: &[u8],
        limits: ModelLimits,
    ) -> Result<StateImage, RuntimeError> {
        let mut guest = self.instantiate(module_bytes)?;
        guest.dispatch_event(event_bytes, limits)?;
        guest.export_state(limits)
    }

    fn build_into_allocated_region<T>(
        &self,
        exports: &GuestExports,
        store: &mut Store<T>,
        pointer: u32,
        required: u32,
        required_i32: i32,
    ) -> Result<Vec<u8>, RuntimeError> {
        let pointer_i32 = pointer as i32;
        self.reset_fuel(store)?;
        let result = exports
            .build
            .call(store.as_context_mut(), (pointer_i32, required_i32))
            .map_err(RuntimeError::execution)?;
        let result = decode_abi_result(result)?;
        if result.status() != AbiStatus::Ok || result.value() != required {
            return Err(RuntimeError::guest_status(
                format!(
                    "aimer_build retry returned {:?} with length {}, expected {required}",
                    result.status(),
                    result.value()
                ),
                self.read_diagnostic(exports, store),
            ));
        }

        let mut bytes = vec![0_u8; required as usize];
        exports
            .memory
            .read(store.as_context(), pointer as usize, &mut bytes)
            .map_err(|error| RuntimeError::new(RuntimeErrorKind::GuestMemory, error))?;
        Ok(bytes)
    }

    fn deallocate<T>(
        &self,
        exports: &GuestExports,
        store: &mut Store<T>,
        pointer: u32,
        length: i32,
        alignment: i32,
    ) -> Result<(), RuntimeError> {
        self.reset_fuel(store)?;
        let status = exports
            .dealloc
            .call(store, (pointer as i32, length, alignment))
            .map_err(RuntimeError::execution)?;
        if status != AbiStatus::Ok as i32 {
            return Err(RuntimeError::detail(
                RuntimeErrorKind::GuestStatus,
                format!("aimer_dealloc returned status {status}"),
            ));
        }
        Ok(())
    }

    #[inline]
    fn reset_fuel<T>(&self, store: &mut Store<T>) -> Result<(), RuntimeError> {
        store
            .set_fuel(self.fuel_per_call)
            .map_err(|error| RuntimeError::new(RuntimeErrorKind::Execution, error))
    }

    fn validate_module_size(&self, module_bytes: &[u8]) -> Result<(), RuntimeError> {
        if module_bytes.len() > self.max_module_bytes {
            return Err(RuntimeError::detail(
                RuntimeErrorKind::ResourceLimit,
                format!(
                    "module contains {} bytes but the configured limit is {}",
                    module_bytes.len(),
                    self.max_module_bytes
                ),
            ));
        }
        Ok(())
    }

    fn new_store(&self) -> Result<Store<HostState>, RuntimeError> {
        let limits = StoreLimitsBuilder::new()
            .memory_size(self.max_memory_bytes)
            .table_elements(self.max_table_elements)
            .instances(1)
            .memories(1)
            .tables(1)
            .trap_on_grow_failure(true)
            .build();
        let mut store = Store::new(&self.engine, HostState::new(limits));
        store.limiter(|state| &mut state.limits);
        self.reset_fuel(&mut store)?;
        Ok(store)
    }

    fn read_diagnostic<T>(
        &self,
        exports: &GuestExports,
        store: &mut Store<T>,
    ) -> Option<GuestDiagnostic> {
        let diagnostic = exports.diagnostic.as_ref()?;
        self.reset_fuel(store).ok()?;
        let probe = diagnostic.call(&mut *store, (0, 0)).ok()?;
        let probe = AbiResult::from_packed(probe).ok()?;
        if probe.status() != AbiStatus::BufferTooSmall || probe.value() == 0 {
            return None;
        }
        let required = probe.value();
        if required as usize > MAX_GUEST_DIAGNOSTIC_BYTES {
            return None;
        }
        let required_i32 = i32::try_from(required).ok()?;
        self.reset_fuel(store).ok()?;
        let allocation = exports
            .alloc
            .call(&mut *store, (required_i32, OUTPUT_ALIGNMENT))
            .ok()?;
        let allocation = AbiResult::from_packed(allocation).ok()?;
        if allocation.status() != AbiStatus::Ok || allocation.value() == 0 {
            return None;
        }
        let pointer = allocation.value();
        let valid = validate_memory_range(exports.memory, &*store, pointer, required).is_ok();
        if !valid {
            let _ = exports.dealloc.call(
                &mut *store,
                (pointer as i32, required_i32, OUTPUT_ALIGNMENT),
            );
            return None;
        }
        self.reset_fuel(store).ok()?;
        let result = diagnostic
            .call(&mut *store, (pointer as i32, required_i32))
            .ok();
        let bytes = result.and_then(|packed| {
            let result = AbiResult::from_packed(packed).ok()?;
            if result.status() != AbiStatus::Ok || result.value() != required {
                return None;
            }
            let mut bytes = vec![0_u8; required as usize];
            exports
                .memory
                .read(&*store, pointer as usize, &mut bytes)
                .ok()?;
            Some(bytes)
        });
        self.reset_fuel(store).ok()?;
        let deallocation = exports
            .dealloc
            .call(
                &mut *store,
                (pointer as i32, required_i32, OUTPUT_ALIGNMENT),
            )
            .ok();
        if deallocation != Some(AbiStatus::Ok as i32) {
            return None;
        }
        bytes.and_then(|bytes| GuestDiagnostic::decode(&bytes, MAX_GUEST_DIAGNOSTIC_BYTES).ok())
    }
}

impl GuestInstance {
    fn initialize_generation(
        &mut self,
        generation_id: crate::GenerationId,
    ) -> Result<(), RuntimeError> {
        let Some(initialize) = self.exports.initialize else {
            return Ok(());
        };
        self.reset_fuel()?;
        let packed = initialize
            .call(&mut self.store, generation_id.get() as i64)
            .map_err(|error| RuntimeError::new(RuntimeErrorKind::Initialization, error))?;
        let result = decode_abi_result(packed)?;
        if result.status() != AbiStatus::Ok || result.value() != 0 {
            return Err(RuntimeError::detail(
                RuntimeErrorKind::Initialization,
                format!(
                    "aimer_initialize returned {:?} with value {}",
                    result.status(),
                    result.value()
                ),
            ));
        }
        Ok(())
    }

    /// Publishes the native window metrics used by responsive portable widgets.
    ///
    /// The export is optional so guests produced before viewport propagation
    /// remain loadable; those guests retain their existing platform defaults.
    /// Candidate generations may receive metrics before activation because this
    /// call carries no capability side effects.
    pub fn set_window_metrics(
        &mut self,
        width: u32,
        height: u32,
        scale_factor: f64,
    ) -> Result<(), RuntimeError> {
        let Some(set_window_metrics) = self.exports.set_window_metrics else {
            return Ok(());
        };
        let width = i32::try_from(width).map_err(|_| {
            RuntimeError::detail(
                RuntimeErrorKind::OutputLimit,
                format!("window width {width} cannot be represented by the guest ABI"),
            )
        })?;
        let height = i32::try_from(height).map_err(|_| {
            RuntimeError::detail(
                RuntimeErrorKind::OutputLimit,
                format!("window height {height} cannot be represented by the guest ABI"),
            )
        })?;
        if !scale_factor.is_finite() || scale_factor <= 0.0 {
            return Err(RuntimeError::detail(
                RuntimeErrorKind::GuestStatus,
                "window scale factor must be finite and positive",
            ));
        }
        self.reset_fuel()?;
        let status = set_window_metrics
            .call(
                &mut self.store,
                (width, height, scale_factor.to_bits() as i64),
            )
            .map_err(RuntimeError::execution)?;
        if status != AbiStatus::Ok as i32 {
            let status = AbiStatus::try_from(status as u32).map_err(|error| {
                RuntimeError::detail(
                    RuntimeErrorKind::GuestStatus,
                    format!("aimer_set_window_metrics returned an unknown status: {error}"),
                )
            })?;
            return Err(RuntimeError::guest_status(
                format!("aimer_set_window_metrics returned {status:?}"),
                self.read_diagnostic(),
            ));
        }
        Ok(())
    }
    /// Returns the reload-coordinator identity when this guest has capabilities.
    #[inline]
    pub fn generation_id(&self) -> Option<crate::GenerationId> {
        self.store
            .data()
            .capabilities
            .as_ref()
            .map(CapabilityBindings::generation_id)
    }

    /// Permanently prevents this generation from initiating capability work.
    #[inline]
    pub fn retire(&self) {
        if let Some(capabilities) = &self.store.data().capabilities {
            capabilities.retire();
        }
    }

    /// Activates committed-only capability calls and publishes staged effects.
    ///
    /// The reload coordinator calls this only at the event-loop commit point,
    /// after all candidate validation and materialization has succeeded.
    #[inline]
    pub fn activate(&self) {
        if let Some(capabilities) = &self.store.data().capabilities {
            capabilities.activate();
        }
    }

    /// Builds and validates one complete Widget IR snapshot from this generation.
    ///
    /// The output is negotiated, copied into host-owned memory, and released
    /// before the validated image is returned. Capability imports invoked by
    /// `aimer_build` can access only this generation's negotiated bindings.
    pub fn build(&mut self, limits: ModelLimits) -> Result<WidgetImage, RuntimeError> {
        self.ensure_active_generation()?;
        self.reset_fuel()?;
        let probe = self
            .exports
            .build
            .call(&mut self.store, (0, 0))
            .map_err(RuntimeError::execution)?;
        let probe = decode_abi_result(probe)?;
        if probe.status() != AbiStatus::BufferTooSmall || probe.value() == 0 {
            return Err(RuntimeError::guest_status(
                format!(
                    "aimer_build probe returned {:?} with length {}",
                    probe.status(),
                    probe.value()
                ),
                self.read_diagnostic(),
            ));
        }
        let required = probe.value();
        if required > limits.max_document_bytes {
            return Err(RuntimeError::detail(
                RuntimeErrorKind::OutputLimit,
                format!(
                    "aimer_build requires {required} bytes but the limit is {}",
                    limits.max_document_bytes
                ),
            ));
        }
        let required_i32 = i32::try_from(required).map_err(|_| {
            RuntimeError::detail(
                RuntimeErrorKind::OutputLimit,
                format!("aimer_build length {required} cannot be represented by the guest ABI"),
            )
        })?;
        let pointer = self.allocate(required_i32, "Widget IR output")?;
        self.reset_fuel()?;
        let result = self
            .exports
            .build
            .call(&mut self.store, (pointer as i32, required_i32))
            .map_err(RuntimeError::execution)
            .and_then(decode_abi_result)
            .and_then(|result| {
                if result.status() != AbiStatus::Ok || result.value() != required {
                    return Err(RuntimeError::guest_status(
                        format!(
                            "aimer_build retry returned {:?} with length {}, expected {required}",
                            result.status(),
                            result.value()
                        ),
                        self.read_diagnostic(),
                    ));
                }
                let mut bytes = vec![0_u8; required as usize];
                self.exports
                    .memory
                    .read(&self.store, pointer as usize, &mut bytes)
                    .map_err(|error| RuntimeError::new(RuntimeErrorKind::GuestMemory, error))?;
                Ok(bytes)
            });
        let deallocation = self.deallocate(pointer, required_i32);
        let bytes = match (result, deallocation) {
            (Err(error), _) => return Err(error),
            (Ok(_), Err(error)) => return Err(error),
            (Ok(bytes), Ok(())) => bytes,
        };
        let validated = WidgetDocumentView::decode(&bytes, limits)
            .map_err(|error| RuntimeError::new(RuntimeErrorKind::WidgetDocument, error))?
            .into_validated();
        Ok(WidgetImage { bytes, validated })
    }

    fn ensure_active_generation(&self) -> Result<(), RuntimeError> {
        if self
            .store
            .data()
            .capabilities
            .as_ref()
            .is_some_and(|capabilities| !capabilities.is_active())
        {
            return Err(RuntimeError::detail(
                RuntimeErrorKind::RetiredGeneration,
                "the reload coordinator retired this guest generation",
            ));
        }
        Ok(())
    }
    /// Queries and validates this generation's canonical application manifest.
    ///
    /// Output capacity is negotiated with the guest, bounded by `limits`, and
    /// copied into host-owned memory before the guest allocation is released.
    /// Manifest validation happens before cleanup so its error remains primary
    /// if both the document and guest deallocation are invalid.
    pub fn manifest(&mut self, limits: ModelLimits) -> Result<ManifestImage, RuntimeError> {
        self.reset_fuel()?;
        let probe = self
            .exports
            .manifest
            .call(&mut self.store, (0, 0))
            .map_err(RuntimeError::execution)?;
        let probe = decode_abi_result(probe)?;
        if probe.status() != AbiStatus::BufferTooSmall || probe.value() == 0 {
            return Err(RuntimeError::guest_status(
                format!(
                    "aimer_manifest probe returned {:?} with length {}",
                    probe.status(),
                    probe.value()
                ),
                self.read_diagnostic(),
            ));
        }
        let required = probe.value();
        if required > limits.max_document_bytes {
            return Err(RuntimeError::detail(
                RuntimeErrorKind::OutputLimit,
                format!(
                    "aimer_manifest requires {required} bytes but the limit is {}",
                    limits.max_document_bytes
                ),
            ));
        }
        let required_i32 = i32::try_from(required).map_err(|_| {
            RuntimeError::detail(
                RuntimeErrorKind::OutputLimit,
                format!("aimer_manifest length {required} cannot be represented by the guest ABI"),
            )
        })?;
        let pointer = self.allocate(required_i32, "manifest output")?;
        let output = self
            .read_manifest_into(pointer, required, required_i32)
            .and_then(|bytes| {
                let validated = ManifestView::decode(&bytes, limits)
                    .map_err(|error| RuntimeError::new(RuntimeErrorKind::ManifestDocument, error))?
                    .into_validated();
                Ok(ManifestImage { bytes, validated })
            });
        let deallocation = self.deallocate(pointer, required_i32);
        match (output, deallocation) {
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(error),
            (Ok(image), Ok(())) => Ok(image),
        }
    }
    /// Imports one canonical state image into this guest generation.
    ///
    /// The state is validated before entering the guest, copied into a bounded
    /// guest-owned allocation, and released before this method returns. The
    /// guest must consume the state synchronously and return an empty successful
    /// result; no pointer into WebAssembly memory survives the call.
    pub fn import_state(
        &mut self,
        state_bytes: &[u8],
        limits: ModelLimits,
    ) -> Result<(), RuntimeError> {
        StateBundleView::decode(state_bytes, limits)
            .map_err(|error| RuntimeError::new(RuntimeErrorKind::StateDocument, error))?;
        let state_len = i32::try_from(state_bytes.len()).map_err(|_| {
            RuntimeError::detail(
                RuntimeErrorKind::OutputLimit,
                format!(
                    "state image length {} cannot be represented by the guest ABI",
                    state_bytes.len()
                ),
            )
        })?;
        let state_pointer = self.allocate(state_len, "state input")?;
        let import = self
            .exports
            .memory
            .write(&mut self.store, state_pointer as usize, state_bytes)
            .map_err(|error| RuntimeError::new(RuntimeErrorKind::GuestMemory, error))
            .and_then(|()| self.invoke_state_import(state_pointer, state_len));
        let deallocation = self.deallocate(state_pointer, state_len);
        match (import, deallocation) {
            (Err(error), _) => Err(error),
            (Ok(()), Err(error)) => Err(error),
            (Ok(()), Ok(())) => Ok(()),
        }
    }

    /// Dispatches one canonical callback event into this guest generation.
    ///
    /// The event is validated before entering the guest and copied into a
    /// bounded guest-owned allocation. The guest receives one host-sized output
    /// buffer so the callback executes exactly once. Empty success returns
    /// `None`; non-empty output is copied, validated as a complete canonical
    /// Widget IR snapshot, and returned in host-owned memory.
    pub fn dispatch_event(
        &mut self,
        event_bytes: &[u8],
        limits: ModelLimits,
    ) -> Result<Option<WidgetImage>, RuntimeError> {
        CallbackEventView::decode(event_bytes, limits)
            .map_err(|error| RuntimeError::new(RuntimeErrorKind::EventDocument, error))?;
        let event_len = i32::try_from(event_bytes.len()).map_err(|_| {
            RuntimeError::detail(
                RuntimeErrorKind::OutputLimit,
                format!(
                    "callback event length {} cannot be represented by the guest ABI",
                    event_bytes.len()
                ),
            )
        })?;
        let event_pointer = self.allocate(event_len, "callback event")?;
        let output_capacity = i32::try_from(limits.max_document_bytes).map_err(|_| {
            RuntimeError::detail(
                RuntimeErrorKind::OutputLimit,
                format!(
                    "callback output limit {} cannot be represented by the guest ABI",
                    limits.max_document_bytes
                ),
            )
        })?;
        let output_pointer = if output_capacity == 0 {
            None
        } else {
            match self.allocate(output_capacity, "callback output") {
                Ok(pointer) => Some(pointer),
                Err(error) => {
                    let _ = self.deallocate(event_pointer, event_len);
                    return Err(error);
                }
            }
        };
        let dispatch = self
            .exports
            .memory
            .write(&mut self.store, event_pointer as usize, event_bytes)
            .map_err(|error| RuntimeError::new(RuntimeErrorKind::GuestMemory, error))
            .and_then(|()| {
                self.invoke_dispatch(
                    event_pointer,
                    event_len,
                    output_pointer.unwrap_or(0),
                    output_capacity,
                    limits,
                )
            });
        let event_deallocation = self.deallocate(event_pointer, event_len);
        let output_deallocation = output_pointer
            .map(|pointer| self.deallocate(pointer, output_capacity))
            .unwrap_or(Ok(()));
        match (dispatch, event_deallocation, output_deallocation) {
            (Err(error), _, _) => Err(error),
            (Ok(_), Err(error), _) | (Ok(_), Ok(()), Err(error)) => Err(error),
            (Ok(image), Ok(()), Ok(())) => Ok(image),
        }
    }

    /// Polls guest-owned async work at a host safe point.
    ///
    /// Older guests without the optional export are treated as idle. A newer
    /// guest may return one complete Widget IR image; that image is validated
    /// before it can reach native materialization.
    pub fn poll_async(&mut self, limits: ModelLimits) -> Result<Option<WidgetImage>, RuntimeError> {
        self.ensure_active_generation()?;
        let Some(poll_async) = self.exports.poll_async else {
            return Ok(None);
        };
        self.reset_fuel()?;
        let probe = poll_async
            .call(&mut self.store, (0, 0))
            .map_err(RuntimeError::execution)?;
        let probe = decode_abi_result(probe)?;
        if probe.status() == AbiStatus::Ok && probe.value() == 0 {
            return Ok(None);
        }
        if probe.status() != AbiStatus::BufferTooSmall || probe.value() == 0 {
            return Err(RuntimeError::guest_status(
                format!(
                    "aimer_poll_async probe returned {:?} with length {}",
                    probe.status(),
                    probe.value()
                ),
                self.read_diagnostic(),
            ));
        }
        let required = probe.value();
        if required > limits.max_document_bytes {
            return Err(RuntimeError::detail(
                RuntimeErrorKind::OutputLimit,
                format!(
                    "aimer_poll_async requires {required} bytes but the limit is {}",
                    limits.max_document_bytes
                ),
            ));
        }
        let required_i32 = i32::try_from(required).map_err(|_| {
            RuntimeError::detail(
                RuntimeErrorKind::OutputLimit,
                format!(
                    "aimer_poll_async length {required} cannot be represented by the guest ABI"
                ),
            )
        })?;
        let pointer = self.allocate(required_i32, "async poll output")?;
        let output = self.reset_fuel().and_then(|()| {
            let result = poll_async
                .call(&mut self.store, (pointer as i32, required_i32))
                .map_err(RuntimeError::execution)?;
            let result = decode_abi_result(result)?;
            if result.status() != AbiStatus::Ok || result.value() != required {
                return Err(RuntimeError::guest_status(
                    format!(
                        "aimer_poll_async retry returned {:?} with length {}, expected {required}",
                        result.status(),
                        result.value()
                    ),
                    self.read_diagnostic(),
                ));
            }
            let mut bytes = vec![0_u8; required as usize];
            self.exports
                .memory
                .read(&self.store, pointer as usize, &mut bytes)
                .map_err(|error| RuntimeError::new(RuntimeErrorKind::GuestMemory, error))?;
            let validated = WidgetDocumentView::decode(&bytes, limits)
                .map_err(|error| RuntimeError::new(RuntimeErrorKind::WidgetDocument, error))?
                .into_validated();
            Ok(Some(WidgetImage { bytes, validated }))
        });
        let deallocation = self.deallocate(pointer, required_i32);
        match (output, deallocation) {
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(error),
            (Ok(image), Ok(())) => Ok(image),
        }
    }

    /// Returns the guest's bounded hint that async work should keep the app
    /// awake. Guests without the optional export are treated as idle.
    pub fn has_async_work(&mut self) -> Result<bool, RuntimeError> {
        self.ensure_active_generation()?;
        let Some(async_ready) = self.exports.async_ready else {
            return Ok(false);
        };
        self.reset_fuel()?;
        let result = async_ready
            .call(&mut self.store, ())
            .map_err(RuntimeError::execution)?;
        let result = decode_abi_result(result)?;
        if result.status() != AbiStatus::Ok || result.value() > 1 {
            return Err(RuntimeError::guest_status(
                format!(
                    "aimer_async_ready returned {:?} with value {}",
                    result.status(),
                    result.value()
                ),
                self.read_diagnostic(),
            ));
        }
        Ok(result.value() == 1)
    }

    /// Delivers one bounded host-owned async completion to this guest.
    ///
    /// The event is decoded before any guest call and the optional export keeps
    /// older guests compatible by returning an explicit unsupported-export
    /// error rather than accepting an unrecognized message.
    pub fn dispatch_async_event(
        &mut self,
        event_bytes: &[u8],
        limits: ModelLimits,
    ) -> Result<Option<WidgetImage>, RuntimeError> {
        AsyncCallbackEventView::decode(event_bytes, limits)
            .map_err(|error| RuntimeError::new(RuntimeErrorKind::EventDocument, error))?;
        let dispatch = self.exports.dispatch_async_event.ok_or_else(|| {
            RuntimeError::detail(
                RuntimeErrorKind::Export,
                "guest does not export `aimer_dispatch_async_event`",
            )
        })?;
        let event_len = i32::try_from(event_bytes.len()).map_err(|_| {
            RuntimeError::detail(
                RuntimeErrorKind::OutputLimit,
                format!(
                    "async callback event length {} cannot be represented by the guest ABI",
                    event_bytes.len()
                ),
            )
        })?;
        let event_pointer = self.allocate(event_len, "async callback event")?;
        let output_capacity = i32::try_from(limits.max_document_bytes).map_err(|_| {
            RuntimeError::detail(
                RuntimeErrorKind::OutputLimit,
                format!(
                    "async callback output limit {} cannot be represented by the guest ABI",
                    limits.max_document_bytes
                ),
            )
        })?;
        let output_pointer = if output_capacity == 0 {
            None
        } else {
            match self.allocate(output_capacity, "async callback output") {
                Ok(pointer) => Some(pointer),
                Err(error) => {
                    let _ = self.deallocate(event_pointer, event_len);
                    return Err(error);
                }
            }
        };
        let dispatch_result = self
            .exports
            .memory
            .write(&mut self.store, event_pointer as usize, event_bytes)
            .map_err(|error| RuntimeError::new(RuntimeErrorKind::GuestMemory, error))
            .and_then(|()| {
                self.reset_fuel()?;
                let result = dispatch
                    .call(
                        &mut self.store,
                        (
                            event_pointer as i32,
                            event_len,
                            output_pointer.unwrap_or(0) as i32,
                            output_capacity,
                        ),
                    )
                    .map_err(RuntimeError::execution)?;
                let result = decode_abi_result(result)?;
                if result.status() == AbiStatus::Ok && result.value() == 0 {
                    return Ok(None);
                }
                if result.status() == AbiStatus::BufferTooSmall {
                    return Err(RuntimeError::detail(
                        RuntimeErrorKind::OutputLimit,
                        format!(
                            "aimer_dispatch_async_event requires {} bytes but the limit is {}",
                            result.value(),
                            limits.max_document_bytes
                        ),
                    ));
                }
                if result.status() != AbiStatus::Ok || result.value() > output_capacity as u32 {
                    return Err(RuntimeError::guest_status(
                        format!(
                            "aimer_dispatch_async_event returned {:?} with length {} for capacity {output_capacity}",
                            result.status(),
                            result.value()
                        ),
                        self.read_diagnostic(),
                    ));
                }
                let mut bytes = vec![0_u8; result.value() as usize];
                self.exports
                    .memory
                    .read(
                        &self.store,
                        output_pointer.unwrap_or(0) as usize,
                        &mut bytes,
                    )
                    .map_err(|error| RuntimeError::new(RuntimeErrorKind::GuestMemory, error))?;
                let validated = WidgetDocumentView::decode(&bytes, limits)
                    .map_err(|error| RuntimeError::new(RuntimeErrorKind::WidgetDocument, error))?
                    .into_validated();
                Ok(Some(WidgetImage { bytes, validated }))
            });
        let event_deallocation = self.deallocate(event_pointer, event_len);
        let output_deallocation = output_pointer
            .map(|pointer| self.deallocate(pointer, output_capacity))
            .unwrap_or(Ok(()));
        match (dispatch_result, event_deallocation, output_deallocation) {
            (Err(error), _, _) => Err(error),
            (Ok(_), Err(error), _) | (Ok(_), Ok(()), Err(error)) => Err(error),
            (Ok(image), Ok(()), Ok(())) => Ok(image),
        }
    }

    /// Exports and validates this generation's current canonical state image.
    ///
    /// Output capacity is negotiated with the guest, bounded by `limits`, and
    /// copied into host-owned memory before the guest allocation is released.
    /// The returned image contains no pointer into WebAssembly memory.
    pub fn export_state(&mut self, limits: ModelLimits) -> Result<StateImage, RuntimeError> {
        let bytes = self.read_state_output(limits)?;
        let validated = StateBundleView::decode(&bytes, limits)
            .map_err(|error| RuntimeError::new(RuntimeErrorKind::StateDocument, error))?
            .into_validated();
        Ok(StateImage { bytes, validated })
    }

    /// Returns whether this candidate declares the optional state migration export.
    #[inline]
    pub fn supports_state_migration(&self) -> bool {
        self.exports.migrate_state.is_some()
    }

    /// Returns fuel consumed by the most recent successful migration negotiation.
    #[inline]
    pub const fn last_migration_fuel_consumed(&self) -> u64 {
        self.last_migration_fuel_consumed
    }

    /// Executes this candidate's migration code over one canonical old snapshot.
    ///
    /// The probe input is released before retry. Retry uses one bounded region
    /// split into disjoint input and output ranges, so a guest allocator cannot
    /// alias live allocations. The migration export receives fresh fuel for
    /// each invocation, and its result is copied and validated before import.
    pub fn migrate_state(
        &mut self,
        previous_state: &[u8],
        limits: ModelLimits,
    ) -> Result<StateImage, RuntimeError> {
        self.last_migration_fuel_consumed = 0;
        StateBundleView::decode(previous_state, limits)
            .map_err(|error| RuntimeError::new(RuntimeErrorKind::StateDocument, error))?;
        let state_len = i32::try_from(previous_state.len()).map_err(|_| {
            RuntimeError::detail(
                RuntimeErrorKind::OutputLimit,
                format!(
                    "state migration input length {} cannot be represented by the guest ABI",
                    previous_state.len()
                ),
            )
        })?;
        let probe_pointer = self.allocate(state_len, "state migration probe input")?;
        let probe = self
            .exports
            .memory
            .write(&mut self.store, probe_pointer as usize, previous_state)
            .map_err(|error| RuntimeError::new(RuntimeErrorKind::GuestMemory, error))
            .and_then(|()| self.probe_migration(probe_pointer, state_len, limits));
        let probe_deallocation = self.deallocate(probe_pointer, state_len);
        let (required, probe_fuel) = match (probe, probe_deallocation) {
            (Err(error), _) => return Err(error),
            (Ok(_), Err(error)) => return Err(error),
            (Ok(result), Ok(())) => result,
        };
        self.retry_migration(previous_state, state_len, required, probe_fuel, limits)
    }

    fn probe_migration(
        &mut self,
        state_pointer: u32,
        state_len: i32,
        limits: ModelLimits,
    ) -> Result<(u32, u64), RuntimeError> {
        let migrate_state = self.exports.migrate_state.ok_or_else(|| {
            RuntimeError::detail(
                RuntimeErrorKind::Export,
                "candidate does not export `aimer_migrate_state`",
            )
        })?;
        self.reset_fuel()?;
        let probe = migrate_state
            .call(&mut self.store, (state_pointer as i32, state_len, 0, 0))
            .map_err(RuntimeError::execution)?;
        let probe_fuel = self.consumed_fuel()?;
        let probe = decode_abi_result(probe)?;
        if probe.status() != AbiStatus::BufferTooSmall || probe.value() == 0 {
            return Err(RuntimeError::guest_status(
                format!(
                    "aimer_migrate_state probe returned {:?} with length {}",
                    probe.status(),
                    probe.value()
                ),
                self.read_diagnostic(),
            ));
        }
        let required = probe.value();
        if required > limits.max_document_bytes {
            return Err(RuntimeError::detail(
                RuntimeErrorKind::OutputLimit,
                format!(
                    "aimer_migrate_state requires {required} bytes but the limit is {}",
                    limits.max_document_bytes
                ),
            ));
        }
        Ok((required, probe_fuel))
    }

    fn retry_migration(
        &mut self,
        previous_state: &[u8],
        state_len: i32,
        required: u32,
        probe_fuel: u64,
        limits: ModelLimits,
    ) -> Result<StateImage, RuntimeError> {
        let migrate_state = self.exports.migrate_state.ok_or_else(|| {
            RuntimeError::detail(
                RuntimeErrorKind::Export,
                "candidate does not export `aimer_migrate_state`",
            )
        })?;
        let required_i32 = i32::try_from(required).map_err(|_| {
            RuntimeError::detail(
                RuntimeErrorKind::OutputLimit,
                format!(
                    "aimer_migrate_state length {required} cannot be represented by the guest ABI"
                ),
            )
        })?;
        let allocation_len = state_len.checked_add(required_i32).ok_or_else(|| {
            RuntimeError::detail(
                RuntimeErrorKind::OutputLimit,
                "combined state migration allocation length overflowed",
            )
        })?;
        let allocation_pointer =
            self.allocate(allocation_len, "state migration input and output")?;
        let output_pointer = allocation_pointer
            .checked_add(state_len as u32)
            .ok_or_else(|| {
                RuntimeError::detail(
                    RuntimeErrorKind::GuestMemory,
                    "state migration output pointer overflowed",
                )
            });
        let output = output_pointer
            .and_then(|output_pointer| {
                self.exports
                    .memory
                    .write(&mut self.store, allocation_pointer as usize, previous_state)
                    .map_err(|error| RuntimeError::new(RuntimeErrorKind::GuestMemory, error))?;
                Ok(output_pointer)
            })
            .and_then(|output_pointer| {
                self.reset_fuel()?;
                Ok(output_pointer)
            })
            .and_then(|output_pointer| {
                let result = migrate_state
                    .call(
                        &mut self.store,
                        (
                            allocation_pointer as i32,
                            state_len,
                            output_pointer as i32,
                            required_i32,
                        ),
                    )
                    .map_err(RuntimeError::execution)?;
                Ok((output_pointer, result))
            })
            .and_then(|(output_pointer, result)| {
                let retry_fuel = self.consumed_fuel()?;
                self.last_migration_fuel_consumed = probe_fuel
                    .checked_add(retry_fuel)
                    .ok_or_else(|| {
                        RuntimeError::detail(
                            RuntimeErrorKind::ResourceLimit,
                            "state migration fuel accounting overflowed",
                        )
                    })?;
                Ok((output_pointer, decode_abi_result(result)?))
            })
            .and_then(|(output_pointer, result)| {
                if result.status() != AbiStatus::Ok || result.value() != required {
                    return Err(RuntimeError::guest_status(
                        format!(
                            "aimer_migrate_state retry returned {:?} with length {}, expected {required}",
                            result.status(),
                            result.value()
                        ),
                        self.read_diagnostic(),
                    ));
                }
                let mut bytes = vec![0_u8; required as usize];
                self.exports
                    .memory
                    .read(&self.store, output_pointer as usize, &mut bytes)
                    .map_err(|error| RuntimeError::new(RuntimeErrorKind::GuestMemory, error))?;
                let validated = StateBundleView::decode(&bytes, limits)
                    .map_err(|error| RuntimeError::new(RuntimeErrorKind::StateDocument, error))?
                    .into_validated();
                Ok(StateImage { bytes, validated })
            });
        let deallocation = self.deallocate(allocation_pointer, allocation_len);
        match (output, deallocation) {
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(error),
            (Ok(image), Ok(())) => Ok(image),
        }
    }

    fn invoke_state_import(
        &mut self,
        state_pointer: u32,
        state_len: i32,
    ) -> Result<(), RuntimeError> {
        self.reset_fuel()?;
        let result = self
            .exports
            .import_state
            .call(&mut self.store, (state_pointer as i32, state_len))
            .map_err(RuntimeError::execution)?;
        let result = decode_abi_result(result)?;
        if result.status() != AbiStatus::Ok || result.value() != 0 {
            return Err(RuntimeError::guest_status(
                format!(
                    "aimer_import_state returned {:?} with length {}",
                    result.status(),
                    result.value()
                ),
                self.read_diagnostic(),
            ));
        }
        Ok(())
    }

    fn read_manifest_into(
        &mut self,
        pointer: u32,
        required: u32,
        required_i32: i32,
    ) -> Result<Vec<u8>, RuntimeError> {
        self.reset_fuel()?;
        let packed = self
            .exports
            .manifest
            .call(&mut self.store, (pointer as i32, required_i32))
            .map_err(RuntimeError::execution)?;
        let result = decode_abi_result(packed)?;
        if result.status() != AbiStatus::Ok || result.value() != required {
            return Err(RuntimeError::guest_status(
                format!(
                    "aimer_manifest retry returned {:?} with length {}, expected {required}",
                    result.status(),
                    result.value()
                ),
                self.read_diagnostic(),
            ));
        }
        let mut bytes = vec![0_u8; required as usize];
        self.exports
            .memory
            .read(&self.store, pointer as usize, &mut bytes)
            .map_err(|error| RuntimeError::new(RuntimeErrorKind::GuestMemory, error))?;
        Ok(bytes)
    }

    fn invoke_dispatch(
        &mut self,
        event_pointer: u32,
        event_len: i32,
        output_pointer: u32,
        output_capacity: i32,
        limits: ModelLimits,
    ) -> Result<Option<WidgetImage>, RuntimeError> {
        self.reset_fuel()?;
        let result = self
            .exports
            .dispatch_event
            .call(
                &mut self.store,
                (
                    event_pointer as i32,
                    event_len,
                    output_pointer as i32,
                    output_capacity,
                ),
            )
            .map_err(RuntimeError::execution)?;
        let result = decode_abi_result(result)?;
        if result.status() == AbiStatus::Ok && result.value() == 0 {
            return Ok(None);
        }
        if result.status() == AbiStatus::BufferTooSmall {
            return Err(RuntimeError::detail(
                RuntimeErrorKind::OutputLimit,
                format!(
                    "aimer_dispatch_event requires {} bytes but the limit is {}",
                    result.value(),
                    limits.max_document_bytes
                ),
            ));
        }
        if result.status() != AbiStatus::Ok || result.value() > output_capacity as u32 {
            return Err(RuntimeError::guest_status(
                format!(
                    "aimer_dispatch_event returned {:?} with length {} for capacity {output_capacity}",
                    result.status(),
                    result.value()
                ),
                self.read_diagnostic(),
            ));
        }
        let mut bytes = vec![0_u8; result.value() as usize];
        self.exports
            .memory
            .read(&self.store, output_pointer as usize, &mut bytes)
            .map_err(|error| RuntimeError::new(RuntimeErrorKind::GuestMemory, error))?;
        let validated = WidgetDocumentView::decode(&bytes, limits)
            .map_err(|error| RuntimeError::new(RuntimeErrorKind::WidgetDocument, error))?
            .into_validated();
        Ok(Some(WidgetImage { bytes, validated }))
    }

    fn read_state_output(&mut self, limits: ModelLimits) -> Result<Vec<u8>, RuntimeError> {
        self.reset_fuel()?;
        let probe = self
            .exports
            .export_state
            .call(&mut self.store, (0, 0))
            .map_err(RuntimeError::execution)?;
        let probe = decode_abi_result(probe)?;
        if probe.status() != AbiStatus::BufferTooSmall || probe.value() == 0 {
            return Err(RuntimeError::guest_status(
                format!(
                    "aimer_export_state probe returned {:?} with length {}",
                    probe.status(),
                    probe.value()
                ),
                self.read_diagnostic(),
            ));
        }
        let required = probe.value();
        if required > limits.max_document_bytes {
            return Err(RuntimeError::detail(
                RuntimeErrorKind::OutputLimit,
                format!(
                    "aimer_export_state requires {required} bytes but the limit is {}",
                    limits.max_document_bytes
                ),
            ));
        }
        let required_i32 = i32::try_from(required).map_err(|_| {
            RuntimeError::detail(
                RuntimeErrorKind::OutputLimit,
                format!(
                    "aimer_export_state length {required} cannot be represented by the guest ABI"
                ),
            )
        })?;
        let pointer = self.allocate(required_i32, "state output")?;
        let output = self.read_state_into(pointer, required, required_i32);
        let deallocation = self.deallocate(pointer, required_i32);
        match (output, deallocation) {
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(error),
            (Ok(bytes), Ok(())) => Ok(bytes),
        }
    }

    fn read_state_into(
        &mut self,
        pointer: u32,
        required: u32,
        required_i32: i32,
    ) -> Result<Vec<u8>, RuntimeError> {
        self.reset_fuel()?;
        let packed = self
            .exports
            .export_state
            .call(&mut self.store, (pointer as i32, required_i32))
            .map_err(RuntimeError::execution)?;
        let result = decode_abi_result(packed)?;
        if result.status() != AbiStatus::Ok || result.value() != required {
            return Err(RuntimeError::guest_status(
                format!(
                    "aimer_export_state retry returned {:?} with length {}, expected {required}",
                    result.status(),
                    result.value()
                ),
                self.read_diagnostic(),
            ));
        }
        let mut bytes = vec![0_u8; required as usize];
        self.exports
            .memory
            .read(&self.store, pointer as usize, &mut bytes)
            .map_err(|error| RuntimeError::new(RuntimeErrorKind::GuestMemory, error))?;
        Ok(bytes)
    }

    fn allocate(&mut self, length: i32, operation: &str) -> Result<u32, RuntimeError> {
        self.reset_fuel()?;
        let allocation = self
            .exports
            .alloc
            .call(&mut self.store, (length, OUTPUT_ALIGNMENT))
            .map_err(RuntimeError::execution)?;
        let allocation = decode_abi_result(allocation)?;
        if allocation.status() != AbiStatus::Ok || allocation.value() == 0 {
            return Err(RuntimeError::detail(
                RuntimeErrorKind::GuestStatus,
                format!(
                    "aimer_alloc for {operation} returned {:?} with pointer {}",
                    allocation.status(),
                    allocation.value()
                ),
            ));
        }
        let pointer = allocation.value();
        if let Err(error) =
            validate_memory_range(self.exports.memory, &self.store, pointer, length as u32)
        {
            let _ = self.deallocate(pointer, length);
            return Err(error);
        }
        Ok(pointer)
    }

    fn deallocate(&mut self, pointer: u32, length: i32) -> Result<(), RuntimeError> {
        self.reset_fuel()?;
        let status = self
            .exports
            .dealloc
            .call(&mut self.store, (pointer as i32, length, OUTPUT_ALIGNMENT))
            .map_err(RuntimeError::execution)?;
        if status != AbiStatus::Ok as i32 {
            return Err(RuntimeError::detail(
                RuntimeErrorKind::GuestStatus,
                format!("aimer_dealloc returned status {status}"),
            ));
        }
        Ok(())
    }

    fn read_diagnostic(&mut self) -> Option<GuestDiagnostic> {
        if self.exports.diagnostic.is_none() {
            return None;
        }
        self.reset_fuel().ok()?;
        let probe = self
            .exports
            .diagnostic
            .as_ref()?
            .call(&mut self.store, (0, 0))
            .ok()?;
        let probe = AbiResult::from_packed(probe).ok()?;
        if probe.status() != AbiStatus::BufferTooSmall || probe.value() == 0 {
            return None;
        }
        let required = probe.value();
        if required as usize > MAX_GUEST_DIAGNOSTIC_BYTES {
            return None;
        }
        let required_i32 = i32::try_from(required).ok()?;
        let pointer = self
            .allocate(required_i32, "guest diagnostic output")
            .ok()?;
        self.reset_fuel().ok()?;
        let result = self
            .exports
            .diagnostic
            .as_ref()?
            .call(&mut self.store, (pointer as i32, required_i32))
            .ok();
        let bytes = result.and_then(|packed| {
            let result = AbiResult::from_packed(packed).ok()?;
            if result.status() != AbiStatus::Ok || result.value() != required {
                return None;
            }
            let mut bytes = vec![0_u8; required as usize];
            self.exports
                .memory
                .read(&self.store, pointer as usize, &mut bytes)
                .ok()?;
            Some(bytes)
        });
        let deallocation = self.deallocate(pointer, required_i32);
        if deallocation.is_err() {
            return None;
        }
        bytes.and_then(|bytes| GuestDiagnostic::decode(&bytes, MAX_GUEST_DIAGNOSTIC_BYTES).ok())
    }

    #[inline]
    fn reset_fuel(&mut self) -> Result<(), RuntimeError> {
        self.store
            .set_fuel(self.fuel_per_call)
            .map_err(|error| RuntimeError::new(RuntimeErrorKind::Execution, error))
    }

    fn consumed_fuel(&self) -> Result<u64, RuntimeError> {
        let remaining = self
            .store
            .get_fuel()
            .map_err(|error| RuntimeError::new(RuntimeErrorKind::Execution, error))?;
        self.fuel_per_call.checked_sub(remaining).ok_or_else(|| {
            RuntimeError::detail(
                RuntimeErrorKind::Execution,
                "guest fuel remaining exceeded the configured per-call budget",
            )
        })
    }
}

struct GuestExports {
    memory: Memory,
    abi_version: TypedFunc<(), i64>,
    alloc: TypedFunc<(i32, i32), i64>,
    dealloc: TypedFunc<(i32, i32, i32), i32>,
    build: TypedFunc<(i32, i32), i64>,
    diagnostic: Option<TypedFunc<(i32, i32), i64>>,
}

#[derive(Debug)]
struct CallbackStateExports {
    memory: Memory,
    abi_version: TypedFunc<(), i64>,
    initialize: Option<TypedFunc<i64, i64>>,
    set_window_metrics: Option<TypedFunc<(i32, i32, i64), i32>>,
    alloc: TypedFunc<(i32, i32), i64>,
    dealloc: TypedFunc<(i32, i32, i32), i32>,
    manifest: TypedFunc<(i32, i32), i64>,
    build: TypedFunc<(i32, i32), i64>,
    diagnostic: Option<TypedFunc<(i32, i32), i64>>,
    dispatch_event: TypedFunc<(i32, i32, i32, i32), i64>,
    poll_async: Option<TypedFunc<(i32, i32), i64>>,
    async_ready: Option<TypedFunc<(), i64>>,
    dispatch_async_event: Option<TypedFunc<(i32, i32, i32, i32), i64>>,
    export_state: TypedFunc<(i32, i32), i64>,
    import_state: TypedFunc<(i32, i32), i64>,
    migrate_state: Option<TypedFunc<(i32, i32, i32, i32), i64>>,
}

impl CallbackStateExports {
    fn new<T>(instance: &Instance, store: &Store<T>) -> Result<Self, RuntimeError> {
        let memory = instance.get_memory(store, "memory").ok_or_else(|| {
            RuntimeError::detail(RuntimeErrorKind::Export, "missing exported memory `memory`")
        })?;
        Ok(Self {
            memory,
            abi_version: instance
                .get_typed_func(store, "aimer_abi_version")
                .map_err(|error| RuntimeError::new(RuntimeErrorKind::Export, error))?,
            initialize: instance
                .get_func(store, "aimer_initialize")
                .map(|function| {
                    function
                        .typed(store)
                        .map_err(|error| RuntimeError::new(RuntimeErrorKind::Export, error))
                })
                .transpose()?,
            set_window_metrics: instance
                .get_func(store, "aimer_set_window_metrics")
                .map(|function| {
                    function
                        .typed(store)
                        .map_err(|error| RuntimeError::new(RuntimeErrorKind::Export, error))
                })
                .transpose()?,
            alloc: instance
                .get_typed_func(store, "aimer_alloc")
                .map_err(|error| RuntimeError::new(RuntimeErrorKind::Export, error))?,
            dealloc: instance
                .get_typed_func(store, "aimer_dealloc")
                .map_err(|error| RuntimeError::new(RuntimeErrorKind::Export, error))?,
            manifest: instance
                .get_typed_func(store, "aimer_manifest")
                .map_err(|error| RuntimeError::new(RuntimeErrorKind::Export, error))?,
            build: instance
                .get_typed_func(store, "aimer_build")
                .map_err(|error| RuntimeError::new(RuntimeErrorKind::Export, error))?,
            diagnostic: instance
                .get_func(store, "aimer_diagnostic")
                .map(|function| {
                    function
                        .typed(store)
                        .map_err(|error| RuntimeError::new(RuntimeErrorKind::Export, error))
                })
                .transpose()?,
            dispatch_event: instance
                .get_typed_func(store, "aimer_dispatch_event")
                .map_err(|error| RuntimeError::new(RuntimeErrorKind::Export, error))?,
            poll_async: instance
                .get_func(store, "aimer_poll_async")
                .map(|function| {
                    function
                        .typed(store)
                        .map_err(|error| RuntimeError::new(RuntimeErrorKind::Export, error))
                })
                .transpose()?,
            async_ready: instance
                .get_func(store, "aimer_async_ready")
                .map(|function| {
                    function
                        .typed(store)
                        .map_err(|error| RuntimeError::new(RuntimeErrorKind::Export, error))
                })
                .transpose()?,
            dispatch_async_event: instance
                .get_func(store, "aimer_dispatch_async_event")
                .map(|function| {
                    function
                        .typed(store)
                        .map_err(|error| RuntimeError::new(RuntimeErrorKind::Export, error))
                })
                .transpose()?,
            export_state: instance
                .get_typed_func(store, "aimer_export_state")
                .map_err(|error| RuntimeError::new(RuntimeErrorKind::Export, error))?,
            import_state: instance
                .get_typed_func(store, "aimer_import_state")
                .map_err(|error| RuntimeError::new(RuntimeErrorKind::Export, error))?,
            migrate_state: instance
                .get_func(store, "aimer_migrate_state")
                .map(|function| {
                    function
                        .typed(store)
                        .map_err(|error| RuntimeError::new(RuntimeErrorKind::Export, error))
                })
                .transpose()?,
        })
    }
}

impl GuestExports {
    fn new<T>(instance: &Instance, store: &Store<T>) -> Result<Self, RuntimeError> {
        let memory = instance.get_memory(store, "memory").ok_or_else(|| {
            RuntimeError::detail(RuntimeErrorKind::Export, "missing exported memory `memory`")
        })?;
        Ok(Self {
            memory,
            abi_version: instance
                .get_typed_func(store, "aimer_abi_version")
                .map_err(|error| RuntimeError::new(RuntimeErrorKind::Export, error))?,
            alloc: instance
                .get_typed_func(store, "aimer_alloc")
                .map_err(|error| RuntimeError::new(RuntimeErrorKind::Export, error))?,
            dealloc: instance
                .get_typed_func(store, "aimer_dealloc")
                .map_err(|error| RuntimeError::new(RuntimeErrorKind::Export, error))?,
            build: instance
                .get_typed_func(store, "aimer_build")
                .map_err(|error| RuntimeError::new(RuntimeErrorKind::Export, error))?,
            diagnostic: instance
                .get_func(store, "aimer_diagnostic")
                .map(|function| {
                    function
                        .typed(store)
                        .map_err(|error| RuntimeError::new(RuntimeErrorKind::Export, error))
                })
                .transpose()?,
        })
    }
}

fn decode_abi_result(packed: i64) -> Result<AbiResult, RuntimeError> {
    AbiResult::from_packed(packed)
        .map_err(|error| RuntimeError::new(RuntimeErrorKind::GuestStatus, error))
}

fn validate_memory_range<T>(
    memory: Memory,
    store: &Store<T>,
    pointer: u32,
    length: u32,
) -> Result<(), RuntimeError> {
    checked_guest_range(pointer, length, memory.data_size(store)).map(|_| ())
}

fn checked_guest_range(pointer: u32, length: u32, memory_size: usize) -> Result<u32, RuntimeError> {
    let end = pointer.checked_add(length).ok_or_else(|| {
        RuntimeError::detail(
            RuntimeErrorKind::GuestMemory,
            "guest output pointer arithmetic overflowed",
        )
    })?;
    if end as usize > memory_size {
        return Err(RuntimeError::detail(
            RuntimeErrorKind::GuestMemory,
            format!(
                "guest output range {pointer}..{end} exceeds memory size {}",
                memory_size
            ),
        ));
    }
    Ok(end)
}

const PERSISTENT_EXPORTS: [(&str, ExternalKind); 16] = [
    ("memory", ExternalKind::Memory),
    ("aimer_abi_version", ExternalKind::Func),
    ("aimer_initialize", ExternalKind::Func),
    ("aimer_set_window_metrics", ExternalKind::Func),
    ("aimer_alloc", ExternalKind::Func),
    ("aimer_dealloc", ExternalKind::Func),
    ("aimer_manifest", ExternalKind::Func),
    ("aimer_build", ExternalKind::Func),
    ("aimer_diagnostic", ExternalKind::Func),
    ("aimer_dispatch_event", ExternalKind::Func),
    ("aimer_poll_async", ExternalKind::Func),
    ("aimer_async_ready", ExternalKind::Func),
    ("aimer_dispatch_async_event", ExternalKind::Func),
    ("aimer_export_state", ExternalKind::Func),
    ("aimer_import_state", ExternalKind::Func),
    ("aimer_migrate_state", ExternalKind::Func),
];

fn validate_persistent_module_shape(
    module_bytes: &[u8],
    allow_capability_import: bool,
) -> Result<(), RuntimeError> {
    let mut memory_count = 0_u32;
    let mut capability_import_seen = false;
    let mut seen_exports = [false; PERSISTENT_EXPORTS.len()];
    for payload in Parser::new(0).parse_all(module_bytes) {
        match payload.map_err(|error| RuntimeError::new(RuntimeErrorKind::Module, error))? {
            Payload::ImportSection(imports) => {
                for import in imports {
                    let import = import
                        .map_err(|error| RuntimeError::new(RuntimeErrorKind::Module, error))?;
                    if !allow_capability_import
                        || import.module != "aimer"
                        || import.name != "capability_call"
                        || !matches!(import.ty, TypeRef::Func(_))
                        || capability_import_seen
                    {
                        return Err(RuntimeError::detail(
                            RuntimeErrorKind::Module,
                            format!(
                                "unsupported guest import `{}.{}`; only one `aimer.capability_call` function is allowed",
                                import.module, import.name
                            ),
                        ));
                    }
                    capability_import_seen = true;
                }
            }
            Payload::MemorySection(memories) => {
                memory_count = memory_count.checked_add(memories.count()).ok_or_else(|| {
                    RuntimeError::detail(
                        RuntimeErrorKind::Module,
                        "guest memory declaration count overflowed",
                    )
                })?;
            }
            Payload::ExportSection(exports) => {
                for export in exports {
                    let export = export
                        .map_err(|error| RuntimeError::new(RuntimeErrorKind::Module, error))?;
                    let Some(index) = PERSISTENT_EXPORTS
                        .iter()
                        .position(|(name, _)| *name == export.name)
                    else {
                        return Err(RuntimeError::detail(
                            RuntimeErrorKind::Export,
                            format!("undeclared guest export `{}`", export.name),
                        ));
                    };
                    if PERSISTENT_EXPORTS[index].1 != export.kind {
                        return Err(RuntimeError::detail(
                            RuntimeErrorKind::Export,
                            format!(
                                "guest export `{}` has the wrong WebAssembly kind",
                                export.name
                            ),
                        ));
                    }
                    if seen_exports[index] {
                        return Err(RuntimeError::detail(
                            RuntimeErrorKind::Export,
                            format!("duplicate guest export `{}`", export.name),
                        ));
                    }
                    seen_exports[index] = true;
                }
            }
            Payload::StartSection { .. } => {
                return Err(RuntimeError::detail(
                    RuntimeErrorKind::Module,
                    "guest start functions are unsupported; initialize through the Aimer ABI",
                ));
            }
            _ => {}
        }
    }
    if memory_count != 1 {
        return Err(RuntimeError::detail(
            RuntimeErrorKind::Module,
            format!("guest declares {memory_count} memories; exactly one is required"),
        ));
    }
    if let Some((missing, _)) = PERSISTENT_EXPORTS
        .iter()
        .zip(seen_exports)
        .find(|((name, _), seen)| {
            *name != "aimer_initialize"
                && *name != "aimer_set_window_metrics"
                && *name != "aimer_diagnostic"
                && *name != "aimer_migrate_state"
                && *name != "aimer_poll_async"
                && *name != "aimer_async_ready"
                && *name != "aimer_dispatch_async_event"
                && !seen
        })
        .map(|(expected, _)| expected)
    {
        return Err(RuntimeError::detail(
            RuntimeErrorKind::Export,
            format!("missing required guest export `{missing}`"),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{RuntimeErrorKind, checked_guest_range};

    #[test]
    fn guest_memory_range_arithmetic_matches_checked_integer_bounds() {
        let values = [0, 1, 65_535, 65_536, u32::MAX - 1, u32::MAX];
        let memory_sizes = [0, 1, 65_536, usize::MAX];

        for pointer in values {
            for length in values {
                for memory_size in memory_sizes {
                    let actual = checked_guest_range(pointer, length, memory_size);
                    let expected = pointer
                        .checked_add(length)
                        .is_some_and(|end| end as usize <= memory_size);
                    assert_eq!(actual.is_ok(), expected);
                    if let Err(error) = actual {
                        assert_eq!(error.kind(), RuntimeErrorKind::GuestMemory);
                    }
                }
            }
        }
    }
}
