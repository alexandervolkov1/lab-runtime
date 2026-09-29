# Protocol and sessions

Applicable to Application protocol v1 and the current repository/v0.1 product.

## Transport-independent message model

Application semantics operate on one bounded JSON object. TCP adds LF-terminated
NDJSON framing; WebSocket carries the same object as one text message. Once a valid
transport message reaches the common Application JSON codec, transport choice does
not change DTO validation, IDs, operations, sessions, deduplication, events, outcomes,
or semantic public-error mappings. TCP framing and HTTP/WebSocket Upgrade,
binary/control-message, WebSocket UTF-8, and protocol-close failures are
transport-specific and may occur before any Application envelope exists.

| Identifier | Current value |
|---|---|
| protocol ID | `lab-runtime.application` |
| protocol version | `1` |
| Application API version | `0.1-pre` |

Requests use strict field allowlists and reject duplicate JSON keys. IDs and u64
counters are canonical decimal strings where documented; callers must not send them
as precision-losing JSON numbers.

## Request envelopes

Query:

```json
{"v":1,"msg_id":"q-1","op":"recording_status","args":{}}
```

Mutation:

```json
{"v":1,"msg_id":"m-1","op":"reference_retune",
 "request_id":{"scope":"<server scope>","seq":"1"},
 "args":{"reference":"1","expected_revision":"1","target":42.0,"rate":2.0}}
```

| Field | Meaning |
|---|---|
| `v` | protocol version; exactly 1 |
| `msg_id` | nonempty connection-local correlation ID, at most 64 UTF-8 bytes |
| `op` | operation name advertised by hello |
| `args` | strict object using that operation's allowlisted fields |
| `request_id` | required for mutations and forbidden for known queries |

`msg_id` and `request_id` are different identity domains:

```text
msg_id
    one connection exchange; may change on an exact resubmission

request_id = {scope, seq}
    process-local retained mutation identity; does not change on exact resubmission
```

## Response and event envelopes

Query result:

```json
{"v":1,"msg_id":"q-1","type":"result","result":{}}
```

Synchronous rejection:

```json
{"v":1,"msg_id":"q-1","type":"error","accepted":false,
 "code":"invalid_args","category":"invalid_request",
 "message":"The request does not satisfy the bounded protocol schema.",
 "retryable":false,"resync_required":false}
```

Mutation lifecycle:

```json
{"v":1,"msg_id":"m-1","type":"operation",
 "request_id":{"scope":"<scope>","seq":"1"},"state":"accepted"}
```

```json
{"v":1,"msg_id":"m-1","type":"operation",
 "request_id":{"scope":"<scope>","seq":"1"},"state":"completed","result":{}}
```

A terminal failed operation uses `state:"failed"` plus the same bounded public error
fields and may include a bounded result/evidence object.

Events are not correlated by `msg_id`:

```json
{"v":1,"type":"event","boot_id":"<boot>","seq":"17",
 "published_at":"123456789","kind":"reference","target":{"id":"1"},
 "data":{},"request_id":{"scope":"<scope>","seq":"1"}}
```

`request_id` on an event is optional mutation cause, not proof that every changed
fact has that cause. A filtered scan may instead emit `type:"subscription_progress"`
with subscription, boot, and sequence so a client can advance its applied cursor.

## Hello-first rule

The first request on a connection must be:

```json
{"v":1,"msg_id":"hello-1","op":"hello","args":{"scope":null}}
```

No other request is valid before successful hello, and hello cannot be repeated on
the same attached connection. The result is the authoritative negotiation response:

| Field | Meaning |
|---|---|
| `boot_id` | current Runtime process identity |
| `scope` | server-issued process-local logical client scope |
| `next_seq` | exact next admissible mutation sequence for this scope |
| `operations` | operations available in the active composition |
| `capabilities` | structured versioned discovery labels |
| `limits` | actual advertised bounds for this Runtime build |
| `event_oldest` | oldest cursor from which complete replay remains possible |
| `event_latest` | freshest committed event cursor |
| `protocol` / `application` | protocol and package/API version identity |

Clients must consume this response. Static client knowledge is useful for typed DTOs,
but it must not override the connected Runtime's advertised operation list or limits.

## New and retained scopes

`scope:null` requests a new server-issued scope. Runtime does not accept a
client-invented mutation scope.

Supplying a retained scope attempts process-local reattachment:

```json
{"v":1,"msg_id":"hello-2","op":"hello","args":{"scope":"<retained scope>"}}
```

On success, hello returns the same scope and its current `next_seq`. Retained
terminal outcomes and pending identities remain governed by the SessionStore, not by
the old socket.

Relevant hello/session failures are:

- `scope_in_use`: another live connection owns the retained scope;
- `scope_unknown`: the scope was never issued here, expired, or was evicted;
- `instance_changed`: the scope belongs to a different Runtime boot.

They do not authorize allocating a replacement identity for old work. See
[Events, mutations, and recovery](events-mutations-and-recovery.md).

## Mutation sequence

For a new mutation:

```text
request_id.scope == hello.scope
request_id.seq   == hello.next_seq
```

The sequence is consecutive and Runtime-owned. Successful first admission advances
the scope high-water mark. A capacity rejection before admission does not advance it.
Terminal-outcome eviction never lowers it.

An old sequence is handled by retained deduplication rules; a future sequence produces
`sequence_gap`. Clients must not repair a gap by guessing.

## Two `outcome_unknown` forms

`operation_status` returns a normal query result. When SessionStore has no retained
record for the supplied exact identity, its result is:

```json
{"state":"outcome_unknown"}
```

That status lookup checks retained records only. It does not compare `seq` with the
scope high-water mark, so the result does not prove that the identity was admitted,
executed, not executed, or already past.

A mutation submission uses the deduplication admission path. If it supplies a
sequence at or below the high-water mark and no retained record exists, Runtime
returns a synchronous error envelope with `code:"outcome_unknown"`. That old
sequence will not be executed again. This is distinct from the status result above.

Neither form is Completed or Failed, proves physical state, or grants blind-retry
authority. `operation_status` and exact resubmission are explicit client actions;
the protocol performs neither automatically.

## Process-local lifetime

Scopes, deduplication, and retained outcomes are process-local and bounded. They
survive connection replacement while retained, but not Runtime boot replacement.
This is not a cross-process exactly-once guarantee.

See [Operations](operations.md), [Events and recovery](events-mutations-and-recovery.md),
and [Errors and limits](errors-and-limits.md).
