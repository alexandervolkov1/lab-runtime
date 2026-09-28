# M12.4 transport parity/fault acceptance report

## 1. Status and scope

M12.4 proves that TCP/NDJSON and WebSocket/JSON are two bounded adapters for one
Application contract. The accepted M12.3 implementation under test is:

```text
6fb867389426fa75033f54312fda8c5556c52ed7
```

The M12.3 acceptance/authorization coordination commit is:

```text
aace1f25de873db11f68e7db364e570af3d0d920
```

The milestone adds one focused integration suite and makes one one-line production
correction demonstrated by a red cross-transport dedup oracle. It adds no operation,
DTO, session/reconnect rule, capacity, transport, endpoint option, async runtime,
client SDK, scripting, presentation or physical-safety semantic.

```text
M12.3 bounded WebSocket/JSON transport: ACCEPTED
M12.4 transport parity/fault acceptance: READY FOR EXTERNAL REVIEW
M12.5 browser/ClojureScript smoke acceptance: NOT AUTHORIZED
```

### External-review remediation

The first M12.4 external review accepted the architecture, existing suite and the
`request_conflict` production correction, but requested two stronger oracles. This
report records those findings as review-time corrections rather than presenting
them as part of the original implementation:

1. The initial event-gap test compared the stable error fields but removed
   `oldest` and `latest` because sequential requests could observe intervening
   native events. The corrected test submits both requests before consuming either
   response and now asserts exact TCP/WS equality for both cursor objects.
2. The initial asynchronous-disconnect evidence used `history_read`. Review
   correctly rejected that as proof of admitted-mutation lifetime because history
   work is intentionally connection-cancelled. It was replaced with a deterministic
   `recording_start` lifecycle oracle that reconciles, retries, stops and cleanly
   shuts down the one retained run after TCP-to-WebSocket migration.

Neither correction required production code. The expected production diff remains
only the previously reviewed one-line `request_conflict` mapping restoration.

## 2. Files changed

| File | Change |
|---|---|
| `apps/lab-runtime/tests/transport_parity.rs` | Test-only TCP/WS client adapters and ten cross-transport acceptance cases. |
| `apps/lab-runtime/src/application.rs` | Correct one internal conflict-code typo to select the already accepted public `request_conflict` mapping. |
| `ai/M12_4_TRANSPORT_PARITY_FAULT_ACCEPTANCE.md` | This implementation and verification evidence. |
| Active coordination files | Mark M12.4 ready for external review and keep M12.5 gated. |

No M12.4 change was made to Runtime, Recorder, SQLite, controller,
`OutputAuthority`, transport capacities, WebSocket configuration or presentation
semantics.

## 3. Test architecture and comparison method

`transport_parity.rs` starts one real `ServiceHost` with both ephemeral IPv4
loopback listeners enabled and drives the existing single reactor. Its test-only
`Peer` enum performs only:

```text
TCP: Application JSON + LF <-> decoded JSON value
WS:  one Text message       <-> decoded JSON value
```

It does not enter production code, model a client SDK, hide transport failures or
duplicate Application validation. Tests compare decoded Application envelopes.
They remove only `msg_id` where the two connections deliberately use different
correlation values. Scope IDs, event cursors and page tokens are compared according
to their documented owner rather than erased indiscriminately.

Detach synchronization uses the authoritative hello result: a reconnect may
temporarily receive `scope_in_use`, and the test retries with a fresh connection
until the serialized owner accepts that exact retained scope. There is no sleep-
based weakening of the detach race.

The suite consumes the authoritative `protocol::OPERATIONS` registry to freeze its
42 entries and compares the hello-advertised operations, capabilities and limits
between transports. It does not maintain a second operation registry.

## 4. Basic semantic parity

Against one Runtime process, TCP and WebSocket return equivalent decoded semantics
for:

- the first-request hello gate;
- successful hello and its boot/protocol/API/operations/capabilities/limits;
- a representative `reference` query;
- an unsupported operation;
- structurally invalid arguments;
- a public `unknown_reference` domain failure;
- accepted and terminal `reference_retune` mutations;
- `operation_status` and retained terminal outcome projection.

The Application response is not required to have byte-identical JSON member order.
Only TCP owns LF framing, and only WebSocket owns message framing/control traffic.

## 5. Scope, reconnect and dedup evidence

The real-network sequence proves both migration directions:

```text
TCP hello/new scope
-> TCP mutation seq 1, terminal completed
-> TCP disconnect and exact detach settlement
-> WS hello/same scope, next_seq 2
-> identical seq 1 retry returns retained result without revision advance
-> seq 1 with a different normalized mutation returns request_conflict
-> WS mutation seq 2 completes
-> WS disconnect and exact detach settlement
-> TCP hello/same scope, next_seq 3
-> identical seq 2 retry returns retained result
-> next consecutive mutation succeeds
```

The same case also proves:

- a live scope cannot attach to a second connection;
- an old boot scope returns `instance_changed`;
- the scope identity and sequence high-water mark survive transport loss;
- the same `msg_id` is reusable on the replacement connection;
- retry identity is normalized mutation plus retained request ID, not transport or
  connection identity;
- retrying retained work does not execute a second time (the reference revision
  remains unchanged);
- after more than 32 terminal outcomes, evicted seq 1 is `outcome_unknown`, is not
  re-executed, and seq 35 remains the only admissible next operation.

### Admitted Recorder mutation survives origin transport loss

A deterministic lifecycle oracle starts a Runtime with a temporary Recorder
database, admits `recording_start(scope, seq=1)` over TCP, observes only
`state=accepted`, and drops TCP without consuming a terminal reply. WebSocket then
reattaches the same retained scope through the exact detach predicate and polls
`operation_status` until the authoritative result is terminal.

The terminal is `completed`, and an exact retry of the same request ID and label
returns the same retained run identity. `recording_status` reports that one run,
proving the retry did not start a second run. The client then uses seq 2 to stop
that exact run, observes the completed stop and idle Recorder state, and uses seq 3
for a finite successful Runtime shutdown whose terminal result reports
`recorder_flushed=true`.

This is Recorder lifecycle work, whose lifetime is authoritative and independent of
the originating connection. It deliberately does not use `history_read` as this
proof: history jobs have the accepted connection-local cancellation contract and
may terminalize as `client_disconnected`.

### Red reproduction and production correction

The different-normalized-mutation oracle initially returned public
`operation_failed`. Source inspection showed `Application` passing the unknown
internal string `request_id_conflict`, while the accepted public taxonomy and
message mapping already declare `request_conflict`. Unknown internal strings
correctly fail closed to `operation_failed`, exposing the typo.

The sole production correction changes that dispatch string to
`request_conflict`. This selects an existing public code/category/message; it adds no
taxonomy entry and changes no `SessionStore` admission rule. The regression now
passes through both transport migration directions.

## 6. `msg_id` and connection-local cleanup

For both transports, two coalesced requests with the same in-flight `msg_id` produce
one result followed by `duplicate_msg_id` and isolate that connection. After a
terminal exchange, reuse on the same connection succeeds. Equal `msg_id` values on
independent TCP and WebSocket connections correlate independently.

Disconnect cleanup evidence covers:

- subscription state: the replacement transport installs a fresh subscription;
- frozen projection state: an old discovery projection returns `snapshot_expired`;
- durable-history page state: an old page token returns `history_page_expired`;
- `msg_id` state: the replacement connection can reuse the old identifier;
- delivery queues: disconnect and replacement do not receive stale output, backed
  by the existing reactor generation-fence tests.

The same disconnect retains the logical scope, admitted operation outcomes,
Runtime experiment state, Recorder archive and global event ring.

## 7. Subscriptions, replay and event gaps

The suite subscribes on TCP from an authoritative cursor, performs a mutation,
receives its ordered reference event, and unsubscribes. After migration to
WebSocket, it installs a new subscription from that retained cursor and observes the
remaining ordered replay before new live events. The reverse WebSocket-to-TCP path
does the same. No connection-local subscription silently migrates.

The accepted 1,024-event ring is then overrun without changing its bound. The TCP
and WebSocket gap requests are both submitted before either response is consumed,
so one serialized owner sweep evaluates them against the same Application/EventLog
snapshot. The test asserts exact equality of `oldest` and exact equality of
`latest`, in addition to the complete stable error envelope. Each response also
proves `code=event_gap`, `resync_required=true`, equal boot identity and an exact
1,024-record retained cursor window.

Existing unchanged `subscription_recovery` tests remain the lower-level evidence
that a gap during an installed replay abandons that subscription and that filtered
scans advance correctly.

## 8. Durable history evidence

A real SQLite archive with a sealed prior run is opened by one Runtime serving both
transports. TCP and WebSocket independently issue the same bounded `history_read`
run selection and receive semantically identical page contents. Tokens differ by
connection ownership; after TCP disconnect and scope reattachment over WebSocket,
the old token is invalid, while a fresh ordinary query returns the same durable
archive page.

The unchanged Recorder history suites continue to prove indexed measurement pages,
cursor bounds, disconnect cancellation, worker isolation, archive reopening and
`client_disconnected` terminalization. M12.4 neither modifies nor bypasses Recorder
or SQLite.

## 9. Input and error parity

Fresh bad peers on both transports freeze the shared codec behavior for:

- duplicate keys;
- invalid JSON;
- depth overflow;
- value-count overflow;
- string bound;
- invalid request envelope/unknown field;
- invalid request ID;
- unsupported operation, invalid arguments and public domain failure.

After each rejection, healthy TCP and WebSocket peers still query the Runtime.
Existing unchanged `api_protocol`, `client_isolation` and `websocket_transport`
tests retain exact 16,383-byte semantic JSON, 16,384-byte NDJSON frame, oversized
message, binary WebSocket and fragmented WebSocket coverage.

Invalid UTF-8 deliberately remains transport-specific: TCP reaches the shared codec
and returns `invalid_utf8`; RFC 6455/Tungstenite rejects invalid Text before
Application dispatch. Both paths scope the bad peer and never dispatch invalid
input.

## 10. Backpressure, admission and fault isolation

The M12.4 composition test pipelines 32 replies into a non-reading/pressured TCP
peer while a healthy WebSocket peer completes repeated queries, then performs the
symmetric WebSocket-pressure/TCP-progress case. No bound is raised.

The full regression run retains the deterministic lower-level evidence for:

| Contract | Existing unchanged evidence |
|---|---|
| `QUEUE = 64`, nonblocking owner/reactor delivery | server coordinator/bounded-peer tests |
| `CLIENT_IN = 8`, `CLIENT_OUT = 8`, `CLIENT_EVENTS = 16` | coordinator, TCP isolation and WS accepted-but-unflushed/event-overflow tests |
| incomplete TCP input does not block WS | `websocket_transport` |
| incomplete/fragmented WS input does not block TCP | `websocket_transport` |
| TCP + WS <= 8, pre-Upgrade included | mixed-client WebSocket acceptance |
| checked unique cross-transport IDs | `tcp_and_websocket_connections_draw_from_one_id_space` |
| detach-pending generation retains capacity | coordinator and unacknowledged-close acceptance |
| exact-once capacity release/detach | coordinator and repeated-disconnect tests |
| stale outgoing data cannot reach replacement | bounded-peer stale-generation regression |
| owner-mailbox saturation remains bounded | WS retained-request and full-mailbox detach tests |

The global pool remains eight; there is no per-transport reserve or duplicated
queue. Slow or malformed network work never owns Runtime progress.

## 11. Shutdown evidence

With TCP and WebSocket simultaneously attached, a real `runtime_shutdown` mutation
is admitted and the server reactor exits within its finite deadline. Both listeners
refuse new connections after exit. The WebSocket's underlying connection reaches
EOF/error without extending shutdown.

Existing unchanged `runtime_shutdown`, `recorder_shutdown` and fatal owner/reactor
tests remain the authority for safe Runtime progression, Recorder flush truth,
best-effort terminal delivery and nonzero fatal cleanup. Network closure is not
treated as physical-safety evidence.

## 12. Verification

Final verification after the last code change:

```text
cargo fmt --all -- --check
    PASS

cargo test -p lab-runtime --test transport_parity --locked
    PASS three consecutive debug runs
    10 passed / 0 failed / 0 ignored each run

cargo test -p lab-runtime --test transport_parity --release --locked
    PASS
    10 passed / 0 failed / 0 ignored

cargo test --workspace --locked
    PASS

cargo test --workspace --release --locked
    PASS

cargo clippy --workspace --all-targets --locked -- -D warnings
    PASS

git diff --check
    PASS
```

Two existing opt-in workloads remain ignored exactly as before:

- diagnostic rotation beyond the 16 MiB retention window;
- the 2,000-turn/eight-recording-cycle developer-preview soak.

During verification, one initial debug workspace run exposed two existing
`emulator_api` tests colliding on/locking a Windows temporary SQLite path; both
passed together with one test thread and the complete debug rerun passed. A later
cached debug run transiently observed `Failed` instead of `Closed` in the existing
Recorder deferred-cancellation unit test; the exact isolated test and complete
debug rerun passed. Neither failure involved an M12.4-modified module, and no test,
timeout or production behavior was weakened to hide them.

## 13. Invariants preserved

```text
ONE Application and ONE serialized owner
ONE SessionStore
ONE operation registry / DTO model / error taxonomy
ONE deduplication, subscription, event and history semantics
ONE checked connection-ID space
ONE TCP + WebSocket <= 8 capacity pool

Application JSON <= 16,383 bytes
TCP NDJSON frame <= 16,384 bytes including LF
QUEUE = 64
CLIENT_IN = 8
CLIENT_OUT = 8
CLIENT_EVENTS = 16

Runtime owns experiment semantics.
Client owns presentation semantics.

client lifetime != experiment lifetime
script lifetime != experiment lifetime

network clients do not block required Runtime safety/control/Recorder progress
disconnect does not undo an admitted mutation or own experiment lifetime
no raw transport ownership escapes Runtime
no client fabricates physical evidence
no UI or scripting semantics enter Runtime core
```

## 14. Remaining M12.5 work

M12.5 remains separately gated. Its intended scope is a browser/ClojureScript smoke
acceptance against the accepted loopback endpoint: exact Origin/subprotocol setup,
hello/discovery, one representative query/mutation, subscription observation,
disconnect/reconnect, and clear browser-facing startup instructions. It must remain
a client-side smoke exercise and must not add Application operations, presentation
semantics, an SDK framework, authentication/TLS/remote binding or Runtime-owned
scripting.

## 15. Review conclusion

No architectural contradiction was found. Both transports exercise the same
bounded delivery seam and the same authoritative Application/session state. The
single production correction restores the already declared public conflict mapping;
all other M12.4 work is acceptance evidence.

```text
STATUS: M12_4_READY_FOR_EXTERNAL_REVIEW
```
