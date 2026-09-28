# M13.1 external Steel host architecture/dependency audit

## Status and conclusion

```text
M12: ACCEPTED
M13.1 external Steel host architecture/dependency audit: READY FOR EXTERNAL REVIEW
M13.2: NOT AUTHORIZED
M14: NOT AUTHORIZED
```

This is a read-only architecture and dependency audit against `main` at
`30800902f5a4ffc74d62ef0b81fab4b5174bb3ef`. It adds no dependency, crate,
executable, Rust source, test, operation, DTO, transport or Runtime behavior.

The source boundary supports an external Steel host without changing the
Application contract:

```text
Steel procedure
      |
      v
lab-steel external process
  Steel VM owner + bounded client bridge + TCP/NDJSON I/O owner
      |
      | existing public Application protocol
      v
lab-runtime TCP listener
      |
      v
ONE delivery boundary / ONE Application / ONE SessionStore / Runtime
```

TCP/NDJSON is the recommended transport. A one-host-process-per-script invocation
is the recommended first lifecycle. No Application gap or architectural
contradiction was found.

There is, however, a dependency gate for M13.2: the current exact candidate
`steel-core 0.8.3` has an unfixed transitive RustSec unsoundness advisory through
`im-rc -> sized-chunks`, plus four unmaintained-package notices in a fresh minimal
resolution. M13.2 must not add this graph until external review chooses an acceptable
resolution, such as a newer upstream release with the chain removed/fixed. Process
isolation protects authoritative Runtime state from a Steel-host failure; it does
not make unsound host code acceptable or make the host itself safe.

## 1. Authority and source boundary

Current authoritative coordination says Steel is an external Application client.
The production source confirms the suitable boundary:

- `apps/lab-runtime/src/server.rs` owns the one network reactor and global client
  admission.
- `apps/lab-runtime/src/server/coordination.rs` owns the shared transport-neutral
  delivery and exact detach fence.
- `apps/lab-runtime/src/wire.rs` defines the server's shared bounded JSON request
  decoder and NDJSON framing.
- `apps/lab-runtime/src/application.rs` owns one serialized Application facade.
- `apps/lab-runtime/src/sessions.rs` owns one process-local `SessionStore`, request
  sequence and retained deduplication state.
- `apps/lab-runtime/src/protocol.rs` owns the one 42-operation registry,
  25-capability projection and public error taxonomy.
- `apps/lab-runtime/src/service.rs`, `host.rs` and `lab-core::Runtime` remain the
  authoritative service/domain owners.

No production Application client library exists. Native integration tests contain
small test-only TCP clients, and `clients/clojurescript-smoke` is deliberately an
external browser acceptance client. The workspace currently contains only
`crates/lab-core` and `apps/lab-runtime`.

The future Steel executable must not depend on the `lab-runtime` crate merely to
reuse its public wire helpers: doing so would pull server, Runtime, Recorder and
physical-adapter implementation into a client. It must consume the documented
public protocol and discover the active operations/capabilities through `hello`.
It must not copy the server registry or public error mapping.

Steel must not execute inside `lab-core`, `HostCore`, the Runtime owner loop,
managed-component execution, controller callbacks, either transport adapter or a
transport worker. No Steel object or callback may cross into those owners.

```text
Steel process lifetime != Runtime lifetime
Steel script lifetime != experiment lifetime
client lifetime != experiment lifetime
```

Normal script exit, panic, infinite loop, VM defect or process kill closes one
ordinary client connection. It does not stop or roll back authoritative experiment
state. Existing explicit Runtime operations may still change lifecycle when the
script successfully requests them; host death itself does not.

## 2. Historical embedded-host contradiction

Older project discussions considered in-process scripting/VM shapes, and archived
documents retain evidence of the removed in-process Lua era. They are historical,
not current authority. The active roadmap, accepted M12 boundary and current source
supersede any in-process Steel possibility.

Embedding Steel into Runtime would make VM panic/unsoundness, runaway execution and
script lifetime part of the authoritative process; would invite direct Rust
callbacks into Runtime-owned state; and would blur the accepted distinction between
external supervisory clients and bounded native managed components. It therefore
contradicts the current invariant even if an older proposal described it.

The current plan wins because it preserves one language-neutral Application API,
uses the already-proven client/session/reconnect model, and gives ordinary OS process
failure separation without changing Runtime scheduling or ownership.

## 3. Transport choice

Use the existing IPv4-loopback TCP/NDJSON endpoint for the native Steel host.

Reasons:

- it carries exactly the same Application JSON objects as WebSocket;
- it requires only `std::net::TcpStream`, bounded line framing and `serde_json`;
- it avoids HTTP Upgrade, Origin allowlisting, WebSocket control/fragment state and
  a Tungstenite client dependency;
- browser Origin policy is irrelevant to a native process;
- M12 already proves TCP/WS semantic parity and cross-transport scope migration;
- the host remains an ordinary member of the single `TCP + WS <= 8` admission pool.

The host connects to a configured/readiness-derived numeric loopback TCP port. It
does not receive a direct `Application`, `HostCore`, Runtime channel or privileged
same-language shortcut. No third transport is needed.

WebSocket remains available for browser clients and can be used for later parity
tests, but is not the M13 baseline.

## 4. Proposed executable and client placement

The smallest repository-conventional future placement is:

```text
apps/lab-steel/
    Cargo.toml
    src/
        main.rs
        client.rs
        bridge.rs
        values.rs
```

M13.1 does not create it. M13.2, if authorized after dependency resolution, may add
one workspace executable package named `lab-steel`.

The first host should contain a deliberately narrow internal TCP Application client,
not add a reusable workspace SDK. It needs only:

- bounded connect/read/write/deadline behavior;
- NDJSON framing with the accepted 16,384-byte frame limit;
- generic JSON envelopes and correlation;
- hello/scope state, pending `msg_id` state and bounded event routing.

It must not contain a second operation registry, semantic DTO hierarchy, public
error taxonomy or domain validation implementation. Server replies and `hello`
remain authoritative. If a second real native client later needs the same Rust
client, extraction into a small independent client crate can be reviewed then.

The future host must not depend on `lab-core` or `lab-runtime`. Its laboratory
authority is the TCP Application connection only.

## 5. Steel dependency audit

### Authoritative upstream facts

Audit date: 2026-09-28.

| Item | Finding |
| --- | --- |
| Project | Steel, `mattwparas/steel`; embeddable Scheme interpreter with a standalone CLI/REPL. |
| Candidate embedding package | `steel-core` (the Rust library crate name is `steel`). |
| Exact current version | `0.8.3`, published 2026-08-20, not yanked. |
| Official CLI package | `steel-interpreter 0.8.3`; not recommended for `lab-steel`. |
| License | MIT OR Apache-2.0. |
| Rust edition | 2021. |
| Declared MSRV | None. Cargo/crates.io report `rust-version: unknown`; no MSRV claim is possible. |
| Default features | `std`, `modules`; upstream comments that `modules` no longer does anything. |
| Optional features | `anyhow`, `biased`, `custom-hash`, `disable-arity-checking`, `dylib-build`, `dylibs`, `dynamic`, `experimental`, `experimental-drop-handler`, `ffi-format`, `git`, `imbl`, `inline-captures`, `interrupt`, `jit`, `jit2`, `markdown`, `op-code-profiling`, `profiling`, `recycle`, `rooted-instructions`, `sandbox`, `smallvec`, `stacker`, `sync`, `triomphe`, `unsafe-internals`, `unsandboxed-kernel`, `ureq`, `without-drop-protection`. |
| Windows evidence | Upstream v0.8.3 CI includes `windows-latest` / `x86_64-pc-windows-msvc`; a fresh local Windows `cargo check` succeeded with Rust 1.95.0. |
| Build script/system dependencies | Published `steel-core` has no `build.rs`. The default/std-only graph built locally without a required external native library or C toolchain. Optional JIT/dylib/git paths are not needed. |
| Maintenance signal | 0.8.0/0.8.1 in 2026-02, 0.8.2 on 2026-02-22, 0.8.3 on 2026-08-20; upstream `master` was active at commit `9774aa06...` dated 2026-09-27. The project warns its pre-1.0 API may change. |

Primary sources:

- <https://crates.io/crates/steel-core/0.8.3>
- <https://docs.rs/steel-core/0.8.3/steel/>
- <https://github.com/mattwparas/steel/tree/v0.8.3>
- <https://github.com/mattwparas/steel/blob/v0.8.3/crates/steel-core/Cargo.toml>
- <https://github.com/mattwparas/steel/blob/v0.8.3/.github/workflows/rust.yml>
- <https://github.com/mattwparas/steel/blob/v0.8.3/Cargo.toml>

### Which package is needed

`steel-core` exposes `steel::steel_vm::engine::Engine`, function/module registration,
plain Steel values and the standard VM machinery needed by a custom host. That is
enough for `lab-steel`.

The official `steel-interpreter` is an application package. At v0.8.3 it adds the
REPL/CLI, `steel-repl`, documentation, package/dylib installer integration, Clap,
allocator selection and a workspace `steel-core` feature set including dylibs,
markdown, stacker, sync, rooted instructions, `imbl`, `jit2` and biased storage.
Those are not needed for a narrow external Application host and materially increase
capability and dependency surface.

The eventual dependency shape, if its advisory gate is resolved, should therefore
be an exact `steel-core` pin with `default-features = false` and only `std` enabled.
A local probe established that no features fails to compile 0.8.3, while `std` alone
compiles; `modules` is a documented no-op. Do not enable dylibs, FFI, JIT, git, HTTP,
`ureq`, markdown, stacker, sync, unsafe-internals or experimental features.

This is a recommendation for a later reviewed Cargo change, not a dependency added
by M13.1.

### Dependency footprint

`cargo info steel-core@0.8.3 --verbose` reports 43 immediate normal dependencies for
the default/std path. A fresh isolated Windows probe resolved 157 dependency package
identities excluding the probe executable in Cargo metadata; 126 identities were
not already present in the current lab-runtime workspace metadata. This is a large
addition, even though many are portable Rust utility/collection/parser crates.

The graph includes Steel's `steel-derive`, `steel-gen`, `steel-parser` and
`steel-quickscope`, plus persistent collections, ICU case mapping, futures utilities,
crossbeam, parking_lot, numeric types, serde/JSON and platform support. There is no
justification for adding the still larger `steel-interpreter` graph.

### Unsafe-code implications

The published `steel-core 0.8.3` source contains explicit unsafe blocks and unsafe
`Send`/`Sync` implementations in its value, GC, VM, polling and registration
machinery even without enabling its optional `unsafe-internals` feature. The
workspace's `unsafe_code = "forbid"` lint applies to local crates, not third-party
dependencies. “Do not enable `unsafe-internals`” is therefore useful but does not
make Steel an unsafe-free dependency.

Keeping the VM in `lab-steel.exe` means a VM memory-safety failure cannot directly
corrupt the `lab-runtime.exe` address space. It can still corrupt, hang or terminate
the host and must be treated as a dependency risk.

### Advisory result and gate

A fresh minimal 0.8.3/std-only resolution was queried against OSV on 2026-09-28.
There was no direct `steel-core` advisory, but the resolved graph had these distinct
RustSec findings:

| Package | Finding | M13 relevance |
| --- | --- | --- |
| `bincode 1.3.3` | RUSTSEC-2025-0141, unmaintained | Informational maintenance debt, direct Steel dependency. |
| `bitmaps 2.1.0` | RUSTSEC-2026-0247, unmaintained | Transitive via `im-rc`/`sized-chunks`. |
| `im-rc 15.1.0` | RUSTSEC-2026-0250, unmaintained | Direct Steel dependency; upstream archive. |
| `sized-chunks 0.6.5` | RUSTSEC-2026-0251, unmaintained | Transitive through `im-rc`. |
| `sized-chunks 0.6.5` | RUSTSEC-2026-0255, panic-safety use-after-free/double-free reachable from safe Rust; no fixed version | Material blocker for adding the current graph. |

Current semver resolution selected fixed `rand 0.9.5` and `thin-vec 0.2.20`, so the
older lockfile copies of the 2026 rand and thin-vec findings are not present in a
fresh host resolution.

Advisory sources:

- <https://rustsec.org/advisories/RUSTSEC-2025-0141.html>
- <https://rustsec.org/advisories/RUSTSEC-2026-0247.html>
- <https://rustsec.org/advisories/RUSTSEC-2026-0250.html>
- <https://rustsec.org/advisories/RUSTSEC-2026-0251.html>
- <https://rustsec.org/advisories/RUSTSEC-2026-0255.html>
- <https://osv.dev/>

Recommendation: do not add `steel-core 0.8.3` in M13.2 as currently resolved.
Before M13.2 authorization, repeat the exact resolution/advisory audit and require
one of:

1. a newer official Steel release that removes or fixes the affected chain;
2. an upstream-accepted dependency migration to maintained/fixed collections; or
3. a separately reviewed, pinned and maintained patch/fork strategy with license,
   provenance and update ownership.

Silently ignoring the advisory or relying only on process separation is not
recommended.

## 6. Host trust and capability boundary

The Steel host is an ordinary supervisory client, not a trusted physical adapter.
All laboratory state changes follow:

```text
Steel plain data
    -> bounded host request
    -> public Application operation
    -> existing admission/dedup/session rules
    -> ServiceHost / HostCore / Runtime
```

Steel receives no `&mut Runtime`, `HostCore`, `Application`, `SessionStore`,
`OutputAuthority`, serial/resource handles, transport executors, Recorder/SQLite
handles, managed-component executor or unsafe Rust callback. It cannot publish
physical observations, fabricate transport completion/ACK/readback/safe evidence,
bypass output authority or call raw register writes. It gets exactly the operations
advertised by `hello` for that Runtime composition.

The initial scripts should be documented as trusted local operator code, not hostile
multi-tenant code. `Engine::new_sandboxed()` blocks direct filesystem ports,
TCP/HTTP/polling and dylib loading, but v0.8.3 source still registers process and git
modules (the latter can shell out to `git` when its optional libgit feature is off).
It is therefore defense-in-depth, not a security sandbox. M13 must not claim hard
filesystem/process/network confinement without a separately reviewed OS sandbox or
a verified reduced-engine construction.

Any filesystem authority a trusted Steel process has comes from its OS account and
Steel library, not through Runtime. Runtime exposes no filesystem convenience
operation for Steel.

## 7. Steel-facing API and plain-data mapping

The first binding should expose one small module, with provisional concepts:

```text
connect
hello
request
query
operation
await-operation / operation-status
subscribe
next-event
unsubscribe
close
```

Names are not frozen by M13.1. These are host functions registered in one Steel VM,
not new Application operations.

Mapping should be mechanical and bounded:

| JSON | Steel |
| --- | --- |
| object | immutable hash map with string keys |
| array | vector |
| string | string |
| boolean | boolean |
| finite JSON number | Steel integer/real where exactly representable by the encoder |
| null | a dedicated `json-null` singleton, distinct from false, void and missing |

The reverse conversion rejects non-string object keys, non-finite numbers,
unsupported/custom Steel objects, cycles and values whose JSON encoding exceeds the
16,383-byte Application body bound. The host must not stringify arbitrary Steel
values as a fallback.

`request` should expose the exact decoded Application envelope. `query` may provide
a thin checked convenience, but it must preserve `type:"result"` versus
`type:"error"`. `operation` waits for and returns the authoritative accepted state;
terminal `completed` or `failed` remains distinct and is observed from the same
exchange, `await-operation`, events or `operation_status`. Network send success is
never operation acceptance or completion.

Public errors remain plain data with the existing code/category/message/retryable/
resync fields. A Steel exception caused by bad local use is a host/script failure,
not a new Application public error.

## 8. Message and request identity

Ownership must remain explicit:

- The client I/O owner allocates checked connection-local `msg_id` values and keeps
  them unique while in flight. A new connection gets a fresh correlation space.
- Runtime allocates a scope on `hello {scope:null}` or authoritatively reattaches a
  supplied retained scope.
- Runtime's `hello.next_seq` is the source of the next mutation sequence.
- The binding may form `{scope,seq}` ergonomically, but advances its local next
  sequence only after authoritative admission. It serializes sequence allocation.
- If admission/reply is ambiguous after disconnect, the host reattaches the same
  scope, uses `operation_status`, and retries only the exact same request ID and
  normalized payload when appropriate. It must not skip to a new sequence to hide
  uncertainty.
- Callers may supply an exact prior `request_id` for reconciliation/retry.

```text
msg_id = connection-local correlation
request_id = process-local retained mutation/deduplication identity
```

The host offers no Steel-specific exactly-once claim. Retention, eviction,
`outcome_unknown`, `scope_in_use` and old-boot invalidation stay unchanged.

A one-shot restart needs the caller/operator to provide the retained scope and the
exact request/payload being reconciled. The host may print a bounded machine-readable
summary, but must not pretend its local file or memory is authoritative experiment
state.

## 9. Subscription and backpressure ownership

Recommended initial host ownership:

```text
VM/main thread
    owns Steel Engine and executes script
    uses bounded command/result bridge

one host I/O thread
    owns TcpStream, NDJSON framing, msg_id table and socket deadlines
    routes replies and events into bounded queues

lab-runtime
    remains independent and owns Application/session/event semantics
```

No I/O-thread callback may enter the Steel VM. The VM pulls events via `next-event`,
which keeps Steel single-owner and makes backpressure observable.

Proposed M13.2 host-local bounds:

| Resource | Bound | Overflow/failure behavior |
| --- | ---: | --- |
| TCP NDJSON frame | 16,384 bytes including LF | Close connection and report host transport failure. |
| Application JSON body | 16,383 bytes | Reject before send; never split one request. |
| Decode scratch | one frame | Reused; no accumulating input buffer. |
| VM -> I/O command bridge | 8 | Steel call gets a host-local busy/failure result; do not block forever. |
| I/O -> VM result bridge | 8 | Close connection; caller reconciles retained operations after reconnect. |
| Pending in-flight `msg_id` exchanges | 8 | Reject locally before send. |
| VM-visible event queue | 16 | Close/detach, report overflow with last delivered cursor, and require fresh subscription/replay. |
| Active subscription | 1 | Existing Application contract. |
| Script output queue | 64 records, 4 KiB each | Truncate one oversized record; drop newest on full with a bounded dropped counter. Never feed Runtime diagnostics. |
| Socket/connect/request wait | finite monotonic deadline | Return/raise host-local transport timeout; mutation ambiguity reconciles through existing semantics. |

There is no generic unbounded incoming-message queue: at most one decode scratch is
routed into the eight bounded result/pending slots or sixteen-event queue. Exact
deadline values should be frozen with deterministic tests in M13.2; they must not
exceed or weaken server pressure isolation.

On event overflow, the host does not invent a new Application `event_gap`. It
remembers the last event cursor actually delivered to the VM, closes the connection,
and on reattach asks the existing `subscribe` operation to replay. The Application
alone decides whether replay succeeds or returns authoritative `event_gap` with
oldest/latest cursors and `resync_required`.

## 10. Execution and process lifetime

Steel can loop forever, allocate heavily, throw or panic. Because the VM is external,
none of those executes on Runtime's serialized owner or native control/acquisition
threads.

Minimum M13 lifecycle:

- one OS process owns one Steel VM and evaluates one script invocation;
- ordinary completion closes the Application connection and exits;
- script exception produces bounded diagnostics, closes and exits nonzero;
- Ctrl-C/operator/harness may terminate the host process without stopping Runtime;
- restart is a new process and explicit Application reattach, not VM state recovery;
- every socket and bridge wait is finite;
- process output is bounded as above.

Steel 0.8.3 exposes `InterruptHandler`, and its upstream tests demonstrate
cooperative interruption of an infinite recursive program. M13.2 may use it for an
optional evaluation deadline. It is not a safety boundary: blocking native calls,
OS primitives or VM defects may evade cooperative progress. External process kill
is the final lifecycle mechanism.

No hard memory limit, CPU quota, Windows Job Object or hostile-code sandbox is
proposed for v0.1. Process address-space separation prevents ordinary direct memory
corruption of Runtime, but it does not promise host availability or prevent the host
from competing for machine resources. Operators must be able to kill/restart it;
strong resource governance is a later deployment concern.

## 11. One-shot versus persistent host

| Concern | A. one process per script invocation | B. persistent host/reload |
| --- | --- | --- |
| Authoritative state | Always Runtime-owned. | Always Runtime-owned. |
| VM/script state | Discarded on exit; simple. | Persists or requires exact reset semantics. |
| Reconnect/dedup | Explicit retained scope/request data; clear failure boundary. | Can retain helpers locally but must still obey Runtime. |
| Subscriptions | One connection lifetime; rebuilt after restart. | Long-lived but requires reload/old-generation cleanup. |
| Failure recovery | Kill process and start fresh. | Must quarantine/reset VM, queues, callbacks and subscriptions. |
| Resource bounds | OS process is a natural unit; finite client queues. | Needs long-lived heap/output/module/reload bounds. |
| Developer experience | Simple `lab-steel script.scm`; startup per run. | Faster iteration and REPL potential. |
| Complexity | Lowest. | Adds supervisor, reload generations and stale-state hazards. |

Recommend A for M13. The Steel issue tracker currently even contains an open question
about clearing an Engine global environment, reinforcing that persistent reset is
not a free or established contract. Long-running procedures may remain in one
one-shot process; “one-shot” means one invocation/lifecycle, not a short duration.

## 12. REPL decision

An interactive REPL is not required for the first M13 vertical slice and is deferred.
The official `steel-interpreter` REPL is precisely the broader dependency/capability
surface this host does not need.

A future REPL may be a thin mode of `lab-steel` using the same one bounded client and
the same registered functions. It must not create a second Application API, a second
session owner, hidden mutation IDs or an immortal VM supervisor. It requires a
separate lifecycle/output/resource review.

## 13. Managed-component separation

```text
Native managed component:
Runtime invokes a bounded compile-time Rust implementation through ComponentExecutor
as part of Runtime-managed execution.

M13 Steel:
an external supervisory process sends ordinary Application requests.
```

Steel does not implement `ComponentExecutor`, run on the managed worker pool, supply
controller callbacks or replace native controllers. Native real-time components and
controller/safety logic remain native Rust. M13 does not reopen M5 Lua, activate
historical `lua.v1` provenance, migrate components or add a dynamic plugin system.

## 14. Presentation boundary

M13 exposes laboratory Application data and orchestration only. It introduces no
Window, Tab, Plot, Panel, Button, Slider, layout, workspace document,
`PresentationDocument`, GUI state or browser ownership. If a future script produces
client-owned presentation data, that is an M14 concern and cannot become a Runtime
operation or M13 host authority.

```text
Runtime owns experiment semantics.
Client owns presentation semantics.
```

## 15. Later acceptance/failure matrix

| Case | Required evidence |
| --- | --- |
| Runtime already running, host starts | TCP hello succeeds through the public endpoint; advertised contract is consumed, not copied. |
| Query | Representative query returns ordinary result data. |
| Mutation | Accepted and terminal completed/failed are observed distinctly. |
| Subscription | Ordered event is delivered through the bounded VM queue; unsubscribe works. |
| Script exits normally | Host detaches once; Runtime/controller/acquisition/Recorder continue. |
| Script throws | Bounded host error/nonzero exit; Runtime continues. |
| Host process killed | Runtime continues; connection-local state is cleaned. |
| Reconnect | Same retained scope follows existing `scope_in_use` detach race and `next_seq`. |
| Admitted mutation, then host death | Reattach, `operation_status`, exact retry returns retained outcome with no second execution. |
| Different payload with same request ID | Existing `request_conflict`; no host override. |
| Slow event consumer | Sixteen-event host queue fills, host closes/reports overflow; Runtime and another client remain healthy. |
| Replay after host overflow/death | Fresh subscription from last delivered cursor either replays or receives authoritative `event_gap`. |
| Pending/result bridge saturation | Host-local bounded failure; no unbounded allocation or Runtime stall. |
| Malformed script-produced request | Local conversion rejects or server scopes public rejection to this connection; Runtime remains healthy. |
| Infinite Steel loop | Runtime continues; cooperative interrupt may stop VM; process kill always remains available. |
| Host panic/VM crash | Runtime continues; admitted operation and experiment state remain authoritative. |
| Runtime shuts down | Host observes EOF/terminal response, closes queues and exits/fails clearly within finite host deadlines. |
| Native controller/acquisition active during host death | Continues unless an existing explicit Runtime safety/lifecycle policy independently changes it. |
| Physical evidence attempt | No binding exists to fabricate observations, ACK/readback, safe evidence or raw output. |
| Windows dependency gate | Exact accepted Steel graph builds locked on the supported toolchain and passes fresh advisory/license review. |

## 16. Proposed M13 decomposition

### M13.1 — architecture/dependency audit

**Goal:** establish the external process, TCP client, trust/lifetime/bounds model and
dependency evidence.

**Production change:** none.

**Gate:** external acceptance and explicit resolution of the `sized-chunks` advisory
before dependency authorization.

### M13.2 — minimal external host and TCP Application transport

**Goal:** add `apps/lab-steel`, one-shot process ownership, bounded TCP client and
hello/query vertical slice.

**Production change:** new external executable only; exact reviewed Steel pin after
the dependency gate is closed. No Runtime change.

**Tests:** locked Windows build, framing/deadlines, hello/query, process exit/detach,
queue bounds, malformed input, Runtime independence.

**Non-goals:** broad bindings, mutation ergonomics, subscription API, REPL,
persistent host.

**Review gate:** dependency/license/advisory lock and proof the executable does not
depend on `lab-runtime`/`lab-core`.

### M13.3 — bounded plain-data and Application bindings

**Goal:** add mechanical JSON/Steel mapping plus request/query/operation/status and
subscription/event functions.

**Tests:** all JSON kinds/null distinction, oversize/cycles/nonfinite rejection,
accepted versus terminal/error distinction, request/msg identity, event queue
overflow.

**Non-goals:** generated DTOs, operation registry copy, SDK, presentation, REPL.

**Review gate:** no semantic API duplication and all host capacities frozen.

### M13.4 — lifecycle, reconnect, backpressure and fault acceptance

**Goal:** prove script/process lifetime independence, retained mutation
reconciliation, replay/gap behavior and healthy Runtime under host faults.

**Tests:** the failure matrix above, including kill, infinite loop, slow consumer,
queue saturation and Runtime shutdown.

**Production change:** only red-oracle corrections within the external host; any
Application/session/Runtime semantic change stops for review.

### M13.5 — real Steel procedure smoke

**Goal:** execute a real Steel script through the external host for hello, query,
safe virtual mutation, event, disconnect/reattach/status/exact retry and committed
state query.

**Non-goals:** UI, long-running supervisor, REPL, physical qualification.

**Review gate:** actual Steel execution evidence, not a Rust client substitute.

### M13 consolidated external review

Re-audit process separation, dependency lock/advisories, bounds, protocol identity,
fault evidence and absence of Runtime/presentation/managed-component ownership
changes. Do not authorize M14 automatically.

This decomposition remains appropriate. M13.2 is not authorized by this report.

## 17. Risks, open questions and exact non-goals

### Risks/open questions for external review

1. **Dependency blocker:** what accepted upstream/pin/fork resolution closes
   RUSTSEC-2026-0255 and the abandoned `im-rc` chain before M13.2?
2. **No declared MSRV:** M13.2 must select and test a supported Rust toolchain; it
   cannot repeat an upstream MSRV claim that does not exist.
3. **Broad dependency footprint:** 126 package identities are new relative to current
   workspace metadata in the probe. License/advisory inventory and package size need
   review at the implementation gate.
4. **Unsafe VM internals:** process separation limits Runtime blast radius but does
   not protect the host; dependency updates remain mandatory.
5. **Steel sandbox limits:** `new_sandboxed` is not hostile-code confinement because
   process/git capabilities remain. Initial scripts must be trusted local code, or a
   later OS/reduced-engine sandbox needs separate design.
6. **Cooperative interruption:** verify behavior for the exact binding calls; never
   rely on it instead of process termination.
7. **Pre-1.0 API:** isolate Steel-specific registration/value conversion inside
   `apps/lab-steel` so upstream changes cannot leak into Application semantics.
8. **One-shot recovery ergonomics:** define a bounded machine-readable summary and
   explicit retained-scope input without making a client checkpoint authoritative.

None requires a new Runtime Application operation. If implementation later proves
otherwise, stop rather than add a Steel convenience operation.

### Exact non-goals

M13.1 and the recommended M13 path do not authorize:

- Steel inside Runtime, HostCore, `lab-core`, managed components or controllers;
- a Steel dependency, crate or executable during M13.1;
- a third transport or privileged same-process channel;
- new Application operations, DTOs, errors, sessions, dedup or history semantics;
- raw serial/resource/Recorder/SQLite/OutputAuthority access;
- fabrication of physical evidence or output-safety state;
- Lua restoration or managed-component migration;
- a persistent host, daemon, script supervisor or REPL in the initial slice;
- dynamic libraries, JIT, FFI/plugin packages or unsafe Rust callbacks;
- authentication, TLS or remote deployment;
- GUI, Workbench, `PresentationDocument` or other M14 semantics;
- hard real-time, hostile-script sandbox or hard memory-isolation claims.

## Preserved invariants

```text
Runtime owns experiment semantics.
Client owns presentation semantics.

client lifetime != experiment lifetime
script lifetime != experiment lifetime
Steel process lifetime != Runtime lifetime

ONE Application API
ONE operation registry
ONE DTO/error/session/dedup/subscription/history semantics

all Steel laboratory mutations pass through ordinary Application admission
admitted work survives client/host loss under the existing retained contract
slow or failed Steel cannot block required Runtime progress
native real-time controllers/components remain native Rust
no UI or scripting semantics enter Runtime core
```

```text
STATUS: M13_1_READY_FOR_EXTERNAL_REVIEW
```

## External review acceptance

External review accepted M13.1 at review-ready commit
`81e303de18021c49c526b12bb1f77f8ea75ae2d9`. The external one-shot TCP/NDJSON host
architecture is accepted. The `steel-core 0.8.3` dependency graph is not accepted;
M13 dependency safety resolution is authorized while M13.2 remains unauthorized.
