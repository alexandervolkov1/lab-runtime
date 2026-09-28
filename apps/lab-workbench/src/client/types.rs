//! Bounded client-owned commands, updates, and recovery records.

use serde_json::Value;

/// Maximum Application JSON body accepted or emitted by the client.
pub(crate) const APPLICATION_JSON_LIMIT: usize = 16_383;
/// Maximum LF-terminated TCP NDJSON frame.
pub(crate) const FRAME_LIMIT: usize = 16_384;
/// Bounded caller-to-worker command mailbox.
pub(crate) const COMMAND_QUEUE: usize = 32;
/// Maximum exchanges awaiting terminal correlation on one connection.
pub(crate) const MAX_IN_FLIGHT: usize = 8;
/// Bounded ordered worker-to-caller update mailbox.
pub(crate) const UPDATE_QUEUE: usize = 64;

/// One process/Application event cursor, never a connection token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EventCursor {
    pub(crate) boot_id: String,
    pub(crate) seq: u64,
}

impl EventCursor {
    pub(crate) fn to_json(&self) -> Value {
        serde_json::json!({"boot_id":self.boot_id,"seq":self.seq.to_string()})
    }
}

/// Retained mutation/deduplication identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MutationIdentity {
    pub(crate) scope: String,
    pub(crate) seq: u64,
}

impl MutationIdentity {
    pub(crate) fn to_json(&self) -> Value {
        serde_json::json!({"scope":self.scope,"seq":self.seq.to_string()})
    }
}

/// Last authoritative admission knowledge retained for reconciliation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum KnownAdmission {
    Pending,
    Accepted,
    Ambiguous,
    Completed,
    Failed,
}

/// Bounded in-memory precursor of the M14.3 recovery journal.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RecoveryRecord {
    pub(crate) boot_id: String,
    pub(crate) identity: MutationIdentity,
    pub(crate) op: String,
    pub(crate) args: Value,
    pub(crate) admission: KnownAdmission,
}

/// Runtime-advertised hello/session data consumed by the client owner.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct HelloState {
    pub(crate) boot_id: String,
    pub(crate) scope: String,
    pub(crate) next_seq: u64,
    pub(crate) operations: Vec<String>,
    pub(crate) capabilities: Value,
    pub(crate) limits: Value,
    pub(crate) event_oldest: EventCursor,
    pub(crate) event_latest: EventCursor,
}

/// Observable worker lifecycle; none of these states is Runtime authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConnectionState {
    Disconnected,
    Connecting,
    AwaitingHello,
    Reattaching,
    Ready,
    Stale,
    Stopping,
    Stopped,
}

/// Internal commands accepted by the single Application client owner.
#[derive(Clone, Debug)]
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "M14.2 client commands precede the M14 GUI consumer"
    )
)]
pub(crate) enum ClientCommand {
    Connect {
        command_id: u64,
        scope: Option<String>,
    },
    Disconnect,
    Query {
        command_id: u64,
        op: String,
        args: Value,
    },
    Mutation {
        command_id: u64,
        op: String,
        args: Value,
    },
    RetryMutation {
        command_id: u64,
        identity: MutationIdentity,
    },
    OperationStatus {
        command_id: u64,
        identity: MutationIdentity,
    },
    Subscribe {
        command_id: u64,
        after: EventCursor,
        filter: Value,
    },
    Unsubscribe {
        command_id: u64,
    },
    BootstrapReference {
        command_id: u64,
        reference: String,
    },
    ShutdownWorker,
}

/// Semantic class of one correlated Application response.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReplyKind {
    Result,
    PublicError,
    MutationAccepted,
    MutationCompleted,
    MutationFailed,
}

/// Ordered worker output. Raw bounded envelopes remain available for exact errors.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ClientUpdate {
    State(ConnectionState),
    Hello(HelloState),
    Reply {
        command_id: u64,
        msg_id: String,
        op: String,
        kind: ReplyKind,
        envelope: Value,
        recovery: Option<RecoveryRecord>,
    },
    Event {
        cursor: EventCursor,
        envelope: Value,
    },
    SubscriptionProgress(Value),
    ReferenceBootstrap {
        command_id: u64,
        snapshot: Value,
        subsequent_events: Vec<Value>,
    },
    ReconciliationRequired {
        records: Vec<RecoveryRecord>,
    },
    ResnapshotRequired {
        reason: String,
        envelope: Option<Value>,
    },
    LocalRejected {
        command_id: u64,
        reason: String,
    },
    TransportFailure {
        reason: String,
    },
    WorkerStopped,
}

/// Failure to submit a local command without blocking.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CommandSendError {
    Busy,
    WorkerStopped,
    IdExhausted,
}

impl std::fmt::Display for CommandSendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Busy => "client command queue is full",
            Self::WorkerStopped => "client worker has stopped",
            Self::IdExhausted => "client command identity exhausted",
        })
    }
}

impl std::error::Error for CommandSendError {}
