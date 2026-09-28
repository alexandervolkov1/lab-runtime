# M14.3 WorkbenchModel and PresentationDocument

## Review status

```text
M14.2 minimal native Application client: ACCEPTED

M14.3 WorkbenchModel + PresentationDocument:
READY FOR EXTERNAL RE-REVIEW

M14.4 GUI: NOT AUTHORIZED

STATUS: M14_3_RECOVERY_PROJECTION_REMEDIATION_READY_FOR_EXTERNAL_REVIEW
```

## External-review acceptance

External re-review accepted M14.3, including fresh-only `RuntimeRef` resolution,
explicit rebuild completion, separate `CommandId`/`MutationIdentity` domains, the
single-Workbench v0.1 workspace-ownership assumption, and the worker-owned bounded
recovery projection. The accepted implementation/evidence commit is
`f01567ba2165b24b9551e3b0acf5b100b0169f32`. M14.4 is authorized only for the
minimal eframe/egui GUI; M14.5 remains unauthorized.

The accepted M14.2 implementation commit is
`bdedf9455305f693a9537f698403c3bbb3840c51`; its coordination acceptance commit is
`668f00369ebeea16f68af61a410c1c0adce03b38`. M14.3 is an uncommitted review
candidate. It changes only the private `lab-workbench` client/model/persistence
surface. It adds no Runtime operation, DTO, public error, session behavior, GUI,
Steel, transport, physical authority or server-owned presentation concept.

## External-review remediation

The first M14.3 external review accepted the persistence and durable-journal
architecture in principle and identified three model-state blockers plus one process
ownership assumption that needed to be explicit. The remediation is deliberately
limited to the private Workbench model and its evidence:

- a `RuntimeRef` resolves only when its matching observation is individually
  `Fresh`; retained `Stale` observations remain displayable cache but cannot prove
  current Runtime existence;
- `begin_rebuild`, per-entity `observe`, and explicit `complete_rebuild` now have
  separate roles, so the first query/event cannot promote a multi-projection rebuild
  to globally fresh;
- caller-local `CommandId` and Application `MutationIdentity` are separate key
  domains; reconciliation records never manufacture an action ID from
  `request_id.seq`;
- v0.1 explicitly supports one active Workbench process per resolved user-data
  workspace until a later process-ownership gate adds enforcement or a reviewed
  journal namespace.

The cleanup also removed normalization for a nonexistent `instrument` query
operation. Instrument identities continue to come from the authoritative discovery
operations; there is still no copied operation registry.

## External re-review recovery-projection remediation

The external re-review accepted the preceding freshness, rebuild, identity-domain
and single-process ownership corrections, then found that the model still attempted
to reconstruct worker recovery from `ReconciliationRequired` and individual Reply
records. That could leave an `operation_status` result displayed as the old
`Ambiguous` state, or reinsert a record that the worker had retired after an exact
retry.

The single Application client worker remains the only recovery-state owner. It now
publishes one bounded, immutable update:

```text
RecoveryState { records: Vec<RecoveryRecord> }
```

The vector is always bounded by `MAX_IN_FLIGHT = 8`. The worker publishes its full
current projection after startup journal load, hello reconciliation, pre-wire
insertion, accepted/ambiguous/terminal state changes, `operation_status`, exact-retry
retirement, resolved-record capacity retirement, removal, and invalid retained-scope
projection removal. No mutable worker storage is exposed.

`WorkbenchModel` transactionally replaces `ClientRecoveryState.mutations` only from
`RecoveryState`; Reply records no longer mutate that projection. An over-bound
internal projection is rejected without replacing the last valid model state.
`ReconciliationRequired` remains a distinct action-needed notification. Its bounded
identity list is separate from current recovery records and includes only
`Pending`, `Ambiguous`, or notified `Accepted` records; `Completed` and `Failed`
records may remain visible as resolved exact-retry records without being described
as requiring reconciliation.

## Implemented boundary

```text
future GUI / future Steel ui/*
             |
             | UiCommand (validated, client-only)
             v
      PresentationDocument owner
             |
             +----------+
                        v
future GUI / future Steel lab/* -> WorkbenchModel
                        |
                        | bounded M14.2 commands/updates
                        v
              ONE Application client worker
                        |
                        | TCP / NDJSON
                        v
 lab-runtime: ONE Application / ONE SessionStore / Runtime
```

The M14.2 worker remains the only socket, scope, `msg_id`, mutation sequence,
subscription, cursor and exact-retry owner. M14.3 does not add another connection or
protocol model. `WorkbenchCommand::{Lab,Ui}` records the authority split: `LabCommand`
is intent for the existing worker, while `UiCommand` mutates only a validated
presentation candidate.

## Source and module ownership

| Module | Owner/responsibility |
| --- | --- |
| `client/worker.rs` | Existing single TCP client owner, extended narrowly to own the durable recovery-journal ordering required by its existing mutation identity. |
| `model/mod.rs` | One renderer-neutral `WorkbenchModel`; consumes ordered `ClientUpdate` values and owns observational freshness, pending-action display state and unresolved references. |
| `model/projections.rs` | Rebuildable observations and bounded 4,096-point live display buffers. |
| `model/command.rs` | Transactional `UiCommand`, small `LabCommand` intents and the explicit `WorkbenchCommand` split. |
| `presentation/document.rs` | Version-one presentation plain data and full-candidate validation. |
| `presentation/reference.rs` | Typed stable Runtime identities; never mutable display names. |
| `presentation/persistence.rs` | Bounded JSON load/save and validate-before-replace behavior. |
| `recovery/journal.rs` | Separate bounded exact-mutation recovery format, validation and hello classification. |
| `storage.rs` | Shared bounded read and sibling-temp/replace file mechanics. |

No module is public outside the `lab-workbench` executable and no reusable SDK or
second DTO hierarchy was introduced.

## Three state classes

`WorkbenchModel` keeps three ownership classes explicit:

1. `RuntimeObservations` is an ephemeral cache of Application projections and live
   display points. Values are marked `Unknown`, `Rebuilding`, `Fresh` or `Stale`;
   those labels describe client observation state, not physical quality or Runtime
   truth.
2. `ClientRecoveryState` holds the observed boot, retained scope,
   `hello.next_seq`, last usable process cursor, the worker's bounded exact recovery
   projection, and a separate bounded reconciliation-required identity list. It is
   session/dedup reconciliation aid and is not layout. The model never reconstructs
   recovery ownership from replies.
3. `PresentationDocument` is persistent client-owned layout/display configuration.
   It contains no Runtime observation, operation outcome, Recorder fact, controller
   state or physical-safety assertion.

Disconnect, transport failure and worker stop mark cached observations stale.
Connecting/reattaching, `event_gap`, ordered-update overflow and explicit
`ResnapshotRequired` enter `Rebuilding`. A boot change makes prior observations
stale and clears in-memory recovery authority. Only new snapshots/events make an
individual observation fresh again. Those observations do not finish the overall
rebuild: the model owner must call `complete_rebuild` only after its required
projection set and replay barriers have completed. An incomplete rebuild therefore
remains globally `Rebuilding`.

## Stable Runtime references and unresolved state

`RuntimeRef` is a tagged, non-recursive enum for:

```text
Instrument
Signal { instrument, parameter }
Reference
Controller
Resource
Component
Recorder
ConfigurationProperty { typed owner, property }
```

Each variant uses stable IDs already exposed by the Application contract. Display
labels are separate presentation strings. Loading does not require the current
Runtime to contain every target. `WorkbenchModel::refresh_unresolved` retains the
valid presentation entry and identifies absent or currently unverified targets
visibly; it never creates or deletes a Runtime entity. A target is resolved only by
a `Fresh` observation in the current rebuild/view. A missing or `Stale` cached
observation is unresolved, including after disconnect, resnapshot and boot change.

## PresentationDocument v1

The renderer-neutral v1 document contains:

```text
format_version = 1
document_id
windows -> tabs -> panels
plots -> axes + traces
controls
```

Panels reference plot/control IDs or a typed Runtime status target. Plots contain
only display configuration: stable ID, title, time window, optional axis range and
traces. A trace contains a `RuntimeRef`, client label, visibility, style and optional
display unit. Measurement arrays are excluded. Controls are declarative categories
that a later layer must map to existing Application operations; they contain no
closures, Steel expressions or optimistic Runtime state.

The shared `UiCommand` boundary currently supports plot/trace add/remove, trace
visibility/source, display time window and item rename. It clones the active
document, applies the intent, validates the whole candidate, and replaces active
state only on success. This is the intended sole mutation route for future GUI
events and Steel `ui/*` bindings.

## Frozen presentation bounds and validation

| Resource | Bound |
| --- | ---: |
| Encoded JSON file | 1 MiB |
| Windows | 64 |
| Tabs per window | 64 |
| Panels total | 64 |
| Plots | 32 |
| Traces per plot | 32 |
| Controls | 64 |
| Any UTF-8 string | 512 bytes |
| Live points per observed trace | 4,096 |
| Display time window | finite, greater than zero, at most seven days |

Validation rejects a missing/future/unknown schema, wrong JSON root, unknown fields
or enum tags, oversized files/strings/cardinalities, duplicate client-owned IDs,
dangling internal plot/control IDs, invalid typed Runtime references, invalid axis
ranges and non-finite/out-of-range numbers. Validation never truncates a document.
The active document is unchanged after failed load or `UiCommand`.

Live display buffers evict the oldest point at 4,096 and retain a local dropped
counter. This is explicitly display truncation and says nothing about Runtime or
Recorder history.

## Presentation persistence

The format is versioned JSON using the existing workspace `serde_json`. M14.3 adds
direct `serde 1` derive use; the lock resolves `serde 1.0.229` and retains
`serde_json 1.0.151`, so no new transitive package was introduced.

The default Windows per-user path is:

```text
%LOCALAPPDATA%\lab-runtime\workbench\presentation-v1.json
```

with `%APPDATA%` as a fallback. Tests always supply temporary explicit paths. No file
is written beside the executable, into Runtime archives, Recorder SQLite or an
experiment directory.

Save validates and serializes a bounded candidate, writes a unique sibling temporary
file, flushes and `sync_all`s it, closes it, then calls `std::fs::rename` once. The
sibling location keeps the operation on one volume, and Rust's supported Windows
implementation uses replace-existing file semantics, so the target directory entry
is replaced without a delete gap. Failure before/at replacement retains the prior
file. This is not a claim of power-loss durability for directory metadata because
stable `std` does not expose a portable directory flush contract.

## Durable recovery journal

The recovery journal is a separate version-one JSON format at:

```text
%LOCALAPPDATA%\lab-runtime\workbench\recovery-v1.json
```

Its exact bounds are 64 KiB and eight records, matching the accepted M14.2 mutation
recovery capacity. It persists only:

```text
format_version
boot_id
scope
last authoritative next_seq observation
request_id.seq
exact op
exact args
known admission state
```

The journal intentionally has no `msg_id`, subscription token, socket state, frozen
projection/page token, history cursor or presentation state. Empty, truncated,
invalid UTF-8, wrong-root, future/missing-version, duplicate/invalid sequence and
oversized candidates fail visibly. Loading produces records for reconciliation only;
it never sends or retries a mutation.

### Multi-instance ownership risk and re-entry gate

The default journal is one file per resolved user-data workspace and has no
interprocess lock or per-process namespace. The supported v0.1 ownership assumption
is therefore exactly one active Workbench process per such workspace. Multi-instance
safety is not claimed. The named **M14.4 pre-implementation process-ownership gate**
must either enforce exclusive ownership or accept a reviewed workspace/instance
journal namespace before a GUI can claim multi-instance support. No locking
framework is introduced in this remediation.

After authoritative hello, same boot and scope produce
`NeedsAuthoritativeStatus`; different boot or scope produces an explicit non-authoritative
disposition. `instance_changed` and `scope_unknown` never reinterpret disk state as
success. The exact retry remains the M14.2 worker-owned `RetryMutation { identity }`
path after `operation_status`, not caller-provided op/args.

## Journal/write ordering

The durable ordering is owned by the same worker that allocates `request_id.seq`:

```text
construct exact Pending record
    -> validate and synchronously replace bounded journal
    -> insert the same record in bounded worker recovery
    -> encode/queue mutation for TCP output
    -> Runtime accepted
    -> durably update admission when possible
    -> terminal result/status
    -> durably update/retire record
```

Thus a new mutation is never queued to the socket unless its exact record already
exists durably when persistence is enabled. A pre-wire save failure emits
`RecoveryJournalProblem`, rejects locally as `recovery_journal_unavailable`, and
writes no mutation bytes. A later status-update/retirement save failure keeps the
already durable exact pre-wire record, blocks further mutations, surfaces
uncertainty, and does not block or roll back Runtime progress. File work is bounded
to 64 KiB but normal OS file calls do not provide a hard completion deadline; a
stalled Workbench filesystem can delay this client worker, never the independent
Runtime process. Worker shutdown retains its accepted finite join deadline/error.

On Workbench startup a valid journal supplies the retained scope when no explicit
scope was requested. The worker still performs ordinary hello and emits
`ReconciliationRequired`; tests prove it emits no operation/retry bytes merely from
loading the journal.

## Projection and bootstrap rules

`WorkbenchModel::apply_client_update` is the one raw-update normalization point. It
handles connection/hello, ordinary/operation replies, ordered events, Reference
bootstrap, resnapshot, reconciliation, journal fault and transport failure. Initial
GUI projection kinds have model support for:

| Projection | Safe rebuild rule |
| --- | --- |
| Discovery | Consume the existing frozen `discover`/`discovery_page` records; page tokens remain connection-local. |
| Current signals | Consume the existing frozen `measurements_current` pages or `latest`; live events update the same typed signal identity. |
| Reference | Use the M14.2 subscribe-at-hello-cursor, query, revision-filtered replay barrier. |
| Controller/resource/Recorder | Query existing `controller`, `resource`, and `recording_status`, then consume ordered matching events; no generic revision is invented. |
| Configuration properties | Consume existing frozen configuration property pages by typed owner/property identity. |

For projections without Reference-style revisions, reconnect begins from stale data,
uses a fresh frozen query, and subscribes/replays from the authoritative hello cursor.
An event gap abandons continuity and requires another full rebuild. M14.3 adds no
server-side snapshot transaction. The accepted Reference barrier remains the coded
vertical slice; M14.4 must orchestrate the full query/page set and visible rebuild
state without teaching individual widgets to decode protocol envelopes. It must
invoke explicit rebuild completion only after the selected projection queries and
subscription/replay barriers have settled.

## Test evidence

Focused deterministic tests cover:

- PresentationDocument v1 round trip and successful replacement;
- every frozen cardinality/string/file bound, duplicate IDs, invalid ranges and
  invalid/future/corrupt/truncated/UTF-8/root/tag input;
- failed load and failed pre-replacement save preserving active/prior valid state;
- fresh Runtime references resolving, stale/missing references remaining unresolved,
  and a stale boot-A entity resolving again only after a fresh boot-B observation;
- 4,096-point oldest-first display eviction and dropped count;
- explicit multi-projection rebuild lifecycle: individual Reference/resource
  refreshes stay globally `Rebuilding`, explicit completion makes the view `Fresh`,
  and disconnect stales every cached entity;
- separate command/reconciliation identity domains, including equal numeric
  `command_id` and `request_id.seq` values without collision;
- `operation_status` replacing an `Ambiguous` worker/model projection with
  `Completed`;
- terminal exact retry emitting an empty projection before its old response record,
  with the model record remaining retired;
- ordinary terminal records retained identically by worker and model, followed by
  identical removal when the bounded capacity policy retires the oldest resolved
  record;
- centralized discovery/signal/controller/resource/Recorder normalization;
- transactional UI command validation and common Lab/UI command boundary;
- recovery-journal round trip, eight-record/64-KiB bounds, exact op/args, no
  connection-local token persistence and boot/scope classification;
- journal load never auto-executing;
- exact durable record existing before peer-observed mutation bytes;
- deterministic journal-write failure rejecting before any mutation bytes;
- all unchanged M14.2 client/framing/reconnect/backpressure/shutdown tests.

The opt-in real process acceptance starts `lab-runtime` with `virtual-demo`, connects
the actual Workbench client, bootstraps/observes Reference state into
`WorkbenchModel`, performs the accepted mutation/disconnect/reattach/status/exact
retry/replay flow, proves disconnect marks observations stale, proves the rebuilt
Reference becomes fresh, proves presentation state survives independently, and
proves Runtime remains alive after client shutdown.

Environment used for review-candidate verification:

```text
Windows 10 Pro 10.0.19045
rustc 1.95.0 (59807616e 2026-04-14)
cargo 1.95.0 (f2d3ce0bd 2026-03-21)
```

## Verification

All commands ran from the final M14.3 working tree on Windows 10 Pro 10.0.19045:

```text
cargo fmt --all -- --check
    PASS

cargo test --workspace --locked
    PASS on complete rerun

cargo test --workspace --release --locked
    PASS

cargo clippy --workspace --all-targets --locked -- -D warnings
    PASS

cargo test -p lab-workbench --locked
    PASS: 44 passed, 1 opt-in process acceptance ignored
    repeated three consecutive times after the final source change

cargo build -p lab-runtime -p lab-workbench --locked
cargo test -p lab-workbench --locked \
    runtime_acceptance::real_runtime_reference_reconnect_reconcile_and_replay \
    -- --ignored --exact --nocapture
    PASS: 1 passed

git diff --check
    PASS
```

The first debug-workspace invocation encountered a pre-existing Windows temporary
path collision inside two unchanged `emulator_api` tests (`file in use` and an
already-created SQLite schema). The isolated `emulator_api` suite passed immediately,
and the complete locked debug workspace rerun passed. No Runtime source or Runtime
test was changed in response.

Normal debug/release workspace runs retain the pre-existing ignored Runtime
diagnostic-rotation and preview-soak workloads unchanged. The one new Workbench
process acceptance is also opt-in because it starts a sibling executable; it passed
when invoked explicitly above.

## Remaining M14.4 work

M14.4 is not authorized. If later authorized it may add the separately reviewed
eframe/egui renderer, GUI command/update draining, repaint behavior and visible
connection/rebuild/unresolved/truncation states. It must reuse this model and the one
M14.2 Application client worker. It may not add Runtime presentation semantics,
another session/dedup owner, GUI-thread network I/O or Steel.

## Explicit non-goals retained

M14.3 adds no GUI/eframe/egui/plot renderer, Steel/Lua, PresentationDocument in
Runtime, Application operation/DTO/error/session change, transport, generic SDK,
remote deployment, authentication/TLS, Recorder/SQLite change, OutputAuthority
change or physical-safety claim.

```text
M14.3:
READY FOR EXTERNAL RE-REVIEW

M14.4:
NOT AUTHORIZED

STATUS: M14_3_RECOVERY_PROJECTION_REMEDIATION_READY_FOR_EXTERNAL_REVIEW
```
