# M12 consolidated external review

## Review state

```text
M12.1: ACCEPTED
M12.2: ACCEPTED
M12.3: ACCEPTED
M12.4: ACCEPTED
M12.5: ACCEPTED

M12 consolidated external review: READY FOR EXTERNAL REVIEW
M13: NOT AUTHORIZED
```

This is a read-only consolidation of the accepted M12 work. The source audited and
tested is `main` at `ef27465a157d427cab02f562b4503604adb607d2`. This review adds no
production Rust change, dependency, public operation, DTO, session rule, capacity,
or presentation/scripting behavior.

## 1. Exact accepted commits

| Evidence/change | Commit |
| --- | --- |
| M12.1 architecture audit and acceptance | `84aac0d8f6b8138a86ddaafca4c700eb4bf571be` |
| M12.2 transport-neutral seam | `0e1bcb228f068eb1fcec6116eebc64f3352e520d` |
| M12.2 acceptance / M12.3 authorization | `e20f74997346f33df718612992dd36e549113291` |
| M12.3 bounded WebSocket transport, including accepted review remediation | `6fb867389426fa75033f54312fda8c5556c52ed7` |
| M12.3 acceptance / M12.4 authorization | `aace1f25de873db11f68e7db364e570af3d0d920` |
| M12.4 parity/fault acceptance implementation and tests | `9b58e92cffa79087f60d78032bc34b89499ae961` |
| M12.4 acceptance / M12.5 authorization | `5d04aa9920a86feec12f577649b9dd2881ab4a8c` |
| M12.5 real-browser ClojureScript smoke | `f291edf35a7805245ce19ea088b9ee899d57f30e` |
| M12.5 acceptance / consolidated-review authorization | `ef27465a157d427cab02f562b4503604adb607d2` |

M12.3 review corrections were made before the single M12.3 implementation commit;
there is therefore no separate remediation commit. The implementation report keeps
the original review finding and correction history. The same applies to the M12.4
review corrections made before its accepted implementation/test commit.

The published `v0.1.0-preview.1` tag remains unchanged at
`70abaf6136a8baa93dfe31aa5d8a7cc56e54ef6e`. The published artifact is the earlier
TCP/NDJSON-only preview; the accepted WebSocket work is post-preview on `main`.

## 2. Final source architecture

The final source has this shape:

```text
TCP listener / TcpStream / NDJSON framing ----+
                                               |
                                               +-> one network reactor
WebSocket listener / RFC 6455 / JSON message --+       |
                                                       v
                                             ConnectionCoordinator
                                             ClientDelivery / OwnerDelivery
                                                       |
                                                       v
                                           shared Application JSON codec
                                                       |
                                                       v
                                            ONE serialized Application
                                                       |
                                                       v
                                             ONE embedded SessionStore
                                                       |
                                                       v
                                          ServiceHost / HostCore / Runtime
```

Source proof:

- `apps/lab-runtime/src/server.rs::run` creates one production
  `Application::new(...)`. The owner loop alone calls `Application::handle(...)` and
  `Application::detach(...)`; those calls are serialized.
- `apps/lab-runtime/src/application.rs` contains the one `SessionStore` field used by
  that Application. `apps/lab-runtime/src/sessions.rs` supplies the single session,
  request-deduplication and retained-outcome model.
- `apps/lab-runtime/src/protocol.rs` contains the one authoritative `OPERATIONS`
  registry and capability projection: 42 operations and 25 capabilities. It also
  contains the one public error mapping.
- `apps/lab-runtime/src/wire.rs` contains the one `WireRequest`/`WireRequestId` DTO
  model and the shared bounded Application JSON decoder/encoder. NDJSON framing and
  WebSocket message mechanics sit above it.
- `apps/lab-runtime/src/server.rs` owns one `ConnectionCoordinator`, one pair of
  bounded owner/reactor mailboxes and one reactor servicing both listeners. Both
  transports use `ClientDelivery` and `OwnerDelivery`; neither has an alternate
  Application call lane.
- Events and subscriptions use the one Application event log and the one
  connection-local subscription model. Recorder history uses the one Application
  history projection over the unchanged Recorder/SQLite ownership.

The source contains no `Arc<Mutex<Application>>`, second production `Application`,
second `SessionStore`, per-transport registry/DTO/error taxonomy, per-client Runtime
thread, async runtime, or transport-owned experiment state. The WebSocket adapter is
synchronous and nonblocking inside the existing reactor. Client transport identity
does not enter Application connection IDs or semantic requests.

## 3. Final dependency and browser-security audit

### Dependency

The pinned dependency is:

```toml
tungstenite = { version = "=0.30.0", default-features = false, features = ["handshake"] }
```

`Cargo.lock` resolves exactly `tungstenite 0.30.0` with checksum
`e48ac77174b19c110a50ab2128b24215ac9cb40e0e12e093fb602d175c569d22`.
`cargo tree -e features -i tungstenite` confirms only `handshake` and its required
data-encoding, HTTP parsing and SHA-1 handshake support. TLS, URL client helpers,
compression, Tokio and other async-runtime features are absent.

The crate declares Rust 1.85 as its MSRV, edition 2021, and dual MIT OR Apache-2.0
licensing. The final gate ran with Rust 1.95.0. A package-specific RustSec review on
2026-09-28 found only RUSTSEC-2023-0065 for Tungstenite; it affects versions through
0.20.0 and was patched in 0.20.1, so pinned 0.30.0 is outside its affected range.
`cargo-audit` was not installed, so this is not represented as a fresh whole-lockfile
`cargo audit` run. Sources: <https://rustsec.org/packages/tungstenite.html> and
<https://rustsec.org/advisories/RUSTSEC-2023-0065.html>.

### Endpoint policy

Source and focused raw-handshake tests confirm:

- an optional, separate IPv4 `127.0.0.1` WebSocket listener;
- exact path `/application/v1`, with query strings rejected;
- `GET HTTP/1.1`, RFC 6455 version 13 and a valid WebSocket key;
- one exact numeric `Host: 127.0.0.1:<actual-port>`; duplicate Host is rejected;
- exactly one `Origin`, byte-for-byte equal to a validated configured allowlist
  entry; missing, `null`, duplicate and foreign origins are rejected;
- required subprotocol `lab-runtime.application.v1` and no alternate Application
  protocol;
- no negotiated extensions or compression;
- masked client frames and Text-only Application messages.

Origin validation is local-browser cross-site WebSocket-hijacking protection, not
authentication. M12 remains local-first and unauthenticated, matching the native
loopback TCP endpoint. It makes no remote, Internet-facing, TLS or authentication
claim. Any non-loopback binding would require a separately reviewed threat model,
authentication/authorization design, confidentiality/integrity transport, proxy and
DNS/Host handling, origin policy, operational rate limits and deployment guidance.

## 4. Final boundedness and ownership audit

| Bound/state | Final value | Owner and behavior |
| --- | ---: | --- |
| Global TCP + WebSocket clients | 8 | One `ConnectionCoordinator`; accepted/pre-Upgrade sockets and detach-pending generations consume the same pool. |
| Connection IDs | `u64`, start at 1 | Coordinator uses checked monotonic allocation; process-local, opaque, never reused; exhaustion fails explicitly. |
| Application JSON body | 16,383 bytes | Shared JSON codec; applies to both transports. |
| TCP NDJSON frame | 16,384 bytes including LF | TCP framer; LF/CRLF behavior remains the accepted TCP contract. |
| JSON nesting | 16 | Shared decoder. |
| JSON values/members | 1,024 | Shared decoder. |
| JSON string/key | 512 bytes | Shared decoder. |
| HTTP Upgrade bytes | 8,192 | WebSocket handshake wrapper; fail/close boundedly. |
| HTTP Upgrade headers | 32 | WebSocket handshake wrapper. |
| Allowed Origins | 16 | Deployment validation. |
| One Origin value | 256 bytes | Deployment validation/handshake policy. |
| Tungstenite read allocation | 4 KiB | WebSocket configuration. |
| Tungstenite initial write buffer | 0 | WebSocket configuration; writes are driven nonblockingly. |
| Tungstenite maximum write buffer | 32 KiB | WebSocket configuration; transport control/frame work remains finite. |
| WebSocket message/frame | 16,383 / 16,383 bytes | Explicit `WebSocketConfig` overrides Tungstenite's 64 MiB message and 16 MiB frame defaults. |
| Accepted-but-unflushed Application write | exactly 1 | WebSocket peer tracks one reply/event identity after Tungstenite accepts it; it continues consuming the shared reply/event slot until successful flush. |
| Fragment observer | 14-byte header buffer, one optional payload counter and one timestamp | Transport-local observer detects incomplete data-message lifetime without copying/assembling a second message or confusing standalone control frames. |
| Owner -> reactor mailbox | 64 | Bounded synchronous channel; nonblocking pressure handling. |
| Reactor -> owner mailbox | 64 | Bounded synchronous channel; nonblocking pressure handling. |
| Per-client admitted input (`CLIENT_IN`) | 8 | Shared delivery/admission state. |
| Per-client reply capacity (`CLIENT_OUT`) | 8 | Shared `ClientDelivery`, including the WebSocket current write. |
| Per-client event capacity (`CLIENT_EVENTS`) | 16 | Shared `ClientDelivery`, including the WebSocket current write. |
| TCP I/O budget per peer/turn | 8 KiB read + 8 KiB write | One reactor; finite fair work. |
| Open WebSocket I/O budget | 8 KiB read + 8 KiB write per normal open-peer service phase | Reset by `begin_turn()` for the open-peer phase. |
| WebSocket handshake I/O budget | 8 KiB read + 8 KiB write | Finite `BoundedStream` budget for `advance_handshake()`. |
| Handshake -> Open transition | one handshake budget, then one normal open-peer budget | Both may be consumed in the single service invocation that completes the transition. The transition does not repeat and does not multiply client, queue, message or retained-state capacity. |
| WebSocket messages per turn | 4 read / 4 write | WebSocket adapter; control traffic cannot monopolize a turn. |
| TCP/WS accept work per sweep | 4 / 4 when WS enabled | Reactor fairness budget, not separate capacity pools. |
| Owner incoming drain | at most 16 per turn | HostCore service occurs before bounded client work. |
| Fair request dispatch | at most 4 connections per owner turn | Shared scheduler. |
| Owner-delivery drain | at most 64 per turn | Bounded by the shared mailbox capacity. |
| Event log | 1,024 records | One Application-owned retained ring. |
| Upgrade/hello/incomplete input/output/close deadlines | absolute 2 seconds | Per-connection monotonic deadlines; trickled bytes or control frames do not reset them. |
| Shutdown terminal-delivery window | 200 ms, best effort | Shutdown cannot wait indefinitely for a client or WebSocket close. |

Adding WebSocket did not multiply any semantic queue, owner mailbox, retained event
ring or client pool. Automatic Pong/Close work is transport-local but finite and is
included in reactor work and output deadlines. A slow or malicious network peer
cannot make required HostCore/Runtime/Recorder progress wait indefinitely.

## 5. Session, reconnect and deduplication audit

The accepted M12.4 parity suite proves both TCP -> WebSocket and WebSocket -> TCP
reattachment to the same retained scope. It preserves the existing exact-detach
race: a new connection can receive `scope_in_use` until the serialized owner has
processed the old connection's detach. Tests synchronize on authoritative
reattachment, not a weakened immediate-reconnect promise.

Across a successful transport migration:

- scope and boot identity follow the one `SessionStore` contract;
- `next_seq` remains authoritative and consecutive;
- retrying the same request ID and normalized mutation returns its retained state or
  outcome without executing again;
- the same request ID with different normalized mutation data returns the existing
  public `request_conflict` mapping;
- an evicted/unknown old operation remains unknown and is not reused or re-executed;
- `msg_id` is connection-local: in-flight duplication is rejected, terminal reuse
  is valid and the same value on another connection is independent;
- an admitted asynchronous Recorder mutation survives loss of its originating TCP
  client, is reconciled from WebSocket by `operation_status`, and exact retry returns
  the retained result without a second execution.

The sole M12.4 production correction changed an accidental
`request_id_conflict` string to the already-defined `request_conflict` public
mapping. This restored the accepted taxonomy; it did not add an error or alter
deduplication/session behavior.

These results directly preserve:

```text
client lifetime != experiment lifetime
```

## 6. Subscription, event and history audit

- A subscription belongs to its connection and is removed on detach. It does not
  silently migrate with a retained scope.
- The bounded event log and cursors are process/Application state. A newly attached
  connection on the other transport can create a fresh subscription from a retained
  cursor and receive ordered replay.
- TCP and WebSocket produce identical `event_gap` semantics, including exact
  `oldest`, `latest`, boot identity, `resync_required` and the 1,024-record retained
  window. Gap handling abandons the affected subscription under the one contract.
- Durable Recorder history is process-owned and returns equivalent semantic pages
  through both transports.
- Page/projection tokens are deliberately connection-local. A token from a detached
  connection is not made valid by reattaching the same scope through another
  transport; a fresh ordinary history query remains valid.
- Pending history work retains its accepted connection-local cancellation and
  `client_disconnected` terminalization behavior. It is not evidence for retained
  mutation lifetime and was not used as such.

## 7. Fault and isolation audit

Focused WebSocket tests, unchanged TCP regression suites and M12.4 composition tests
establish that malformed or incomplete TCP does not harm a healthy WebSocket peer,
and malformed/incomplete WebSocket traffic does not harm a healthy TCP peer.
Non-reading and slow peers in either direction remain bounded by the shared delivery
capacities, transport buffers, per-turn budgets and absolute deadlines. Owner
mailbox saturation is bounded and does not introduce a blocking send into the
Runtime owner path.

The coordinator retains a global slot while exact detach is pending; generation
fencing prevents a late close or stale owner message from freeing another
connection's slot or delivering stale output to a replacement connection. Every
normal close, protocol failure, reset, timeout or forced shutdown produces at most
one Application detach. WebSocket close progression and best-effort terminal output
cannot extend process shutdown indefinitely. Mixed TCP/WS shutdown retains
owner-priority Runtime safety/Recorder progress and finite reactor join behavior.

## 8. Real-browser acceptance

M12.5 contains a deliberately small external smoke client under
`clients/clojurescript-smoke/`. It is compiled ClojureScript using native
`js/WebSocket`, `js/Promise`, timers, JSON and minimal DOM status. It is not Node,
Tungstenite, a Rust client or a simulated browser.

The final-HEAD rerun used real Chromium (Google Chrome 153.0.8010.53) and a page
served from `http://127.0.0.1:9000`. The browser itself supplied that Origin and
negotiated `lab-runtime.application.v1` against
`ws://127.0.0.1:51408/application/v1`. The bounded browser-side state reached PASS
after observing hello, query, subscription, accepted/completed mutation, Application
event, clean first close, retained-scope reattach, terminal `operation_status`, exact
dedup retry, fresh-subscription replay, authoritative final query and clean second
close. DevTools was used only to read the final DOM result; it did not substitute for
the browser execution.

Observed final rerun facts:

```text
Runtime commit: ef27465a157d427cab02f562b4503604adb607d2
OS: Windows NT 10.0.19045.0
Java: 25.0.4.1 LTS
ClojureScript: 1.12.145
Browser: Google Chrome 153.0.8010.53
HTTP Origin: http://127.0.0.1:9000
TCP readiness port: 51407
WebSocket readiness port: 51408
Subprotocol: lab-runtime.application.v1
Boot ID: 88dea9fc0c9a35fd80dd43c72a71be73
Retained scope: 88dea9fc0c9a35fd80dd43c72a71be73:1
Result: PASS
Mutation accepted/completed: true/true
Application event observed: true
First/second clean close: true/true
Retained reattach/status/exact retry: true/true/true
Replay/final committed query: true/true
Final reference revision: 2
```

Generated JavaScript, Closure output, browser profile/cache, downloaded compiler
artifacts, logs and temporary HTTP content remain outside the repository. M12 adds
no bundled GUI, general client SDK or presentation schema.

## 9. Presentation and scripting boundary

A current source search found no Runtime-owned `PresentationDocument`, Steel VM,
ClojureScript runtime, egui/widget tree, window/tab/panel layout, plot, button or
slider semantics. The ClojureScript code is an external acceptance client only.

Search-word false positives were classified: `window` denotes bounded history or
sample windows, `Windows` denotes the operating system/COM, `row` and `column`
denote SQLite/data records, and `workspace` denotes Cargo workspace structure. None
is a presentation model.

M12 preserves:

```text
Runtime owns experiment semantics.
Client owns presentation semantics.

script lifetime != experiment lifetime
client lifetime != experiment lifetime
```

M12 introduced neither executable scripting inside Runtime nor a client authority
to fabricate physical evidence, ACK, readback, transport completion or safety state.

## 10. Final verification

All commands ran from final accepted M12 HEAD
`ef27465a157d427cab02f562b4503604adb607d2` before this uncommitted review document
and coordination transition:

| Command | Result |
| --- | --- |
| `cargo fmt --all -- --check` | PASS |
| `cargo test --workspace --locked` | PASS |
| `cargo test --workspace --release --locked` | PASS |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | PASS |
| `cargo test -p lab-runtime --test websocket_transport --locked` | PASS, 13/13 |
| `cargo test -p lab-runtime --test transport_parity --locked` | PASS, 10/10 |
| `clients/clojurescript-smoke/run-smoke.ps1` | PASS in real Chrome; evidence above |
| `git diff --check` | PASS after the final report/coordination edits |

The two pre-existing opt-in rotation/soak workloads remain ignored and unchanged.
No test was weakened, unignored for cosmetic counts, or replaced with a non-browser
stand-in. The smoke compiler emitted Java/Closure `Unsafe` deprecation warnings only;
the compile and browser acceptance succeeded.

## 11. Documentation consistency

The active coordination files now agree that M12.1 through M12.5 are accepted, the
consolidated M12 review is ready for external review and M13 is not authorized. They
also agree on 42 operations, 25 capabilities, the one shared eight-client pool, the
optional loopback-only WebSocket endpoint and the absence of remote/TLS/auth claims.

`README.md` and `docs/` intentionally describe the immutable
`v0.1.0-preview.1` Developer Preview Reference and its packaged TCP/NDJSON-only
artifact. Their statements that the preview has no additional transport are correct
for tag `70abaf6...`, not a claim that post-preview `main` lacks WebSocket. Active
coordination explicitly distinguishes the published preview from the accepted M12
implementation on current `main`. Historical milestone reports were not rewritten.

No current document claims browser UI, remote deployment, TLS/authentication,
production certification or M13 authorization.

## 12. Scope conclusion and preserved invariants

The consolidated source, test and real-browser evidence is coherent: M12 adds a
second bounded loopback transport adapter to one Application contract. It does not
add a second semantic model or move experiment ownership to a transport/client.

| Area | M12 conclusion |
| --- | --- |
| Application operation semantics | Unchanged: one 42-operation registry and 25 capabilities. |
| DTOs | Unchanged: one shared wire/request model; no WebSocket DTOs. |
| Session semantics | Unchanged: one `SessionStore`, exact detach race, retained-scope and dedup rules. |
| Runtime ownership | Unchanged: one serialized owner; no Application mutex or per-client Runtime thread. |
| Recorder/SQLite semantics | Unchanged. Browser and parity tests consume the existing contract. |
| OutputAuthority | Unchanged and not bypassed by either transport. |
| Physical-safety claims | Unchanged; network success is not physical evidence. |
| Presentation ownership | Unchanged: client-owned; no Runtime UI/presentation schema. |

The transport-neutral event-size oracle introduced in M12.2 preserves the accepted
semantic event bound while removing an NDJSON dependency. The accepted M12.4
`request_conflict` one-line correction restores, rather than changes, the existing
public taxonomy. No other semantic change was found.

Required invariants remain:

```text
ONE Application API
ONE operation registry
ONE DTO model
ONE public error taxonomy
ONE SessionStore and session/dedup model
ONE subscription/event/history model
ONE serialized Application owner

TCP + WebSocket clients <= 8
bounded queues, histories, workers and transport state
network clients do not block required Runtime safety/control/Recorder progress

Runtime owns experiment semantics.
Client owns presentation semantics.
client lifetime != experiment lifetime
script lifetime != experiment lifetime

WebSocket disconnect does not own experiment lifetime
no raw transport ownership escapes Runtime
no client may fabricate physical evidence
no UI or scripting semantics enter Runtime core
```

No unreviewed M12 change is a blocker. M13 remains explicitly unauthorized.

```text
STATUS: M12_READY_FOR_EXTERNAL_REVIEW
```
