# M12.1 WebSocket architecture audit

External review accepted this audit on 2026-09-28:

```text
M12.1: ACCEPTED
```

The report below remains the accepted architecture evidence. Its final status records
the review-ready state in which it was submitted.

## Decision

WebSocket can be added as a second bounded local transport for the existing
Application API without adding a second `Application`, session store, registry,
DTO model, error model, or async runtime.

The source already has a useful semantic boundary:

```text
bounded decoded WireRequest
    -> Application::handle(connection, request)
    -> ServiceHost
    -> HostCore
    -> Runtime
```

The missing seam is in server orchestration. `server.rs` currently combines the
only TCP reactor, global client admission, connection ID allocation, request
scheduling, response/event delivery, and the one `Application`. M12 must first
make those shared delivery responsibilities explicit, while leaving TCP behavior
unchanged. A WebSocket listener can then join that same bounded delivery path.

There is no architectural contradiction and no reason to introduce Tokio for the
accepted local-first scope.

## 1. Repository state

State recorded at the start of this audit on 2026-09-28:

| Item | Value |
|---|---|
| branch | `main` |
| HEAD | `11b53b190a040ccc0afae5738ee115a927343421` |
| working tree | clean |
| published preview | `v0.1.0-preview.1` |
| tag target / packaged-source commit | `70abaf6136a8baa93dfe31aa5d8a7cc56e54ef6e` |

The preview tag resolves directly to the stated packaged-source commit. Current
`main` is later documentation/coordination state; the tag was not moved.

The audit authorization is the explicit M12.1 request. The pre-audit `WORK.md`
correctly prohibited starting WebSocket work automatically; no production code,
tests, dependencies, configuration schema, or Application behavior was changed by
this audit.

Baseline verification after the documentation update:

```text
cargo test --workspace --locked
PASS (all executed workspace and doc tests; only the two pre-existing soak/rotation
tests remained ignored)
```

## 2. Material inspected

### Current authority and public reference

- `AGENTS.md`
- `ai/PROJECT_BRIEF.md`
- `ai/HANDOFF.md`
- `ai/ROADMAP.md`
- `ai/WORK.md`
- `ai/RELEASE_PLAN_TO_V0_1.md`
- `README.md`
- `docs/architecture.md`
- `docs/application-api.md`
- `docs/getting-started.md`
- `docs/safety-and-failures.md`
- `docs/recorder-sqlite.md`
- `docs/extending-runtime.md`

Archived M8/M9D/M11 reports and design rationale were treated as historical
context only. They were not used to override current source.

### Application, protocol, sessions, and delivery

- `apps/lab-runtime/src/application.rs`
- `apps/lab-runtime/src/application/common.rs`
- `apps/lab-runtime/src/application/delivery.rs`
- `apps/lab-runtime/src/application/operations.rs`
- `apps/lab-runtime/src/application/recording.rs`
- `apps/lab-runtime/src/application/{controllers,references,virtuals,discovery,projections}.rs`
- `apps/lab-runtime/src/protocol.rs`
- `apps/lab-runtime/src/wire.rs`
- `apps/lab-runtime/src/sessions.rs`
- `apps/lab-runtime/src/events.rs`
- `apps/lab-runtime/src/measurements.rs`
- `apps/lab-runtime/src/recorder_api.rs`

### Transport, process owner, and startup

- `apps/lab-runtime/src/server.rs`
- `apps/lab-runtime/src/service.rs`
- `apps/lab-runtime/src/service/{shutdown,reconnect,configuration}.rs`
- `apps/lab-runtime/src/host.rs`
- `apps/lab-runtime/src/main.rs`
- `apps/lab-runtime/src/lib.rs`
- `apps/lab-runtime/src/configuration.rs`
- `apps/lab-runtime/Cargo.toml`
- workspace `Cargo.toml` and `Cargo.lock`

### Relevant regression evidence

- `apps/lab-runtime/tests/api_protocol.rs`
- `apps/lab-runtime/tests/client_isolation.rs`
- `apps/lab-runtime/tests/request_deduplication.rs`
- `apps/lab-runtime/tests/subscription_recovery.rs`
- `apps/lab-runtime/tests/recorder_history_api.rs`
- `apps/lab-runtime/tests/runtime_events.rs`
- `apps/lab-runtime/tests/runtime_shutdown.rs`
- the bounded peer tests in `apps/lab-runtime/src/server.rs`

## 3. Current transport/Application boundary

### Actual request path

```text
ServiceHost::startup
  binds 127.0.0.1 TCP listener
        |
server::run
  clones listener and starts one reactor thread
        |
reactor::Peer::read
  bounded stream read -> LF search -> wire::decode_frame
        |
sync_channel<Incoming>(64)
        |
serialized server owner loop
  per-connection queue/fair scheduling
        |
Application::handle(connection: u64, WireRequest)
  hello/session -> registry/feature gate -> query or mutation
        |
ServiceHost -> HostCore -> Runtime/Recorder/configuration lifecycle
        |
Application returns serde_json::Value replies/events
        |
wire::encode_frame -> sync_channel<Outgoing>(64)
        |
reactor per-client reply/event queues -> partial TCP writes
```

### Direct answers

1. **Where transport-specific processing ends.** Today it ends in
   `Peer::dispatch_buffered` after stream buffering, LF extraction, and
   `wire::decode_frame`. The resulting owned `WireRequest` crosses the bounded
   `Incoming` mailbox. On output, transport-specific work begins after a bounded
   JSON value is encoded as an NDJSON frame and sent to the reactor.

2. **Where semantic Application processing begins.** It begins at
   `Application::handle`. That method owns the hello gate, logical-scope attach,
   capability/operation availability, query/mutation routing, dedup admission,
   operation lifecycle, public responses, and connection delivery state.

3. **Does Application depend on TCP/NDJSON concepts?** Its data path does not:
   it accepts `u64` plus `WireRequest` and returns JSON values. It does not use
   `TcpStream`, LF, partial reads, or socket errors. There are minor structural
   couplings to remove: module documentation calls the encoder/network TCP;
   `EventLog` measures event size through `wire::encode_frame`; `protocol::limits`
   imports server/wire constants; and `ServiceHost`, which Application receives,
   currently owns the TCP listener. None changes Application semantics.

4. **Per-connection state.** In `Application`: connection-to-scope attachment,
   one frozen projection, one subscription, one history page, retained history
   cursors tied to that connection, and at most one pending history job for it.
   In the TCP reactor: socket, input/partial-frame state, hello/write deadlines,
   reply/event queues, partial-write offset, pending request count, in-flight
   `msg_id` set, rejection/closing state. In the owner loop: queued requests and
   closing/close-sent sets keyed by connection.

5. **State surviving connection loss.** The logical scope, sequence high-water,
   admitted mutation identities, pending mutations, and retained terminal outcomes
   survive according to `SessionStore` bounds and TTLs. Experiment state,
   process event ring, Recorder state, and durable history also survive. A pending
   recording lifecycle or shutdown operation completes and is retained even if its
   reply is no longer deliverable. A disconnected history job is explicitly
   terminalized as `client_disconnected` and cancelled/orphan-drained.

   Connection attachment, `msg_id`, subscription/filter/scan state, frozen
   projections, history page and continuation tokens, queued network output, and
   partial input/output do not survive.

6. **Runtime-owned state.** `lab_core::Runtime` is the sole authoritative mutable
   experiment owner. `HostCore` owns scheduling/adapters/event projection/Recorder
   admission around it; `ServiceHost` owns process lifecycle, configuration,
   reconnect, boot identity, clock, and shutdown progression. Application state is
   coordination/delivery state, never experiment authority.

7. **State currently owned by the TCP server/reactor.** The listener is presently
   stored in `ServiceHost` and cloned into the reactor. `server.rs` owns physical
   connections, IDs, the eight-client gate, both 64-slot mailboxes, per-client
   input/output buffering, pending `msg_id` tracking, request fairness, event
   pumping, detach delivery, and network shutdown. Some of those are shared server
   policy rather than TCP semantics and must move behind the M12.2 seam.

8. **Can one Application safely serve multiple transports?** Yes, if the same
   `Application` remains on the one serialized owner loop and all transports use
   globally unique connection IDs, one global admission budget, the same bounded
   mailboxes/scheduler, and exact attach/detach notifications. It is not safe to run
   one `Application` per listener: that would duplicate scopes, deduplication,
   subscriptions, asynchronous correlation, and shutdown delivery.

## 4. Current ownership and desired ownership

| Responsibility | Current owner/module | Remain transport-specific? | M12 owner/direction | Reason |
|---|---|---:|---|---|
| loopback bind/listen/accept | `ServiceHost`, `server::reactor` | yes | TCP and WS adapters under one server coordinator | OS endpoint/framing concern |
| WebSocket HTTP Upgrade, path, Origin, subprotocol | absent | yes | WS adapter | meaningful only to WebSocket |
| connection ID allocation | TCP reactor | no | shared connection registry | IDs key one Application and must not collide |
| total client admission | TCP reactor | no | shared connection registry, total remains 8 | two listeners must not double capacity |
| socket read/write and partial I/O | `Peer` | yes | each transport peer | stream versus WebSocket protocol mechanics differ |
| LF/optional CRLF framing | `wire::decode_frame`, `Peer` | yes | TCP/NDJSON adapter | not part of a WebSocket message |
| WS fragmentation/control frames/close | absent | yes | WS adapter/library | RFC 6455 transport state |
| UTF-8 JSON object parsing | `wire::decode_frame` | no | shared Application JSON codec | same payload for both transports |
| duplicate keys/depth/value/string limits | `wire::decode_frame` | no | shared Application JSON codec | semantic input hardening must be identical |
| envelope and `request_id` parse | `wire::decode_frame` | no | shared Application JSON codec | one request DTO model |
| operation lookup/top-level arg allowlist | `protocol` via `wire` | no | shared registry/JSON validation | one operation registry |
| in-flight duplicate `msg_id` rule | TCP `Peer` | no | shared per-connection delivery state | one exchange rule across transports |
| owner/reactor mailbox | `server::run`/reactor | no | shared delivery boundary | prevents transport-specific scheduling |
| per-client request admission/fairness | `Peer`, owner loop | no | shared delivery scheduler | one backpressure policy |
| hello/scope attach | `Application` | no | unchanged `Application` | logical session semantic |
| dedup/operation lifecycle | `Application`, `SessionStore` | no | unchanged single instances | process-global contract |
| query/mutation dispatch/errors | `Application` modules | no | unchanged | semantic API |
| reply/event JSON envelopes | `Application` | no | unchanged | same response/error/event model |
| JSON serialization/size cap | `wire::encode_frame` | no | shared JSON encoder | byte-identical JSON payloads |
| LF append | `wire::encode_frame` | yes | TCP adapter | NDJSON only |
| WS text-message wrapping | absent | yes | WS adapter | WebSocket only |
| reply/event queue policy | TCP `Peer` | no | shared per-connection delivery policy | same slow-client isolation |
| actual write buffer/offset | TCP `Peer` | yes | transport peer/library | lower-level framing differs |
| subscription pump | `Application` called by owner loop | no | shared owner loop | one subscription semantic and cursor |
| disconnect cleanup | reactor -> owner -> `Application::detach` | split | adapter detects; shared owner detaches | cleanup must happen exactly once |
| process shutdown | `ServiceHost` plus `server::run` | no | shared server coordinator | both listeners/peer sets stop together |
| protocol-specific close | raw drop/close flag | yes | each adapter, bounded | TCP EOF and WS close handshake differ |

## 5. Current concurrency model

The Application service path is synchronous and serialized:

- the calling/main service thread owns `ServiceHost`, `Application`, request
  scheduling, Application dispatch, subscription pumping, and shutdown progression;
- one fixed network reactor thread owns all TCP sockets;
- two bounded `std::sync::mpsc::sync_channel` mailboxes connect them;
- neither side blocks waiting for mailbox capacity: `try_send` pressure detaches the
  affected peer;
- each owner turn services `HostCore` before taking client work;
- the owner drains at most 16 incoming messages per turn and processes one queued
  request for at most four clients in rotating order;
- the reactor accepts at most eight sockets per pass, dispatches at most four
  complete frames per peer pass, writes at most four chunks and 8 KiB per peer pass,
  then sleeps 5 ms;
- existing Recorder, physical transport, managed-component, and diagnostic workers
  remain separately bounded and do not transfer Runtime ownership.

M12 should preserve the single serialized Application/ServiceHost owner. A second
listener does not require concurrent calls into `Application` and must not add a
mutex-protected second mutation path.

## 6. Current bounds and M12 classification

`hello.result.limits` is the current public source for most values.

| Bound | Current value/behavior | WebSocket treatment |
|---|---|---|
| clients | 8 peers, counting pending detach generations | one shared global total: TCP + WS <= 8 |
| complete NDJSON frame | 16,384 bytes including LF | transport-specific TCP bound |
| JSON object payload | effectively at most 16,383 bytes with LF framing | shared semantic JSON-message bound; make the internal name explicit without changing TCP behavior |
| JSON nesting | 16 | shared |
| JSON lexical values/members | 1,024 | shared |
| JSON string/key | 512 UTF-8 bytes | shared |
| TCP input buffer | one 16,384-byte frame/peer | TCP-specific; WS also needs bounded handshake, frame, and assembled-message buffers |
| network pass | 8,192 bytes | transport-specific I/O work budget; WS needs an equivalent finite turn budget |
| hello/partial input/blocked output deadline | absolute 2 s | same isolation policy; WS separately applies it to Upgrade, incomplete message, and blocked close/write |
| owner/reactor mailboxes | 64 messages each direction | shared, not 64 per listener |
| requests awaiting owner/reply per client | 8 | shared per connection |
| owner queued requests per client | 8 | shared per connection |
| reply frames per client | 8 | shared delivery bound |
| event frames per client | 16 | shared delivery bound |
| current write | 1 frame plus reply/event/rejection queues | transport-specific encoded buffer under shared queue accounting |
| in-flight `msg_id` | set per connection; duplicate rejected until terminal reply; no independent constant | shared per-connection rule; retain an explicit demonstrable bound rather than create one set per adapter |
| scopes | 16 process-wide | shared single `SessionStore` |
| pending mutations | 8/scope, 64 total | shared |
| terminal outcomes | 32/scope, 256 total | shared |
| terminal outcome size | 4,096 bytes | shared |
| terminal retention | 600 s | shared monotonic TTL |
| detached scope retention | 1,800 s | shared monotonic TTL |
| one semantic event | 4,096 JSON bytes | shared; stop measuring it through NDJSON encoding |
| event replay ring | 1,024 records | shared process ring |
| event replay scan/pump | scan at most 32, offer at most 4 matching events/turn | shared |
| subscriptions | 1/client; 8 kinds; 16 targets | shared connection-local state |
| frozen projection | 1/client; 64 records/8 KiB page; 5 s | shared connection-local state |
| recent measurement query | at most 128 records and 8 KiB result | shared |
| durable history | at most 128 measurement rows or 32 run rows | shared |
| history work/pages | 8 total, at most 1 pending/page per connection; job deadline 2 s | shared |
| history continuation cursors | 8 total; 30 s; tied to connection/scope | shared |
| history page | 8 KiB; retained 5 s; tied to connection | shared |
| shutdown terminal offer | best effort 200 ms after terminal status | shared policy; WS close cannot extend Runtime shutdown indefinitely |

Adding a WebSocket listener must not create another eight-client pool, another pair
of 64-slot mailboxes, or independent Application queues. Handshake sockets awaiting
admission also need finite accounting so Upgrade work cannot evade the global limit.

## 7. JSON versus NDJSON separation

The current codec combines three layers in `wire::decode_frame` and
`wire::encode_frame`:

| Rule | Classification |
|---|---|
| locate LF in a byte stream | NDJSON only, currently `Peer` |
| require trailing LF and accept optional preceding CR | NDJSON only |
| 16,384-byte frame including delimiter | NDJSON only |
| JSON payload UTF-8 | shared JSON; WS text validation may reject earlier but must have equivalent outcome |
| duplicate-key rejection at every object depth | shared JSON |
| depth/value/string/key limits | shared JSON |
| reject trailing JSON data | shared JSON |
| root must be an object | shared JSON |
| top-level request field allowlist | shared Application JSON |
| protocol version and bounded `msg_id`/`op` | shared Application JSON |
| authoritative operation lookup | shared registry |
| known-operation args allowlists/nested shape | shared Application JSON |
| mutation `request_id` parse/canonical decimal sequence | shared Application JSON |
| serialize response/event JSON under a hard cap | shared JSON |
| append LF | NDJSON only |

The desired refactor is therefore:

```text
decode_application_json(bytes_without_transport_framing)
    -> WireRequest

decode_ndjson_frame(bytes_with_lf)
    -> validate LF / CRLF / NDJSON frame size
    -> decode_application_json(body)

decode_websocket_message(text_message_bytes)
    -> validate WS text-message policy and assembled-message size
    -> decode_application_json(bytes)

encode_application_json(Value)
    -> bounded UTF-8 JSON bytes

encode_ndjson_frame(Value)
    -> encode_application_json
    -> append LF

encode_websocket_message(Value)
    -> encode_application_json
    -> one WS text message
```

The common JSON body limit should preserve the current maximum of 16,383 bytes.
The existing advertised `frame_bytes: 16384` remains the TCP complete-frame limit;
M12 must document the WS assembled-message payload limit without silently allowing a
larger semantic request. No parallel request/response DTOs are necessary.

`EventLog::append` currently calls the NDJSON encoder to measure a semantic event.
M12.2 should measure the shared JSON encoding instead while preserving the present
4,096-byte JSON event limit exactly.

## 8. Session, connection, and reconnect findings

### Existing contract

- `hello` is the first Application request on every connection.
- `hello {scope:null}` creates `<boot-id>:<counter>` and attaches it to the opaque
  `u64` connection ID.
- `hello {scope:<retained>}` reattaches only if the boot matches, the scope remains
  retained, and no other live connection owns it.
- `msg_id` is connection-local correlation and is explicitly not a dedup key.
- `(scope, seq)` is the typed mutation identity. Sequences are consecutive.
- dedup equality compares the normalized typed `Mutation`, not bytes, connection,
  transport, or `msg_id`.
- detach removes connection-local delivery state but retains the scope and admitted
  operation records.
- an old boot produces `instance_changed`; an attached scope produces
  `scope_in_use`; an expired/unknown scope produces `scope_unknown`.
- event cursors are process boot plus sequence and are transport-neutral.
- subscriptions themselves are connection-local and are removed on detach. A new
  connection subscribes again after a retained valid cursor.
- frozen projections and history page/continuation tokens are connection-local and
  intentionally expire or disappear on reconnect. Durable history can be queried
  again with a new operation.

### Cross-transport conclusion

This must be valid within the same process and current retention bounds:

```text
TCP connection
  -> hello/new scope
  -> admitted mutation and/or retained event cursor
  -> disconnect and exact detach processing
WebSocket connection
  -> hello/same scope
  -> same next_seq and dedup/operation_status result
  -> recreate subscription from retained event cursor
```

The reverse direction must also work. No current Application contract prevents it.
The only current implementation blockers are server wiring: one TCP-only ID
allocator/admission set and one TCP reactor. A reconnect racing ahead of detach may
correctly receive `scope_in_use`; that existing behavior should not be weakened or
made transport-dependent.

Cross-process reconnect remains invalid because boot identity, scopes, dedup state,
and the event ring are deliberately process-local. M12 does not change that.

## 9. Browser-specific security findings and policy

Loopback binding alone is insufficient for a browser transport: script from an
arbitrary web origin can attempt a WebSocket connection to localhost. RFC 6455
defines the `Origin` header for the server to reject unauthorized browser origins.
Origin is a browser boundary, not authentication for native clients that can forge
headers.

### Required M12 local-first policy

| Area | Policy |
|---|---|
| bind | IPv4 `127.0.0.1` only, preserving current validated configuration; no wildcard, LAN, or remote bind |
| listener | separate optional WS port is simpler and safer than sniffing HTTP and NDJSON on one port |
| path | exact `/application/v1`; reject other paths and query strings before Upgrade |
| Host | require the numeric loopback host and actual WS port; do not trust arbitrary DNS hostnames |
| method/version/Upgrade | strict RFC 6455 HTTP GET/1.1 Upgrade and version 13 validation |
| Origin | require exactly one syntactically valid Origin and exact match against a startup-validated allowlist |
| allowed origins | explicit scheme/host/port tuples; no `*`, suffix, substring, regex, or implicit “all localhost ports” match |
| `Origin: null` | reject; file/data/sandboxed pages must instead be served from an explicitly allowed local HTTP origin |
| missing Origin | reject in the browser-oriented M12 endpoint; a native client can continue using TCP |
| subprotocol | require and return `lab-runtime.application.v1`; it is protocol negotiation, not a credential |
| extensions | negotiate none in M12; in particular do not add compression and its additional memory/accounting surface |
| authentication | no token framework is necessary for loopback + strict Origin in M12; local native processes already have the unauthenticated TCP path |
| errors | bounded HTTP rejection; do not echo secrets/raw headers or expose an Application scope before successful Upgrade and `hello` |
| deadlines/bounds | absolute 2 s Upgrade deadline; bounded header bytes/count, frame size, assembled message size, write buffer, and close duration |

Enabling WS should require at least one explicit allowed origin. A ClojureScript
development server therefore supplies its exact origin, for example
`http://127.0.0.1:8080`, and connects to a numeric-loopback WS URL. An origin
allowlist must never be treated as authorization for non-browser clients.

### Remote binding review gate

M12 must keep non-loopback binding rejected. Before any later remote binding is
allowed, conduct a separate architecture/security review covering at least TLS and
certificate lifecycle, authenticated principals and authorization, credential
storage/rotation, proxy and forwarded-header trust, origin policy behind proxies,
network-level admission/rate limits, denial-of-service budgets, audit requirements,
secure discovery, browser mixed-content constraints, and threat/fault testing.

## 10. Recommended M12 architecture

```text
                   shared Network/Delivery Coordinator
                  (global IDs, admission <= 8, bounded queues)
                                  |
      +---------------------------+---------------------------+
      |                                                       |
TCP listener / NDJSON peer                         WS listener / RFC 6455 peer
- stream reads/writes                              - bounded HTTP Upgrade
- LF/CRLF framing                                  - path/Host/Origin/subprotocol
- partial byte offsets                             - text/fragment/control/close
      |                                                       |
      +------------------- bounded JSON messages -------------+
                                  |
                    shared Application JSON codec
                    (one WireRequest / one encoder)
                                  |
                    shared fair delivery boundary
                 (msg_id, reply/event bounds, detach)
                                  |
                         ONE Application
              (ONE SessionStore, subscriptions, dedup)
                                  |
                            ServiceHost
                                  |
                              HostCore
                                  |
                               Runtime
```

### Explicit ownership

- **Shared:** JSON codec, `WireRequest`, response/error/event envelopes, operation
  registry, capabilities, global connection IDs/admission, owner mailboxes, request
  scheduling, per-connection `msg_id` rule, reply/event queue policy, Application
  attach/detach, subscription pumping, and shutdown coordination.
- **Per transport:** accept/handshake, stream/message framing, low-level input and
  output buffers, partial write mechanics, transport protocol errors, EOF/close
  mechanics, and WS Origin/path/subprotocol checks.
- **Connection IDs:** one checked monotonic allocator in the shared coordinator;
  never one counter per listener. IDs are never reusable during the process.
- **Global admission:** the shared coordinator owns one eight-slot set. Accepted TCP
  peers, WS peers, and detach-pending generations count against it. Pre-Upgrade work
  is also bounded and cannot form a second uncharged pool.
- **Message/request bounds:** the common codec owns JSON limits; transport peers own
  lower-level framing bounds; the shared delivery layer owns request, reply, event,
  and in-flight exchange capacities.
- **Attach/detach:** only the serialized owner calls `Application::handle` for hello
  and `Application::detach` once for a closed connection. A transport reports facts;
  it does not edit session state.
- **Subscriptions/events:** the owner loop pumps the one Application subscription
  state and routes each event by connection ID to the appropriate transport peer.
- **Shutdown:** `ServiceHost` remains semantic shutdown owner. The shared server
  coordinator stops both listeners, permits bounded terminal delivery, drives WS
  close best-effort within the existing finite window, and joins network work. A
  slow close never delays Runtime safety/Recorder progress indefinitely.

`ServiceHost` currently stores one TCP listener and exposes TCP address/readiness.
M12.2 should separate process semantics from transport ownership while preserving
the existing TCP endpoint and readiness JSON. M12.3 can then add an optional WS
endpoint/readiness field and explicit allowed-origin configuration without changing
Application DTOs.

## 11. Implementation library direction

### Existing `std::net` model

Keeping the current single nonblocking reactor and serialized owner best preserves
the accepted scheduler and failure model. Implementing RFC 6455 itself is not
recommended; masking, fragmentation, control frames, HTTP Upgrade, and close
semantics are substantial protocol surface unrelated to laboratory semantics.

### Synchronous/nonblocking Tungstenite direction

At implementation time, evaluate and pin a reviewed `tungstenite` release with only
the required server-handshake feature and no TLS/client URL features for loopback
M12. Current Tungstenite documentation confirms:

- it operates over any `Read + Write` stream and its handshake can retain an
  interrupted nonblocking state;
- the header callback can inspect/reject the Upgrade request;
- `max_message_size`, `max_frame_size`, read buffer, write buffer, maximum write
  buffer, and masked-client policy are configurable;
- `WouldBlock` is nonfatal, fragmented messages are reassembled under the message
  cap, ping produces an automatic pong that still needs I/O driving, and close must
  be driven until complete or the local deadline expires.

Do not accept library defaults. The currently documented defaults include a 128 KiB
read buffer, 64 MiB message limit, 16 MiB frame limit, and unlimited maximum write
buffer, all too large for this server. M12.3 should set, test, and document:

- approximately 4 KiB eager read allocation per peer;
- maximum assembled text message 16,383 bytes;
- maximum frame payload no larger than 16,383 bytes;
- eager/small write buffering plus a finite internal maximum sufficient for one
  maximum message and protocol overhead, while the shared 8/16 frame queues remain
  authoritative;
- masked client frames required;
- a separately enforced handshake byte/count limit and the absolute 2 s deadline;
- no extensions;
- explicit ping/pong and close driving in each finite reactor turn.

The library's current handshake implementation documents a hard internal 65,536-byte
attack threshold and 124 parsed headers. M12.3 must either place a stricter bounded
reader in front of it or explicitly accept and test a lower project-owned policy;
the library threshold must not become an unreviewed project default.

Primary references used for this dependency direction:

- [RFC 6455: The WebSocket Protocol](https://www.rfc-editor.org/rfc/rfc6455)
- [Tungstenite `WebSocketConfig`](https://docs.rs/tungstenite/latest/tungstenite/protocol/struct.WebSocketConfig.html)
- [Tungstenite server handshake](https://docs.rs/tungstenite/latest/tungstenite/fn.accept_hdr_with_config.html)
- [Tungstenite nonblocking handshake error](https://docs.rs/tungstenite/latest/tungstenite/handshake/enum.HandshakeError.html)
- [Tungstenite feature/dependency manifest](https://docs.rs/crate/tungstenite/latest/source/Cargo.toml.orig)

### Tokio/async alternative

Tokio is not justified by present evidence. It would add an async runtime, task and
shutdown lifecycle, new channel/backpressure choices, and a second scheduling model
around a service deliberately built as one bounded owner plus one nonblocking
reactor. Reconsider it only if later measured requirements demand connection scale
or integrations that the current reactor cannot meet. Eight local clients and a
16 KiB message bound do not constitute that blocker.

**Conclusion:** WebSocket can be integrated without an async runtime; preserve the
current concurrency model and use a synchronous/nonblocking RFC 6455 library after
the M12.2 seam exists.

## 12. Presentation boundary check

The authoritative 42-operation registry contains laboratory/runtime operations for
sessions, discovery, measurements, configuration, control, References, virtual
publication, Recorder lifecycle, history, subscription, and shutdown. There are no
Application DTOs or operations for `Window`, `Tabs`, `Row`, `Column`, `Plot`,
`Button`, `Slider`, layout, widget, egui, workspace, trace, or panel semantics.

Uses of “window” in production source mean bounded measurement/history windows or
algorithm sample windows. SQLite “row” and diagnostic `TRACE` are not presentation
concepts. The finite demo comment correctly says presentation uses descriptors; it
does not create a presentation API.

M12 therefore has no presentation debt to repair and must not add browser-oriented
convenience operations. A future `PresentationDocument` remains client-owned M14
work and is outside this milestone.

## 13. M12.2-M12.5 decomposition

### M12.2 — transport-neutral server seam

**Goal:** make the current shared delivery boundary explicit with byte-for-byte and
behavior-for-behavior TCP compatibility.

**Production change:** split shared JSON encoding/decoding from NDJSON framing;
extract global connection admission/IDs, common delivery queues, scheduling,
`msg_id`, attach/detach, event pumping, and shutdown coordination from the TCP peer;
remove listener ownership leakage from semantic `ServiceHost` as needed. Keep one
`Application` and one `SessionStore`.

**Tests:** all existing TCP tests unchanged; add focused seam tests proving the
shared codec has the current strict behavior and a mock second transport cannot
duplicate IDs/capacity or Application state.

**Non-goals:** no WS handshake, no dependency, no new listener/config, no public
operation/DTO/error change, no capacity increase.

**Review gate:** exact current TCP protocol/bounds/ordering/isolation regression and
external review before M12.3.

### M12.3 — bounded WebSocket/JSON transport

**Goal:** add an optional local browser transport into the M12.2 shared seam.

**Production change:** add the reviewed minimal WebSocket library; optional separate
loopback listener; explicit allowed origins and endpoint path/subprotocol; bounded
nonblocking handshake/message/write/close state; text JSON adapter; readiness and
transport configuration necessary to discover the endpoint.

**Tests:** handshake/path/Host/Origin/subprotocol matrix; text/binary/fragment/control
frames; explicit library bounds; client admission across both transports; unit and
integration fault cases.

**Non-goals:** no remote bind, TLS/auth framework, UI, ClojureScript, new
Application operation, changed Runtime/Recorder/safety semantics, or async runtime.

**Review gate:** security/bounds/dependency review and proof that TCP remains
unchanged before parity expansion.

### M12.4 — parity, reconnect, backpressure, and fault acceptance

**Goal:** prove both transports are adapters for the same Application behavior.

**Production change:** only fixes required by parity/fault evidence; no new
semantics. Tighten transport-local limits if testing demonstrates a gap.

**Tests:** complete matrix in the next section, including TCP-to-WS and WS-to-TCP
scope reattach, cross-transport dedup, replay/gaps/history, non-readers, saturation,
detach races, and clean/fatal shutdown.

**Non-goals:** no presentation, remote deployment, SDK, scripting host, or broader
API revision.

**Review gate:** all current TCP regression gates plus new dual-transport acceptance
pass with no capacity multiplication and no Runtime-progress regression.

### M12.5 — browser/ClojureScript smoke acceptance

**Goal:** demonstrate a real browser/ClojureScript client can use the same API
without acquiring experiment ownership.

**Production change:** none expected beyond defects found by acceptance.

**Tests/evidence:** serve a minimal external smoke client from one explicitly
allowed loopback HTTP origin; connect with the required subprotocol; hello, query,
mutation lifecycle, subscription/replay, disconnect/reattach, and shutdown or clean
client close. Record commands and bounded results; do not add a bundled UI.

**Non-goals:** no Workbench, layout/schema, plotting, PresentationDocument, Steel,
client SDK commitment, or Internet deployment.

**Review gate:** browser smoke evidence and coordination update, then M12 external
review. Do not begin M13 automatically.

## 14. Future acceptance-test matrix

No tests are implemented in M12.1. The following matrix is required after the seam
and WS transport exist.

| Area | Required acceptance |
|---|---|
| same operation | run representative query and mutation, then registry-driven supported-operation coverage, through TCP and WS against one process |
| response semantics | compare decoded JSON envelopes/results independent of transport framing; mutation accepted/terminal order identical |
| error semantics | same public code/category/message/retry/resync fields for semantic errors |
| request ID | canonical scope/sequence validation and consecutive sequence behavior identical |
| dedup | same typed mutation retried on either transport replays retained outcome; different mutation conflicts; evicted result remains unknown and never re-executes |
| `msg_id` | duplicate while in flight closes/rejects only that connection; reuse after terminal remains valid on both |
| session/scope | first-message hello gate, new scope, same-scope live conflict, expiry, old boot, and exact next sequence match |
| TCP -> WS reconnect | detach TCP, attach retained scope on WS, query status and continue next sequence |
| WS -> TCP reconnect | symmetric case |
| subscriptions | create/pump/filter/unsubscribe on both; subscription removed on disconnect and recreated from cursor |
| event replay | retained cursor returns the same ordered events through either transport |
| event gap | ring overrun returns the same `event_gap`, oldest/latest, resync flag, and detaches installed overflowing subscription as currently specified |
| history | same durable run/measurement selection and page content; connection-local page/cursor loss on reconnect remains explicit |
| oversized input | TCP over-frame and WS over-message/fragment accumulation are bounded and scoped; Runtime continues |
| malformed JSON | complete malformed objects receive equivalent public protocol rejection where applicable and only bad peer closes |
| duplicate JSON keys | reject at every depth identically before Application dispatch |
| invalid UTF-8 | TCP JSON bytes reject as `invalid_utf8`; invalid WS text fails at RFC 6455/codec boundary without Application dispatch |
| binary WS | reject as unsupported data and close with bounded protocol handling; never interpret it as JSON |
| fragmented WS text | legal fragments assemble once under the common message cap and dispatch exactly one request; control-frame interleaving remains correct |
| slow sender | partial TCP frame and incomplete WS message/Upgrade hit absolute deadline without starving a healthy peer |
| slow/non-reading receiver | reply and event pressure detaches only that peer within queue/deadline policy |
| queue saturation | both directions of the shared mailbox and per-client request/reply/event queues remain bounded; pressure never blocks owner progress |
| global clients | every mix satisfying TCP + WS = 8 admits; the ninth is rejected; detach-pending generation still counts; capacity is released exactly once |
| disconnect cleanup | one detach removes delivery state, retains admitted session outcome, cancels pending history per current contract, and prevents stale delivery to a new ID |
| owner continuity | malformed/slow/crashed clients do not stop acquisition, control, safety, Recorder, another client, or experiment lifetime |
| ping/pong | ping is answered/driven within bounded turns without entering Application or hiding blocked output |
| close | peer close, server close, reset without close, and close timeout all detach exactly once |
| clean shutdown | both listeners stop; terminal response is best effort within current finite window; reactors/workers join truthfully |
| Origin allowed | exact configured origin + path + Host + subprotocol upgrades and still requires Application `hello` |
| Origin rejected | foreign, missing, multiple, malformed, wildcard-like, and `null` origins receive bounded HTTP rejection before Application attachment |
| path/Host/subprotocol | wrong path/query/Host or missing/wrong subprotocol rejects before Upgrade |

### Existing unchanged TCP regression gates

These test targets must continue to pass without weakening or rewriting their TCP
expectations:

- all tests in `api_protocol.rs`, including exact-size/CRLF, duplicate-key,
  depth/value, UTF-8, version, and output-authority rejection cases;
- all tests in `client_isolation.rs`, including fragmented/coalesced input, absolute
  trickle deadline, malformed/oversized isolation, disconnect continuity, and
  pending/completed `msg_id` behavior;
- all bounded peer tests in `server.rs`, including exact input cap, partial write
  offset, nonreader event overflow, eight-client capacity, detach under full mailbox,
  and bounded oversized response fallback;
- all tests in `request_deduplication.rs`;
- all tests in `subscription_recovery.rs`;
- all tests in `recorder_history_api.rs`, especially reconnect/expiry/capacity and
  stale completion isolation;
- `runtime_events.rs` and `runtime_shutdown.rs`, including the fatal owner/reactor
  shutdown path.

The new cross-transport suite should reuse the same semantic assertions and fixtures,
not copy the registry, DTOs, or expected error taxonomy into a second implementation.

## 15. Risks and unresolved implementation questions

1. **`ServiceHost` owns the TCP listener.** This is the clearest structural layering
   debt. The M12.2 move must preserve startup failure ordering, loopback binding,
   readiness output, tests using trusted hosts, and shutdown behavior.
2. **Limit vocabulary is partly NDJSON-named.** `frame_bytes` includes LF, while the
   shared JSON body does not. Preserve TCP compatibility and document the WS
   assembled-message bound explicitly; do not let naming accidentally change the
   accepted byte budget.
3. **`EventLog` uses the NDJSON encoder as a size oracle.** Refactoring it must retain
   exactly 4,096 JSON bytes and fail-closed behavior.
4. **In-flight `msg_id` has no standalone constant.** It is bounded indirectly by
   request and pending-operation limits. M12.2 should make the proof and owner
   explicit without changing valid pipelining.
5. **Detach/reattach race.** A reconnect can see `scope_in_use` until the exact old
   detach reaches the owner. The shared coordinator must never synthesize an early
   detach or reuse an ID merely to make cross-transport reconnect appear immediate.
6. **WebSocket endpoint configuration/readiness.** The exact names for optional WS
   port and allowed-origin fields are an M12.3 deployment-schema review item. They
   are not Application DTOs. Existing TCP `port` and readiness fields must remain.
7. **Handshake lower-level limits.** A candidate library can have larger hardcoded
   header thresholds than this project wants. Inspect the pinned release and test a
   project-owned bounded wrapper before acceptance.
8. **Hidden write buffering/control replies.** Automatic pong/close data and library
   write buffers must count toward finite transport memory/deadlines; an apparently
   empty Application queue is not proof that the WS socket has no pending bytes.
9. **Dependency evolution.** Recheck the exact pinned Tungstenite API, advisories,
   license inventory, transitive dependencies, MSRV, and packaging at M12.3. This
   audit evaluated direction only and added no dependency.
10. **Separate-port resource accounting.** Two listeners are simpler, but accept and
    pre-handshake work must share one admission structure. Per-listener counters are
    prohibited.

None of these questions requires a new Application operation, DTO, session model,
or Runtime semantic.

## 16. Invariants M12 must preserve

```text
Runtime owns experiment semantics.
Client owns presentation semantics.

client lifetime != experiment lifetime
script lifetime != experiment lifetime

ONE Application API
ONE Application instance per Runtime process
ONE operation registry
ONE request/response DTO model
ONE public error model
ONE SessionStore and scope namespace
ONE dedup/admission history
ONE subscription/replay semantic
ONE event ring and gap semantic
ONE history semantic

TCP + WebSocket clients <= 8 unless a separately reviewed future change says otherwise
all queues, buffers, histories, handshakes, workers, and shutdown paths remain bounded
slow or malformed network clients do not block required Runtime safety/control/Recorder progress

connection loss never owns or rolls back experiment lifetime
WebSocket disconnect does not cancel already admitted mutations
reconnect, reload, transport health, or fresh input never automatically rearms output

requested != authorized != send_started != ACK != readback != physical_effect
no transport/client can fabricate observation, ACK, readback, safe evidence, or completion
no raw transport or OutputAuthority ownership escapes Runtime

WebSocket remains loopback-only in M12
browser Origin is explicitly validated before Upgrade
remote binding requires a separate security review

no UI/workspace/plot/layout/widget semantics enter Runtime or Application
no scripting VM enters Runtime ownership
no Recorder, SQLite, physical safety, or controller semantic changes
```

STATUS: M12_1_READY_FOR_EXTERNAL_REVIEW
