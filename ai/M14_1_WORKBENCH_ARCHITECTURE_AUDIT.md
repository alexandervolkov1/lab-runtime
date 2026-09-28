# M14.1 Workbench client architecture audit

## External-review acceptance

External review accepted M14.1 on 2026-09-28. The review-ready evidence below is
preserved as written. Its implementation gate is now recorded in active
coordination: M14.2 is authorized, M14.3 is not authorized, and M13.2 remains
blocked on Steel dependency safety.

## Status and conclusion

```text
M12: ACCEPTED

M13.1: ACCEPTED
M13 dependency safety resolution: ACCEPTED
M13.2: BLOCKED / NOT AUTHORIZED

M14.1 Workbench architecture audit: READY FOR EXTERNAL REVIEW
M14.2: NOT AUTHORIZED
```

This is a read-only architecture and dependency audit against `main` at
`3c654e8b54d3654a92b1425f0bb7d607dcf1ebc4`. It adds no crate, dependency,
executable, Rust source, test, Application operation, DTO or Runtime behavior.

The current Application contract is sufficient for the first Workbench slices.
No missing GUI requirement requires a new Runtime/Application semantic operation.
The smallest coherent architecture is:

```text
lab-workbench.exe

GUI/main thread
  owns PresentationDocument, operator interaction and rendered view state
        |
        | bounded commands / bounded updates
        v
one Application client worker
  owns TCP/NDJSON, hello/scope, msg_id, request_id sequencing,
  request correlation, subscription/replay and reconnect
        |
        | existing public Application protocol
        v
lab-runtime.exe
  ONE Application / ONE SessionStore / authoritative Runtime
```

Use TCP/NDJSON as the initial native-client transport, keep the client
implementation private to the Workbench executable, store presentation state in a
bounded versioned client-owned JSON document, and use a dedicated non-blocking
network worker. The initial GUI dependency candidate is exact `eframe 0.36.2` with
`glow`, `default_fonts` and `accesskit`, plus exact `egui_plot 0.37.0`, all with
default features disabled. These are recommendations for later review, not
dependencies added by M14.1.

Future Steel, if its dependency gate is later cleared, belongs on a dedicated VM
thread inside Workbench and uses the same client command boundary as GUI actions.
That weakens Steel failure isolation from a separate process to the Workbench
process, but cannot weaken the Runtime boundary:

```text
Runtime owns experiment semantics.
Client owns presentation semantics.

client lifetime != experiment lifetime
GUI lifetime != experiment lifetime
script lifetime != experiment lifetime
```

## 1. Audit basis and current source boundary

### Repository state

| Item | Audited value |
| --- | --- |
| Branch | `main` |
| Audit base | `3c654e8b54d3654a92b1425f0bb7d607dcf1ebc4` |
| Accepted M13 dependency-resolution commit | `7a150cd8e15d990e00ad62b5c9d9b66c401b7086` |
| Published preview tag | `v0.1.0-preview.1` |
| Preview packaged-source commit | `70abaf6136a8baa93dfe31aa5d8a7cc56e54ef6e` |
| Workspace toolchain | Rust/Cargo 1.95.0, edition 2024 |
| Audit host | Windows 10 Pro 10.0.19045, x86_64-pc-windows-msvc |

The pre-existing user `.gitignore` modification and untracked external-review text
files were not read, modified or staged as M14.1 work.

### Source and current references inspected

The audit used current source as authority, especially:

- `apps/lab-runtime/src/protocol.rs` — the authoritative 42-operation registry,
  25 capabilities, public errors and advertised bounds;
- `apps/lab-runtime/src/application.rs` and its `application/*` modules — the one
  Application facade, query/mutation dispatch, projections and delivery;
- `apps/lab-runtime/src/sessions.rs` — scope attachment, retained mutation identity,
  deduplication, operation status and retention;
- `apps/lab-runtime/src/events.rs` and `application/delivery.rs` — event log,
  subscription, replay and gap behavior;
- `apps/lab-runtime/src/measurements.rs`, `configuration_api.rs`, `recorder.rs` and
  `application/recording.rs` — current/recent data, properties and durable history;
- `apps/lab-runtime/src/server.rs`, `server/coordination.rs`, `wire.rs` and
  `server/websocket_peer.rs` — accepted transports, connection delivery and bounds;
- `docs/application-api.md`, `docs/architecture.md`, `README.md` and the accepted
  M12/M13 reports;
- root and package `Cargo.toml` files and the current lockfile.

There is no production Rust Application client library. The Rust TCP/WS clients in
integration tests are test infrastructure, and the ClojureScript smoke is a small
external acceptance client. The workspace still has only `lab-core` and
`lab-runtime` members.

A source search found no Runtime presentation or scripting ownership. Matches such
as measurement `window`, moving-mean `window`, test byte `windows`, diagnostic
`TRACE`, and Cargo `workspace` are domain or implementation terms, not GUI state.
There is no `eframe`, `egui`, `PresentationDocument` or Steel dependency in current
production source.

## 2. What the current Application API already supplies

`hello` is the Workbench's runtime-negotiation source. It returns the protocol and
Application versions, boot identity, attached or allocated scope, authoritative
`next_seq`, feature-filtered operations and capabilities, actual limits, and event
oldest/latest cursors. The client must consume this response rather than embed a
second authoritative registry.

| Workbench need | Existing source/API | Assessment |
| --- | --- | --- |
| Hello, feature negotiation and limits | `hello` | Complete. Operations, capabilities and bounds are runtime-advertised. |
| Discovery | `discover`, `discovery_page`, `describe` | Stable instrument, signal, component, controller, reference, output, resource and configuration-property identities/descriptors. |
| Instrument descriptors/state | `describe`, `latest`, `measurements_current`, `measurements_page`, `measurement_window` | Complete for metadata, current state and bounded recent signal data. Queries read committed state and do not poll hardware. |
| References | `reference`, `reference_configure`, `reference_retune` | Current target/revision plus accepted mutation lifecycle. |
| Controllers | `controller`, configure/PID/start/pause/resume/reset mutations | Current lifecycle/configuration plus existing mutation semantics. |
| Managed components | Discovery/describe and `component` projection | Enough to display native component descriptors and current projections. No executor access is exposed. |
| Resource state | `resource`, discovery and `reconnect_resource` | Current state and explicit reconnect mutation. |
| Configuration/properties | `configuration_status`, `configuration_properties`, `configuration_page`, `configuration_stage`, `configuration_apply`, `configuration_reload`, `property_configure` | Enough for status, typed property editing and the accepted stage/apply/reload lifecycle. |
| Recorder | `recording_status`, start/stop/annotate | Complete authoritative status/control lifecycle. |
| Durable history | `history_read`, `history_page`, `history_release` | Bounded Recorder-backed runs/measurements/events with connection-local cursors. |
| Live updates | `subscribe`, `unsubscribe` | One connection-local filtered subscription with replay, progress and gap semantics. |
| Mutation lifecycle | accepted replies, terminal result, `operation_status` | Complete. Admission, terminal state, dedup and reconciliation are explicit. |
| Virtual/demo operations | `emulator_publish`, `virtual_models_restart` | Existing explicitly virtual authority only. No physical evidence fabrication. |
| Runtime shutdown | `runtime_shutdown` | Existing explicit mutation; GUI close must not invoke it implicitly. |

The registry remains exactly 42 operations and 25 capabilities. The initial
Workbench must not generate a second 42-method semantic API or hard-code a competing
capability list. Small local request builders may validate UI input, but only the
Runtime response is authoritative.

Relevant current bounds are already returned by `hello` and must constrain the
client rather than be independently enlarged:

| Runtime/Application resource | Current bound |
| --- | ---: |
| TCP NDJSON frame / Application JSON body | 16,384 bytes including LF / 16,383 bytes |
| JSON depth / values / string or key bytes | 16 / 1,024 / 512 |
| Global TCP + WS clients | 8 |
| Owner/reactor mailbox; per-client pending/reply/event queues | 64; 8 / 8 / 16 |
| Client absolute partial/blocked deadline | 2 seconds |
| Retained scopes | 16 |
| Pending operations per scope / total | 8 / 64 |
| Terminal operations per scope / total | 32 / 256 |
| Terminal / detached-scope retention | 600 / 1,800 seconds |
| Event replay ring / encoded event | 1,024 records / 4,096 bytes |
| Subscriptions per connection; filter kinds/targets | 1; 8 / 16 |
| Discovery/current/config page | 64 records and 8 KiB where applicable |
| Recent/durable history page records; history cursors | 128 / 128; 8 |

These are server admission/retention bounds. Workbench-local render buffers and UI
queues must also be finite but cannot reinterpret, multiply or promise longer
authoritative retention.

### Gaps and deliberate limits

No M14-blocking Application gap was found. Two limits need honest product treatment:

1. Signals have current projections, a bounded recent Runtime window, live/replayed
   events and Recorder-backed durable history. References, controllers and outputs
   have current projections plus live/replayed events, but no dedicated long-range
   durable-history query. The minimal M14 live plot can use signals; Workbench may
   retain a bounded client-side live window for other projections. If a future
   requirement demands durable non-signal traces, that is a separately reviewed
   Application requirement, not a GUI convenience to invent in M14.
2. Configuration operations expose typed property records and the existing
   server-side stage/apply/reload lifecycle. They do not accept an arbitrary
   client-uploaded deployment manifest. An editor may change exposed properties and
   operate the accepted lifecycle; a general manifest upload/editor would be a new
   product/API requirement and is outside M14.

Friendly labels, layout labels, unit display overrides and grouping are client
presentation concerns and do not constitute missing Runtime semantics. Stable IDs,
not those labels, bind UI objects to Runtime entities.

## 3. Process, trust and ownership boundary

The Workbench is an ordinary external client:

```text
lab-workbench.exe                  lab-runtime.exe

presentation and cache        ->  authoritative experiment state
operator requests             ->  Application admission/session/dedup
local persistence             X   Recorder/SQLite
```

Workbench owns:

- connection/reconnect mechanics and its local view of scope/session metadata;
- windows, tabs, panels, splits, plots, controls and formatting;
- observational projections and history-page caches;
- operator-edit drafts and pending-command display state;
- presentation files and a separate bounded recovery journal;
- later, editor state and an optional Steel VM.

Runtime remains authoritative for:

- instrument truth, measurements, Reference target/revision and controller state;
- OutputAuthority, physical ACK/readback/effect and all safe-state evidence;
- Runtime/Recorder lifecycle, durable history and configuration state;
- Application scopes, request admission/deduplication and operation outcomes;
- event log/cursors and resource state.

A cached value never becomes evidence merely because it is visible. “Last received”
must be distinguishable from “currently authoritative,” and disconnected/stale
views must be explicit. Closing or killing Workbench closes a client; it must not
send `runtime_shutdown`, stop a controller, stop recording or undo an admitted
mutation unless the operator explicitly requested the corresponding existing
operation.

## 4. Recommended Workbench internal architecture

```text
                 future, optional
                 Steel VM thread
                       |
                       | bounded lab commands / validated ui commands
                       v
+---------------- lab-workbench.exe --------------------------------+
|                                                                 |
| GUI/main thread                                                 |
|   PresentationDocument + UI state + immutable render snapshots |
|        |                                      ^                 |
|        | bounded commands                     | bounded updates |
|        v                                      |                 |
| one Application client worker ---------------------------------+|
|   TcpStream / NDJSON / deadlines                                ||
|   hello + retained scope + boot_id                              ||
|   checked msg_id + pending exchanges                            ||
|   serialized request_id.seq + recovery journal                  ||
|   one subscription + event cursor + projection assembly         ||
|   reconnect / resnapshot / gap recovery                          ||
+--------|---------------------------------------------------------+
         | existing public TCP/NDJSON Application protocol
         v
 lab-runtime: ONE Application / ONE SessionStore / Runtime
```

### Single owners

| Concern | Owner |
| --- | --- |
| Socket, framing, partial I/O, deadlines | Application client worker |
| Retained `scope`, `boot_id`, authoritative `next_seq` view | Application client worker |
| Checked connection-local `msg_id` allocation/correlation | Application client worker |
| Serialization of mutation `request_id.seq` | Application client worker |
| Subscription token, scan cursor and replay/gap handling | Application client worker |
| Protocol-to-client projection assembly | Application client worker |
| Presentation layout and operator interaction | GUI/main thread |
| Rendering | GUI/main thread |
| Authoritative experiment state | Runtime process only |
| Future Steel evaluation | One dedicated Workbench VM thread |

GUI and future Steel send the same typed internal command enum to the one client
worker. They do not open independent hidden Application sessions. This prevents two
client-side owners from racing `request_id.seq`, losing ambiguity/retry state or
creating different event views. Multiple explicitly operator-created Runtime
connections could be designed later, but are not part of the minimal Workbench.

The worker should own protocol state and publish immutable/copyable client
projections or bounded deltas. The GUI owns its render model. Do not put the socket,
presentation tree, pending operations and all caches behind one
`Arc<Mutex<WorkbenchEverything>>`; it would hide ownership and let rendering or file
work delay connection progress.

### Correlation and mutation identity

The worker allocates `msg_id` from a checked monotonically increasing local counter.
It is correlation for one connection only, is unique while an exchange is in flight,
may be reused only after that exchange terminates, and is discarded on reconnect.
The same `msg_id` on a later connection has no deduplication meaning.

Runtime supplies `scope` and `hello.next_seq`. The worker is the sole allocator of
mutation `{scope, seq}` for both GUI and future Steel commands. The minimal client
should serialize mutation admission: do not advance its local sequence until
authoritative admission, and do not allocate a later sequence while an earlier send
has an ambiguous outcome. After disconnect it reattaches the retained scope, checks
`next_seq`/`operation_status`, and retries only the exact normalized prior request
where the accepted contract permits it. GUI widgets and scripts never invent a
sequence to bypass this state machine.

```text
msg_id = connection-local correlation
request_id = retained Application mutation/dedup identity
```

Neither identity creates a Workbench-specific exactly-once guarantee. Runtime
retention, conflict, eviction, old-boot and `outcome_unknown` behavior remains
authoritative.

## 5. Transport decision

Use TCP/NDJSON for the initial native Rust Workbench.

| Criterion | TCP/NDJSON | WebSocket/JSON |
| --- | --- | --- |
| Application semantics | Same accepted contract | Same accepted contract |
| Native dependency | `std::net` plus existing JSON stack | Requires a WS client dependency and protocol state |
| Framing | Bounded LF/CRLF stream frames | Text messages, fragments and control frames |
| Browser policy | Not applicable | HTTP Upgrade, Host, path, subprotocol and browser Origin policy |
| Diagnostics | Plain bounded frames are easy to capture | More framing/control state |
| Reconnect/session | Identical Application rules | Identical Application rules |
| Browser reuse | Not browser-native | Browser-native, already accepted for browser clients |
| Testability | Existing test-client patterns and raw sockets | Existing parity tests, but more mechanics |

TCP avoids carrying browser-specific handshake/Origin mechanics into a native
client, adds no Tungstenite client requirement, and has already-proven semantic
parity with WebSocket. It still consumes one slot from the same global
`TCP + WS <= 8` server pool.

WebSocket remains an accepted endpoint for browser clients, not a rejected design.
The private client boundary should keep Application envelopes independent from the
NDJSON reader so a later justified adapter is possible, but M14 should not create a
generic transport trait/plugin framework or a third transport.

## 6. Client code placement

Choose a Workbench-private client implementation under the future
`apps/lab-workbench` executable. Do not add a public `lab-client` crate in the first
slice.

The private module needs only:

- bounded TCP connect/read/write and NDJSON framing;
- generic JSON request/result/error/event envelopes;
- hello, correlation, scope/reconnect and operation reconciliation;
- small typed stable-identity helpers used by Workbench projections.

It must not copy the server operation registry, duplicate the public error taxonomy,
depend on `lab-runtime`, depend on `lab-core`, or manufacture a second hierarchy of
all server DTOs. Server `hello`, documented JSON and received error/result envelopes
remain authoritative.

Future Steel inside the same executable is a consumer of this private client, not a
second accepted native program requiring a reusable SDK. Extraction should be
revisited only after a second real native executable needs it.

## 7. Three kinds of Workbench state

### A. Authoritative Runtime state

Examples include Reference revisions/targets, controller lifecycle, measurements,
Recorder state, OutputAuthority/resource/configuration projections and operation
outcomes. Runtime owns this state. Workbench may display only a timestamped/cursored
observation and must visibly mark it stale on disconnect or gap.

### B. Recoverable client cache and recovery journal

Examples include last snapshots, event cursor, connection health, pending request UI
state, displayed history pages, and the minimum mutation-reconciliation record:
`boot_id`, retained `scope`, authoritative `next_seq`, plus the exact normalized
request ID/payload for any ambiguous admitted-or-sent mutation.

This state is not experiment authority. Most cache entries can be discarded and
rebuilt. Losing an ambiguity record reduces what the client can safely reconcile; it
does not permit assuming success/failure or allocating around an uncertain request.
Connection-local `msg_id`, subscription token, frozen-page token and history cursor
are never restored as if valid on a new connection.

### C. Persistent presentation state

Layout, open panels/plots, trace selection, colors, labels, axes, time windows,
visibility, window position and operator dashboard arrangement are Workbench-owned.
They may survive both Runtime and Workbench restarts, but they never assert a
controller state, safe output, durable commit or physical fact.

Keep presentation persistence separate from the recovery journal. This prevents a
portable dashboard document from silently carrying live session/dedup identity and
lets either file be discarded independently.

## 8. PresentationDocument and persistence direction

### Conceptual schema

The future client-owned document should be a tagged/versioned plain-data model,
conceptually:

```text
PresentationDocument
  format_version
  document_id
  windows[]
    placement
    tabs[] / split layout
      panel
        kind: discovery | status | plot | history | properties | controls
        title / visibility / display formatting
        references[]
  plots[]
    axes / viewport / time window
    traces[]
      source: typed stable Runtime identity
      label / display unit / color / style / visibility
  controls[]
    action template + stable target + confirmation presentation
```

Examples of valid content are “show controller 7 in panel A,” “plot signal
`{instrument, parameter}`,” and “invoke existing `recording_start` from this
control.” Invalid content includes “controller 7 is Running,” “output is Safe,” or
“this recording is durably committed.” Those values come only from current
Application projections/events.

References should be typed stable identities: signal identity, Reference ID,
controller ID, resource ID, component ID or configuration-property owner/property.
Display names are mutable labels and must not become identity. A missing target
renders as unresolved/stale with an explanation; it is not synthesized.

### File format

Recommend versioned JSON for M14.3:

- the project already uses `serde`/`serde_json` and JSON value semantics;
- it is human-inspectable and language-neutral for later Steel UI bindings;
- it avoids adding TOML/EDN/RON merely for presentation persistence;
- it does not couple the model to eframe's optional persistence feature.

Use a Workbench-owned per-user path, never Runtime's Recorder SQLite database or an
experiment archive. Loading must read a bounded file, parse into a candidate,
validate version/cardinality/string limits and stable-reference shapes, then replace
the active document atomically. Saving should write a sibling temporary file,
flush/close it, and replace only after successful serialization; a failed save leaves
the prior document intact.

Provisional limits to freeze and test in M14.3 are a 1 MiB presentation file, 64
windows, 64 panels, 32 plots, 32 traces per plot and 512-byte strings. These numbers
are proposals, not current contract. The separate recovery journal should be much
smaller (provisionally 64 KiB) and retain at most the server's eight pending
operations for this client. M14.3 must justify final values and corruption/upgrade
behavior before implementation acceptance.

Steel does not determine this format. A later `ui/*` binding manipulates the
validated in-memory model and uses the same persistence path; it does not parse and
replace files behind Workbench ownership.

## 9. GUI dependency audit

Audit date: 2026-09-28. No dependency was added. Current stable candidates were
checked from crates.io/docs.rs/upstream source and in isolated temporary Windows
probe crates outside the repository.

| Package | Exact candidate | License | Edition / MSRV | Role and relevant facts |
| --- | --- | --- | --- | --- |
| `eframe` | `0.36.2` | MIT OR Apache-2.0 | 2024 / Rust 1.95 | Native egui application/window integration. Supports both glow and wgpu. Optional persistence uses serde/RON and is not recommended for PresentationDocument. |
| `egui` | `0.36.2` (through eframe) | MIT OR Apache-2.0 | 2024 / Rust 1.95 | Immediate-mode UI. `Context` is cloneable and `request_repaint` may be called from another thread. |
| `egui_plot` | `0.37.0` | MIT OR Apache-2.0 | 2024 / Rust 1.95 | Immediate-mode 2D plot API; depends on `egui ^0.36.0`, compatible with eframe 0.36.2. Its `serde` feature is unnecessary for the proposed own schema. |

Candidate future declarations, subject to M14 implementation review, are:

```toml
eframe = { version = "=0.36.2", default-features = false,
           features = ["glow", "default_fonts", "accesskit"] }
egui_plot = { version = "=0.37.0", default-features = false }
```

This deliberately excludes eframe defaults such as wgpu, links, Wayland/X11 and
web screen-reader support for the initial Windows-native executable. The exact
eframe 0.36.2 default set is `accesskit`, `default_fonts`, `links`, `wayland`,
`web_screen_reader`, `wgpu`, `winit/default` and `x11`; it must not be inherited
blindly. `egui` defaults to `default_fonts`, while `egui_plot` has no default
feature. `accesskit` is retained for native accessibility integration and
`default_fonts` supplies a usable baseline font set. File-link launching, dialogs
and a general asset framework are not needed for the first vertical slice.

### glow versus wgpu

Two isolated probes used identical `eframe`, font, accessibility and `egui_plot`
requirements, differing only in renderer:

| Probe | Windows target dependency identities | Full lock identities | Windows `cargo check` |
| --- | ---: | ---: | --- |
| glow | 118 | 344 | Passed with Rust 1.95.0 |
| wgpu | 146 | 376 | Passed with Rust 1.95.0 |

The glow graph uses `egui_glow 0.36.2`, `glow 0.17.0`, `glutin 0.32.3` and
`winit 0.30.13`. The wgpu graph adds `wgpu 30.0.1`, core/HAL/shader machinery and
multiple backend support. Upstream itself notes that glow can significantly reduce
binary size. For a Windows laboratory Workbench rendering immediate-mode 2D plots,
glow is the smaller adequate first renderer. wgpu remains a future option if a
concrete rendering/driver requirement appears; enabling both initially is not
justified.

Both complete fresh lock graphs were batch-queried through OSV and returned zero
known advisories on the audit date. All resolved third-party packages declared a
license expression; the direct candidates are dual MIT/Apache-2.0. This is a
point-in-time result, not a substitute for repeating advisory/license review when
the dependency is actually added. The workspace currently compiles with Rust 1.95,
but has no declared `rust-version`; adopting these versions effectively establishes
a 1.95 floor and should be made explicit at the implementation review.

Both native probes built with the installed Rust/MSVC Windows toolchain and did not
require a separately installed native library during `cargo check`; deployed glow
still relies on the host graphics/window stack and an adequate OpenGL driver. The
workspace unsafe-code lint does not constrain transitive platform/renderer crates,
so the GUI process must not be described as unsafe-free or safety-authoritative.
`egui_plot` 0.37.0's own documentation says it is looking for a maintainer. That is
a maintenance risk to re-evaluate at M14.4, even though the pinned release is
license-compatible, compiled successfully and had no advisory in this audit.

Primary dependency sources:

- <https://crates.io/crates/eframe/0.36.2>
- <https://docs.rs/eframe/0.36.2/eframe/>
- <https://github.com/emilk/egui/releases/tag/0.36.2>
- <https://crates.io/crates/egui_plot/0.37.0>
- <https://docs.rs/egui_plot/0.37.0/egui_plot/>
- <https://github.com/emilk/egui_plot>
- <https://osv.dev/>

## 10. GUI/network thread model and bounds

The GUI thread must never call blocking connect/read/write, wait for an Application
terminal reply, load a large history page or evaluate Steel. One client worker owns
the network and uses finite monotonic connect/read/write/request/reconnect deadlines.
The GUI drains bounded updates during frames and renders local immutable/copied
state.

Provisional Workbench-local bounds for M14.2/M14.3 review:

| Resource | Proposed bound | Full/overflow behavior |
| --- | ---: | --- |
| GUI/Steel -> client commands | 32 total | `try_send`; reject locally as client busy and leave authoritative state unchanged. |
| In-flight Application exchanges | 8 | Match server per-client admission; reject locally before writing a ninth. |
| Client -> GUI ordered updates | 64 | Do not silently drop an ordered reply/event. Mark local projection stale, disconnect/reattach and perform bounded full resnapshot. |
| Active Runtime subscription | 1 | Existing server contract; one aggregate subscription, never one per widget. |
| Per-trace live points | 4,096 | Drop oldest client display samples with a visible local truncation marker; authoritative history remains in Runtime/Recorder. |
| Workbench diagnostic/status records | 256 records, 4 KiB each | Drop newest with a bounded dropped counter; never affect Runtime semantics. |
| Presentation file | provisional 1 MiB | Reject candidate before replacing active document. |
| Recovery journal | provisional 64 KiB / at most 8 pending mutations | Fail visibly; never erase ambiguity or infer completion. |

These queues are Workbench memory bounds, not new Runtime capacity. M14.2 must freeze
the exact connection deadlines and update representation with deterministic tests.
Queue overflow is an observable client fault/recovery transition, not permission to
block indefinitely or silently skip state.

The worker may hold a cloned `egui::Context` solely to call thread-safe
`request_repaint` after successfully enqueuing an update. The client protocol module
should otherwise be GUI-independent. A finite `request_repaint_after` fallback while
connected avoids relying on a lost wake-up; continuous busy repaint is unnecessary.

Shutdown order is:

1. GUI stops accepting new operator actions and sends a bounded worker-stop command;
2. worker stops reconnecting, best-effort closes the TCP connection, releases local
   tokens and reports termination;
3. GUI joins the worker within a finite deadline, persists presentation/recovery
   candidates if valid, and exits;
4. failure to deliver a client close or final UI update does not delay or mutate
   Runtime shutdown.

Closing Workbench never implies `runtime_shutdown`.

## 11. Event, snapshot and recovery model

No widget subscribes independently. The client worker owns one aggregate
subscription (empty filters mean all current event kinds/targets) and fans normalized
updates into the local projection/model.

A reconnect/resnapshot cycle should use an explicit state machine:

1. connect and `hello`, either attaching the retained scope or allocating a new one;
2. validate boot identity and `next_seq`; treat `scope_in_use` as the accepted detach
   race and retry only within a fixed deadline;
3. create a fresh connection-local subscription from an authoritative retained
   cursor where valid;
4. obtain frozen discovery/current pages plus direct reference/controller/resource/
   configuration/Recorder projections needed by the open presentation;
5. apply the coherent snapshot, then ordered events newer than its captured cursor;
6. publish a new client projection to the GUI;
7. on disconnect, local queue overflow, boot change or `event_gap`, mark projections
   stale, abandon connection-local tokens, and repeat full discovery/snapshot rather
   than guessing missed state.

M14.2 must refine and test the exact subscribe-versus-snapshot barrier so events
during bootstrap are neither lost nor applied twice. The existing Runtime event
cursor, frozen projections, replay and `event_gap` mechanisms provide the necessary
primitives; no new server operation is currently justified.

Connection-local state rebuilt after every connection includes `msg_id`, frozen-page
and history cursors, subscription token, socket buffers and pending delivery state.
Process/Application state that may be reconciled includes retained scope/dedup
records, admitted operation outcomes, Runtime projections, event ring and Recorder
history.

## 12. Plotting ownership and data sources

A plot is a Workbench presentation object referencing data by stable identity. It is
not a Runtime operation or experiment object.

| Trace source | Initial data path | Ownership note |
| --- | --- | --- |
| Signal | current/page/window, live measurement events, durable history | Runtime owns samples/history; Workbench owns viewport/downsampling/style. |
| Reference | current reference projection and reference events | Client may retain a bounded live display window; no durable reference-history claim. |
| Controller/output | current projection and lifecycle/output events | Display only the reported state; never infer physical effect or safe state. |
| Resource/config/Recorder status | current projection and events | Appropriate for status/timeline display, not fabricated samples. |

Workbench owns trace colors, display unit conversion, axes, time windows,
downsampling, clipping, cursor/legend behavior and renderer buffers. Downsampling is
presentation and must preserve visible indication when data is aggregated or
truncated. The durable source remains Recorder history, not the screen's ring.

No Runtime `Plot`, `Trace`, axis, viewport or rendering operation is needed.

## 13. Operator controls and configuration editing

Every operator control is a presentation affordance for an existing operation:

```text
operator intent
    -> bounded client command
    -> Application mutation with request_id
    -> accepted admission
    -> terminal completed/failed or reconnect reconciliation
    -> fresh authoritative projection/event
```

The visible lifecycle should distinguish `idle`, `pending_admission`, `accepted`,
`completed`, `failed` and `unknown/reconcile`. Socket write success is not admission;
an accepted reply is not terminal completion; an ambiguous disconnect is not
failure. Disable or annotate conflicting UI actions while the worker owns an
uncertain sequence, but never bypass Runtime sequencing to make the interface seem
responsive.

Examples map directly to existing operations: Reference retune/configure, controller
start/pause/resume/reset/configure, recording start/stop/annotate,
property/configuration lifecycle, resource reconnect and explicit Runtime shutdown.
Potentially consequential controls should use client-owned confirmation presentation,
not new Runtime semantics.

For properties/configuration:

1. clone an exposed typed property projection into a local edit candidate;
2. validate display/type/constraint input locally for feedback;
3. submit through `property_configure` with the accepted revision/conflict contract,
   or use existing `configuration_stage`/apply/reload for server-known deployment
   candidates;
4. wait for accepted/terminal lifecycle and refresh the authoritative projection;
5. on conflict, preserve the local draft separately and show the new authoritative
   revision.

UI widgets never hold Rust references into Runtime configuration, and editing one
field never partially mutates authoritative configuration before admission.

## 14. Workbench death, reconnect and restart

Required recovery behavior is:

```text
native controller + Recorder active
    -> Workbench connection/process dies
    -> Runtime/controller/Recorder continue under existing policy
    -> Workbench restarts
    -> load validated presentation + bounded recovery journal
    -> hello/reattach retained scope when boot identity permits
    -> tolerate serialized detach race
    -> operation_status / exact retry for ambiguous mutations
    -> fresh discovery/snapshots/subscription/replay
    -> event_gap or old boot => full resnapshot/new scope as required
    -> rebuild GUI
```

The recovery journal is a client aid, not a claim that scope survives beyond the
Runtime's 1,800-second detached retention or across boot identity. Terminal outcomes
remain bounded by Runtime retention (600 seconds and the documented per-scope/global
counts). If authoritative state reports `outcome_unknown`, Workbench must surface
that uncertainty and must not replay a different mutation with the old identity.

The old subscription, frozen pages, history cursor and `msg_id` table never survive
reattach. Presentation layout may survive indefinitely because it has no experiment
authority.

```text
GUI lifetime != experiment lifetime
```

## 15. Future Steel placement and M13.1 delta

Steel remains blocked on dependency safety and is not added by M14. Future accepted
placement should be:

```text
lab-workbench.exe

GUI thread -------------------+
                              |
future Steel VM thread -------+-> bounded client commands -> one Application client
          |
          +-> validated UI-model commands -> GUI/PresentationDocument owner
```

Two conceptual binding groups may later exist without freezing names now:

- `lab/*`: plain-data calls through the Workbench Application client;
- `ui/*`: validated mutations of WorkbenchModel/PresentationDocument.

Steel never calls egui directly and never calls Runtime directly. A script action
that changes an experiment competes/serializes at the same client command owner and
uses the same retained scope/request sequence as GUI operator actions. A UI action
becomes a validated model command applied by the GUI/model owner.

### Conclusions preserved from accepted M13.1

- plain-data JSON/Steel mapping and distinct result/accepted/terminal/event/error
  states;
- client-owned connection-local `msg_id` and Runtime scope/request dedup semantics;
- explicit reconnect, `operation_status` and exact-retry reconciliation;
- bounded event delivery with replay/`event_gap`, not unbounded script mailboxes;
- no physical-evidence, OutputAuthority, Recorder/SQLite, transport or Runtime
  handles;
- no managed-component role and no scripting semantics in Runtime.

### Changed by Workbench embedding

- there is no separate one-process-per-script `lab-steel.exe` in the current plan;
- script and GUI share one Workbench process and one Application-client owner;
- script lifetime can be shorter than the persistent Workbench connection;
- Steel can request client-only presentation changes through a separate UI boundary;
- process isolation is weaker: Steel panic, unsoundness or corruption may terminate
  or corrupt Workbench, though never the separate Runtime process.

A later accepted integration should use one dedicated VM thread, bounded command/
event/output queues, an operator Stop Script action, cooperative interruption only
if upstream behavior is verified, and replaceable VM state. Thread isolation is not
memory isolation and does not justify accepting the currently blocked Steel graph.
Optional future helper-process isolation may be reconsidered, but no IPC framework
is designed in M14.1.

## 16. Managed-component separation

```text
native managed component
  Runtime invokes bounded native Rust computation
  Runtime owns scheduling and lifecycle

future Workbench Steel
  user supervisory/client presentation procedure
  runs outside Runtime and requests public Application operations
```

Steel is not a `ComponentExecutor`. Workbench does not receive or implement the
managed-component invocation/result contract. Native real-time controllers and
managed components remain native Rust. M14 does not reopen Lua, dynamic plugins or
managed-component migration.

## 17. Risks and open questions

None is an architectural blocker for M14.2, but later slices must close these review
points before implementation acceptance:

1. Freeze M14.2 client deadlines, exact bootstrap barrier and update-queue overflow
   state machine with deterministic fault tests.
2. Decide the supported Runtime-launch/discovery UX. Reading readiness from a child
   Runtime is simple, but attaching to an independently running loopback Runtime
   needs an explicit configured/readiness-file workflow. This is client deployment,
   not a new transport or Runtime operation.
3. Freeze PresentationDocument cardinalities, file location, atomic-replace behavior,
   version migration and corrupt-file recovery in M14.3.
4. Define numeric/unit display conversion carefully. Display conversions must not
   rewrite authoritative units or mutation payload meaning.
5. Test OpenGL/glow on the supported Windows/driver baseline. Keep a reviewed wgpu
   fallback decision available only if real compatibility evidence requires it.
6. Decide whether Reference/controller/output long-range traces are needed. Current
   API supports live/replay client windows, not dedicated durable history for those
   projections.
7. Decide whether a future full deployment-manifest editor is a product requirement.
   Current accepted API deliberately exposes properties and server-side staging, not
   client upload of arbitrary manifests.
8. Explicitly declare the Rust 1.95 MSRV if the audited GUI versions are adopted.
9. Repeat full license/advisory resolution immediately before adding GUI
   dependencies; the point-in-time audit is not permanent approval.
10. Future Steel remains blocked until a separately accepted dependency re-review.

## 18. Proposed implementation decomposition

### M14.2 — minimal native Application client

**Goal:** add `lab-workbench` as a native, non-GUI executable/library boundary that
proves the one-worker TCP/NDJSON client lifecycle.

**Production change:** one workspace executable with private bounded client modules;
hello, representative query, accepted/terminal mutation, one subscription,
disconnect/reattach, status reconciliation and clean shutdown. No GUI dependency.

**Tests:** framing/codec bounds; msg_id reuse rules; request sequence serialization;
scope-in-use detach race; exact retry; event replay/gap; slow/non-reading Runtime;
queue saturation; finite worker shutdown; Runtime remains alive after client death.

**Non-goals:** public SDK, generated operation API, presentation schema, GUI, Steel,
new Runtime semantics.

**Review gate:** source proves one client owner, every queue/deadline/overflow rule is
documented, accepted server tests stay unchanged, and no `lab-runtime`/`lab-core`
dependency enters the client.

### M14.3 — WorkbenchModel and PresentationDocument

**Goal:** implement the three-state separation and client-owned persistence without
rendering complexity.

**Production change:** typed stable presentation references, bounded model,
versioned JSON, separate recovery journal, candidate validation and atomic replace.

**Tests:** round trip; all cardinality/string/file limits; corrupt/truncated/future
version; failed replace preserves old file; unresolved identities; Runtime snapshots
never serialized as presentation truth; pending mutation recovery records.

**Non-goals:** GUI renderer, Runtime SQLite use, arbitrary deployment upload, Steel.

**Review gate:** format/upgrade policy and all ownership/bounds are frozen; no
presentation DTO crosses into Runtime.

### M14.4 — minimal eframe/egui GUI

**Goal:** prove a responsive native GUI over the accepted client/model boundary.

**Production change:** audited pinned glow stack, connection/boot/scope status,
discovery view, one live signal plot and basic authoritative status panels.

**Tests:** GUI logic/model tests plus real Windows smoke; worker wake/repaint;
disconnect/stale display; bounded plot memory; no UI-thread network blocking.

**Non-goals:** full dashboard designer, every operation control, wgpu unless a
separate compatibility finding justifies it, Steel.

**Review gate:** dependency/license/advisory refresh passes, renderer works on the
supported Windows baseline, and Runtime continues after forced GUI termination.

### M14.5 — operator controls and properties/configuration

**Goal:** expose a small safe operator workflow using only existing operations.

**Production change:** Reference/controller/Recorder/resource and typed property /
configuration controls with explicit admission/terminal/reconcile states.

**Tests:** revision conflict; local validation; accepted versus terminal display;
ambiguous disconnect/reconcile; exact retry; no optimistic success; explicit-only
Runtime shutdown; no physical-evidence fabrication.

**Non-goals:** new server convenience operations, raw transport/output access,
arbitrary configuration-file upload, presentation scripting.

**Review gate:** each control maps to an existing documented operation and all
safety/authority language remains Runtime-derived.

### M14.6 — reconnect/recovery/fault acceptance

**Goal:** demonstrate `GUI lifetime != experiment lifetime` under realistic faults.

**Production change:** only corrections proven necessary by red acceptance tests.

**Tests:** Workbench kill/restart while controller/Recorder continue; old/new boot;
detach race; retained-scope operation reconciliation; fresh subscriptions and
replay/gap resnapshot; local queue overflow; corrupt presentation/recovery files;
slow network; finite shutdown; stale output cannot appear authoritative.

**Non-goals:** broaden Runtime semantics, physical qualification, remote/TLS/auth,
Steel.

**Review gate:** cross-process acceptance demonstrates Runtime continuity and a
complete bounded rebuild without weakening session/dedup/safety semantics.

### M14 consolidated external review

Re-audit final source ownership, dependency graph, persistence, fault evidence and
presentation boundary. M14 completion does not authorize Steel.

### Later optional Steel-in-Workbench sequence

Only after a new dependency-safety acceptance:

1. Steel placement/dependency re-review;
2. dedicated VM thread and bounded `lab/*` bindings;
3. validated `ui/*` / PresentationDocument bindings;
4. script fault/stop/VM-restart acceptance;
5. real experiment-procedure smoke.

## 19. Exact M14.1 non-goals and preserved invariants

M14.1 does not authorize or add:

- production GUI/client code or a workspace member;
- eframe, egui, egui_plot, Steel or any Cargo dependency;
- `PresentationDocument` structs or persistence implementation;
- a new Application operation, DTO, public error or session rule;
- server-side presentation, Runtime GUI concepts or a generic SDK;
- a third transport, remote deployment, TLS or authentication;
- Lua, dynamic plugins or managed-component changes;
- Runtime, Recorder, SQLite, OutputAuthority or physical-safety changes;
- M14.2 or any Steel implementation.

The recommended later design must preserve:

```text
Runtime owns experiment semantics.
Client owns presentation semantics.

ONE Runtime Application contract
ONE Runtime SessionStore
ONE public operation/error/dedup/subscription/history semantics

client lifetime != experiment lifetime
GUI lifetime != experiment lifetime
script lifetime != experiment lifetime

Workbench caches are observational and rebuildable
PresentationDocument is client-owned
network I/O never blocks the GUI thread
slow GUI or future script never blocks Runtime progress
socket send != mutation admission != terminal completion
no client fabricates physical evidence
no UI or scripting semantics enter Runtime core
```

## Final status

```text
M12: ACCEPTED

M13.1: ACCEPTED
M13 dependency safety resolution: ACCEPTED
M13.2: BLOCKED / NOT AUTHORIZED

M14.1 Workbench architecture audit: READY FOR EXTERNAL REVIEW
M14.2: NOT AUTHORIZED

STATUS: M14_1_READY_FOR_EXTERNAL_REVIEW
```
