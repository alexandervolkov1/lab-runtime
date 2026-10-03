# Workbench external API

Workbench's optional API owns presentation and client-state access. It is distinct
from the [Runtime Application API](api/README.md), which owns all experiment semantics.

```text
direct client -> Runtime Application (TCP/NDJSON or WebSocket/JSON)
Workbench caller -> Workbench TCP/NDJSON -> dispatcher -> one ClientHandle -> Runtime
```

Direct clients manage their own Runtime scope/sequencing/recovery using Runtime's
negotiated state. Workbench callers never select a new mutation's Runtime `{scope,seq}`:
the existing Workbench worker owns that lane, socket, journal and aggregate subscription.
GUI and external UI commands share one serialized presentation owner. Workbench does
not acquire HostCore, serial, output, Recorder, or experiment authority.

## Start and trust boundary

```powershell
cargo run -p lab-workbench --locked -- --connect 127.0.0.1:7420 `
  --workspace .workbench-demo --workbench-listen 127.0.0.1:7421
```

The endpoint is disabled unless requested. Only numeric IPv4 loopback is accepted;
hostnames, wildcard, non-loopback and IPv6 binds are rejected. Port `0` selects an
ephemeral port. Read the actual address from stdout's `workbench_endpoint` readiness
field. There is no authentication, TLS, HTTP, WebSocket, or remote-access mode; any
local process able to connect is trusted. Do not forward this endpoint off-host.

## Wire surface

UTF-8 NDJSON uses LF; CRLF is also accepted. The first valid request must be hello:

```json
{"v":1,"type":"request","call_id":"hello-1","op":"hello","args":{}}
```

Results have `v`, `type:"result"`, `call_id`, and `result`. Workbench errors have
`v`, `type:"error"`, `call_id`, and an `error` object containing `domain`, `code`,
`message`, `retryable`, and `resync_required`. Notifications have `v`, `type:"event"`,
`workbench_id`, `event`, and `data`. Unknown fields, duplicate keys, invalid UTF-8,
non-object roots, trailing data, nonfinite numbers and over-bound input are rejected.

Exactly 15 operations exist:

- `hello`, `client_status`, `recovery_get`, `presentation_get`;
- `lab_query`, `lab_mutation`, `lab_operation_status`;
- `ui_add_plot`, `ui_remove_plot`, `ui_add_trace`, `ui_remove_trace`,
  `ui_set_trace_visibility`, `ui_set_time_window`, `ui_rename_item`, `ui_set_trace_source`.

The [frozen contract](../ai/M17_1_EXTERNAL_CLIENT_CONTRACT_OWNERSHIP_FREEZE.md)
defines exact per-operation arguments/results and source DTOs; this guide does not
create another registry. Normal Runtime operation/argument JSON is forwarded unchanged.
Worker-owned `operation_status`, `subscribe`, and `unsubscribe` cannot be ordinary
`lab_query` operations. Use the fenced `lab_operation_status` operation for status.
Explicit `runtime_shutdown` via `lab_mutation` remains a Runtime mutation; closing
Workbench or a caller never synthesizes it.

`call_id` is connection-local correlation, not mutation identity or idempotency.
Local calls end with one result/error. Lab submission returns `state:"submitted"`
and a decimal `command_id`, followed by caller-only `lab_update` events containing
the unchanged Runtime envelope. `mutation_accepted` is nonterminal. Reservation lasts
through the terminal frame **and every related duplicate-response write**, including
queued/current/partial writes. Disconnect discards correlation, never admitted work.

## Presentation and recovery

Every UI mutation requires `expected:{workbench_id,revision}`. Counters are canonical
decimal strings. Stale expectations fail before candidate application; complete
structural and canonical serialized-size validation precede atomic replacement and
one revision increment. Failure leaves the document/revision unchanged. UI-only work
creates no Runtime operation. `workbench_id` and revision are process-local.

`recovery_get` first-read args are `{"kind":"active","index":"0","expected":null}`;
`expected` must be present. Subsequent reads supply the returned Workbench identity
and `recovery_generation`. A changed identity/generation returns `restart` rather
than inconsistent indexed data. No snapshots or paging history are retained.
`lab_operation_status` requires exact Workbench/generation/boot/scope/sequence identity
of a current active recovery record; quarantined evidence is not an active target.
Check Status and GUI Exact Retry remain manual. No external Exact Retry operation,
automatic status polling, resubmission, or synthetic rejection on continuity loss exists.

Broadcasts are only `client_state`, `presentation_changed`, `recovery_changed`, and
`client_notice`. There is no event history, filter or Workbench subscribe operation.
Recover missed client notifications using status/presentation/recovery reads. Runtime
reconnect does not make observations Fresh without the existing rebuild barrier.

## Frozen bounds and lifecycle

| Resource | Limit |
|---|---|
| JSON body / physical frame | 2,097,152 / 2,097,153 bytes; physical frame includes LF and optional CR |
| JSON depth / values or object members | 16 / 16,384 |
| UTF-8 string or key / names / error message | 512 / 64 / 256 bytes |
| Active presentation, canonical pretty JSON | 1,048,576 bytes |
| External callers, including detach-pending | 8 |
| Per-caller input / output | 4 messages and 4 MiB each; output includes current write |
| Each owner/network mailbox | 32 messages and 16 MiB |
| Reserved call IDs / outstanding lab correlations | each 8 per caller, 32 total |
| Per-caller socket turn | 8 KiB read, 8 KiB write, 4 complete input frames |
| Owner turn | 4 distinct callers, at most 1 request each; pending admission schedules another turn |
| Hello / partial input / blocked output | absolute 2-second monotonic deadline |
| Existing worker command / in-flight / update queues | 32 / 8 / 64 |
| Recovery projections | 8 active, 8 quarantined |

Count and byte credits are both required. The Runtime Application body remains
16,383 bytes; Workbench's larger frame does not enlarge lab arguments. Return `busy`
only if a correctly correlated bounded reply can be delivered. Reserved-ID saturation
detaches the offending caller rather than creating an uncounted rejection ID.

One network thread serves all callers, without per-caller tasks. Malformed, slow or
dead callers are isolated. Clean shutdown stops acceptance, permits at most 200 ms
for endpoint delivery, then closes it before finite Runtime-client shutdown. Caller
death cannot roll back committed presentation or cancel admitted Runtime work.
SimpleDevice publication/reconnect fences, OutputAuthority and Recorder remain unchanged.

The [Babashka smoke](../clients/babashka-smoke/README.md) is acceptance/example code,
not an SDK. Full Arduino, Clojure and Clay integration remains post-preview work.
