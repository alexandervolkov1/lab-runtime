# M12.3 bounded WebSocket/JSON transport implementation report

## 1. Status and scope

M12.3 adds one optional IPv4-loopback WebSocket/JSON endpoint to the accepted M12.2
server seam. TCP/NDJSON and WebSocket/JSON share the same coordinator, delivery
queues, Application JSON codec, serialized `Application`, `SessionStore`, operation
registry, DTOs, errors, subscriptions, deduplication and history behavior.

```text
M12.1 WebSocket architecture audit: ACCEPTED
M12.2 transport-neutral server seam: ACCEPTED
M12.3 bounded WebSocket/JSON transport: READY FOR EXTERNAL REVIEW
M12.4 transport parity/fault acceptance: NOT AUTHORIZED / NOT STARTED
```

No Application operation, capability, DTO, public error, session rule, Runtime,
Recorder, SQLite, controller, output-authority, scripting or presentation semantic
changed. The registry remains 42 operations and 25 capabilities.

## External-review remediation

The first M12.3 external review accepted the architecture but requested three
focused transport corrections. This section records those findings after the
original implementation rather than rewriting them as pre-review knowledge.

### Red reproductions and root causes

1. A deterministic module test filled the eight-reply shared queue, moved one reply
   into Tungstenite as accepted-but-unflushed work, and showed that the old
   `None` current-write accounting incorrectly admitted a ninth reply. The same
   defect applied to the sixteen-event queue. The root cause was clearing shared
   delivery identity on Tungstenite `write`, before successful transport flush.
2. A raw client coalesced a non-final Text frame and an interleaved Ping in one TCP
   write after a normal hello. After excluding socket read timeout from the meaning
   of "closed", the test remained connected beyond the absolute two-second
   incomplete-message deadline while a healthy client progressed. Tungstenite had
   retained the fragment internally and returned Ping, so the prior
   `WouldBlock`-only timer did not see the incomplete message. A separate idle-Ping
   regression passed and established that ordinary control traffic must not start
   this deadline.
3. Raw Upgrade regressions for HTTP/1.0, missing/wrong `Connection`, missing/wrong
   `Upgrade`, wrong version, missing/invalid key and duplicate Host all passed before
   production changes. Tungstenite 0.30.0 already rejects the delegated RFC 6455
   cases, so project code did not duplicate those checks.

### Production corrections

`WebSocketPeer` now retains `Option<bool>` current-Application-write identity,
equivalent to the TCP peer's reply/event identity. An accepted frame stays counted
in `queued`, `push_reply`, `push_event` and `stage_rejection`; no later Application
message is dequeued while it is current. Only successful Tungstenite `flush` clears
the identity and advances reply/event alternation. A `WouldBlock` never retries the
body and never frees its shared capacity slot. This adds no queue and does not
change `CLIENT_OUT = 8`, `CLIENT_EVENTS = 16` or the 32,768-byte Tungstenite write
buffer bound.

Tungstenite does not expose its internal incomplete-message flag. The bounded stream
therefore maintains a deadline-only frame-progress observer using Tungstenite's own
public `FrameHeader::parse`: at most 14 header bytes, one `u64` remaining-payload
counter and one timestamp. Completing a non-final Text/Binary frame starts the
absolute timer; control frames do not change it; a final Continuation clears it.
The observer never copies payloads, unmasks, validates, reassembles or dispatches
messages. Any observer parse failure retires the observer and leaves the actual
protocol decision to Tungstenite. Thus standalone Ping/Pong cannot be confused with
an incomplete Application message, while interleaved control traffic cannot reset a
real fragment deadline.

## 2. Files and modules changed

| File | Change |
|---|---|
| `apps/lab-runtime/Cargo.toml`, `Cargo.lock` | Pin synchronous `tungstenite` 0.30.0 with only its `handshake` feature. |
| `apps/lab-runtime/src/websocket.rs` | Fixed endpoint constants, explicit bounds, exact-Origin configuration and validation. |
| `apps/lab-runtime/src/server/websocket_peer.rs` | Bounded nonblocking Upgrade, WebSocket framing/control/close state and shared-delivery adapter. |
| `apps/lab-runtime/src/server.rs` | Adds the optional listener to the existing reactor and the one global coordinator. |
| `apps/lab-runtime/src/configuration.rs` | Adds optional deployment-only `[server.websocket]` configuration. |
| `apps/lab-runtime/src/service.rs` | Binds the optional loopback listener and publishes additive readiness discovery. |
| `apps/lab-runtime/src/lib.rs` | Exposes the endpoint constants/options module. |
| `apps/lab-runtime/tests/websocket_transport.rs` | Focused browser policy, framing, bounds, capacity, deadline and isolation acceptance. |
| `apps/lab-runtime/tests/configuration_validation.rs` | Fail-closed WebSocket deployment configuration coverage. |
| `apps/lab-runtime/tests/runtime_startup.rs` | CLI validation, loopback binding and additive readiness coverage. |

## 3. Implemented architecture

```text
TCP listener -> TcpStream / NDJSON Peer ---------+
                                                  |
WS listener -> bounded Upgrade / WebSocketPeer --+--> ONE ConnectionCoordinator
                                                  |       IDs / capacity / detach
                                                  v
                                            ClientDelivery
                                      msg IDs / replies / events
                                                  |
                                      bounded sync channels (64)
                                                  |
                                                  v
                                            OwnerDelivery
                                                  |
                                                  v
                                      ONE serialized Application
                                           ONE SessionStore
                                                  |
                                                  v
                                  ServiceHost / HostCore / Runtime
```

Both listeners are cloned into the existing single network reactor. There is no
second reactor, `Application`, `SessionStore`, owner loop, capacity pool, delivery
queue, async runtime, per-client thread, Application mutex or generic transport
framework.

## 4. Dependency review

The exact dependency is:

```toml
tungstenite = { version = "=0.30.0", default-features = false, features = ["handshake"] }
```

Review findings:

- version 0.30.0 is the current stable release inspected for this implementation;
- license is `MIT OR Apache-2.0`;
- declared MSRV is Rust 1.85; the verified workspace toolchain is Rust 1.95;
- `handshake` enables only `data-encoding`, `http`, `httparse` and `sha1` features;
- URL helpers, native TLS, Rustls and compression are not enabled; Tokio,
  tokio-tungstenite and Axum are absent;
- the lockfile additions are `bytes`, `chacha20`, `data-encoding`, `getrandom` 0.4,
  `http`, `httparse`, `log`, `r-efi` 6, `rand`, `rand_core`, `sha1` and
  `tungstenite`; their declared licenses offer MIT and/or Apache-2.0 terms (the
  `r-efi` declaration additionally offers LGPL as an alternative);
- `ServerHandshake::start` plus `MidHandshake` preserves interrupted nonblocking
  handshakes instead of requiring a blocking accept call;
- the known RustSec handshake denial-of-service advisory RUSTSEC-2023-0065 affects
  releases through 0.20.0 and is patched from 0.20.1, so it does not include 0.30.0;
- no `cargo audit` executable is installed in the environment; the advisory review
  used the current RustSec entry and the exact locked dependency graph.

Library defaults were not accepted. The adapter supplies every relevant
`WebSocketConfig` bound and wraps the handshake parser with stricter project-owned
limits.

## 5. Endpoint, configuration and browser policy

The endpoint is optional and always binds a separate IPv4 loopback listener:

```text
ws://127.0.0.1:<ws-port>/application/v1
subprotocol: lab-runtime.application.v1
```

Deployment configuration is nested under `[server.websocket]` with `enabled`,
`port` and `allowed_origins`. The legacy virtual-profile CLI accepts an optional
`--ws-port` followed by one or more `--ws-origin` values. Configuration never enters
Application DTOs.

Startup rejects an enabled endpoint without origins, wildcard/invalid/non-canonical
origins, missing explicit origin ports, duplicates, more than 16 origins, retained
settings while disabled, and unknown endpoint fields such as `host` or `path`.
Binding is hard-coded to `127.0.0.1`; no configuration can request a wildcard or
remote interface.

Before Upgrade, the callback requires:

- exactly `GET` and HTTP/1.1;
- exact path `/application/v1` and no query string;
- exactly one `Host` equal to numeric `127.0.0.1:<actual-ws-port>`;
- exactly one `Origin`, neither missing nor `null`, byte-exact in the validated
  scheme/host/port allowlist;
- RFC 6455 version 13 and the library's normal Upgrade/key checks;
- exactly one required `Sec-WebSocket-Protocol` value;
- no negotiated extensions, even if an extension is requested.

Origin is CSWSH protection, not authentication. M12 remains local-first and
unauthenticated. Remote binding, TLS and an authentication framework remain out of
scope and require a separate threat review.

Readiness preserves the existing top-level TCP `port`. When enabled it adds only:

```json
{"websocket":{"port":12345,"path":"/application/v1","subprotocol":"lab-runtime.application.v1"}}
```

## 6. Ownership and concurrency

`ServiceHost` owns both bound listeners through startup/failure unwind. The network
reactor owns their clones, every socket, the one `ConnectionCoordinator`, every TCP
peer and every WebSocket protocol state machine. `server::run` still exclusively
owns `Application`, `SessionStore`, `ServiceHost`, `HostCore`, Runtime and
`OwnerDelivery`.

All accepted TCP and pre-Upgrade WebSocket sockets draw from the same checked `u64`
ID allocator and the same eight-slot admission set. IDs contain no transport bit and
are never reused. A detach-pending generation retains its slot until the owner has
received its detach. Both peer types route exactly the same `Incoming::Request`,
`Outgoing::Reply`, `Outgoing::Event`, and `Outgoing::Close` values over the existing
bounded channels.

HostCore service still precedes bounded client work. The reactor uses nonblocking
socket calls, `try_send` pressure paths, four-message peer turns, an 8,192-byte I/O
sweep and the existing five-millisecond poll sleep. WebSocket enabled accept turns
consider up to four TCP and four WS accepts, so neither listener can monopolize an
accept sweep. No network path can wait indefinitely or own Runtime state.

## 7. Exact bounds and overflow behavior

| Bound | Value | Owner/failure behavior |
|---|---:|---|
| TCP + WS admitted/pre-Upgrade clients | 8 total | One `ConnectionCoordinator`; excess sockets are dropped. |
| Connection IDs | checked `u64` | One process-local allocator; exhaustion is explicit, never wraps. |
| Application JSON body / WS message | 16,383 bytes | Shared codec and Tungstenite message limit. |
| WS frame payload | 16,383 bytes | Tungstenite frame limit; oversized fragmented or single-frame input closes that peer. |
| HTTP Upgrade bytes | 8,192 | Project stream wrapper; excess closes before dispatch. |
| HTTP Upgrade headers | 32 | Callback policy; excess is rejected. |
| Configured origins | 16, each at most 256 bytes | Startup validation rejects excess. |
| Tungstenite eager read buffer | 4,096 bytes | Fixed per admitted WS connection. |
| Tungstenite write target | 0 bytes | Writes are attempted eagerly. |
| Tungstenite maximum write buffer | 32,768 bytes | Overflow closes/detaches the affected connection. |
| Current WS Application write | 1 identity, no duplicate body | Counted inside the existing reply/event limits until successful flush. |
| Fragment deadline observer | 14 header bytes + one length/timestamp | Uses Tungstenite header parsing only; no payload or second message assembly. |
| Reactor read/write work per peer turn | 8,192 bytes | `BoundedStream`; `WouldBlock` yields to other work. |
| Owner/reactor channels | 64 each direction | Existing bounded `sync_channel`; no blocking send. |
| Requests pending per connection | 8 | Shared `ClientDelivery`/`OwnerDelivery`. |
| Completed WS request awaiting owner admission | 1 | Reading stops; the request is retained in order under the existing two-second partial/input deadline. |
| Reply queue | 8 | Shared queue; overflow detaches only that connection. |
| Event queue | 16 | Shared queue; overflow detaches only that connection. |
| Messages read/written per peer turn | 4 | Prevents data/control-frame monopolization. |
| Upgrade/hello/partial-message/output/close deadline | 2 seconds absolute | Connection-local close/detach; progress bytes do not reset partial-message or close deadlines. |

TCP retains its separate 16,384-byte NDJSON-frame bound and 16,384-byte input
allocation. Adding the listener does not double any semantic or global capacity.

## 8. Message, backpressure and control behavior

One complete WebSocket Text message is passed directly to
`decode_application_json`; one encoded Application reply/event byte body becomes
one Text message. There is no `WsRequest`, `WsReply`, WS operation or WS error
taxonomy. UTF-8, duplicates, depth/value/string bounds, envelopes, operations,
arguments and request IDs remain owned by the shared codec.

Tungstenite reassembles valid fragments under both frame and message limits. Only a
completed Text message can dispatch one Application request. Binary messages close
with Unsupported Data and never enter the codec. Unmasked client frames remain
rejected. Ping/Pong and Close are transport-only and are driven within the same I/O
and message budgets.

The adapter accounts for Tungstenite-buffered output: a `WouldBlock` after `write`
means the message is already accepted into the library buffer and is not retried or
duplicated. Its reply/event identity remains current and consumes the same shared
capacity slot until `flush` succeeds; another Application message cannot pass it or
enter the Tungstenite buffer first. Reply/event alternation advances only at that
flush boundary. Automatic pong/close work is flushed even when Application queues
are empty. An outstanding-output deadline begins once work is queued and does not
move with trickle progress. Queue or library-buffer pressure removes only the
affected peer.

The absolute fragmented-message deadline is driven by bounded frame progress below
Tungstenite message assembly. Interleaved Ping/Pong neither starts nor resets it;
ordinary control-only traffic leaves it unset. The message/frame limits, four-message
turn and 8 KiB I/O turn are unchanged.

If the shared owner mailbox fills after one Text message has been assembled and
validated, the adapter retains exactly that one request and stops reading. The next
reactor turn retries the same bounded delivery; it cannot drop, duplicate or reorder
the request, and the existing absolute input deadline still isolates sustained
owner-mailbox pressure.

Decoder capacity/protocol/UTF-8 errors are terminal for input. This matters because
the library may retain a cursor inside the rejected frame. The adapter queues a
bounded close, never re-enters poisoned input state, and enforces the absolute close
deadline. Normal/server-initiated closes continue read/flush progression to an
acknowledgment or that same deadline.

Every socket exit—normal close, protocol failure, reset, EOF, timeout, queue
overflow or shutdown—passes through one coordinator detach fence. Application sees
at most one detach. A disconnect does not undo accepted mutations or own experiment
lifetime.

## 9. Startup, shutdown and failure isolation

Existing validation and safe-host construction still precede listener publication.
TCP binds first, then the optional WS listener, then Recorder attachment/readiness;
any WS bind failure unwinds without readiness. TCP remains unchanged when WS is
disabled.

Application/external shutdown sends the existing `StopAccept` to the one reactor,
which stops both listeners. Runtime safe shutdown and Recorder progress remain on
the owner. Terminal delivery retains the existing finite window; the reactor cannot
extend process shutdown indefinitely for a WebSocket close handshake.

Malformed Upgrade, malformed JSON, oversized/fragmented input, binary input,
control traffic, reset, slow sender/receiver and non-reading peers are scoped to one
connection. Focused tests prove slow/malformed TCP does not delay healthy WS work
and slow/malformed WS does not delay healthy TCP or Runtime owner progress.

## 10. Test evidence

Focused coverage includes:

- allowed, foreign, missing, `null` and multiple Origin cases;
- exact/wrong Host, path, query, method and subprotocol cases;
- extension request without negotiation and explicit byte/header Upgrade caps;
- hello, representative query and mutation through WebSocket;
- shared duplicate-key, depth, value-count, string, invalid-JSON and exact/oversize
  body validation;
- binary rejection, one-dispatch fragmented Text, and fragmented oversize rejection;
- ping/pong, normal close, unacknowledged close timeout and capacity release;
- slow Upgrade, slow fragmented sender, non-reading event overflow;
- reply and event capacity retention while one Tungstenite-accepted Application
  frame is blocked before flush;
- a coalesced non-final Text plus interleaved Ping reaching the absolute deadline,
  with an idle Ping/Pong connection remaining valid;
- HTTP/1.0, missing/wrong Connection and Upgrade headers, wrong RFC 6455 version,
  missing/invalid key and duplicate Host rejection;
- completed-request retention and ordering while the owner mailbox is full;
- mixed TCP+WS eight-client admission, ninth rejection, shared monotonic IDs and
  detach-pending generation fencing;
- two-way slow/malformed transport isolation and clean Runtime shutdown.

All existing TCP suites and expectations remain in place, including
`api_protocol`, `client_isolation`, `request_deduplication`,
`subscription_recovery`, `recorder_history_api`, `runtime_events`,
`runtime_shutdown`, and the bounded-peer unit tests.

## 11. Verification

The final implementation diff passes:

```text
cargo fmt --all -- --check
cargo test --workspace --locked
cargo test --workspace --release --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
git diff --check
```

The focused `websocket_transport` suite passes 13/13 in debug and release. The
current-write accounting, shared-ID and non-reading WebSocket bounded-peer unit
tests, configuration validation and readiness tests also pass. Timing/close tests
were run repeatedly while iterating. The complete focused suite passed three
consecutive debug runs after remediation, including the external-review
fragment/control deadline regression and idle-control counterexample.

The first complete debug workspace run encountered one pre-existing parallel-test
temporary-file cleanup race in
`resource_configuration_api::generic_projection_exposes_native_component_properties_without_special_operations`
(`remove_file` observed `NotFound`). That unchanged test passed immediately in
isolation; a complete debug workspace rerun passed. The complete release workspace
run passed on its first final run. This was not hidden or addressed by weakening the
test.

During remediation, the first complete debug workspace run encountered a separate
unchanged Windows temporary-file lock in
`emulator_api::external_virtual_publication_reaches_durable_measurement_history`
(OS error 32). That test passed immediately in isolation and the subsequent complete
debug workspace rerun passed. The complete release workspace run passed. No test or
expectation was weakened for either transient failure.

Exactly two pre-existing opt-in workloads remain ignored in each default workspace
run:

- diagnostic production rotation beyond the 16 MiB retention window;
- the 2,000-turn/eight-recording-cycle developer-preview soak.

Neither ignored test nor its annotation changed in M12.3.

## 12. Risks and deferred work

1. M12 remains loopback-only and unauthenticated. Any non-loopback binding requires
   a separate review of authentication, TLS, proxy/Host handling, deployment and
   denial-of-service limits.
2. Tungstenite intentionally owns RFC 6455 parsing and bounded fragment reassembly;
   project wrappers still own admission, handshake limits, fairness and deadlines.
3. Browser/ClojureScript client smoke acceptance belongs to M12.5, not this slice.
4. The complete cross-transport session/dedup/subscription/history parity matrix
   belongs to M12.4. M12.3 proves representative shared delivery and preserves the
   existing semantic owners; it does not duplicate that future matrix.
5. No authentication, TLS, compression, async runtime or Internet-service design is
   implied by the local endpoint.

No contradiction was found in Application ownership, session lifetime,
deduplication, backpressure, Runtime progress, shutdown or output safety.

## 13. Explicitly preserved invariants

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

TCP + WebSocket clients <= 8
Application JSON <= 16,383 bytes
TCP NDJSON frame <= 16,384 bytes including LF
bounded queues / histories / workers
network clients do not block Runtime safety progress
WebSocket disconnect does not own experiment lifetime
no raw transport ownership escapes Runtime
no client may fabricate physical evidence
no UI or scripting semantics enter Runtime core
```

M12.4 has not started and remains unauthorized pending external review.

STATUS: M12_3_READY_FOR_EXTERNAL_REVIEW
