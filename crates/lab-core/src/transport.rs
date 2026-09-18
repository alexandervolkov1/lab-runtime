//! Bounded, single-owner byte transaction execution.
//!
//! A [`ResourceExecutor`] owns one byte adapter and serializes all logical
//! instruments sharing that resource. Calls are deliberately nonblocking attempts:
//! zero transferred bytes means Pending, while a partial write means a physical
//! operation may already have started. Software cancellation never retracts bytes.

use std::{collections::VecDeque, time::Duration};

use crate::metakon::MAX_FRAME_BYTES;
use crate::output::{DispatchId, OutputIntent};

/// Maximum ordinary transactions waiting behind one resource owner.
pub const MAX_QUEUED_TRANSACTIONS: usize = 32;
/// Largest accepted queue or execution duration in this milestone.
pub const MAX_TRANSACTION_DURATION: Duration = Duration::from_secs(60);

/// Stable local identity for one conflicting byte resource such as an RS-485 bus.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ResourceId(u64);

impl ResourceId {
    /// Construct a resource identity; Runtime registration enforces uniqueness.
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Return the local numeric value for diagnostics and adapter configuration.
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Local transaction correlation scoped to one executor instance.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TransactionId(u64);

impl TransactionId {
    /// Return the local sequence for diagnostics, not as a remote deduplication key.
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Adapter-level failure. It describes byte movement, not protocol meaning.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransportIoError {
    /// The underlying resource is no longer connected.
    Disconnected,
    /// The adapter rejected an operation for a resource-specific reason.
    Other,
}

/// Progress from one bounded recovery attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveryStatus {
    /// Recovery has not established a clean generation yet.
    Pending,
    /// Old response bytes/effects are settled according to this adapter's contract.
    Complete,
}

/// Progress of a nonblocking adapter retirement request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransportShutdown {
    /// A worker or OS handle is still retiring; callers must not join it.
    Pending,
    /// The adapter reports that its worker and handle are closed.
    Complete,
}

/// Minimal OS-independent byte boundary.
///
/// Implementations must return promptly. `Ok(0)` means no progress and is not send
/// evidence. A successful recovery must ensure bytes from the previous generation
/// cannot be delivered as a response in the new generation.
pub trait ByteTransport {
    /// Attempt to accept a prefix of `bytes` for transmission.
    fn try_write(&mut self, bytes: &[u8]) -> Result<usize, TransportIoError>;
    /// Attempt to read a prefix into the bounded destination.
    fn try_read(&mut self, bytes: &mut [u8]) -> Result<usize, TransportIoError>;
    /// Make one bounded recovery step.
    fn try_recover(&mut self) -> Result<RecoveryStatus, TransportIoError>;
    /// Fence new adapter work and make one nonblocking close-progress attempt.
    fn try_shutdown(&mut self) -> TransportShutdown {
        TransportShutdown::Complete
    }
}

/// Executor-level validation/admission failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransportError {
    /// The bounded ordinary queue is full.
    QueueFull,
    /// Frame or deadline configuration violates a hard M3 bound.
    InvalidTransaction,
    /// Poll time moved backwards.
    InvalidTime,
    /// A checked local counter cannot advance without wrapping.
    CounterExhausted,
    /// A Runtime resource identity is already registered.
    DuplicateResource,
    /// No executor owns the requested resource identity.
    UnknownResource,
    /// Runtime already owns the maximum eight resources.
    ResourceLimit,
    /// Existing executor or adapter has not reached a replaceable closed boundary.
    ResourceBusy,
    /// The resource is recovering, offline, closing, or already closed.
    ResourceUnavailable,
}

impl From<TransportError> for crate::Error {
    fn from(error: TransportError) -> Self {
        Self::Transport(error)
    }
}

/// Terminal result retained in the executor's single latest-result slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransactionOutcome {
    /// The expected number of response bytes was collected.
    Completed,
    /// Work reached its exclusive queue deadline before execution began.
    QueueExpired,
    /// Byte I/O or transaction time failed and recovery established a boundary.
    Failed,
}

/// Bounded latest transaction record; full history belongs to diagnostics/recording later.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransactionRecord {
    /// Correlation assigned at admission.
    pub id: TransactionId,
    /// Terminal executor result.
    pub outcome: TransactionOutcome,
    /// Whether at least one request byte was accepted before the result.
    pub started: bool,
    /// Resource generation in which the result became terminal.
    pub generation: u64,
    /// Trusted binding generation captured by the request.
    pub binding_generation: u64,
    /// Trusted operation-mapping revision captured by the request.
    pub mapping_revision: u64,
}

/// Coarse resource state, independent of a particular instrument's health.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecutorState {
    /// No transaction owns the resource.
    Idle,
    /// One request/response transaction owns the resource.
    InFlight,
    /// No new transaction may start until the adapter establishes a clean boundary.
    Recovering,
    /// Recovery failed; explicit replacement/rebind is required.
    Offline,
}

/// Bounded inspection view; querying it never polls an adapter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExecutorSnapshot {
    /// Current resource lifecycle.
    pub state: ExecutorState,
    /// Number of ordinary transactions waiting, at most 32.
    pub queue_len: usize,
    /// Active transaction correlation, if any.
    pub active: Option<TransactionId>,
    /// Generation incremented only after successful recovery.
    pub generation: u64,
    /// Last terminal result, replacing rather than accumulating history.
    pub latest: Option<TransactionRecord>,
}

#[derive(Clone)]
struct Transaction {
    id: TransactionId,
    request: Vec<u8>,
    expected_response: usize,
    queue_deadline: Duration,
    timeout: Duration,
    retryable: bool,
    retries: u8,
    binding_generation: u64,
    mapping_revision: u64,
    kind: TransactionKind,
}

#[derive(Clone)]
enum TransactionKind {
    Read,
    Output(Box<OutputTransaction>),
}

#[derive(Clone, Copy)]
struct OutputTransaction {
    intent: OutputIntent,
    dispatch: Option<DispatchId>,
}

struct Active {
    transaction: Transaction,
    write_offset: usize,
    response: Vec<u8>,
    execution_deadline: Duration,
}

struct Recovery {
    transaction: Transaction,
    started: bool,
    deadline: Duration,
}

enum OwnedState {
    Idle,
    Active(Active),
    Recovering(Recovery),
    ProtocolRecovery { deadline: Duration },
    Offline,
}

/// Sole owner and serializer for one conflicting byte resource.
pub struct ResourceExecutor {
    id: ResourceId,
    adapter: Box<dyn ByteTransport>,
    queue: VecDeque<Transaction>,
    safe_queue: Option<Transaction>,
    state: OwnedState,
    recovery_timeout: Duration,
    generation: u64,
    next_transaction: u64,
    last_poll: Duration,
    latest: Option<TransactionRecord>,
    latest_response: Option<Vec<u8>>,
    events: VecDeque<TransportEvent>,
    closed: bool,
}

/// Step requested from the trusted Runtime output coordinator.
pub(crate) enum AuthorizationStep {
    /// Check all current authority/binding/revision data before a zero-offset attempt.
    Validate,
    /// Record that the adapter accepted at least one byte in the same serialized call.
    Started,
}

/// One bounded executor event consumed immediately by Runtime.
pub(crate) enum TransportEvent {
    ReadUnavailable {
        id: TransactionId,
        binding_generation: u64,
        mapping_revision: u64,
    },
    ReadFenced {
        id: TransactionId,
    },
    ReadTerminal {
        record: TransactionRecord,
        response: Option<Vec<u8>>,
    },
    OutputUncertain {
        id: TransactionId,
        intent: OutputIntent,
        dispatch: DispatchId,
    },
    OutputTerminal {
        intent: OutputIntent,
        dispatch: Option<DispatchId>,
        record: TransactionRecord,
        response: Option<Vec<u8>>,
    },
    BoundaryRecovered,
    BoundaryFailed,
}

impl ResourceExecutor {
    /// Create an idle first-generation owner with the legacy maximum recovery bound.
    pub fn new(id: ResourceId, adapter: Box<dyn ByteTransport>) -> Self {
        Self::with_recovery_timeout(id, adapter, MAX_TRANSACTION_DURATION)
            .expect("the built-in recovery timeout is valid")
    }

    /// Create an owner with one checked monotonic recovery bound.
    ///
    /// The host supplies a validated deployment duration without exposing TOML,
    /// OS handles, or wall-clock time to Core.
    pub fn with_recovery_timeout(
        id: ResourceId,
        adapter: Box<dyn ByteTransport>,
        recovery_timeout: Duration,
    ) -> Result<Self, TransportError> {
        if recovery_timeout.is_zero() || recovery_timeout > MAX_TRANSACTION_DURATION {
            return Err(TransportError::InvalidTransaction);
        }
        Ok(Self {
            id,
            adapter,
            queue: VecDeque::new(),
            safe_queue: None,
            state: OwnedState::Idle,
            recovery_timeout,
            generation: 1,
            next_transaction: 1,
            last_poll: Duration::ZERO,
            latest: None,
            latest_response: None,
            events: VecDeque::with_capacity(MAX_QUEUED_TRANSACTIONS + 2),
            closed: false,
        })
    }

    /// Return this executor's stable resource identity.
    pub const fn id(&self) -> ResourceId {
        self.id
    }

    /// Return the checked monotonic recovery policy owned by this resource.
    pub const fn recovery_timeout(&self) -> Duration {
        self.recovery_timeout
    }

    /// Admit one bounded read transaction.
    ///
    /// `queue_deadline` is an absolute exclusive deadline. `timeout` begins when
    /// the transaction becomes active. Only an explicitly retryable read receives
    /// one retry, and only after successful recovery within the original deadline.
    #[allow(clippy::too_many_arguments)]
    pub fn enqueue_read(
        &mut self,
        request: &[u8],
        expected_response: usize,
        queued_at: Duration,
        queue_deadline: Duration,
        timeout: Duration,
        retryable: bool,
        binding_generation: u64,
        mapping_revision: u64,
    ) -> Result<TransactionId, TransportError> {
        if self.closed
            || matches!(
                self.state,
                OwnedState::Recovering(_)
                    | OwnedState::ProtocolRecovery { .. }
                    | OwnedState::Offline
            )
        {
            return Err(TransportError::ResourceUnavailable);
        }
        if self.queue.len() >= MAX_QUEUED_TRANSACTIONS {
            return Err(TransportError::QueueFull);
        }
        if request.is_empty()
            || request.len() > MAX_FRAME_BYTES
            || !(1..=MAX_FRAME_BYTES).contains(&expected_response)
            || queue_deadline <= queued_at
            || queue_deadline - queued_at > MAX_TRANSACTION_DURATION
            || timeout.is_zero()
            || timeout > MAX_TRANSACTION_DURATION
            || binding_generation == 0
            || mapping_revision == 0
        {
            return Err(TransportError::InvalidTransaction);
        }
        let following = self
            .next_transaction
            .checked_add(1)
            .ok_or(TransportError::CounterExhausted)?;
        let id = TransactionId(self.next_transaction);
        self.next_transaction = following;
        self.queue.push_back(Transaction {
            id,
            request: request.to_vec(),
            expected_response,
            queue_deadline,
            timeout,
            retryable,
            retries: 0,
            binding_generation,
            mapping_revision,
            kind: TransactionKind::Read,
        });
        Ok(id)
    }

    /// Make at most one write attempt and one read/recovery attempt.
    ///
    /// Equal timestamps are allowed for deterministic orchestration; backwards
    /// timestamps are rejected before adapter state changes.
    pub fn poll(&mut self, at: Duration) -> Result<(), TransportError> {
        self.poll_authorized(at, &mut |_, _| Err(()))?;
        Ok(())
    }

    pub(crate) fn enqueue_output(
        &mut self,
        request: &[u8],
        expected_response: usize,
        queued_at: Duration,
        queue_deadline: Duration,
        timeout: Duration,
        intent: OutputIntent,
    ) -> Result<TransactionId, TransportError> {
        if self.closed
            || (!intent.safe
                && matches!(
                    self.state,
                    OwnedState::Recovering(_)
                        | OwnedState::ProtocolRecovery { .. }
                        | OwnedState::Offline
                ))
        {
            return Err(TransportError::ResourceUnavailable);
        }
        if (!intent.safe && self.queue.len() >= MAX_QUEUED_TRANSACTIONS)
            || (intent.safe && self.safe_queue.is_some())
        {
            return Err(TransportError::QueueFull);
        }
        if request.is_empty()
            || request.len() > MAX_FRAME_BYTES
            || !(1..=MAX_FRAME_BYTES).contains(&expected_response)
            || queue_deadline <= queued_at
            || queue_deadline - queued_at > MAX_TRANSACTION_DURATION
            || timeout.is_zero()
            || timeout > MAX_TRANSACTION_DURATION
        {
            return Err(TransportError::InvalidTransaction);
        }
        let following = self
            .next_transaction
            .checked_add(1)
            .ok_or(TransportError::CounterExhausted)?;
        let id = TransactionId(self.next_transaction);
        self.next_transaction = following;
        let transaction = Transaction {
            id,
            request: request.to_vec(),
            expected_response,
            queue_deadline,
            timeout,
            retryable: false,
            retries: 0,
            binding_generation: intent.binding_generation,
            mapping_revision: intent.mapping_revision,
            kind: TransactionKind::Output(Box::new(OutputTransaction {
                intent,
                dispatch: None,
            })),
        };
        if intent.safe {
            self.safe_queue = Some(transaction);
        } else {
            self.queue.push_back(transaction);
        }
        Ok(id)
    }

    pub(crate) fn poll_authorized(
        &mut self,
        at: Duration,
        authorizer: &mut impl FnMut(OutputIntent, AuthorizationStep) -> Result<Option<DispatchId>, ()>,
    ) -> Result<Option<TransportEvent>, TransportError> {
        if at < self.last_poll {
            return Err(TransportError::InvalidTime);
        }
        self.last_poll = at;
        // A recovery transition can fence at most the fixed ordinary queue plus
        // its active transaction. Drain those owner events before progressing
        // another state transition so this VecDeque cannot accumulate by cycle.
        if let Some(event) = self.events.pop_front() {
            return Ok(Some(event));
        }
        let state = std::mem::replace(&mut self.state, OwnedState::Idle);
        self.state = match state {
            OwnedState::Idle => self.start_or_remain_idle(at, authorizer)?,
            OwnedState::Active(active) => self.progress_active(active, at, authorizer)?,
            OwnedState::Recovering(recovery) => self.progress_recovery(recovery, at)?,
            OwnedState::ProtocolRecovery { deadline } => {
                self.progress_protocol_recovery(deadline, at)?
            }
            OwnedState::Offline => OwnedState::Offline,
        };
        Ok(self.events.pop_front())
    }

    /// Return a bounded state copy without polling I/O.
    pub fn snapshot(&self) -> ExecutorSnapshot {
        let (state, active) = match &self.state {
            OwnedState::Idle => (ExecutorState::Idle, None),
            OwnedState::Active(active) => (ExecutorState::InFlight, Some(active.transaction.id)),
            OwnedState::Recovering(recovery) => {
                (ExecutorState::Recovering, Some(recovery.transaction.id))
            }
            OwnedState::ProtocolRecovery { .. } => (ExecutorState::Recovering, None),
            OwnedState::Offline => (ExecutorState::Offline, None),
        };
        ExecutorSnapshot {
            state,
            queue_len: self.queue.len() + usize::from(self.safe_queue.is_some()),
            active,
            generation: self.generation,
            latest: self.latest,
        }
    }

    /// Fence owned work and make one nonblocking adapter-retirement attempt.
    ///
    /// Retirement starts from every executor state. Already accepted bytes keep
    /// their honest `started` evidence; this method never waits for or joins an
    /// unfinished worker.
    pub fn try_shutdown(&mut self) -> TransportShutdown {
        self.fence_ordinary_queue();
        if let Some(transaction) = self.safe_queue.take() {
            self.fence_queued_transaction(transaction);
        }
        if self.closed {
            return TransportShutdown::Complete;
        }
        let state = std::mem::replace(&mut self.state, OwnedState::Offline);
        match state {
            OwnedState::Active(active) => self.finish(
                &active.transaction,
                TransactionOutcome::Failed,
                active.write_offset > 0,
                None,
            ),
            OwnedState::Recovering(recovery) => self.finish(
                &recovery.transaction,
                TransactionOutcome::Failed,
                recovery.started,
                None,
            ),
            OwnedState::Idle | OwnedState::ProtocolRecovery { .. } | OwnedState::Offline => {}
        }
        let status = self.adapter.try_shutdown();
        if status == TransportShutdown::Complete {
            self.closed = true;
        }
        status
    }

    pub(crate) fn take_event(&mut self) -> Option<TransportEvent> {
        self.events.pop_front()
    }

    /// Borrow the last completed response bytes; a later terminal result replaces them.
    pub fn latest_response(&self) -> Option<&[u8]> {
        self.latest_response.as_deref()
    }

    /// Require a clean adapter generation after protocol-level response rejection.
    pub(crate) fn protocol_failure(&mut self) -> Result<(), TransportError> {
        if matches!(self.state, OwnedState::Idle) {
            let deadline = self
                .last_poll
                .checked_add(self.recovery_timeout)
                .ok_or(TransportError::InvalidTransaction)?;
            self.fence_ordinary_queue();
            self.state = OwnedState::ProtocolRecovery { deadline };
        }
        Ok(())
    }

    fn start_or_remain_idle(
        &mut self,
        at: Duration,
        authorizer: &mut impl FnMut(OutputIntent, AuthorizationStep) -> Result<Option<DispatchId>, ()>,
    ) -> Result<OwnedState, TransportError> {
        let Some(transaction) = self.safe_queue.take().or_else(|| self.queue.pop_front()) else {
            return Ok(OwnedState::Idle);
        };
        if at >= transaction.queue_deadline {
            self.finish(&transaction, TransactionOutcome::QueueExpired, false, None);
            return Ok(OwnedState::Idle);
        }
        let execution_deadline = at
            .checked_add(transaction.timeout)
            .ok_or(TransportError::InvalidTransaction)?
            .min(transaction.queue_deadline);
        self.progress_active(
            Active {
                transaction,
                write_offset: 0,
                response: Vec::with_capacity(MAX_FRAME_BYTES),
                execution_deadline,
            },
            at,
            authorizer,
        )
    }

    fn progress_active(
        &mut self,
        mut active: Active,
        at: Duration,
        authorizer: &mut impl FnMut(OutputIntent, AuthorizationStep) -> Result<Option<DispatchId>, ()>,
    ) -> Result<OwnedState, TransportError> {
        if at >= active.execution_deadline {
            return self.enter_recovery(active, at);
        }

        if active.write_offset < active.transaction.request.len() {
            if active.write_offset == 0
                && let TransactionKind::Output(output) = &active.transaction.kind
                && authorizer(output.intent, AuthorizationStep::Validate).is_err()
            {
                self.finish(&active.transaction, TransactionOutcome::Failed, false, None);
                return Ok(OwnedState::Idle);
            }
            let remaining = &active.transaction.request[active.write_offset..];
            match self.adapter.try_write(remaining) {
                Ok(count) if count <= remaining.len() => {
                    if active.write_offset == 0
                        && count > 0
                        && let TransactionKind::Output(output) = &mut active.transaction.kind
                    {
                        match authorizer(output.intent, AuthorizationStep::Started) {
                            Ok(Some(id)) => output.dispatch = Some(id),
                            _ => return self.enter_recovery(active, at),
                        }
                    }
                    active.write_offset += count;
                }
                Ok(_) | Err(_) => {
                    return self.enter_recovery(active, at);
                }
            }
        }

        if active.write_offset == active.transaction.request.len() {
            let remaining = active.transaction.expected_response - active.response.len();
            let mut buffer = [0; MAX_FRAME_BYTES];
            match self.adapter.try_read(&mut buffer[..remaining]) {
                Ok(count) if count <= remaining => {
                    active.response.extend_from_slice(&buffer[..count])
                }
                Ok(_) | Err(_) => {
                    return self.enter_recovery(active, at);
                }
            }
            if active.response.len() == active.transaction.expected_response {
                self.finish(
                    &active.transaction,
                    TransactionOutcome::Completed,
                    active.write_offset > 0,
                    Some(active.response),
                );
                return Ok(OwnedState::Idle);
            }
        }
        Ok(OwnedState::Active(active))
    }

    fn enter_recovery(
        &mut self,
        active: Active,
        at: Duration,
    ) -> Result<OwnedState, TransportError> {
        let started = active.write_offset > 0;
        match &active.transaction.kind {
            TransactionKind::Read => self.events.push_back(TransportEvent::ReadUnavailable {
                id: active.transaction.id,
                binding_generation: active.transaction.binding_generation,
                mapping_revision: active.transaction.mapping_revision,
            }),
            TransactionKind::Output(output) => {
                if let Some(dispatch) = output.dispatch {
                    self.events.push_back(TransportEvent::OutputUncertain {
                        id: active.transaction.id,
                        intent: output.intent,
                        dispatch,
                    });
                }
            }
        }
        self.fence_ordinary_queue();
        let deadline = at
            .checked_add(self.recovery_timeout)
            .ok_or(TransportError::InvalidTransaction)?;
        Ok(OwnedState::Recovering(Recovery {
            started,
            transaction: active.transaction,
            deadline,
        }))
    }

    fn progress_recovery(
        &mut self,
        mut recovery: Recovery,
        at: Duration,
    ) -> Result<OwnedState, TransportError> {
        if at >= recovery.deadline {
            self.finish(
                &recovery.transaction,
                TransactionOutcome::Failed,
                recovery.started,
                None,
            );
            return Ok(OwnedState::Offline);
        }
        match self.adapter.try_recover() {
            Ok(RecoveryStatus::Pending) => Ok(OwnedState::Recovering(recovery)),
            Ok(RecoveryStatus::Complete) => {
                self.generation = self
                    .generation
                    .checked_add(1)
                    .ok_or(TransportError::CounterExhausted)?;
                if recovery.transaction.retryable
                    && recovery.transaction.retries == 0
                    && at < recovery.transaction.queue_deadline
                {
                    recovery.transaction.retries = 1;
                    self.queue.push_front(recovery.transaction);
                } else {
                    self.finish(
                        &recovery.transaction,
                        TransactionOutcome::Failed,
                        recovery.started,
                        None,
                    );
                }
                Ok(OwnedState::Idle)
            }
            Err(_) => {
                self.finish(
                    &recovery.transaction,
                    TransactionOutcome::Failed,
                    recovery.started,
                    None,
                );
                Ok(OwnedState::Offline)
            }
        }
    }

    fn progress_protocol_recovery(
        &mut self,
        deadline: Duration,
        at: Duration,
    ) -> Result<OwnedState, TransportError> {
        if at >= deadline {
            self.events.push_back(TransportEvent::BoundaryFailed);
            return Ok(OwnedState::Offline);
        }
        match self.adapter.try_recover() {
            Ok(RecoveryStatus::Pending) => Ok(OwnedState::ProtocolRecovery { deadline }),
            Ok(RecoveryStatus::Complete) => {
                self.generation = self
                    .generation
                    .checked_add(1)
                    .ok_or(TransportError::CounterExhausted)?;
                self.events.push_back(TransportEvent::BoundaryRecovered);
                Ok(OwnedState::Idle)
            }
            Err(_) => {
                self.events.push_back(TransportEvent::BoundaryFailed);
                Ok(OwnedState::Offline)
            }
        }
    }

    fn finish(
        &mut self,
        transaction: &Transaction,
        outcome: TransactionOutcome,
        started: bool,
        response: Option<Vec<u8>>,
    ) {
        self.latest = Some(TransactionRecord {
            id: transaction.id,
            outcome,
            started,
            generation: self.generation,
            binding_generation: transaction.binding_generation,
            mapping_revision: transaction.mapping_revision,
        });
        self.latest_response = response;
        let record = self.latest.expect("record was assigned above");
        self.events.push_back(match &transaction.kind {
            TransactionKind::Read => TransportEvent::ReadTerminal {
                record,
                response: self.latest_response.clone(),
            },
            TransactionKind::Output(output) => TransportEvent::OutputTerminal {
                intent: output.intent,
                dispatch: output.dispatch,
                record,
                response: self.latest_response.clone(),
            },
        });
    }

    fn fence_ordinary_queue(&mut self) {
        while let Some(transaction) = self.queue.pop_front() {
            self.fence_queued_transaction(transaction);
        }
    }

    fn fence_queued_transaction(&mut self, transaction: Transaction) {
        match transaction.kind {
            TransactionKind::Read => self
                .events
                .push_back(TransportEvent::ReadFenced { id: transaction.id }),
            TransactionKind::Output(output) => {
                self.events.push_back(TransportEvent::OutputTerminal {
                    intent: output.intent,
                    dispatch: output.dispatch,
                    record: TransactionRecord {
                        id: transaction.id,
                        outcome: TransactionOutcome::Failed,
                        started: false,
                        generation: self.generation,
                        binding_generation: transaction.binding_generation,
                        mapping_revision: transaction.mapping_revision,
                    },
                    response: None,
                });
            }
        }
    }
}
