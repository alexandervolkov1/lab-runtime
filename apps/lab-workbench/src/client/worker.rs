//! Single-owner bounded TCP/NDJSON Application client worker.

use super::{
    framing::{FrameDecoder, PendingWrite, encode_frame},
    types::{
        COMMAND_QUEUE, ClientCommand, ClientUpdate, CommandSendError, ConnectionState, EventCursor,
        HelloState, KnownAdmission, MAX_IN_FLIGHT, MutationIdentity, QuarantinedRecoveryRecord,
        RecoveryQuarantineReason, RecoveryRecord, ReplyKind, UPDATE_QUEUE,
    },
};
use crate::recovery::{RecoveryJournal, load_journal, retire_journal, save_journal};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, VecDeque},
    io::{self, Read},
    net::{SocketAddr, TcpStream},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError},
    },
    thread,
    time::{Duration, Instant},
};

pub(crate) const CONNECT_DEADLINE: Duration = Duration::from_secs(2);
pub(crate) const HELLO_DEADLINE: Duration = Duration::from_secs(2);
pub(crate) const PARTIAL_FRAME_DEADLINE: Duration = Duration::from_secs(2);
pub(crate) const BLOCKED_WRITE_DEADLINE: Duration = Duration::from_secs(2);
pub(crate) const REQUEST_DEADLINE: Duration = Duration::from_secs(5);
pub(crate) const REATTACH_DEADLINE: Duration = Duration::from_secs(3);
pub(crate) const WORKER_SHUTDOWN_DEADLINE: Duration = Duration::from_secs(3);
const RETRY_DELAY: Duration = Duration::from_millis(10);
const IDLE_POLL: Duration = Duration::from_millis(2);
const READ_TURN_BYTES: usize = 8 * 1024;
const COMMANDS_PER_TURN: usize = 8;
const BOOTSTRAP_EVENTS: usize = 64;

/// Renderer-neutral notification invoked only after an update enters the bounded queue.
pub(crate) type WakeCallback = Arc<dyn Fn() + Send + Sync + 'static>;

fn connect_timeout_for_attempt(
    now: Instant,
    reattach_deadline: Option<Instant>,
) -> Option<Duration> {
    match reattach_deadline {
        None => Some(CONNECT_DEADLINE),
        Some(deadline) => deadline
            .checked_duration_since(now)
            .filter(|remaining| !remaining.is_zero())
            .map(|remaining| remaining.min(CONNECT_DEADLINE)),
    }
}

/// Owning handle for the one client worker and its bounded mailboxes.
pub(crate) struct ClientHandle {
    command_tx: SyncSender<ClientCommand>,
    update_rx: Receiver<ClientUpdate>,
    stop: Arc<AtomicBool>,
    #[cfg(test)]
    update_overflow_observed: Arc<AtomicBool>,
    next_command: AtomicU64,
    join: Option<thread::JoinHandle<()>>,
}

#[cfg_attr(
    not(test),
    allow(dead_code, reason = "M14.2 client API precedes the M14 GUI consumer")
)]
impl ClientHandle {
    pub(crate) fn spawn(address: SocketAddr) -> io::Result<Self> {
        Self::spawn_with_recovery_journal(address, None)
    }

    /// Starts the one client owner with an optional bounded durable recovery journal.
    pub(crate) fn spawn_with_recovery_journal(
        address: SocketAddr,
        journal_path: Option<PathBuf>,
    ) -> io::Result<Self> {
        Self::spawn_with_recovery_journal_and_wake(address, journal_path, None)
    }

    /// Starts the client owner with durable recovery and an optional renderer wake hook.
    pub(crate) fn spawn_with_recovery_journal_and_wake(
        address: SocketAddr,
        journal_path: Option<PathBuf>,
        wake: Option<WakeCallback>,
    ) -> io::Result<Self> {
        if !address.ip().is_loopback() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Workbench M14 client requires a numeric loopback endpoint",
            ));
        }
        let (command_tx, command_rx) = mpsc::sync_channel(COMMAND_QUEUE);
        let (update_tx, update_rx) = mpsc::sync_channel(UPDATE_QUEUE);
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        #[cfg(test)]
        let update_overflow_observed = Arc::new(AtomicBool::new(false));
        #[cfg(test)]
        let worker_update_overflow = Arc::clone(&update_overflow_observed);
        let join = thread::Builder::new()
            .name("lab-workbench-application-client".into())
            .spawn(move || {
                Worker::new(
                    address,
                    journal_path,
                    command_rx,
                    update_tx,
                    worker_stop,
                    wake,
                    #[cfg(test)]
                    worker_update_overflow,
                )
                .run();
            })?;
        Ok(Self {
            command_tx,
            update_rx,
            stop,
            #[cfg(test)]
            update_overflow_observed,
            next_command: AtomicU64::new(1),
            join: Some(join),
        })
    }

    fn command_id(&self) -> Result<u64, CommandSendError> {
        self.next_command
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_add(1)
            })
            .map_err(|_| CommandSendError::IdExhausted)
    }

    fn submit(&self, command: ClientCommand) -> Result<(), CommandSendError> {
        match self.command_tx.try_send(command) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => Err(CommandSendError::Busy),
            Err(TrySendError::Disconnected(_)) => Err(CommandSendError::WorkerStopped),
        }
    }

    pub(crate) fn connect(&self, scope: Option<String>) -> Result<u64, CommandSendError> {
        let command_id = self.command_id()?;
        self.submit(ClientCommand::Connect { command_id, scope })?;
        Ok(command_id)
    }

    pub(crate) fn disconnect(&self) -> Result<(), CommandSendError> {
        self.submit(ClientCommand::Disconnect)?;
        Ok(())
    }

    pub(crate) fn query(&self, op: &str, args: Value) -> Result<u64, CommandSendError> {
        let command_id = self.command_id()?;
        self.submit(ClientCommand::Query {
            command_id,
            op: op.to_owned(),
            args,
        })?;
        Ok(command_id)
    }

    pub(crate) fn mutation(&self, op: &str, args: Value) -> Result<u64, CommandSendError> {
        let command_id = self.command_id()?;
        self.submit(ClientCommand::Mutation {
            command_id,
            op: op.to_owned(),
            args,
        })?;
        Ok(command_id)
    }

    pub(crate) fn retry_mutation(
        &self,
        identity: MutationIdentity,
    ) -> Result<u64, CommandSendError> {
        let command_id = self.command_id()?;
        self.submit(ClientCommand::RetryMutation {
            command_id,
            identity,
        })?;
        Ok(command_id)
    }

    pub(crate) fn operation_status(
        &self,
        identity: MutationIdentity,
    ) -> Result<u64, CommandSendError> {
        let command_id = self.command_id()?;
        self.submit(ClientCommand::OperationStatus {
            command_id,
            identity,
        })?;
        Ok(command_id)
    }

    pub(crate) fn subscribe(
        &self,
        after: EventCursor,
        filter: Value,
    ) -> Result<u64, CommandSendError> {
        let command_id = self.command_id()?;
        self.submit(ClientCommand::Subscribe {
            command_id,
            after,
            filter,
        })?;
        Ok(command_id)
    }

    pub(crate) fn unsubscribe(&self) -> Result<u64, CommandSendError> {
        let command_id = self.command_id()?;
        self.submit(ClientCommand::Unsubscribe { command_id })?;
        Ok(command_id)
    }

    pub(crate) fn bootstrap_reference(&self, reference: &str) -> Result<u64, CommandSendError> {
        let command_id = self.command_id()?;
        self.submit(ClientCommand::BootstrapReference {
            command_id,
            reference: reference.to_owned(),
        })?;
        Ok(command_id)
    }

    pub(crate) fn recv_timeout(
        &self,
        timeout: Duration,
    ) -> Result<ClientUpdate, mpsc::RecvTimeoutError> {
        self.update_rx.recv_timeout(timeout)
    }

    pub(crate) fn try_recv(&self) -> Result<ClientUpdate, TryRecvError> {
        self.update_rx.try_recv()
    }

    #[cfg(test)]
    fn update_overflow_observed(&self) -> bool {
        self.update_overflow_observed.load(Ordering::Acquire)
    }

    pub(crate) fn shutdown(mut self) -> Result<(), &'static str> {
        self.stop.store(true, Ordering::Release);
        let _ = self.command_tx.try_send(ClientCommand::ShutdownWorker);
        let deadline = Instant::now() + WORKER_SHUTDOWN_DEADLINE;
        let Some(join) = self.join.take() else {
            return Ok(());
        };
        while !join.is_finished() {
            if Instant::now() >= deadline {
                return Err("client worker did not stop within its deadline");
            }
            thread::sleep(IDLE_POLL);
        }
        join.join().map_err(|_| "client worker panicked")
    }
}

impl Drop for ClientHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = self.command_tx.try_send(ClientCommand::ShutdownWorker);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Purpose {
    Hello,
    Query,
    Mutation,
    RetryMutation,
    OperationStatus,
    Subscribe,
    Unsubscribe,
    BootstrapSubscribe,
    BootstrapReference,
}

struct PendingExchange {
    command_id: u64,
    msg_id: String,
    op: String,
    purpose: Purpose,
    mutation: Option<RecoveryRecord>,
    status_identity: Option<MutationIdentity>,
    transmitted: bool,
    deadline: Option<Instant>,
}

struct Outgoing {
    msg_id: String,
    frame: PendingWrite,
}

struct Bootstrap {
    command_id: u64,
    reference: String,
    events: Vec<Value>,
}

struct Worker {
    address: SocketAddr,
    journal_path: Option<PathBuf>,
    command_rx: Receiver<ClientCommand>,
    update_tx: SyncSender<ClientUpdate>,
    wake: Option<WakeCallback>,
    stop: Arc<AtomicBool>,
    #[cfg(test)]
    update_overflow_observed: Arc<AtomicBool>,
    state: ConnectionState,
    stream: Option<TcpStream>,
    decoder: FrameDecoder,
    outgoing: VecDeque<Outgoing>,
    pending: BTreeMap<String, PendingExchange>,
    next_msg: u64,
    hello: Option<HelloState>,
    desired_scope: Option<String>,
    reattach_until: Option<Instant>,
    retry_at: Option<Instant>,
    subscription_token: Option<String>,
    event_cursor: Option<EventCursor>,
    recovery: Vec<RecoveryRecord>,
    quarantined_recovery: Vec<QuarantinedRecoveryRecord>,
    journal_failed: bool,
    startup_journal_error: Option<String>,
    sequence_blocked: bool,
    deferred_update: Option<ClientUpdate>,
    update_overflow: bool,
    bootstrap: Option<Bootstrap>,
}

impl Worker {
    fn new(
        address: SocketAddr,
        journal_path: Option<PathBuf>,
        command_rx: Receiver<ClientCommand>,
        update_tx: SyncSender<ClientUpdate>,
        stop: Arc<AtomicBool>,
        wake: Option<WakeCallback>,
        #[cfg(test)] update_overflow_observed: Arc<AtomicBool>,
    ) -> Self {
        let (recovery, startup_journal_error) = load_startup_recovery(journal_path.as_deref());
        Self {
            address,
            journal_path,
            command_rx,
            update_tx,
            wake,
            stop,
            #[cfg(test)]
            update_overflow_observed,
            state: ConnectionState::Disconnected,
            stream: None,
            decoder: FrameDecoder::new(),
            outgoing: VecDeque::with_capacity(MAX_IN_FLIGHT),
            pending: BTreeMap::new(),
            next_msg: 1,
            hello: None,
            desired_scope: None,
            reattach_until: None,
            retry_at: None,
            subscription_token: None,
            event_cursor: None,
            recovery,
            quarantined_recovery: Vec::with_capacity(MAX_IN_FLIGHT),
            journal_failed: startup_journal_error.is_some(),
            startup_journal_error,
            sequence_blocked: false,
            deferred_update: None,
            update_overflow: false,
            bootstrap: None,
        }
    }

    fn run(mut self) {
        self.emit(ClientUpdate::State(ConnectionState::Disconnected));
        self.emit_recovery_projection();
        if let Some(reason) = self.startup_journal_error.take() {
            self.emit(ClientUpdate::RecoveryJournalProblem { reason });
        }
        while !self.stop.load(Ordering::Acquire) {
            self.flush_deferred_update();
            if self.update_overflow {
                self.close_connection("ordered update queue saturated", false);
                self.state = ConnectionState::Stale;
                self.update_overflow = false;
            }
            if self.expire_reattach_if_due(Instant::now()) {
                continue;
            }
            self.retry_reattach();
            self.service_commands();
            self.service_write();
            self.service_read();
            self.check_deadlines();
            thread::sleep(IDLE_POLL);
        }
        self.state = ConnectionState::Stopping;
        self.emit(ClientUpdate::State(ConnectionState::Stopping));
        self.close_connection("worker stopping", false);
        self.state = ConnectionState::Stopped;
        self.emit(ClientUpdate::State(ConnectionState::Stopped));
        self.emit(ClientUpdate::WorkerStopped);
    }

    fn emit(&mut self, update: ClientUpdate) {
        if self.deferred_update.is_some() {
            return;
        }
        match self.update_tx.try_send(update) {
            Ok(()) => self.wake(),
            Err(TrySendError::Full(_)) => {
                #[cfg(test)]
                self.update_overflow_observed.store(true, Ordering::Release);
                self.deferred_update = Some(ClientUpdate::ResnapshotRequired {
                    reason: "ordered_update_queue_full".into(),
                    envelope: None,
                    connection_lost: true,
                });
                self.update_overflow = true;
            }
            Err(TrySendError::Disconnected(_)) => {
                self.stop.store(true, Ordering::Release);
            }
        }
    }

    fn flush_deferred_update(&mut self) {
        let Some(update) = self.deferred_update.take() else {
            return;
        };
        match self.update_tx.try_send(update) {
            Ok(()) => self.wake(),
            Err(TrySendError::Full(update)) => self.deferred_update = Some(update),
            Err(TrySendError::Disconnected(_)) => self.stop.store(true, Ordering::Release),
        }
    }

    fn wake(&self) {
        if let Some(wake) = &self.wake {
            wake();
        }
    }

    fn transition(&mut self, state: ConnectionState) {
        self.state = state;
        self.emit(ClientUpdate::State(state));
    }

    fn service_commands(&mut self) {
        for _ in 0..COMMANDS_PER_TURN {
            match self.command_rx.try_recv() {
                Ok(ClientCommand::ShutdownWorker) => {
                    self.stop.store(true, Ordering::Release);
                    return;
                }
                Ok(command) => self.handle_command(command),
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => {
                    self.stop.store(true, Ordering::Release);
                    return;
                }
            }
        }
    }

    fn handle_command(&mut self, command: ClientCommand) {
        match command {
            ClientCommand::Connect { command_id, scope } => {
                if self.stream.is_some() || self.retry_at.is_some() {
                    self.reject(command_id, "already_connected_or_connecting");
                    return;
                }
                self.desired_scope = scope;
                self.reattach_until = self
                    .desired_scope
                    .as_ref()
                    .map(|_| Instant::now() + REATTACH_DEADLINE);
                self.open_connection(command_id);
            }
            ClientCommand::Disconnect => {
                self.close_connection("client disconnect", false);
                self.transition(ConnectionState::Disconnected);
            }
            ClientCommand::Query {
                command_id,
                op,
                args,
            } => {
                self.queue_query(command_id, op, args, Purpose::Query);
            }
            ClientCommand::Mutation {
                command_id,
                op,
                args,
            } => self.queue_mutation(command_id, op, args),
            ClientCommand::RetryMutation {
                command_id,
                identity,
            } => self.queue_retry(command_id, identity),
            ClientCommand::OperationStatus {
                command_id,
                identity,
            } => self.queue_operation_status(command_id, identity),
            ClientCommand::Subscribe {
                command_id,
                after,
                filter,
            } => {
                self.queue_subscribe(command_id, after, filter, Purpose::Subscribe);
            }
            ClientCommand::Unsubscribe { command_id } => {
                let Some(token) = self.subscription_token.clone() else {
                    self.reject(command_id, "no_active_subscription");
                    return;
                };
                self.queue_query(
                    command_id,
                    "unsubscribe".into(),
                    json!({"subscription":token}),
                    Purpose::Unsubscribe,
                );
            }
            ClientCommand::BootstrapReference {
                command_id,
                reference,
            } => self.start_reference_bootstrap(command_id, reference),
            ClientCommand::ShutdownWorker => unreachable!("handled before dispatch"),
        }
    }

    fn open_connection(&mut self, command_id: u64) {
        self.transition(if self.desired_scope.is_some() {
            ConnectionState::Reattaching
        } else {
            ConnectionState::Connecting
        });
        let now = Instant::now();
        let attempt_timeout = if self.desired_scope.is_some() {
            connect_timeout_for_attempt(now, self.reattach_until)
        } else {
            connect_timeout_for_attempt(now, None)
        };
        let Some(attempt_timeout) = attempt_timeout else {
            self.retry_at = None;
            self.reattach_until = None;
            self.emit(ClientUpdate::TransportFailure {
                reason: "reattach deadline exceeded".into(),
            });
            self.transition(ConnectionState::Disconnected);
            return;
        };
        match TcpStream::connect_timeout(&self.address, attempt_timeout) {
            Ok(stream) => {
                if let Err(error) = stream.set_nonblocking(true) {
                    self.emit(ClientUpdate::TransportFailure {
                        reason: format!("could not configure nonblocking socket: {error}"),
                    });
                    self.transition(ConnectionState::Disconnected);
                    return;
                }
                let _ = stream.set_nodelay(true);
                self.stream = Some(stream);
                self.decoder = FrameDecoder::new();
                self.next_msg = 1;
                self.transition(ConnectionState::AwaitingHello);
                let scope = self.desired_scope.clone();
                self.queue_request(
                    command_id,
                    "hello".into(),
                    json!({"scope":scope}),
                    None,
                    Purpose::Hello,
                );
            }
            Err(error) => {
                if self.should_retry_reattach() {
                    self.retry_at = Some(Instant::now() + RETRY_DELAY);
                } else {
                    self.reattach_until = None;
                    self.emit(ClientUpdate::TransportFailure {
                        reason: format!("connect failed: {error}"),
                    });
                    self.transition(ConnectionState::Disconnected);
                }
            }
        }
    }

    fn should_retry_reattach(&self) -> bool {
        self.desired_scope.is_some()
            && self
                .reattach_until
                .is_some_and(|deadline| Instant::now() < deadline)
    }

    fn retry_reattach(&mut self) {
        if self.stream.is_none()
            && self.retry_at.is_some_and(|retry| Instant::now() >= retry)
            && self.should_retry_reattach()
        {
            self.retry_at = None;
            self.open_connection(0);
        }
    }

    fn expire_reattach_if_due(&mut self, now: Instant) -> bool {
        if self.reattach_until.is_none_or(|deadline| now < deadline) {
            return false;
        }
        self.retry_at = None;
        self.reattach_until = None;
        self.close_connection("reattach deadline exceeded", true);
        self.transition(ConnectionState::Disconnected);
        true
    }

    fn queue_query(&mut self, command_id: u64, op: String, args: Value, purpose: Purpose) -> bool {
        if self.state != ConnectionState::Ready {
            self.reject(command_id, "client_not_ready");
            return false;
        }
        self.queue_request(command_id, op, args, None, purpose)
    }

    fn queue_mutation(&mut self, command_id: u64, op: String, args: Value) {
        if self.state != ConnectionState::Ready {
            self.reject(command_id, "client_not_ready");
            return;
        }
        if self.has_unresolved_quarantine() {
            self.reject(command_id, "recovery_quarantine_unresolved");
            return;
        }
        if self.sequence_blocked {
            self.reject(command_id, "mutation_reconciliation_required");
            return;
        }
        if self.journal_failed {
            self.reject(command_id, "recovery_journal_unavailable");
            return;
        }
        if self.pending.len() >= MAX_IN_FLIGHT {
            self.reject(command_id, "in_flight_limit");
            return;
        }
        let mut candidate = self.recovery.clone();
        if candidate.len() >= MAX_IN_FLIGHT {
            if let Some(resolved) = candidate.iter().position(|record| {
                matches!(
                    record.admission,
                    KnownAdmission::Completed | KnownAdmission::Failed
                )
            }) {
                candidate.remove(resolved);
            } else {
                self.reject(command_id, "recovery_capacity");
                return;
            }
        }
        let Some(hello) = self.hello.as_ref() else {
            self.reject(command_id, "hello_missing");
            return;
        };
        let identity = MutationIdentity {
            scope: hello.scope.clone(),
            seq: hello.next_seq,
        };
        let record = RecoveryRecord {
            boot_id: hello.boot_id.clone(),
            identity,
            op: op.clone(),
            args: args.clone(),
            admission: KnownAdmission::Pending,
        };
        if !upsert_record(&mut candidate, record.clone()) {
            self.reject(command_id, "recovery_capacity");
            return;
        }
        if let Err(error) = self.persist_recovery(&candidate) {
            self.journal_failed = true;
            self.emit(ClientUpdate::RecoveryJournalProblem {
                reason: format!("could not persist mutation before wire emission: {error}"),
            });
            self.reject(command_id, "recovery_journal_unavailable");
            return;
        }
        self.recovery = candidate;
        self.emit_recovery_projection();
        self.sequence_blocked = true;
        if !self.queue_request(
            command_id,
            op,
            args,
            Some(record.clone()),
            Purpose::Mutation,
        ) {
            let _ = self.try_remove_recovery(&record.identity);
            self.sequence_blocked = false;
        }
    }

    fn queue_retry(&mut self, command_id: u64, identity: MutationIdentity) {
        if self.state != ConnectionState::Ready {
            self.reject(command_id, "client_not_ready");
            return;
        }
        if self.journal_failed {
            self.reject(command_id, "recovery_journal_unavailable");
            return;
        }
        if self.has_unresolved_quarantine() {
            self.reject(command_id, "recovery_quarantine_unresolved");
            return;
        }
        let Some(hello) = self.hello.as_ref() else {
            self.reject(command_id, "hello_missing");
            return;
        };
        let Some(record) = self
            .recovery
            .iter()
            .find(|record| record.identity == identity)
            .cloned()
        else {
            self.reject(command_id, "recovery_not_found");
            return;
        };
        if record.boot_id != hello.boot_id || record.identity.scope != hello.scope {
            self.reject(command_id, "recovery_identity_not_attached");
            return;
        }
        if !matches!(
            record.admission,
            KnownAdmission::Pending | KnownAdmission::Accepted | KnownAdmission::Ambiguous
        ) {
            self.reject(command_id, "recovery_terminal");
            return;
        }
        if self.recovery_action_in_flight(&identity) {
            self.reject(command_id, "recovery_action_in_flight");
            return;
        }
        self.queue_request(
            command_id,
            record.op.clone(),
            record.args.clone(),
            Some(record),
            Purpose::RetryMutation,
        );
    }

    fn queue_operation_status(&mut self, command_id: u64, identity: MutationIdentity) {
        if self.state != ConnectionState::Ready {
            self.reject(command_id, "client_not_ready");
            return;
        }
        let Some(hello) = self.hello.as_ref() else {
            self.reject(command_id, "hello_missing");
            return;
        };
        let Some(record) = self
            .recovery
            .iter()
            .find(|record| record.identity == identity)
        else {
            self.reject(command_id, "recovery_not_found");
            return;
        };
        if record.boot_id != hello.boot_id || record.identity.scope != hello.scope {
            self.reject(command_id, "recovery_identity_not_attached");
            return;
        }
        if self.recovery_action_in_flight(&identity) {
            self.reject(command_id, "recovery_action_in_flight");
            return;
        }
        self.queue_request(
            command_id,
            "operation_status".into(),
            json!({"request_id":identity.to_json()}),
            None,
            Purpose::OperationStatus,
        );
    }

    fn recovery_action_in_flight(&self, identity: &MutationIdentity) -> bool {
        self.pending.values().any(|pending| {
            pending.status_identity.as_ref() == Some(identity)
                || pending
                    .mutation
                    .as_ref()
                    .is_some_and(|record| &record.identity == identity)
        })
    }

    fn queue_subscribe(
        &mut self,
        command_id: u64,
        after: EventCursor,
        filter: Value,
        purpose: Purpose,
    ) -> bool {
        if self.subscription_token.is_some()
            || self.pending.values().any(|pending| {
                matches!(
                    pending.purpose,
                    Purpose::Subscribe | Purpose::BootstrapSubscribe
                )
            })
        {
            self.reject(command_id, "subscription_already_active");
            return false;
        }
        let previous_cursor = self.event_cursor.clone();
        self.event_cursor = Some(after.clone());
        let queued = self.queue_query(
            command_id,
            "subscribe".into(),
            json!({"after":after.to_json(),"filter":filter}),
            purpose,
        );
        if !queued {
            self.event_cursor = previous_cursor;
        }
        queued
    }

    fn start_reference_bootstrap(&mut self, command_id: u64, reference: String) {
        if self.state != ConnectionState::Ready {
            self.reject(command_id, "client_not_ready");
            return;
        }
        if self.bootstrap.is_some() {
            self.reject(command_id, "bootstrap_already_active");
            return;
        }
        let Some(hello) = self.hello.as_ref() else {
            self.reject(command_id, "hello_missing");
            return;
        };
        let after = hello.event_latest.clone();
        if !self.queue_subscribe(
            command_id,
            after,
            json!({"kinds":["reference"],"targets":[]}),
            Purpose::BootstrapSubscribe,
        ) {
            return;
        }
        self.bootstrap = Some(Bootstrap {
            command_id,
            reference,
            events: Vec::with_capacity(BOOTSTRAP_EVENTS),
        });
    }

    fn queue_request(
        &mut self,
        command_id: u64,
        op: String,
        args: Value,
        mutation: Option<RecoveryRecord>,
        purpose: Purpose,
    ) -> bool {
        if self.stream.is_none() {
            self.reject(command_id, "not_connected");
            return false;
        }
        if self.pending.len() >= MAX_IN_FLIGHT {
            self.reject(command_id, "in_flight_limit");
            return false;
        }
        let Some(next) = self.next_msg.checked_add(1) else {
            self.reject(command_id, "msg_id_exhausted");
            self.close_connection("msg_id exhausted", true);
            return false;
        };
        let msg_id = self.next_msg.to_string();
        self.next_msg = next;
        let status_identity = if purpose == Purpose::OperationStatus {
            args.get("request_id").and_then(parse_identity_value)
        } else {
            None
        };
        let mut request = json!({"v":1,"msg_id":msg_id,"op":op,"args":args});
        if let Some(record) = mutation.as_ref() {
            request["request_id"] = record.identity.to_json();
        }
        let frame = match encode_frame(&request) {
            Ok(frame) => frame,
            Err(error) => {
                self.reject(command_id, &format!("request_encoding_failed:{error}"));
                return false;
            }
        };
        self.pending.insert(
            msg_id.clone(),
            PendingExchange {
                command_id,
                msg_id: msg_id.clone(),
                op,
                purpose,
                mutation,
                status_identity,
                transmitted: false,
                deadline: None,
            },
        );
        self.outgoing.push_back(Outgoing {
            msg_id,
            frame: PendingWrite::new(frame),
        });
        true
    }

    fn service_write(&mut self) {
        let Some(stream) = self.stream.as_mut() else {
            return;
        };
        let Some(outgoing) = self.outgoing.front_mut() else {
            return;
        };
        let now = Instant::now();
        match outgoing.frame.advance(stream, now) {
            Ok(done) => {
                if outgoing.frame.wrote_any()
                    && let Some(pending) = self.pending.get_mut(&outgoing.msg_id)
                {
                    pending.transmitted = true;
                }
                if done {
                    let outgoing = self.outgoing.pop_front().expect("front exists");
                    if let Some(pending) = self.pending.get_mut(&outgoing.msg_id) {
                        pending.deadline = Some(
                            now + if pending.purpose == Purpose::Hello {
                                HELLO_DEADLINE
                            } else {
                                REQUEST_DEADLINE
                            },
                        );
                    }
                } else if outgoing.frame.timed_out(now, BLOCKED_WRITE_DEADLINE) {
                    self.fail_transport("blocked output deadline exceeded");
                }
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
            Err(error) => self.fail_transport(&format!("socket write failed: {error}")),
        }
    }

    fn service_read(&mut self) {
        let Some(stream) = self.stream.as_mut() else {
            return;
        };
        let mut bytes = [0u8; READ_TURN_BYTES];
        match stream.read(&mut bytes) {
            Ok(0) => self.fail_transport("server closed the connection"),
            Ok(count) => match self.decoder.push(&bytes[..count], Instant::now()) {
                Ok(values) => {
                    for value in values {
                        if self.stream.is_none() {
                            break;
                        }
                        self.handle_incoming(value);
                    }
                }
                Err(error) => self.fail_transport(&format!("invalid server frame: {error}")),
            },
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => self.fail_transport(&format!("socket read failed: {error}")),
        }
    }

    fn check_deadlines(&mut self) {
        let now = Instant::now();
        if self
            .decoder
            .check_deadline(now, PARTIAL_FRAME_DEADLINE)
            .is_err()
        {
            self.fail_transport("partial input frame deadline exceeded");
            return;
        }
        if self
            .pending
            .values()
            .any(|pending| pending.deadline.is_some_and(|deadline| now >= deadline))
        {
            self.fail_transport("request/reply deadline exceeded");
        }
    }

    fn handle_incoming(&mut self, value: Value) {
        match value.get("type").and_then(Value::as_str) {
            Some("event") => self.handle_event(value),
            Some("subscription_progress") => self.handle_subscription_progress(value),
            Some("result") | Some("error") | Some("operation") => {
                let Some(msg_id) = value.get("msg_id").and_then(Value::as_str) else {
                    if value.get("type").and_then(Value::as_str) == Some("error")
                        && value.get("code").and_then(Value::as_str) == Some("event_gap")
                    {
                        self.subscription_token = None;
                        self.bootstrap = None;
                        self.emit(ClientUpdate::ResnapshotRequired {
                            reason: "event_gap".into(),
                            envelope: Some(value),
                            connection_lost: false,
                        });
                        return;
                    }
                    self.fail_transport("server response omitted msg_id");
                    return;
                };
                let msg_id = msg_id.to_owned();
                if !self.pending.contains_key(&msg_id) {
                    self.fail_transport("server used an unknown or terminal msg_id");
                    return;
                }
                match value.get("type").and_then(Value::as_str) {
                    Some("operation") => self.handle_operation(msg_id, value),
                    Some("result") => self.handle_result(msg_id, value),
                    Some("error") => self.handle_error(msg_id, value),
                    _ => unreachable!(),
                }
            }
            _ => self.fail_transport("server emitted an unknown envelope type"),
        }
    }

    fn handle_event(&mut self, value: Value) {
        let Some(cursor) = parse_event_cursor(&value) else {
            self.fail_transport("event cursor was malformed");
            return;
        };
        if let Some(previous) = self.event_cursor.as_ref()
            && (previous.boot_id != cursor.boot_id || cursor.seq <= previous.seq)
        {
            self.fail_transport("event cursor was not strictly ordered");
            return;
        }
        self.event_cursor = Some(cursor.clone());
        if let Some(bootstrap) = self.bootstrap.as_mut()
            && value.get("kind").and_then(Value::as_str) == Some("reference")
            && value
                .pointer("/target/id")
                .and_then(Value::as_str)
                .is_some_and(|id| id == bootstrap.reference)
        {
            if bootstrap.events.len() == BOOTSTRAP_EVENTS {
                self.emit(ClientUpdate::ResnapshotRequired {
                    reason: "reference_bootstrap_event_buffer_full".into(),
                    envelope: None,
                    connection_lost: false,
                });
                self.bootstrap = None;
                return;
            }
            bootstrap.events.push(value);
            return;
        }
        self.emit(ClientUpdate::Event {
            cursor,
            envelope: value,
        });
    }

    fn handle_subscription_progress(&mut self, value: Value) {
        let Some(cursor) = parse_event_cursor(&value) else {
            self.fail_transport("subscription progress cursor was malformed");
            return;
        };
        if let Some(previous) = self.event_cursor.as_ref()
            && (previous.boot_id != cursor.boot_id || cursor.seq <= previous.seq)
        {
            self.fail_transport("subscription progress cursor was not strictly ordered");
            return;
        }
        self.event_cursor = Some(cursor);
        self.emit(ClientUpdate::SubscriptionProgress(value));
    }

    fn handle_result(&mut self, msg_id: String, value: Value) {
        let pending = self.pending.remove(&msg_id).expect("checked pending");
        match pending.purpose {
            Purpose::Hello => self.handle_hello_result(value),
            Purpose::Subscribe | Purpose::BootstrapSubscribe => {
                let Some(token) = value
                    .pointer("/result/subscription")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                else {
                    self.fail_transport("subscribe result omitted its token");
                    return;
                };
                self.subscription_token = Some(token);
                self.emit_reply(&pending, ReplyKind::Result, value.clone(), None);
                if pending.purpose == Purpose::BootstrapSubscribe {
                    let Some(bootstrap) = self.bootstrap.as_ref() else {
                        self.fail_transport("reference bootstrap state was lost");
                        return;
                    };
                    self.queue_query(
                        bootstrap.command_id,
                        "reference".into(),
                        json!({"reference":bootstrap.reference}),
                        Purpose::BootstrapReference,
                    );
                }
            }
            Purpose::Unsubscribe => {
                self.subscription_token = None;
                self.emit_reply(&pending, ReplyKind::Result, value, None);
            }
            Purpose::BootstrapReference => self.finish_reference_bootstrap(pending, value),
            Purpose::OperationStatus => self.handle_operation_status_result(pending, value),
            _ => self.emit_reply(&pending, ReplyKind::Result, value, None),
        }
    }

    fn handle_hello_result(&mut self, value: Value) {
        if self.expire_reattach_if_due(Instant::now()) {
            return;
        }
        let Some(hello) = parse_hello(&value) else {
            self.fail_transport("hello result was malformed");
            return;
        };
        let quarantine_reason = self.recovery.iter().find_map(|record| {
            if record.boot_id != hello.boot_id {
                Some(RecoveryQuarantineReason::AttachedBootMismatch)
            } else if record.identity.scope != hello.scope {
                Some(RecoveryQuarantineReason::AttachedScopeMismatch)
            } else {
                None
            }
        });
        self.desired_scope = Some(hello.scope.clone());
        self.retry_at = None;
        self.reattach_until = None;
        if let Some(reason) = quarantine_reason
            && !self.classify_active_recovery(reason)
        {
            self.note_journal_failure(
                "bounded recovery quarantine classification failed; no mutation was sent".into(),
            );
        }
        for record in &mut self.recovery {
            if record.boot_id == hello.boot_id
                && record.identity.scope == hello.scope
                && record.identity.seq < hello.next_seq
                && matches!(
                    record.admission,
                    KnownAdmission::Pending | KnownAdmission::Ambiguous
                )
            {
                record.admission = KnownAdmission::Accepted;
            }
        }
        self.sequence_blocked = self.has_unresolved_quarantine()
            || self.recovery.iter().any(|record| {
                record.boot_id == hello.boot_id
                    && record.identity.scope == hello.scope
                    && record.identity.seq >= hello.next_seq
                    && !matches!(
                        record.admission,
                        KnownAdmission::Completed | KnownAdmission::Failed
                    )
            });
        self.hello = Some(hello.clone());
        if quarantine_reason.is_none()
            && self.quarantined_recovery.is_empty()
            && !self.journal_failed
            && let Err(error) = self.persist_recovery(&self.recovery)
        {
            self.note_journal_failure(format!(
                "could not persist authoritative hello reconciliation: {error}"
            ));
        }
        self.transition(ConnectionState::Ready);
        self.emit(ClientUpdate::Hello(hello));
        self.emit_recovery_projection();
        if !self.recovery.is_empty() {
            self.emit(ClientUpdate::ReconciliationRequired {
                records: self.recovery.clone(),
            });
        }
    }

    fn handle_error(&mut self, msg_id: String, value: Value) {
        let pending = self.pending.remove(&msg_id).expect("checked pending");
        let code = value
            .get("code")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        if pending.purpose == Purpose::Hello
            && code == "scope_in_use"
            && self.should_retry_reattach()
        {
            self.reset_transport_only();
            self.retry_at = Some(Instant::now() + RETRY_DELAY);
            self.transition(ConnectionState::Reattaching);
            return;
        }
        if pending.purpose == Purpose::Hello {
            self.reattach_until = None;
            if matches!(code, "instance_changed" | "scope_unknown") {
                let reason = if code == "instance_changed" {
                    RecoveryQuarantineReason::InstanceChanged
                } else {
                    RecoveryQuarantineReason::ScopeUnknown
                };
                if !self.classify_active_recovery(reason) {
                    self.note_journal_failure(
                        "bounded recovery quarantine classification failed; no mutation was sent"
                            .into(),
                    );
                }
                self.emit_recovery_projection();
                self.sequence_blocked = self.has_unresolved_quarantine();
                self.desired_scope = None;
            }
            self.emit_reply(&pending, ReplyKind::PublicError, value, None);
            self.reset_transport_only();
            self.transition(ConnectionState::Disconnected);
            return;
        }
        if code == "event_gap"
            && matches!(
                pending.purpose,
                Purpose::Subscribe | Purpose::BootstrapSubscribe
            )
        {
            self.bootstrap = None;
            self.emit(ClientUpdate::ResnapshotRequired {
                reason: "event_gap".into(),
                envelope: Some(value.clone()),
                connection_lost: false,
            });
        }
        if matches!(
            pending.purpose,
            Purpose::BootstrapSubscribe | Purpose::BootstrapReference
        ) {
            self.bootstrap = None;
        }
        if pending.purpose == Purpose::Mutation
            && let Some(record) = pending.mutation.as_ref()
            && record.admission == KnownAdmission::Pending
        {
            let _ = self.try_remove_recovery(&record.identity);
            self.sequence_blocked = false;
        }
        self.emit_reply(
            &pending,
            ReplyKind::PublicError,
            value,
            pending.mutation.clone(),
        );
    }

    fn handle_operation(&mut self, msg_id: String, value: Value) {
        let state = value.get("state").and_then(Value::as_str);
        match state {
            Some("accepted") => {
                let (identity, recovery) = {
                    let pending = self.pending.get_mut(&msg_id).expect("checked pending");
                    let Some(record) = pending.mutation.as_mut() else {
                        self.fail_transport("operation response had no mutation identity");
                        return;
                    };
                    record.admission = KnownAdmission::Accepted;
                    pending.deadline = Some(Instant::now() + REQUEST_DEADLINE);
                    (record.identity.clone(), record.clone())
                };
                if !self.try_upsert_recovery(recovery.clone()) {
                    self.fail_transport("recovery capacity invariant violated");
                    return;
                }
                if let Some(hello) = self.hello.as_mut()
                    && identity.seq == hello.next_seq
                {
                    let Some(next) = hello.next_seq.checked_add(1) else {
                        self.fail_transport("mutation sequence exhausted");
                        return;
                    };
                    hello.next_seq = next;
                }
                self.sequence_blocked = false;
                let pending = self.pending.get(&msg_id).expect("pending retained");
                let update = ClientUpdate::Reply {
                    command_id: pending.command_id,
                    msg_id: pending.msg_id.clone(),
                    op: pending.op.clone(),
                    kind: ReplyKind::MutationAccepted,
                    envelope: value,
                    recovery: Some(recovery),
                };
                self.emit(update);
            }
            Some("completed") | Some("failed") => {
                let pending = self.pending.remove(&msg_id).expect("checked pending");
                let mut recovery = pending.mutation.clone();
                if let Some(record) = recovery.as_mut() {
                    record.admission = if state == Some("completed") {
                        KnownAdmission::Completed
                    } else {
                        KnownAdmission::Failed
                    };
                    if !self.try_upsert_recovery(record.clone()) {
                        self.fail_transport("recovery capacity invariant violated");
                        return;
                    }
                    if let Some(hello) = self.hello.as_mut()
                        && record.identity.seq == hello.next_seq
                    {
                        let Some(next) = hello.next_seq.checked_add(1) else {
                            self.fail_transport("mutation sequence exhausted");
                            return;
                        };
                        hello.next_seq = next;
                    }
                }
                self.sequence_blocked = false;
                self.emit_reply(
                    &pending,
                    if state == Some("completed") {
                        ReplyKind::MutationCompleted
                    } else {
                        ReplyKind::MutationFailed
                    },
                    value,
                    recovery,
                );
            }
            _ => self.fail_transport("operation response had an unknown state"),
        }
    }

    fn finish_reference_bootstrap(&mut self, pending: PendingExchange, value: Value) {
        let Some(bootstrap) = self.bootstrap.take() else {
            self.fail_transport("reference bootstrap state was lost");
            return;
        };
        let Some(snapshot) = value.get("result").cloned() else {
            self.fail_transport("reference bootstrap query omitted result");
            return;
        };
        let Some(revision) = snapshot
            .get("revision")
            .and_then(Value::as_str)
            .and_then(parse_decimal)
        else {
            self.fail_transport("reference bootstrap snapshot omitted revision");
            return;
        };
        let subsequent_events = bootstrap
            .events
            .into_iter()
            .filter(|event| {
                event
                    .pointer("/data/revision")
                    .and_then(Value::as_str)
                    .and_then(parse_decimal)
                    .is_some_and(|event_revision| event_revision > revision)
            })
            .collect();
        self.emit(ClientUpdate::ReferenceBootstrap {
            command_id: pending.command_id,
            snapshot,
            subsequent_events,
        });
    }

    fn handle_operation_status_result(&mut self, pending: PendingExchange, value: Value) {
        let state = value.pointer("/result/state").and_then(Value::as_str);
        let Some(identity) = pending.status_identity.clone() else {
            self.fail_transport("operation_status correlation identity was lost");
            return;
        };
        let mut updated = self
            .recovery
            .iter()
            .find(|record| record.identity == identity)
            .cloned();
        if let Some(record) = updated.as_mut() {
            match state {
                Some("accepted") => {
                    record.admission = KnownAdmission::Accepted;
                }
                Some("completed") => {
                    record.admission = KnownAdmission::Completed;
                }
                Some("failed") => {
                    record.admission = KnownAdmission::Failed;
                }
                Some("outcome_unknown") | None | Some(_) => {}
            }
            if matches!(state, Some("accepted" | "completed" | "failed")) {
                if let Some(hello) = self.hello.as_mut()
                    && identity.seq >= hello.next_seq
                {
                    let Some(next) = identity.seq.checked_add(1) else {
                        self.fail_transport("mutation sequence exhausted");
                        return;
                    };
                    hello.next_seq = next;
                }
                self.sequence_blocked = false;
            }
        }
        if let Some(record) = updated
            && matches!(state, Some("accepted" | "completed" | "failed"))
            && !self.try_upsert_recovery(record)
        {
            self.fail_transport("recovery capacity invariant violated");
            return;
        }
        self.emit_reply(&pending, ReplyKind::Result, value, None);
    }

    fn emit_reply(
        &mut self,
        pending: &PendingExchange,
        kind: ReplyKind,
        envelope: Value,
        recovery: Option<RecoveryRecord>,
    ) {
        self.emit(ClientUpdate::Reply {
            command_id: pending.command_id,
            msg_id: pending.msg_id.clone(),
            op: pending.op.clone(),
            kind,
            envelope,
            recovery,
        });
    }

    fn reject(&mut self, command_id: u64, reason: &str) {
        self.emit(ClientUpdate::LocalRejected {
            command_id,
            reason: reason.into(),
        });
    }

    fn fail_transport(&mut self, reason: &str) {
        self.close_connection(reason, true);
        self.transition(ConnectionState::Disconnected);
    }

    fn close_connection(&mut self, reason: &str, report_failure: bool) {
        let mut records = Vec::new();
        for pending in self.pending.values_mut() {
            if let Some(record) = pending.mutation.as_mut()
                && !matches!(
                    record.admission,
                    KnownAdmission::Completed | KnownAdmission::Failed
                )
                && (pending.transmitted || record.admission == KnownAdmission::Accepted)
            {
                if record.admission == KnownAdmission::Pending {
                    record.admission = KnownAdmission::Ambiguous;
                }
                if records.len() < MAX_IN_FLIGHT {
                    records.push(record.clone());
                }
            }
        }
        let mut recovery_invariant_failed = false;
        for record in records {
            recovery_invariant_failed |= !self.try_upsert_recovery(record);
        }
        self.sequence_blocked = self.recovery.iter().any(|record| {
            matches!(
                record.admission,
                KnownAdmission::Pending | KnownAdmission::Ambiguous
            )
        });
        self.reset_transport_only();
        if recovery_invariant_failed {
            self.sequence_blocked = true;
            self.emit(ClientUpdate::TransportFailure {
                reason: "recovery capacity invariant violated".into(),
            });
        }
        if report_failure {
            self.emit(ClientUpdate::TransportFailure {
                reason: reason.into(),
            });
        }
        if !self.recovery.is_empty() {
            self.emit(ClientUpdate::ReconciliationRequired {
                records: self.recovery.clone(),
            });
        }
    }

    fn reset_transport_only(&mut self) {
        self.stream.take();
        self.decoder = FrameDecoder::new();
        self.outgoing.clear();
        self.pending.clear();
        self.subscription_token = None;
        self.bootstrap = None;
        self.next_msg = 1;
    }

    fn try_upsert_recovery(&mut self, record: RecoveryRecord) -> bool {
        let mut candidate = self.recovery.clone();
        if !upsert_record(&mut candidate, record) {
            return false;
        }
        let changed = candidate != self.recovery;
        if let Err(error) = self.persist_recovery(&candidate) {
            // The pre-wire Pending record already retains exact payload. Keep the
            // fresher in-memory state, block new mutations, and surface the
            // durability uncertainty without affecting Runtime progress.
            self.recovery = candidate;
            if changed {
                self.emit_recovery_projection();
            }
            self.note_journal_failure(format!("could not update recovery journal: {error}"));
            return true;
        }
        self.recovery = candidate;
        if changed {
            self.emit_recovery_projection();
        }
        true
    }

    fn try_remove_recovery(&mut self, identity: &MutationIdentity) -> bool {
        let mut candidate = self.recovery.clone();
        candidate.retain(|record| &record.identity != identity);
        if candidate.len() == self.recovery.len() {
            return true;
        }
        if let Err(error) = self.persist_recovery(&candidate) {
            self.note_journal_failure(format!("could not retire recovery record: {error}"));
            return false;
        }
        self.recovery = candidate;
        self.emit_recovery_projection();
        true
    }

    fn emit_recovery_projection(&mut self) {
        debug_assert!(self.recovery.len() <= MAX_IN_FLIGHT);
        debug_assert!(self.quarantined_recovery.len() <= MAX_IN_FLIGHT);
        self.emit(ClientUpdate::RecoveryProjection {
            active: self.recovery.clone(),
            quarantined: self.quarantined_recovery.clone(),
        });
    }

    fn classify_active_recovery(&mut self, reason: RecoveryQuarantineReason) -> bool {
        let mut candidate = self.quarantined_recovery.clone();
        for record in &self.recovery {
            if let Some(existing) = candidate.iter().find(|existing| {
                existing.record.boot_id == record.boot_id
                    && existing.record.identity == record.identity
            }) {
                if existing.record != *record {
                    return false;
                }
                continue;
            }
            if candidate.len() == MAX_IN_FLIGHT {
                return false;
            }
            candidate.push(QuarantinedRecoveryRecord {
                record: record.clone(),
                reason,
            });
        }
        self.quarantined_recovery = candidate;
        self.recovery.clear();
        self.sequence_blocked = self.has_unresolved_quarantine();
        true
    }

    fn has_unresolved_quarantine(&self) -> bool {
        !self.quarantined_recovery.is_empty()
    }

    fn persist_recovery(&self, records: &[RecoveryRecord]) -> Result<(), String> {
        let Some(path) = self.journal_path.as_deref() else {
            return Ok(());
        };
        if records.is_empty() {
            return retire_journal(path).map_err(|error| error.to_string());
        }
        let Some(hello) = self.hello.as_ref() else {
            // Before hello a loaded journal remains untouched and cannot be
            // treated as retry authority.
            return Ok(());
        };
        let journal = RecoveryJournal::from_records(
            hello.boot_id.clone(),
            hello.scope.clone(),
            hello.next_seq,
            records,
        )
        .map_err(|error| error.to_string())?;
        save_journal(path, &journal).map_err(|error| error.to_string())
    }

    fn note_journal_failure(&mut self, reason: String) {
        self.journal_failed = true;
        self.sequence_blocked = true;
        self.emit(ClientUpdate::RecoveryJournalProblem { reason });
    }
}

fn upsert_record(records: &mut Vec<RecoveryRecord>, record: RecoveryRecord) -> bool {
    if let Some(existing) = records
        .iter_mut()
        .find(|existing| existing.identity == record.identity)
    {
        *existing = record;
        true
    } else if records.len() < MAX_IN_FLIGHT {
        records.push(record);
        true
    } else {
        false
    }
}

fn load_startup_recovery(path: Option<&Path>) -> (Vec<RecoveryRecord>, Option<String>) {
    let Some(path) = path else {
        return (Vec::with_capacity(MAX_IN_FLIGHT), None);
    };
    if !path.exists() {
        return (Vec::with_capacity(MAX_IN_FLIGHT), None);
    }
    match load_journal(path) {
        Ok(journal) => (journal.to_recovery_records(), None),
        Err(error) => (
            Vec::with_capacity(MAX_IN_FLIGHT),
            Some(format!("recovery journal is unusable: {error}")),
        ),
    }
}

fn parse_decimal(value: &str) -> Option<u64> {
    if value.is_empty()
        || value.len() > 20
        || (value.len() > 1 && value.starts_with('0'))
        || !value.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    value.parse().ok()
}

fn parse_event_cursor(value: &Value) -> Option<EventCursor> {
    Some(EventCursor {
        boot_id: value.get("boot_id")?.as_str()?.to_owned(),
        seq: parse_decimal(value.get("seq")?.as_str()?)?,
    })
}

fn parse_identity_value(value: &Value) -> Option<MutationIdentity> {
    Some(MutationIdentity {
        scope: value.get("scope")?.as_str()?.to_owned(),
        seq: parse_decimal(value.get("seq")?.as_str()?)?,
    })
}

fn cursor_from(value: &Value, name: &str) -> Option<EventCursor> {
    let cursor = value.get(name)?;
    Some(EventCursor {
        boot_id: cursor.get("boot_id")?.as_str()?.to_owned(),
        seq: parse_decimal(cursor.get("seq")?.as_str()?)?,
    })
}

fn parse_hello(envelope: &Value) -> Option<HelloState> {
    let result = envelope.get("result")?;
    let operations = result
        .get("operations")?
        .as_array()?
        .iter()
        .map(|operation| operation.as_str().map(str::to_owned))
        .collect::<Option<Vec<_>>>()?;
    Some(HelloState {
        boot_id: result.get("boot_id")?.as_str()?.to_owned(),
        scope: result.get("scope")?.as_str()?.to_owned(),
        next_seq: parse_decimal(result.get("next_seq")?.as_str()?)?,
        operations,
        capabilities: result.get("capabilities")?.clone(),
        limits: result.get("limits")?.clone(),
        event_oldest: cursor_from(result, "event_oldest")?,
        event_latest: cursor_from(result, "event_latest")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{model::WorkbenchModel, presentation::PresentationDocument};
    use std::{
        fs,
        io::{BufRead, BufReader, Write},
        net::TcpListener,
        path::PathBuf,
        sync::atomic::{
            AtomicU64 as TestAtomicU64, AtomicUsize as TestAtomicUsize, Ordering as TestOrdering,
        },
        sync::mpsc,
    };

    const TEST_TIMEOUT: Duration = Duration::from_secs(3);
    static TEST_FILE_ID: TestAtomicU64 = TestAtomicU64::new(1);

    fn journal_path(name: &str) -> PathBuf {
        let id = TEST_FILE_ID.fetch_add(1, TestOrdering::Relaxed);
        let directory = std::env::temp_dir().join(format!(
            "lab-workbench-worker-journal-{name}-{}-{id}",
            std::process::id()
        ));
        fs::create_dir_all(&directory).unwrap();
        directory.join("recovery.json")
    }

    fn scripted_peer(
        script: impl FnOnce(TcpStream) + Send + 'static,
    ) -> (SocketAddr, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let join = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(TEST_TIMEOUT)).unwrap();
            stream.set_write_timeout(Some(TEST_TIMEOUT)).unwrap();
            script(stream);
        });
        (address, join)
    }

    fn read_request(reader: &mut BufReader<TcpStream>) -> Value {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        assert!(!line.is_empty());
        serde_json::from_str(&line).unwrap()
    }

    fn write_value(stream: &mut TcpStream, value: &Value) {
        let bytes = encode_frame(value).unwrap();
        for chunk in bytes.chunks(3) {
            stream.write_all(chunk).unwrap();
        }
    }

    fn hello_reply(msg_id: &Value, scope: &str, next_seq: u64) -> Value {
        hello_reply_for(msg_id, "boot", scope, next_seq)
    }

    fn hello_reply_for(msg_id: &Value, boot: &str, scope: &str, next_seq: u64) -> Value {
        json!({"v":1,"msg_id":msg_id,"type":"result","result":{
            "boot_id":boot,"scope":scope,"next_seq":next_seq.to_string(),
            "operations":["hello","reference","reference_retune","operation_status","subscribe","unsubscribe"],
            "capabilities":[],"limits":{"client_pending_requests":8},
            "event_oldest":{"boot_id":boot,"seq":"0"},
            "event_latest":{"boot_id":boot,"seq":"0"}
        }})
    }

    fn wait_for(client: &ClientHandle, predicate: impl Fn(&ClientUpdate) -> bool) -> ClientUpdate {
        let deadline = Instant::now() + TEST_TIMEOUT;
        loop {
            let update = client
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("client update before deadline");
            if predicate(&update) {
                return update;
            }
        }
    }

    fn recovery_record(seq: u64, admission: KnownAdmission) -> RecoveryRecord {
        RecoveryRecord {
            boot_id: "boot".into(),
            identity: MutationIdentity {
                scope: "scope".into(),
                seq,
            },
            op: "reference_retune".into(),
            args: json!({"reference":"1","expected_revision":seq.to_string(),
                "target":seq as f64,"rate":1.0}),
            admission,
        }
    }

    fn hello_state(next_seq: u64) -> HelloState {
        HelloState {
            boot_id: "boot".into(),
            scope: "scope".into(),
            next_seq,
            operations: vec!["operation_status".into(), "reference_retune".into()],
            capabilities: json!([]),
            limits: json!({}),
            event_oldest: EventCursor {
                boot_id: "boot".into(),
                seq: 0,
            },
            event_latest: EventCursor {
                boot_id: "boot".into(),
                seq: 0,
            },
        }
    }

    fn unit_worker() -> (Worker, Receiver<ClientUpdate>) {
        let (_command_tx, command_rx) = mpsc::sync_channel(COMMAND_QUEUE);
        let (update_tx, update_rx) = mpsc::sync_channel(UPDATE_QUEUE);
        let worker = Worker::new(
            "127.0.0.1:1".parse().unwrap(),
            None,
            command_rx,
            update_tx,
            Arc::new(AtomicBool::new(false)),
            None,
            Arc::new(AtomicBool::new(false)),
        );
        (worker, update_rx)
    }

    fn unit_worker_with_journal(path: PathBuf) -> (Worker, Receiver<ClientUpdate>) {
        let (_command_tx, command_rx) = mpsc::sync_channel(COMMAND_QUEUE);
        let (update_tx, update_rx) = mpsc::sync_channel(UPDATE_QUEUE);
        let worker = Worker::new(
            "127.0.0.1:1".parse().unwrap(),
            Some(path),
            command_rx,
            update_tx,
            Arc::new(AtomicBool::new(false)),
            None,
            Arc::new(AtomicBool::new(false)),
        );
        (worker, update_rx)
    }

    fn pending_hello(command_id: u64) -> PendingExchange {
        PendingExchange {
            command_id,
            msg_id: command_id.to_string(),
            op: "hello".into(),
            purpose: Purpose::Hello,
            mutation: None,
            status_identity: None,
            transmitted: true,
            deadline: None,
        }
    }

    fn save_test_journal(path: &Path, records: &[RecoveryRecord]) {
        let journal =
            RecoveryJournal::from_records("boot".into(), "scope".into(), 1, records).unwrap();
        save_journal(path, &journal).unwrap();
    }

    fn assert_recovery_evidence_visible(model: &WorkbenchModel, record: &RecoveryRecord) {
        let active = model
            .recovery
            .mutations
            .iter()
            .any(|candidate| candidate == record);
        let quarantined = model
            .recovery
            .quarantined
            .iter()
            .any(|candidate| candidate.record == *record);
        assert!(
            active || quarantined || model.recovery_problem.is_some(),
            "ordered update prefix falsely hid unresolved recovery evidence"
        );
    }

    #[test]
    fn matching_hello_keeps_loaded_recovery_active_and_quarantine_empty() {
        let path = journal_path("matching-active");
        let record = recovery_record(1, KnownAdmission::Ambiguous);
        save_test_journal(&path, std::slice::from_ref(&record));
        let (mut worker, updates) = unit_worker_with_journal(path.clone());

        worker.handle_hello_result(hello_reply_for(&json!("1"), "boot", "scope", 1));

        assert_eq!(worker.recovery, vec![record]);
        assert!(worker.quarantined_recovery.is_empty());
        assert!(!worker.journal_failed);
        assert!(updates.try_iter().any(|update| matches!(
            update,
            ClientUpdate::RecoveryProjection { active, quarantined }
                if active.len() == 1 && quarantined.is_empty()
        )));
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn invalid_hello_quarantines_exact_records_without_changing_journal_bytes() {
        for (code, reason) in [
            (
                "instance_changed",
                RecoveryQuarantineReason::InstanceChanged,
            ),
            ("scope_unknown", RecoveryQuarantineReason::ScopeUnknown),
        ] {
            let path = journal_path(code);
            let record = recovery_record(1, KnownAdmission::Ambiguous);
            save_test_journal(&path, std::slice::from_ref(&record));
            let before = fs::read(&path).unwrap();
            let (mut worker, updates) = unit_worker_with_journal(path.clone());
            worker.pending.insert("1".into(), pending_hello(1));

            worker.handle_error(
                "1".into(),
                json!({"v":1,"msg_id":"1","type":"error","code":code}),
            );

            assert!(worker.recovery.is_empty());
            assert_eq!(
                worker.quarantined_recovery,
                vec![QuarantinedRecoveryRecord {
                    record: record.clone(),
                    reason,
                }]
            );
            assert_eq!(fs::read(&path).unwrap(), before);
            assert!(worker.outgoing.is_empty());
            assert!(worker.pending.is_empty());
            let seen = updates.try_iter().collect::<Vec<_>>();
            assert!(seen.iter().any(|update| matches!(
                update,
                ClientUpdate::RecoveryProjection { active, quarantined }
                    if active.is_empty()
                        && quarantined.len() == 1
                        && quarantined[0].reason == reason
            )));
            assert!(
                !seen
                    .iter()
                    .any(|update| matches!(update, ClientUpdate::RecoveryJournalProblem { .. }))
            );
            let _ = fs::remove_dir_all(path.parent().unwrap());
        }
    }

    #[test]
    fn successful_mismatched_hello_quarantines_and_blocks_mutation_and_status() {
        for (boot, scope, reason) in [
            (
                "other-boot",
                "scope",
                RecoveryQuarantineReason::AttachedBootMismatch,
            ),
            (
                "boot",
                "other-scope",
                RecoveryQuarantineReason::AttachedScopeMismatch,
            ),
        ] {
            let path = journal_path("successful-mismatch");
            let record = recovery_record(1, KnownAdmission::Ambiguous);
            save_test_journal(&path, std::slice::from_ref(&record));
            let before = fs::read(&path).unwrap();
            let (mut worker, updates) = unit_worker_with_journal(path.clone());

            worker.handle_hello_result(hello_reply_for(&json!("1"), boot, scope, 1));
            worker.queue_mutation(2, "reference_retune".into(), json!({"target":2.0}));
            worker.queue_operation_status(3, record.identity.clone());

            assert_eq!(worker.state, ConnectionState::Ready);
            assert!(worker.recovery.is_empty());
            assert_eq!(worker.quarantined_recovery.len(), 1);
            assert_eq!(worker.quarantined_recovery[0].record, record);
            assert_eq!(worker.quarantined_recovery[0].reason, reason);
            assert_eq!(fs::read(&path).unwrap(), before);
            assert!(worker.outgoing.is_empty());
            let seen = updates.try_iter().collect::<Vec<_>>();
            assert!(
                seen.iter().any(|update| matches!(
                    update,
                    ClientUpdate::LocalRejected { command_id: 2, .. }
                ))
            );
            assert!(seen.iter().any(|update| matches!(
                update,
                ClientUpdate::LocalRejected { command_id: 3, reason }
                    if reason == "recovery_not_found"
            )));
            assert!(
                !seen
                    .iter()
                    .any(|update| matches!(update, ClientUpdate::ReconciliationRequired { .. }))
            );
            let _ = fs::remove_dir_all(path.parent().unwrap());
        }
    }

    #[test]
    fn successful_mismatched_hello_projection_is_prefix_safe() {
        for (boot, scope, reason) in [
            (
                "other-boot",
                "scope",
                RecoveryQuarantineReason::AttachedBootMismatch,
            ),
            (
                "boot",
                "other-scope",
                RecoveryQuarantineReason::AttachedScopeMismatch,
            ),
        ] {
            let path = journal_path("successful-prefix");
            let record = recovery_record(1, KnownAdmission::Ambiguous);
            save_test_journal(&path, std::slice::from_ref(&record));
            let (mut worker, updates) = unit_worker_with_journal(path.clone());
            let mut model = WorkbenchModel::new(PresentationDocument::empty("doc"));
            model.apply_client_update(ClientUpdate::RecoveryProjection {
                active: vec![record.clone()],
                quarantined: Vec::new(),
            });

            worker.handle_hello_result(hello_reply_for(&json!("1"), boot, scope, 1));
            let seen = updates.try_iter().collect::<Vec<_>>();

            assert_eq!(seen.len(), 3);
            assert!(matches!(
                seen[0],
                ClientUpdate::State(ConnectionState::Ready)
            ));
            assert!(matches!(seen[1], ClientUpdate::Hello(_)));
            assert!(matches!(
                &seen[2],
                ClientUpdate::RecoveryProjection { active, quarantined }
                    if active.is_empty()
                        && quarantined.len() == 1
                        && quarantined[0].reason == reason
            ));
            for update in seen {
                model.apply_client_update(update);
                assert_recovery_evidence_visible(&model, &record);
            }
            assert!(model.recovery.mutations.is_empty());
            assert_eq!(model.recovery.quarantined[0].record, record);
            let _ = fs::remove_dir_all(path.parent().unwrap());
        }
    }

    #[test]
    fn hello_error_quarantine_projection_is_prefix_safe() {
        for (code, reason) in [
            (
                "instance_changed",
                RecoveryQuarantineReason::InstanceChanged,
            ),
            ("scope_unknown", RecoveryQuarantineReason::ScopeUnknown),
        ] {
            let path = journal_path("error-prefix");
            let record = recovery_record(1, KnownAdmission::Ambiguous);
            save_test_journal(&path, std::slice::from_ref(&record));
            let (mut worker, updates) = unit_worker_with_journal(path.clone());
            let mut model = WorkbenchModel::new(PresentationDocument::empty("doc"));
            model.apply_client_update(ClientUpdate::RecoveryProjection {
                active: vec![record.clone()],
                quarantined: Vec::new(),
            });
            worker.pending.insert("1".into(), pending_hello(1));

            worker.handle_error(
                "1".into(),
                json!({"v":1,"msg_id":"1","type":"error","code":code}),
            );
            let seen = updates.try_iter().collect::<Vec<_>>();

            assert_eq!(seen.len(), 3);
            assert!(matches!(
                &seen[0],
                ClientUpdate::RecoveryProjection { active, quarantined }
                    if active.is_empty()
                        && quarantined.len() == 1
                        && quarantined[0].reason == reason
            ));
            assert!(matches!(
                seen[1],
                ClientUpdate::Reply {
                    kind: ReplyKind::PublicError,
                    ..
                }
            ));
            assert!(matches!(
                seen[2],
                ClientUpdate::State(ConnectionState::Disconnected)
            ));
            for update in seen {
                model.apply_client_update(update);
                assert_recovery_evidence_visible(&model, &record);
            }
            assert!(model.recovery.mutations.is_empty());
            assert_eq!(model.recovery.quarantined[0].record, record);
            let _ = fs::remove_dir_all(path.parent().unwrap());
        }
    }

    #[test]
    fn saturated_atomic_quarantine_handoff_keeps_old_evidence_until_fail_closed_update() {
        let (_command_tx, command_rx) = mpsc::sync_channel(COMMAND_QUEUE);
        let (update_tx, updates) = mpsc::sync_channel(1);
        let mut worker = Worker::new(
            "127.0.0.1:1".parse().unwrap(),
            None,
            command_rx,
            update_tx,
            Arc::new(AtomicBool::new(false)),
            None,
            Arc::new(AtomicBool::new(false)),
        );
        let record = recovery_record(1, KnownAdmission::Ambiguous);
        worker.recovery = vec![record.clone()];
        let mut model = WorkbenchModel::new(PresentationDocument::empty("doc"));
        model.apply_client_update(ClientUpdate::RecoveryProjection {
            active: vec![record.clone()],
            quarantined: Vec::new(),
        });

        worker.emit(ClientUpdate::State(ConnectionState::Ready));
        assert!(worker.classify_active_recovery(RecoveryQuarantineReason::AttachedBootMismatch));
        worker.emit_recovery_projection();

        assert!(worker.recovery.is_empty());
        assert_eq!(worker.quarantined_recovery[0].record, record);
        assert!(matches!(
            worker.deferred_update,
            Some(ClientUpdate::ResnapshotRequired {
                connection_lost: true,
                ..
            })
        ));
        model.apply_client_update(updates.try_recv().unwrap());
        assert_recovery_evidence_visible(&model, &record);

        worker.flush_deferred_update();
        let fail_closed = updates.try_recv().unwrap();
        assert!(matches!(
            fail_closed,
            ClientUpdate::ResnapshotRequired {
                connection_lost: true,
                ..
            }
        ));
        model.apply_client_update(fail_closed);
        assert_eq!(model.connection, ConnectionState::Stale);
        assert_recovery_evidence_visible(&model, &record);
        assert!(updates.try_recv().is_err());
    }

    #[test]
    fn restart_reclassifies_unchanged_journal_without_growth_or_auto_send() {
        let path = journal_path("restart-quarantine");
        let records = (1..=MAX_IN_FLIGHT as u64)
            .map(|seq| recovery_record(seq, KnownAdmission::Ambiguous))
            .collect::<Vec<_>>();
        save_test_journal(&path, &records);
        let before = fs::read(&path).unwrap();

        for _ in 0..3 {
            let (mut worker, _updates) = unit_worker_with_journal(path.clone());
            worker.pending.insert("1".into(), pending_hello(1));
            worker.handle_error(
                "1".into(),
                json!({"v":1,"msg_id":"1","type":"error","code":"instance_changed"}),
            );
            worker.pending.insert("2".into(), pending_hello(2));
            worker.handle_error(
                "2".into(),
                json!({"v":1,"msg_id":"2","type":"error","code":"instance_changed"}),
            );
            assert!(worker.recovery.is_empty());
            assert_eq!(worker.quarantined_recovery.len(), MAX_IN_FLIGHT);
            assert!(worker.outgoing.is_empty());
            assert_eq!(fs::read(&path).unwrap(), before);
        }
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn scripted_instance_change_preserves_journal_and_emits_no_followup_request() {
        let path = journal_path("scripted-instance-change");
        let record = recovery_record(1, KnownAdmission::Ambiguous);
        save_test_journal(&path, std::slice::from_ref(&record));
        let before = fs::read(&path).unwrap();
        let (address, peer) = scripted_peer(|stream| {
            let mut reader = BufReader::new(stream);
            let hello = read_request(&mut reader);
            write_value(
                reader.get_mut(),
                &json!({"v":1,"msg_id":hello["msg_id"],"type":"error",
                    "accepted":false,"code":"instance_changed"}),
            );
            let mut unexpected = String::new();
            match reader.read_line(&mut unexpected) {
                Ok(0) => {}
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) => {}
                other => {
                    panic!("classification emitted an unexpected request: {other:?} {unexpected}")
                }
            }
        });
        let client =
            ClientHandle::spawn_with_recovery_journal(address, Some(path.clone())).unwrap();
        client.connect(Some("scope".into())).unwrap();
        let quarantine = wait_for(&client, |update| {
            matches!(
                update,
                ClientUpdate::RecoveryProjection { quarantined, .. }
                    if !quarantined.is_empty()
            )
        });
        let ClientUpdate::RecoveryProjection {
            active,
            quarantined: records,
        } = quarantine
        else {
            unreachable!()
        };
        assert!(active.is_empty());
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].record, record);
        assert_eq!(records[0].reason, RecoveryQuarantineReason::InstanceChanged);
        assert_eq!(fs::read(&path).unwrap(), before);
        client.shutdown().unwrap();
        peer.join().unwrap();
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn manual_new_scope_can_rebuild_observations_but_quarantine_still_blocks_mutation() {
        let path = journal_path("manual-new-scope");
        let record = recovery_record(1, KnownAdmission::Ambiguous);
        save_test_journal(&path, std::slice::from_ref(&record));
        let before = fs::read(&path).unwrap();
        let (mut worker, updates) = unit_worker_with_journal(path.clone());
        let mut model = WorkbenchModel::new(PresentationDocument::empty("doc"));
        worker.pending.insert("1".into(), pending_hello(1));
        worker.handle_error(
            "1".into(),
            json!({"v":1,"msg_id":"1","type":"error","code":"scope_unknown"}),
        );
        apply_worker_updates(&updates, &mut model);
        assert_eq!(model.recovery.quarantined.len(), 1);

        worker.handle_hello_result(hello_reply_for(&json!("2"), "new-boot", "new-scope", 1));
        apply_worker_updates(&updates, &mut model);
        model.complete_rebuild();

        assert_eq!(model.connection, ConnectionState::Ready);
        assert_eq!(model.observations.freshness, crate::model::Freshness::Fresh);
        assert_eq!(model.recovery.quarantined.len(), 1);
        assert!(model.quarantine_blocks_mutations());
        worker.queue_mutation(3, "reference_retune".into(), json!({"target":3.0}));
        assert!(worker.outgoing.is_empty());
        assert!(updates.try_iter().any(|update| matches!(
            update,
            ClientUpdate::LocalRejected { command_id: 3, reason }
                if reason == "recovery_quarantine_unresolved"
        )));
        assert_eq!(fs::read(&path).unwrap(), before);
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn successful_bounded_update_publish_invokes_renderer_neutral_wake() {
        let (_command_tx, command_rx) = mpsc::sync_channel(COMMAND_QUEUE);
        let (update_tx, update_rx) = mpsc::sync_channel(UPDATE_QUEUE);
        let wakes = Arc::new(TestAtomicUsize::new(0));
        let observed = Arc::clone(&wakes);
        let mut worker = Worker::new(
            "127.0.0.1:1".parse().unwrap(),
            None,
            command_rx,
            update_tx,
            Arc::new(AtomicBool::new(false)),
            Some(Arc::new(move || {
                observed.fetch_add(1, TestOrdering::Release);
            })),
            Arc::new(AtomicBool::new(false)),
        );

        worker.emit(ClientUpdate::State(ConnectionState::Disconnected));

        assert!(matches!(
            update_rx.try_recv(),
            Ok(ClientUpdate::State(ConnectionState::Disconnected))
        ));
        assert_eq!(wakes.load(TestOrdering::Acquire), 1);
    }

    fn apply_worker_updates(
        updates: &Receiver<ClientUpdate>,
        model: &mut WorkbenchModel,
    ) -> Vec<ClientUpdate> {
        let mut seen = Vec::new();
        while let Ok(update) = updates.try_recv() {
            model.apply_client_update(update.clone());
            seen.push(update);
        }
        seen
    }

    #[test]
    fn operation_status_replaces_ambiguous_model_recovery_projection() {
        let (mut worker, updates) = unit_worker();
        let record = recovery_record(1, KnownAdmission::Ambiguous);
        worker.hello = Some(hello_state(1));
        worker.recovery = vec![record.clone()];
        let mut model = WorkbenchModel::new(PresentationDocument::empty("doc"));
        model.apply_client_update(ClientUpdate::RecoveryProjection {
            active: vec![record],
            quarantined: Vec::new(),
        });
        assert_eq!(
            model.recovery.mutations[0].admission,
            KnownAdmission::Ambiguous
        );

        worker.handle_operation_status_result(
            PendingExchange {
                command_id: 1,
                msg_id: "1".into(),
                op: "operation_status".into(),
                purpose: Purpose::OperationStatus,
                mutation: None,
                status_identity: Some(MutationIdentity {
                    scope: "scope".into(),
                    seq: 1,
                }),
                transmitted: true,
                deadline: None,
            },
            json!({"v":1,"msg_id":"1","type":"result","result":{
                "request_id":{"scope":"scope","seq":"1"},"state":"completed"
            }}),
        );
        let seen = apply_worker_updates(&updates, &mut model);

        assert!(seen.iter().any(|update| matches!(
            update,
            ClientUpdate::RecoveryProjection { active, .. }
                if active[0].admission == KnownAdmission::Completed
        )));
        assert_eq!(worker.recovery[0].admission, KnownAdmission::Completed);
        assert_eq!(
            model.recovery.mutations[0].admission,
            KnownAdmission::Completed
        );
    }

    #[test]
    fn exact_retry_terminal_projection_retains_model_record() {
        let (mut worker, updates) = unit_worker();
        let record = recovery_record(1, KnownAdmission::Ambiguous);
        worker.hello = Some(hello_state(2));
        worker.recovery = vec![record.clone()];
        worker.pending.insert(
            "1".into(),
            PendingExchange {
                command_id: 1,
                msg_id: "1".into(),
                op: record.op.clone(),
                purpose: Purpose::RetryMutation,
                mutation: Some(record.clone()),
                status_identity: None,
                transmitted: true,
                deadline: None,
            },
        );
        let mut model = WorkbenchModel::new(PresentationDocument::empty("doc"));
        model.apply_client_update(ClientUpdate::RecoveryProjection {
            active: vec![record],
            quarantined: Vec::new(),
        });

        worker.handle_operation(
            "1".into(),
            json!({"v":1,"msg_id":"1","type":"operation","state":"completed",
                "result":{"revision":"2"}}),
        );
        let seen = apply_worker_updates(&updates, &mut model);

        assert!(seen.iter().any(|update| matches!(
            update,
            ClientUpdate::RecoveryProjection { active, .. }
                if active.len() == 1 && active[0].admission == KnownAdmission::Completed
        )));
        assert_eq!(worker.recovery.len(), 1);
        assert_eq!(worker.recovery[0].admission, KnownAdmission::Completed);
        assert_eq!(model.recovery.mutations, worker.recovery);
        assert!(seen.iter().any(|update| matches!(
            update,
            ClientUpdate::Reply {
                kind: ReplyKind::MutationCompleted,
                recovery: Some(_),
                ..
            }
        )));
    }

    #[test]
    fn exact_retry_terminal_is_durable_and_reloads_without_automatic_send() {
        for (state, admission) in [
            ("completed", KnownAdmission::Completed),
            ("failed", KnownAdmission::Failed),
        ] {
            let path = journal_path(&format!("retry-terminal-{state}"));
            let record = recovery_record(1, KnownAdmission::Ambiguous);
            save_test_journal(&path, std::slice::from_ref(&record));
            let (mut worker, _updates) = unit_worker_with_journal(path.clone());
            worker.state = ConnectionState::Ready;
            worker.hello = Some(hello_state(2));
            worker.pending.insert(
                "1".into(),
                PendingExchange {
                    command_id: 1,
                    msg_id: "1".into(),
                    op: record.op.clone(),
                    purpose: Purpose::RetryMutation,
                    mutation: Some(record.clone()),
                    status_identity: None,
                    transmitted: true,
                    deadline: None,
                },
            );

            worker.handle_operation(
                "1".into(),
                json!({"v":1,"msg_id":"1","type":"operation","state":state,
                    "result":{"revision":"2"}}),
            );

            assert_eq!(worker.recovery.len(), 1);
            assert_eq!(worker.recovery[0].identity, record.identity);
            assert_eq!(worker.recovery[0].op, record.op);
            assert_eq!(worker.recovery[0].args, record.args);
            assert_eq!(worker.recovery[0].admission, admission);
            let persisted = load_journal(&path).unwrap().to_recovery_records();
            assert_eq!(persisted, worker.recovery);

            let (restarted, _restart_updates) = unit_worker_with_journal(path.clone());
            assert_eq!(restarted.recovery, worker.recovery);
            assert!(restarted.outgoing.is_empty());
            assert!(restarted.pending.is_empty());
            let _ = fs::remove_dir_all(path.parent().unwrap());
        }
    }

    #[test]
    fn retry_public_error_never_erases_or_reclassifies_exact_evidence() {
        for code in ["outcome_unknown", "busy"] {
            let (mut worker, updates) = unit_worker();
            let record = recovery_record(1, KnownAdmission::Pending);
            worker.recovery = vec![record.clone()];
            worker.pending.insert(
                "1".into(),
                PendingExchange {
                    command_id: 1,
                    msg_id: "1".into(),
                    op: record.op.clone(),
                    purpose: Purpose::RetryMutation,
                    mutation: Some(record.clone()),
                    status_identity: None,
                    transmitted: true,
                    deadline: None,
                },
            );

            worker.handle_error(
                "1".into(),
                json!({"v":1,"msg_id":"1","type":"error","code":code}),
            );

            assert_eq!(worker.recovery, vec![record.clone()]);
            assert!(updates.try_iter().any(|update| matches!(
                update,
                ClientUpdate::Reply {
                    kind: ReplyKind::PublicError,
                    recovery: Some(recovery),
                    envelope,
                    ..
                } if recovery == record && envelope["code"] == code
            )));
        }
    }

    #[test]
    fn retry_worker_rejects_terminal_quarantine_and_journal_failure_before_wire() {
        let identity = recovery_record(1, KnownAdmission::Ambiguous).identity;
        for (reason, configure) in [
            ("recovery_journal_unavailable", 0_u8),
            ("recovery_quarantine_unresolved", 1_u8),
            ("recovery_terminal", 2_u8),
        ] {
            let (mut worker, updates) = unit_worker();
            worker.state = ConnectionState::Ready;
            worker.hello = Some(hello_state(2));
            let mut record = recovery_record(1, KnownAdmission::Ambiguous);
            match configure {
                0 => worker.journal_failed = true,
                1 => {
                    worker.quarantined_recovery = vec![QuarantinedRecoveryRecord {
                        record: record.clone(),
                        reason: RecoveryQuarantineReason::AttachedBootMismatch,
                    }];
                }
                2 => record.admission = KnownAdmission::Completed,
                _ => unreachable!(),
            }
            worker.recovery = vec![record];

            worker.queue_retry(7, identity.clone());

            assert!(worker.outgoing.is_empty());
            assert!(worker.pending.is_empty());
            assert!(updates.try_iter().any(|update| matches!(
                update,
                ClientUpdate::LocalRejected { command_id: 7, reason: actual }
                    if actual == reason
            )));
        }
    }

    #[test]
    fn status_and_retry_are_mutually_exclusive_and_duplicate_retry_is_single_wire_request() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let stream = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (_peer, _) = listener.accept().unwrap();
        stream.set_nonblocking(true).unwrap();
        let (mut worker, updates) = unit_worker();
        let record = recovery_record(1, KnownAdmission::Ambiguous);
        worker.state = ConnectionState::Ready;
        worker.hello = Some(hello_state(2));
        worker.recovery = vec![record.clone()];
        worker.stream = Some(stream);

        worker.queue_operation_status(1, record.identity.clone());
        worker.queue_retry(2, record.identity.clone());
        assert_eq!(worker.pending.len(), 1);
        assert_eq!(worker.outgoing.len(), 1);
        assert!(updates.try_iter().any(|update| matches!(
            update,
            ClientUpdate::LocalRejected { command_id: 2, reason }
                if reason == "recovery_action_in_flight"
        )));

        worker.pending.clear();
        worker.outgoing.clear();
        worker.queue_retry(3, record.identity.clone());
        worker.queue_operation_status(4, record.identity.clone());
        worker.queue_retry(5, record.identity);
        assert_eq!(worker.pending.len(), 1);
        assert_eq!(worker.outgoing.len(), 1);
        let rejected = updates
            .try_iter()
            .filter(|update| {
                matches!(
                    update,
                    ClientUpdate::LocalRejected { command_id: 4 | 5, reason }
                        if reason == "recovery_action_in_flight"
                )
            })
            .count();
        assert_eq!(rejected, 2);
    }

    #[test]
    fn terminal_retry_under_update_pressure_keeps_durable_restart_evidence() {
        let path = journal_path("retry-terminal-pressure");
        let record = recovery_record(1, KnownAdmission::Ambiguous);
        save_test_journal(&path, std::slice::from_ref(&record));
        let (_command_tx, command_rx) = mpsc::sync_channel(COMMAND_QUEUE);
        let (update_tx, updates) = mpsc::sync_channel(1);
        let mut worker = Worker::new(
            "127.0.0.1:1".parse().unwrap(),
            Some(path.clone()),
            command_rx,
            update_tx,
            Arc::new(AtomicBool::new(false)),
            None,
            Arc::new(AtomicBool::new(false)),
        );
        worker.state = ConnectionState::Ready;
        worker.hello = Some(hello_state(2));
        worker.pending.insert(
            "1".into(),
            PendingExchange {
                command_id: 1,
                msg_id: "1".into(),
                op: record.op.clone(),
                purpose: Purpose::RetryMutation,
                mutation: Some(record.clone()),
                status_identity: None,
                transmitted: true,
                deadline: None,
            },
        );
        let mut model = WorkbenchModel::new(PresentationDocument::empty("doc"));
        model.apply_client_update(ClientUpdate::RecoveryProjection {
            active: vec![record.clone()],
            quarantined: Vec::new(),
        });
        worker.emit(ClientUpdate::State(ConnectionState::Ready));

        worker.handle_operation(
            "1".into(),
            json!({"v":1,"msg_id":"1","type":"operation","state":"completed",
                "result":{"revision":"2"}}),
        );

        assert_eq!(worker.recovery[0].admission, KnownAdmission::Completed);
        assert_eq!(
            load_journal(&path).unwrap().to_recovery_records(),
            worker.recovery
        );
        model.apply_client_update(updates.try_recv().unwrap());
        assert_recovery_evidence_visible(&model, &record);
        worker.flush_deferred_update();
        model.apply_client_update(updates.try_recv().unwrap());
        assert_eq!(model.connection, ConnectionState::Stale);
        assert_recovery_evidence_visible(&model, &record);

        let (restarted, _restart_updates) = unit_worker_with_journal(path.clone());
        assert_eq!(restarted.recovery[0].admission, KnownAdmission::Completed);
        assert_eq!(restarted.recovery[0].identity, record.identity);
        assert_eq!(restarted.recovery[0].op, record.op);
        assert_eq!(restarted.recovery[0].args, record.args);
        assert!(restarted.outgoing.is_empty());
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn retained_terminal_and_capacity_retirement_share_worker_projection() {
        let (mut worker, updates) = unit_worker();
        let record = recovery_record(1, KnownAdmission::Pending);
        worker.hello = Some(hello_state(1));
        worker.pending.insert(
            "1".into(),
            PendingExchange {
                command_id: 1,
                msg_id: "1".into(),
                op: record.op.clone(),
                purpose: Purpose::Mutation,
                mutation: Some(record),
                status_identity: None,
                transmitted: true,
                deadline: None,
            },
        );
        let mut model = WorkbenchModel::new(PresentationDocument::empty("doc"));

        worker.handle_operation(
            "1".into(),
            json!({"v":1,"msg_id":"1","type":"operation","state":"completed",
                "result":{"revision":"2"}}),
        );
        apply_worker_updates(&updates, &mut model);
        assert_eq!(worker.recovery, model.recovery.mutations);
        assert_eq!(worker.recovery[0].admission, KnownAdmission::Completed);

        worker.recovery = (1..=MAX_IN_FLIGHT as u64)
            .map(|seq| recovery_record(seq, KnownAdmission::Completed))
            .collect();
        worker.emit_recovery_projection();
        apply_worker_updates(&updates, &mut model);
        worker.hello = Some(hello_state(9));
        worker.state = ConnectionState::Ready;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let stream = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (_peer, _) = listener.accept().unwrap();
        stream.set_nonblocking(true).unwrap();
        worker.stream = Some(stream);

        worker.queue_mutation(
            9,
            "reference_retune".into(),
            json!({"reference":"1","expected_revision":"9","target":9.0,"rate":1.0}),
        );
        let seen = apply_worker_updates(&updates, &mut model);

        assert!(seen.iter().any(|update| matches!(
            update,
            ClientUpdate::RecoveryProjection { active, .. }
                if active.len() == MAX_IN_FLIGHT
                    && !active.iter().any(|record| record.identity.seq == 1)
                    && active.iter().any(|record| record.identity.seq == 9)
        )));
        assert_eq!(worker.recovery, model.recovery.mutations);
        assert!(
            !model
                .recovery
                .mutations
                .iter()
                .any(|record| record.identity.seq == 1)
        );
        assert!(
            model
                .recovery
                .mutations
                .iter()
                .any(|record| record.identity.seq == 9)
        );
    }

    #[test]
    fn durable_exact_record_exists_before_mutation_reaches_wire() {
        let path = journal_path("pre-wire");
        let peer_path = path.clone();
        let (address, peer) = scripted_peer(move |stream| {
            let mut reader = BufReader::new(stream);
            let hello = read_request(&mut reader);
            let reply = hello_reply(&hello["msg_id"], "scope", 1);
            write_value(reader.get_mut(), &reply);

            let mutation = read_request(&mut reader);
            let journal = load_journal(&peer_path).expect("journal must precede wire bytes");
            assert_eq!(journal.boot_id, "boot");
            assert_eq!(journal.scope, "scope");
            assert_eq!(journal.records.len(), 1);
            assert_eq!(journal.records[0].seq, "1");
            assert_eq!(journal.records[0].op, "reference_retune");
            assert_eq!(journal.records[0].args, mutation["args"]);
            assert_eq!(
                journal.records[0].admission,
                crate::recovery::JournalAdmission::Pending
            );

            write_value(
                reader.get_mut(),
                &json!({"v":1,"msg_id":mutation["msg_id"],"type":"operation",
                    "state":"completed","result":{"revision":"2"}}),
            );
        });
        let client =
            ClientHandle::spawn_with_recovery_journal(address, Some(path.clone())).unwrap();
        client.connect(None).unwrap();
        wait_for(&client, |update| matches!(update, ClientUpdate::Hello(_)));
        client
            .mutation(
                "reference_retune",
                json!({"reference":"1","expected_revision":"1","target":2.0,"rate":1.0}),
            )
            .unwrap();
        wait_for(&client, |update| {
            matches!(
                update,
                ClientUpdate::Reply {
                    kind: ReplyKind::MutationCompleted,
                    ..
                }
            )
        });
        client.shutdown().unwrap();
        peer.join().unwrap();
        let directory = path.parent().unwrap().to_owned();
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn loaded_journal_only_requests_reconciliation_and_never_auto_sends() {
        let path = journal_path("no-auto-send");
        let record = RecoveryRecord {
            boot_id: "boot".into(),
            identity: MutationIdentity {
                scope: "scope".into(),
                seq: 1,
            },
            op: "reference_retune".into(),
            args: json!({"reference":"1","target":2.0,"rate":1.0}),
            admission: KnownAdmission::Ambiguous,
        };
        let journal = RecoveryJournal::from_records(
            "boot".into(),
            "scope".into(),
            2,
            std::slice::from_ref(&record),
        )
        .unwrap();
        save_journal(&path, &journal).unwrap();

        let (quiet_tx, quiet_rx) = mpsc::channel();
        let (address, peer) = scripted_peer(move |stream| {
            let mut reader = BufReader::new(stream);
            let hello = read_request(&mut reader);
            assert_eq!(hello["args"]["scope"], "scope");
            write_value(reader.get_mut(), &hello_reply(&hello["msg_id"], "scope", 2));
            reader
                .get_mut()
                .set_read_timeout(Some(Duration::from_millis(250)))
                .unwrap();
            let mut line = String::new();
            let result = reader.read_line(&mut line);
            quiet_tx.send((result, line)).unwrap();
        });
        let client =
            ClientHandle::spawn_with_recovery_journal(address, Some(path.clone())).unwrap();
        client.connect(Some("scope".into())).unwrap();
        wait_for(&client, |update| matches!(update, ClientUpdate::Hello(_)));
        let reconciliation = wait_for(&client, |update| {
            matches!(update, ClientUpdate::ReconciliationRequired { .. })
        });
        let ClientUpdate::ReconciliationRequired { records } = reconciliation else {
            unreachable!()
        };
        assert_eq!(records[0].op, record.op);
        let (read, line) = quiet_rx.recv_timeout(TEST_TIMEOUT).unwrap();
        assert!(read.is_err(), "journal load unexpectedly wrote {line:?}");
        client.shutdown().unwrap();
        peer.join().unwrap();
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn journal_failure_rejects_mutation_before_wire_emission() {
        let base_path = journal_path("write-failure");
        let directory = base_path.parent().unwrap().to_owned();
        let blocked_parent = directory.join("not-a-directory");
        fs::write(&blocked_parent, b"block directory creation").unwrap();
        let unusable_path = blocked_parent.join("recovery.json");

        let (quiet_tx, quiet_rx) = mpsc::channel();
        let (address, peer) = scripted_peer(move |stream| {
            let mut reader = BufReader::new(stream);
            let hello = read_request(&mut reader);
            write_value(reader.get_mut(), &hello_reply(&hello["msg_id"], "scope", 1));
            reader
                .get_mut()
                .set_read_timeout(Some(Duration::from_millis(250)))
                .unwrap();
            let mut line = String::new();
            quiet_tx.send((reader.read_line(&mut line), line)).unwrap();
        });
        let client =
            ClientHandle::spawn_with_recovery_journal(address, Some(unusable_path)).unwrap();
        client.connect(None).unwrap();
        wait_for(&client, |update| matches!(update, ClientUpdate::Hello(_)));
        let command_id = client
            .mutation(
                "reference_retune",
                json!({"reference":"1","target":2.0,"rate":1.0}),
            )
            .unwrap();
        wait_for(&client, |update| {
            matches!(update, ClientUpdate::RecoveryJournalProblem { .. })
        });
        wait_for(&client, |update| {
            matches!(update, ClientUpdate::LocalRejected { command_id: id, reason }
                if *id == command_id && reason == "recovery_journal_unavailable")
        });
        let (read, line) = quiet_rx.recv_timeout(TEST_TIMEOUT).unwrap();
        assert!(
            read.is_err(),
            "journal failure leaked mutation bytes: {line:?}"
        );
        client.shutdown().unwrap();
        peer.join().unwrap();
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn hello_owns_runtime_advertised_scope_and_resets_msg_id_on_reconnect() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (seen_tx, seen_rx) = mpsc::channel();
        let peer = thread::spawn(move || {
            for scope in [None, Some("boot:1")] {
                let (stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream);
                let request = read_request(&mut reader);
                seen_tx
                    .send((request["msg_id"].clone(), request["args"]["scope"].clone()))
                    .unwrap();
                let reply = hello_reply(&request["msg_id"], "boot:1", 1);
                write_value(reader.get_mut(), &reply);
                if scope.is_none() {
                    drop(reader);
                }
            }
        });
        let client = ClientHandle::spawn(address).unwrap();
        client.connect(None).unwrap();
        let hello = wait_for(&client, |u| matches!(u, ClientUpdate::Hello(_)));
        let ClientUpdate::Hello(hello) = hello else {
            unreachable!()
        };
        assert_eq!(hello.scope, "boot:1");
        wait_for(&client, |u| {
            matches!(u, ClientUpdate::TransportFailure { .. })
        });
        client.connect(Some(hello.scope)).unwrap();
        wait_for(&client, |u| matches!(u, ClientUpdate::Hello(_)));
        assert_eq!(seen_rx.recv().unwrap(), (json!("1"), Value::Null));
        assert_eq!(seen_rx.recv().unwrap(), (json!("1"), json!("boot:1")));
        client.shutdown().unwrap();
        peer.join().unwrap();
    }

    #[test]
    fn reattach_retries_scope_in_use_inside_one_absolute_deadline() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let peer = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut first = BufReader::new(stream);
            let hello = read_request(&mut first);
            write_value(
                first.get_mut(),
                &json!({"v":1,"msg_id":hello["msg_id"],"type":"error","accepted":false,
                    "code":"scope_in_use","category":"session_error","message":"busy",
                    "retryable":true,"resync_required":false}),
            );
            drop(first);

            let (stream, _) = listener.accept().unwrap();
            let mut second = BufReader::new(stream);
            let hello = read_request(&mut second);
            assert_eq!(hello["args"]["scope"], "boot:1");
            write_value(
                second.get_mut(),
                &hello_reply(&hello["msg_id"], "boot:1", 2),
            );
        });
        let client = ClientHandle::spawn(address).unwrap();
        client.connect(Some("boot:1".into())).unwrap();
        let hello = wait_for(&client, |u| matches!(u, ClientUpdate::Hello(_)));
        assert!(matches!(
            hello,
            ClientUpdate::Hello(HelloState { next_seq: 2, .. })
        ));
        client.shutdown().unwrap();
        peer.join().unwrap();
    }

    #[test]
    fn reattach_connect_timeout_is_capped_by_one_absolute_deadline() {
        let start = Instant::now();
        let deadline = start + REATTACH_DEADLINE;
        assert_eq!(
            connect_timeout_for_attempt(start, Some(deadline)),
            Some(CONNECT_DEADLINE)
        );
        assert_eq!(
            connect_timeout_for_attempt(start + Duration::from_millis(2_500), Some(deadline)),
            Some(Duration::from_millis(500))
        );
        assert_eq!(connect_timeout_for_attempt(deadline, Some(deadline)), None);
        assert_eq!(
            connect_timeout_for_attempt(deadline + Duration::from_millis(1), Some(deadline)),
            None
        );
        assert_eq!(
            connect_timeout_for_attempt(start, None),
            Some(CONNECT_DEADLINE)
        );
    }

    #[test]
    fn replies_correlate_out_of_order_while_events_route_independently() {
        let (address, peer) = scripted_peer(|stream| {
            let mut reader = BufReader::new(stream);
            let hello = read_request(&mut reader);
            write_value(
                reader.get_mut(),
                &hello_reply(&hello["msg_id"], "boot:1", 1),
            );
            let first = read_request(&mut reader);
            let second = read_request(&mut reader);
            write_value(
                reader.get_mut(),
                &json!({"v":1,"type":"event","boot_id":"boot","seq":"1",
                "kind":"reference","target":{"id":"1"},"data":{"revision":"1"}}),
            );
            write_value(
                reader.get_mut(),
                &json!({"v":1,"msg_id":second["msg_id"],"type":"result","result":{"which":2}}),
            );
            write_value(
                reader.get_mut(),
                &json!({"v":1,"msg_id":first["msg_id"],"type":"result","result":{"which":1}}),
            );
        });
        let client = ClientHandle::spawn(address).unwrap();
        client.connect(None).unwrap();
        wait_for(&client, |u| matches!(u, ClientUpdate::Hello(_)));
        let first = client.query("reference", json!({"reference":"1"})).unwrap();
        let second = client.query("reference", json!({"reference":"2"})).unwrap();
        let mut order = Vec::new();
        let mut saw_event = false;
        while order.len() != 2 || !saw_event {
            match client.recv_timeout(TEST_TIMEOUT).unwrap() {
                ClientUpdate::Reply { command_id, .. } => order.push(command_id),
                ClientUpdate::Event { .. } => saw_event = true,
                _ => {}
            }
        }
        assert_eq!(order, vec![second, first]);
        client.shutdown().unwrap();
        peer.join().unwrap();
    }

    #[test]
    fn ninth_in_flight_request_is_rejected_before_socket_write() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (read_tx, read_rx) = mpsc::channel();
        let peer = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream);
            let hello = read_request(&mut reader);
            write_value(
                reader.get_mut(),
                &hello_reply(&hello["msg_id"], "boot:1", 1),
            );
            for _ in 0..MAX_IN_FLIGHT {
                let _ = read_request(&mut reader);
            }
            read_tx.send(()).unwrap();
        });
        let client = ClientHandle::spawn(address).unwrap();
        client.connect(None).unwrap();
        wait_for(&client, |u| matches!(u, ClientUpdate::Hello(_)));
        for _ in 0..MAX_IN_FLIGHT {
            client.query("reference", json!({"reference":"1"})).unwrap();
        }
        let ninth = client.query("reference", json!({"reference":"1"})).unwrap();
        let rejected = wait_for(&client, |update| {
            matches!(update, ClientUpdate::LocalRejected { command_id, reason }
                if *command_id == ninth && reason == "in_flight_limit")
        });
        assert!(matches!(rejected, ClientUpdate::LocalRejected { .. }));
        read_rx.recv_timeout(TEST_TIMEOUT).unwrap();
        client.shutdown().unwrap();
        peer.join().unwrap();
    }

    #[test]
    fn mutation_sequence_advances_only_after_accepted_and_disconnect_is_ambiguous() {
        let (address, peer) = scripted_peer(|stream| {
            let mut reader = BufReader::new(stream);
            let hello = read_request(&mut reader);
            write_value(
                reader.get_mut(),
                &hello_reply(&hello["msg_id"], "boot:1", 1),
            );
            let mutation = read_request(&mut reader);
            assert_eq!(mutation["request_id"]["seq"], "1");
            write_value(
                reader.get_mut(),
                &json!({"v":1,"msg_id":mutation["msg_id"],"type":"operation",
                "request_id":mutation["request_id"],"state":"accepted"}),
            );
        });
        let client = ClientHandle::spawn(address).unwrap();
        client.connect(None).unwrap();
        wait_for(&client, |u| matches!(u, ClientUpdate::Hello(_)));
        client
            .mutation(
                "reference_retune",
                json!({"reference":"1","expected_revision":"1","target":40.0,"rate":2.0}),
            )
            .unwrap();
        let accepted = wait_for(&client, |u| {
            matches!(
                u,
                ClientUpdate::Reply {
                    kind: ReplyKind::MutationAccepted,
                    ..
                }
            )
        });
        let ClientUpdate::Reply { recovery, .. } = accepted else {
            unreachable!()
        };
        assert_eq!(recovery.unwrap().identity.seq, 1);
        let reconciliation = wait_for(&client, |u| {
            matches!(u, ClientUpdate::ReconciliationRequired { .. })
        });
        let ClientUpdate::ReconciliationRequired { records } = reconciliation else {
            unreachable!()
        };
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].admission, KnownAdmission::Accepted);
        client.shutdown().unwrap();
        peer.join().unwrap();
    }

    #[test]
    fn full_recovery_capacity_rejects_before_wire_and_reopens_after_reconciliation() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let peer = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(TEST_TIMEOUT)).unwrap();
            stream.set_write_timeout(Some(TEST_TIMEOUT)).unwrap();
            let mut first = BufReader::new(stream);
            let hello = read_request(&mut first);
            write_value(first.get_mut(), &hello_reply(&hello["msg_id"], "boot:1", 1));
            for seq in 1..=MAX_IN_FLIGHT as u64 {
                let mutation = read_request(&mut first);
                assert_eq!(mutation["op"], "reference_retune");
                assert_eq!(mutation["request_id"]["seq"], seq.to_string());
                write_value(
                    first.get_mut(),
                    &json!({"v":1,"msg_id":mutation["msg_id"],"type":"operation",
                        "request_id":mutation["request_id"],"state":"accepted"}),
                );
            }
            let mut eof = String::new();
            assert_eq!(first.read_line(&mut eof).unwrap(), 0);

            let (stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(TEST_TIMEOUT)).unwrap();
            stream.set_write_timeout(Some(TEST_TIMEOUT)).unwrap();
            let mut second = BufReader::new(stream);
            let hello = read_request(&mut second);
            write_value(
                second.get_mut(),
                &hello_reply(&hello["msg_id"], "boot:1", 9),
            );

            let retry = read_request(&mut second);
            assert_eq!(retry["op"], "reference_retune");
            assert_eq!(retry["request_id"]["seq"], "1");
            write_value(
                second.get_mut(),
                &json!({"v":1,"msg_id":retry["msg_id"],"type":"operation",
                    "request_id":retry["request_id"],"state":"completed",
                    "result":{"revision":"2"}}),
            );

            let mutation = read_request(&mut second);
            assert_eq!(mutation["op"], "reference_retune");
            assert_eq!(mutation["request_id"]["seq"], "9");
            write_value(
                second.get_mut(),
                &json!({"v":1,"msg_id":mutation["msg_id"],"type":"operation",
                    "request_id":mutation["request_id"],"state":"accepted"}),
            );
        });

        let client = ClientHandle::spawn(address).unwrap();
        client.connect(None).unwrap();
        let hello = wait_for(&client, |update| matches!(update, ClientUpdate::Hello(_)));
        let ClientUpdate::Hello(hello) = hello else {
            unreachable!()
        };
        for target in 1..=MAX_IN_FLIGHT {
            client
                .mutation(
                    "reference_retune",
                    json!({"reference":"1","expected_revision":target.to_string(),
                        "target":target as f64,"rate":2.0}),
                )
                .unwrap();
            wait_for(&client, |update| {
                matches!(
                    update,
                    ClientUpdate::Reply {
                        kind: ReplyKind::MutationAccepted,
                        ..
                    }
                )
            });
        }
        client.disconnect().unwrap();
        wait_for(&client, |update| {
            matches!(update, ClientUpdate::State(ConnectionState::Disconnected))
        });
        client.connect(Some(hello.scope.clone())).unwrap();
        wait_for(&client, |update| matches!(update, ClientUpdate::Hello(_)));

        let rejected = client
            .mutation(
                "reference_retune",
                json!({"reference":"1","expected_revision":"9","target":9.0,"rate":2.0}),
            )
            .unwrap();
        wait_for(&client, |update| {
            matches!(update, ClientUpdate::LocalRejected { command_id, reason }
                if *command_id == rejected && reason == "recovery_capacity")
        });

        client
            .retry_mutation(MutationIdentity {
                scope: hello.scope,
                seq: 1,
            })
            .unwrap();
        wait_for(&client, |update| {
            matches!(
                update,
                ClientUpdate::Reply {
                    kind: ReplyKind::MutationCompleted,
                    ..
                }
            )
        });
        client
            .mutation(
                "reference_retune",
                json!({"reference":"1","expected_revision":"9","target":9.0,"rate":2.0}),
            )
            .unwrap();
        wait_for(&client, |update| {
            matches!(
                update,
                ClientUpdate::Reply {
                    kind: ReplyKind::MutationAccepted,
                    ..
                }
            )
        });
        client.shutdown().unwrap();
        peer.join().unwrap();
    }

    #[test]
    fn retry_uses_only_worker_owned_exact_payload_and_rejects_untracked_identity() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let original_args = json!({"reference":"1","expected_revision":"1",
            "target":41.5,"rate":2.25});
        let peer_args = original_args.clone();
        let peer = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(TEST_TIMEOUT)).unwrap();
            stream.set_write_timeout(Some(TEST_TIMEOUT)).unwrap();
            let mut first = BufReader::new(stream);
            let hello = read_request(&mut first);
            write_value(first.get_mut(), &hello_reply(&hello["msg_id"], "boot:1", 1));
            let mutation = read_request(&mut first);
            assert_eq!(mutation["op"], "reference_retune");
            assert_eq!(mutation["args"], peer_args);
            write_value(
                first.get_mut(),
                &json!({"v":1,"msg_id":mutation["msg_id"],"type":"operation",
                    "request_id":mutation["request_id"],"state":"accepted"}),
            );
            let mut eof = String::new();
            assert_eq!(first.read_line(&mut eof).unwrap(), 0);

            let (stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(TEST_TIMEOUT)).unwrap();
            stream.set_write_timeout(Some(TEST_TIMEOUT)).unwrap();
            let mut second = BufReader::new(stream);
            let hello = read_request(&mut second);
            write_value(
                second.get_mut(),
                &hello_reply(&hello["msg_id"], "boot:1", 2),
            );
            let retry = read_request(&mut second);
            assert_eq!(retry["op"], "reference_retune");
            assert_eq!(retry["args"], peer_args);
            assert_eq!(retry["request_id"]["scope"], "boot:1");
            assert_eq!(retry["request_id"]["seq"], "1");
            write_value(
                second.get_mut(),
                &json!({"v":1,"msg_id":retry["msg_id"],"type":"operation",
                    "request_id":retry["request_id"],"state":"completed",
                    "result":{"revision":"2"}}),
            );
        });

        let client = ClientHandle::spawn(address).unwrap();
        client.connect(None).unwrap();
        let hello = wait_for(&client, |update| matches!(update, ClientUpdate::Hello(_)));
        let ClientUpdate::Hello(hello) = hello else {
            unreachable!()
        };
        client.mutation("reference_retune", original_args).unwrap();
        let accepted = wait_for(&client, |update| {
            matches!(
                update,
                ClientUpdate::Reply {
                    kind: ReplyKind::MutationAccepted,
                    ..
                }
            )
        });
        let ClientUpdate::Reply {
            recovery: Some(record),
            ..
        } = accepted
        else {
            unreachable!()
        };
        let in_flight = client.retry_mutation(record.identity.clone()).unwrap();
        wait_for(&client, |update| {
            matches!(update, ClientUpdate::LocalRejected { command_id, reason }
                if *command_id == in_flight && reason == "recovery_action_in_flight")
        });
        client.disconnect().unwrap();
        wait_for(&client, |update| {
            matches!(update, ClientUpdate::State(ConnectionState::Disconnected))
        });
        client.connect(Some(hello.scope.clone())).unwrap();
        wait_for(&client, |update| matches!(update, ClientUpdate::Hello(_)));

        let fabricated = client
            .retry_mutation(MutationIdentity {
                scope: hello.scope,
                seq: 99,
            })
            .unwrap();
        wait_for(&client, |update| {
            matches!(update, ClientUpdate::LocalRejected { command_id, reason }
                if *command_id == fabricated && reason == "recovery_not_found")
        });
        client.retry_mutation(record.identity).unwrap();
        wait_for(&client, |update| {
            matches!(
                update,
                ClientUpdate::Reply {
                    kind: ReplyKind::MutationCompleted,
                    ..
                }
            )
        });
        client.shutdown().unwrap();
        peer.join().unwrap();
    }

    #[test]
    fn bootstrap_and_subscription_lifecycle_share_one_owner() {
        let (address, peer) = scripted_peer(|stream| {
            let mut reader = BufReader::new(stream);
            let hello = read_request(&mut reader);
            write_value(
                reader.get_mut(),
                &hello_reply(&hello["msg_id"], "boot:1", 1),
            );

            let subscribe = read_request(&mut reader);
            assert_eq!(subscribe["op"], "subscribe");
            write_value(
                reader.get_mut(),
                &json!({"v":1,"msg_id":subscribe["msg_id"],"type":"result",
                    "result":{"subscription":"sub-1","accepted_cursor":{"boot_id":"boot","seq":"0"}}}),
            );
            let reference = read_request(&mut reader);
            assert_eq!(reference["op"], "reference");
            write_value(
                reader.get_mut(),
                &json!({"v":1,"type":"event","boot_id":"boot","seq":"1",
                    "kind":"reference","target":{"id":"1"},"data":{"reference":"1","revision":"2"}}),
            );
            write_value(
                reader.get_mut(),
                &json!({"v":1,"msg_id":reference["msg_id"],"type":"result",
                    "result":{"reference":"1","revision":"1","value":20.0}}),
            );

            let unsubscribe = read_request(&mut reader);
            assert_eq!(unsubscribe["op"], "unsubscribe");
            assert_eq!(unsubscribe["args"]["subscription"], "sub-1");
            write_value(
                reader.get_mut(),
                &json!({"v":1,"msg_id":unsubscribe["msg_id"],"type":"result","result":{"unsubscribed":true}}),
            );

            let subscribe = read_request(&mut reader);
            assert_eq!(subscribe["op"], "subscribe");
            write_value(
                reader.get_mut(),
                &json!({"v":1,"msg_id":subscribe["msg_id"],"type":"result",
                    "result":{"subscription":"sub-2","accepted_cursor":{"boot_id":"boot","seq":"1"}}}),
            );
            let unsubscribe = read_request(&mut reader);
            assert_eq!(unsubscribe["op"], "unsubscribe");
            write_value(
                reader.get_mut(),
                &json!({"v":1,"msg_id":unsubscribe["msg_id"],"type":"result","result":{"unsubscribed":true}}),
            );
        });
        let client = ClientHandle::spawn(address).unwrap();
        client.connect(None).unwrap();
        wait_for(&client, |u| matches!(u, ClientUpdate::Hello(_)));

        let bootstrap = client.bootstrap_reference("1").unwrap();
        let update = wait_for(
            &client,
            |u| matches!(u, ClientUpdate::ReferenceBootstrap { command_id, .. } if *command_id == bootstrap),
        );
        let ClientUpdate::ReferenceBootstrap {
            snapshot,
            subsequent_events,
            ..
        } = update
        else {
            unreachable!()
        };
        assert_eq!(snapshot["revision"], "1");
        assert_eq!(subsequent_events.len(), 1);

        client.unsubscribe().unwrap();
        wait_for(
            &client,
            |u| matches!(u, ClientUpdate::Reply { op, kind: ReplyKind::Result, .. } if op == "unsubscribe"),
        );
        client
            .subscribe(
                EventCursor {
                    boot_id: "boot".into(),
                    seq: 1,
                },
                json!({"kinds":[],"targets":[]}),
            )
            .unwrap();
        wait_for(
            &client,
            |u| matches!(u, ClientUpdate::Reply { op, kind: ReplyKind::Result, .. } if op == "subscribe"),
        );
        client.unsubscribe().unwrap();
        wait_for(
            &client,
            |u| matches!(u, ClientUpdate::Reply { op, kind: ReplyKind::Result, .. } if op == "unsubscribe"),
        );
        assert!(matches!(client.try_recv(), Err(TryRecvError::Empty)));
        client.shutdown().unwrap();
        peer.join().unwrap();
    }

    #[test]
    fn disconnected_bootstrap_rejects_without_poisoning_later_reattach() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let peer = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(TEST_TIMEOUT)).unwrap();
            stream.set_write_timeout(Some(TEST_TIMEOUT)).unwrap();
            let mut first = BufReader::new(stream);
            let hello = read_request(&mut first);
            write_value(first.get_mut(), &hello_reply(&hello["msg_id"], "boot:1", 1));
            let mut eof = String::new();
            assert_eq!(first.read_line(&mut eof).unwrap(), 0);

            let (stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(TEST_TIMEOUT)).unwrap();
            stream.set_write_timeout(Some(TEST_TIMEOUT)).unwrap();
            let mut second = BufReader::new(stream);
            let hello = read_request(&mut second);
            write_value(
                second.get_mut(),
                &hello_reply(&hello["msg_id"], "boot:1", 1),
            );
            let subscribe = read_request(&mut second);
            assert_eq!(subscribe["op"], "subscribe");
            write_value(
                second.get_mut(),
                &json!({"v":1,"msg_id":subscribe["msg_id"],"type":"result",
                    "result":{"subscription":"sub-1",
                        "accepted_cursor":{"boot_id":"boot","seq":"0"}}}),
            );
            let reference = read_request(&mut second);
            assert_eq!(reference["op"], "reference");
            write_value(
                second.get_mut(),
                &json!({"v":1,"msg_id":reference["msg_id"],"type":"result",
                    "result":{"reference":"1","revision":"1","value":20.0}}),
            );
        });

        let client = ClientHandle::spawn(address).unwrap();
        client.connect(None).unwrap();
        let hello = wait_for(&client, |update| matches!(update, ClientUpdate::Hello(_)));
        let ClientUpdate::Hello(hello) = hello else {
            unreachable!()
        };
        client.disconnect().unwrap();
        wait_for(&client, |update| {
            matches!(update, ClientUpdate::State(ConnectionState::Disconnected))
        });
        let rejected = client.bootstrap_reference("1").unwrap();
        wait_for(&client, |update| {
            matches!(update, ClientUpdate::LocalRejected { command_id, reason }
                if *command_id == rejected && reason == "client_not_ready")
        });

        client.connect(Some(hello.scope)).unwrap();
        wait_for(&client, |update| matches!(update, ClientUpdate::Hello(_)));
        let bootstrap = client.bootstrap_reference("1").unwrap();
        wait_for(&client, |update| {
            matches!(update, ClientUpdate::ReferenceBootstrap { command_id, .. }
                if *command_id == bootstrap)
        });
        client.shutdown().unwrap();
        peer.join().unwrap();
    }

    #[test]
    fn public_error_and_event_gap_remain_structured_client_updates() {
        let (address, peer) = scripted_peer(|stream| {
            let mut reader = BufReader::new(stream);
            let hello = read_request(&mut reader);
            write_value(
                reader.get_mut(),
                &hello_reply(&hello["msg_id"], "boot:1", 1),
            );
            let query = read_request(&mut reader);
            write_value(
                reader.get_mut(),
                &json!({"v":1,"msg_id":query["msg_id"],"type":"error","accepted":false,
                    "code":"unknown_reference","category":"not_found","message":"unknown",
                    "retryable":false,"resync_required":false,"details":{"id":"9"}}),
            );
            let subscribe = read_request(&mut reader);
            write_value(
                reader.get_mut(),
                &json!({"v":1,"msg_id":subscribe["msg_id"],"type":"error","accepted":false,
                    "code":"event_gap","category":"resync_required","message":"gap",
                    "retryable":true,"resync_required":true,
                    "oldest":{"boot_id":"boot","seq":"2"},
                    "latest":{"boot_id":"boot","seq":"9"}}),
            );
        });
        let client = ClientHandle::spawn(address).unwrap();
        client.connect(None).unwrap();
        wait_for(&client, |u| matches!(u, ClientUpdate::Hello(_)));
        client.query("reference", json!({"reference":"9"})).unwrap();
        let error = wait_for(&client, |u| {
            matches!(
                u,
                ClientUpdate::Reply {
                    kind: ReplyKind::PublicError,
                    ..
                }
            )
        });
        let ClientUpdate::Reply { envelope, .. } = error else {
            unreachable!()
        };
        assert_eq!(envelope["code"], "unknown_reference");
        assert_eq!(envelope["details"]["id"], "9");
        client
            .subscribe(
                EventCursor {
                    boot_id: "boot".into(),
                    seq: 0,
                },
                json!({"kinds":[],"targets":[]}),
            )
            .unwrap();
        let gap = wait_for(
            &client,
            |u| matches!(u, ClientUpdate::ResnapshotRequired { reason, .. } if reason == "event_gap"),
        );
        let ClientUpdate::ResnapshotRequired {
            envelope: Some(envelope),
            connection_lost,
            ..
        } = gap
        else {
            unreachable!()
        };
        assert!(!connection_lost);
        assert_eq!(envelope["resync_required"], true);
        assert_eq!(envelope["oldest"]["seq"], "2");
        client.shutdown().unwrap();
        peer.join().unwrap();
    }

    #[test]
    fn worker_shutdown_is_finite_and_sends_no_runtime_shutdown_operation() {
        let (address, peer) = scripted_peer(|stream| {
            let mut reader = BufReader::new(stream);
            let hello = read_request(&mut reader);
            assert_eq!(hello["op"], "hello");
            write_value(
                reader.get_mut(),
                &hello_reply(&hello["msg_id"], "boot:1", 1),
            );
            let mut line = String::new();
            assert_eq!(reader.read_line(&mut line).unwrap(), 0);
        });
        let client = ClientHandle::spawn(address).unwrap();
        client.connect(None).unwrap();
        wait_for(&client, |u| matches!(u, ClientUpdate::Hello(_)));
        let started = Instant::now();
        client.shutdown().unwrap();
        assert!(started.elapsed() < WORKER_SHUTDOWN_DEADLINE);
        peer.join().unwrap();
    }

    #[test]
    fn update_queue_pressure_closes_connection_and_publishes_resnapshot_state() {
        let (address, peer) = scripted_peer(|stream| {
            let mut reader = BufReader::new(stream);
            let hello = read_request(&mut reader);
            write_value(
                reader.get_mut(),
                &hello_reply(&hello["msg_id"], "boot:1", 1),
            );
            for seq in 1..=(UPDATE_QUEUE as u64 + 8) {
                let event = json!({"v":1,"type":"event","boot_id":"boot",
                    "seq":seq.to_string(),"kind":"reference","target":{"id":"1"},
                    "data":{"revision":seq.to_string()}});
                let bytes = encode_frame(&event).unwrap();
                if bytes
                    .chunks(3)
                    .try_for_each(|chunk| reader.get_mut().write_all(chunk))
                    .is_err()
                {
                    // Queue saturation deliberately closes the transport. Depending
                    // on scheduling, the test peer observes that as EOF or reset
                    // before it has emitted every surplus event.
                    break;
                }
            }
        });
        let client = ClientHandle::spawn(address).unwrap();
        client.connect(None).unwrap();
        let pressure_deadline = Instant::now() + TEST_TIMEOUT;
        while !client.update_overflow_observed() {
            assert!(
                Instant::now() < pressure_deadline,
                "worker did not observe bounded update pressure"
            );
            thread::yield_now();
        }
        let mut resnapshot = None;
        let deadline = Instant::now() + TEST_TIMEOUT;
        while Instant::now() < deadline {
            match client.recv_timeout(Duration::from_millis(50)) {
                Ok(update) => {
                    if matches!(
                        &update,
                        ClientUpdate::ResnapshotRequired { reason, .. }
                            if reason == "ordered_update_queue_full"
                    ) {
                        resnapshot = Some(update);
                        break;
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        assert!(matches!(
            &resnapshot,
            Some(ClientUpdate::ResnapshotRequired {
                connection_lost: true,
                ..
            })
        ));
        let mut model = WorkbenchModel::new(PresentationDocument::empty("doc"));
        model.apply_client_update(ClientUpdate::Hello(hello_state(1)));
        model.complete_rebuild();
        model.apply_client_update(resnapshot.expect("pressure notification"));
        assert_eq!(model.connection, ConnectionState::Stale);
        client.shutdown().unwrap();
        peer.join().unwrap();
    }

    #[test]
    fn malformed_server_frame_and_partial_timeout_are_connection_local() {
        let (address, peer) = scripted_peer(|mut stream| {
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let _ = read_request(&mut reader);
            stream.write_all(b"not-json\n").unwrap();
        });
        let client = ClientHandle::spawn(address).unwrap();
        client.connect(None).unwrap();
        wait_for(&client, |u| {
            matches!(u, ClientUpdate::TransportFailure { .. })
        });
        client.shutdown().unwrap();
        peer.join().unwrap();

        let (address, peer) = scripted_peer(|mut stream| {
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let hello = read_request(&mut reader);
            write_value(&mut stream, &hello_reply(&hello["msg_id"], "boot:1", 1));
            stream.write_all(b"{").unwrap();
            thread::sleep(PARTIAL_FRAME_DEADLINE + Duration::from_secs(1));
        });
        let client = ClientHandle::spawn(address).unwrap();
        client.connect(None).unwrap();
        wait_for(&client, |u| matches!(u, ClientUpdate::Hello(_)));
        let failure = wait_for(&client, |u| {
            matches!(u, ClientUpdate::TransportFailure { .. })
        });
        assert!(
            matches!(failure, ClientUpdate::TransportFailure { reason } if reason.contains("partial"))
        );
        client.shutdown().unwrap();
        peer.join().unwrap();
    }
}
