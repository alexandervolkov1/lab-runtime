# Workbench user guide

This guide applies to the native `lab-workbench` in the current repository/v0.1
product. Workbench connects to Application protocol v1 but exposes a deliberately
smaller, typed operator surface than the complete Application API.

For external programs, the opt-in [Workbench TCP/NDJSON API](workbench-api.md) exposes
the accepted 15-operation presentation/client surface through the same owner. Its
`call_id` correlation and recovery semantics are distinct from direct Runtime access.

## What Workbench is

`lab-runtime.exe` and `lab-workbench.exe` are separate processes with different
authority:

```text
lab-runtime.exe                    lab-workbench.exe
---------------                    -----------------
authoritative experiment state    operator and presentation client
devices and resources             observed projections and selections
References and controllers        plots and bounded display buffers
output authority                  drafts and confirmations
Recorder and configuration        client recovery evidence
sessions and operation outcomes   local presentation state
```

Runtime owns experiment semantics. Workbench owns presentation semantics.
Workbench can express control intent through the Application API, but it does not
own instrument truth, controller or output authority, Recorder facts, Runtime
session/deduplication truth, physical acknowledgements, readback, or physical effect.

Workbench lifetime is not Runtime lifetime, and GUI lifetime is not experiment
lifetime. Closing, crashing, or disconnecting Workbench does not shut down Runtime,
stop its Recorder, pause a controller, or undo work already admitted by Runtime.

## Start Workbench

Start Runtime separately and wait for its readiness JSON before starting Workbench.
For the `virtual-demo` example in [Getting started](getting-started.md), open a second
PowerShell terminal and run:

```powershell
$workspace = [IO.Path]::GetFullPath((Join-Path $PWD ".workbench-demo"))

cargo run -p lab-workbench --locked -- `
  --connect 127.0.0.1:7420 `
  --workspace $workspace
```

From an extracted preview package, run the packaged executable instead:

```powershell
./lab-workbench.exe --connect 127.0.0.1:7420 --workspace $workspace
```

`--connect` is required and accepts a numeric socket address. `--workspace` is
optional; when omitted, Workbench uses its Windows user-data location. An optional
`--scope` value requests that retained Application scope.

Workbench acquires exclusive process ownership of the resolved workspace, loads its
presentation and recovery files when present, and immediately starts connecting. A
second Workbench process cannot use the same workspace at the same time.

The native Workbench currently runs on Windows and uses TCP/NDJSON. Runtime may also
offer loopback WebSocket/JSON to other clients; Workbench does not currently have a
WebSocket transport.

## Connection and freshness

The top status row reports the connection state, observation freshness, Runtime boot
identity, and retained scope. The connection commonly progresses through
`Connecting`, `AwaitingHello`, and `Ready`. `Reattaching`, `Disconnected`, `Stale`,
`Stopping`, and `Stopped` appear when their respective lifecycle conditions apply.

Connection state and observation freshness answer different questions:

- **Ready** means the Application connection completed hello and is usable. It does
  not mean every required projection has been rebuilt.
- **Rebuilding** means Workbench is rebuilding a coherent observation set and
  catching up its aggregate event subscription.
- **Fresh** means that rebuild and catch-up barrier completed for the attached
  Runtime boot/session/event stream.
- **Stale** means cached observations may still be shown, but they are not evidence
  of current Runtime state.
- **Unknown** means Workbench has not obtained the observation in this client
  lifetime.

The detail pane can also mark an entity **Unresolved in the current Runtime view**
when a stored reference no longer resolves in current discovery.

Keep these distinctions in mind:

```text
connected socket != Ready hello != Fresh observations
Completed operation != Fresh observation
Fresh observation != mutation reconciliation
```

Workbench disables mutation controls unless the connection, overall observation
set, selected target, advertised operation, and required full detail are all current.
Recovery or journal problems can impose additional fail-closed restrictions.

## Discovery and observations

The **Discovery** pane lists entities learned from Runtime. Current entity types are:

- instruments;
- signals;
- References;
- controllers;
- resources;
- components;
- the Recorder;
- configuration properties.

Select an entity to show its latest bounded Application projection and any controls
that apply to that type. The detail pane labels the value as a **Fresh observation**,
**Cached last observation (stale)**, **Unresolved in the current Runtime view**, or
**Unknown**.

A cached value remains useful context during recovery, but it is not current physical
truth. Workbench never promotes its cache into Runtime authority.

## Live signal plots

Selecting a signal displays a native live plot populated by good, finite signal
events from the Application subscription. Workbench does not query Runtime on every
rendered frame and does not invent points for unavailable, bad-quality, null, or
non-finite samples.

Each signal's display buffer is Workbench-owned and bounded to 4,096 points. When the
buffer fills, the oldest display points are evicted and the GUI reports the local
truncation count. A plain Disconnect or Stale transition preserves existing live
points as stale cached presentation. When Workbench begins a new rebuild observation
epoch after reattach, a Runtime boot change, an event gap, or ordered-update
continuity loss, it clears the live display buffers rather than drawing a misleading
line between two observation epochs. This is a local presentation reset, not loss of
Runtime history or Recorder data.

This plot is a live presentation aid, not durable experiment history. Local eviction
does not delete Runtime history or Recorder data. Workbench does not currently
provide a durable history browser; see [Recorder and SQLite](recorder-sqlite.md) for
the durable archive.

## Typed operator controls

Workbench offers typed workflows for a selected safe subset of the Application API.
A disabled control is not an error: it can mean the target is stale, the operation is
not advertised by this Runtime composition, the observed lifecycle state does not
allow the action, required detail is stale, another mutation is unresolved, or a
recovery problem is blocking authority.

An enabled control means the current client-side eligibility checks passed. It does
not prove future admission, device acknowledgement, readback, or physical effect.

### References

For a selected Reference, Workbench can prepare:

- **Configure fixed** with a finite fixed value;
- **Configure ramp** with a finite target and a positive finite rate;
- **Retune** for a Reference currently observed as a ramp.

The confirmation captures the authoritative Reference revision. If the revision or
target observation changes before confirmation, the draft becomes stale and must be
cancelled and reviewed again; Workbench does not silently rebase it.

### Controller lifecycle

For a selected controller, Workbench exposes lifecycle actions only when applicable
to the observed state:

- **Start** from ready;
- **Pause** from warming or running;
- **Resume** from paused;
- **Reset failed** from failed.

These buttons submit Runtime-owned lifecycle intent. They do not bypass output
authority, leases, safety checks, or controller policy.

### PID configuration

The controller editor is intentionally PID-specific. It accepts finite `kp`, `ki`,
`kd`, output-minimum, and output-maximum values, with minimum no greater than maximum.
Configuration requires a fresh full controller detail and uses the current revision
in its confirmation.

Workbench does not provide a general controller-policy editor.

### Configuration properties

Workbench displays configuration-property type and access metadata. Its current
mutation surface is deliberately limited to writable (`read_write`) **Integer** and
**Text** properties. Read-only values and other types, including Boolean, Number, and
enum-like observations, may be displayed but are not universally editable here.

Deployment staging, apply, and reload are Application API capabilities, not current
Workbench workflows.

### Resource reconnect

A reconnect-capable resource with fresh full detail offers **Reconnect**. The action
uses the observed binding generation and requires confirmation.

Resource reconnect is not controller rearm and is not proof that transport recovery
or a physical operation succeeded. Review the later authoritative resource and
controller observations separately.

### Recorder

When Recorder state is present, Workbench shows its authoritative Runtime state and
offers:

- **Start recording** from idle with a non-empty label;
- **Stop recording** when the projection contains an active run.

Stop uses the actual active run identity shown by Runtime. Both actions use the same
confirmation and operation lifecycle as other mutations.

Recorder is Runtime-owned. Closing or disconnecting Workbench does not implicitly
stop an active recording. See [Recorder and SQLite](recorder-sqlite.md) for archive
and durability semantics.

## Confirmation and operation lifecycle

A normal operator action follows this user-visible sequence:

```text
draft
  -> Confirmation required
  -> Submitted
  -> Accepted
  -> Completed | Failed
```

- **Confirmation required** shows the exact prepared intent and any warning. Clicking
  **Confirm** expresses intent only.
- **Submitted** means the client handed off the request; Runtime admission is not yet
  known.
- **Accepted** is authoritative admission evidence, but the operation may still be in
  progress.
- **Completed** is an authoritative terminal operation outcome. It is not a general
  claim of physical effect and does not replace the current entity projection.
- **Failed** means the submitted workflow ended in a terminal mutation failure, an
  Application rejection/failure, or a correlated local client rejection. A local
  client rejection is not Runtime admission, completion, or physical-result
  evidence. Review current projections before preparing another action.

If the underlying authoritative revision or state changes while a confirmation is
open, Workbench invalidates the draft. Conflicts and reconnects do not cause silent
rebasing or automatic mutation retry.

## Disconnect, reconnect, and Runtime independence

### Explicit Disconnect

The **Disconnect** button closes the current client connection, discards
connection-local work, and marks cached observations stale. It preserves exact
recovery evidence and starts no automatic reconnect. After **Disconnected** is
visible, **Connect** is a separate explicit action.

Disconnect does not send Runtime shutdown, pause a controller, stop the Recorder, or
change experiment ownership.

### Unexpected connection loss

Unexpected continuity loss after an attached retained scope may start one bounded
automatic reattach episode. During this period the connection can show
**Reattaching**, observations remain stale, and mutation controls are unavailable.

Successful transport reattach still has to complete a new authoritative rebuild and
catch-up barrier before observations become Fresh. If the episode expires or Runtime
rejects the old scope/instance, Workbench becomes Disconnected and requires an
explicit connection action.

Automatic transport reattach is not automatic mutation recovery. Workbench does not
automatically submit a mutation, Check Status, or Exact Retry.

Closing the window stops only the Workbench client process. A Workbench crash likewise
does not stop Runtime, controllers, the Recorder, or the experiment. Stop Runtime
through its own explicit process lifecycle.

## Recovery and reconciliation

The **Recovery / reconciliation** section presents bounded, exact client evidence for
mutations whose lifecycle matters across connection loss or process restart:

- **Pending**: the exact mutation was durably prepared before wire emission; Runtime
  admission is not yet authoritative.
- **Ambiguous**: continuity was lost after transmission might have begun, so admission
  or outcome is unresolved.
- **Accepted**: Runtime authoritatively admitted the identity; terminal outcome may
  still be pending.
- **Completed**: Runtime reported a successful terminal operation outcome.
- **Failed**: Runtime reported a terminal failed outcome.

Operation outcome is not current physical state. Use normal fresh Runtime
observations for the entity's current projection.

### Check Status

**Check Status** is an explicit one-shot lookup for one exact retained mutation
identity. It is not polling and is never sent automatically. Connection loss
interrupts the request without automatic resubmission.

An `outcome_unknown` status means Runtime currently has no retained operation-state
evidence for that exact identity. It does not become Completed or Failed, prove prior
execution or non-execution, describe physical state, or permit blind retry.

### Exact Retry

**Exact Retry…** opens a separate confirmation showing the retained scope, sequence,
operation, admission evidence, and read-only arguments. **Confirm Exact Retry** sends
the same retained request identity, operation, and normalized payload; the user does
not edit or reconstruct them.

If Runtime already admitted the retained identity and still has its deduplication
record, the same exact request cannot execute a second time. If the original request
never reached Runtime admission and the identity is still admissible, the exact
resubmission may be its first execution. An `outcome_unknown` result can remain
unresolved. For these reasons, Exact Retry is a manual reconciliation tool, not a
universal "safe retry," and it is never automatic.

### Quarantined evidence

Evidence from an incompatible old Runtime boot or scope remains visible as
**Quarantined recovery evidence**. It is not attached authority for the current
session: Check Status and Exact Retry are unavailable for it, and new experiment
mutations remain blocked where the current recovery state requires fail-closed
behavior.

When quarantine is present, the disconnected connection control is labelled
**Connect new scope**. It starts a separate explicit connection action; it does not
attach the quarantined evidence to the new session or make that evidence retryable.

Workbench has no Discard or Forget action. Detailed fault diagnosis and recovery
policy are outside this operator overview; the protocol-level lifecycle is described
in [Events, mutations, and recovery](api/events-mutations-and-recovery.md).

## Workspace and presentation state

A workspace is client-owned storage for one Workbench process. On Windows, an
OS-owned guard rejects a second process using the same resolved workspace. The guard
is released when the owning process exits, including abnormal termination.

The workspace uses separate files for presentation and recovery state:

- `presentation-v1.json` contains bounded, validated `PresentationDocument` state;
- `recovery-v1.json` contains bounded exact mutation-recovery evidence.

Neither file is Runtime experiment state. Presentation includes client concepts such
as plot definitions and Runtime references; live samples are not persisted in the
document. Recovery evidence is separate so presentation changes cannot erase or
reinterpret mutation authority.

The current persistence layer is a foundation, not a full layout editor. The GUI has
no Save Layout button, dashboard designer, arbitrary plot editor, multi-window editor,
or user-facing persistence manager. If no plot is loaded, the GUI creates its current
default live-signal plot in memory after discovery.

If a presentation file is invalid or unreadable, Workbench reports a client-local
problem and uses a safe empty presentation for the current process. It does not treat
the invalid document as authoritative or silently replace the source file. A recovery
journal failure is also visible; observations can still be available, but mutation
authority fails closed because exact recovery persistence cannot be trusted.

## Not available in the current GUI

Workbench intentionally does not expose every Application operation as a GUI action.
The current GUI does not provide:

- a raw full-operation console;
- a durable history browser;
- experiment annotation controls;
- Runtime shutdown controls;
- deployment stage/apply/reload controls;
- a general controller-policy editor;
- a Workbench WebSocket transport;
- Discard or Forget recovery actions;
- an embedded scripting or automation runtime.

The broader API and the narrower typed GUI serve different clients and risk profiles;
their difference is intentional. Future automation clients or procedures must use
the same language-neutral Application semantics.

## Related documentation

- [Getting started](getting-started.md) — run the virtual profile and complete a
  first Reference and Recorder workflow.
- [System architecture](architecture.md) — Runtime/Workbench authority and process
  lifetime.
- [Application API reference](api/README.md) — full operation, session, event,
  mutation, error, and limit semantics.
- [Recorder and SQLite](recorder-sqlite.md) — durable recording behavior.
- [Safety and failure behavior](safety-and-failures.md) — output certainty and
  fail-closed behavior.
