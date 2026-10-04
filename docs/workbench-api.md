# Workbench external API

Applicable to Workbench protocol v1 and the current repository/v0.1 product.

Workbench's optional API exposes presentation and client-state semantics. It is
separate from the [Runtime Application API](api/README.md), which owns authoritative
experiment semantics.

```text
direct automation
    client -> Runtime API -> Runtime

presentation-aware automation
    client -> Workbench API -> dispatcher
           -> existing Runtime client -> Runtime
```

A caller that needs no Workbench presentation or recovery projection should normally
connect directly to Runtime. Workbench is not a second Runtime, a transparent raw
proxy, a second mutation sequencer, a second recovery journal, or a Runtime event
relay.

## Start and trust boundary

The endpoint is disabled by default. Enable it explicitly:

```powershell
cargo run -p lab-workbench --locked -- --connect 127.0.0.1:7420 `
  --workspace .workbench-demo --workbench-listen 127.0.0.1:7421
```

From an extracted preview package:

```powershell
./lab-workbench.exe --connect 127.0.0.1:7420 `
  --workspace .workbench-demo --workbench-listen 127.0.0.1:7421
```

`--workbench-listen` accepts a numeric IPv4 loopback address only. Hostnames,
wildcards, non-loopback addresses, and IPv6 are rejected before binding. Port `0`
selects an ephemeral port; stdout publishes the actual address as
`{"workbench_endpoint":"127.0.0.1:<port>"}`.

The endpoint is unauthenticated local TCP/NDJSON. There is no TLS, HTTP, WebSocket,
remote-listener, or authentication mode. Any local process able to connect is
trusted; do not port-forward it. The endpoint belongs to the Workbench process and
its external callers consume no additional Runtime Application client slots. The
single private Workbench Runtime client still occupies its existing one slot.

## Wire format and hello gate

Each request is one strict UTF-8 JSON object terminated by LF:

```json
{"v":1,"type":"request","call_id":"hello-1","op":"hello","args":{}}
```

CRLF is accepted equivalently. TCP read boundaries are not frame boundaries. The
physical-frame maximum includes the LF and, when present, the CR. Duplicate object
keys, unknown envelope fields, invalid UTF-8, non-object roots, trailing JSON data,
nonfinite numbers, and over-bound JSON are rejected.

The first valid request on a connection must be `hello`. Any earlier operation gets
`hello_required`; another hello after success gets `already_hello`. All counters and
revisions shown as strings below use canonical unsigned decimal spelling: `"0"` or a
nonzero digit followed by digits, with no sign or leading zero.

Request fields are exact:

| Field | Rule |
|---|---|
| `v` | integer `1` |
| `type` | string `"request"` |
| `call_id` | caller-selected, 1..64 UTF-8 bytes |
| `op` | one of the 15 names advertised by hello |
| `args` | strict object for that operation |

Result:

```json
{"v":1,"type":"result","call_id":"status-1","result":{}}
```

Error:

```json
{"v":1,"type":"error","call_id":"status-1","error":{"domain":"workbench","code":"client_not_ready","message":"Runtime client is not ready","retryable":false,"resync_required":false}}
```

An error detected before a usable bounded `call_id` can have `call_id:null` and the
caller is detached. Events are not result envelopes:

```json
{"v":1,"type":"event","workbench_id":"<id>","event":"client_state","data":{"connection":"ready","freshness":"fresh"}}
```

## Correlation and identity domains

Do not call every identifier a request ID.

| Identity | Owner | Lifetime | Meaning |
|---|---|---|---|
| `call_id` | external caller | one live exchange on one TCP connection | Workbench wire correlation; not idempotency |
| `command_id` | Workbench | one process-local worker command | correlates dispatcher work with its existing Runtime client |
| Runtime `{scope,seq}` | Runtime/Workbench worker | bounded Runtime scope and outcome retention | authoritative Runtime mutation identity |
| `workbench_id` | Workbench | one Workbench process | fences presentation and recovery continuity |
| presentation `revision` | Workbench | current presentation in this process/workspace | optimistic UI concurrency |
| `recovery_generation` | Workbench | current process-local recovery projection | fences indexed recovery reads/status targets |
| Runtime `boot_id` | Runtime | one Runtime process | fences scopes, events, and recovery across restart |

Local and UI exchanges reserve `call_id` until their terminal result/error and all
related duplicate-response frames are completely written. Lab exchanges remain
reserved after the initial `submitted` result and after `mutation_accepted`; they end
only after the terminal `lab_update` and every related write complete. Queued,
current, and partially written frames still own the reservation. A duplicate live ID
gets `duplicate_call_id` when a bounded reply fits. Detach releases only that
connection's reservations.

## Operations

The source-owned v1 registry contains exactly 15 operations. The `Runtime?` column
states whether normal success submits work to the one existing Workbench Runtime
client; `Presentation?` states whether it can replace the active document.

| Operation | Exact top-level args | Success | Runtime? | Presentation? |
|---|---|---|---:|---:|
| `hello` | none | protocol, identity, inventory, limits, status | no | no |
| `client_status` | none | current Runtime-client/recovery/presentation summary | no | no |
| `recovery_get` | `kind`, `index`, required `expected` | one active/quarantined record, end, or restart | no | no |
| `lab_query` | `op`, `args` | process-local `submitted` correlation | yes | no |
| `lab_mutation` | `op`, `args` | process-local `submitted` correlation | yes | no |
| `lab_operation_status` | `target` | process-local `submitted` correlation | yes | no |
| `presentation_get` | none | current document, identity, revision | no | no |
| `ui_add_plot` | `expected`, `plot` | new revision | no | yes |
| `ui_remove_plot` | `expected`, `plot_id` | new revision | no | yes |
| `ui_add_trace` | `expected`, `plot_id`, `trace` | new revision | no | yes |
| `ui_remove_trace` | `expected`, `plot_id`, `trace_id` | new revision | no | yes |
| `ui_set_trace_visibility` | `expected`, `plot_id`, `trace_id`, `visible` | new revision | no | yes |
| `ui_set_time_window` | `expected`, `plot_id`, `seconds` | new revision | no | yes |
| `ui_rename_item` | `expected`, `item_id`, `label` | new revision | no | yes |
| `ui_set_trace_source` | `expected`, `plot_id`, `trace_id`, `source` | new revision | no | yes |

Unknown argument fields are rejected for every operation.

## Hello

`hello` takes `{}`. Its result has the following top-level field structure; array,
limit, nested Runtime-hello, and status values are live:

```json
{
  "protocol":{"id":"lab-runtime.workbench","version":1},
  "workbench_id":"<32 lowercase hex characters>",
  "operations":["hello","client_status","recovery_get","lab_query","lab_mutation","lab_operation_status","presentation_get","ui_add_plot","ui_remove_plot","ui_add_trace","ui_remove_trace","ui_set_trace_visibility","ui_set_time_window","ui_rename_item","ui_set_trace_source"],
  "limits":{"<source-advertised fields>":"<numbers>"},
  "runtime_client":{"connection":"<state>","freshness":"<state>","hello":null},
  "recovery":{"recovery_generation":"1","active_count":"0","quarantined_count":"0","reconciliation_required_count":"0"},
  "presentation":{"presentation_revision":"1"}
}
```

When the internal Runtime hello has completed, `runtime_client.hello` contains its
`boot_id`, `scope`, `next_seq`, advertised operations/capabilities/limits, and event
cursors. Those values describe the existing Workbench client; they do not give the
external caller control of its scope or sequence. A Workbench restart produces a new
`workbench_id` and a new wire-correlation boundary.

## Client status

`client_status` takes `{}` and returns the same `workbench_id`, `runtime_client`,
`recovery`, and `presentation` fields as hello. Connection values are
`disconnected`, `connecting`, `awaiting_hello`, `reattaching`, `ready`, `stale`,
`stopping`, or `stopped`. Freshness is independently `unknown`, `rebuilding`,
`fresh`, or `stale`.

Endpoint reachability is not Runtime readiness. `ready` means the private Runtime
client completed hello; `fresh` additionally means the authoritative rebuild and
catch-up barrier completed. Lab submission requires a ready Runtime client, while
client-local presentation reads and edits remain available without Runtime.

## Recovery get

The first read must explicitly include nullable `expected` and begin at index zero:

```json
{"kind":"active","index":"0","expected":null}
```

Omitting `expected` is `invalid_args`. Later reads use:

```json
{"kind":"active","index":"1","expected":{"workbench_id":"<id>","recovery_generation":"2"}}
```

`kind` is `active` or `quarantined`. A result contains `state` (`record`, `end`, or
`restart`), `workbench_id`, `recovery_generation`, counts, echoed kind/index,
`record`, and nullable `next_index`. An active record is:

```json
{"boot_id":"<boot>","request_id":{"scope":"<scope>","seq":"1"},"op":"reference_retune","args":{},"admission":"accepted"}
```

Admission is `pending`, `accepted`, `ambiguous`, `completed`, or `failed`. A
quarantined item is `{"record":<record>,"reason":"<reason>"}`; reasons are
`instance_changed`, `scope_unknown`, `attached_boot_mismatch`, and
`attached_scope_mismatch`.

An expected identity/generation mismatch returns `state:"restart"` with the current
identity and counts instead of mixing indexed generations. There is no retained
projection history. Generation starts at 1 and advances only when a valid accepted
worker recovery projection differs from the current one. It is unrelated to Runtime
mutation sequence. A Workbench restart is a new continuity boundary.

## Mediated laboratory operations

```text
external call
    |
    v
Workbench dispatcher
    |
    v
existing ClientHandle and worker
    |
    v
Runtime
```

There is exactly one Workbench Runtime socket/session/sequencer/subscription/recovery
path. The adapter never accesses `ClientHandle` directly.

### `lab_query`

Arguments are `{"op":"<Runtime query>","args":{...}}`. A valid ordinary Runtime
query and its args enter the existing worker unchanged. Workbench does not maintain a
second Runtime registry, but it reserves worker/session-control queries:
`operation_status`, `subscribe`, and `unsubscribe`. They are rejected before worker
submission; status uses `lab_operation_status`, and external Workbench callers have
no Runtime subscription operation.

The initial result is nonterminal:

```json
{"state":"submitted","command_id":"17"}
```

The originating caller later receives a terminal `lab_update` with `kind:"result"`
or `kind:"public_error"`. `data.runtime` is the unchanged bounded Runtime envelope.

### `lab_mutation`

Arguments are `{"op":"<Runtime mutation>","args":{...}}`. The caller must not put
`request_id` in args and never selects Runtime `scope` or `seq`; the existing worker
does so. Explicit `runtime_shutdown` remains a normal mediated Runtime mutation.
Closing Workbench or a caller never synthesizes shutdown.

The initial `submitted` result is not Runtime admission. A later
`kind:"mutation_accepted"` update carries authoritative Runtime acceptance and is
still nonterminal. `mutation_completed`, `mutation_failed`, `public_error`, or
`local_rejected` is terminal for the Workbench call. `local_rejected` originates only
from the worker's local-rejection output; continuity uncertainty never synthesizes
it. Caller death removes correlation but does not cancel admitted Runtime work.

### `lab_operation_status`

This is the only external Workbench status route. Its exact args are:

```json
{"target":{"workbench_id":"<id>","recovery_generation":"2","boot_id":"<boot>","request_id":{"scope":"<scope>","seq":"7"}}}
```

The target must match the current Workbench identity, current recovery generation,
attached Runtime boot and scope, and one exact active recovery record. Quarantined,
old-generation, old-boot, wrong-scope, or absent records get
`recovery_unavailable`. Success follows the same `submitted` plus terminal
`lab_update` flow as `lab_query`. Workbench does not automatically poll status.

## Presentation snapshot

`presentation_get` takes `{}` and returns:

```json
{"workbench_id":"<id>","presentation_revision":"1","document":{"format_version":1,"document_id":"main","windows":[],"plots":[],"controls":[]}}
```

The complete renderer-neutral `PresentationDocument` is current Workbench state, not
Runtime state. Its canonical pretty-JSON representation is limited to 1,048,576
bytes. The structural limits include 64 windows, 64 total panels, 64 controls, 64
tabs/window, 32 plots, 32 traces/plot, and 512 UTF-8 bytes per nonempty string.
The snapshot contains definitions and Runtime references, not live sample history.

Windows are `{"id":...,"title":...,"tabs":[...]}`; tabs are
`{"id":...,"title":...,"panels":[...]}`. A panel has `id`, `title`, and a tagged
`kind`: `{"kind":"plot","plot_id":...}`,
`{"kind":"controls","control_ids":[...]}`, or
`{"kind":"status","source":<RuntimeRef>}`. A control has `id`, `label`, typed
`target`, `kind` (`set_reference`, `controller_lifecycle`, `recording_lifecycle`,
`apply_configuration`, or `reconnect_resource`), and boolean `confirm`. IDs are
unique document-wide and panel references must resolve inside the document.

## UI mutation contract

Every UI mutation carries:

```json
{"expected":{"workbench_id":"<id>","revision":"1"}}
```

The dispatcher performs one atomic transition:

```text
check expectation
    -> clone document
    -> apply edit
    -> structural and size validation
    -> atomic replace
    -> revision + 1
    -> presentation_changed
```

On any error, the document and revision are unchanged and no
`presentation_changed` event is emitted. A success returns only `workbench_id` and
the new `presentation_revision`. UI operations never contact Runtime or create a
Runtime operation.

A plot has exact fields:

```json
{"id":"plot-1","title":"Temperature","time_window_seconds":60.0,"axes":{"y_min":null,"y_max":null},"traces":[]}
```

Time windows are finite, greater than zero, and at most 604,800 seconds. If both axis
bounds exist, `y_min < y_max`. A trace has exact fields:

```json
{"id":"trace-1","source":{"kind":"signal","instrument":"1","parameter":"2"},"display_label":"T","visible":true,"style":{"color":"#00aaff","width":1.0},"display_unit":null}
```

Width is finite, greater than zero, and at most 32. Runtime reference kinds are
`instrument`, `signal`, `reference`, `controller`, `resource`, `component`,
`recorder`, and `configuration_property`; each kind accepts only its typed identity
fields. A configuration-property reference is, for example,
`{"kind":"configuration_property","owner":{"kind":"instrument","instrument":"1"},"property":"gain"}`;
owner kind may instead be `component` or `resource` with its matching ID field.

The eight edits are:

- `ui_add_plot`: append the supplied complete `plot`.
- `ui_remove_plot`: remove `plot_id`; a still-referenced plot makes the candidate
  invalid rather than leaving a dangling panel.
- `ui_add_trace`: append complete `trace` to `plot_id`.
- `ui_remove_trace`: remove `trace_id` from `plot_id`.
- `ui_set_trace_visibility`: set boolean `visible` for the named trace.
- `ui_set_time_window`: set finite bounded `seconds` for the plot.
- `ui_rename_item`: set `label` on a plot, trace, control, window, tab, or panel
  selected by globally stable `item_id`.
- `ui_set_trace_source`: replace the named trace's typed Runtime `source`.

Unknown IDs get `unknown_item`; invalid/oversized candidates get
`invalid_presentation`; stale identity/revision gets `revision_conflict`.

## Events

Only `lab_update` is caller-routed. Its data is:

```json
{"call_id":"lab-1","command_id":"17","op":"reference","kind":"result","runtime":{"v":1,"msg_id":"<worker-id>","type":"result","result":{}},"local_reason":null}
```

Broadcast event kinds are exactly:

| Event | Data |
|---|---|
| `client_state` | `connection`, `freshness` |
| `presentation_changed` | `presentation_revision`, `operation` |
| `recovery_changed` | `recovery_generation`, `active_count`, `quarantined_count` |
| `client_notice` | `kind`, bounded `detail` |

Notice kinds are `resnapshot_required`, `reconciliation_required`,
`recovery_journal_problem`, and `transport_failure`. Workbench has no external
subscribe/filter operation, replay ring, or event history. A caller that misses a
broadcast recovers current state with `client_status`, `presentation_get`, and
`recovery_get`.

## Error model

Wire/protocol codes are `invalid_shape`, `invalid_args`, `invalid_utf8`,
`invalid_json`, `frame_too_large`, `version_mismatch`, `hello_required`,
`already_hello`, `duplicate_call_id`, `unsupported_operation`, and `busy`.
Dispatcher/domain codes are `client_not_ready`, `recovery_unavailable`,
`revision_conflict`, `unknown_item`, `invalid_presentation`, `worker_stopped`,
`response_too_large`, and `internal_error`.

Only `busy` sets `retryable:true`; this is not permission to replay a mutation.
Only `revision_conflict` sets `resync_required:true`. Runtime public errors remain
inside `lab_update.data.runtime` and keep Runtime's own error domain/meaning.
Malformed/oversized input, deadlines, impossible bounded replies, or blocked output
detach only the offending caller. Transport loss does not become a semantic
`local_rejected` or mutation failure.

## Bounds

Hello advertises the main endpoint bounds. Additional lexical/turn limits are fixed
by the same adapter.

| Resource | Limit |
|---|---:|
| JSON body with LF | 2,097,152 bytes |
| JSON body with CRLF | 2,097,151 bytes |
| physical frame including LF and optional CR | 2,097,153 bytes |
| JSON container depth | 16 |
| JSON values/object members | 16,384 |
| one UTF-8 string or key | 512 bytes |
| `call_id`, operation, event, error name | 64 bytes |
| error message / client notice detail | 256 bytes |
| active presentation canonical pretty JSON | 1,048,576 bytes |
| callers including detach-pending | 8 |
| per-caller admitted input | 4 messages and 4 MiB |
| per-caller output including current write | 4 messages and 4 MiB |
| each owner/network mailbox | 32 messages and 16 MiB |
| reserved call IDs | 8/caller, 32 total |
| outstanding lab correlations | 8/caller, 32 total |
| socket work/caller turn | 8 KiB read plus 8 KiB write |
| complete input frames/caller turn | 4 |
| owner work/turn | 4 callers, at most 1 request each |
| hello, partial input, blocked output deadline | absolute 2 seconds |
| recovery projection | 8 active, 8 quarantined |
| endpoint terminal-delivery attempt | at most 200 ms |

Both count and byte credit must be available. The single 8-per-caller/32-total ID
pool includes every ID that cannot yet be reused; capacity rejection cannot hide an
extra correlation. Reserved-ID or lab-correlation exhaustion detaches if a correctly
correlated reply cannot be reserved. Input/mailbox pressure after successful
reservation returns bounded `busy` when output capacity permits, otherwise it
detaches only that caller.

The Runtime Application JSON body remains limited to 16,383 bytes. Workbench's
larger frame therefore does not enlarge opaque `lab_query`/`lab_mutation` arguments
after they reach the existing Runtime client.

One fixed network thread serves all callers; there are no per-caller tasks. Owner
work remains at most four distinct callers and one request each per turn. Pending or
newly admitted mailbox work schedules another owner turn, so coalesced GUI wakes do
not strand requests and the bounded turn does not starve GUI/worker progress.

## Disconnect and shutdown

External socket death calls the dispatcher's caller-detach path and removes only
connection-local correlation. It does not disconnect Workbench from Runtime, cancel
admitted work, retry/status/resubmit it, roll back presentation, alter another
caller, or synthesize Runtime shutdown. A presentation edit is either not admitted
or committed atomically exactly once; caller death cannot roll it back.

The operator/client decision model for lost continuity and quarantined evidence is
in [Recovery and fault handling](recovery-and-faults.md#workbench-recovery).

On clean Workbench shutdown, the endpoint first stops accepting callers, attempts
bounded terminal delivery for at most 200 ms, and closes. Workbench then uses its
existing finite Runtime-client shutdown. Neither path sends `runtime_shutdown`.

## End-to-end Workbench transcript

This abbreviated flow matches the optional Babashka smoke. `<id>` and decimal
revisions are values returned by the running Workbench.

```json
{"v":1,"type":"request","call_id":"hello","op":"hello","args":{}}
{"v":1,"type":"result","call_id":"hello","result":{"protocol":{"id":"lab-runtime.workbench","version":1},"workbench_id":"<id>","operations":["<15 operations>"],"limits":{"<advertised>":"<bounds>"},"runtime_client":{"connection":"ready","freshness":"fresh","hello":{"<Runtime hello>":"<values>"}},"recovery":{"recovery_generation":"1","active_count":"0","quarantined_count":"0","reconciliation_required_count":"0"},"presentation":{"presentation_revision":"1"}}}
{"v":1,"type":"request","call_id":"status","op":"client_status","args":{}}
{"v":1,"type":"result","call_id":"status","result":{"workbench_id":"<id>","runtime_client":{"connection":"ready","freshness":"fresh","hello":{"<Runtime hello>":"<values>"}},"recovery":{"recovery_generation":"1","active_count":"0","quarantined_count":"0","reconciliation_required_count":"0"},"presentation":{"presentation_revision":"1"}}}
{"v":1,"type":"request","call_id":"lab","op":"lab_query","args":{"op":"reference","args":{"reference":"1"}}}
{"v":1,"type":"result","call_id":"lab","result":{"state":"submitted","command_id":"17"}}
{"v":1,"type":"event","workbench_id":"<id>","event":"lab_update","data":{"call_id":"lab","command_id":"17","op":"reference","kind":"result","runtime":{"v":1,"msg_id":"<worker-id>","type":"result","result":{"reference":"1","kind":"ramp","value":0.0,"target":1.0,"rate":0.1,"revision":"1","status":"valid","configurable":true,"last_at":"<monotonic-ns>","last_evaluated_at_ns":"<monotonic-ns>","unit":{"id":"<unit-id>","symbol":"<unit>"}}},"local_reason":null}}
{"v":1,"type":"request","call_id":"get","op":"presentation_get","args":{}}
{"v":1,"type":"result","call_id":"get","result":{"workbench_id":"<id>","presentation_revision":"1","document":{"format_version":1,"document_id":"main","windows":[],"plots":[],"controls":[]}}}
{"v":1,"type":"request","call_id":"add","op":"ui_add_plot","args":{"expected":{"workbench_id":"<id>","revision":"1"},"plot":{"id":"plot-1","title":"Example","time_window_seconds":60.0,"axes":{"y_min":null,"y_max":null},"traces":[]}}}
{"v":1,"type":"result","call_id":"add","result":{"workbench_id":"<id>","presentation_revision":"2"}}
{"v":1,"type":"event","workbench_id":"<id>","event":"presentation_changed","data":{"presentation_revision":"2","operation":"ui_add_plot"}}
{"v":1,"type":"request","call_id":"stale","op":"ui_remove_plot","args":{"expected":{"workbench_id":"<id>","revision":"1"},"plot_id":"plot-1"}}
{"v":1,"type":"error","call_id":"stale","error":{"domain":"workbench","code":"revision_conflict","message":"presentation expectation is stale","retryable":false,"resync_required":true}}
```

Read `presentation_get` again to obtain revision 2 and the committed document, then
close the socket. Disconnect requires no protocol operation and does not remove the
plot or stop Runtime.

## Deliberately absent surface

There is no arbitrary raw Runtime request operation, Runtime event relay,
Workbench-owned Runtime session store, second Runtime socket owner, external Exact
Retry, Runtime connect/disconnect shortcut, implicit shutdown shortcut,
presentation-replace/save endpoint, filesystem/serial API, script evaluation,
subscription, replay, or resumable external caller session.

The [Babashka smoke](../clients/babashka-smoke/README.md) is acceptance/example code,
not an SDK. See the [Workbench user guide](workbench.md) for GUI workflows and the
[Runtime API](api/README.md) for direct experiment automation.
