use std::error::Error;
use std::fmt;
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::{self, JoinHandle};

use aimer_anteros::{
    CapabilityRegistry, GenerationId, GuestInstance, ReloadCoordinator, ReloadStage, Runtime,
    RuntimeConfig, StableId128, StateTransferCoordinator,
};
use aimer_reload_protocol::{ProtocolLimits, ReloadResult, SessionCredentials};
use aimer_reload_server::{ListenerError, ListenerSecurity, ReloadCommandListener};
use aimer_venus::LocalScheduler;
use aimer_widget::base::BuildContext;
use aimer_widget::{AnyElement, ErrorWidget, Widget, plan_element_reconciliation};

use super::{
    ProtocolReloadInbox, ReloadCandidateLimits, ReloadCandidatePreparer,
    reload_command_bridge_with_wake,
};

const CANDIDATE_REJECTED: u32 = 0x7101;
const SAFE_POINT_REJECTED: u32 = 0x7102;

/// Explicit runtime, protocol, and resource policy for one live reload host.
///
/// Aimer deliberately provides no permissive defaults for interpreter or model
/// limits. Applications must select measured limits and pass them here; a
/// release build cannot construct this type because the whole module is behind
/// Quiver's debug-only `wasm-hot-reload` feature.
pub struct LiveReloadConfig {
    runtime: RuntimeConfig,
    protocol: ProtocolLimits,
    security: ListenerSecurity,
    candidate: ReloadCandidateLimits,
    capabilities: CapabilityRegistry,
    state_transfer: StateTransferCoordinator,
    max_queued_events: usize,
    widget_ir_diagnostics: bool,
}

impl LiveReloadConfig {
    /// Creates a fail-closed host policy from explicit execution ceilings.
    #[inline]
    pub fn new(
        runtime: RuntimeConfig,
        protocol: ProtocolLimits,
        security: ListenerSecurity,
        candidate: ReloadCandidateLimits,
    ) -> Self {
        Self {
            runtime,
            protocol,
            security,
            candidate,
            capabilities: super::portable_http::default_registry(),
            state_transfer: StateTransferCoordinator::new(),
            max_queued_events: 0,
            widget_ir_diagnostics: false,
        }
    }

    /// Installs the permanent host capability registry used by every candidate.
    #[inline]
    pub fn capabilities(mut self, capabilities: CapabilityRegistry) -> Self {
        self.capabilities = capabilities;
        self
    }

    /// Installs the deterministic state-transfer and migration policy.
    #[inline]
    pub fn state_transfer(mut self, state_transfer: StateTransferCoordinator) -> Self {
        self.state_transfer = state_transfer;
        self
    }

    /// Sets the bounded FIFO size used while a candidate owns the event barrier.
    #[inline]
    pub const fn max_queued_events(mut self, max_queued_events: usize) -> Self {
        self.max_queued_events = max_queued_events;
        self
    }

    /// Enables verbose Widget IR stage output after successful native materialization.
    #[inline]
    pub const fn widget_ir_diagnostics(mut self, enabled: bool) -> Self {
        self.widget_ir_diagnostics = enabled;
        self
    }
}

/// Observable result of one candidate installed at the live safe point.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LiveReloadCommit {
    generation_id: GenerationId,
}

impl LiveReloadCommit {
    /// Returns the generation that became active atomically.
    #[inline]
    pub const fn generation_id(self) -> GenerationId {
        self.generation_id
    }
}

/// Owns the authenticated listener and the active interpreted application tree.
///
/// Network I/O is confined to a listener thread. Module instantiation, state
/// transfer, materialization, reconciliation, and root installation execute
/// only when [`Self::process_safe_point`] is called by the application thread.
pub struct LiveReloadHost {
    local_addr: SocketAddr,
    stop: Arc<AtomicBool>,
    listener_thread: Option<JoinHandle<()>>,
    inbox: Option<ProtocolReloadInbox>,
    runtime: Runtime,
    capabilities: CapabilityRegistry,
    state_transfer: StateTransferCoordinator,
    scheduler: Rc<LocalScheduler>,
    candidate_limits: ReloadCandidateLimits,
    max_queued_events: usize,
    widget_ir_diagnostics: bool,
    active: Option<ReloadCoordinator<GuestInstance, AnyElement, ()>>,
    callback_sender: mpsc::SyncSender<StableId128>,
    callback_receiver: mpsc::Receiver<StableId128>,
    dropped_callback_events: Arc<AtomicU64>,
    wake: Arc<dyn Fn() + Send + Sync>,
    callback_wake: Arc<dyn Fn() + Send + Sync>,
    next_event_sequence: u64,
    layout_required: bool,
    reload_diagnostic: Option<String>,
    reload_overlay: Option<AnyElement>,
}

impl LiveReloadHost {
    /// Binds an authenticated listener and starts its bounded network worker.
    pub fn bind(
        address: impl ToSocketAddrs,
        credentials: SessionCredentials,
        config: LiveReloadConfig,
        scheduler: Rc<LocalScheduler>,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Result<Self, LiveReloadStartError> {
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(wake);
        Self::bind_inner(address, credentials, config, scheduler, wake.clone(), wake)
    }

    /// Binds a listener with an independent wake for native callback events.
    ///
    /// Reload commands and animation polling may share one coalesced frame
    /// request, but a native callback needs its own event-loop wake. Keeping
    /// the two callbacks separate prevents an already pending animation frame
    /// from delaying the callback safe point.
    pub fn bind_with_callback_wake(
        address: impl ToSocketAddrs,
        credentials: SessionCredentials,
        config: LiveReloadConfig,
        scheduler: Rc<LocalScheduler>,
        wake: impl Fn() + Send + Sync + 'static,
        callback_wake: impl Fn() + Send + Sync + 'static,
    ) -> Result<Self, LiveReloadStartError> {
        Self::bind_inner(
            address,
            credentials,
            config,
            scheduler,
            Arc::new(wake),
            Arc::new(callback_wake),
        )
    }

    fn bind_inner(
        address: impl ToSocketAddrs,
        credentials: SessionCredentials,
        config: LiveReloadConfig,
        scheduler: Rc<LocalScheduler>,
        wake: Arc<dyn Fn() + Send + Sync>,
        callback_wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Self, LiveReloadStartError> {
        if config.max_queued_events == 0 {
            return Err(LiveReloadStartError::InvalidQueueCapacity);
        }
        let bridge_wake = Arc::clone(&wake);
        let (sink, inbox) = reload_command_bridge_with_wake(1, 0, move || bridge_wake());
        let (callback_sender, callback_receiver) = mpsc::sync_channel(config.max_queued_events);
        let listener = ReloadCommandListener::bind_secure(
            address,
            credentials,
            config.protocol,
            config.security,
            sink,
        )?;
        let local_addr = listener.local_addr()?;
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let listener_thread = thread::Builder::new()
            .name("aimer-live-reload-listener".to_owned())
            .spawn(move || {
                while !worker_stop.load(Ordering::Acquire) {
                    let _ = listener.accept_connection();
                }
            })?;
        Ok(Self {
            local_addr,
            stop,
            listener_thread: Some(listener_thread),
            inbox: Some(inbox),
            runtime: Runtime::new(config.runtime),
            capabilities: config.capabilities,
            state_transfer: config.state_transfer,
            scheduler,
            candidate_limits: config.candidate,
            max_queued_events: config.max_queued_events,
            widget_ir_diagnostics: config.widget_ir_diagnostics,
            active: None,
            callback_sender,
            callback_receiver,
            dropped_callback_events: Arc::new(AtomicU64::new(0)),
            wake,
            callback_wake,
            next_event_sequence: 1,
            layout_required: false,
            reload_diagnostic: None,
            reload_overlay: None,
        })
    }

    /// Returns the concrete listener endpoint, including an OS-selected port.
    #[inline]
    pub const fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Returns the active guest generation, if the initial module has committed.
    #[inline]
    pub fn active_generation(&self) -> Option<GenerationId> {
        self.active
            .as_ref()
            .map(|coordinator| coordinator.active().generation_id())
    }

    /// Returns callback events rejected because the bounded host queue was full.
    #[inline]
    pub fn dropped_callback_events(&self) -> u64 {
        self.dropped_callback_events.load(Ordering::Acquire)
    }

    /// Borrows the only interpreted root visible to event and render dispatch.
    #[inline]
    pub fn active_root(&self) -> Option<&AnyElement> {
        self.active
            .as_ref()
            .map(|coordinator| coordinator.active().root())
    }

    /// Returns the current host-owned reload diagnostic, if a candidate is
    /// rejected or a guest callback fails.
    #[inline]
    pub fn reload_diagnostic(&self) -> Option<&str> {
        self.reload_diagnostic.as_deref()
    }

    /// Borrows the host-owned error overlay drawn above the active root.
    #[inline]
    pub(crate) fn reload_overlay(&self) -> Option<&AnyElement> {
        self.reload_overlay.as_ref()
    }

    fn show_reload_diagnostic(&mut self, ctx: &BuildContext, diagnostic: String) {
        aimer_utils::error!("Aimer hot reload diagnostic: {diagnostic}");
        let diagnostic = bound_reload_diagnostic(diagnostic);
        self.reload_overlay = Some(ErrorWidget::new(diagnostic.clone()).to_element(ctx));
        self.reload_diagnostic = Some(diagnostic);
        self.layout_required = true;
    }

    fn clear_reload_diagnostic(&mut self) {
        if self.reload_diagnostic.take().is_some() {
            self.reload_overlay = None;
            self.layout_required = true;
        }
    }

    /// Exports the active guest state for diagnostics and proof tooling.
    pub fn export_active_state(&mut self) -> Result<Option<Vec<u8>>, aimer_anteros::RuntimeError> {
        let Some(coordinator) = self.active.as_mut() else {
            return Ok(None);
        };
        coordinator
            .active_mut()
            .generation_mut()
            .guest_mut()
            .export_state(self.candidate_limits.model())
            .map(|state| Some(state.as_bytes().to_vec()))
    }

    /// Registers one host-capability task owned by the active generation.
    ///
    /// The returned identity is safe to place in a bounded
    /// [`aimer_anteros::AsyncCallbackEvent`]. The actual I/O remains in the
    /// typed capability provider and is never captured by the guest tree.
    pub fn register_active_async_task(
        &mut self,
        callback_id: StableId128,
    ) -> Result<aimer_anteros::AsyncTaskId, LiveReloadError> {
        let Some(coordinator) = self.active.as_mut() else {
            return Err(LiveReloadError::Callback(
                "no active guest generation owns async callbacks".to_owned(),
            ));
        };
        coordinator
            .active_mut()
            .generation_mut()
            .register_async_task(callback_id)
            .map_err(|error| LiveReloadError::Callback(error.to_string()))
    }

    /// Delivers one host-owned async completion at the application safe point.
    ///
    /// Validation consumes the task only after all identity, ordering, and
    /// payload checks succeed. A rejected event leaves the active tree and
    /// generation task table unchanged.
    pub fn dispatch_async_event(
        &mut self,
        event_bytes: &[u8],
        ctx: &BuildContext,
    ) -> Result<(), LiveReloadError> {
        let Some(coordinator) = self.active.as_mut() else {
            return Err(LiveReloadError::Callback(
                "no active guest generation owns async callbacks".to_owned(),
            ));
        };
        let generation = coordinator.active_mut().generation_mut();
        generation
            .validate_async_event(event_bytes, self.candidate_limits.model())
            .map_err(|error| LiveReloadError::Callback(error.to_string()))?;
        set_guest_window_metrics(generation.guest_mut(), ctx)?;
        let image = generation
            .guest_mut()
            .dispatch_async_event(event_bytes, self.candidate_limits.model())
            .map_err(|error| LiveReloadError::Callback(error.to_string()))?;
        if let Some(image) = image {
            self.install_widget_image(image, ctx)?;
        }
        self.request_async_frame()
    }

    /// Returns and clears whether the active disconnected root needs first layout.
    #[inline]
    #[doc(hidden)]
    pub fn take_layout_required(&mut self) -> bool {
        std::mem::take(&mut self.layout_required)
    }

    /// Processes at most one authenticated command at the application safe point.
    pub fn process_safe_point(
        &mut self,
        ctx: &BuildContext,
    ) -> Result<Option<LiveReloadCommit>, LiveReloadError> {
        self.poll_async(ctx)?;
        while let Ok(callback_id) = self.callback_receiver.try_recv() {
            if let Err(error) = self.dispatch_callback(callback_id, ctx) {
                self.show_reload_diagnostic(ctx, error.to_string());
                return Err(error);
            }
        }
        let pending = match self
            .inbox
            .as_ref()
            .ok_or(LiveReloadError::ListenerDisconnected)?
            .try_recv()
        {
            Ok(pending) => pending,
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                self.request_async_frame()?;
                return Ok(None);
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                return Err(LiveReloadError::ListenerDisconnected);
            }
        };
        let generation_id = GenerationId::new(
            self.active_generation()
                .map_or(1, |generation| generation.get().saturating_add(1)),
        );
        let preparer = ReloadCandidatePreparer::new(
            &self.runtime,
            &self.capabilities,
            &self.state_transfer,
            Rc::clone(&self.scheduler),
            self.candidate_limits,
        )
        .widget_ir_diagnostics(self.widget_ir_diagnostics);
        let callback_sender = self.callback_sender.clone();
        let callback_wake = Arc::clone(&self.callback_wake);
        let dropped_callback_events = Arc::clone(&self.dropped_callback_events);
        let dispatch_callback = move |callback_id| {
            enqueue_callback(
                &callback_sender,
                &callback_wake,
                &dropped_callback_events,
                callback_id,
            );
        };

        let Some(coordinator) = self.active.as_mut() else {
            let snapshot = match preparer.prepare_initial(
                pending.command().module(),
                generation_id,
                ctx,
                dispatch_callback,
            ) {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    let stage = error.stage();
                    let diagnostic = error.to_string();
                    self.show_reload_diagnostic(ctx, diagnostic.clone());
                    pending.complete_rejection(stage, CANDIDATE_REJECTED, 0, diagnostic)?;
                    return Ok(None);
                }
            };
            let mut coordinator =
                ReloadCoordinator::new(snapshot).max_queued_events(self.max_queued_events);
            coordinator
                .active_mut()
                .generation_mut()
                .guest_mut()
                .activate();
            coordinator.active().root().invalidate_layout();
            self.active = Some(coordinator);
            self.clear_reload_diagnostic();
            self.layout_required = true;
            pending.complete(ReloadResult::Committed {
                active_generation: generation_id.get(),
                reset_state_entries: 0,
                cleanup_warnings: 0,
            })?;
            self.request_async_frame()?;
            return Ok(Some(LiveReloadCommit { generation_id }));
        };

        let transaction = coordinator.begin_reload();
        let prepared = match preparer.prepare(
            pending.command().module(),
            generation_id,
            coordinator.active_mut(),
            ctx,
            dispatch_callback,
        ) {
            Ok(prepared) => prepared,
            Err(error) => {
                let stage = error.stage();
                let diagnostic = error.to_string();
                let _ = coordinator.rollback(transaction);
                let active_generation = coordinator.active().generation_id().get();
                self.show_reload_diagnostic(ctx, diagnostic.clone());
                pending.complete_rejection(
                    stage,
                    CANDIDATE_REJECTED,
                    active_generation,
                    diagnostic,
                )?;
                return Ok(None);
            }
        };
        let reset_state_entries =
            u32::try_from(prepared.state_transfer_report().reset_state_ids().len())
                .unwrap_or(u32::MAX);
        coordinator.stage_candidate(transaction, prepared.into_snapshot())?;
        let commit = match coordinator.commit(
            transaction,
            |old, candidate| {
                plan_element_reconciliation(old.root().as_ref(), candidate.root().as_ref())
                    .validate()
            },
            |old, candidate| {
                plan_element_reconciliation(old.root().as_ref(), candidate.root().as_ref())
                    .commit(ctx)
                    .expect("validated live reconciliation changed during safe-point commit");
            },
        ) {
            Ok(commit) => commit,
            Err(error) => {
                let diagnostic = format!("live safe-point reconciliation failed: {error:?}");
                let active_generation = coordinator.active().generation_id().get();
                self.show_reload_diagnostic(ctx, diagnostic.clone());
                pending.complete_rejection(
                    ReloadStage::PrepareReconciliation,
                    SAFE_POINT_REJECTED,
                    active_generation,
                    diagnostic,
                )?;
                return Ok(None);
            }
        };
        coordinator.active().root().invalidate_layout();
        self.clear_reload_diagnostic();
        self.layout_required = true;
        pending.complete_commit(&commit, reset_state_entries, 0)?;
        self.request_async_frame()?;
        Ok(Some(LiveReloadCommit { generation_id }))
    }

    fn poll_async(&mut self, ctx: &BuildContext) -> Result<(), LiveReloadError> {
        let Some(coordinator) = self.active.as_mut() else {
            return Ok(());
        };
        let generation = coordinator.active_mut().generation_mut();
        set_guest_window_metrics(generation.guest_mut(), ctx)?;
        let Some(widget_image) = generation
            .guest_mut()
            .poll_async(self.candidate_limits.model())
            .map_err(|error| LiveReloadError::Callback(error.to_string()))?
        else {
            return Ok(());
        };
        self.install_widget_image(widget_image, ctx)
    }

    fn request_async_frame(&mut self) -> Result<(), LiveReloadError> {
        let Some(coordinator) = self.active.as_mut() else {
            return Ok(());
        };
        let has_async_work = coordinator
            .active_mut()
            .generation_mut()
            .guest_mut()
            .has_async_work()
            .map_err(|error| LiveReloadError::Callback(error.to_string()))?;
        if has_async_work {
            (self.wake)();
        }
        Ok(())
    }

    fn install_widget_image(
        &mut self,
        widget_image: aimer_anteros::WidgetImage,
        ctx: &BuildContext,
    ) -> Result<(), LiveReloadError> {
        let Some(coordinator) = self.active.as_mut() else {
            return Ok(());
        };
        let snapshot = coordinator.active_mut();
        let generation = snapshot.generation_mut();
        if GenerationId::new(widget_image.view().generation_id()) != generation.generation_id() {
            return Err(LiveReloadError::Callback(
                "async callback rebuild changed its active generation identity".to_owned(),
            ));
        }
        let callback_sender = self.callback_sender.clone();
        let callback_wake = Arc::clone(&self.callback_wake);
        let dropped_callback_events = Arc::clone(&self.dropped_callback_events);
        let mut root = super::materialize_aimer_widget_tree(
            widget_image.as_bytes(),
            self.candidate_limits.model(),
            ctx,
            move |callback_id| {
                enqueue_callback(
                    &callback_sender,
                    &callback_wake,
                    &dropped_callback_events,
                    callback_id,
                );
            },
        )
        .map_err(|error| LiveReloadError::Callback(error.to_string()))?;
        generation
            .replace_callback_bindings(
                &widget_image.view(),
                self.candidate_limits.max_callback_bindings(),
            )
            .map_err(|error| LiveReloadError::Callback(error.to_string()))?;
        if let Some(diagnostics) = super::WidgetIrStageDiagnostics::new(self.widget_ir_diagnostics)
            .render(widget_image.as_bytes(), self.candidate_limits.model())
            .map_err(|error| LiveReloadError::Callback(error.to_string()))?
        {
            eprintln!("{diagnostics}");
        }
        plan_element_reconciliation(snapshot.root().as_ref(), root.as_ref())
            .commit(ctx)
            .map_err(|error| LiveReloadError::Callback(error.to_string()))?;
        std::mem::swap(snapshot.root_mut(), &mut root);
        snapshot.root().invalidate_layout();
        self.layout_required = true;
        Ok(())
    }

    fn dispatch_callback(
        &mut self,
        callback_id: StableId128,
        ctx: &BuildContext,
    ) -> Result<(), LiveReloadError> {
        let Some(coordinator) = self.active.as_mut() else {
            return Ok(());
        };
        let snapshot = coordinator.active_mut();
        let generation = snapshot.generation_mut();
        let sequence = self.next_event_sequence;
        self.next_event_sequence = self.next_event_sequence.saturating_add(1);
        let event = generation
            .encode_callback_event(
                callback_id,
                sequence,
                sequence,
                &[],
                self.candidate_limits.model(),
            )
            .map_err(|error| LiveReloadError::Callback(error.to_string()))?;
        generation
            .validate_event(&event, self.candidate_limits.model())
            .map_err(|error| LiveReloadError::Callback(error.to_string()))?;
        set_guest_window_metrics(generation.guest_mut(), ctx)?;
        let Some(widget_image) = generation
            .guest_mut()
            .dispatch_event(&event, self.candidate_limits.model())
            .map_err(|error| LiveReloadError::Callback(error.to_string()))?
        else {
            return Ok(());
        };
        self.install_widget_image(widget_image, ctx)
    }
}

fn set_guest_window_metrics(
    guest: &mut GuestInstance,
    ctx: &BuildContext,
) -> Result<(), LiveReloadError> {
    let size = ctx.window.inner_size();
    guest
        .set_window_metrics(size.width, size.height, ctx.window.scale_factor())
        .map_err(|error| LiveReloadError::Callback(error.to_string()))
}

const RELOAD_DIAGNOSTIC_SUFFIX: &str = "\n\n[reload diagnostic truncated]";

fn bound_reload_diagnostic(mut diagnostic: String) -> String {
    let maximum = aimer_anteros::MAX_GUEST_DIAGNOSTIC_BYTES;
    if diagnostic.len() <= maximum {
        return diagnostic;
    }

    let keep = maximum.saturating_sub(RELOAD_DIAGNOSTIC_SUFFIX.len());
    let mut end = keep.min(diagnostic.len());
    while end > 0 && !diagnostic.is_char_boundary(end) {
        end -= 1;
    }
    diagnostic.truncate(end);
    diagnostic.push_str(RELOAD_DIAGNOSTIC_SUFFIX);
    diagnostic
}

fn enqueue_callback(
    sender: &mpsc::SyncSender<StableId128>,
    wake: &Arc<dyn Fn() + Send + Sync>,
    dropped: &AtomicU64,
    callback_id: StableId128,
) {
    match sender.try_send(callback_id) {
        Ok(()) => wake(),
        Err(mpsc::TrySendError::Full(_)) => {
            dropped.fetch_add(1, Ordering::AcqRel);
            aimer_utils::error!("Aimer hot reload callback queue is full; the event was rejected");
        }
        Err(mpsc::TrySendError::Disconnected(_)) => {
            dropped.fetch_add(1, Ordering::AcqRel);
            aimer_utils::error!(
                "Aimer hot reload callback queue is disconnected; the event was rejected"
            );
        }
    }
}

impl Drop for LiveReloadHost {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.inbox.take();
        let _ = TcpStream::connect(self.local_addr);
        if let Some(thread) = self.listener_thread.take() {
            let _ = thread.join();
        }
    }
}

/// Failure to bind or start the live reload listener.
#[derive(Debug)]
pub enum LiveReloadStartError {
    /// Callback and event barriers require at least one bounded queue slot.
    InvalidQueueCapacity,
    /// The authenticated server could not bind or inspect its endpoint.
    Listener(ListenerError),
    /// The listener worker thread could not be created.
    Thread(std::io::Error),
}

impl fmt::Display for LiveReloadStartError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidQueueCapacity => {
                formatter.write_str("live reload queue capacity must be non-zero")
            }
            Self::Listener(error) => write!(formatter, "live reload listener failed: {error}"),
            Self::Thread(error) => write!(formatter, "live reload worker failed: {error}"),
        }
    }
}

impl Error for LiveReloadStartError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidQueueCapacity => None,
            Self::Listener(error) => Some(error),
            Self::Thread(error) => Some(error),
        }
    }
}

impl From<ListenerError> for LiveReloadStartError {
    #[inline]
    fn from(error: ListenerError) -> Self {
        Self::Listener(error)
    }
}

impl From<std::io::Error> for LiveReloadStartError {
    #[inline]
    fn from(error: std::io::Error) -> Self {
        Self::Thread(error)
    }
}

/// Application-thread failure outside a candidate's structured rejection path.
#[derive(Debug)]
pub enum LiveReloadError {
    /// The listener-side command sender ended unexpectedly.
    ListenerDisconnected,
    /// The protocol client disconnected before receiving the terminal result.
    ResultDisconnected,
    /// The reload transaction violated its ownership state machine.
    Transaction(aimer_anteros::ReloadTransactionError),
    /// A callback could not be validated, dispatched, or materialized.
    Callback(String),
}

impl fmt::Display for LiveReloadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ListenerDisconnected => formatter.write_str("live reload listener disconnected"),
            Self::ResultDisconnected => formatter.write_str("live reload client disconnected"),
            Self::Transaction(error) => {
                write!(formatter, "live reload transaction failed: {error}")
            }
            Self::Callback(error) => write!(formatter, "live reload callback failed: {error}"),
        }
    }
}

impl Error for LiveReloadError {}

impl From<std::sync::mpsc::SendError<ReloadResult>> for LiveReloadError {
    #[inline]
    fn from(_: std::sync::mpsc::SendError<ReloadResult>) -> Self {
        Self::ResultDisconnected
    }
}

impl From<aimer_anteros::ReloadTransactionError> for LiveReloadError {
    #[inline]
    fn from(error: aimer_anteros::ReloadTransactionError) -> Self {
        Self::Transaction(error)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[test]
    fn bounded_callback_queue_reports_rejected_events() {
        let (sender, receiver) = mpsc::sync_channel(1);
        let wakes = Arc::new(AtomicUsize::new(0));
        let wake_count = Arc::clone(&wakes);
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
            wake_count.fetch_add(1, Ordering::AcqRel);
        });
        let dropped = AtomicU64::new(0);

        enqueue_callback(&sender, &wake, &dropped, StableId128::from_bytes([1; 16]));
        enqueue_callback(&sender, &wake, &dropped, StableId128::from_bytes([2; 16]));

        assert_eq!(
            receiver.try_recv().unwrap(),
            StableId128::from_bytes([1; 16])
        );
        assert_eq!(wakes.load(Ordering::Acquire), 1);
        assert_eq!(dropped.load(Ordering::Acquire), 1);
    }

    #[test]
    fn reload_diagnostic_bound_preserves_utf8_and_suffix() {
        let diagnostic = "🙂".repeat(aimer_anteros::MAX_GUEST_DIAGNOSTIC_BYTES);
        let bounded = bound_reload_diagnostic(diagnostic);

        assert!(bounded.len() <= aimer_anteros::MAX_GUEST_DIAGNOSTIC_BYTES);
        assert!(bounded.ends_with(RELOAD_DIAGNOSTIC_SUFFIX));
        assert!(bounded.is_char_boundary(bounded.len()));
    }
}
