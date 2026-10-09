# Recorder and SQLite archive reference

Runtime owns Recorder. Clients request lifecycle changes and bounded history reads;
they do not write the database, allocate run identities, construct provenance, or
decide that pending ingress is durable.

```text
Runtime facts -> Recorder admission -> SQLite archive
                                      -> history API -> clients
```

This page is the canonical archive and Recorder guide. For incident handling,
continuity loss, and retry decisions, use
[Recovery and fault handling](recovery-and-faults.md).

## Recorder modes and policy

Recorder configuration has one disabled state and two enabled policies:

| Mode | Startup/open | Runtime failure after attachment |
|---|---|---|
| disabled | No database is opened. `recording_status` reports `unconfigured`; Recorder-dependent operations/capabilities are unavailable. | There is no Recorder coverage. Unrelated non-Recorder Runtime behavior remains available. |
| `required` | The local archive must open and validate before listener readiness. Open failure aborts startup. | Recorder failure is sticky and invokes the central fail-closed recording-failure path: ordinary critical control loses authority and safe/fault work remains visible. |
| `best_effort` | The same open/validation requirement applies; it is not startup fallback. | Recorder becomes visibly `failed`, closes ordinary fact admission, and preserves truthful committed-prefix/coverage evidence. Unrelated valid native acquisition/control is not stopped solely by storage failure. |

Both enabled modes reject false durable acknowledgement. `best_effort` does not hide
errors, and `required` does not redefine every unrelated subsystem failure as process
exit. The policy is fixed by trusted startup composition and is not hot-reloaded.

## Database, boot, run, and operation identities

Do not call these all sessions:

| Identity | Owner and lifetime | Meaning |
|---|---|---|
| `database_id` | Recorder archive; stable for that database | 32-character lowercase hexadecimal archive identity |
| `boot_id` | Runtime process | 16-byte process identity, also exposed by Application hello |
| `(boot_id,run_no)` | Runtime Recorder lifecycle | one recorded run; `run_no` is generated within the boot |
| `(boot_id,interval_no)` | Runtime Recorder lifecycle | one coverage interval within a run |
| signal/instrument IDs | Runtime composition | stable semantic source identity within the configured topology |
| generation / mapping or configuration revision | Runtime composition lifecycle | fences evidence across reconnect/rebind/reconfiguration |
| Runtime `{scope,seq}` | Runtime SessionStore | authoritative mutation/recovery identity; independent of run number |

The human label supplied to `recording_start` is not identity. Clients copy returned
IDs exactly and use decimal strings where the Application protocol requires them.

## Virtual Recorder tutorial

This tutorial uses only the safe `virtual-demo` composition. Start Runtime from the
source tree:

```powershell
$db = [IO.Path]::GetFullPath((Join-Path $PWD "recorder-tutorial.sqlite"))
cargo run -p lab-runtime --locked -- `
  --serve --profile virtual-demo --port 7420 `
  --record-db $db --record-policy required
```

From an extracted preview package, replace the `cargo run ... --` prefix with
`./lab-runtime.exe`. The database path must be local and absolute. Wait for Runtime's
readiness line before connecting.

The following PowerShell client performs hello, starts a run, records an annotation,
stops and seals the exact run, lists archived runs through the public history API,
and requests clean Runtime shutdown. It consumes both the immediate `accepted` and
terminal operation envelopes; it never assumes that one line is the whole mutation.

```powershell
$tcp = [Net.Sockets.TcpClient]::new("127.0.0.1", 7420)
$stream = $tcp.GetStream()
$reader = [IO.StreamReader]::new($stream, [Text.UTF8Encoding]::new($false))
$writer = [IO.StreamWriter]::new($stream, [Text.UTF8Encoding]::new($false))
$writer.NewLine = "`n"
$writer.AutoFlush = $true

function Send-Json([hashtable]$message) {
  $writer.WriteLine(($message | ConvertTo-Json -Compress -Depth 20))
}
function Read-Json { $reader.ReadLine() | ConvertFrom-Json }
function Read-Terminal {
  $accepted = Read-Json
  if ($accepted.type -ne "operation" -or $accepted.state -ne "accepted") {
    throw "mutation was not accepted: $($accepted | ConvertTo-Json -Compress)"
  }
  $terminal = Read-Json
  if ($terminal.type -ne "operation" -or
      $terminal.state -notin @("completed", "failed")) {
    throw "terminal operation result missing"
  }
  if ($terminal.state -eq "failed") {
    throw "operation failed: $($terminal.code)"
  }
  return $terminal
}

Send-Json @{v=1; msg_id="hello"; op="hello"; args=@{scope=$null}}
$hello = Read-Json
$scope = $hello.result.scope

Send-Json @{v=1; msg_id="status-before"; op="recording_status"; args=@{}}
$before = Read-Json
$databaseId = $before.result.database_id

Send-Json @{v=1; msg_id="start"; op="recording_start"
  request_id=@{scope=$scope; seq="1"}; args=@{label="virtual tutorial"}}
$started = Read-Terminal
$runId = $started.result.run_id

Send-Json @{v=1; msg_id="note"; op="experiment_annotate"
  request_id=@{scope=$scope; seq="2"}
  args=@{name="operator_note"; data=@{text="virtual tutorial marker"}}}
$annotation = Read-Terminal
# annotation.result.durability is "pending"; stop supplies the drain/seal barrier.

Send-Json @{v=1; msg_id="stop"; op="recording_stop"
  request_id=@{scope=$scope; seq="3"}; args=@{run_id=$runId}}
$stopped = Read-Terminal

Send-Json @{v=1; msg_id="status-after"; op="recording_status"; args=@{}}
$after = Read-Json
if ($after.result.state -ne "idle") { throw "Recorder did not become idle" }

Send-Json @{v=1; msg_id="runs"; op="history_read"
  request_id=@{scope=$scope; seq="4"}
  args=@{mode="runs"; database_id=$databaseId; max_records=32; cursor=$null}}
$history = Read-Terminal
$pageToken = $history.result.page_token
Send-Json @{v=1; msg_id="page"; op="history_page"
  args=@{page_token=$pageToken}}
$page = Read-Json
$page.result.runs | Format-Table
Send-Json @{v=1; msg_id="release"; op="history_release"
  args=@{page_token=$pageToken}}
$null = Read-Json

Send-Json @{v=1; msg_id="shutdown"; op="runtime_shutdown"
  request_id=@{scope=$scope; seq="5"}; args=@{}}
$shutdown = Read-Terminal
$tcp.Dispose()
```

The start terminal result reports that the start/provenance transaction committed
and fact admission opened. The annotation terminal result proves bounded FIFO
admission only (`durability:"pending"`). The stop terminal result is the barrier that
proves accepted facts drained and the run/interval seal transaction committed. The
history page comes from the frozen committed archive view, not direct SQL.

## What is durable?

The implementation distinguishes these boundaries:

| Evidence | What it proves |
|---|---|
| Runtime mutation `accepted` | SessionStore admitted exact mutation identity/payload; not Recorder completion. |
| Recorder ingress admission | A bounded fact/group entered Recorder FIFO credit; SQLite may still be pending. |
| `persisted_through_seq` | Highest Recorder record sequence committed and receipted by the owner. |
| completed `recording_start` | Run, interval, activation/provenance, and start transaction committed before ordinary fact admission opened. |
| completed annotation with `durability:"pending"` | Annotation was admitted with a record sequence; use later checkpoint/stop evidence for durability. |
| completed `recording_stop` | Accepted facts drained FIFO; interval/run were sealed and committed. Writer and boot remain open. |
| completed `history_read` | A bounded archive job completed and froze one connection-local page token. |
| `history_page` row | Data was read from the committed archive snapshot represented by that retained page. |
| clean Recorder close | terminal boot seal committed, worker closed, no outstanding records, and no Recorder error. |

Activation/provenance is committed before an enabled Recorder reaches ordinary
recording availability. Observations, operation/controller/reference/output/runtime
facts, annotations, and gaps enter bounded causal groups. A group is atomic at the
SQLite transaction boundary. Never infer rows beyond the last receipted checkpoint
after failure.

## Provenance

Recorder preserves enough source identity to interpret evidence in the topology that
produced it. Depending on the active composition, this includes Runtime build and
boot, frozen configuration/artifact content and hashes, instrument/parameter and
resource identities, device address/channel, units/ranges, binding generation,
mapping/configuration revision, observations and their lineage, and distinct output
proposal/send/ACK/readback/failure evidence.

Reconnect or reconfiguration creates new generation/revision evidence. Historical
rows retain their original identities; Recorder never rewrites them as current.
SimpleDevice provenance is explained in [SimpleDevice](simple-device.md#recorder-and-provenance),
and native integrations in the
[native driver guide](developer/full-driver-tutorial.md#step-11-recorder-provenance).

## SQLite ownership and supported access

One Recorder storage thread exclusively owns the live SQLite connection and all
blocking SQL. Runtime owners use bounded nonblocking admission and receipt polling.
Version-one archives use WAL and `synchronous=FULL`, but those settings do not turn an
unreceipted fact into a durability promise.

Use a local filesystem path. Startup validates archive identity/schema/indexes and
fails rather than migrating or silently replacing an incompatible/corrupt archive.
Runtime does not expose schema migration as a public operation.

The stable product boundary is the Application history API, not SQL tables. Do not
write the live database with external tools. Offline read-only inspection is useful
for diagnostics/export only: stop Runtime cleanly or work from a consistent copy that
includes the SQLite/WAL state. No online backup workflow is currently promised.

## History API model

`history_read` is a mutation because it schedules bounded archive work and needs an
admitted operation outcome. It has two modes:

- `runs`: archive run discovery, at most 32 summaries per page;
- `measurements`: one archive boot/run/signal and half-open publication-time range,
  at most 128 raw measurement rows per page.

The flow is:

```text
history_read -> accepted -> completed(page_token)
             -> history_page -> history_release
```

One connection can hold at most one pending history job and one completed retained
page. Across Runtime there are at most eight history jobs/pages and eight retained
continuation cursors. Jobs have a two-second Application deadline. Pages expire after
five seconds; continuation cursors after thirty seconds. A page is at most 8 KiB.

`page_token` retrieves/releases the already completed immutable page. `next_cursor`
selects the following archive page and is supplied to a new `history_read` mutation.
Tokens/cursors are bound to connection, scope where applicable, archive, mode/filter,
and expiry. Disconnect cancels/fences pending work and removes pages/cursors; they do
not survive reconnect. A disconnected admitted history operation is retained as
failed with `client_disconnected`, while a late worker result is prevented from
entering a reused client slot.

Important failures include `history_busy`, `history_timeout`,
`history_database_unknown`, `history_archive_mismatch`, `history_cursor_mismatch`,
`history_cursor_expired`, `history_page_expired`, `history_page_oversize`, and
`history_token_exhausted`. These affect history selection, not experiment state.
See [operations](api/operations.md) for exact argument shapes and
[errors and limits](api/errors-and-limits.md) for public categories.

## Three separate contracts

```text
Application API contract != Recorder semantic contract != SQLite schema
```

The Application API projects lifecycle and bounded durable-history pages. The
Recorder accepts semantic facts and establishes durable boundaries. SQLite is the
versioned physical archive format. Runtime code does not issue SQL, and direct SQL
is never a live-control interface.

## Lifecycle and durability

The owner-visible Recorder states are:

| State | Meaning |
|---|---|
| `unconfigured` | Application projection only: no Recorder was attached. |
| `idle` | Storage worker is open; no run accepts facts. |
| `starting` | Durable start/provenance boundary is queued; Required control remains inhibited. |
| `recording` | Start committed; bounded fact admission is open. |
| `stopping` | Ordinary admission is closed while accepted groups drain and seals commit. |
| `failed` | Sticky admission, worker, or SQLite failure; ordinary coverage is closed. |
| `closed` | Worker closed its SQLite connection. |

The important boundaries are:

```text
fact accepted by Recorder ingress != fact committed to SQLite
```

The status `persisted_through_seq` is the highest record sequence committed and
receipted by the owner. A successful `recording_start` operation means the run,
interval, activation/provenance, and start transaction committed before fact
admission opened. A successful `recording_stop` means accepted facts drained FIFO,
the interval and run were sealed, and the transaction committed. It does not mean
the writer thread was destroyed or the process boot was terminally sealed.

Run and interval identities are `(boot_id, run_no)` and `(boot_id, interval_no)`.
The boot ID is the same 16-byte process identity exposed by the Application API.
Counters and monotonic timestamps are encoded as eight-byte unsigned big-endian
BLOBs so byte ordering is numeric ordering.

### Recording policy

- `required`: Recorder failure is fail-closed for experiment-critical control. The
  controller/output authority transitions through the accepted safe/failure path;
  storage failure cannot silently become a warning.
- `best_effort`: the Recorder becomes visibly failed and its committed prefix stops,
  but unrelated valid native acquisition/control is not stopped solely by storage
  failure.

Neither policy allows a false durable acknowledgement.

## Ingress and storage settings

Default owner-side ingress credit is 1,545 records, 4 MiB accounted bytes, thirteen
causal groups, and at most 512 KiB per group. The supported budget calculation is
four acquisition groups, one reconnect's five groups (accepted audit, activation,
baseline, probe, terminal audit), and two Reference mutations' accepted/completion
pairs. It assumes that bounded combination of outstanding work; it does not
guarantee reconnect admission under arbitrary acquisition pressure. Four groups and
six records are protected for those Reference pairs; ordinary traffic cannot spend
that credit. Reconnect has no separately protected pool: acquisition and reconnect
share the remaining nine groups and ordinary record/byte credit. Its activation,
rebind and probe reservations are incremental, not an atomic five-group admission.
Their waits retain the existing absolute deadlines; sustained competing acquisition
can exhaust those deadlines. SQLite still coalesces at most four fact groups per
transaction.

Before a recorded Reference mutation is accepted, the owner atomically reserves
its accepted audit and completion budget. Insufficient credit returns `busy` with
`accepted: false`, without consuming its mutation sequence or changing Reference.
Wire request sequencing is unchanged. A known identity still resolves through the
existing identity/status rules. The completion group contains the Reference fact,
if produced, and its terminal audit. Reservation guarantees bounded admission,
not successful SQL commit: only a committed receipt advances the durable prefix.
Required Recorder failure rejects subsequent Reference control; best-effort retains
its visible failed recording status while permitting otherwise valid control.

Up to eight durable-history jobs run through the same bounded worker boundary.
Producers use nonblocking admission; SQLite blocking occurs only on the storage
thread. This finite operating envelope does not cover unlimited arrivals or an
arbitrarily stalled disk. Ordinary overflow retains the fail-closed/gap policy.

Version-one databases use:

| Setting | Value |
|---|---|
| `PRAGMA application_id` | `1279345234` (`LABR`) |
| `PRAGMA user_version` | `1` |
| declared schema / record encoding | `1` / `1` |
| journal / synchronous | WAL / FULL |
| foreign keys | enabled |
| SQLite busy timeout | 100 ms |
| main logical file cap | 1 GiB, with a required 5% reserve |
| WAL threshold | 16 MiB; reaching it requires a successful truncate checkpoint |

These are storage safety bounds, not performance promises.

## Time and identities

Control and record ordering use process-monotonic nanoseconds. `published_at` is the
Runtime publication/commit time; `observed_at` preserves the source/model freshness
time; `captured_at` records Recorder capture where applicable. Eight-byte time BLOBs
contain unsigned big-endian nanoseconds.

`clock_anchors` bracket a wall-clock read with the process monotonic clock. The
optional `wall_estimate_us` in `records` is a display estimate based on that fixed
mapping. It is not a device timestamp and never influences control. Wall time uses
signed Unix microseconds.

`database_id` is a stable 32-character lowercase hexadecimal archive identity.
`boot_id` is a 16-byte process identity. `record_seq` is the canonical durable order
within a boot. Optional `fact_seq` preserves the Runtime source-fact identity and is
unique within the boot.

## Relationship overview

```text
schema_version

runtime_boots 1 -- * clock_anchors
      |
      +-- * configurations 1 -- * object_snapshots
      |            |
      |            +-- provenance_content (content-addressed)
      |
      +-- * runs 1 -- * recording_intervals
      |       |
      |       +-- * records 1 -- 0..1 measurements
      |                    +-- 0..1 operation_events
      |                    +-- 0..1 controller_events
      |                    +-- 0..1 reference_events
      |                    +-- 0..1 output_events
      |                    +-- 0..1 runtime_events
      |                    +-- 0..1 gaps
      |
      +-- 1 durable_checkpoints
```

`records` is the ordered common envelope. Typed projection tables use the same
`(boot_id, record_seq)` primary key and foreign key.

## Complete schema v1

The following tables and columns are the complete schema created by
`recorder/sqlite/schema.rs`. `NN` means `NOT NULL`; `PK` identifies the primary key.

### Archive and lifecycle

| Table | Columns | Keys, checks, and meaning |
|---|---|---|
| `schema_version` | `singleton INTEGER`, `database_id TEXT NN`, `schema_version INTEGER NN`, `record_encoding INTEGER NN`, `created_wall_us INTEGER` | `singleton` PK and must be 1; database ID unique. One archive-format declaration. |
| `runtime_boots` | `boot_id BLOB`, `build_version TEXT NN`, `started_wall_us INTEGER`, `ended_wall_us INTEGER`, `anchor_before BLOB`, `anchor_after BLOB`, `anchor_uncertainty_ns BLOB`, `state TEXT NN`, `exit_summary TEXT`, `recovered_by_boot BLOB` | `boot_id` PK, exactly 16 bytes. One process/storage-owner lifecycle. |
| `clock_anchors` | `boot_id BLOB NN`, `anchor_no BLOB NN`, `record_seq BLOB`, `kind TEXT NN`, `monotonic_before BLOB NN`, `monotonic_after BLOB NN`, `uncertainty_ns BLOB NN`, `wall_us INTEGER`, `unavailable_reason TEXT` | PK `(boot_id,anchor_no)`; boot FK; optional record FK. Actual or explicitly unavailable UTC sample. |
| `runs` | `boot_id BLOB NN`, `run_no BLOB NN`, `label TEXT NN`, `policy TEXT NN`, `state TEXT NN`, `coverage TEXT NN`, `started_wall_us INTEGER`, `ended_wall_us INTEGER`, `initial_activation_id BLOB` | PK `(boot_id,run_no)`; run number is 8 bytes; boot FK. |
| `recording_intervals` | `boot_id BLOB NN`, `interval_no BLOB NN`, `run_no BLOB NN`, `state TEXT NN`, `coverage TEXT NN`, `boundary_seq BLOB`, `start_seq BLOB`, `end_seq BLOB`, `loss_summary TEXT` | PK `(boot_id,interval_no)`; FK to run. Coverage segment and seal boundaries. |
| `durable_checkpoints` | `boot_id BLOB`, `commit_no BLOB NN`, `persisted_through_seq BLOB NN`, `last_probe_at BLOB`, `coverage TEXT` | `boot_id` PK/FK. Owner-verifiable committed prefix and coverage. |

### Configuration and provenance

| Table | Columns | Keys, checks, and meaning |
|---|---|---|
| `configurations` | `boot_id BLOB NN`, `activation_no BLOB NN`, `manifest_root_hash BLOB`, `manifest_content_hash BLOB`, `encoding TEXT`, `committed_at BLOB`, `object_revisions TEXT` | PK `(boot_id,activation_no)`; boot FK. Frozen activation identity and revisions. |
| `provenance_content` | `content_hash BLOB NN`, `encoding TEXT NN`, `kind TEXT NN`, `content BLOB NN` | PK `(content_hash,kind,encoding)`; hash is 32 bytes. Deduplicated exact source/definition/build content. |
| `object_snapshots` | `boot_id BLOB NN`, `activation_no BLOB NN`, `object_kind TEXT NN`, `object_id BLOB NN`, `logical_key TEXT`, `label TEXT`, `generation BLOB`, `descriptor TEXT`, `unit_key TEXT`, `instance_binding TEXT`, `definition_hash BLOB`, `source_hash BLOB`, `safety_hash BLOB` | PK `(boot_id,activation_no,object_kind,object_id)`; FK to configuration. Immutable per-activation object projection. |

`source_hash` and historical provenance kinds such as `managed_lua_source` and
`managed_component_source` are retained for archive compatibility. They are storage
vocabulary, not active scripting/runtime architecture. Native activations include
semantic implementation ID, exact configuration/state identity, and Runtime binary
SHA-256 through the generic provenance path.

### Common record envelope

| Table | Columns | Keys, checks, and meaning |
|---|---|---|
| `records` | `boot_id BLOB NN`, `record_seq BLOB NN`, `run_no BLOB`, `interval_no BLOB`, `kind TEXT NN`, `version INTEGER NN`, `fact_seq BLOB`, `published_at BLOB`, `observed_at BLOB`, `captured_at BLOB`, `wall_estimate_us INTEGER`, `wall_basis TEXT`, `origin TEXT`, `target TEXT`, `cause TEXT`, `payload BLOB` | PK `(boot_id,record_seq)`; version must be 1; sequence/fact IDs are 8 bytes; optional FKs to run/interval; `(boot_id,fact_seq)` unique when present. |

### Typed facts

| Table | Columns | Keys, checks, and meaning |
|---|---|---|
| `measurements` | `boot_id BLOB NN`, `record_seq BLOB NN`, `run_no BLOB NN`, `instrument_id BLOB NN`, `parameter_id BLOB NN`, `generation BLOB NN`, `revision BLOB NN`, `state_revision BLOB`, `observed_at BLOB NN`, `published_at BLOB NN`, `unit_key TEXT NN`, `quality TEXT NN`, `failure TEXT`, `value_kind TEXT NN`, `float_value REAL`, `integer_value INTEGER`, `bool_value INTEGER`, `text_value TEXT`, `lineage BLOB` | PK/FK to record. Unit length 1..64; quality is `good` or `unavailable`; checks enforce exactly one correctly typed value for Good and no value plus a failure for Unavailable. |
| `operation_events` | `boot_id BLOB NN`, `record_seq BLOB NN`, `request_scope TEXT`, `request_seq TEXT`, `phase TEXT`, `command TEXT`, `result TEXT`, `outcome_basis TEXT` | PK/FK to record. Application admission and terminal domain outcome. |
| `controller_events` | `boot_id BLOB NN`, `record_seq BLOB NN`, `controller_id BLOB`, `before_state TEXT`, `after_state TEXT`, `event_kind TEXT`, `config_revision BLOB`, `input_correlation TEXT`, `reference_correlation TEXT`, `output_correlation TEXT`, `diagnostics TEXT` | PK/FK to record. Controller lifecycle/configuration evidence. |
| `reference_events` | `boot_id BLOB NN`, `record_seq BLOB NN`, `reference_id BLOB`, `revision BLOB`, `event_kind TEXT`, `value REAL`, `target REAL`, `rate REAL`, `unit_key TEXT`, `progress_at BLOB` | PK/FK to record. Fixed/ramp lifecycle and value facts. |
| `output_events` | `boot_id BLOB NN`, `record_seq BLOB NN`, `attempt_id BLOB`, `dispatch_id BLOB`, `resource_id BLOB`, `instrument_id BLOB NN`, `parameter_id BLOB NN`, `authority_epoch BLOB`, `generation BLOB`, `revision BLOB`, `stage TEXT NN`, `value REAL`, `unit_key TEXT`, `evidence_source TEXT`, `evidence_basis TEXT`, `failure TEXT`, `ambiguous INTEGER`, `settled INTEGER` | PK/FK to record. Distinct proposal/send/ACK/readback/failure evidence; never proof of physical effect. |
| `runtime_events` | `boot_id BLOB NN`, `record_seq BLOB NN`, `category TEXT`, `severity TEXT`, `code TEXT`, `data BLOB` | PK/FK to record. Bounded semantic Runtime facts, not diagnostic log chatter. |
| `gaps` | `boot_id BLOB NN`, `record_seq BLOB NN`, `interval_no BLOB`, `first_missing BLOB`, `last_missing BLOB`, `known_count BLOB`, `unknown_tail INTEGER`, `reason TEXT`, `last_confirmed BLOB` | PK/FK to record. Known missing range or conservative unknown tail. |

Measurement `lineage` is bounded JSON for a managed transform's captured source:
source signal/value/unit, publication/observation times, and source
generation/revision/state revision. It is not resolved against current state when
reading historical data.

### Required indexes

| Index | Columns / predicate |
|---|---|
| `runtime_boots_unfinished` | `(state,boot_id)` |
| `runs_unfinished` | `(state,boot_id,run_no)` |
| `intervals_unfinished` | `(state,boot_id,run_no)` |
| `intervals_by_run` | `(boot_id,run_no)` |
| `records_source_fact` | unique `(boot_id,fact_seq)` where fact is not null |
| `measurements_history` | `(boot_id,run_no,instrument_id,parameter_id,published_at,record_seq)` |
| `measurements_all_history` | `(instrument_id,parameter_id,published_at,record_seq)` |
| `operation_identity` | `(request_scope,request_seq,record_seq)` |

## Gaps, seals, and completeness

`coverage='complete'` is meaningful only with the expected clean lifecycle seals.
An explicit loss has a `gaps` record and gap coverage. A storage failure may persist
a failure seal; if that cannot itself be committed, coverage remains conservatively
unsealed/unknown. An abnormal process end leaves active runs/intervals unsealed.

| Archive condition | Structural readability | Semantic completeness |
|---|---|---|
| clean stop + clean process shutdown | Integrity checked; WAL/SHM settle on close | Sealed run/interval with `complete` coverage can be treated as complete within the Recorder contract. |
| explicit gap | Surviving committed rows remain readable | Incomplete; inspect `gaps` and coverage. |
| Recorder failure | Committed prefix may remain readable | Never infer rows beyond the checkpoint; failure seal or unknown tail is authoritative. |
| process kill | SQLite recovery can preserve committed WAL prefix | Run/interval remain active until reopen marks them `interrupted` with conservative `unknown_tail`. Not a clean experiment. |
| corrupt/incompatible file | Open validation fails | No completeness claim. |

On reopening a structurally valid archive, an earlier active boot and unfinished
run/interval are marked interrupted by the new storage owner after validating the
checkpoint against the final committed record. Reopen is for inspection/new boot;
it does not resurrect controller state or output authority.

Clean-close tests establish sealed boot/run/interval state, `integrity_check=ok`,
committed facts, and clean WAL/SHM behavior. Process-kill tests establish recovery
of a committed prefix and truthful interruption. They do not qualify real power
loss, drive caches, or physical disk-full behavior.

## Offline SQL examples

Open a copied or closed archive read-only. The examples use `hex()` because semantic
u64 IDs are eight-byte big-endian BLOBs.

List runs:

```sql
SELECT lower(hex(boot_id)) AS boot_id,
       lower(hex(run_no)) AS run_no_be,
       label, policy, state, coverage, started_wall_us, ended_wall_us
FROM runs
ORDER BY boot_id, run_no;
```

Read one signal/run. Supply `:boot_id_hex` as 32 hex characters and the remaining
IDs as 16-character big-endian hex:

```sql
SELECT lower(hex(m.record_seq)) AS record_seq_be,
       lower(hex(m.published_at)) AS published_ns_be,
       lower(hex(m.observed_at)) AS observed_ns_be,
       m.quality, m.failure, m.value_kind,
       m.float_value, m.integer_value, m.bool_value, m.text_value,
       m.unit_key, lower(hex(m.generation)) AS generation_be
FROM measurements AS m
WHERE lower(hex(m.boot_id)) = lower(:boot_id_hex)
  AND lower(hex(m.run_no)) = lower(:run_no_be_hex)
  AND lower(hex(m.instrument_id)) = lower(:instrument_id_be_hex)
  AND lower(hex(m.parameter_id)) = lower(:parameter_id_be_hex)
ORDER BY m.published_at, m.record_seq;
```

Inspect gaps:

```sql
SELECT lower(hex(g.boot_id)) AS boot_id,
       lower(hex(g.interval_no)) AS interval_no_be,
       lower(hex(g.first_missing)) AS first_missing_be,
       lower(hex(g.last_missing)) AS last_missing_be,
       lower(hex(g.known_count)) AS known_count_be,
       g.unknown_tail, g.reason,
       lower(hex(g.last_confirmed)) AS last_confirmed_be
FROM gaps AS g
ORDER BY g.boot_id, g.record_seq;
```

Inspect activation provenance and content identities:

```sql
SELECT lower(hex(o.boot_id)) AS boot_id,
       lower(hex(o.activation_no)) AS activation_no_be,
       o.object_kind, lower(hex(o.object_id)) AS object_id_hex,
       o.logical_key, o.label, o.descriptor,
       lower(hex(o.definition_hash)) AS definition_sha256,
       lower(hex(o.source_hash)) AS source_sha256,
       lower(hex(o.safety_hash)) AS safety_sha256
FROM object_snapshots AS o
ORDER BY o.boot_id, o.activation_no, o.object_kind, o.object_id;
```

Check seals and completeness for all intervals:

```sql
SELECT lower(hex(r.boot_id)) AS boot_id,
       lower(hex(r.run_no)) AS run_no_be,
       r.state AS run_state, r.coverage AS run_coverage,
       lower(hex(i.interval_no)) AS interval_no_be,
       i.state AS interval_state, i.coverage AS interval_coverage,
       i.start_seq IS NOT NULL AS has_start,
       i.end_seq IS NOT NULL AS has_end,
       (SELECT count(*) FROM gaps g
        WHERE g.boot_id=i.boot_id AND g.interval_no=i.interval_no) AS gap_count
FROM runs r
JOIN recording_intervals i USING (boot_id, run_no)
ORDER BY r.boot_id, r.run_no, i.interval_no;
```

Inspect Application operation outcomes:

```sql
SELECT lower(hex(e.boot_id)) AS boot_id,
       lower(hex(e.record_seq)) AS record_seq_be,
       e.request_scope, e.request_seq, e.phase, e.command,
       e.outcome_basis, e.result
FROM operation_events e
ORDER BY e.boot_id, e.record_seq;
```

Use the Application API for live control and bounded history access. Direct SQLite
queries are intended for offline analysis, archive inspection, and export only.
