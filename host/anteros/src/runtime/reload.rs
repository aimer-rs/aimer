use std::error::Error;
use std::fmt;

use crate::{Generation, GenerationId};

/// Guest lifecycle operations performed only by the transactional reload owner.
///
/// Candidate guests remain inactive while fallible preparation runs. Activation
/// occurs at the host safe point immediately before the coherent snapshot swap.
/// Retirement is idempotently requested for rejected, superseded, and replaced
/// snapshots before their guest payload is dropped.
pub trait ReloadGuest {
    /// Publishes effects that were staged while this guest was a candidate.
    fn activate(&mut self);

    /// Rejects future guest work and releases guest-owned host bindings.
    fn retire(&mut self);
}

impl ReloadGuest for () {
    #[inline]
    fn activate(&mut self) {}

    #[inline]
    fn retire(&mut self) {}
}

impl<T> ReloadGuest for Box<T>
where
    T: ReloadGuest + ?Sized,
{
    #[inline]
    fn activate(&mut self) {
        self.as_mut().activate();
    }

    #[inline]
    fn retire(&mut self) {
        self.as_mut().retire();
    }
}

#[cfg(feature = "wasm-hot-reload")]
impl ReloadGuest for crate::GuestInstance {
    #[inline]
    fn activate(&mut self) {
        crate::GuestInstance::activate(self);
    }

    #[inline]
    fn retire(&mut self) {
        crate::GuestInstance::retire(self);
    }
}

/// One coherent guest generation, callback table, and disconnected native root.
///
/// The generation owns callback and asynchronous-resource state. Keeping its
/// root in the same value prevents a host from publishing a new tree with an
/// old callback table or generation identity.
pub struct ReloadSnapshot<G: ReloadGuest, R> {
    generation: Generation<G>,
    root: R,
    retired: bool,
}

impl<G: ReloadGuest, R> ReloadSnapshot<G, R> {
    /// Creates one snapshot owned by a reload coordinator.
    #[inline]
    pub const fn new(generation: Generation<G>, root: R) -> Self {
        Self {
            generation,
            root,
            retired: false,
        }
    }

    /// Returns the identity shared by this root and callback generation.
    #[inline]
    pub const fn generation_id(&self) -> GenerationId {
        self.generation.generation_id()
    }

    /// Borrows this snapshot's generation and callback owner.
    #[inline]
    pub const fn generation(&self) -> &Generation<G> {
        &self.generation
    }

    /// Mutably borrows the generation for one serialized callback operation.
    #[inline]
    pub const fn generation_mut(&mut self) -> &mut Generation<G> {
        &mut self.generation
    }

    /// Borrows the native root installed with this generation.
    #[inline]
    pub const fn root(&self) -> &R {
        &self.root
    }

    /// Mutably borrows the native root during the infallible safe-point carry.
    #[inline]
    pub const fn root_mut(&mut self) -> &mut R {
        &mut self.root
    }

    fn activate(&mut self) {
        self.generation.guest_mut().activate();
    }

    fn retire(&mut self) {
        if self.retired {
            return;
        }
        self.retired = true;
        self.generation.retire();
        self.generation.guest_mut().retire();
    }
}

impl<G: ReloadGuest, R> Drop for ReloadSnapshot<G, R> {
    fn drop(&mut self) {
        self.retire();
    }
}

/// Monotonic identity of one candidate preparation attempt.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ReloadTransactionId(u64);

impl ReloadTransactionId {
    /// Returns the host-local transaction sequence.
    #[inline]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// The immediate disposition of an event submitted to the reload barrier.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReloadEventDisposition<E> {
    /// No transaction is open, so the caller must dispatch this event now.
    Dispatch(E),
    /// A transaction is open and the event was retained for deterministic replay.
    Queued,
}

/// An event that could not enter the bounded reload barrier.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReloadEventOverflow<E> {
    event: E,
    limit: usize,
}

impl<E> ReloadEventOverflow<E> {
    /// Borrows the event which remains owned by the caller-visible error.
    #[inline]
    pub const fn event(&self) -> &E {
        &self.event
    }

    /// Returns the configured maximum queued event count.
    #[inline]
    pub const fn limit(&self) -> usize {
        self.limit
    }

    /// Recovers the undispatched event without cloning it.
    #[inline]
    pub fn into_event(self) -> E {
        self.event
    }
}

/// A transaction token does not identify the currently open reload.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReloadTransactionError {
    /// A newer transaction replaced the supplied transaction.
    Superseded {
        supplied: ReloadTransactionId,
        current: ReloadTransactionId,
    },
    /// No reload transaction is currently open.
    NoTransaction,
    /// The current transaction has not staged a complete candidate snapshot.
    CandidateNotStaged { transaction: ReloadTransactionId },
}

impl fmt::Display for ReloadTransactionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Superseded { supplied, current } => write!(
                formatter,
                "reload transaction {} was superseded by {}",
                supplied.get(),
                current.get()
            ),
            Self::NoTransaction => formatter.write_str("no reload transaction is open"),
            Self::CandidateNotStaged { transaction } => write!(
                formatter,
                "reload transaction {} has no staged candidate",
                transaction.get()
            ),
        }
    }
}

impl Error for ReloadTransactionError {}

/// A fallible boundary before native state carry begins.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReloadStage {
    /// Module envelope, policy, or host compatibility preflight.
    Preflight,
    /// Candidate runtime instantiation and import linking.
    Instantiate,
    /// Candidate initialization and default-state creation.
    Initialize,
    /// Active-generation state export behind the event barrier.
    ExportState,
    /// Candidate-owned state migration.
    MigrateState,
    /// Candidate state import and verification export.
    ImportState,
    /// Candidate Widget IR build.
    Build,
    /// Canonical Widget IR and callback validation.
    Validate,
    /// Disconnected native tree materialization.
    Materialize,
    /// Side-effect-free native reconciliation planning.
    PrepareReconciliation,
    /// Cancellation immediately before the host safe point.
    PreCommitCancellation,
}

/// Structured rollback result for a failed candidate preparation stage.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReloadRejection<D, E> {
    stage: ReloadStage,
    error: D,
    replay: ReloadReplay<E>,
}

impl<D, E> ReloadRejection<D, E> {
    /// Returns the preparation boundary that rejected the candidate.
    #[inline]
    pub const fn stage(&self) -> ReloadStage {
        self.stage
    }

    /// Borrows the stage-specific failure diagnostic.
    #[inline]
    pub const fn error(&self) -> &D {
        &self.error
    }

    /// Borrows events released to the unchanged active generation.
    #[inline]
    pub const fn replay(&self) -> &ReloadReplay<E> {
        &self.replay
    }

    /// Moves out events released to the unchanged active generation.
    #[inline]
    pub fn into_replay(self) -> ReloadReplay<E> {
        self.replay
    }
}

/// Events released exactly once when a transaction commits or rolls back.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReloadReplay<E> {
    events: Vec<E>,
}

impl<E> ReloadReplay<E> {
    /// Borrows the retained events in FIFO order.
    #[inline]
    pub fn as_slice(&self) -> &[E] {
        &self.events
    }

    /// Returns the number of events awaiting replay.
    #[inline]
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// Returns whether no events were retained.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Attempts every retained event exactly once in FIFO order.
    ///
    /// Dispatch continues after an error so a callback removed by the new
    /// generation cannot suppress unrelated later input. Each failure records
    /// its stable queue position for terminal reload diagnostics.
    pub fn dispatch<D>(
        self,
        mut dispatch: impl FnMut(E) -> Result<(), D>,
    ) -> ReloadReplayReport<D> {
        let attempted_events = self.events.len();
        let mut delivered_events = 0;
        let mut failures = Vec::new();
        for (event_index, event) in self.events.into_iter().enumerate() {
            match dispatch(event) {
                Ok(()) => delivered_events += 1,
                Err(error) => failures.push(ReloadReplayFailure { event_index, error }),
            }
        }
        ReloadReplayReport {
            attempted_events,
            delivered_events,
            failures,
        }
    }
}

impl<E> IntoIterator for ReloadReplay<E> {
    type Item = E;
    type IntoIter = std::vec::IntoIter<E>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.events.into_iter()
    }
}

/// One callback or host-event diagnostic produced during deterministic replay.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReloadReplayFailure<D> {
    event_index: usize,
    error: D,
}

impl<D> ReloadReplayFailure<D> {
    /// Returns this event's zero-based position in the retained FIFO.
    #[inline]
    pub const fn event_index(&self) -> usize {
        self.event_index
    }

    /// Borrows the event-specific dispatch diagnostic.
    #[inline]
    pub const fn error(&self) -> &D {
        &self.error
    }
}

/// Exactly-once replay counts and event-specific delivery diagnostics.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReloadReplayReport<D> {
    attempted_events: usize,
    delivered_events: usize,
    failures: Vec<ReloadReplayFailure<D>>,
}

impl<D> ReloadReplayReport<D> {
    /// Returns the complete number of events removed from the barrier.
    #[inline]
    pub const fn attempted_events(&self) -> usize {
        self.attempted_events
    }

    /// Returns the number of events accepted by their replay target.
    #[inline]
    pub const fn delivered_events(&self) -> usize {
        self.delivered_events
    }

    /// Borrows callback-removal and other dispatch diagnostics in FIFO order.
    #[inline]
    pub fn failures(&self) -> &[ReloadReplayFailure<D>] {
        &self.failures
    }
}

/// Result of one successful safe-point snapshot installation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReloadCommit<E> {
    generation_id: GenerationId,
    replay: ReloadReplay<E>,
}

impl<E> ReloadCommit<E> {
    /// Returns the newly active generation.
    #[inline]
    pub const fn generation_id(&self) -> GenerationId {
        self.generation_id
    }

    /// Borrows the deterministic post-commit replay batch.
    #[inline]
    pub const fn replay(&self) -> &ReloadReplay<E> {
        &self.replay
    }

    /// Moves out the deterministic post-commit replay batch.
    #[inline]
    pub fn into_replay(self) -> ReloadReplay<E> {
        self.replay
    }
}

/// Failure before the safe-point mutation boundary.
#[derive(Debug)]
pub enum ReloadCommitError<P, E> {
    /// The transaction token or staged-candidate state was invalid.
    Transaction(ReloadTransactionError),
    /// Candidate preflight failed; the candidate was retired and old events resumed.
    Preflight { error: P, replay: ReloadReplay<E> },
}

impl<P, E> ReloadCommitError<P, E> {
    /// Borrows the candidate preflight error, when preparation reached that stage.
    #[inline]
    pub const fn preflight_error(&self) -> Option<&P> {
        match self {
            Self::Transaction(_) => None,
            Self::Preflight { error, .. } => Some(error),
        }
    }

    /// Borrows events released by a preflight rollback.
    #[inline]
    pub const fn replay(&self) -> Option<&ReloadReplay<E>> {
        match self {
            Self::Transaction(_) => None,
            Self::Preflight { replay, .. } => Some(replay),
        }
    }

    /// Moves out events released by a preflight rollback.
    #[inline]
    pub fn into_replay(self) -> Option<ReloadReplay<E>> {
        match self {
            Self::Transaction(_) => None,
            Self::Preflight { replay, .. } => Some(replay),
        }
    }
}

/// Owns the active snapshot, staged candidate, and bounded event barrier.
///
/// All guest execution, state transfer, Widget IR validation, materialization,
/// and reconciliation validation must finish before [`Self::commit`]. The final
/// commit callback must be infallible; it may only carry native state and apply
/// the already validated reconciliation plan.
pub struct ReloadCoordinator<G: ReloadGuest, R, E> {
    active: ReloadSnapshot<G, R>,
    candidate: Option<(ReloadTransactionId, ReloadSnapshot<G, R>)>,
    current: Option<ReloadTransactionId>,
    next_transaction: u64,
    max_queued_events: usize,
    queued_events: Vec<E>,
}

impl<G: ReloadGuest, R, E> ReloadCoordinator<G, R, E> {
    /// Creates a coordinator with a fail-closed zero-length event barrier.
    #[inline]
    pub const fn new(active: ReloadSnapshot<G, R>) -> Self {
        Self {
            active,
            candidate: None,
            current: None,
            next_transaction: 1,
            max_queued_events: 0,
            queued_events: Vec::new(),
        }
    }

    /// Sets the hard count limit for events retained during preparation.
    #[inline]
    pub const fn max_queued_events(mut self, limit: usize) -> Self {
        self.max_queued_events = limit;
        self
    }

    /// Borrows the only snapshot visible to event and render dispatch.
    #[inline]
    pub const fn active(&self) -> &ReloadSnapshot<G, R> {
        &self.active
    }

    /// Mutably borrows the active snapshot for one serialized host operation.
    #[inline]
    pub const fn active_mut(&mut self) -> &mut ReloadSnapshot<G, R> {
        &mut self.active
    }

    /// Returns the number of events currently retained by the barrier.
    #[inline]
    pub fn queued_event_count(&self) -> usize {
        self.queued_events.len()
    }

    /// Returns the transaction currently holding the event barrier open.
    #[inline]
    pub const fn current_transaction(&self) -> Option<ReloadTransactionId> {
        self.current
    }

    /// Opens an event barrier and supersedes any staged candidate.
    ///
    /// Events already queued for an earlier candidate remain behind the same
    /// barrier and are replayed only when the newest attempt commits or rolls
    /// back. Superseded candidate resources are retired immediately.
    pub fn begin_reload(&mut self) -> ReloadTransactionId {
        self.candidate.take();
        let transaction = ReloadTransactionId(self.next_transaction);
        self.next_transaction = self
            .next_transaction
            .checked_add(1)
            .expect("reload transaction identity space exhausted");
        self.current = Some(transaction);
        transaction
    }

    /// Transfers ownership of a fully prepared disconnected candidate.
    pub fn stage_candidate(
        &mut self,
        transaction: ReloadTransactionId,
        candidate: ReloadSnapshot<G, R>,
    ) -> Result<(), ReloadTransactionError> {
        let current = self.current.ok_or(ReloadTransactionError::NoTransaction)?;
        if transaction != current {
            return Err(ReloadTransactionError::Superseded {
                supplied: transaction,
                current,
            });
        }
        self.candidate = Some((transaction, candidate));
        Ok(())
    }

    /// Dispatches immediately or retains one event behind the active barrier.
    pub fn route_event(
        &mut self,
        event: E,
    ) -> Result<ReloadEventDisposition<E>, ReloadEventOverflow<E>> {
        if self.current.is_none() {
            return Ok(ReloadEventDisposition::Dispatch(event));
        }
        if self.queued_events.len() >= self.max_queued_events {
            return Err(ReloadEventOverflow {
                event,
                limit: self.max_queued_events,
            });
        }
        self.queued_events.push(event);
        Ok(ReloadEventDisposition::Queued)
    }

    /// Validates and atomically installs the current candidate at a host safe point.
    ///
    /// `preflight` is the last fallible operation and must not mutate either
    /// snapshot. If it fails, the candidate is destroyed and queued events are
    /// released for the unchanged active snapshot. `commit` runs only after
    /// preflight succeeds and must be infallible.
    pub fn commit<P>(
        &mut self,
        transaction: ReloadTransactionId,
        preflight: impl FnOnce(&ReloadSnapshot<G, R>, &ReloadSnapshot<G, R>) -> Result<(), P>,
        commit: impl FnOnce(&mut ReloadSnapshot<G, R>, &mut ReloadSnapshot<G, R>),
    ) -> Result<ReloadCommit<E>, ReloadCommitError<P, E>> {
        self.commit_prepared(transaction, preflight, |old, candidate, ()| {
            commit(old, candidate)
        })
    }

    /// Prepares and atomically installs the current candidate at a host safe point.
    ///
    /// Unlike [`Self::commit`], the fallible preflight returns an owned
    /// preparation artifact which is moved into the infallible commit callback.
    /// This lets reconciliation or another host subsystem validate exactly once
    /// without reconstructing a second plan after the transaction can no longer
    /// roll back safely.
    pub fn commit_prepared<P, T>(
        &mut self,
        transaction: ReloadTransactionId,
        preflight: impl FnOnce(&ReloadSnapshot<G, R>, &ReloadSnapshot<G, R>) -> Result<T, P>,
        commit: impl FnOnce(&mut ReloadSnapshot<G, R>, &mut ReloadSnapshot<G, R>, T),
    ) -> Result<ReloadCommit<E>, ReloadCommitError<P, E>> {
        let current = self.current.ok_or(ReloadCommitError::Transaction(
            ReloadTransactionError::NoTransaction,
        ))?;
        if transaction != current {
            return Err(ReloadCommitError::Transaction(
                ReloadTransactionError::Superseded {
                    supplied: transaction,
                    current,
                },
            ));
        }
        let Some((candidate_transaction, candidate)) = self.candidate.as_ref() else {
            return Err(ReloadCommitError::Transaction(
                ReloadTransactionError::CandidateNotStaged { transaction },
            ));
        };
        debug_assert_eq!(*candidate_transaction, transaction);
        let prepared = match preflight(&self.active, candidate) {
            Ok(prepared) => prepared,
            Err(error) => {
                self.candidate.take();
                self.current = None;
                return Err(ReloadCommitError::Preflight {
                    error,
                    replay: self.take_replay(),
                });
            }
        };

        let (_, mut candidate) = self.candidate.take().expect("candidate checked above");
        commit(&mut self.active, &mut candidate, prepared);
        candidate.activate();
        let generation_id = candidate.generation_id();
        let mut old = std::mem::replace(&mut self.active, candidate);
        old.retire();
        drop(old);
        self.current = None;
        Ok(ReloadCommit {
            generation_id,
            replay: self.take_replay(),
        })
    }

    /// Rejects the current candidate and releases queued events to the old snapshot.
    pub fn rollback(
        &mut self,
        transaction: ReloadTransactionId,
    ) -> Result<ReloadReplay<E>, ReloadTransactionError> {
        let current = self.current.ok_or(ReloadTransactionError::NoTransaction)?;
        if transaction != current {
            return Err(ReloadTransactionError::Superseded {
                supplied: transaction,
                current,
            });
        }
        self.candidate.take();
        self.current = None;
        Ok(self.take_replay())
    }

    /// Rejects a candidate at a named fallible boundary.
    ///
    /// This operation is valid before or after a complete candidate snapshot
    /// has been staged. Any owned candidate is retired, the active snapshot is
    /// unchanged, and the complete FIFO is returned for old-generation replay.
    pub fn reject<D>(
        &mut self,
        transaction: ReloadTransactionId,
        stage: ReloadStage,
        error: D,
    ) -> Result<ReloadRejection<D, E>, ReloadTransactionError> {
        let current = self.current.ok_or(ReloadTransactionError::NoTransaction)?;
        if transaction != current {
            return Err(ReloadTransactionError::Superseded {
                supplied: transaction,
                current,
            });
        }
        self.candidate.take();
        self.current = None;
        Ok(ReloadRejection {
            stage,
            error,
            replay: self.take_replay(),
        })
    }

    fn take_replay(&mut self) -> ReloadReplay<E> {
        ReloadReplay {
            events: std::mem::take(&mut self.queued_events),
        }
    }
}
