# M14.4 — Minimal eframe/egui GUI

## Status

```text
M14.3: ACCEPTED

M14.4 minimal eframe/egui GUI:
READY FOR EXTERNAL RE-REVIEW

M14.5: NOT AUTHORIZED

STATUS: M14_4_REMEDIATION_READY_FOR_EXTERNAL_REVIEW
```

M14.4 adds the first native observational Workbench window. It does not add an
Application operation, Runtime DTO, experiment mutation control, Steel, or Runtime
production change.

## Accepted baseline

The accepted M14.3 implementation commit is
`f01567ba2165b24b9551e3b0acf5b100b0169f32`; the coordination acceptance commit is
`31e44aecac495823f12480d9fab3371fbc08bd4e`.

## External-review remediation

The first M14.4 external review accepted the dependency stack, process-ownership
guard, Glow renderer, wake boundary, ordinary projection barrier, and observational
GUI in principle. It found two later fault-boundary defects; they were not treated as
known before review.

Deterministic red reproductions established both findings before correction:

- applying the ordered-update-pressure resnapshot left the model connection `Ready`;
- `event_gap` left the old live trace buffer populated, so a later sample could join
  an interval whose continuity was not observed.

The correction keeps recovery ownership in the M14.2 worker. The bounded internal
`ClientUpdate::ResnapshotRequired` now includes `connection_lost`. The single deferred
notification produced by `ordered_update_queue_full` sets it to `true`, so it carries
both the resnapshot requirement and the otherwise-lost externally visible transport
state. Event-gap notifications set it to `false` because that transport remains
usable. This is a private Workbench update, not a new Runtime DTO or public error.

When `connection_lost` is true, `WorkbenchModel` immediately changes `Ready` to
`Stale` and retains cached observations for visible stale inspection. The rebuild
coordinator submits exactly one `Connect` command with the retained scope and issues
no snapshot query against the closed transport. The existing worker remains the only
socket/session owner and applies its accepted absolute reattach deadline. Only the
new `Hello` starts the normal discovery/measurement/Reference/fence/subscription
barrier. A usable-connection event gap instead starts that same bounded barrier
without reconnecting.

Live plot buffers now have an explicit continuity epoch:

```text
plain disconnect / Stale
    -> retain cached old points and truncation marker

begin rebuild after reattach, boot change, event gap, or ordered-update loss
    -> clear live buffers and their local truncation counters
    -> retain PresentationDocument and stale entity projections
    -> first later good signal event starts a new line
```

Clearing this display-only cache is not presented as Runtime or Recorder data loss.
Persistent plots/traces are unchanged.

The frozen ownership boundary remains:

```text
eframe main thread
    WorkbenchApp
    WorkbenchModel / PresentationDocument
    GUI selection and rebuild coordinator
    ClientHandle
             |
             | bounded commands / bounded updates
             v
one M14.2 Application client worker
    TcpStream, hello/scope, msg_id, request_id sequencing,
    recovery journal, subscription, cursor, reconnect/reconciliation
             |
             v
TCP / NDJSON -> lab-runtime -> ONE Application / ONE SessionStore / Runtime
```

No socket is shared or placed behind `Arc<Mutex<_>>`. The GUI thread performs no
blocking network I/O and does not wait for Application terminal replies in a frame.

## Changed production surface

- `apps/lab-workbench/src/ownership.rs` adds the Windows workspace-ownership guard.
- `apps/lab-workbench/src/gui/mod.rs` performs guarded persistence loading and starts
  eframe with the Glow renderer.
- `apps/lab-workbench/src/gui/app.rs` owns the minimal observational window, bounded
  update drain, live plot, visible freshness/recovery state, and smoke predicates.
- `apps/lab-workbench/src/gui/rebuild.rs` owns the finite projection/bootstrap
  sequence and its single aggregate subscription.
- `apps/lab-workbench/src/client/worker.rs` accepts an optional renderer-neutral
  wake callback and invokes it only after a `ClientUpdate` is successfully published.
- `apps/lab-workbench/src/model/mod.rs` maps accepted signal events into bounded
  display points only from good finite values and authoritative observed time.
- `apps/lab-workbench/src/main.rs` starts the native GUI. Its command line selects
  the Runtime address, optional retained scope, and optional Workbench workspace.
- `apps/lab-workbench/run-gui-smoke.ps1` is a Windows process-level acceptance
  harness. It creates all smoke state under a temporary directory.

No `lab-runtime` or `lab-core` production file changed.

## Workspace process ownership

M14.3's v0.1 rule is enforced before either `presentation-v1.json` or
`recovery-v1.json` is opened:

```text
one active Workbench process per resolved user-data workspace
```

The candidate workspace is resolved without creating it by canonicalizing its
nearest existing ancestor and lexically appending the missing suffix. A stable
case-folded path digest selects a Windows named mutex in the global namespace.
`WorkspaceOwnership` keeps the OS handle alive for the entire GUI lifetime. Kernel
object release makes normal exit, panic unwind, and forced process termination
non-stranding; there is no create/delete lock file and no project-owned unsafe FFI.
Acquisition or policy failure is visible and occurs before persistent state access.

The process-level regression proves that the first owner excludes a second process,
the first remains able to use its workspace, and a new process acquires ownership
after the first guard exits.

The exact Windows-only helper is:

```toml
win-desktop-utils = { version = "=0.5.7",
                      default-features = false,
                      features = ["instance"] }
```

It is MIT OR Apache-2.0, declares Rust 1.82, and resolves to the official `windows`
0.62.2 family. Its selected API is a safe RAII wrapper over a Windows named mutex.

## GUI dependency and renderer gate

The package pins:

```toml
eframe = { version = "=0.36.2",
           default-features = false,
           features = ["glow", "default_fonts", "accesskit"] }

egui_plot = { version = "=0.37.0", default-features = false }
```

Both are MIT OR Apache-2.0 and declare Rust 1.95. `lab-workbench` therefore declares
`rust-version = "1.95"`; the workspace and Runtime crates were not assigned a new
MSRV. The resolved native path contains `egui`/`egui-winit`/`egui_glow` 0.36.2,
`glow` 0.17.0, `glutin` 0.32.3, `glutin-winit` 0.5.0, and `winit` 0.30.13.

`cargo tree -e features` confirms that only the reviewed eframe features are enabled.
`cargo tree -i wgpu` and `cargo tree -i ron` both report that those packages are
absent. Eframe persistence, egui serde persistence, WGPU, and RON are not enabled.
The Windows `lab-workbench` dependency graph contains 123 resolved package identities
and every package has a
Cargo license expression. A fresh `cargo audit 0.22.2` scan of the full locked graph,
using a 1,273-advisory database, found no vulnerabilities.

The dependency facts were checked against the exact crates.io packages and official
Rust documentation/source. The prior M14.1 maintenance warning for `egui_plot`
remains an explicit risk; it does not alter this small renderer-neutral model/plot
boundary.

Primary dependency sources: [eframe 0.36.2](https://crates.io/crates/eframe/0.36.2),
[egui_plot 0.37.0](https://crates.io/crates/egui_plot/0.37.0),
[win-desktop-utils 0.5.7](https://crates.io/crates/win-desktop-utils/0.5.7), and the
[RustSec advisory database](https://github.com/RustSec/advisory-db).

## Wake and frame ownership

The worker accepts an optional `Arc<dyn Fn() + Send + Sync>` wake callback. The GUI
supplies a closure over cloned `egui::Context` which calls `request_repaint()`.
`ClientUpdate` publication remains the synchronization point: a successful bounded
queue send invokes the callback; a full/disconnected queue cannot manufacture a
wake or an unbounded intermediate queue.

Every GUI frame drains with `try_recv` only and processes at most 64 updates, exactly
the accepted worker update capacity. Unit tests freeze both the wake-after-publish
rule and the 64-update frame budget. The ordinary UI does not continuously repaint;
smoke-only state machines use a bounded 100 ms repaint timer while active.

## Minimal GUI surface

The window is intentionally observational:

- top status shows connection state, global Fresh/Rebuilding/Stale state, boot ID,
  retained scope, presentation-load errors, recovery-journal problems, recovery
  records, reconciliation warnings, and unresolved reference count;
- discovery lists known instruments, signals, References, controllers, resources,
  and components from the centralized model;
- the main area plots one selected live signal and shows its latest observation;
- the status area distinguishes unresolved or stale state and reports local display
  truncation.

Only Workbench Connect and Disconnect actions are exposed. GUI close shuts down the
client worker finitely. It sends no `runtime_shutdown`, controller lifecycle,
Recorder lifecycle, property/configuration mutation, resource reconnect, or Reference
mutation operation.

Presentation loading uses the accepted M14.3 JSON parser after workspace ownership
is acquired. A corrupt file produces a visible client-local error and an empty
in-memory fallback; it is not overwritten. If an empty document later gains a signal
selection, the GUI creates one deterministic in-memory plot through the accepted
transactional `UiCommand` boundary. Live points remain outside the document and
eframe persistence is disabled.

## Rebuild and one-subscription strategy

Widgets never issue Application queries. One `RebuildCoordinator` owns command IDs
and the following finite state machine:

1. `hello` starts `Rebuilding` and obtains the first frozen discovery projection;
2. all discovery pages are consumed and their first revision cursor is retained;
3. frozen current-measurement pages are consumed;
4. each discovered Reference is queried sequentially, bounded to 64 identities;
5. a second discovery projection supplies an event-sequence fence;
6. one aggregate subscription for `signal` and `reference` starts after the first
   cursor, replaying changes concurrent with the snapshots;
7. only subscription event/progress reaching the fence invokes explicit
   `complete_rebuild()` and publishes global Fresh.

This uses only existing Application operations and never infers freshness from the
first reply. Unsupported/malformed projections fail visibly. Disconnect returns the
model to Stale. `event_gap`, ordered-update overflow, boot change, or explicit
`ResnapshotRequired` returns it to a bounded rebuild; no continuity is fabricated.
There is exactly one Runtime subscription for the Workbench connection.

## Live signal data path

The path is:

```text
Application signal event
    -> centralized WorkbenchModel decoder
    -> good quality + finite numeric value + observed_at_ns
    -> 4,096-point RuntimeRef::Signal display buffer
    -> egui_plot Line on the GUI thread
```

Unavailable/null/non-finite samples do not become points. Time is taken from the
Application observation, not GUI wall-clock time. Point 4,097 evicts only the oldest
display point and increments the local truncation marker; this is never represented
as Recorder or Runtime history loss. Runtime is not queried per frame.

## Freshness and recovery presentation

Fresh, Rebuilding, Stale, and Unresolved have distinct text/color treatment. A stale
value can be shown only as a last observation. Connectivity never becomes a claim
about physical quality, ACK, readback, safety, or Recorder durability.

The model continues to consume the worker-owned bounded `RecoveryState` projection.
Resolved recovery records and `ReconciliationRequired` are shown as distinct client
conditions. Journal load, reattach, and rendering do not automatically replay a
mutation. The worker remains the sole recovery and exact-retry authority.

## Deterministic tests

The Workbench suite includes focused non-rendering regressions for:

- named-mutex process exclusion and release;
- publish-triggered renderer-neutral wake;
- a nonblocking 64-update frame drain;
- `hello -> Rebuilding`, explicit cursor barrier -> Fresh, disconnect -> Stale, and
  event gap -> Rebuilding;
- ordered-update pressure publishing one transport-lost resnapshot, model
  `Ready -> Stale`, retained-scope reconnect before any query, new `Hello`, and an
  explicit barrier back to Fresh;
- usable-connection `event_gap` rebuilding without an unnecessary reconnect;
- boot change staling previous observations and Fresh-only reference resolution;
- good finite signal-event admission, authoritative observed time, and rejection of
  unavailable/null/non-finite signal values;
- 4,096-point eviction/truncation;
- stale disconnect retaining cached points, while reattach, boot change, event gap,
  and ordered-update recovery each start a new empty live epoch with reset local
  truncation accounting;
- presentation survival across stale/reconnect state;
- recovery projection versus reconciliation warning presentation;
- finite worker close with no experiment lifecycle operation.

All accepted M14.2/M14.3 client, journal, persistence, model, and real-process tests
remain unchanged except for the narrow wake hook and signal-event projection tests.

## Windows GUI smoke

The native smoke ran on Windows 10 Pro 10.0.19045 x86_64 with Rust/Cargo 1.95.0 and
the final locked tree. The harness started `lab-runtime --profile virtual-demo`, read
its real readiness JSON, then started the real `lab-workbench.exe` native window with
the Glow renderer and a temporary owned workspace.

Observed machine-readable result:

```json
{"status":"pass","renderer":"glow","runtime_pid":15884,"runtime_port":53518,"native_window_observed":true,"minimize_restore_observed":true,"maximum_working_set_bytes":87883776,"idle_cpu_milliseconds_over_2s":0,"idle_working_set_start_bytes":87248896,"idle_working_set_end_bytes":87248896,"initial_fresh":true,"stale_after_disconnect":true,"fresh_after_reattach":true,"live_signal_points":2,"reference_visible":true,"runtime_alive_after_clean_close":true,"runtime_alive_after_forced_termination":true}
```

The observed sequence proves: native window creation, Glow selection, hello,
discovery, a live signal plot, Reference visibility, explicit stale state after
client disconnect, retained-scope reattach/rebuild to Fresh, minimize/restore, finite
clean close, and Runtime survival. A second connected Workbench was terminated
forcibly and Runtime remained alive. A two-second minimized steady-state sample used
0 ms measured CPU and a stable 87,248,896-byte working set; the peak observed across
the smoke was 87,883,776 bytes. These are smoke observations, not general performance
or memory guarantees.

The existing real Runtime process acceptance also passed and continued to prove
hello, Reference bootstrap, subscription/event flow, disconnect, retained-scope
reattach, operation reconciliation/exact retry, clean client shutdown, and independent
Runtime lifetime.

## Verification

The final M14.4 tree was checked with:

```text
cargo fmt --all -- --check
cargo test --workspace --locked
cargo test --workspace --release --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test -p lab-workbench --locked
git diff --check
```

The focused `lab-workbench` suite completed successfully three consecutive times.
The opt-in real Runtime process acceptance and the native GUI smoke also passed from
the final tree. The two pre-existing Runtime rotation/soak tests remain ignored and
unchanged; the Workbench real-process acceptance remains opt-in/ignored in the normal
workspace pass and was run explicitly.

## Risks and remaining M14.5 work

- `egui_plot` maintenance status remains the accepted M14.1 dependency risk.
- The rebuild set is deliberately limited to discovery, current signals, References,
  and one signal/reference subscription. Controller/resource/Recorder detail views
  may remain visibly unresolved until a later reviewed projection set is added.
- Global Windows named mutex creation can be denied by restrictive system policy;
  that is a visible fail-before-persistence startup error, not a fallback to unsafe
  multi-process access.
- M14.5 remains responsible for reviewed operator mutation controls, property and
  configuration workflows, confirmation/error UX, and their accepted lifecycle.

M14.4 adds no Steel, scripting, GUI-owned experiment authority, new transport,
Runtime operation, Runtime DTO/error, server-side presentation state, TLS/auth, or
physical qualification claim.

```text
M14.3: ACCEPTED

M14.4 minimal eframe/egui GUI:
READY FOR EXTERNAL RE-REVIEW

M14.5: NOT AUTHORIZED

STATUS: M14_4_REMEDIATION_READY_FOR_EXTERNAL_REVIEW
```
