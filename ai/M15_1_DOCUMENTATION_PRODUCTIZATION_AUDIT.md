# M15.1 Documentation and productization audit

Status: **READY FOR EXTERNAL REVIEW**

This is a documentation/architecture audit of the accepted repository at
`fa74052fae8c087a822e2c6847acc222fec15730`. It does not authorize or implement a
documentation rewrite, automation runtime, or later milestone. The audit follows this
authority order whenever sources disagree:

1. accepted production source;
2. accepted executable tests;
3. accepted current milestone evidence;
4. coordination documents;
5. historical reports.

## 1. Current product boundary

The product is two cooperating processes with deliberately different ownership:

```text
lab-runtime.exe                       lab-workbench.exe
----------------                       -----------------
authoritative experiment state   <->  private Rust Application client
devices, References, controllers      WorkbenchModel
resources and configuration           PresentationDocument
Recorder                               native egui presentation
sessions, deduplication                operator and recovery workflows
operation outcomes
```

The only semantic boundary between them is the language-neutral Application API.
Runtime owns experiment semantics; Workbench owns presentation semantics. Workbench
close, crash, disconnect, or replacement does not mean Runtime, controller, Recorder,
or experiment shutdown. A client cannot fabricate physical state, acknowledgements,
terminal mutation outcomes, or recovery evidence.

## 2. Documentation inventory

### Public/product documentation

| Path | Current purpose and audience | Current? | Conflict or gap | Recommended disposition |
|---|---|---:|---|---|
| `README.md` | Repository introduction, build/run sketch, safety summary; first-time users and developers | No | Says there is no GUI/Presentation API and no additional transport; omits Workbench and its launch path | Rewrite as the short product landing page |
| `docs/getting-started.md` | Runtime profile/configuration startup and a raw TCP exchange; advanced operators | Partly | Runtime details remain useful, but there is no Workbench happy path; calls TCP the sole current transport; ends the first example with Runtime shutdown | Rewrite around the two-process happy path; retain a separate API path |
| `docs/architecture.md` | Runtime ownership, scheduling, safety, and extension boundaries; developers | Partly | Correct for Runtime but says the repository has no presentation model; M14 added a Workbench-owned one | Rewrite/merge into current system architecture and ownership documents |
| `docs/application-api.md` | Detailed protocol, all operations, capabilities, errors, and bounds; API-client authors | Mostly | Its registry content matches source, but its transport introduction omits the accepted WebSocket adapter and it lacks Workbench recovery context | Retain authoritative content, split into a small coherent API section, and mechanically validate tables |
| `docs/recorder-sqlite.md` | Recorder lifecycle, durability, schema boundary, and query behavior; operators/developers | Yes | No material contradiction found; needs navigation and Workbench recording/recovery cross-links | Retain with targeted integration edits |
| `docs/safety-and-failures.md` | Runtime failure/safety contracts; operators/developers | Mostly | Strong Runtime material, but not a Workbench troubleshooting guide and does not present the accepted M14 lifecycle UX | Retain Runtime scope; link to a new Workbench recovery/troubleshooting guide |
| `docs/extending-runtime.md` | Trusted Rust extension rules; Runtime developers | Yes | No material contradiction found; future automation must not be mistaken for direct Runtime extension authority | Retain and add a neutral automation-boundary cross-link |

### Developer/architecture and coordination documentation

| Path | Current purpose | Audience | Current/conflict | Recommended disposition |
|---|---|---|---|---|
| `AGENTS.md` | Repository rules, accepted-state index, and engineering constraints | Agents/contributors | Current M14 acceptance snapshot; not product prose | Retain as coordination; never make it a user prerequisite |
| `ai/PROJECT_BRIEF.md` | Scope, architectural invariants, and accepted-state summary | Maintainers/reviewers | Current but milestone-oriented | Retain; promote stable product facts to `docs/` |
| `ai/HANDOFF.md` | Detailed state and continuation handoff | Maintainers/reviewers | Current but intentionally repetitive | Retain as engineering handoff |
| `ai/ROADMAP.md` | Accepted and blocked milestone sequence | Maintainers/reviewers | Current authorization record, not a product roadmap for users | Retain internally; do not mirror milestone chronology in public docs |
| `ai/WORK.md` | Only detailed current implementation authorization | Implementers/reviewers | Current; no later slice authorized before this audit request | Retain as coordination authority |
| `ai/RELEASE_PLAN_TO_V0_1.md` | Technical gate/release evidence and remaining conditions | Release engineers/reviewers | Current engineering record; not a user install guide | Retain; publish only relevant supported-version facts elsewhere |
| workspace/package `Cargo.toml` files | Build graph, features, binaries, versions, minimum Rust requirements | Developers/build tooling | Source truth; not a tutorial; Workbench requires Rust 1.95 | Use as the source for prerequisites; avoid vague “current stable” claims |
| Rust documentation/comments under `crates/**` and `apps/**` | Ownership and implementation contracts near code | Developers | Mixed; a few M14-era comments still name future Steel/UI-adapter plans | Correct later with neutral automation wording; add no abstractions |

### Historical milestone/review documentation

| Path | Current purpose | Audience | Current/conflict | Recommended disposition |
|---|---|---|---|---|
| `ai/M12_1_WEBSOCKET_ARCHITECTURE_AUDIT.md` | WebSocket boundary and threat/ownership audit | Architects/reviewers | Accepted design evidence | Promote stable transport facts; retain/archive evidence |
| `ai/M12_2_TRANSPORT_NEUTRAL_SERVER_SEAM.md` | Transport-neutral server ownership seam | Runtime developers/reviewers | Accepted implementation evidence | Retain/archive after architecture promotion |
| `ai/M12_3_BOUNDED_WEBSOCKET_TRANSPORT.md` | Bounded loopback WebSocket implementation | Runtime/API developers | Accepted implementation evidence | Promote endpoint/origin/subprotocol/bounds; retain report |
| `ai/M12_4_TRANSPORT_PARITY_FAULT_ACCEPTANCE.md` | TCP/WebSocket semantic parity and fault acceptance | Maintainers/reviewers | Accepted executable evidence | Use to validate public transport claims; retain report |
| `ai/M12_5_BROWSER_CLOJURESCRIPT_SMOKE.md` | Browser-client smoke acceptance | Maintainers/reviewers | Accepted test evidence; not a language selection | Retain; keep out of automation architecture claims |
| `ai/M12_CONSOLIDATED_EXTERNAL_REVIEW.md` | Consolidated M12 review result | Review/history readers | Historical acceptance record | Archive/retain as evidence |
| `ai/M13_1_STEEL_EXTERNAL_HOST_ARCHITECTURE_AUDIT.md` | Historical candidate-host audit | Architects/reviewers | Language-specific and not implementation authority | Retain as history; do not promote language-specific design |
| `ai/M13_STEEL_DEPENDENCY_SAFETY_RESOLUTION.md` | Dependency-safety conclusion and block | Maintainers/reviewers | Current reason M13.2 remains blocked | Retain; promote only the neutral deferral statement |
| `ai/M14_1_WORKBENCH_ARCHITECTURE_AUDIT.md` | Runtime/Workbench ownership architecture | Architects/reviewers | Accepted, but milestone-formatted | Promote system boundary/lifetime facts; retain report |
| `ai/M14_2_MINIMAL_WORKBENCH_CLIENT.md` | Bounded private Application client evidence | Client developers/reviewers | Accepted implementation evidence | Promote connection/limits facts; retain report |
| `ai/M14_3_WORKBENCH_MODEL_PRESENTATION.md` | WorkbenchModel and PresentationDocument evidence | GUI developers/reviewers | Accepted implementation evidence | Promote current presentation ownership and format; retain report |
| `ai/M14_4_MINIMAL_GUI.md` | Native egui GUI evidence | GUI developers/reviewers | Accepted implementation evidence | Promote actual launch/measurement/plot behavior; retain report |
| `ai/M14_5_OPERATOR_CONTROLS.md` | Typed operator/property/Recorder workflows | Operators/developers/reviewers | Accepted implementation evidence | Promote user workflows; retain report |
| `ai/M14_6_RECOVERY_FAULT_AUDIT.md` | Frozen recovery/fault policy and matrix | Architects/reviewers | Accepted policy evidence | Promote user-visible recovery concepts; retain report |
| `ai/M14_6B1_RECOVERY_STATUS_UI.md` | Check Status workflow | Operators/developers/reviewers | Accepted implementation evidence | Promote manual reconciliation behavior; retain report |
| `ai/M14_6B2A_RECOVERY_QUARANTINE.md` | restart classification/quarantine | Operators/developers/reviewers | Accepted implementation evidence | Promote quarantine and invalidation behavior; retain report |
| `ai/M14_6B2B1_EXACT_RETRY_CORE.md` | Exact Retry evidence lifecycle/core | Client developers/reviewers | Accepted implementation evidence | Promote immutable/manual retry semantics; retain report |
| `ai/M14_6B2B2_EXACT_RETRY_GUI.md` | Exact Retry confirmation UI | Operators/GUI reviewers | Accepted implementation evidence | Promote warning/confirmation UX; retain report |
| `ai/M14_6B3_BOUNDED_FAULT_REATTACH.md` | explicit Disconnect and bounded fault reattach | Operators/client developers/reviewers | Accepted implementation evidence | Promote lifecycle distinction and fixed bounds; retain report |
| `ai/M14_6B4_RECOVERY_FAULT_ACCEPTANCE.md` | 30-row and A-J consolidated executable evidence | Maintainers/reviewers | Accepted final M14 evidence | Use as acceptance index, not user prose; retain report |
| `ai/archive/design/0001-runtime-ownership-and-domain-boundary.md` | Original ownership decision | Architecture historians | Superseded/historical | Retain in archive; verify against source before citation |
| `ai/archive/design/0002-central-output-authority.md` | Original output-authority decision | Architecture historians | Historical but foundational | Retain in archive; public safety docs carry current truth |
| `ai/archive/design/RUNTIME_AND_SAFETY_MODEL.md` | Earlier consolidated Runtime/safety model | Architecture historians | Historical snapshot | Retain; do not use as sole current authority |
| `ai/archive/audits/MILESTONE_11_FAILURE_MATRIX.md` | Earlier failure matrix | Review/history readers | Historical | Retain in archive |
| `ai/archive/audits/MILESTONE_11_HARDENING_AUDIT.md` | Earlier hardening audit | Review/history readers | Historical | Retain in archive |
| `ai/archive/milestones/MILESTONE_8_REPORT.md` | M8 implementation evidence | Review/history readers | Historical | Retain in archive |
| `ai/archive/milestones/MILESTONE_9D_REPORT.md` | M9D implementation evidence | Review/history readers | Historical | Retain in archive |
| `ai/archive/milestones/MILESTONE_11_REPORT.md` | M11 implementation evidence | Review/history readers | Historical | Retain in archive |
| `ai/archive/milestones/POST_M9C_HARDWARE_SMOKE.md` | Historical physical smoke record | Review/history readers | Historical, environment-specific | Retain in archive; never present as current universal qualification |
| `ai/archive/README.md` | Archive policy/index | Maintainers/history readers | Current for the archive | Retain |

### Test-only executable documentation

| Path/group | What it proves | Audience | Disposition |
|---|---|---|---|
| `apps/lab-runtime/tests/README.md` | Test topology and how integration fixtures are organized | Contributors | Retain as test documentation, not user documentation |
| `clients/clojurescript-smoke/README.md` | Browser/WebSocket acceptance harness | Contributors | Retain and label clearly as a smoke client, not the selected automation language or supported SDK |
| `apps/lab-workbench/src/main.rs` ignored/process acceptances | Real-process launch, reconnect, crash/restart, recovery, and corrupt-journal behavior | Maintainers | Use to validate tutorial claims; do not turn test source into the guide |
| `apps/lab-workbench/src/client/worker.rs`, `gui/rebuild.rs`, `recovery/tests.rs` tests | Deterministic transport, recovery, freshness, pressure, and journal contracts | Maintainers | Keep executable; cite exact tests in documentation review checklists |
| Runtime integration tests under `apps/lab-runtime/tests/**` | Application API, Recorder, configuration, reconnect, safety, and transport contracts | Maintainers/API authors | Use as executable validators for examples and reference tables |

### Obsolete or duplicated material

No document should be deleted during this audit. The principal duplication is the same
protocol limits and ownership rules appearing in `README.md`, several `docs/` files,
coordination files, and milestone reports. The rewrite should establish one current
public home per fact and link to it. Historical reports may retain their snapshot.

Information currently available only, or most completely, in `ai/` and therefore in
need of promotion includes:

- optional loopback WebSocket behavior and security requirements from M12;
- the Workbench process/model/presentation ownership boundary and actual launch flow;
- the complete Fresh rebuild barrier and one-aggregate-subscription rule;
- Check Status, Exact Retry, exact journal evidence, quarantine, and journal failures;
- explicit Disconnect versus bounded fault reattach and overflow/event-gap distinctions;
- actual Workbench crash/restart, Runtime restart, and corrupt-journal behavior;
- the neutral future-automation boundary, without any M13 language candidate.

## 3. Source-of-truth map

| Domain | Primary production authority | Executable/accepted supporting authority |
|---|---|---|
| Runtime CLI/startup and readiness | `apps/lab-runtime/src/main.rs`, `service.rs`, `configuration.rs` | process-readiness and configuration integration tests |
| Application transport ownership | `apps/lab-runtime/src/server.rs`, `server/coordination.rs`, `server/websocket_peer.rs`, `websocket.rs` | transport parity/fault tests and M12 acceptance |
| envelope, strict JSON, NDJSON framing | `apps/lab-runtime/src/wire.rs` | protocol foundation and framing-boundary tests |
| hello/session/scope | `application.rs`, `sessions.rs` | session/reconnect tests |
| operation registry/capabilities/availability | `apps/lab-runtime/src/protocol.rs` | registry assertions and `hello` tests |
| queries and mutations | `application.rs` and its focused submodules | Application integration tests |
| request ID, deduplication, retained outcomes | `request_deduplication.rs`, `operation_outcomes.rs` | `request_deduplication.rs` integration tests |
| events/subscriptions/cursors | `events.rs`, `application/delivery.rs` | subscription, transport-parity, and event-gap tests |
| public errors and limits | `protocol.rs`, `wire.rs`, server/application constants | boundary and fault tests; `hello.result.limits` at runtime |
| Recorder/API/history | `recorder_api.rs`, `application/recording.rs`, `recorder/**` | `recorder_api.rs`, `recorder_history_api.rs`, Recorder acceptance tests |
| Reference | `application/references.rs`, `lab-core` Reference/runtime modules | Reference/control API and Workbench operator process tests |
| controller/output | `application/controllers.rs`, `lab-core` control/output modules | control and output-safety tests |
| resource/reconnect | `configuration_api.rs`, `service/reconnect.rs`, host configuration | configured-resource integration tests |
| configuration/properties | `configuration.rs`, `configuration_api.rs`, host/service configuration | configuration validation/lifecycle tests |
| Workbench connection lifecycle | `apps/lab-workbench/src/client/types.rs`, `client/worker.rs` | worker deterministic and process acceptance tests |
| Fresh/Stale/Rebuilding | `model/projections.rs`, `model/mod.rs`, `gui/rebuild.rs` | rebuild barrier, partial-loss, event-gap, and overflow tests |
| operator workflows | `model/operator.rs`, `gui/app.rs` | operator model/GUI and real Runtime tests |
| Check Status | `model/recovery_status.rs`, `client/worker.rs` | recovery-status and real Runtime reconciliation tests |
| Exact Retry | `model/exact_retry.rs`, `client/worker.rs`, `gui/app.rs` | exact-retry unit, GUI, and real Runtime tests |
| recovery journal/quarantine | `recovery/journal.rs`, worker/model recovery projection | recovery tests, B4 journal/process acceptance |
| Disconnect/fault reattach | `client/worker.rs` | B3 deterministic regressions and B4 process tests |
| presentation persistence | `presentation/**`, `model/command.rs`, GUI startup loading | presentation validation/model tests |
| workspace ownership | `ownership.rs`, `gui/mod.rs`, `main.rs` | native process crash/reacquisition acceptance |

When public wording disagrees with these sources, the rewrite must record the source
behavior and open an issue if intent is unclear. It must not blend an old report into a
new contract by inference.

## 4. Runtime CLI and startup audit

### Accepted invocation forms

The server CLI is strict and currently has two product forms:

```powershell
lab-runtime.exe --serve --config <absolute-or-relative-config.toml>
```

```powershell
lab-runtime.exe --serve --profile virtual-demo --port <0..65535> `
  [--record-db <absolute-local-path>] `
  [--record-policy required|best-effort] `
  [--ws-port <0..65535> --ws-origin <exact-origin> ...]
```

Important source-derived behavior:

- TCP and optional WebSocket listeners bind IPv4 loopback only. Port `0` asks the OS
  to select a port; the readiness JSON written to stdout is then authoritative.
- `--record-db` requires an absolute local path; UNC/network storage is rejected.
  Its policy defaults to `required` unless explicitly changed.
- WebSocket is optional. It requires at least one exact allowed origin, uses
  `/application/v1`, and requires subprotocol `lab-runtime.application.v1`.
- Configuration mode obtains listener, Recorder, resource, controller, and virtual
  composition from TOML. `examples/runtime.virtual.toml` is the safe virtual example.
- Readiness is a JSON line containing the boot identity, selected port, and
  `state:"ready"`, plus WebSocket details when enabled.
- Ctrl+C requests finite Runtime-owned shutdown. The `runtime_shutdown` Application
  mutation is a separate explicit operation; client exit is never shutdown authority.
- Running `lab-runtime` with no arguments executes a finite demonstration path and
  exits. It should be documented as developer/demo behavior, not server startup.
- There is no conventional `--help` or `--version` path today; invalid arguments
  produce the accepted-use forms as an error. This is a productization gap to document
  honestly rather than paper over.

Normal operational environment variables are `LAB_RUNTIME_LOG_LEVEL` and
`LAB_RUNTIME_LOG_DIRECTORY`. Test harness variables such as Runtime binary overrides,
GUI smoke result paths, kill-ready markers, and child-test selectors must not be
published as product features.

Workbench currently accepts:

```powershell
lab-workbench.exe --connect 127.0.0.1:<port> [--scope <scope>] [--workspace <path>]
```

The endpoint must parse as a numeric socket address. The GUI client currently uses
TCP, even if Runtime also exposes WebSocket. The default workspace is beneath the
platform local application-data directory and contains `presentation-v1.json` and
`recovery-v1.json`. Native workspace ownership and GUI operation are currently
Windows-specific; documentation must say this directly. Workbench likewise has no
conventional help/version command yet.

## 5. Application API surface audit

The operation and capability counts were derived directly from the static registries
in `apps/lab-runtime/src/protocol.rs`, not from an older report:

```text
operations:   42 total = 22 queries + 20 mutations
capabilities: 25
```

A mechanical name comparison found 42 unique source operations and 42 unique report
rows, with no missing or extra name; the source split is 22/20.

Feature availability below means advertisement is conditional on the active Runtime
composition. `recording_status` itself is always present and reports `unconfigured`
when no Recorder exists.

The source registry's exact availability rule is: every operation below is `Always`
unless its row says Recorder, Configuration, ResourceReconnect,
EmulatorPublication, or VirtualModelLifecycle. Those five labels correspond directly
to `ProtocolFeatures` and control whether hello advertises the operation. Capabilities
are structured discovery labels, not credentials; clients must use the advertised
operation list rather than infer permission from a similarly named capability.

| Operation | Kind | Arguments (strict top-level allowlist; semantic requirements can be conditional) | Important result | Availability/recovery relevance | Workbench GUI / advanced client |
|---|---|---|---|---|---|
| `hello` | Q | `scope` null or retained | boot/API identity, scope, next sequence, registry, cursors, limits | First request; establishes recovery classification | Internal Workbench / essential API |
| `discover` | Q | none | first frozen discovery page | Rebuild input | Internal Workbench / API |
| `discovery_page` | Q | `projection`, `index` | next frozen page | Rebuild input | Internal Workbench / API |
| `describe` | Q | `instrument` | descriptor and parameters | Observational | API-only |
| `resource` | Q | `resource` | authoritative resource state/generations | ResourceReconnect; rebuild input | Internal detail plus reconnect UI / API |
| `configuration_status` | Q | none | revision/source/staged/overlay state | Configuration | API-only |
| `configuration_properties` | Q | none | first frozen property page | Configuration; rebuild input | Properties UI/internal / API |
| `configuration_page` | Q | `projection`, `index` | next property page | Configuration | Internal Workbench / API |
| `latest` | Q | `signal` | current observation or `not_observed` | Observational | API-only; GUI uses bulk current state |
| `measurements_current` | Q | none | first frozen current page | Rebuild input | Internal Workbench / API |
| `measurements_page` | Q | `projection`, `index` | next current page | Rebuild input | Internal Workbench / API |
| `measurement_window` | Q | `signal`, `max_records` | bounded in-memory history | Observational | API-only; GUI maintains live plot data |
| `reference` | Q | `reference` | complete Reference projection | Authoritative refresh | Internal/detail UI / API |
| `controller` | Q | `controller` | lifecycle, policy, revision, last output | Authoritative refresh | Internal/controller UI / API |
| `component` | Q | `component` | component state/revision/diagnostics | Observational | API-only (discovery remains visible) |
| `output` | Q | `actuator` | read-only output authority/evidence | Observational safety evidence | API-only |
| `operation_status` | Q | `request_id` | accepted/completed/failed/outcome_unknown | Manual reconciliation only | **Check Status** / API |
| `subscribe` | Q | `after`, `filter` | aggregate token/cursor | Live projections; connection-local | Internal Workbench / API |
| `unsubscribe` | Q | `subscription` | removed flag | Connection-local | Internal Workbench / API |
| `recording_status` | Q | none | Recorder/archive/run/admission projection | Always available; rebuild input | Recording UI/internal / API |
| `history_page` | Q | `page_token` | retained durable page | Recorder; connection-local token | API-only |
| `history_release` | Q | `page_token` | released flag | Recorder | API-only |
| `reference_configure` | M | `reference`, `expected_revision`, `kind`, `value`, `target`, `rate` | committed Reference projection | Durable exact recovery before wire | Typed Reference UI / API |
| `reference_retune` | M | `reference`, expected revision, `target`, `rate` | committed ramp projection | Durable exact recovery before wire | Typed Reference UI / API |
| `controller_configure_pid` | M | controller/revision and PID | updated projection | Durable exact recovery | Typed PID UI / API |
| `controller_configure` | M | `controller`, `expected_revision`, `pid`, `ema`, `max_input_age_ns`, `max_tick_gap_ns`, `lease_lifetime_ns`, `proposal_ttl_ns` | full policy projection | Durable exact recovery | API-only |
| `controller_start` | M | `controller` | lifecycle projection | Durable exact recovery | Typed lifecycle UI / API |
| `controller_pause` | M | `controller` | safe-transition projection | Durable exact recovery | Typed lifecycle UI / API |
| `controller_resume` | M | `controller` | warming projection | Durable exact recovery | Typed lifecycle UI / API |
| `controller_reset_failed` | M | `controller` | paused projection | Durable exact recovery | Typed lifecycle UI / API |
| `runtime_shutdown` | M | none | finite cleanup/safety result | Explicit process authority; recoverable outcome | API-only, deliberately not a GUI close action |
| `stage_configuration` | M | none | candidate/base revision/expiry/effects | Configuration; durable recovery | API-only |
| `apply_configuration` | M | `candidate_id`, `expected_revision` | new revision | Configuration; durable recovery | API-only |
| `reload_configuration` | M | none | new revision | Configuration; durable recovery | API-only |
| `property_configure` | M | `target`, `property`, `value`, `expected_revision` | target/property/new revision | Configuration; durable recovery | Typed integer/text property UI / API |
| `emulator_publish` | M | `signal`, `state`, `value`, `expected_generation` | virtual signal/generation/state | EmulatorPublication; durable recovery | API-only |
| `virtual_models_restart` | M | none | model count/new generation | VirtualModelLifecycle; durable recovery | API-only |
| `reconnect_resource` | M | `resource`, `expected_binding_generation` | new binding generation | ResourceReconnect; never rearms control | Typed resource UI / API |
| `recording_start` | M | `label` | database/run/interval identity | Recorder; durable recovery | Recording UI / API |
| `recording_stop` | M | `run_id` | sealed/committed result | Recorder; durable recovery | Recording UI / API |
| `experiment_annotate` | M | `name`, bounded JSON `data` | record sequence, pending durability | Recorder; durable recovery | API-only |
| `history_read` | M | `mode`, `database_id`, `boot_id`, `run_id`, `signal`, `from_ns`, `to_ns`, `max_records`, `cursor` | terminal page token | Recorder; asynchronous retained operation | API-only |

The 25 capability names are:

| Capability | Representative surface | Capability | Representative surface |
|---|---|---|---|
| `operation_lifecycle` | `operation_status` | `structured_discovery` | `discover` |
| `live_subscriptions` | `subscribe` | `current_measurements` | `measurements_current` |
| `recent_measurement_history` | `measurement_window` | `instrument_queries` | `describe` |
| `reference_read_write` | Reference query/mutations | `controller_status` | `controller` |
| `controller_configuration` | configure operations | `controller_lifecycle` | lifecycle mutations |
| `managed_components` | `component` | `output_status` | `output` |
| `runtime_shutdown` | `runtime_shutdown` | `recording_status` | `recording_status` |
| `recording_control` | start/stop | `measurement_history` | history operations |
| `resource_status` | `resource` | `configuration_read` | configuration queries |
| `configuration_properties` | property pages | `configuration_write` | `property_configure` |
| `deployment_configuration` | stage/apply/reload | `resource_reconnect` | `reconnect_resource` |
| `virtual_instruments` | discovery of virtual devices | `emulator_publication` | `emulator_publish` |
| `virtual_model_lifecycle` | `virtual_models_restart` |  |  |

This table is the full Application surface, not a GUI backlog. Workbench intentionally
offers a safe typed operator subset rather than a raw operation console. Unsupported
or advanced operations should remain API-client capabilities unless a later product
decision defines a specific safe GUI workflow.

## 6. Current Workbench feature inventory

| Product area | Classification | Current behavior |
|---|---|---|
| TCP connection and explicit Disconnect | Complete user-facing feature | Startup/manual Connect, explicit Disconnect, state and boot/scope visibility |
| unexpected-fault reattach | Complete user-facing behavior | `Reattaching`/stale indication; one retained-scope episode with absolute 3 s deadline, per-attempt timeout at most min(2 s, remaining), spacing at least 10 ms |
| discovery/live measurements | Complete user-facing feature | Bounded discovery tree, selection detail, current measurements and updates |
| live plots | Complete user-facing feature | Selected-signal live trace with a bounded 4,096-point display history |
| Reference operations | Complete user-facing feature | Fixed/ramp configure and ramp retune with revision-aware confirmation |
| controller lifecycle | Complete user-facing feature | Start, pause, resume, and reset-failed typed workflows |
| PID configuration | Complete user-facing feature | PID-only typed edit; full controller timing/EMA policy remains API-only |
| resource reconnect | Complete user-facing feature | Revision/generation-aware reconnect action; no controller rearm |
| properties/configuration | Partial typed user-facing feature | Read and typed integer/text mutation for advertised properties; booleans are displayed read-only. Deployment stage/apply/reload is API-only |
| Recorder start/stop | Complete user-facing feature when configured | Authoritative status, run identity, typed start/stop workflow |
| Check Status | Complete user-facing feature | One explicit status request for exact retained identity; never automatic |
| Exact Retry | Complete user-facing feature | Warning and manual confirmation; exact immutable operation/args/request ID; never automatic |
| recovery/quarantine | Complete user-facing feature | Pending/Ambiguous/Accepted/Completed/Failed records, journal warnings, old boot/scope quarantine |
| Fresh/Stale/Rebuilding | Complete user-facing feature | Global visible state follows the full authoritative rebuild barrier |
| workspace ownership | Complete user-facing behavior | One native Workbench owns a workspace; OS releases ownership after process death |
| recovery persistence | Complete internal/user-visible support | Exact bounded journal loads at startup; corrupt/read/write failures are visible and fail closed for mutation authority |
| presentation persistence | Internal foundation, not a complete GUI workflow | Versioned `PresentationDocument` loading/validation exists, but current GUI has no user save/layout editor path; it may create an in-memory default plot |
| WebSocket Workbench transport | Not implemented | Runtime supports WebSocket; native Workbench currently connects over TCP only |
| raw operation console, durable history browser, annotations, Runtime shutdown UI | Not implemented in GUI | Supported through the advanced Application API where advertised |
| Discard/Forget recovery action | Not implemented | Quarantined evidence remains visible by design |
| scripting/automation runtime | Not implemented and no language selected | Only a neutral future boundary is proposed below |

`Fresh` has a precise meaning. It requires discovery and all pages, current
measurements and all pages, required Reference/controller/resource/Recorder/property
details, a second discovery fence, one aggregate subscription, catch-up, and completion
of detail refreshes. Hello, a connected socket, process liveness, an old cursor, or a
terminal mutation outcome is not sufficient.

## 7. Material documentation conflicts and gaps

1. **No-GUI claim is false.** `README.md` describes a Runtime-only repository, while
   the accepted `lab-workbench` is a native product process with presentation and
   operator/recovery workflows.
2. **TCP-only claim is false for Runtime.** README, getting-started, architecture, and
   the API introduction predate the optional loopback WebSocket adapter. Native
   Workbench itself remains TCP-only; the rewrite must preserve that distinction.
3. **“No presentation model in the repository” is too broad.** Runtime still has no
   presentation concepts, but Workbench owns `WorkbenchModel` and
   `PresentationDocument`.
4. **There is no public Workbench guide.** Launch, Fresh/Stale/Rebuilding, typed
   controls, plots, recording, recovery, workspace ownership, and crash behavior exist
   only in source/tests/M14 reports.
5. **Recovery is undocumented for users.** The exact state vocabulary, Check Status,
   Exact Retry, `outcome_unknown`, quarantine, and “operation outcome != current
   physical state” need a single current explanation.
6. **Failure/lifetime behavior is split across reports.** Explicit Disconnect,
   unexpected faults, Runtime restart, event gaps, update overflow, and corrupt journal
   behavior are accepted but not navigable as product guidance.
7. **CLI discoverability is weak.** Both executables have strict parsing but no normal
   help/version surface. Documentation must list exact invocations until that changes.
8. **Toolchain wording is vague.** The Workbench manifest's Rust 1.95 requirement is a
   better authority than README's generic “current stable Rust.”
9. **A few internal comments name Steel as a future adapter.** That is stale planning,
   not accepted architecture. A later documentation cleanup should make these comments
   language-neutral without adding new abstractions.

## 8. Proposed final documentation architecture

The existing strong Runtime documents should be preserved rather than fragmented into
dozens of tiny pages. A suitable target is:

```text
README.md

docs/
    getting-started.md
    architecture.md

    workbench/
        user-guide.md
        operator-and-recording.md
        recovery-and-troubleshooting.md

    api/
        README.md
        protocol-and-sessions.md
        operations.md
        events-mutations-and-recovery.md
        errors-and-limits.md
        examples.md

    tutorials/
        first-api-client.md
        reconnect-and-recovery.md

    automation/
        README.md
        architecture.md

    recorder-sqlite.md
    safety-and-failures.md
    extending-runtime.md
```

`docs/application-api.md` should become the API landing page or be merged into
`docs/api/README.md`; it must not remain a second independently maintained operation
registry. `architecture.md` should describe both processes at the system level, then
link to focused ownership/recovery material rather than repeat it.

## 9. Public README plan

The rewritten top-level README should remain short and answer, in order:

1. `lab-runtime` is the headless authoritative laboratory Runtime;
2. `lab-workbench` is the separate native operator/presentation client;
3. separation preserves experiment/controller/Recorder lifetime across GUI failure;
4. current capabilities: virtual/physical composition, References, control, Recorder,
   configuration, TCP/WebSocket API, native Workbench, and manual recovery workflows;
5. supported host/toolchain facts and one build command;
6. one Runtime command and one Workbench command;
7. what `Fresh` means and where to click first;
8. links to getting started, Workbench guide, API reference, safety, and Recorder docs;
9. Workbench crash does not stop Runtime and replacement can rebuild observations;
10. intentional limits: local-only listeners, Windows-native Workbench today, no
    scripting runtime/language, no Discard/Forget, and no blind mutation retry.

It must contain no M1-M15 chronology, review status, implementation commit hashes, or
claims that milestone completion itself is a product feature.

## 10. Getting-started happy path

Use only the accepted `virtual-demo` composition and an explicitly configured local
Recorder. The eventual PowerShell tutorial can use:

```powershell
cargo build --workspace --locked

$db = [IO.Path]::GetFullPath((Join-Path $PWD "demo.sqlite"))
cargo run -p lab-runtime --locked -- --serve --profile virtual-demo --port 7420 `
  --record-db $db --record-policy required
```

In a second terminal:

```powershell
$workspace = [IO.Path]::GetFullPath((Join-Path $PWD ".workbench-demo"))
cargo run -p lab-workbench --locked -- --connect 127.0.0.1:7420 `
  --workspace $workspace
```

Port 7420 must be free. A more robust advanced form can use Runtime port 0, read the
readiness JSON, and substitute the selected port. The tutorial flow should be:

```text
build -> start virtual Runtime -> start Workbench -> wait for Fresh
      -> select a signal and observe its plot
      -> select Reference 1 -> enter a safe ramp retune -> review/confirm
      -> observe the authoritative revision/target update
      -> optionally start and stop the configured Recorder
```

The tutorial must call virtual measurements virtual, require mutation confirmation,
and never imply that accepted operation evidence is physical readback. A companion
configuration path may use `examples/runtime.virtual.toml`, with an explicit note to
review its Recorder path. Physical examples belong in a later, clearly marked advanced
guide.

## 11. Application API tutorial and example policy

### First API client

The first API tutorial should treat the API as supported product surface and drive a
real Runtime:

1. connect to the loopback TCP listener;
2. send `hello` with null scope and retain boot/scope/next sequence;
3. read `discover` and every frozen page;
4. read `measurements_current` and every page;
5. create one aggregate `subscribe` from the snapshot cursor;
6. issue a virtual-demo Reference mutation with the next exact request ID;
7. distinguish operation `accepted` from terminal `completed`;
8. refresh/apply the authoritative Reference projection/event;
9. close the client without sending Runtime shutdown.

### Recovery tutorial

The second tutorial should introduce connection loss, retained-scope hello, an
Ambiguous exact recovery record, manual `operation_status`, manual Exact Retry,
`outcome_unknown`, boot/scope invalidation, and quarantine. It must state prominently:

```text
automatic mutation retry is not part of the protocol contract
```

An operation outcome answers what happened to an identified mutation. It is not a
substitute for current physical/projection state. Likewise, Fresh observations do not
reconcile mutation evidence.

### Maintained examples

The canonical examples should combine:

- short copyable raw NDJSON transcripts for exact framing and envelopes;
- one small Rust client example, preferably dependency-free beyond workspace crates,
  exercised against a real Runtime;
- PowerShell only for process launch and copy/paste orchestration;
- optional raw WebSocket JSON examples validated by the existing browser smoke.

A Python example is not needed to make the API real. If one is added later, it is an
API transport example, not selection of Python for embedded automation. The same rule
applies to the existing ClojureScript browser smoke. Example language and future
automation language are independent decisions.

Examples should live with the repository, carry the API/protocol version, and be
executed or mechanically parsed in documentation acceptance. Hand-written operation
lists and limit tables should be generated/checked from `protocol.rs` where practical.

## 12. Recovery and failure/lifetime documentation plan

The Workbench recovery guide needs a compact state glossary:

| Term | User-facing meaning |
|---|---|
| Pending | Exact mutation evidence was durably established before possible wire use; admission is not yet authoritative |
| Ambiguous | Some bytes may have been transmitted, but Runtime admission/outcome is unknown |
| Accepted | Runtime authoritatively admitted this exact identity/payload; execution may still be nonterminal |
| Completed | Runtime retained an authoritative terminal operation result |
| Failed | Runtime retained an authoritative terminal operation failure; this does not fabricate current physical state |
| `outcome_unknown` | Runtime cannot currently provide retained outcome evidence; do not downgrade to Completed or Failed |
| quarantine | Exact evidence belongs to an invalidated boot/scope and remains visible without retry authority |
| Check Status | Explicit one-shot `operation_status` for an eligible exact identity |
| Exact Retry | Explicitly confirmed resend of the immutable exact identity/payload, only where retained dedup authority permits |

It must also give an operator action table:

| Situation | What happens / remains authoritative | Automatic action | User action |
|---|---|---|---|
| Workbench clean close | Runtime, controller, Recorder, journal evidence continue | No Runtime lifecycle mutation | Restart Workbench when wanted |
| Workbench crash | Same; workspace mutex is released by the OS | No Runtime lifecycle mutation | Restart with same workspace |
| Runtime unavailable initially | Attempt terminates Disconnected; no scope invented | No retry loop | Start Runtime, click Connect |
| unexpected transport loss | Cached observations become stale; recovery evidence remains | One bounded retained-scope reattach episode | Wait; after expiry use Connect |
| explicit Disconnect | Socket/work for old epoch is cancelled; evidence remains | No reconnect | Use a later explicit Connect |
| Runtime restart / `instance_changed` | Old observations/drafts stale; old recovery quarantined | Old episode stops; no mutation send | Connect a new scope and rebuild |
| `scope_unknown` | Exact old evidence quarantined | Episode stops | Manual new-scope Connect |
| `scope_in_use` | Same retained identity may still be owned elsewhere | Retry only within the same immutable 3 s episode | Wait or reconnect manually after expiry |
| `event_gap` | Connection remains usable; contiguous projection history is invalid | Same-socket bounded resnapshot and new subscription | Wait for Fresh; do not infer missing state |
| ordered update overflow | Model fails closed/stale | One deferred resnapshot notification and one bounded fault episode | Wait for rebuild or reconnect manually |
| corrupt/read-failed journal | Bytes/evidence are not replaced with an empty default; mutation authority is blocked | Observational connection/rebuild may continue | Preserve file and investigate warning |

Operational docs should expose stable behavior and remedies, not worker implementation
internals. The absolute 3 s reattach window, 2 s attempt cap, and 10 ms minimum spacing
are public limits worth naming once in the API/limits reference and linking from the
guide.

## 13. Neutral automation placeholder

Create exactly two small documents in a later authorized slice.

### `docs/automation/README.md`

State plainly:

```text
No scripting language is currently selected.
No scripting runtime is part of v0.1.
Future automation must preserve Application API semantics.
A future implementation may be embedded in Workbench, a separate client process,
or both. The language choice is intentionally deferred.
```

Show both supported architectural possibilities:

```text
future embedded automation
  -> neutral automation host/facade
  -> existing Workbench Application client
  -> Application API
  -> Runtime

future external automation client
  -> Application API
  -> Runtime
```

### `docs/automation/architecture.md`

Freeze these constraints without creating code:

- embedded automation semantics == external automation semantics == Application API
  semantics;
- a separate neutral presentation surface may expose Workbench-owned presentation
  operations;
- automation gets neither Runtime internals nor direct `egui::Ui` access;
- automation lifetime does not own experiment lifetime;
- no direct transport, output, Recorder, deduplication, or recovery authority;
- language and embedding technology remain deferred.

Future shared/domain names must stay neutral. Avoid `SteelRuntime`, `LuaEngine`,
`LuaCommand`, and `SteelValue`. Candidate vocabulary, if implementation is later
authorized, includes `AutomationHost`, `AutomationCommand`, `AutomationValue`,
`AutomationError`, and `PresentationCommand`. This audit does **not** add or authorize
those Rust types, and it specifically rejects a speculative generic `ScriptEngine`
trait.

## 14. Historical `ai/` policy

Adopt a clear two-tree rule:

```text
docs/ = current product truth for users, operators, and client authors
ai/   = engineering coordination, review evidence, decisions, and history
```

Product usage must never require reading `ai/`. Accepted reports remain valuable as
traceability evidence and should not be erased. After their stable facts are promoted,
closed reports may be indexed or moved under an archive/reviews grouping in a separate
authorized cleanup; links and history must be preserved. Active coordination files
stay at the current `ai/` level while they are active.

Public documents should not reproduce review verdicts, commit hashes, milestone IDs,
or “ready for review” vocabulary. Conversely, historical reports need not be rewritten
each time current product wording improves.

## 15. Documentation consistency policy

1. Public docs describe the current released/accepted behavior, not milestone history.
2. Operation/capability names and argument allowlists come from `protocol.rs`.
3. Wire/framing limits come from `wire.rs`; runtime-advertised values in
   `hello.result.limits` are authoritative for a running build.
4. Maintain one canonical `docs/api/errors-and-limits.md`; other pages link to named
   entries instead of copying the full table.
5. Process ownership, Runtime authority, Workbench presentation authority, and
   operation outcome versus physical state are never conflated.
6. Examples are executable against a real Runtime or mechanically validated for exact
   DTO/framing semantics where practical.
7. GUI docs distinguish the typed safe subset from the full API without calling either
   incomplete.
8. Test-only environment variables, self-spawn modes, and fault seams are not product
   features.
9. Neutral architecture docs contain no future scripting-language name.
10. Every page states its applicable product/API version and has an owner/check path.

For version-sensitive limits, keep definitions in production source, advertise the
applicable Runtime limits through hello, generate/check the canonical reference table,
and link to it from tutorials. Human explanations may summarize behavior but should
not clone every number.

## 16. Tutorial/test relationship

Tests remain executable oracles, not prose documentation. Suitable validators include:

| Tutorial/claim | Existing executable reference |
|---|---|
| first connection and Fresh | `b4_scenario_a_runtime_absent_then_manual_connect_reaches_fresh`; process readiness/protocol tests |
| explicit disconnect/manual reconnect | `b4_scenario_b_explicit_disconnect_requires_manual_fresh_reconnect` and worker disconnect-fence regressions |
| Reference mutation/authoritative refresh | `real_runtime_operator_layer_confirms_completes_refreshes_and_never_retries_conflict` |
| Recorder lifecycle/client independence | Runtime `recorder_api` tests and B4 native process continuity acceptance |
| Check Status | `real_runtime_reference_reconnect_reconcile_and_replay` plus recovery-status unit tests |
| Exact Retry retained outcome | `real_runtime_exact_retry_retained_outcome_never_reexecutes` |
| Exact Retry evicted outcome | `real_runtime_exact_retry_outcome_unknown_never_reexecutes` |
| Runtime restart/quarantine | `b4_scenario_d_runtime_restart_quarantines_old_boot_and_builds_new_epoch` |
| process restart/exact journal | `b4_scenario_e_process_crash_preserves_exact_journal_without_auto_send` |
| corrupt journal | `b4_scenario_j_corrupt_journal_allows_fresh_observation_but_blocks_mutation` and real journal parser tests |
| event gap | `event_gap_rebuilds_same_connection_to_fresh_with_one_new_aggregate_subscription` |
| update overflow | `ordered_update_overflow_runs_one_reattach_and_full_freshness_barrier` |

A later docs-acceptance harness should extract or replay tutorial requests rather than
merely assert that files contain expected strings.

## 17. Recommended implementation slices (proposals only)

These are bounded proposals, not authorization:

| Proposed slice | Deliverable |
|---|---|
| M15.2 README + getting started | Replace false landing-page claims; add verified virtual-demo Runtime/Workbench happy path and navigation |
| M15.3 architecture + Application API reference | Rewrite the two-process/authority overview; establish one source-checked 42-operation/25-capability reference including both transports |
| M15.4 Workbench user guide | Current GUI, plots, typed controls, Recorder, Fresh barrier, workspace/presentation limitations |
| M15.5 API examples/tutorials | Validated NDJSON/Rust examples, first client, mutation lifecycle, WebSocket note |
| M15.6 recovery/troubleshooting | Recovery glossary, manual workflows, lifecycle/failure action table, journal and restart behavior |
| M15.7 neutral automation placeholder | Add only the two neutral automation documents described above; no code or dependency |
| M15.8 documentation acceptance | Link checking, source-derived registry/limit validation, executable examples, stale-term scan, and final product-doc review |

The order intentionally establishes the user path and canonical references before
adding automation placeholders. No M15.2+ slice is authorized by this audit.

## 18. Audit conclusion

The implementation is already a coherent product boundary, but the public document
set still presents the pre-Workbench Runtime-only product. The highest-value rewrite is
not more feature prose; it is one accurate entry path, one authoritative API reference,
one Workbench/operator guide, and one recovery/lifetime guide. Existing Runtime safety,
Recorder, and extension documents should be integrated rather than discarded.

The future automation reservation can be documented cleanly without selecting a
language or introducing any implementation abstraction. M13.2 Steel remains blocked
and not authorized.

```text
M14 consolidated: ACCEPTED

M15.1 documentation/productization audit:
READY FOR EXTERNAL REVIEW

M15.2+:
NOT AUTHORIZED

M13.2 Steel:
BLOCKED / NOT AUTHORIZED

STATUS: M15_1_DOCUMENTATION_PRODUCTIZATION_AUDIT_READY_FOR_EXTERNAL_REVIEW
```
