# M17.1 external client contract and ownership freeze

## 1. Status and scope

```text
M16 consolidated acceptance: ACCEPTED
Pre-M17 cleanup and readiness: COMPLETE
M17.1 external review: ACCEPTED
M17.1 status: ACCEPTED
M17.2 implementation: ACCEPTED
M17.3 implementation: ACCEPTED
M17.4 implementation: ACCEPTED
Consolidated M17.1-M17.4 review: ACCEPTED (B1-B5 remediated and independently closed)
M17.5: AUTHORIZED / READY TO START / NOT IMPLEMENTED after the M17.4 acceptance commit
```

This document freezes the proposed M17 external boundary before implementation. It
is derived from the current source at
`229fb7d0eadab970249cb088aefae7e30914e6aa` and the accepted M12, M14, and M16
contracts. M17.1 changes coordination and design documentation only. It adds no Rust
source, dependency, listener, operation, client SDK, GUI behavior, Steel integration,
or Runtime/Application semantic.

The pre-M17 source audit found the existing Runtime transport boundary ready for
reuse:

```text
TCP / NDJSON --------+
                     +-> one connection coordinator -> one Application
WebSocket / JSON ----+                              -> one SessionStore
                                                    -> one Runtime owner
```

M17 must not create another Runtime API, transport framework, `Application`,
`SessionStore`, operation registry, mutation sequencer, recovery engine,
subscription system, event history, or experiment owner.

The M17 product boundary has two deliberately distinct client surfaces:

```text
direct Runtime Application API
    authoritative laboratory operations and observations

Workbench client API
    Workbench-owned presentation plus mediated use of Workbench's one
    existing Runtime Application client
```

They are not two implementations of Runtime semantics. A caller chooses the surface
according to ownership, not according to programming language.

## 2. Frozen ownership model

### Runtime owns

- instrument, signal, Reference, controller, output, resource, configuration, and
  Recorder semantics;
- committed observations, event order, durable history, and scientific provenance;
- Application scopes, authoritative mutation sequence, normalized mutation
  identity, admission, deduplication, and retained operation outcomes;
- `OutputAuthority`, transport generations, ACK/readback/effect evidence, safe
  transitions, and physical ambiguity;
- all 43 current Application operations, 26 capabilities, public Runtime errors,
  and the accepted TCP/WebSocket bounds.

### Workbench owns

- its one existing private Runtime Application client worker;
- socket/framing state, caller-local worker command IDs, observed hello/scope, one
  aggregate Runtime subscription, recovery journal, and bounded reattach behavior;
- `WorkbenchModel`, freshness/rebuild state, operator workflow state, and bounded
  display caches;
- the active `PresentationDocument`, presentation validation, local revision, GUI
  selection, layout, plots, traces, controls, labels, and other display choices;
- the future bounded Workbench client endpoint and its connection-local correlation.

### External callers own

- their own connection and `call_id` allocation;
- their source-language data structures, control flow, procedure logic, and local
  persistence;
- direct Runtime Application scope/sequence/recovery state when they choose the
  direct Runtime surface;
- deciding whether lost Workbench-call correlation requires human reconciliation.

An external caller, browser, GUI, Workbench adapter, or future script never becomes
an experiment owner. Workbench presentation is authoritative only as presentation.
It is never laboratory, Recorder, ACK, readback, safe-state, or physical-effect
authority.

## 3. Surface A: direct Runtime Application API

The direct Runtime surface is the existing Application protocol. It remains exposed
over bounded local TCP/NDJSON and optional loopback WebSocket/JSON through the same
transport-neutral codec, coordinator, `Application`, and `SessionStore`.

Callers use this surface for:

- discovery and authoritative committed queries;
- live events and replay through the existing single-subscription contract;
- Runtime mutations and their authoritative `{scope, seq}` identity;
- operation status, exact resubmission under the existing deduplication rules, and
  caller-owned reconnect/recovery;
- Recorder lifecycle and durable history;
- configuration, SimpleDevice provisioning, resources, controllers, References,
  virtual instruments, and explicit Runtime shutdown, subject to advertised
  capabilities and current Runtime policy.

The direct surface remains the preferred boundary for an external procedure runner
that needs its own durable mutation correlation. Clojure/Babashka may use TCP/NDJSON.
Browser code may use the accepted WebSocket/JSON endpoint. Neither requires
Workbench to be running.

M17 adds no operation or alternate envelope to this surface. Clients must negotiate
the actual operations, capabilities, limits, boot, scope, next sequence, and event
cursors returned by Runtime `hello`. M17 must not copy those facts into a competing
Workbench registry.

## 4. Surface B: Workbench client API

The Workbench surface is a language-neutral, local, client-side contract. Its first
external adapter is a separate opt-in IPv4-loopback TCP/NDJSON endpoint owned by the
Workbench process. The endpoint is not a Runtime listener, does not consume another
Runtime client slot, and does not expose `ServiceHost`, `HostCore`, `lab-core`,
Recorder storage, serial transport, or output authority.

M17.2 first implements the transport-independent Workbench dispatcher and ownership
path. M17.3 may attach the bounded loopback adapter to that same dispatcher. A later
in-process Steel adapter, if separately authorized and dependency-safe, must use the
same dispatcher and admission rules. A future Workbench WebSocket adapter would also
require separate review; M17.1 does not authorize it.

The endpoint is local and unauthenticated, matching the current local-first product
boundary. Non-loopback binding, remote access, TLS, authentication, authorization,
proxying, origin policy, or Internet exposure are not part of M17.

### 4.1 Workbench operations

The v1 Workbench registry is fixed to these operations:

| Operation | Kind | Owner/effect |
| --- | --- | --- |
| `hello` | local query | Attach one external connection to this Workbench process and return protocol/instance/limit data. No Runtime scope is created. |
| `client_status` | local query | Return Workbench's observed Runtime-client state, freshness and bounded recovery summary. It is explicitly observational. |
| `recovery_get` | local query | Enumerate one active or quarantined Workbench recovery record at a time under an exact process-local generation fence. It does not reconcile or retry. |
| `lab_query` | mediated lab command | Submit an ordinary query through the existing `ClientHandle::query` path. |
| `lab_mutation` | mediated lab command | Submit a Runtime mutation through the existing `ClientHandle::mutation` path; the worker alone assigns `{scope, seq}` and journals recovery evidence. |
| `lab_operation_status` | mediated lab command | Manually ask status for an exact active worker-owned recovery identity through the existing status path. |
| `presentation_get` | local query | Return an atomic clone of the current validated `PresentationDocument` and its process-local revision. |
| `ui_add_plot` | local mutation | Apply existing `UiCommand::AddPlot`. |
| `ui_remove_plot` | local mutation | Apply existing `UiCommand::RemovePlot`. |
| `ui_add_trace` | local mutation | Apply existing `UiCommand::AddTrace`. |
| `ui_remove_trace` | local mutation | Apply existing `UiCommand::RemoveTrace`. |
| `ui_set_trace_visibility` | local mutation | Apply existing `UiCommand::SetTraceVisibility`. |
| `ui_set_time_window` | local mutation | Apply existing `UiCommand::SetTimeWindow`. |
| `ui_rename_item` | local mutation | Apply existing `UiCommand::RenamePresentationItem`. |
| `ui_set_trace_source` | local mutation | Apply existing `UiCommand::SetTraceSource`. |

There is no Workbench `subscribe`, Runtime-event relay, history cache, arbitrary raw
Runtime request, exact retry, connect/disconnect, Runtime shutdown shortcut,
presentation replace, presentation save, script evaluation, filesystem access, or
raw serial/device operation in v1.

`lab_query` and `lab_mutation` deliberately name the requested lane rather than
reproduce Runtime's registry. The worker emits a query without a Runtime
`request_id`, or a mutation with its worker-owned exact identity. Runtime's existing
wire and operation registry remains authoritative and rejects a wrong lane,
unsupported operation, or invalid arguments with its normal public error.

An explicit `runtime_shutdown` submitted as `lab_mutation` is still the existing
Runtime operation, not Workbench lifecycle behavior. Closing Workbench or a caller
never synthesizes it.

`lab_operation_status` is admitted only when the exact identity is present in the
active, non-quarantined Workbench recovery projection for the currently attached
boot/scope. It is one explicit manual request. The Workbench endpoint never polls
status and never offers Exact Retry in v1.

### 4.2 Presentation concurrency

Workbench has one presentation owner. GUI actions, external calls, and any later
adapter must enter the same serialized dispatcher. They may not mutate
`PresentationDocument` independently.

At process start, the active validated document has revision `1`. Each successful
presentation mutation increments a checked `u64` revision exactly once. The revision
is process-local and is not stored in the presentation file. Workbench also creates a
fresh 32-lowercase-hex-character `workbench_id` at every process start.

Every `ui_*` request must include:

```json
{
  "expected": {
    "workbench_id": "0123456789abcdef0123456789abcdef",
    "revision": "7"
  }
}
```

The serialized owner first compares the expected `workbench_id` and revision with
the active values. A mismatch returns `revision_conflict` before cloning, applying,
or fully validating a candidate. This ordering avoids expensive work for a known
stale request but does not change its semantics. After a successful precheck,
Workbench clones the active document, applies the existing typed `UiCommand`,
validates the complete candidate, atomically replaces active state, and increments
the checked revision exactly once. An invalid candidate returns
`invalid_presentation`. Revision conflict and invalid candidate both leave active
state and revision unchanged.

Success means the active in-memory presentation changed atomically. M17 does not add
automatic disk persistence or a save operation. Existing presentation persistence
remains Workbench-owned and separately bounded; no API result may imply a durable
write that did not occur.

Presentation commands cannot send a Runtime request. Runtime references in plots or
traces select observed entities only; changing them cannot fabricate an entity,
observation, ACK, readback, output, operation outcome, or freshness.

### 4.3 Normative operation arguments and results

This section is normative for all 15 Workbench v1 operations. Every operation
`args` value is an object with exactly the fields listed below. Unknown fields are
rejected at every Workbench-owned level. Required fields may not be omitted. There
are no optional fields unless this section explicitly permits `null`.

Runtime operation arguments inside `lab_query.args.args` and
`lab_mutation.args.args` are the one exception to Workbench-owned field validation:
Workbench requires a bounded JSON object and carries it unchanged to the existing
worker. The Runtime's single operation registry and strict operation decoder remain
the authority for its allowed fields and semantics. Unknown Runtime argument fields
are therefore still rejected by the existing Runtime decoder and returned in the
terminal `lab_update`; Workbench does not copy or reinterpret that registry.

All `u64` identities, revisions, generations, indices, counts, and sequence values
in the Workbench protocol are lowercase canonical decimal strings: `"0"` or a
nonzero digit followed by digits, with no sign, whitespace, exponent, or leading
zero. JSON integers are used only for protocol version and fixed advertised limits.

#### `hello`

Exact arguments:

```json
{}
```

Exact result:

```json
{
  "protocol": {"id": "lab-runtime.workbench", "version": 1},
  "workbench_id": "0123456789abcdef0123456789abcdef",
  "operations": [
    "hello", "client_status", "recovery_get", "lab_query",
    "lab_mutation", "lab_operation_status", "presentation_get",
    "ui_add_plot", "ui_remove_plot", "ui_add_trace", "ui_remove_trace",
    "ui_set_trace_visibility", "ui_set_time_window", "ui_rename_item",
    "ui_set_trace_source"
  ],
  "limits": {
    "json_body_bytes": 2097152,
    "frame_bytes": 2097153,
    "json_depth": 16,
    "json_values": 16384,
    "json_string_bytes": 512,
    "callers": 8,
    "caller_input_messages": 4,
    "caller_input_bytes": 4194304,
    "owner_mailbox_messages": 32,
    "owner_mailbox_bytes": 16777216,
    "caller_output_messages": 4,
    "caller_output_bytes": 4194304,
    "reserved_call_ids_per_caller": 8,
    "reserved_call_ids_total": 32,
    "outstanding_lab_calls_per_caller": 8,
    "outstanding_lab_calls_total": 32,
    "client_deadline_ms": 2000,
    "presentation_document_bytes": 1048576,
    "recovery_active_records": 8,
    "recovery_quarantined_records": 8
  },
  "runtime_client": {},
  "recovery": {},
  "presentation": {}
}
```

`runtime_client`, `recovery`, and `presentation` are exactly the corresponding
objects from `client_status` below, captured in the same serialized owner turn.
`operations` contains exactly the complete ordered list shown above. `hello` creates
no Runtime scope and no resumable Workbench session.

#### `client_status`

Exact arguments:

```json
{}
```

Exact result:

```json
{
  "workbench_id": "0123456789abcdef0123456789abcdef",
  "runtime_client": {
    "connection": "ready",
    "freshness": "fresh",
    "hello": {
      "boot_id": "0123456789abcdef0123456789abcdef",
      "scope": "0123456789abcdef0123456789abcdef:1",
      "next_seq": "4",
      "operations": ["hello", "discover"],
      "capabilities": [],
      "limits": {},
      "event_oldest": {"boot_id": "0123456789abcdef0123456789abcdef", "seq": "0"},
      "event_latest": {"boot_id": "0123456789abcdef0123456789abcdef", "seq": "9"}
    }
  },
  "recovery": {
    "recovery_generation": "3",
    "active_count": "1",
    "quarantined_count": "0",
    "reconciliation_required_count": "1"
  },
  "presentation": {"presentation_revision": "7"}
}
```

`connection` is exactly one of `disconnected`, `connecting`, `awaiting_hello`,
`reattaching`, `ready`, `stale`, `stopping`, or `stopped`. `freshness` is exactly one
of `unknown`, `rebuilding`, `fresh`, or `stale`. `hello` is either `null` or the
complete last observed existing `HelloState` shape shown above. Its operations,
capabilities, limits, boot/scope/sequence, and event cursors are copied from Runtime
and are explicitly observations; Workbench does not author them. Counts describe the
atomic current Workbench recovery projection.

#### `recovery_get`

First-read arguments bind enumeration to the current projection:

```json
{"kind":"active","index":"0","expected":null}
```

Every subsequent read, including a switch from active to quarantined records, uses
the exact identity/generation returned by the first result:

```json
{
  "kind": "active",
  "index": "1",
  "expected": {
    "workbench_id": "0123456789abcdef0123456789abcdef",
    "recovery_generation": "3"
  }
}
```

`kind` is exactly `active` or `quarantined`. `index` is a canonical decimal string.
`expected` is exactly `null` or the object shown; it may be `null` only for the first
read at index `"0"` of an enumeration. Indices address the exact order in the atomic
worker projection; Workbench does not reorder records within a generation.

A matching generation with a record returns:

```json
{
  "state": "record",
  "workbench_id": "0123456789abcdef0123456789abcdef",
  "recovery_generation": "3",
  "counts": {"active":"2","quarantined":"1"},
  "kind": "active",
  "index": "0",
  "record": {
    "boot_id": "0123456789abcdef0123456789abcdef",
    "request_id": {"scope":"0123456789abcdef0123456789abcdef:1","seq":"4"},
    "op": "reference_retune",
    "args": {},
    "admission": "ambiguous"
  },
  "next_index": "1"
}
```

For a record, `next_index` is the next canonical index when another record exists and
is `null` for the last record. At the exact end of the selected kind, it returns the
same fields with `state:"end"`, `record:null`, and `next_index:null`. An index greater
than the selected count is `invalid_args`.

If either expected value differs from the current Workbench identity/generation,
the result is deterministic and contains no record:

```json
{
  "state": "restart",
  "workbench_id": "fedcba9876543210fedcba9876543210",
  "recovery_generation": "4",
  "counts": {"active":"1","quarantined":"1"},
  "kind": "active",
  "index": "1",
  "record": null,
  "next_index": null
}
```

After `restart`, the caller restarts at index `"0"` using the returned
`workbench_id` and `recovery_generation` as `expected`. If the projection changes
again, the next call returns `restart` again. A caller may use one matching generation
to enumerate both kinds; any change requires restarting the whole enumeration.

Current source supplies each active/quarantined recovery update as one atomic complete
`ClientUpdate::RecoveryProjection`; it does not currently supply a change counter.
The recovery generation is therefore a checked process-local dispatcher `u64`,
initially `1`. The single Workbench dispatcher increments it exactly once before
publishing each atomic projection whose active or quarantined content differs from
the prior accepted projection. Repeated identical projections do not advance it.
Exhaustion fails the Workbench client boundary closed. The generation resets only
with a new `workbench_id`; it is a snapshot fence, not recovery authority or durable
state.

An active `record` has exactly `boot_id`, `request_id`, `op`, `args`, and
`admission`. `admission` is `pending`, `accepted`, `ambiguous`, `completed`, or
`failed`. A quarantined result uses the same outer result but its `record` is exactly:

```json
{
  "record": {
    "boot_id": "0123456789abcdef0123456789abcdef",
    "request_id": {"scope":"0123456789abcdef0123456789abcdef:1","seq":"4"},
    "op": "reference_retune",
    "args": {},
    "admission": "ambiguous"
  },
  "reason": "instance_changed"
}
```

`reason` is exactly `instance_changed`, `scope_unknown`,
`attached_boot_mismatch`, or `attached_scope_mismatch`, matching the existing
`RecoveryQuarantineReason`. This operation does not retain a snapshot, token,
history, record copy, or second recovery store.

#### `lab_query`

Exact arguments:

```json
{"op":"reference","args":{"reference":"1"}}
```

`op` is the requested Runtime operation name and `args` is its existing Runtime
arguments object. Workbench submits exactly `ClientHandle::query(op, args)`. It does
not allocate a Runtime mutation identity. The immediate result is exactly the common
lab-submission result `{"state":"submitted","command_id":"41"}`. The originating
caller later receives one terminal `lab_update` containing the exact Runtime result
or public-error envelope.

#### `lab_mutation`

Exact arguments:

```json
{"op":"reference_retune","args":{"reference":"1","expected_revision":"2","target":5.0,"rate":1.0}}
```

`op` is the requested Runtime mutation name and `args` is its existing Runtime
arguments object without `request_id`. Workbench submits exactly
`ClientHandle::mutation(op, args)`. The existing worker alone selects Runtime
`msg_id`, scope, and next sequence and constructs the Runtime mutation envelope. The
immediate result is the common lab-submission result. Later `lab_update` frames carry
the exact Runtime accepted and terminal envelopes. `submitted` is not Runtime
admission.

#### `lab_operation_status`

Exact arguments identify one caller-visible active record obtained from
`recovery_get`:

```json
{
  "target": {
    "workbench_id": "0123456789abcdef0123456789abcdef",
    "recovery_generation": "3",
    "boot_id": "0123456789abcdef0123456789abcdef",
    "request_id": {"scope":"0123456789abcdef0123456789abcdef:1","seq":"4"}
  }
}
```

The target must match the current Workbench identity/generation, `boot_id` and
`request_id` of one current active, non-quarantined recovery record, and the currently
attached Runtime boot/scope. The dispatcher rechecks this condition in its serialized
turn, then submits exactly `ClientHandle::operation_status` with that record's
existing `MutationIdentity`. Absent, changed, quarantined, or wrong-session targets
return `recovery_unavailable`. The immediate result is the common lab-submission
result; the terminal `lab_update` contains the exact Runtime `operation_status`
result or public error. No status polling or retry is implied.

#### `presentation_get`

Exact arguments:

```json
{}
```

Exact result:

```json
{
  "workbench_id": "0123456789abcdef0123456789abcdef",
  "presentation_revision": "7",
  "document": {"format_version":1,"document_id":"main","windows":[],"plots":[],"controls":[]}
}
```

`document` is an atomic clone using the exact existing serde representation of
`PresentationDocument` in `apps/lab-workbench/src/presentation/document.rs`:
`format_version`, `document_id`, `windows`, `plots`, and `controls`, including the
existing strict nested `PresentationWindow`, `PresentationTab`, `Panel`, `PanelKind`,
`Plot`, `AxisOptions`, `Trace`, `TraceStyle`, `Control`, and `ControlKind` DTOs. This
source DTO is the normative presentation shape; M17 creates no parallel document
schema.

#### Common presentation expectation and result

Every `ui_*` operation has required `expected` plus the operation-specific fields
below. `expected` has exactly `workbench_id` and canonical decimal-string `revision`.
Every successful `ui_*` result has exactly:

```json
{
  "workbench_id": "0123456789abcdef0123456789abcdef",
  "presentation_revision": "8"
}
```

`plot` and `trace` use the exact existing strict serde DTOs:

```json
{
  "id": "plot-1",
  "title": "Temperature",
  "time_window_seconds": 60.0,
  "axes": {"y_min": null, "y_max": null},
  "traces": []
}
```

```json
{
  "id": "trace-1",
  "source": {"kind":"signal","instrument":"1","parameter":"1"},
  "display_label": "Temperature",
  "visible": true,
  "style": {"color":"#ffffff","width":1.0},
  "display_unit": null
}
```

`RuntimeRef` is normatively the existing internally tagged, snake-case,
unknown-field-denying DTO in `apps/lab-workbench/src/presentation/reference.rs`:
`instrument`, `signal`, `reference`, `controller`, `resource`, `component`,
`recorder`, or `configuration_property`. Each variant accepts exactly its existing
source fields; `configuration_property.owner` uses the existing `instrument`,
`component`, or `resource` `ConfigurationOwner` shape.

#### `ui_add_plot`

```json
{"expected":{"workbench_id":"0123456789abcdef0123456789abcdef","revision":"7"},"plot":{"id":"plot-1","title":"Temperature","time_window_seconds":60.0,"axes":{"y_min":null,"y_max":null},"traces":[]}}
```

Applies exactly `UiCommand::AddPlot { plot }` and returns the common presentation
result.

#### `ui_remove_plot`

```json
{"expected":{"workbench_id":"0123456789abcdef0123456789abcdef","revision":"7"},"plot_id":"plot-1"}
```

Applies exactly `UiCommand::RemovePlot { plot_id }` and returns the common result.

#### `ui_add_trace`

```json
{"expected":{"workbench_id":"0123456789abcdef0123456789abcdef","revision":"7"},"plot_id":"plot-1","trace":{"id":"trace-1","source":{"kind":"signal","instrument":"1","parameter":"1"},"display_label":"Temperature","visible":true,"style":{"color":"#ffffff","width":1.0},"display_unit":null}}
```

Applies exactly `UiCommand::AddTrace { plot_id, trace }` and returns the common result.

#### `ui_remove_trace`

```json
{"expected":{"workbench_id":"0123456789abcdef0123456789abcdef","revision":"7"},"plot_id":"plot-1","trace_id":"trace-1"}
```

Applies exactly `UiCommand::RemoveTrace { plot_id, trace_id }` and returns the common
result.

#### `ui_set_trace_visibility`

```json
{"expected":{"workbench_id":"0123456789abcdef0123456789abcdef","revision":"7"},"plot_id":"plot-1","trace_id":"trace-1","visible":false}
```

Applies exactly `UiCommand::SetTraceVisibility` with the three fields shown and
returns the common result.

#### `ui_set_time_window`

```json
{"expected":{"workbench_id":"0123456789abcdef0123456789abcdef","revision":"7"},"plot_id":"plot-1","seconds":120.0}
```

Applies exactly `UiCommand::SetTimeWindow { plot_id, seconds }` and returns the common
result. `seconds` must satisfy existing complete-document finite/range validation.

#### `ui_rename_item`

```json
{"expected":{"workbench_id":"0123456789abcdef0123456789abcdef","revision":"7"},"item_id":"plot-1","label":"Reactor temperature"}
```

Applies exactly `UiCommand::RenamePresentationItem { item_id, label }` and returns
the common result. Existing `UiCommand` target resolution defines which presentation
item kinds may be renamed.

#### `ui_set_trace_source`

```json
{"expected":{"workbench_id":"0123456789abcdef0123456789abcdef","revision":"7"},"plot_id":"plot-1","trace_id":"trace-1","source":{"kind":"reference","reference":"1"}}
```

Applies exactly `UiCommand::SetTraceSource { plot_id, trace_id, source }` and returns
the common result.

## 5. Workbench wire contract

### 5.1 Framing and lexical bounds

The first external adapter uses UTF-8 JSON objects terminated by LF. CRLF is accepted
as the same delimiter. It has no WebSocket or HTTP mode.

| Item | Frozen bound |
| --- | ---: |
| JSON body | 2,097,152 bytes |
| Complete NDJSON frame | 2,097,153 bytes including LF |
| JSON nesting depth | 16 |
| JSON values/object members | 16,384 |
| One UTF-8 string or key | 512 bytes |
| `call_id`, operation, event or error name | 64 bytes |
| Error message | 256 bytes |
| One active presentation file/document | existing 1,048,576-byte bound |

Duplicate keys, trailing data, non-object roots, unknown envelope fields, invalid
UTF-8, nonfinite numbers, or values over a bound are rejected. UI argument objects
reuse the existing strict serde and complete-document validation. Runtime `args` are
carried as JSON but still must fit the smaller Runtime Application limit when the
worker encodes them.

The larger Workbench frame exists only so one bounded presentation document plus its
envelope can be returned without inventing a second paged presentation schema. It
does not increase Runtime's 16,383-byte Application body limit.

### 5.2 Request envelope

After transport connection, the first valid request must be `hello`:

```json
{
  "v": 1,
  "type": "request",
  "call_id": "caller-1",
  "op": "hello",
  "args": {}
}
```

These are the only request-envelope keys. `v` is the JSON integer `1`. `type` is
exactly `request`. `call_id` is a nonempty connection-local string. `op` is one name
from the Workbench registry. `args` is an object.

One `call_id` may belong to only one wire exchange on a connection. It remains
reserved while any terminal result, terminal error, or final `lab_update` for that
exchange is queued, staged as the current write, partially written, or otherwise not
yet fully written to that connection. It becomes reusable only after successful
completion of the terminal frame write, or when the connection detaches. This rule
applies equally to local queries, UI operations, and lab calls. The initial
`state:"submitted"` result for a lab call is nonterminal for `call_id` lifetime.

A duplicate received while the old terminal frame is queued/current-write remains
`duplicate_call_id`; no connection can contain two live wire exchanges with the same
`call_id`.

`call_id` is correlation, not idempotency. It is not retained across disconnect,
does not survive Workbench restart, and never becomes a Runtime `msg_id` or
`request_id`.

### 5.3 Result envelope

Local queries and UI commands return one terminal result:

```json
{
  "v": 1,
  "type": "result",
  "call_id": "caller-1",
  "result": {}
}
```

The normative operation-specific results are in section 4.3. For example, successful
`hello` returns the full shape defined there; abbreviated here:

```json
{
  "protocol": {"id": "lab-runtime.workbench", "version": 1},
  "workbench_id": "0123456789abcdef0123456789abcdef",
  "operations": ["hello", "client_status"],
  "limits": {},
  "runtime_client": {},
  "recovery": {},
  "presentation": {}
}
```

The abbreviated arrays/objects above show shape, not a reduced registry. The real
result includes the complete Workbench operation list and the exact advertised
Workbench bounds. `runtime_client` is an observation and may be disconnected, stale,
rebuilding, or ready independently of Workbench API availability.

Successful lab submission returns only local admission:

```json
{
  "state": "submitted",
  "command_id": "41"
}
```

`command_id` is allocated by the existing checked Workbench client handle, encoded
as a decimal string, scoped to this Workbench process, and never caller-selected.
`submitted` proves only that the command entered the existing bounded worker mailbox.
It does not prove that bytes were sent, Runtime admitted an operation, the operation
completed, or a physical effect occurred.

Successful UI mutation returns:

```json
{
  "workbench_id": "0123456789abcdef0123456789abcdef",
  "presentation_revision": "8"
}
```

`presentation_get` returns the same identity/revision plus the complete current
`PresentationDocument`. `recovery_get` follows the exact expected-generation rules
in section 4.3 and returns one record, end, or deterministic restart result. This
keeps enumeration consistent without retaining a snapshot or another recovery store.

### 5.4 Error envelope

Workbench-local rejection uses:

```json
{
  "v": 1,
  "type": "error",
  "call_id": "caller-1",
  "error": {
    "domain": "workbench",
    "code": "busy",
    "message": "Workbench admission is full",
    "retryable": true,
    "resync_required": false
  }
}
```

`call_id` is `null` only when no valid bounded correlation could be recovered. The
frozen Workbench-local codes are:

| Code | Meaning |
| --- | --- |
| `invalid_shape` | Envelope/root/field type or unknown-field failure. |
| `invalid_args` | Operation arguments or bounded value are invalid. |
| `invalid_utf8` / `invalid_json` | Transport body is not valid bounded JSON. |
| `frame_too_large` | Workbench frame/body bound exceeded. |
| `version_mismatch` | Workbench protocol version is unsupported. |
| `hello_required` / `already_hello` | Connection hello gate failed. |
| `duplicate_call_id` | The same connection already has that call outstanding. |
| `unsupported_operation` | Name is not in the Workbench registry. |
| `busy` | A frozen Workbench queue/count/byte capacity rejected admission. |
| `client_not_ready` | A lab command cannot enter the Runtime client in its current state. |
| `recovery_unavailable` | Status identity is absent, quarantined, wrong-session, or durable recovery is unusable. |
| `revision_conflict` | Workbench identity or presentation revision is not current. |
| `unknown_item` | Presentation target does not exist. |
| `invalid_presentation` | Complete candidate validation failed. |
| `worker_stopped` | The one Runtime client worker is stopping/stopped. |
| `response_too_large` | A bounded local response cannot be encoded. |
| `internal_error` | Non-public Workbench failure, without leaking OS/path/internal text. |

Runtime public errors are not translated into this table. A lab update carries the
exact bounded Runtime envelope under `runtime`; callers inspect its existing
`code`, `category`, `retryable`, and `resync_required` fields.

### 5.5 Event envelope

Workbench events use:

```json
{
  "v": 1,
  "type": "event",
  "workbench_id": "0123456789abcdef0123456789abcdef",
  "event": "client_state",
  "data": {}
}
```

The fixed event names are:

| Event | Delivery and data |
| --- | --- |
| `lab_update` | Originating caller only. Contains `call_id`, decimal `command_id`, requested `op`, `kind`, and optional exact `runtime` envelope. |
| `client_state` | All hello-complete callers. Contains Workbench client connection state and model freshness. |
| `presentation_changed` | All hello-complete callers. Contains new presentation revision and originating Workbench operation name, but no full document. |
| `recovery_changed` | All hello-complete callers. Contains active/quarantined counts and a checked process-local change generation, but no large payload. |
| `client_notice` | All hello-complete callers. Contains one bounded kind: `resnapshot_required`, `reconciliation_required`, `recovery_journal_problem`, or `transport_failure`, plus bounded non-authoritative detail. |

`lab_update.kind` maps exactly to existing worker output:

```text
result
public_error
mutation_accepted
mutation_completed
mutation_failed
local_rejected
```

For every kind except `local_rejected`, `runtime` is the exact Runtime envelope
received by the worker. `mutation_accepted` is nonterminal. The other mutation kinds
are terminal for that worker command. `public_error` is terminal for the exchange,
but its meaning comes from the enclosed Runtime error. `local_rejected` proves only
the bounded local reason supplied by the existing worker.

Workbench events have no replay cursor, retained ring, filter, or subscription
operation. They are current-process client notifications. A caller that loses them
uses `client_status`, `presentation_get`, and `recovery_get`; it never asks Workbench
to reconstruct Runtime event history. Direct Runtime clients use Runtime's existing
subscription/replay contract.

## 6. Correlation and mutation identity

The three identity domains must remain separate:

| Identity | Allocator | Scope | Authority |
| --- | --- | --- | --- |
| Workbench `call_id` | external caller | one Workbench API connection | wire correlation only; no dedup/recovery authority |
| Workbench `command_id` | existing `ClientHandle` | one Workbench process | correlates one command with worker updates |
| Runtime `{scope, seq}` | existing Workbench worker using Runtime hello state | one Runtime boot/scope | authoritative mutation admission/dedup/recovery identity |

Equal numeric text in two domains has no relationship. The Workbench endpoint must
never derive Runtime sequence from `call_id`, expose a caller-selected Runtime
sequence for new work, or mark a mutation accepted before the Runtime envelope says
so.

The worker durably retains exact mutation operation/arguments and identity under the
accepted recovery contract before possible ambiguous emission. The endpoint neither
adds another journal nor retains a parallel dedup table.

If a caller loses its Workbench connection after local submission, its `call_id`
association is lost. Any worker-admitted mutation continues and remains governed by
the existing Workbench recovery projection. The caller must not resubmit a new
`lab_mutation` and infer safety. An automation client requiring durable caller-owned
correlation should use the direct Runtime Application API and own its Runtime scope.

## 7. Concurrency, admission, and bounds

One Workbench model/dispatcher owner serializes all GUI, external, and future-adapter
commands. Only the existing client worker may allocate worker command IDs, Runtime
`msg_id`, or mutation `{scope, seq}`. No external connection owns the Runtime socket
or aggregate subscription.

Frozen Workbench endpoint bounds are:

| Resource | Capacity/behavior |
| --- | --- |
| External callers | 8 total; accepted and detach-pending connections share the same pool. |
| Per-caller admitted input | 4 complete requests and 4 MiB aggregate encoded bytes. |
| Network-to-owner mailbox | 32 messages and 16 MiB aggregate encoded bytes. |
| Owner-to-network mailbox | 32 messages and 16 MiB aggregate encoded bytes. |
| Per-caller output | 4 messages and 4 MiB aggregate encoded bytes, including current write. |
| Reserved `call_id` values | 8 per caller, 32 total; reservation includes a queued/current/partial terminal frame. |
| Outstanding lab calls | 8 per caller, 32 total external correlations. |
| Existing worker command mailbox | unchanged 32; nonblocking `Busy` on pressure. |
| Existing worker in-flight exchanges | unchanged 8 total. |
| Existing worker update mailbox | unchanged 64. |
| Existing recovery records | unchanged 8 active and 8 quarantined projections. |
| External socket work | at most 8 KiB read and 8 KiB write per caller/turn; at most four complete messages/turn. |
| Partial input / blocked output / hello | absolute 2-second monotonic deadline. |

Both message count and byte credit must be available before admission. Credit remains
charged while a message is queued or being written. A `call_id` consumes reserved-ID
credit until its terminal frame write completes or its connection detaches. There is
no unbounded task, future, per-caller history, or hidden retry queue.

Fair owner dispatch considers at most four caller connections per owner turn and at
most one request from each considered caller. Existing GUI/model update work and the
Runtime client worker must continue making progress under external pressure.

Overload returns `busy` when a bounded reply can still be delivered. If input,
output, mailbox, byte-credit, deadline, or protocol failure makes safe delivery
impossible, only that external caller is detached. Detaching a caller cannot stop or
reconnect the Workbench Runtime client, roll back presentation, cancel an admitted
Runtime mutation, or affect another caller.

There is no cancellation operation in v1. A command rejected before the existing
worker accepts it has not entered the worker. After `lab_mutation` enters the worker,
caller disconnect, timeout, or death cannot prove non-admission and cannot cancel or
retry it. Ordinary query results may be discarded when their originating caller no
longer exists; queries perform no hidden physical polling or mutation.

## 8. Disconnect and lifecycle semantics

The following boundaries are distinct:

| Event | Required behavior |
| --- | --- |
| External caller disconnects from Workbench | Remove only that Workbench API connection/correlation. Keep Workbench, presentation, Runtime connection, controllers, Recorder, experiment and admitted work running. |
| Workbench explicitly disconnects from Runtime | Existing explicit-disconnect fence applies; no automatic reconnect. All callers observe client state stale/disconnected. Presentation remains available. External v1 callers cannot initiate this action. |
| Workbench unexpectedly loses Runtime continuity | Existing one retained-scope reattach episode and absolute three-second deadline apply. No query, mutation, status, or Exact Retry is automatically replayed. |
| Runtime restarts | Old boot/scope recovery becomes quarantined; presentation remains client-owned; Fresh requires the complete accepted rebuild barrier. |
| Workbench clean shutdown | Stop external admission, attempt bounded terminal delivery for 200 ms, close endpoint, then use the existing finite three-second worker shutdown. Never send Runtime shutdown implicitly. |
| Workbench crash/OS termination | OS closes caller and Runtime sockets. Runtime experiment, controllers and Recorder continue. Workbench journal/presentation retain only their already-durable client evidence. |
| External caller dies during UI command | Either the serialized command was not admitted, or it atomically changed the active document once. Caller death does not roll it back. Revision resolves later observation. |
| External caller dies during lab mutation | Existing worker/recovery evidence governs. Caller correlation is not authoritative and is not reconstructed. |

Freshness, socket connectivity, hello success, operation completion, and physical
state remain distinct. A Workbench `client_state` event cannot upgrade stale cached
data to Fresh; only the existing complete authoritative rebuild barrier can do so.

## 9. Language neutrality and future adapters

The contract consists only of bounded JSON-compatible values, stable operation/event
names, decimal-string counters, and existing `PresentationDocument` DTOs. It contains
no Clojure keyword, EDN, JavaScript callback, Rust enum representation, Steel value,
GUI widget, async-runtime handle, thread identity, or language-owned exception.

- Clojure/Babashka procedure clients normally use the direct Runtime TCP/NDJSON
  surface. They use the Workbench surface only when they intend to inspect or change
  that Workbench's client/presentation state.
- Browser clients use the existing Runtime WebSocket/JSON endpoint for laboratory
  semantics. M17 does not add browser access to Workbench.
- A later Steel host may translate Steel values to the same typed dispatcher calls.
  It receives no direct `ClientHandle`, socket, Runtime sequence, recovery journal,
  model mutability, filesystem, or output authority.
- Other language adapters may frame the same protocol or invoke the same dispatcher,
  but cannot change admission, identity, ownership, or result meaning.

The Workbench API is not a client SDK. M17 may add focused external smoke clients as
acceptance fixtures, but no language package, generated binding, embedded procedure
runtime, or convenience semantic layer is part of this milestone.

## 10. Frozen non-goals and protected subsystems

M17 must not alter:

- Runtime operations, capabilities, public errors, sessions, mutation admission,
  deduplication, subscription/replay, history, or transport parity;
- `lab-core`, controller behavior, `OutputAuthority`, physical I/O, transport
  generations, serial ownership, ACK/readback/effect separation, or safe transition;
- SimpleDevice definition/provisioning/publication, pending-apply fences, held event
  capacity, rollback, quarantine, reconnect behavior, or provenance;
- Recorder ownership, bounded ingress, SQLite schema, durability, history cursors,
  required-mode failure, or archive sealing;
- accepted Workbench recovery rules, automatic reattach bound, explicit Disconnect,
  manual status/Exact Retry behavior, or Fresh barrier;
- GUI features, layout behavior, plot rendering, operator controls, or presentation
  file format merely to expose the boundary.

Implementation must not refactor protected code for cleanliness. A demonstrated
contract need must precede any focused change outside Workbench boundary modules.

## 11. Acceptance matrix for M17.2-M17.5

### M17.2 — single-owner Workbench dispatcher

| Acceptance | Required evidence |
| --- | --- |
| One lab owner | GUI and test callers enter one dispatcher and one existing `ClientHandle`; source/test proof shows no second socket, sequencer, journal or recovery engine. |
| Operation contract | Strict decoders/results cover exactly the 15 frozen operations, reject Workbench-owned unknown fields, pass Runtime op/args unchanged, and map all eight `UiCommand` variants to their existing DTOs. |
| Identity separation | Equal-looking `call_id`, `command_id`, and Runtime `seq` remain distinct; callers cannot choose mutation identity. |
| Lab mapping | Query, mutation, and manual status map to existing worker methods; worker updates map to frozen `lab_update` kinds without changing Runtime envelopes. |
| Presentation serialization | All UI commands use one owner; expected identity/revision is checked first, then successful clone/apply/complete validation/atomic replace advances revision once. Conflict/invalid candidates leave state unchanged. |
| Local status/recovery | Status is explicitly observational; atomic recovery changes advance one checked process-local generation; first/subsequent/restart enumeration is deterministic and never triggers status or retry. |
| Regression | Existing Workbench model, recovery, GUI, real-Runtime and M16 acceptance behavior remains unchanged. |

M17.2 adds no external listener unless separately authorized after M17.1 acceptance.

### M17.3 — bounded external Workbench adapter

| Acceptance | Required evidence |
| --- | --- |
| Exact wire contract | Strict hello/request/result/error/event and all 15 operation args/results pass unknown-field, type, decimal-counter, and boundary tests. |
| Shared capacity | Eight callers, count/byte credits, detach-pending accounting, fair dispatch, and one bounded set of mailboxes are demonstrated. |
| Slow/malformed isolation | Oversize, duplicate key, partial frame, blocked writer, duplicate `call_id`, and malformed client detach only that caller. A queued/current/partial terminal frame keeps its `call_id` reserved until full write or detach. |
| Backpressure | Every count and byte bound reaches deterministic overload behavior with no unbounded allocation or required-owner starvation. |
| Recovery snapshot fence | Mutation of either recovery collection between indexed calls yields `state:"restart"`; stable generations enumerate each record exactly once without retained snapshots/tokens. |
| Lifecycle | Caller disconnect, clean Workbench close, Workbench kill, and Runtime disconnect/restart produce the frozen independent lifetimes. |
| Local-only policy | Numeric IPv4 loopback bind is proven; no remote/WebSocket/authentication claim is introduced. |

### M17.4 — ownership, recovery, and parity acceptance

| Acceptance | Required evidence |
| --- | --- |
| Multi-caller lab work | Concurrent callers share one worker; mutation sequence remains consecutive and Runtime-owned; pressure cannot create a second lane. |
| Lost caller | Mutation before/after possible wire emission remains honestly pending/ambiguous/accepted according to existing recovery evidence and is never auto-replayed. |
| Runtime transport parity | Existing TCP/WebSocket parity, cross-transport scope reattachment, dedup, subscription, history, faults and shared eight-client Runtime pool remain green. |
| Presentation isolation | Every UI operation changes only client-owned presentation; Runtime operation/event/Recorder state is byte-for-byte unaffected by UI-only scenarios. |
| M16 and safety regression | SimpleDevice generic paths, publication fences, reconnect quarantine, controllers, output ambiguity and Recorder/provenance acceptances remain green. |
| Process survival | External caller death and Workbench death leave the Runtime experiment running; Runtime death leaves Workbench presentation accessible but stale. |

### M17.5 — language-boundary smoke and consolidated review

| Acceptance | Required evidence |
| --- | --- |
| Babashka/Clojure direct client | A minimal external process performs hello/query/mutation/status or an approved safe subset over Runtime TCP/NDJSON without a client SDK. |
| Browser regression | Real-browser ClojureScript or equivalent continues to use the existing Runtime WebSocket endpoint; no Workbench proxy is involved. |
| Workbench language-neutral smoke | A minimal external process exercises Workbench hello/status, one mediated safe lab query, presentation read, revision conflict, one UI-only mutation, events and disconnect. |
| Documentation | Both surfaces, ownership choice, recovery limits, local-only security and exact bounds are documented without presenting client observations as authority. |
| Consolidated review | Source, dependencies, bounds, failure isolation, frozen regressions and external evidence receive explicit acceptance before M17 closes. |

Steel implementation, GUI features, a reusable client SDK, real Arduino procedure
work, remote networking, and release publication are outside this matrix.

## 12. Review gates and unresolved decisions

The following decisions are frozen by this proposal and require external review
before M17.2:

1. two surfaces rather than a combined Runtime/Workbench semantic API;
2. direct Runtime clients own their own scope/recovery, while mediated Workbench lab
   commands share the existing worker scope/recovery;
3. one process-local Workbench API with no resumable Workbench session or dedup store;
4. `call_id`, `command_id`, and `{scope, seq}` as separate identities, with
   `call_id` reserved through successful terminal-frame write;
5. exact arguments/results for all 15 Workbench operations and generation-fenced
   recovery enumeration without retained paging state;
6. no Workbench Runtime-event subscription/relay and no Workbench Exact Retry;
7. transactional presentation revision and fixed v1 UI operation set;
8. the exact queue/count/byte/deadline bounds above;
9. M17.2 dispatcher, M17.3 adapter, M17.4 acceptance, M17.5 language smoke/review.

Nonblocking implementation choices intentionally left to the authorized slice are:

- exact Rust module/file names and private DTO representation;
- the opt-in command-line/configuration spelling and readiness-line field used to
  publish the Workbench endpoint address in M17.3;
- internal coalescing of duplicate observational `client_state` or
  `recovery_changed` events, provided count/byte bounds and event meaning remain
  exact;
- exact human-readable Workbench error messages; codes and fields above are stable.

Deferred product decisions, not M17 blockers, are Workbench WebSocket access,
remote security, automatic presentation persistence, richer model projections,
client SDKs, and Steel dependency/runtime selection.

## 13. M17.1 verdict

The current architecture supports this contract without modifying Runtime semantics
or protected M16/safety subsystems. No pre-M17 cleanup is required.

The initial external review found the architecture sound but did not accept the
contract until exact operation shapes, recovery enumeration consistency, and
terminal-write correlation lifetime were remediated. External re-review accepted the
remediated contract with no remaining blockers.

```text
M17.1: ACCEPTED
M17.2: ACCEPTED
M17.3: ACCEPTED
M17.4: ACCEPTED
Consolidated M17.1-M17.4 review: ACCEPTED (B1-B5 remediated and independently closed)
M17.5: AUTHORIZED / READY TO START / NOT IMPLEMENTED after the M17.4 acceptance commit
```
