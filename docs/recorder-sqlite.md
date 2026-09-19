# Recorder and SQLite archive reference

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

Default owner-side ingress credit is 1,024 records, 4 MiB accounted bytes, four
causal groups, and at most 512 KiB per group. Up to eight durable-history jobs run
through the same bounded worker boundary. Producers use nonblocking admission;
SQLite blocking occurs only on the storage thread.

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

runtime_boots 1 ── * clock_anchors
      |
      +── * configurations 1 ── * object_snapshots
      |            └──────────── provenance_content (content-addressed)
      |
      +── * runs 1 ── * recording_intervals
      |       |
      |       +── * records 1 ── 0..1 measurements
      |                    ├──── 0..1 operation_events
      |                    ├──── 0..1 controller_events
      |                    ├──── 0..1 reference_events
      |                    ├──── 0..1 output_events
      |                    ├──── 0..1 runtime_events
      |                    └──── 0..1 gaps
      |
      └── 1 durable_checkpoints
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
