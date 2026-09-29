# Application API

Applicable to Application protocol v1 and the current repository/v0.1 product.

The Application API is the supported language-neutral boundary for observing and
operating Runtime. It exposes laboratory semantics without exposing Rust internals,
transport handles, SQLite internals, or GUI presentation state.

## Reference map

- [Protocol and sessions](protocol-and-sessions.md): transports, envelopes, hello,
  scopes, IDs, and versioning.
- [Operations](operations.md): the exact 42-operation registry, availability, and 25
  capabilities.
- [Events, mutations, and recovery](events-mutations-and-recovery.md): subscriptions,
  frozen projections, operation lifecycle, sequencing, and deduplication.
- [Errors and limits](errors-and-limits.md): stable public errors and canonical bounds.
- [System architecture](../architecture.md): Runtime/Workbench ownership and process
  lifetime.

## Purpose and transports

One production Application and SessionStore serve both current transports:

- IPv4-loopback TCP with one UTF-8 JSON object per LF-terminated NDJSON frame;
- optional loopback WebSocket with one Application JSON object per text message.

The WebSocket endpoint is `/application/v1`, requires subprotocol
`lab-runtime.application.v1`, and checks an exact configured Origin allowlist. Both
transports share one bounded client-capacity model. Once a valid transport message
reaches the common Application JSON codec, they share DTOs, operations, sessions,
deduplication, events, outcomes, and semantic public-error mappings. TCP framing and
HTTP/WebSocket Upgrade, binary/control-message, UTF-8, and protocol-close failures
remain transport-specific and need not produce an Application error envelope.

Native Workbench is one TCP client. The API is also intended for advanced external
clients; it is not private Workbench protocol.

## Version and first request

The current identifiers are:

| Field | Value |
|---|---|
| protocol ID | `lab-runtime.application` |
| protocol version | `1` |
| Application API version | `0.1-pre` |

The first request on every connection must be `hello`. It creates a scope when
`scope:null` is supplied or attempts to attach an existing process-local scope.
The response negotiates the active composition and returns:

- Runtime `boot_id`;
- authoritative `scope` and mutation `next_seq`;
- supported `operations` and structured `capabilities`;
- current `limits`;
- `event_oldest` and `event_latest` cursors.

Clients must consume these values. A compiled-in registry must not override what the
connected Runtime advertises.

## Queries and mutations

A query has connection-local `msg_id` correlation and returns one result or
synchronous error. It has no mutation `request_id`.

A mutation additionally carries `request_id:{scope,seq}`. Runtime admits the
normalized typed mutation into its process-local SessionStore before domain
execution, reports `accepted`, and later retains `completed` or `failed` within
fixed limits. `msg_id` and `request_id` are different identity domains.

Mutation retry is never automatic protocol behavior. Exact resubmission, when a
client deliberately chooses it, uses the same request ID, operation, and normalized
typed arguments.

## Events and snapshots

Clients obtain current state through bounded queries/frozen pages and then create at
most one aggregate connection-local subscription. Event cursors support replay and
catch-up while retained. An `event_gap` invalidates incremental continuity and
requires a new authoritative snapshot/rebuild; events never replace snapshot
authority.

## Operation outcomes and state

`accepted` means Runtime admitted an exact identity and payload. `completed` and
`failed` are retained terminal operation outcomes. Two different wire forms use the
text `outcome_unknown`:

- an `operation_status` result with `state:"outcome_unknown"` means only that Runtime
  currently has no retained operation-state evidence for that exact identity; the
  status lookup does not establish prior admission or compare the sequence with the
  high-water mark;
- a mutation submission rejected with public error `code:"outcome_unknown"` means
  the submitted sequence is already at or below the scope high-water mark but has no
  retained record, so Runtime will not execute that old identity again.

Neither form means Completed or Failed, proves physical state, or permits blind
retry. Status checks and exact resubmission remain deliberate client actions, never
automatic protocol behavior.

Operation outcome is not current physical state. ACK, readback, and physical effect
remain separate, and Fresh observations do not by themselves reconcile mutation
evidence.

For Recorder durability and output evidence, see
[Recorder and SQLite](../recorder-sqlite.md) and
[Safety and failure behavior](../safety-and-failures.md).
