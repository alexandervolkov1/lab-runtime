# M12.2 transport-neutral server seam implementation report

## 1. Status and scope

M12.2 is a behavior-preserving refactor of the accepted TCP/NDJSON server. It
separates Application JSON from NDJSON framing and makes bounded connection/delivery
ownership concrete. TCP/NDJSON remains the only transport.

```text
M12.1 WebSocket architecture audit: ACCEPTED
M12.2 transport-neutral server seam: READY FOR EXTERNAL REVIEW
M12.3 bounded WebSocket/JSON transport: NOT AUTHORIZED / NOT STARTED
```

The accepted public contract remains protocol version 1, 42 operations and 25
capabilities. No dependency, listener, endpoint, operation, DTO, public error,
session rule, Runtime behavior, Recorder behavior, SQLite schema, output-authority
rule, scripting feature, or presentation concept was added or changed.

## 2. Files and modules changed

| File | Change |
|---|---|
| `apps/lab-runtime/src/wire.rs` | Adds the shared bounded Application JSON codec and leaves NDJSON framing as a thin compatibility adapter. |
| `apps/lab-runtime/src/events.rs` | Measures semantic event size with the shared JSON encoder instead of an NDJSON frame. |
| `apps/lab-runtime/src/server/coordination.rs` | Adds concrete transport-neutral connection admission, per-client delivery, and serialized-owner scheduling state. |
| `apps/lab-runtime/src/server.rs` | Composes the existing TCP reactor with the shared coordination state; TCP retains sockets, stream framing and partial writes. |
| `apps/lab-runtime/src/lib.rs` | Updates module navigation text to name both Application JSON and NDJSON framing. |
| `apps/lab-runtime/tests/api_protocol.rs` | Adds focused shared-codec, exact-bound and compatibility coverage. |

Coordination files identify this report and the external-review gate. No Cargo file
changed and no dependency was added.

## 3. Implemented architecture

```text
                         one process-global ConnectionCoordinator
                                      (capacity 8, IDs, detach fence)
                                                   |
TCP listener -> TcpStream/NDJSON Peer -> ClientDelivery -> bounded channels
                    |                    (msg IDs,       |
                    |                     reply/event     v
                    |                     queues)      OwnerDelivery
                    |                                  (fair request queues,
                    |                                   close lifecycle)
                    |                                         |
                    +-> decode_ndjson_frame                    v
                          |                         ONE serialized Application
                          v                              / ONE SessionStore
                 decode_application_json                         |
                                                                v
                                                    ServiceHost / HostCore / Runtime

Application response/event Value
          |
          v
encode_application_json -> ClientDelivery -> TCP Peer appends LF -> partial write
```

There remains exactly one `Application` value, created and called only by the
serialized owner loop in `server::run`. There is no `Application` mutex, second
owner loop, second `SessionStore`, generic transport trait, dynamic registry, async
runtime, or per-client thread.

## 4. Old and new ownership

| Responsibility | Before M12.2 | After M12.2 | Blocking/overflow behavior |
|---|---|---|---|
| Process connection IDs | Local `next_id` in TCP reactor | `ConnectionCoordinator` | Checked `u64`; exhaustion is an explicit reactor error; IDs never wrap or reuse. |
| Global client admission | `peers + pending_detach` in TCP reactor | `ConnectionCoordinator.active + pending_detach` | One eight-slot pool; a pending detach retains its slot until owner delivery. |
| Detach generation fence | TCP reactor `VecDeque<u64>` | `ConnectionCoordinator` | Bounded by the eight admitted slots; nonblocking `try_send`; exactly-once enqueue. |
| Per-connection pending count and `msg_id` set | Fields on TCP `Peer` | `ClientDelivery` | Eight pending requests; duplicate in-flight ID closes only that client with the existing error. |
| Reply/event queues and alternation | Fields/methods on TCP `Peer` | `ClientDelivery` | Reply 8, event 16; overflow detaches the affected connection. |
| Malformed/duplicate rejection staging | TCP `Peer` | `ClientDelivery`, requested by TCP decode path | One bounded rejection waits for prior exchanges, then the connection closes. |
| Fair owner request queues | Local maps/sets/counter in `server::run` | `OwnerDelivery` | Eight per client; at most one request from up to four rotated clients per owner turn. |
| Application detach/transport close lifecycle | Repeated inline map/set changes | `OwnerDelivery` | Forced close detaches Application once; later network detach only retires coordination state. |
| Application calls | `server::run` | Unchanged: `server::run` | One serialized call lane after required HostCore service. |
| Socket read/write and deadlines | TCP `Peer` | Unchanged: TCP `Peer` | Nonblocking; 8,192-byte sweep; two-second finite deadlines. |
| NDJSON delimiter and CRLF handling | `wire` plus TCP buffers | `decode_ndjson_frame`; TCP appends LF on output | Complete frames remain limited to 16,384 bytes. |
| Semantic JSON validation/encoding | Embedded in frame functions | `decode_application_json` / `encode_application_json` | Message bodies remain limited to 16,383 bytes. |

The new coordination module contains no `TcpListener`, `TcpStream`, LF/CRLF rule,
socket error, EOF rule, or partial-write byte offset. It is a small concrete seam,
not a general transport framework.

## 5. Mutable-state and thread ownership

### Serialized Application owner thread

`server::run` exclusively owns:

- the one `Application` and its one `SessionStore`;
- `ServiceHost`, `HostCore`, Runtime and Recorder coordination;
- `OwnerDelivery` request queues, fairness rotation and close state;
- Application attach/handle, detach, subscription pumping, history/recording
  completions and terminal shutdown replies.

It services `HostCore` before bounded client work. It never waits for a network
client and uses nonblocking `try_send` on the 64-entry owner-to-reactor mailbox.

### One network reactor thread

The existing reactor exclusively owns:

- the loopback `TcpListener` clone and every `TcpStream`;
- `ConnectionCoordinator`, shared across all connections admitted by that reactor;
- each TCP `Peer`, including input buffer, partial-frame time, handshake time,
  current partial write and last-write progress time;
- each peer's transport-neutral `ClientDelivery` state.

The reactor uses nonblocking accept/read/write and nonblocking mailbox operations.
It may sleep for the existing five-millisecond polling interval. It never borrows or
locks Runtime/Application state.

### Other workers

Recorder, serial and managed-component worker ownership did not change. M12.2 does
not change who may block in those subsystems. Network clients still cannot block
required Runtime safety, control or Recorder owner progress.

## 6. Preserved bounds and overflow policy

| Bound | Value | Owner and policy after M12.2 |
|---|---:|---|
| Global network clients | 8 | `ConnectionCoordinator`; active plus detach-pending generations share one pool. Future transports must join this same pool. |
| NDJSON complete frame | 16,384 bytes including LF | TCP input/framing; oversize closes the offending peer under the existing behavior. |
| Application JSON body | 16,383 bytes | Shared codec; encode/decode fail with the existing `frame_too_large` taxonomy. |
| TCP input allocation | 16,384 bytes | `Peer`; one connection-local buffer. |
| Owner/reactor channel, each direction | 64 messages | Bounded `sync_channel`; all pressure paths use `try_send`. |
| Requests admitted per connection | 8 | `ClientDelivery` plus `OwnerDelivery`; TCP stops reading at the bound. |
| Reply messages per connection | 8 | `ClientDelivery`; overflow detaches that connection. |
| Event messages per connection | 16 | `ClientDelivery`; overflow detaches that connection. |
| Reactor byte work per pass | 8,192 bytes | TCP `Peer`; partial writes resume at their exact byte offset. |
| Frames dispatched/written per peer pass | 4 | TCP `Peer`; unchanged. |
| Clients scheduled per owner turn | 4 | `OwnerDelivery` rotating fairness; unchanged. |
| Incoming owner messages per turn | 16 | Serialized owner loop; unchanged. |
| Reactor owner messages per pass | 64 | Reactor loop; unchanged. |
| Hello/partial/write progress deadline | 2 seconds | TCP `Peer`; absolute connection-local deadline; unchanged. |
| Event semantic encoded size | 4,096 JSON bytes | `EventLog`, now using shared JSON encoding; same effective acceptance as the former `frame length <= 4,097` check. |
| Event replay ring | 1,024 records | `EventLog`; unchanged. |

Adding another listener in a future milestone must not instantiate another
`ConnectionCoordinator` or another eight-client pool. The accepted contract remains
`TCP + future WebSocket <= 8`.

## 7. Application JSON / NDJSON separation

`decode_application_json(bytes)` now owns all transport-neutral request behavior:

- the 16,383-byte body bound;
- UTF-8 validation;
- lexical depth/value bounds;
- duplicate-key rejection at every depth;
- string/key bounds;
- complete JSON and trailing-data rejection;
- root object and request-envelope validation;
- protocol version, `msg_id`, operation lookup and strict arguments;
- mutation/query `request_id` rules and canonical sequence parsing.

`decode_ndjson_frame(bytes)` owns only the 16,384-byte complete-frame bound, required
LF and optional CR removal, then delegates to `decode_application_json`.
`decode_frame` remains as the accepted compatibility entry point.

`encode_application_json(Value)` uses the same hard-cap incremental writer as before
but returns the delimiter-free body. `encode_ndjson_frame` appends LF and
`encode_frame` remains the compatibility entry point. JSON map ordering was not
made canonical and no new byte-order contract was introduced.

The owner/reactor channel now carries bounded delimiter-free Application JSON
messages. The TCP peer appends LF only when a message becomes its current write.
Thus shared queues and scheduling do not know NDJSON framing or TCP partial offsets.

`EventLog` now asks `encode_application_json` for its semantic size. The old check
accepted at most 4,096 JSON bytes plus one LF; the new check accepts at most 4,096
JSON bytes directly. This removes the NDJSON dependency without changing the set of
admitted semantic events or weakening fail-closed oversized-event handling.

## 8. Connection and delivery behavior

`ConnectionCoordinator` allocates opaque process-local IDs from one checked `u64`
sequence. An ID becomes active only after capacity admission. Closing removes the
active generation exactly once and moves it into the bounded detach-pending FIFO.
Its capacity slot is not released until the detach reaches the Application owner.
An unknown or repeated close cannot remove a different active generation.

`ClientDelivery` owns transport-neutral per-connection exchange state: admitted
request count, in-flight `msg_id` set, hello completion, reply/event queues,
reply/event alternation, bounded rejection staging and closing state. TCP owns only
how those messages enter or leave a byte stream.

`OwnerDelivery` retains the exact existing ordered-map rotation: it snapshots
connection IDs, considers at most four per turn, removes at most one request from
each considered connection, then advances rotation by one. It also fences forced
Application detach from the later network detach, so a forced close cannot invoke
`Application::detach` twice.

There is still one bounded path to `Application::handle`, one path to session state,
and one event/history/subscription pump. M12.2 creates no alternative Application
entry point.

## 9. TCP compatibility and listener ownership

TCP retains:

- the existing `127.0.0.1` listener and port selection owned by `ServiceHost`;
- listener clone/startup ordering and readiness JSON;
- nonblocking accept and `TcpStream` ownership;
- LF/CRLF stream framing and coalesced/fragmented input handling;
- input buffering, socket/EOF errors, partial output offsets and slow-peer deadlines;
- the same close-after-bounded-rejection behavior.

The M12.1 layering debt—`ServiceHost` owns the TCP listener—remains intentionally
unchanged. Moving it is unnecessary for the proven codec/delivery seam and would
expand startup/readiness risk. M12.3 must revisit only the minimum listener
coordination necessary to add its first real second transport; it must preserve
startup failure ordering and must not introduce a generic endpoint framework.

## 10. Proof of unchanged semantics

Source structure and regression tests establish:

- the operation registry, 42 operations and 25 capabilities were not edited;
- `Application`, `SessionStore`, hello/scope attach, deduplication, subscription,
  replay/gap and history code were not edited;
- existing TCP callers continue to use `decode_frame`/`encode_frame` unchanged;
- exact 16,383-byte Application bodies and 16,384-byte LF frames are accepted;
- 16,384-byte Application bodies are rejected;
- LF and CRLF decode identically, while malformed UTF-8, duplicate keys, depth,
  value, operation/argument and request-ID failures retain their codes;
- in-flight `msg_id`, slow reader/writer, partial write, mailbox saturation, event
  overflow, stale-generation and eight-client churn tests continue through TCP;
- global capacity remains eight, the ninth client is rejected, and a detach-pending
  generation does not release capacity early;
- checked-ID, exact capacity-release, fairness and exactly-once close/detach state
  have focused unit coverage;
- Runtime shutdown and native/Recorder progress still use the existing serialized
  owner and finite shutdown path.

## 11. Verification

The final M12.2 gate consists of:

```text
cargo fmt --all -- --check
cargo test --workspace --locked
cargo test --workspace --release --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
git diff --check
```

All five commands pass. Focused iteration also passed:

```text
api_protocol                       6 passed
client_isolation                   8 passed
lab-runtime library/server        67 passed, 1 ignored
```

The complete debug and release workspace runs passed all default tests. They include
the unchanged `api_protocol`, `client_isolation`, `request_deduplication`,
`subscription_recovery`, `recorder_history_api`, `runtime_events`,
`runtime_shutdown`, server bounded-peer tests, and all other workspace regression
suites.

Exactly two pre-existing opt-in workloads remain ignored in each default workspace
run:

- diagnostic production rotation beyond the 16 MiB retention window;
- the 2,000-turn/eight-recording-cycle developer-preview soak.

Neither ignored test nor its annotation was changed for M12.2.

## 12. Shutdown and failure behavior

External stop and accepted Application shutdown still request Runtime-owned safe
shutdown first, stop accepting through the bounded owner/reactor channel, allow the
existing terminal-delivery interval, stop the reactor and join its one thread.
Terminal responses remain best effort and bounded. A reactor panic/error or closed
owner channel remains a fatal process shutdown input; it is never reported as safe
success.

Malformed input, duplicate in-flight IDs, input/output/event pressure, deadline,
EOF and socket failures remain scoped to the affected client. Owner/reactor mailbox
pressure never blocks; it closes/detaches the affected client under the existing
rules. Detach retains capacity until owner notification, preventing a stale
generation from crossing into a replacement connection.

## 13. Risks and deferred questions

1. `ServiceHost` still owns the sole TCP listener. This is documented layering debt,
   not a current correctness defect. The first WebSocket listener will prove the
   minimum startup/readiness extraction in M12.3.
2. `ConnectionCoordinator` currently lives on the one network reactor thread.
   M12.3 must join that coordinator rather than create a second reactor-side pool.
3. Shared outbound messages are encoded JSON byte bodies. M12.3 must use them as one
   WebSocket text message and apply its own lower-level frame/message limits without
   changing the 16,383-byte semantic bound.
4. The browser Origin/Upgrade policy remains design-only from M12.1. No HTTP or
   WebSocket security surface exists in M12.2.
5. The synchronous/nonblocking WebSocket library decision remains deferred. M12.2
   adds no dependency and reveals no need for an async runtime.

No contradiction was found in Application ownership, session lifetime,
deduplication, backpressure, Runtime progress, shutdown or output safety.

## 14. Explicitly preserved invariants

```text
Runtime owns experiment semantics.
Client owns presentation semantics.

client lifetime != experiment lifetime
script lifetime != experiment lifetime

ONE Application API
ONE serialized Application owner
ONE operation registry
ONE DTO model
ONE SessionStore / session semantics
ONE subscription semantics
ONE dedup semantics
ONE error model
ONE history semantics

TCP + future WebSocket clients <= 8
bounded queues / histories / workers
network clients do not block Runtime safety progress
WebSocket disconnect does not own experiment lifetime
no raw transport ownership escapes Runtime
no client may fabricate physical evidence
no UI or scripting semantics enter Runtime core
```

M12.3 has not started and remains unauthorized pending external review.

STATUS: M12_2_READY_FOR_EXTERNAL_REVIEW
