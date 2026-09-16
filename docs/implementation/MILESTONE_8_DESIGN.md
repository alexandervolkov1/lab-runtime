# M8 — declarative deployment, safe lifecycle and read-only Windows COM

Status: ASTRA_HIGH design frozen for the explicit SOL_HIGH handoff, 2026-09-16.
This document specifies future behavior; it is not an implementation or hardware
acceptance report. M8 implementation is not authorized until the user crosses
the model gate. M9 is out of scope.

Baseline: `f3ff456`, clean checkout when inspected. The user externally accepted
M7, including D1-D18 and the reported 345-test debug/release gates. Preserve
[M7 design](MILESTONE_7_DESIGN.md), [M7 report](MILESTONE_7_REPORT.md),
[M3 transport contract](MILESTONE_3_DESIGN.md),
[M5 component contract](MILESTONE_5_DESIGN.md), and the accepted M1-M6 contracts.
Authority: the user's M7 external-review result and
[release plan sections 17-26](RELEASE_PLAN_M7_TO_V0_1.md).

## 1. Scope and fixed architectural choices

M8 adds deployment of existing native/M5 capabilities, three distinct lifecycle
operations, and physical Metakon **read-only** acquisition. It does not add new
controllers, a general graph engine, executable configuration, Lua workspace,
GUI, remote security, arbitrary serial commands or physical output acceptance.

Keep the three existing packages. TOML/JSON parsing, file loading, hashing, COM
handles, workers, SQLite and wire DTOs stay outside `lab-core`. Core receives
validated bounded values, identities and lifecycle commands; it owns revisions,
dependency fencing, Signal publication, OutputAuthority and Required safety.
The host coordinates asynchronous preparation and persistence, never invents
safe evidence, and never replaces the live Runtime wholesale to apply a file.

The existing M3 `ResourceExecutor` remains the sole transaction scheduler for a
resource. A COM worker is an I/O adapter, not another instrument/transaction
owner. Physical COM resources in M8 cannot carry output-affecting operations.
Existing virtual/test-transport output behavior remains available for safety
acceptance, explicitly labeled as such. Physical-output support would require
separate review of the final authority/first-byte boundary; a queued COM worker
must not silently inherit that capability.

## 2. Deployment entry point and schema

Add `lab-runtime --serve --config <local-runtime.toml>`. Preserve the finite demo
and existing `--serve --profile virtual-demo` entry points and M6/M7 tests.
Reject combinations of `--config` with profile/port/recording overrides rather
than providing hidden precedence. The config's parent directory is the base for
relative source and database paths, never the process's later working directory.
The trusted loader resolves paths off the Runtime owner. Preserve M7's local
database/path validation. No environment expansion, includes, globbing, template
evaluation, network fetch, shell command, startup procedure or automatic arming.

Use strict TOML schema version 1 and a maintained host-side TOML deserializer.
Pin the resolved parser version/features in Cargo.lock and report them in M8;
the acceptance contract, not a permissive generic TOML value tree, is normative.
Unknown fields at every level, duplicate keys/tables, duplicate IDs/logical keys,
unsupported variants, NaN/infinity and overflow are errors. Reject UTF-8 BOM and
invalid UTF-8 with a bounded diagnostic. Hash/store the exact accepted bytes,
including comments, whitespace and line endings, before parsing normalization.

Top-level fields are exactly `schema_version`, `runtime`, `server`, `recording`,
`resources`, `instruments`, `managed_components`, `references`, `controllers`,
and `safe_profiles`. Omitted lists are empty, not an implicit demo. Nested tables
are typed per variant; no arbitrary extras map. Required identifiers/settings
have no environment-dependent defaults. Optional defaults are versioned constants
and are shown in the effective configuration snapshot.

| Table | Schema-v1 responsibility |
| --- | --- |
| `runtime` | Stable deployment key, display name; neither confers authority |
| `server` | Loopback host and port; preserve M6 local-only policy |
| `recording` | Enabled flag, local database path when enabled, `best_effort` or `required`; M7 fixed limits are not tunables |
| `resources` | Explicit numeric stable resource ID, unique logical key, `windows_com_read_only` kind and serial settings below |
| `instruments` | Existing `virtual_measurement`, `thermal_plant` or `metakon` variants, stable instrument ID/key, typed native config or strict-JSON definition path/instance binding; positive polling period |
| `managed_components` | Stable ID/key, source path, M5 Source/Transform manifest, bounded plain config, signal binding, period; no new Lua capabilities |
| `references` | Existing Fixed/Ramp configuration with stable ID/key and compatible unit; no executable expressions |
| `controllers` | Existing native PID configuration, signal/reference/output IDs, cadence/freshness/finite lease; initially stopped |
| `safe_profiles` | Existing typed M2/M3 safe target, range, deadline and evidence requirement attached to an eligible output |

Numeric IDs are explicit positive integers representable by their existing Core
type. Zero, duplicates, invalid ParameterIds and reserved IDs are rejected before
construction. Logical keys are ASCII `[A-Za-z][A-Za-z0-9_-]*`, 1..64 bytes;
display names are nonblank UTF-8 up to 128 bytes. Do not allocate IDs according to
array order or current COM enumeration. Deployment key plus object kind/key is
the cross-boot logical identity; Runtime boot plus typed ID is the local identity.
TOML document order does not change dependency scheduling or default values.

Illustrative minimal physical deployment (the referenced definition must pass
the existing strict M3 loader; this is not a claim about a bench's settings):

```toml
schema_version = 1

[runtime]
key = "metakon-bench"
display_name = "Read-only temperature bench"

[server]
host = "127.0.0.1"
port = 7420

[recording]
enabled = true
path = "history.sqlite"
policy = "required"

[[resources]]
id = 1
key = "temperature-bus"
kind = "windows_com_read_only"
port = "COM3"
baud_rate = 9600
data_bits = 8
parity = "none"
stop_bits = 1
flow_control = "none"
read_timeout_ms = 50
write_timeout_ms = 50
open_timeout_ms = 2000
recovery_timeout_ms = 2000

[[instruments]]
id = 1
key = "furnace-temperature"
kind = "metakon"
definition = "definitions/metakon-temperature-tenths-fixture.json"
resource_id = 1
address = 1
poll_period_ms = 1000
```

Serial settings: baud 1..4,000,000 with actual driver support checked at open;
data bits 5/6/7/8, parity none/odd/even, stop bits 1/2, flow none/software/hardware.
Unsupported combinations fail explicitly, never silently substitute settings.
Read/write call timeouts are 1..250 ms, open/recovery deadlines 1..2000 ms.
They are distinct from M3 queue/total transaction deadlines (positive, <=60 s).
TOML's optional per-instrument `queue_timeout_ms` and `transaction_timeout_ms`
use those M3 bounds; defaults are 1000/1000 ms. Poll periods are 1..60000 ms,
with aggregate workload validated against host bounds, not interpreted as a hard
real-time guarantee.
Do not toggle DTR/RTS to reset a device implicitly; any required wiring/line policy
must be explicit in the bench procedure. Hardware-dependent unsupported settings
are activation failures, not pure parse failures.

No physical instrument definition may expose output/write capability on this
resource, including a register disguised as read-only. Reuse the M3 trusted
operation whitelist, address/register/type/CRC checks and channel-type probe.
Channel type 3 is compatibility information, not a unique device identity.
Physical temperature uses the M3 sentinel/range/scaling rules, not Modbus or
donor framing guessed from a name.

## 3. Validation, bounds and staged candidates

Validation order is read -> parse -> schema -> cross-reference -> safety ->
effective configuration/diff -> isolated preparation -> explicit activation.
Parsing/validation does not open COM/SQLite, invoke Lua, construct running
controllers, dispatch bytes or mutate active state. File reads and immutable
candidate allocation are the only loader effects. Bounded M5 init is a later
preparation step with no active-state access or physical capabilities.

Validate IDs, descriptors and parameter types; known operation mappings; unit
identity/conversions; finite ranges; positive monotonic durations and checked
deadline arithmetic; Reference/controller limits; output/safe-profile coverage;
finite leases; M5 dependency shape; source/transform/controller references;
duplicate normalized COM paths; and all existing Core/host limits. A Required
deployment cannot disable recording or start ordinary control before a durable
activation and valid M7 health gate. Invalid startup leaves no listeners/resources
advertised as ready and performs no experiment side effects.

| New retained structure | Hard bound / behavior |
| --- | --- |
| Loaded TOML | 64 KiB, enforced while reading; parser nesting <=8 and <=4096 syntactic values/tables; reject before building an unbounded tree |
| Definition/source | Existing M3 16 KiB JSON and M5 32 KiB Lua limits, including depth/member rules |
| Provenance source set | Existing 128 entries / 1 MiB, including TOML, manifests and accounting overhead; entries <=64 KiB |
| Active + candidate | One active immutable source set and one candidate set; no version cache; shared immutable ownership, no extra worker payload clones |
| Staged object baseline | <=256 objects /256 KiB as in M7; graph counts also obey the smaller existing host limits |
| Host graph | <=64 instruments total; <=8 resources, plants, references, controllers, outputs respectively; <=8 managed components combined; no increase to M5/M6 limits |
| Lifecycle operations | One active/staged operation globally, one loader job and one result; overlapping stage/apply/reload/restart/reconnect return Busy |
| Diff / diagnostics | <=256 diff entries, <=16 errors of <=256 bytes each plus explicit omitted count; wire pages <=32 entries and M6 frame bound |
| Loader worker | One fixed slot; read/prepare deadline 2 s; blocked slot is quarantined, not replaced repeatedly; no owner join |
| Candidate residence | 30 s after stage completion; expiry releases it, apply then returns Expired; no infinite renewal by Query |
| COM workers | <=8 fixed slots, one per resource; bounded mailboxes/buffers in section 9; no replacement while an old slot owns a handle or is quarantined |

Apply existing M5 VM/state/call/worker limits unchanged, including the two worker
slots and one staged component. Prepare a multi-component change serially; retain
only bounded validated plain init results in the one deployment candidate, not
extra VMs or parallel staging supervisors. Recheck budgets before retaining each
result. Immutable source memory shared with an in-flight old M5 job remains
charged until that bounded job is retired; include it in the report inventory.

A staged candidate includes exact loaded artifacts, effective typed values,
base deployment revision, affected object revisions, dependency closure, diff,
prepared reset states, conservative Recorder charge and an expiry. It contains
no OutputLease/authority or mutable Runtime references. Stage does not switch
active sources, update latest observations, or evict committed signal history.

Rejected validation can update the bounded *operation diagnostic* and record a
rejection fact; zero active mutation means unchanged committed configuration,
sources, revisions, generations, samples, controller state/authority and transport
work as a consequence of that candidate. Normal independent ticks may continue.
Tests compare deterministic checkpoints with those independent ticks controlled.

## 4. Identity and diff classification

Keep these identities separate; all counters are checked, never wrap:

| Identity | Advances when |
| --- | --- |
| Deployment revision | A different loaded TOML/effective deployment is committed, including byte-only changes |
| Loaded-artifact identity | Exact content changes; SHA-256 over bytes, plus kind and encoding |
| Logical resource ID/key | Stable across reconnect/rebind; changing it is topology change |
| Binding generation | COM handle/binding replacement or rebind invalidates an old physical association |
| M3 resource generation | Existing executor recovery boundary; not a substitute for binding generation |
| Instrument/component/model generation | Restart/reinitialize/replacement invalidates prior observations/jobs |
| Controller/Reference/mapping revision | Existing authoritative configuration changes; unrelated changes do not spuriously reset them |
| Output epoch/attempt identity | Existing revoke/arm/safe lifecycle; never derived from configuration revision |
| Recorder activation identity | A committed loaded-state baseline, including script reload/restart baselines when TOML revision is unchanged |

The diff is a set of effects, not one mutually exclusive enum. Show each changed
field, old/new bounded values or hashes, affected IDs, required effects, and the
aggregate barrier scope. A mixed candidate executes the union atomically.

| Change | Mandatory classification and result |
| --- | --- |
| Display label, TOML comments/format only | Live-safe; new exact-byte identity/revision, no controller/model reset |
| Read-only polling cadence | Ordinary live; advance schedule from commit time, no missed-tick burst; bounds/freshness dependencies revalidated |
| PID gains/Reference parameters, controller cadence/freshness/lease | Pause/re-warm + output-safe barrier; preserve accepted explicit operation rules; no implicit bumpless retune claim |
| Native model parameters or managed plain config | Reinitialize + dependent controller stop/re-warm + affected output barrier |
| COM name/settings, address, definition/scaling/mapping | Rebind/reinitialize + invalidate observations + dependent re-warm and barrier where applicable |
| Safe target/range/evidence policy | Barrier under the **old** profile, then install new profile while disarmed; do not relax evidence to pass the barrier |
| Managed file content at unchanged declaration | Changed only by managed-script reload, not silently by config reload |
| Managed source pathname/manifest | Config reload may stage that new declaration/source with M5 init and reinitialize/barrier effects |
| Runtime key, object ID/key/kind, graph add/remove/reconnect, server listener, Recorder enabled/path/policy, hard bounds | Restart required in M8; reject live apply before any barrier |

Fixed topology for live reload is deliberate. Startup supports bounded declarative
graphs of the listed existing variants; M8 is not arbitrary hot graph editing.
Definition bytes are loaded for config staging because their semantics configure
bindings. For unchanged managed declarations, reuse the active frozen source;
do not incidentally reload modified script files. Script reload reads only the
selected committed manifest paths; emulator restart rereads no file.

Reject stale base revisions or affected-object mutations at apply. During isolated
preparation, existing work continues; at the barrier fence conflicting mutation
commands return Conflict/Busy. Unrelated work remains allowed unless the Required
Recorder barrier expands the scope as specified below. Safety, Stop and Shutdown
always take priority. Duplicate M6 request IDs retain existing dedup semantics;
client disconnect does not cancel accepted Runtime-owned lifecycle work.

## 5. Operation lifecycle and commit point

Expose distinct M6-style asynchronous public operations and bounded snapshots:

- `stage_configuration`: load the pinned startup path and expose candidate/diff;
- `apply_configuration`: apply a candidate ID with expected base revision;
- `reload_configuration`: explicit stage-and-apply convenience, same state machine;
- `reload_managed_scripts`: selected existing IDs (<=8), expected generations;
- `restart_virtual_model`: one existing native/M5 model ID and expected generation;
- `reconnect_resource`: explicit existing resource ID and expected binding generation.

Only trusted startup composition chooses the deployment path. Wire calls cannot
request arbitrary filesystem paths, bytecode, raw serial bytes or safe evidence.
Read-only configuration/resource/operation Queries return committed snapshots and
bounded diffs; they never load files, enumerate ports, poll I/O or refresh devices.
Public snapshots expose hashes and bounded settings, not source contents/secrets.
Use existing M6 operation retention, request/frame, event and dedup bounds.

```text
Loading -> Validating -> Preparing -> Staged
                                      |
                              revalidate / reserve
                                      v
Quiescing -> AwaitingSafe -> PreparingBinding -> AwaitingProvenance
                                      |
                              owner atomic commit
                                      v
                         AwaitingDurability -> Completed
```

Skip unnecessary stages, but expose the commit distinction. Terminal outcomes:
Rejected/Expired/Conflict (no candidate activation), FailedBeforeCommit (old
configuration retained, safety side effects remain), Applied (active and durable
when recording), AppliedRecordingFailed (active, recording failed; Required
faulted), or AbortedByShutdown. Never label a staged/prepared candidate Applied.

After staging, the apply deadline is 10 s monotonic; each suboperation has its
smaller existing deadline (safe profile, M5, M3, 2-s COM open, M7 health/flush).
No deadline is extended by retries, new events, wall-clock changes or client
polling. Expiry closes the operation honestly; it does not automatically restore
an old handle, source, state or authority. A subsequent explicit operation may
recover a stopped/faulted deployment after prerequisites have been re-established.

1. Revalidate revisions, candidate lifetime, complete graph constraints, counter
   increments and Recorder budgets. Prepare every fallible in-memory replacement
   and reserve bounded slots before active mutation. No sequential public command
   loop pretending to be atomic application.
2. Fence affected ordinary production/admission. Stop dependent controllers in
   both Warming and Running; revoke native/manual owners and fence queued work.
   Request safe using the current profile and existing trusted dispatch path.
   Continue safety polling and lease service without blocking on workers.
3. Await the old profile's required evidence for the current attempt/epoch/binding,
   with prior in-flight ambiguity settled. ACK is not readback; readback is not
   measured physical effect. A timeout/disconnect is never proof of safety.
4. Prepare replacement bindings only after old work is stopped/fenced. A handle
   may need to close before its replacement can open. This is an irreversible
   external side effect before domain commit, reported separately from config.
   A failed open leaves the old configuration selected but the resource Offline;
   never lie that its old binding is still online or auto-reopen it as rollback.
5. Complete the Recorder protocol in section 7. At one owner turn, revalidate
   guards again and install the prepared bounded domain update set and schedule.
   This is the only configuration commit point. Counters, sources, descriptors,
   dependent invalidation and revision facts become visible coherently. No I/O,
   Lua, hashing or recoverably fallible construction occurs inside this commit.
6. Confirm lifecycle durability when enabled before releasing the recorded
   dependent producer fence. Reset components/models re-warm; affected controllers
   remain Stopped/Paused/Ready as their existing lifecycle permits, without owner,
   lease or ordinary output authority. Deliberate user Start/Arm is still required.

Barrier failure is **not rollback of safety**: retain old committed configuration
and source identity, retain revocation/stop/fault and actual evidence, record the
failed operation, and reject the candidate. Never restore an armed epoch, lease,
old queued proposal or old controller Running state. Unrelated healthy resources
may continue. If the candidate's physical effect is uncertain, report it as such.
Failure after the atomic commit never rewrites the active identity back to old.

Shutdown aborts uncommitted candidates, invalidates pending jobs, and uses normal
M6/M7 safe/flush service. Already committed changes are included in the final
snapshot; their durability may be unknown. No lifecycle operation defers shutdown
until its 10-s deadline or waits for the loader/COM/SQLite worker to join.

## 6. Managed reload and model restart

Managed reload uses M5 bounded syntax/manifest validation and isolated init before
the safety barrier. Invalid source/init preserves the old committed generation
and ongoing work where M5 permits it. A barrier failure can still stop/fault that
old generation's dependents; preserving source does not mean restoring authority.
For a multi-ID selection, all sources/init results must validate before any source
is installed. Commit all selected changes together, reset plain state and warm-up,
advance generations and retire old pending invocation correlations.

Restart uses already loaded immutable model parameters/source, resets native model
state or runs bounded M5 init, advances model generation even if content is equal,
and invalidates dependent transforms and controller inputs. Reset PID timing/
integral state through the accepted controller re-warm lifecycle, not by feeding a
fabricated dt/sample. References stop/progress according to their explicit existing
pause/restart semantics; no implicit continuation of a previously running procedure.

Old samples stay in bounded/durable history with their original identity. Current
latest validity/warm-up cannot consume them as new-generation Good observations.
Check boot, logical object, generation, binding, mapping/config revision and job/
transaction correlation as relevant at submission **and** completion. Drop late
old results from current state and evidence; bounded ignored-result diagnostics
may retain their original correlation. Bytes already accepted cannot be retracted.

## 7. Recorder, provenance and Required reload safety

Preserve the M7 database/boot/run/interval/activation identities, sequence ordering,
WAL/FULL policy, storage bounds, history pagination and crash unknown tails. A
reload does not restart the Runtime, create a fresh boot, restart a run, clear a
gap, reset health age or make an old interval complete.

Freeze one loaded-artifact bundle at each activation: exact TOML bytes/hash,
effective validated native/config values, strict-JSON definition bytes/hash,
actual managed source bytes/hash, active bindings, generations and safety profiles.
Store kind `runtime_toml`, UTF-8 encoding and SHA-256, plus canonical effective
manifest separately. A path is metadata, never content identity. Overwrite/delete
the files after staging in acceptance: activation and a later StartRecording must
still use the frozen bytes. A script-only reload retains TOML identity and changes
source/activation identity; restart retains content identities and changes model
generation/baseline. Byte-only TOML edits still get a new loaded activation.

Use the existing M7 versioned schema and configuration/provenance/object snapshot
tables; add versioned lifecycle fact payloads using the existing record encoding.
Do not change schema version 1's meaning or silently migrate an unsupported
database. `commit_activation` already stores multiple immutable activations; its
worker's startup-only admission is not a live-reload API. Extend that admission
through explicit bounded lifecycle messages/receipts, retaining startup behavior.
If implementation discovers a required incompatible schema change, stop for review
with the exact missing representation rather than weakening reopen compatibility.

Lifecycle facts use a version-1 typed payload with operation ID, operation kind,
phase/outcome, base and committed deployment revision, activation identity/root,
loaded TOML hash, affected logical IDs with old/new generations/revisions, reason
and monotonic commit time. Split per-object facts only as members of the same
atomic apply group. Preserve old payload decoding unchanged; new tags have
explicit bounded decoders. Existing object/configuration snapshots carry full
effective values; facts link to that immutable baseline, not a mutable path.
No dependent measurement needs a new history index: its existing generation/
revision plus ordered activation/configuration facts resolve the binding. Test
that reconstruction, including two activations with the same TOML hash.

The required ordering is:

1. Quiesce the affected producers, and drain previously admitted groups by polling
   receipts. Record old-revision stop/revoke/safe facts under their actual identity.
2. A storage-worker preparation transaction may store deduplicated content blobs;
   it must not create an active configuration/object baseline or publish an active
   receipt yet. Unreferenced prepared content after crash is not an activation.
3. Once preparation is confirmed and lifecycle ingress credit is reserved, commit
   the domain change on the owner. Queue the immutable activation baseline and its
   actual applied revision facts in one ordered lifecycle transaction. SQL inserts
   configuration/object rows, facts and durable checkpoint atomically **after**
   the domain commit, never a fabricated future successful configuration.
4. Hold dependent publication/ordinary work until the cumulative receipt confirms
   that exact activation and applied sequence. Older/future/wrong-generation
   receipts cannot release it. All subsequent dependent facts link to this loaded
   baseline and the actual committed revision. No mixed old/new attribution.

Only one lifecycle payload is retained, within M7's 1 MiB +256 KiB provenance/
boundary budget. The object baseline, apply facts and boundary metadata together
fit that single 256-KiB boundary budget, not 256 KiB each; the ordinary apply facts
also count against ordinary ingress until their receipt. Reject a candidate whose
combined charge cannot fit. Share/transfer immutable ownership between host and
worker; do not retain a second encoded full bundle as an undocumented queue. A prepared blob
receipt does not renew Required health unless it meets M7's real durable-progress
rules; preparing, enqueuing and successful parsing are never durable progress.

For Required, any lifecycle needing quiescence uses a global recording barrier:
all recording-dependent ordinary controller/manual admission is closed, all
owners in that expanded scope are stopped/revoked/safed, and acquisition is
temporarily suspended after its current bounded unit. This intentionally expands the dependency scope shown
in the staged diff; do not surprise callers with an undisclosed global pause.
Live-safe updates may avoid the global pause only if the complete bounded update
and provenance transaction have reserved credit and no dependent fact can pass
their ordering fence. An implementation may conservatively classify such an
update as a recording barrier, but must report that effect before apply.

The ordinary gate is the conjunction of the unchanged M7 health gate and the
pending activation fence. No toggling Required to BestEffort/Disabled, opening a
new unrecorded interval, transiently resuming ordinary work, fake probe receipt
or extending the 2-s health age. M7 failure still faults Warming/Running controllers,
revokes manual/native authority and rejects queued ordinary M3 work immediately;
safe service never waits for Recorder. Resume acquisition only after the activation
receipt; resume stopped controllers only by a later deliberate user action.

If storage fails before owner commit, reject activation, retain old config and
any completed safety side effects. If it fails after commit, report active new
config plus recording failure; Required remains faulted, BestEffort preserves the
actual domain result and visible gap/unknown tail. BestEffort's healthy unrelated
work need not be globally stopped. If recording is disabled, retain the same
bounded frozen baseline in memory; a later recording start persists the current
loaded state, not files reread from disk. If an enabled Recorder is already failed,
new reload admission fails before mutation; that does not change existing
BestEffort experiment outcomes. No automatic rearm on later Recorder recovery.

### 7.1 Fixed ingress accounting, not larger queues

Keep M7's Core outbox 256 facts/256 KiB, individual ordinary record 64 KiB,
ordinary ingress 1024 records/4 MiB including worker-held work, **four outstanding
groups**, atomic group 256 records/512 KiB, SQL batch 256 records/512 KiB, and
the separate single provenance/boundary transaction. Keep 100-ms batch age,
250-ms durable probe, 2-s Required health and 2-s final flush. The reserved
failure/seal channel is not ordinary reload capacity.

Before any barrier, compute a conservative charge for every synchronous commit
unit: configuration facts, per-object generation/config facts, invalidations,
controller/output lifecycle, and host diagnostics/projections. Charge capacities,
bounded encoding overhead and the full causal burst, not just changed fields or
one sample. The atomic owner apply unit must fit BOTH the Core outbox's remaining
256/256-KiB budget and a single capture group's 256/512-KiB budget. The baseline
content travels in the dedicated lifecycle transaction, not a giant ordinary fact.
Reject `LifecycleBudgetExceeded` during staging if this cannot be proven; do not
split one supposedly atomic domain commit into partially admitted groups.

Use a bounded precommit ingress reservation for one complete apply group. It
counts against the same four groups/1024 records/4 MiB from reservation until a
validated receipt releases it; there is no fifth hidden group. With four credits
busy, wait asynchronously BEFORE mutation while continuing safety service; honor
deadlines/Required age. At most one reservation exists, and expiry/failure releases
it exactly once. Subsequent producers cannot steal its credit. Drain older Core
facts first. Retain only charge metadata, not another outbox. Lifecycle safety
steps before commit use existing bounded units and normal ingress; overflow still
fails recording and Required safely, never suppresses a safe transition.

Acceptance must exercise largest admitted deployment/diff, mixed reload/rebind,
all four credits occupied, delayed SQL/probe, safe/revocation bursts, and exactly
over-budget candidates. Report measured worst-case counts/bytes and a static
bound for each unit; tests cannot pace an unbounded native burst into compliance.

## 8. COM ownership and evidence boundary

Implement a host `ByteTransport` adapter backed by one fixed worker per resource.
Prefer the safe API of `serialport` with Windows support, pin/record its resolved
version, and keep workspace `unsafe_code = forbid`. The crate has a blocking
Read/Write interface and RAII close; neither belongs on the Runtime owner lane.
See [serialport primary documentation](https://docs.rs/serialport/latest/serialport/)
and [project documentation](https://github.com/serialport/serialport-rs).
Dependency-internal unsafe code is not permission to add raw Win32 unsafe to Core.

M3 owns ordering, one active transaction, the 32-entry ordinary queue, deadlines,
read-only retry decision, response framing/validation and resource lifecycle.
The worker owns only the COM handle, configured OS settings, a bounded byte
request/response and a cancellation generation. It cannot schedule measurements,
retry frames, parse a temperature, modify Runtime or produce ACK/safe evidence.

One adapter mailbox request and one completion, each <=38 payload bytes plus
fixed correlation/status; RX/TX retained buffers <=38 bytes each. At most one
OS call per worker iteration; no unbounded reader/event thread. A prefix retained
in any mailbox/buffer is charged once or twice explicitly if actually copied;
the maximum per slot is four 38-byte buffers plus fixed metadata. Extra bytes
or framing surplus fail the current exchange and require recovery, not buffer
growth. One fixed cumulative status cell, error text <=256 bytes, and one
coalesced stop/cancel cell remain available even when the data mailbox is full.

`try_write` promptly accepts only a bounded prefix into the current-generation
adapter buffer or returns zero/error; it never waits for COM. Positive acceptance
is the M3 send-start boundary: bytes may reach the device thereafter, **not**
proof they have already left the driver. Record acceptance separately from known
OS byte counts; failure afterward is conservatively post-acceptance/ambiguous.
Zero means no bytes accepted by that attempt. The worker cannot replay or forward
bytes under a new binding generation. `try_read` only drains current-generation
completed bytes; `try_recover` only polls an already initiated bounded recovery.

This boundary is acceptable for the explicitly read-only physical capability.
Reject output intents, generic writes, output descriptors and raw-byte API paths
before admission; test this at config, Core registration/operation and host adapter
composition boundaries. The worker's I/O write sends a read request, not an actuator
write command. Do not claim M8 proves a physical final-authority-before-byte path.

## 9. COM timeouts, recovery, rebind and shutdown

Open/configure occurs on the resource worker after validation and lifecycle
authorization. After setting the chosen serial properties, read them back where
supported; mismatch/unsupported configuration is a named failed activation.
Opening a handle cannot be treated as a successful probe or fresh sample.

Set separate finite read/write call timeouts immediately before those calls (a
shared timeout API is acceptable because one worker serializes calls). Do not use
`read_exact`, `write_all`, an unbounded `flush` or an internal retry loop. Partial
OS writes advance the offset of the **same** request; they do not restart its
prefix. Partial reads accumulate only within the fixed expected frame bound.
Timeout/zero-progress is pending only while the original transaction deadline
and generation still permit progress. An error after acceptance is not retried
as a fresh write. Only the existing M3 whitelist may request its at-most-one
read-only retry after verified recovery, within the original total deadline.

Windows driver behavior is not a hard real-time guarantee: serial timeout fields
govern I/O, and cancellation alone does not mean an outstanding operation has
completed. See Microsoft's [COMMTIMEOUTS](https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-commtimeouts)
and [CancelIoEx](https://learn.microsoft.com/en-us/windows/win32/api/ioapiset/nf-ioapiset-cancelioex).
The design consequence is host watchdog isolation, not a claim that any USB driver
honors its timeout. Late open/results are rejected after deadline; worker closes
its late handle. A hung worker consumes its fixed quarantined slot until process
exit. Do not start another worker against that resource/port or accumulate detached
threads to recover availability. Failed open never makes a resource Online.

States exposed to clients: Unopened, Opening, Online, Recovering, Offline,
Closing, Quarantined, with logical ID, configured/bound port, binding/resource
generations, active operation, last monotonic progress/error and sample freshness.
Each transition is owner-committed; Queries never interrogate the driver.
Optional port enumeration is a separate explicit host operation, <=64 names and
<=8 KiB result, one request/result and 2-s deadline using the loader slot. It
reports truncation/errors and never opens, selects, reconnects or rebinds a port.

On disconnect/timeout, stop admitting new transactions, fence pending reads and
mark observations unavailable/stale through the normal Core path. Recovery has
the configured finite deadline, then Offline. No automatic rebind based on COM
number, USB enumeration, matching channel type, address, CRC or a response value.

**No transaction ID exists in this Metakon profile.** Purging a host buffer,
sleeping for an arbitrary quiet interval, reopening the same COM name, or rejecting
an old worker completion does not prove that a delayed old response cannot arrive
on the new handle. `RecoveryStatus::Complete` requires an actual clean-boundary
basis. M8's default for an uncertain physical exchange is Offline, not Complete.

For the real bench, reconnect requires explicit external reconciliation: retire
old worker I/O/handle, establish a device/bus reset boundary that clears pending
responses (following applicable device procedure), reconnect/open, and run the
read-only compatibility probe. Document the performed reset, device/adapter and
why old responses cannot survive it in the hardware report. An operator report
is labeled `operator_reported_reset`, not Rust-observed output-safe evidence or
device identity. The deployment must not use an unsubstantiated reset/quiet-time
claim to return Complete. If the device's reset behavior cannot be established,
remain Offline and leave the reconnect hardware gate incomplete. A verified
device-specific response-lifetime bound could support a later reviewed recovery
profile; M8 does not invent that bound or make it an arbitrary TOML tuning knob.

The public reconnect request starts this lifecycle; it cannot submit ACK, readback,
safe completion or a raw recovery-success boolean. Trusted host composition may
accept the explicit bench operator reconciliation as a documented external
precondition for a new **read-only** session, not fabricate it on the client's
behalf. No safety-critical controller may treat that claim as physical evidence.
Tests must distinguish a clean scripted boundary from unproven purge/reopen.

Rebind keeps logical resource identity but invalidates queued work, active result
correlations, affected samples and binding generation before replacement I/O can
be observed. Every worker request/result carries a session generation; reject a
late open/read/write/error from the retired one. Only one worker/handle owns a
port; rebind waits asynchronously for clean retirement within its deadline or
fails Offline/Quarantined. Never reuse a hung slot or attach old bytes to a new
configuration. Probe success opens acquisition, not output authority.

Shutdown first fences acquisition and ordinary admission, then services accepted
safe obligations on existing eligible resources, cancels/retires COM work, captures
the final real state and requests M7 flush. COM retirement has a 2-s monotonic
deadline concurrent with normal host shutdown service; Recorder keeps its own
2-s deadline from its final safety/cleanup snapshot. Report independent COM close,
safe-evidence and Recorder flush results. Never join a worker that has not finished.
A stuck driver or writer yields finite unsuccessful process exit with honest
unknown/failed state, not a successful clean close/flush. Test both orderings of
COM versus Recorder completion and both workers stalled simultaneously.

## 10. Measurements and public inspection

Validated physical responses publish through the existing Instrument/Signal
path, with real transport origin, original observation monotonic time, quality,
unit, logical instrument/resource and binding/config/mapping correlation. Host
wall-clock anchors remain separate; clock changes never change deadlines or sample
ordering. A read request acceptance, OS write completion or channel-type probe is
not a temperature observation. Unavailable sentinel, CRC/address/type/range errors,
timeouts and disconnects do not produce invented Good samples or repeat last value
as newly observed. Old actual samples remain queryable with age/quality/history.

Recorder captures physical observations and failure/recovery/configuration facts
through the same bounded fact path as virtual measurements. Use M7 bounded indexed
history operations and frozen cursors; no ad-hoc SQL or file I/O in a Query. Local
SQLite inspection after shutdown is an additional acceptance oracle, not the
Runtime application's query boundary. Extend Babashka helpers only for the new
public operations/status/history workflow; keep Babashka optional at startup.

## 11. Tests-first C1-C20 acceptance contract

The release-plan IDs below are retained, not replaced by a weaker software-only
list. Each row is required; subcases in its oracle are conjunctive. SOL_HIGH adds
a failing behavioral test before each implementation increment, records the actual
red/green sequence and names in `MILESTONE_8_REPORT.md`, and honestly identifies
any implementation that already preceded its coverage. No tests are added or run
as M8 implementation during this design phase.

| ID | Test stimulus and required independent oracle |
| --- | --- |
| C1 | Load native, managed and read-only physical TOML fixtures twice/from different working directories and reordered tables. Equal effective graph/explicit IDs/schedules; exact bytes have distinct correct hashes when formatting changes. Startup stopped/disarmed, Required not ready before durable baseline. Real example files parse, native finite/demo compatibility remains. |
| C2 | Malformed, oversized, unsafe, missing-file and invalid-init candidates against an active deterministic Runtime. No candidate-caused change to config, sources, generations, samples, authority, queue or open-call counter. Rejection diagnostics only; no Lua/COM/SQLite effects in pure validation. Include stage expiry and stale revision conflict. |
| C3 | Unknown fields at every table/variant, duplicate keys/tables/IDs/keys, invalid UTF-8/BOM, excessive nesting/value count and numeric overflow. Bounded rejection, no permissive ignored settings or allocation beyond loader bound. |
| C4 | Missing resource/signal/Reference/output, mismatched units/ranges, cycles/unsupported M5 graph, invalid safe profiles, illegal physical output binding, duplicate normalized ports, excessive graph/burst budgets. All reject before activation/open; largest admitted graph succeeds. |
| C5 | Live label/byte-only/cadence reload plus mixed diff and restart-required fields. Exactly one coherent config revision/activation; no unnecessary model generation or autoarm; cadence has no catch-up burst. Mutate/delete source path after stage and before later recording start: frozen exact TOML/definition/source bytes persist. Unsupported live topology/Recorder changes reject unchanged. |
| C6 | Change model/PID/safe profile/binding with Warming/Running controllers, manual owner and queued M3 work. Check union dependency scope, stop/revoke and old-profile safe evidence before commit; no ordinary bytes cross fence; all replacements visible at one commit. Required delayed activation receipt holds admission; no unrecorded control interval. |
| C7 | Fail safe evidence, hold partial old I/O, fail new COM open, fail prepared provenance and fail postcommit SQL separately. Before commit: old config, honest stopped/faulted/Offline state. After commit: new active identity with explicit recording failure. Never restore old armed authority, claim failed-before-send after possible I/O, or create a durable activation for an uncommitted candidate. Crash at each provenance/commit boundary and reopen unknown tails honestly. |
| C8 | Complete/fail each config, scripts, restart and reconnect lifecycle; reconnect client and Recorder. No restored lease/epoch, implicit Start/Arm or queued old dispatch. A later deliberate Start still requires current Good inputs, re-warm and Required health. Test manual and native owners. |
| C9 | Valid single/multi-component script reload, delayed old result and partial init failure. New frozen sources/plain initial state/generations commit together only after all prepare; warm-up resets; old jobs ignored; TOML hash unchanged. Two-worker/one-stage M5 bounds remain. |
| C10 | Syntax, manifest, init timeout, memory/state limits, wrong units and quarantined worker. Invalid candidate preserves allowed old generation/source; failure after barrier preserves stop/revoke side effects, never rearms. No third Lua worker or uncontrolled candidate cache. |
| C11 | Restart native and managed models with dependent transforms/PID, held old observation/completion and unchanged source files. Exactly one new model generation, reset state/timing, old data not current Good, sources not reread, controller remains stopped pending re-warm/explicit Start. |
| C12 | Windows adapter tests cover actual platform build, setting validation/mismatch, busy/missing port, slow/late/hung open and close. Real bench also records actual open/configure/close/settings. Owner progress independent of worker; slot quarantine bounded; no silent parameter fallback. |
| C13 | Byte-adapter fault harness returns zero, every prefix split, partial read/write, timeout before/after acceptance, surplus/CRC failure, stalled driver and late completion. Preserve M3 deadlines/correlation/retry cap; continue only same frame offset, no hidden fresh write retry; unknown send/effect remains unknown. Purge alone never completes uncertain recovery. |
| C14 | Disconnect in each I/O phase; explicit reconnect with new session, and failed/unproven reconciliation. Stop Good publication, expose Recovering/Offline, resume only after justified boundary/probe; no auto rebind. Deterministic software evidence and actual disconnect/reconnect evidence are separately labeled. |
| C15 | Rebind same logical bus to different port/settings/address/definition with queued/in-flight work and late old handle result. Stable logical IDs, advanced binding/config generations, invalidated dependent samples; stale bytes/results/errors cannot mutate new binding. Close/open failure retains honest old-config-but-offline state. |
| C16 | **Hardware required:** actual Windows host + actual Metakon + actual COM adapter, compatible read-only probe and repeated real temperature/status reads through public API/Babashka; inspect transmitted protocol operations and confirm no actuator writes. Document cable/device removal, explicit failure, justified reset/reconnect and resumed real observations. A fake/loopback transport cannot satisfy this row. |
| C17 | **Hardware required:** record C16 observations in actual SQLite, cleanly reopen, and compare boot/run/Signal/value/unit/quality/time/binding/revision and exact loaded TOML/source hashes with API/Babashka history. Inspect real disconnect/recovery and reload boundaries; missing samples remain missing. Compare pre/post-reconnect real measurements, not fabricated expected temperature. |
| C18 | Real process A/B and native read-only acquisition tests: no GUI, kill/disconnect Babashka A, keep Runtime and Recorder acquiring, B reconciles/query history then performs explicit lifecycle operation. Native acquisition/control safety continue independently; absence of bb executable is not startup failure. Include this absence/reconnect observation in the real bench record. |
| C19 | **Software and hardware required:** finite shutdown with actual COM and Recorder active, no pending handle/thread/write or false successful flush; reopen valid history. Separate process tests stall COM, SQLite and both, interrupt reload/rebind and test unresolved old I/O: finite failure, unknown tail/evidence, no unbounded join. Only a real device run establishes the clean hardware subgate. |
| C20 | Complete M1-M7 regression/final gates below plus all C1-C19 software tests. Saturate all four Recorder groups during reload/rebind/revocation, maximal atomic facts and over-limit candidate; unchanged bounds, Required fails closed and BestEffort preserves true domain outcome. Verify bounded snapshots/history/wire, UTC jumps, dedup/disconnect, restart-required classification, process crash/reopen and no donor changes. |

Suggested behavioral test modules: `configuration_validation`,
`configuration_reload`, `managed_script_reload`, `model_restart`,
`configuration_provenance`, `recorder_reload_budget`, `windows_com_transport`,
`resource_rebind`, `physical_measurement_recording`, `com_recorder_shutdown`.
Tests are independent; names do not impose execution order. Unit/fault tests use
injected clocks/I/O, not wall-clock sleeps to hide races. Hardware tests are
explicit opt-in and must never default to a fake if a port/device is missing.

## 12. Implementation order and final gates after handoff

Implement small logical tests-first increments: pure loader/schema (C1-C4), bounded
stage/diff/atomic lifecycle (C5-C8), scripts/model fencing (C9-C11), provenance and
Recorder burst/fault/crash cases, COM adapter/fault tests (C12-C15), public API/
Babashka process acceptance (C18), then the actual hardware bench (C16/C17/C19).
Add no M9 concepts. Keep documentation changes separate from behavioral commits.
Source/module/rustdoc/comments/examples must be English with meaningful ownership,
time, failure, boundedness and shutdown documentation; retain missing-docs checks.

Final software gate on the latest implementation HEAD, in PowerShell:

```powershell
git status --short
git log --oneline -20
cargo test --workspace
cargo test --workspace --release
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
$m8PreviousRustdocFlags = $env:RUSTDOCFLAGS
try {
    $env:RUSTDOCFLAGS = '-D warnings'
    cargo doc --workspace --no-deps
} finally {
    $env:RUSTDOCFLAGS = $m8PreviousRustdocFlags
}
cargo run -p lab-runtime
Push-Location clients/babashka
try {
    bb test-client
} finally {
    Pop-Location
}
git diff --check
```

Run each gate with exit-code checking; the block is a command inventory, not a
script that aggregates failures automatically. List named tests and actual
resolved dependencies. Require the existing actual M6/M7
Babashka process tests and recording A-kill/B-reconnect/Pause/Stop/Shutdown/reopen
oracle, plus configuration-enabled reload/provenance/reopen process acceptance.
Do not count an environment-skipped Babashka test as execution evidence.

### Real hardware gate checklist

Record date, implementation commit, Windows/driver/USB-serial/device model and
firmware if available, wiring and serial settings, protocol definition/hash,
exact TOML hash, command log, public operation/history results, reset-boundary
basis, SQLite path/hash after close, relevant rows and shutdown exit result.
Keep source/private machine metadata out of public logs as appropriate. The donor
is knowledge only; never modify it or make it a workspace dependency.

1. Have the user connect the real Metakon on the declared COM port. Establish the
   read-only bench and a documented clean starting boundary. Do not issue register
   writes, output tests or implicit control-line reset commands.
2. Start the configuration-driven Runtime with recording enabled (Required for
   the required-recording bench). Inspect loaded identities and resource status.
   Obtain several real validated readings in normal Signal and history queries.
3. Disconnect the actual cable/device. Observe explicit failure and no fabricated
   Good samples. Reconnect using the documented external reconciliation/reset
   procedure and explicit Runtime reconnect operation; observe resumed measurements
   with the same logical ID and new binding generation.
4. Inspect via the actual public API and Babashka; disconnect the client and confirm
   acquisition persists, reconnect and reconcile. Exercise a live-safe config
   revision and verify exact provenance before/after; no actuator command involved.
5. Cleanly shut down COM and Recorder, reopen SQLite and verify real observations,
   lifecycle facts, identities, ordering, coverage and durable final state.

If any physical device, Windows COM access, justified reset boundary, actual
Babashka run or operator step is unavailable, report precisely what was not run.
M8 may finish software implementation but remains **incomplete at the hardware
acceptance gate**. Set `M8_HARDWARE_ACCEPTANCE_PENDING` in the handoff and stop for
the missing bench input; do not substitute fake evidence for C16/C17/C19 and do
not proceed to M9. Software success is not whole-milestone acceptance.

Once every software and real-hardware requirement passes, finalize the M8 report,
set `STATUS: READY_FOR_EXTERNAL_REVIEW` in `ai/HANDOFF.md`, and stop for external
review. No physical actuator safety, power-loss certification, remote-security,
GUI or long-soak claim follows from this milestone.

## 13. Design-phase handoff

This design phase changed documentation only. It did not open COM, execute device
commands, add dependencies/production/tests, rerun the accepted M7 test suite or
claim new hardware evidence. The donor path was unavailable on this computer;
the checked-in M3 contract/protocol knowledge was used, with no donor modification.
The implementation report must distinguish planned tests from actual execution.

MODEL HANDOFF

STOP HERE.

Switch:

ASTRA_HIGH -> SOL_HIGH

Resume with:

M8 tests-first implementation against MILESTONE_8_DESIGN.md.

The user must explicitly authorize/cross this gate. Do not start M9.
