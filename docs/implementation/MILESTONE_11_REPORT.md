# Milestone 11 implementation report

## M11.6 — Application, client, emulator, and process pressure hardening

### Status and scope

```text
M8-M10: ACCEPTED
M11.1-M11.6: COMPLETE
M11.7: NOT STARTED
M12+: NOT AUTHORIZED
```

M11.6 re-audited the real TCP/NDJSON path and consolidated the accepted M9B.8
pressure model. No production state machine, public operation, capability, error
category, capacity, scheduling rule, Recorder/SQLite contract, controller or
OutputAuthority behavior changed. The implementation change is test-only: malformed
traffic coverage was expanded and one socket oracle was synchronized against actual
reactor admission instead of TCP-connect timing. No COM port or hardware was used.

The governing process invariants remain:

```text
bad / slow / malformed / dead / noisy client != bad / slow / dead experiment
client lifetime != experiment lifetime
Application pressure cannot wait inside authoritative native work
```

### Connection, framing, and session boundaries

The reactor owns exactly one nonblocking socket thread. It never borrows Runtime and
communicates with the owner over two capacity-64 synchronous mailboxes. The owner
services Runtime before it drains at most sixteen inbound messages and before it
dispatches a rotating slice of at most four client queues. Native safety,
acquisition, controller and Recorder work therefore retain their accepted priority;
perfect client fairness is not promised.

The complete Application/process bounds are:

| Structure | Owner | Capacity | Full / invalid behavior | Release / recovery | Principal regression |
|---|---|---:|---|---|---|
| Active TCP peers | reactor | 8, including detach generations awaiting owner delivery | New peer is closed; existing peers continue | Exact detach delivery | 8-peer plus 16 replacement-cycle reactor oracle |
| Inbound NDJSON frame | peer | 16,384 bytes including LF; 16 nesting levels; 1,024 lexical values; 512-byte strings | Bounded rejection for a complete malformed frame or close at the hard byte bound | Socket close | real UTF-8/JSON/shape/DTO/oversize matrix |
| Owner/reactor mailboxes | reactor/owner | 64 each | Nonblocking pause, scoped detach, or terminal owner failure according to direction | Receiver progress/detach | mailbox saturation and stale-generation tests |
| Admitted requests | peer plus owner queue | 8 per client | Read pauses; no further allocation | Reply consumption/detach | bounded-peer and M9B.8 suites |
| Reply/event queues | peer | 8 replies / 16 events, including current write | Only affected peer detaches | Detach | nonreader flood oracle |
| Socket work | peer | 8 KiB per sweep; four read/write units | Work resumes at the exact byte offset | Later reactor sweep | partial-write unit oracle |
| Hello/partial/write deadline | peer | 2 seconds from the fixed boundary | Only affected peer closes | Reconnect | real partial-frame trickler |
| Session scopes | Application | 16 process-wide | `capacity_exhausted` | 1,800-second detached TTL/terminal eviction | request deduplication |
| Pending mutations | Application | 8/scope, 64 process-wide | Existing `busy`/`capacity_exhausted` mapping | Terminal outcome | session/global capacity oracle |
| Retained outcomes | Application | 32/scope, 256 process-wide; 4 KiB each | Old terminal records evict; evicted IDs remain unknown, never reusable | TTL/eviction | request deduplication |
| Frozen projection | Application | one per connection; 64 records/page, 8 KiB encoded | Structured bounded-page failure | Page completion, 5-second expiry, detach | discovery/current projection suites |
| Subscription | Application | one/client; 8 kinds, 16 targets | `subscription_busy`/bounded validation error | Unsubscribe, gap, detach | subscription recovery |
| Event replay | Host event log | 1,024 records, 4 KiB each; scan 32/offer 4 per owner turn | Old cursor receives `event_gap` with `resync_required` | Current/history snapshot plus fresh subscribe | M9B.8 gap/resync |
| Recent history | Runtime signal | configured ring; query maximum 128 | Oldest attempts evict | Normal signal publication | measurement API |
| Durable history | Recorder/Application | 8 jobs, 8 pages, 8 cursors; 128 rows/8 KiB page | `history_busy`, bounded page failure | Terminal result, page/cursor expiry, detach cancellation | recorder history API |
| Configuration staging | Service/Application | one candidate, 32 overlays | Existing busy/capacity/revision errors; atomic rejection | Apply, replacement, 30-second expiry | configuration/resource API |
| Emulator publication | Runtime/Application | one scalar per request within session bounds | Validation or existing capacity error; no buffering | Synchronous terminal outcome | emulator and M9B.8 pressure |

All maps and queues above are either directly capped or keyed by the eight live
connections, sixteen scopes, or eight durable-history jobs. Detached history jobs
are drained through the bounded Recorder job capacity; their late results are
discarded by job/correlation fencing. No client-controlled unbounded map or lazy
stream was found.

### Malformed, slow, and disconnected clients

The real loopback server now exercises invalid UTF-8, invalid and empty JSON,
unknown envelope fields, an invalid nested DTO, and an unterminated frame at the
hard size limit. A complete malformed frame may receive one bounded rejection and
then closes; the hard-size frame closes without allocating beyond the fixed input
buffer. A separate healthy client remains usable after every case. Diagnostics log
only the fixed code, connection identity and byte count, never the arbitrary frame.

The existing slow-writer oracle fixes the deadline at the first partial byte and
continues successful work through a healthy peer until the trickler closes. The
slow-reader oracle fills the sixteen-event queue with maximum-size frames and proves
scoped detach, exact stale-connection fencing, and reuse by a new connection. Socket
writes are nonblocking and limited per sweep; neither client can wait in the Runtime
owner. Process shutdown stops acceptance, completes authoritative shutdown first,
then gives terminal delivery a fixed 200-ms best-effort window before stopping and
joining the cooperative reactor thread.

### Subscription, replay, deduplication, and client death

One aggregate subscription per client scans at most 32 retained events and offers
at most four per owner turn. Falling behind the 1,024-record ring removes the
subscription and returns `event_gap` with `resync_required`; it never fabricates a
contiguous sequence. Recovery remains an explicit current snapshot, optional recent
or durable history, and a fresh subscription at the current boot/cursor. Detach
removes connection-local subscription, projection, page and cursor state, so none
can enter a reused socket generation.

Mutation deduplication remains scope plus consecutive sequence, not `msg_id`.
Identical retained requests replay the terminal outcome without executing twice;
conflicting payloads fail, pending work is never evicted, and an evicted sequence is
unknown rather than reusable. Disconnect fences reply delivery but does not roll
back an accepted Runtime mutation. A later attachment to the same process-local
scope can observe or replay its retained terminal result. Connection-local `msg_id`
duplicates close only that peer and a completed exchange releases its ID.

### History, configuration, emulator, and multi-client pressure

Runtime recent history remains distinct from Recorder durable history. Durable jobs,
pages and continuation cursors share the accepted eight-slot bound; detach cancels
or orphans the exact job, drops its page/cursor, and late completion cannot publish
into reused client capacity. SQLite busy and worker failure remain bounded Recorder
results and never block the owner.

Repeated configuration/property queries, invalid edits and revision conflicts use
the existing one-candidate/32-overlay lifecycle. Rejection is atomic and cannot grow
another staging structure. Ordinary queries do not create overlays.

The M9B.8 pressure oracle alternates 600 Good/Unavailable virtual publications from
independent scopes while current queries and native thermal scheduling progress.
Stale generations, nonfinite/range-invalid values and publication to the native
thermal instrument fail through existing categories. Virtual publication can create
only a normal virtual measurement; it cannot fabricate physical observation, Ready,
ACK, readback or output authority. The virtual sample continues through generic
current/history/subscription/Recorder paths, including the durable-history oracle.

Combined coverage uses malformed, slow subscription, history, emulator and normal
query/mutation clients. Isolation is bounded rather than perfectly fair: one peer's
backpressure closes that peer, while owner-first service, committed measurement
generations, controller progress, Recorder prefixes and transport/safety completions
remain authoritative progress predicates. Held-SQLite and managed-worker pressure
oracles from M11.2-M11.4 remain part of this composition.

### Socket-timeout investigation

The single M11.5 full-suite timeout occurred in
`nonreading_event_flood_detaches_and_stale_slot_frames_cannot_reach_reused_capacity`.
The test assumed that successful `TcpStream::connect` meant the nonblocking reactor
had already inserted `connection=1`, then immediately injected synthetic events for
that ID. Under compile/full-suite load the reactor could legitimately process those
events first and discard them as stale. No event queue was filled, so the test's
one-second wait for `Detach(1)` timed out. This was a **test harness race**, not a
production socket or backpressure defect.

The oracle now writes a valid hello and waits until the owner channel observes the
exact `Incoming::Request(1, ...)` admission predicate before flooding. The adjacent
gap-delivery test uses the same barrier. No timeout was lengthened. The corrected
flood oracle passed 20 focused repetitions; the new capacity/churn oracle passed 10,
and the real malformed matrix passed 10.

### Startup, shutdown, diagnostics, and contract freeze

Existing startup tests prove invalid configuration fails before listener binding,
occupied loopback bind unwinds without readiness, and transport/Recorder startup
failures do not claim a half-ready service. Existing process tests cover accepted
`runtime_shutdown`, other-client fencing, clients and Recorder work in flight, and
finite truthful terminal cleanup. Shutdown acceptance is not a claim that sockets,
transports or Recorder have already retired; the terminal result carries that proof.

M11.5 diagnostics now cover malformed/oversized input, admission capacity, reply/
event backpressure, server start and terminal shutdown. These records remain lossy,
bounded and observational. Client traffic never gates on successful logging.

The public contract is frozen at 42 operations, 25 capabilities, protocol version
one and the existing twelve error categories. No WebSocket, HTTP, remote access,
authentication, client SDK or convenience operation was introduced. M11.7 has not
started.

### Verification

The final candidate passed formatting, debug and release workspace suites,
warning-denied Clippy and warning-denied rustdoc. Focused M9B.8, client isolation,
subscription/replay, request deduplication, durable history/cursor, emulator and
Runtime-shutdown suites also passed. The corrected nonreader oracle passed 20
focused repetitions, exact client-capacity/churn passed 10, and the real malformed
matrix passed 10. Full workspace gates also include M11.2 acquisition/managed-worker,
M11.3 output recovery, M11.4 Recorder/crash, M11.5 logging and M9D physical-output
software regressions. No production defect or capacity change was required.

## M11.5 — bounded diagnostic logging

### Status and scope

```text
M8-M10: ACCEPTED
M11.1-M11.4: COMPLETE
M11.5: COMPLETE
M11.6: NOT STARTED
M12+: NOT AUTHORIZED
```

M11.5 adds one process-wide, bounded, best-effort troubleshooting path. It does not
change Runtime authority, the 42-operation Application contract, Recorder policy or
schema, scheduler order, transport behavior, or OutputAuthority. No COM port or
hardware was used.

The implementation uses `tracing`, `tracing-subscriber` and `tracing-appender`.
`tracing-appender` owns the one background writer and its bounded lossy channel;
`diagnostics.rs` adds record capping, size rotation, failure fallback and bounded
shutdown observation. No custom async logging state machine, new Runtime worker
lifecycle, remote backend, telemetry service or metrics surface was introduced.

### Contract and Recorder boundary

```text
Runtime / Service / adapters
  -> observational tracing event
  -> 8 KiB capped formatter record
  -> 1,024-record lossy try-send queue
  -> library-owned diagnostic worker
  -> stderr + bounded rotating text files

Runtime semantic fact
  -> Recorder bounded ingress
  -> SQLite durable scientific/audit history
```

Logging success never gates a state transition, Recorder receipt, safety work,
transport completion or Application result. Diagnostic records are not experiment
evidence and never enter SQLite. Existing controller, configuration, reconnect,
output and Recorder semantic facts remain unchanged.

The standard levels are:

- `ERROR`: process/subsystem failure requiring attention;
- `WARN`: bounded abnormal or recoverable conditions, including ambiguity,
  backpressure and disconnect/failure states;
- `INFO`: low-frequency startup, readiness, reconnect, Recorder and shutdown
  lifecycle;
- `DEBUG`: client and output state transitions useful during development;
- `TRACE`: available to the facade but unused for ordinary measurement, poll, PID,
  SQL or raw-frame streams.

The default threshold is `INFO`. `LAB_RUNTIME_LOG_LEVEL` accepts exactly `ERROR`,
`WARN`, `INFO`, `DEBUG` or `TRACE`; an invalid value emits one bounded stderr
warning and falls back to `INFO`. No Application operation configures logs.

### Destination and strict storage bounds

On Windows the default directory is:

```text
%LOCALAPPDATA%\lab-runtime\logs
```

`LAB_RUNTIME_LOG_DIRECTORY` may select another process-start directory. Without
`LOCALAPPDATA`, `XDG_STATE_HOME/lab-runtime/logs` is used when available, otherwise
the OS temporary directory is the fallback. Logs are not written beside the
executable or into experiment archives.

The active file is `lab-runtime.log`; rotations are `lab-runtime.log.1` through
`.3`. Rotation occurs before a write that would cross 4 MiB. The active file plus
three rotations therefore have a strict generated-data bound of 16 MiB. One record
is capped at 8 KiB with an explicit `[truncated]` suffix. Lines contain RFC 3339 UTC
time, process-monotonic milliseconds, level, thread/target, a stable event name and
bounded fields.

The producer queue contains at most 1,024 complete records. Producers use lossy
`try_send`; a full or disconnected queue drops diagnostics rather than blocking
authoritative work. The drop counter produces a bounded overflow line when the sink
next progresses and one stderr summary at shutdown. Diagnostic completeness is
explicitly **BEST EFFORT**.

### Destination failure and shutdown

Directory creation, open, rotation or later write failure permanently disables the
file side for that process and reports one bounded `diagnostic_file_unavailable`
warning directly to stderr. The background sink continues its stderr mirror. There
is no recursive retry through the failed queue and no interaction with Required
Recorder policy. A full log disk cannot masquerade as Recorder failure.

Normal exit first freezes the existing authoritative Runtime/Recorder result,
closes diagnostic admission and gives admitted records 250 ms to drain. A drained
writer uses `tracing-appender`'s bounded shutdown handshake (100 ms enqueue plus
1,000 ms acknowledgement maximum). If the destination is stuck, the handle is
detached after the observation window rather than joined. Tail diagnostics may be
lost, but cannot extend Runtime shutdown indefinitely or change its exit status.
The panic hook emits bounded context and invokes the previous hook; it does not
claim panic recovery.

### Subsystem coverage

Low-frequency structured events cover:

- package/protocol/API identity, selected profile, readiness and process result;
- server listen, client accept/detach, malformed/oversized frames, client capacity
  and reply/event backpressure without copying arbitrary request payloads;
- reconnect request, replacement COM readiness/open failure and completion;
- fresh physical-measurement transport loss/recovery, without every sample/poll;
- controller failure and output safe obligation, ambiguity/readback failure, safe
  readback confirmation and general output state at `DEBUG`;
- Recorder archive-open failure, sticky worker/storage failure, lifecycle,
  coverage and clean/incomplete shutdown;
- shutdown request, fatal escalation and truthful terminal safe/worker/transport/
  Recorder status.

Output text preserves:

```text
requested != send_started != ACK != readback != physical effect
```

A safe readback line says the register readback was verified and explicitly does
not claim independent physical effect. An ambiguous safe obligation says it is not
permission to blindly resend.

### Diagnostic failure matrix contribution

| Failure | Detection | Runtime / Recorder consequence | Diagnostic consequence | Recovery | Guarantee / regression |
|---|---|---|---|---|---|
| Directory/file unavailable | Sink open/create result | None; both continue | File disabled; one stderr warning | Correct path/permissions and restart | **BEST EFFORT**; unit and child-process oracles |
| File write/rotation fails | Background sink I/O result | None | Sticky file disable; no recursive retry | Correct storage and restart | **BEST EFFORT** by construction |
| Queue full | Lossy `try_send` counter | None; producer never waits | Record dropped; cumulative bounded summary | Automatic when writer progresses | **BEST EFFORT**; held-writer/native-progress oracle |
| Record exceeds 8 KiB | Capped formatter | None | Tail truncated with marker | Emit smaller fields | **GUARANTEED** size bound |
| Worker/destination stalls | 250-ms drain observation expires | None; authoritative result already frozen | Tail may be lost; guard detached | Process restart | Runtime liveness **GUARANTEED**; flush **BEST EFFORT** |
| Panic | Installed hook observes context | No recovery claimed | One bounded event attempted | Process restart | Delivery **BEST EFFORT** |

### Verification and limitations

Focused tests prove file creation, lifecycle entries, level filtering, truncation,
rotate-before-write, retained-file/total-byte bounds, file failure fallback,
Recorder independence, lossy overflow, native Runtime progress under a held writer,
finite completion, malformed-client payload non-disclosure and precise output
ambiguity wording.

The final M11.5 candidate passed `cargo fmt --all -- --check`, debug and release
workspace suites, warning-denied Clippy and warning-denied rustdoc. The release
`lab-runtime` library suite was additionally repeated five times after isolating
the bounded-sink contract into one test; all five runs passed. Full workspace suites
include the accepted M11.2 acquisition/managed-worker, M11.3 output-recovery, M11.4
Recorder/crash, M9B.8 process-fault, M9D physical-output, shutdown and exact
42-operation/25-capability registry regressions.

The log is intentionally not complete, transactional, remotely shipped or
cross-process coordinated. One active process per configured directory is normal;
a conflicting writer can make rotation fail and therefore falls back safely to
stderr. Raw frames, arbitrary client JSON, every measurement, PID tick, successful
poll, SQL statement and Recorder fact are absent at ordinary levels. Longer
rotation/file-growth soak belongs to M11.7. M11.6 has not started.

## M11.4 — Recorder, SQLite, crash and filesystem hardening

### Status and scope

```text
M8-M10: ACCEPTED
M11.1-M11.3: COMPLETE
M11.4: COMPLETE
M11.5: NOT STARTED
M12+: NOT AUTHORIZED
```

M11.4 closed the focused storage-acceptance gaps identified by M11.1. It added
deterministic external-lock and Windows read-only startup oracles and made offline
integrity, WAL recovery and lifecycle completeness explicit in the clean-close and
real process-kill tests. No production Rust, SQLite schema, transaction policy,
Application operation or public DTO changed.

### Frozen durability contract

The authoritative boundary remains:

```text
Runtime semantic facts
-> Host admission
-> bounded RecorderWorker ingress
-> SQLite worker transaction
-> durable receipt / committed watermark
```

The concrete guarantees are:

- Recorder open validates the application ID, schema version, required tables,
  indexes, encoding and bounded file settings before creating the serving boot;
- start becomes `Recording` only after the start/provenance/boundary transaction is
  committed and its worker receipt is observed;
- successful ingress reserves bounded credit and assigns FIFO identities, but does
  not claim that the facts are durable;
- only a successful worker receipt advances the committed prefix and releases the
  corresponding ingress credit;
- stop is ordered after all admitted FIFO work and becomes `Idle` only after the
  interval and run are sealed in a committed transaction;
- process finish separately commits the boot seal and then closes SQLite; a sealed
  boot does not by itself fabricate a successful connection close;
- a storage error makes Recorder failure sticky. The already committed prefix
  remains authoritative, while an unconfirmed tail becomes a known gap where that
  can be durably represented or conservatively `unknown_tail` otherwise;
- process death has no graceful-stop guarantee. On validated reopen, active boot,
  run and interval rows become `interrupted`, and run/interval coverage becomes
  `unknown_tail` under a new boot identity.

Therefore:

```text
fact admitted != fact durably committed
archive structurally readable != experiment semantically complete
```

### Storage failure matrix

| Failure | Detection | Recorder state | Runtime consequence | Durability consequence | API consequence | Recovery | Guarantee / regression |
|---|---|---|---|---|---|---|---|
| Missing parent or path is a directory | SQLite open error during process startup | No attached Ready worker and no active run | Serving never becomes ready for either policy | No database or half-started boot is claimed | Bounded startup error | Correct the configured path and restart | **GUARANTEED**; `explicit_unopenable_storage_fails_startup_before_readiness_for_both_policies` |
| Windows file has the read-only attribute | Real SQLite open/initialization fails | No Ready worker | Required and BestEffort startup both fail before readiness | Existing main-file bytes and sole sealed boot remain unchanged | Bounded startup error, no raw SQL contract | Restore write access and restart | **GUARANTEED on Windows**; `readonly_windows_archive_fails_startup_for_both_policies_without_mutation` |
| Foreign, corrupt, future or structurally incomplete archive | Pre-WAL compatibility validation fails | No serving boot | No experiment starts against the file | Existing bytes/schema are not repaired or rewritten | Bounded incompatible-storage failure | Supply a compatible archive or new path | **GUARANTEED**; transaction/reopen compatibility suite |
| External writer holds `BEGIN EXCLUSIVE` | Configured 100-ms SQLite busy timeout expires | Open fails; no second owner | Runtime never sees a usable Recorder | No second boot or lifecycle mutation | Bounded startup failure | Release the external owner, then open in a new process | **GUARANTEED**; `external_sqlite_writer_lock_fails_within_the_busy_bound_without_starting_a_boot` |
| Start/provenance transaction fails | Atomic SQL/commit error before start receipt | Starting becomes Failed or start is rejected | Required authority stays closed; BestEffort never gains Recorder-owned authority | No successful run boundary or partial provenance claim | Failed operation/status | Correct storage and start a new worker/process | **GUARANTEED** by lifecycle, provenance and transaction tests |
| Ingress credit exhausted | Pre-send group/record/byte accounting rejects admission | Sticky Failed; reserved gap path is attempted | Required trips control/output; BestEffort native work may continue | Prefix stays fixed; loss is a gap or `unknown_tail` | Failed status with bounded counts and coverage | New worker/process after correcting pressure | **GUARANTEED** by backpressure/failure suites |
| Fact/operation transaction or deferred constraint fails | SQLite statement or commit fails | Failed; worker terminates | Required fails closed; BestEffort does not transfer experiment authority to storage | Whole transaction rolls back; checkpoint cannot pass it | `recording_failed`, not a successful receipt | New worker/process; prior prefix remains readable | **GUARANTEED** by transaction/failure suites |
| WAL threshold checkpoint fails | Required checkpoint reports busy/error or injected exact fault | Failed | Policy split applies without owner blocking | Last confirmed checkpoint/prefix remains; no fake success | Failed status preserves bounded storage health | Reopen after storage correction | **GUARANTEED** by startup/checkpoint tests |
| Storage worker panics | Owner observes a finished thread without its terminal receipt | Failed/closed worker, pending credit remains honestly unconfirmed | Required fails controller/output; BestEffort acquisition/control remains independent | Only prior committed prefix is authoritative | Failed status reports the bounded worker failure | Process restart | **GUARANTEED** by worker-panic, Required and isolation tests |
| Stop/seal transaction fails | Terminal SQL/commit error | Failed, not Idle | Required cannot claim complete coverage | Run/interval remain unsealed or interrupted on reopen | Stop cannot report durable success | Reopen for inspection; use a new process for recording | **GUARANTEED** by shutdown transaction tests |
| Boot seal commits but close fails | Close fault after real terminal transaction | Failed; `worker_closed`/flush success is false | Process terminal result is unsuccessful | Seal may be durable, but successful close is not fabricated | Shutdown reports no successful Recorder flush | Inspect/reopen; restart | **GUARANTEED** by close-after-seal regression |
| Process killed before fact commit | OS process termination at the pre-commit barrier | No owner transition can finish | No shutdown or safe-effect claim | Transaction is absent; committed prefix remains | Next open exposes interrupted/unknown-tail history | SQLite recovery plus a new boot | **GUARANTEED** by process-reopen oracle and offline integrity check |
| Process killed after commit before receipt | OS termination at the post-commit barrier | Old owner receipt is unknowable | No control operation is replayed and no authority is restored | Nonempty WAL contains the committed row/checkpoint | Durable history exposes row under interrupted/unknown-tail run | SQLite recovery plus a new boot | **GUARANTEED** by process-reopen oracle and offline integrity check |
| History job/cursor pressure or locked worker | Bounded job/cursor admission, cancellation and TTL | Recording lifecycle is unchanged | Acquisition, control and Recorder admission remain independent | Queries never expose an uncommitted page as durable | `history_busy`, cancellation or bounded failure | Consume, cancel, disconnect or wait for expiry | **GUARANTEED** by history API/isolation suites |

An actual full disk is not created in tests. Exhaustion semantics are exercised
deterministically by the checked one-GiB `max_page_count`, five-percent reserve,
small test quota, WAL checkpoint fault and real insert/commit faults. The guarantee
is that a write which cannot commit never receives durable acknowledgement; the
exact Windows error code from a physical full volume remains **BEST EFFORT**.

### Required and BestEffort policies

Recorder policy remains a Runtime safety policy, not a SQLite setting:

- `Required` blocks controller start until the start boundary is durable. Admission
  failure, writer death or a missed two-second durable-progress deadline closes the
  recording gate, fails active control, revokes leases and drives output safety;
- `BestEffort` uses the same bounded worker and honest Failed/coverage state, but a
  Recorder failure does not acquire authority to stop otherwise valid native
  acquisition or control;
- both policies reject an unusable configured archive before process readiness;
- neither policy turns accepted ingress into a durable claim before a receipt.

Existing `recorder_required`, `recorder_isolation`, `recorder_failure` and
`recorder_backpressure` tests prove this split. M11.4 did not introduce another
recording policy or recovery mechanism.

### Clean close, process crash and WAL/SHM

The paired acceptance is now explicit:

**Clean path**

```text
start -> admit facts -> stop/drain -> seal run and interval
-> finish/seal boot -> close -> offline integrity check
```

The clean shutdown oracle requires `PRAGMA integrity_check = ok`, one sealed boot,
one sealed complete run, one sealed complete interval, successful Recorder flush
and no active `-wal` or `-shm` sidecar after the worker connection closes.

**Crash path**

```text
start -> commit known prefix -> kill before owner receipt
-> nonempty WAL remains -> offline integrity check
-> lifecycle is still active/recording and therefore unsealed
-> validated product reopen -> interrupted + unknown_tail
```

The crash fixture deliberately proves that rows may be structurally readable and
their local coverage field may contain `complete` while the boot/run/interval state
is still active/recording. Such an archive is not semantically complete. Normal
SQLite WAL recovery is used; M11.4 does not delete sidecars or copy only the main
file. After product reopen and clean close, the committed prefix remains readable,
the old lifecycle is interrupted/unknown-tail, and `integrity_check` remains `ok`.

### Gaps, sealing and completeness

| Archive condition | Structural readability | Lifecycle | Coverage interpretation |
|---|---|---|---|
| Clean complete run | `integrity_check = ok` | Boot, run and interval sealed | `complete` |
| Explicit admitted-loss gap | Readable committed prefix and gap row | Failed/sealed according to the terminal path | `gap`; never complete |
| Storage failure without a durable gap seal | Prior prefix readable | Failed, then interrupted on reopen if unfinished | `unknown_tail` |
| Kill before commit | SQLite rolls back/omits the open transaction | Active before recovery; interrupted after reopen | `unknown_tail` |
| Kill after commit before receipt | Committed WAL prefix is readable | Active before recovery; interrupted after reopen | `unknown_tail` despite the surviving row |
| Unsealed interval or run | May pass SQLite integrity | Recording/starting/stopping, then interrupted on reopen | Not semantically complete |

Successful `recording_stop` continues to mean that accepted facts drained and the
interval/run seal transaction committed. It does not mean the storage thread was
destroyed or that the later process boot seal has completed.

### Provenance and durable history under faults

No provenance format changed. Start/boundary provenance remains atomic; component
activation, configuration revision and native build identity continue to use the
same generic content/identity records. The compatibility roles
`managed_lua_source`, `managed_component_source`, old operation facts and historical
`source_hash` data remain readable and were not rewritten.

Durable history still reads only committed SQLite state. It pages active recording,
clean sealed runs, interrupted crash archives and explicit gaps through the same
bounded query layer. Frozen checkpoints, page/cursor bounds, cancellation,
disconnect release and `history_busy` behavior are unchanged. A failed Recorder can
leave its committed prefix readable; it cannot promote pending ingress into durable
history.

### Platform and bounds notes

- Missing-parent, directory-path, incompatible-file, lock and integrity tests are
  portable SQLite/filesystem oracles used on the current Windows target as well;
- the read-only-file oracle is explicitly `cfg(windows)` and exercises the Windows
  file attribute rather than claiming POSIX permission behavior;
- the physical disk-full class is test-emulated with SQLite quota/checkpoint/write
  faults; no test attempts to fill the host disk;
- clean sidecar absence is required only after the accepted clean close. A killed
  process may legitimately leave WAL/SHM and those files are part of the archive;
- Recorder ingress remains four groups, 1,024 records and four MiB total, with the
  existing per-group/record limits; history remains eight jobs with bounded page
  sizes/cursors; the main database remains capped at one GiB with its five-percent
  reserve, and WAL checking remains at sixteen MiB. No bound was increased.

### Remaining soak/manual gaps

M11.4 does not claim evidence for multi-day archive growth, millions of samples,
physical volume exhaustion, repeated thousands of start/stop cycles or power loss
during an OS/filesystem cache flush. Short deterministic batching, quota, repeated
process-kill/reopen, transaction rollback, history pressure and clean-shutdown
oracles cover the same invariants without making default `cargo test` unbounded.
Long Recorder growth remains an opt-in M11.7 soak concern.

### M11.4 verification

The focused Recorder integration set passed, including startup/open, transaction,
backpressure, Required/BestEffort isolation, provenance, history, process reopen,
shutdown/process-shutdown, WAL/checkpoint and WriterBarrier tests. The new exact
oracles passed on Windows:

```text
external SQLite writer lock and recovery                    PASS
Windows read-only startup under Required and BestEffort     PASS
pre-commit process kill + offline integrity                 PASS
post-commit WAL process kill + offline integrity/reopen     PASS
clean seal/close + integrity + absent WAL/SHM               PASS
```

The new startup/process/shutdown oracles passed three consecutive focused
repetitions. One unrelated socket replay-gap unit test timed out once during an
intermediate quiet workspace rerun; it then passed three immediate isolated
repetitions and both final complete debug and release runs. No M11.4 file was
involved in that test.

Full gates passed:

```text
cargo fmt --all -- --check                                PASS
cargo test --workspace                                    PASS
cargo test --workspace --release                          PASS
cargo clippy --workspace --all-targets -- -D warnings     PASS
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps PASS
git diff --check                                          PASS
```

The exact 42-operation/25-capability registry, M9B.8 fault acceptance, M11.2
configured acquisition faults, M11.3 output recovery, M9D physical-output software
suite and all Recorder integration binaries remained green. No hardware test was
run, COM5 was not opened, and accepted evidence was not modified.

## M11.3 — controller, OutputAuthority and physical-output recovery hardening

### Status and scope

```text
M8-M10: ACCEPTED
M11.1-M11.2: COMPLETE
M11.3: COMPLETE
M11.4: COMPLETE
M11.5: NOT STARTED
M12+: NOT AUTHORIZED
```

M11.3 consolidated the accepted controller/OutputAuthority/Metakon rules into one
recovery model and added the missing configured end-to-end oracle. It did not
redesign OutputAuthority, add a retry policy, alter Metakon bytes, add an
Application operation, change Recorder/SQLite, or use hardware.

### Consolidated recovery model

The accepted sequence is now tested as one policy rather than only as isolated
state-machine properties:

```text
normal physical control
-> send-started output or resource fault
-> old lease/epoch/generation revoked
-> controller Failed
-> safe obligation retained
-> explicit resource reconnect and compatibility probe
-> fresh OutputAuthority instance and confirmed safe readback
-> controller still Failed
-> fresh Good input still does not rearm
-> explicit reset_failed
-> Paused
-> explicit resume and warm-up
-> fresh lease on the current authority/generation
-> normal physical output may resume
```

Transport recovery and input recovery are evidence, not output permission:

```text
resource Ready != input Good != controller armed != output authorized
```

`reset_failed` is accepted only after the current authority is safely Disarmed. It
clears algorithm memory and moves `Failed -> Paused`; it does not acquire a lease.
`resume` is the separate deliberate rearm action and enters Warming. Only accepted
warm-up on distinct fresh observations acquires a new finite automatic lease.

### Physical WRITE fault matrix

| Fault point | Authoritative result | Automatic resend | Recovery |
|---|---|---|---|
| Rejected or failed before `send_started` | No physical-byte claim; reservation is released | Ordinary intent is not resurrected. An unstarted safe obligation may use the existing later-delivery policy | Later bounded safety service or explicit lifecycle correction |
| Nonzero WRITE started, no valid ACK | Outcome is Ambiguous after the bounded recovery boundary; lease is revoked, controller becomes Failed and a distinct safe obligation is requested | **No** retry of the ambiguous WRITE | Establish a clean/rebind boundary and safe evidence, then explicit controller recovery |
| ACK succeeds, readback is unavailable/invalid | ACK remains visible, readback is absent, failure is Unavailable, outcome is Ambiguous, controller fails | **No** blind nonzero resend | Safe transition; reconnect if the resource cannot recover; explicit rearm |
| ACK succeeds, readback mismatches | Reported value and Mismatch remain visible, outcome is Failed, controller fails | **No** repeat of the original nonzero WRITE. Existing policy may issue a distinct safe WRITE because the readback positively reports a non-safe register value | Matching safe readback, then explicit controller recovery |
| ACK and matching readback | Outcome is ReadbackVerified; a current controller lease may renew | No duplicate WRITE | Normal bounded control continues |
| Safe WRITE becomes ambiguous after start | `safe_obligation_pending=true`, `ambiguous_safe_resend_blocked=true`, `safe_confirmed=false`, FaultLatched | **No** automatic safe resend, including repeated safety turns or another safe request | A validated rebind creates a fresh authority/safe-establishment lifecycle; it still does not rearm the controller |

The distinctions remain normative:

```text
requested
!= authorized
!= send_started
!= ACK
!= readback
!= physical_effect
```

Matching register readback is software evidence about reg06, not proof of heater or
other physical effect.

### Configured resource-loss and no-auto-rearm oracle

`tests/configured_physical_output.rs` now drives the output-enabled frozen deployment
through `HostCore`, the same configured reconnect primitives used by
`ServiceHost::reconnect_resource`, and the real Runtime/ResourceExecutor/Metakon
path. Its byte adapter is test-only.

The oracle proves:

1. startup probe, safe-zero ACK/readback and controller preparation succeed;
2. the controller obtains a finite lease and produces a verified nonzero output;
3. the next nonzero WRITE is applied by the fake device, but its ACK is suppressed
   and transport recovery fails;
4. the resource becomes Offline, output is fault-latched with an unconfirmed safe
   obligation, and the controller is Failed with no lease;
5. the old-generation ambiguous nonzero frame is never sent again;
6. explicit retirement/rebind advances binding generation 1 to 2, then the
   compatibility probe and a new authority-gated safe-zero ACK/readback succeed;
7. ordinary acquisition resumes and publishes a fresh Good measurement, but
   multiple owner turns produce no nonzero WRITE and the controller stays Failed;
8. only `reset_failed` followed by `resume` and normal warm-up can produce another
   nonzero WRITE; the resulting lease has a different authority-instance identity
   from the retired lease.

Existing `ServiceHost::reconnect_resource` tests continue to freeze the surrounding
operation lifecycle: old-worker retirement, actual-open Ready, compatibility probe,
Core generation fence, durable lifecycle commit and acquisition release. The new
oracle exercises the output-enabled host half deterministically without opening an
OS COM port.

### Scoped controller-fault correction

The new oracle found one production defect in `host/scheduler.rs`. Runtime already
made an unsuccessful scheduled physical tick fail closed: it set the controller to
`Failed`, revoked its lease, requested safe output and emitted controller/output
facts. Host then propagated the returned tick error as an owner failure. In a real
process that could unnecessarily escalate a correctly contained controller/output
fault into fatal service shutdown.

Host now consumes a tick error only after querying the authoritative Runtime state
and proving that the same controller is `Failed`. That is the existing scoped
domain transition. If Runtime did not reach `Failed`—for example because of a clock,
configuration or owner invariant error—Host still propagates the error. Scheduler
phase order, cadence and controller semantics are unchanged.

### Lease, epoch and generation fencing

The new configured oracle proves a fresh authority instance after rebind. Existing
M2/M3/M4 regressions continue to prove that:

- lease expiry is exclusive and immediately prevents renewal/final send;
- revocation advances the epoch and removes pending ordinary authority;
- a queued stale proposal fails the final authority check before byte zero;
- a binding/mapping generation replacement fences old queued work and completion;
- a late old dispatch cannot confirm a newer safe epoch;
- reconnect never reconstructs an old controller lease.

No reconnect, elapsed time, fresh measurement, ACK or readback implicitly performs
controller rearm.

### Timing faults, shutdown and visibility

Existing controller suites cover unavailable and stale input, exact lease expiry,
excessive tick gap, invalid algorithm/configuration results and unavailable output.
All converge on Failed control plus the existing safe transition; recovery remains
`safe evidence -> reset_failed -> resume`.

Existing shutdown acceptance covers active and warming control, ambiguous/unconfirmed
physical safety, offline/recovering transports, `TransportShutdown::Pending`,
Recorder flush/failure and unfinished managed workers. Shutdown continues bounded
owner turns, never reports safe merely because zero was requested, and returns an
unsuccessful terminal status when physical ambiguity or cleanup remains. It does not
hide an unresolved safe obligation.

No new durable fact or schema was required. Existing facts retain controller
lifecycle and correlated output Requested, Authorized, SendStarted, ACK, readback,
mismatch, TransportUncertain, Ambiguous, SafeRequested and safe evidence stages.
The existing Application projection exposes resource state/generation, controller
state/lease absence, and output state, fault latch, safe confirmation, in-flight,
ACK/readback/failure/outcome. A client can therefore distinguish “resource
recovered” from “controller not rearmed” and “safe confirmed/unconfirmed” without
private Runtime fields or a new operation.

### Verification

Concurrency/safety-sensitive oracles were repeated independently:

```text
configured ambiguous-output -> reconnect -> no-auto-rearm   10/10 PASS
ambiguous safe WRITE -> no automatic resend                 10/10 PASS
```

Focused suites passed for M2/M3 transport fencing, M4 controller lifecycle and
timing, M9D physical output, configured acquisition/reconnect/output, Host
scheduling, controller API projections, Recorder evidence/isolation/provenance and
finite shutdown.

Full gates passed:

```text
cargo fmt --all -- --check                                 PASS
cargo test --workspace                                     PASS
cargo test --workspace --release                           PASS
cargo clippy --workspace --all-targets -- -D warnings      PASS
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps PASS
```

Both workspace matrices retained the exact 42-operation/25-capability registry,
M9B.8 fault acceptance, M11.2 configured acquisition matrix, all Recorder suites
and the M9D output matrix. No hardware test ran. The accepted M9D evidence archive
and hash remain unchanged.

## M11.2 — acquisition, managed-executor lifecycle and starvation hardening

### Status at M11.2 completion and scope

```text
M8-M10: ACCEPTED
M11.1 hardening audit: COMPLETE
M11.2: COMPLETE
M11.3: NOT STARTED
M12+: NOT AUTHORIZED
```

M11.2 consolidated configured physical-acquisition fault oracles, corrected one
managed-executor lifecycle defect, and froze the existing required-work progress
guarantees. It did not change the Application contract, scheduling order, reconnect
policy, Metakon protocol, OutputAuthority, Recorder/SQLite, configuration semantics,
or any hardware definition. No COM port or hardware was used.

### Configured physical-acquisition matrix

The new matrix in `tests/configured_physical.rs` exercises the real configured
composition seam:

```text
FrozenDeployment
-> HostCore::configured_with_transports
-> configured Metakon binding and schedule
-> ResourceExecutor
-> typed Metakon decode
-> Runtime signal/resource state
-> generic Application discovery/current projection
```

The injected `ByteTransport` is test-only. It replaces only the blocking OS byte
device; it does not bypass deployment parsing, instrument composition, scheduling,
the resource executor, protocol decoder, Runtime commit, or Application projection.

| Case | Runtime/resource result | Retry/recovery classification | Public/Recorder consequence |
|---|---|---|---|
| Valid compatibility and temperature response | Probe becomes ready; finite Good value commits at generation 1 | Normal periodic acquisition | Generic current/discovery projection is Good; existing Recorder integration persists the same semantic fact |
| Bad temperature CRC | No corrupt value commits; one `Unavailable(Transport)` becomes current; resource reaches `Offline`, generation remains 1 and queue is empty | No retry through a production-COM-style `Pending` recovery boundary; explicit reconnect is required | Generic Application current is unavailable/transport and discovery is offline; the common unavailable-fact path is covered durably by `recorder_quality` |
| Short/truncated temperature frame | Deadline produces the same finite unavailable/offline result; later scripted frame is not consumed | No hidden retry or frame carry-over; explicit reconnect | Same generic projection and Recorder fact semantics |
| Silence | Transaction deadline terminalizes finitely; no pending request or queue growth remains | No unbounded retries; explicit reconnect | Same generic projection and Recorder fact semantics |
| Disconnect | Exactly one current unavailable transition for the failed logical read; recovery reaches offline at its fixed deadline | Accepted M8 explicit reconnect policy remains unchanged | Same generic projection and durable unavailable semantics |
| Wrong channel type | Compatibility fails; replacement remains quiesced/offline and ordinary acquisition never releases | Correct configuration and another explicit reconnect | Structured bounded failure; no physical output authority |
| Bad-CRC or truncated compatibility response | No false compatibility success, no Good temperature, no ordinary temperature READ, offline resource | Explicit corrected reconnect | Signal stays never-observed or unavailable; both are truthful, and no output is configured or authorized |
| Stale old-generation completion | Correlation/generation fence discards it | Current generation continues; no retry from stale work | No public or durable false fact |

For bad CRC, short frame, silence, and disconnect, the test also leaves a valid later
temperature frame queued and proves it is not consumed. This freezes framing and
transaction isolation: malformed work cannot leak bytes into a next logical READ or
fabricate `33.3 degC`. Service through two monotonic seconds leaves exactly three
writes (probe, initial Good read, failed read), an empty executor queue, generation
1, and no owner wait.

The generic `ResourceExecutor` still permits at most one retry for an explicitly
retryable READ only after an adapter proves a clean recovery within the original
deadline. Production `ComTransport` does not claim such a boundary for a faulted
session; elapsed time alone is not recovery proof. Physical WRITE retry semantics
were not touched.

### Recorder and Application consequences

All covered acquisition failures converge on the existing Runtime semantic sample:

```text
SampleQuality::Unavailable
+ MeasurementFailure::Transport
+ current binding generation
```

The configured matrix verifies the unchanged `measurements_current` and discovery
resource projections for every ordinary-read fault class. `recorder_quality` proves
that this same Runtime fact is durably ordered after the preceding Good sample and
reopens without a fabricated value. Recorder does not branch on CRC versus silence
versus disconnect; low-level cause detail remains diagnostic work deferred to M11.5.
No new public operation, DTO, error category, fact type, or SQLite column was added.

### Managed-worker death defect and correction

Before M11.2, `ManagedExecutor::try_poll` handled
`TryRecvError::Disconnected` exactly like `Empty`. If an implementation panicked,
the worker thread and completion sender died, but its `Busy` correlation stayed in
the slot. Runtime eventually failed the affected component at the independent
100-ms deadline, while the dead slot continued to masquerade as permanently pending
capacity. Two dead workers therefore caused later submission to report `Busy`
instead of terminal executor loss.

Each of the two fixed slots now has an explicit internal lifecycle:

```text
Alive
  idle or one pending correlation

Dead
  no job sender
  no pending correlation
  never selected for admission
  never respawned
```

Completion-channel disconnect atomically marks the slot Dead and, if a job was
pending, returns exactly one correlated, untimely
`Err(ComponentError::Executor)`. Runtime's existing validation path scopes that
failure to the affected component and invalidates its signal. Repeated polling is
finite and produces no duplicate completion. If one worker survives, it remains the
only available capacity; if both are dead, new admission returns `Executor`, not
`Busy`.

There is deliberately no automatic respawn, replacement loop, plugin lifecycle, or
thread kill. A non-cooperative hung worker remains different from a dead worker: its
slot is quarantined and unfinished until process exit, while the independent
deadline fails its component. Process restart is the recovery mechanism for dead or
hung capacity.

### Managed lifecycle regressions

The real threaded executor now proves:

- panic before completion closes the channel and produces one typed terminal result;
- a pending Runtime component becomes Failed with `pending = None` before the
  ordinary deadline is needed;
- repeated polling after death is finite and nonduplicating;
- two dead workers leave zero false pending slots and return `Executor` on admission;
- one dead worker does not prevent another component from reaching Warming and then
  Ready on the surviving worker;
- independent virtual acquisition and semantic Recorder facts continue;
- an unrelated native plant/controller keeps producing measurements, PID ticks and
  a live lease while the managed component fails;
- quiesce/shutdown closes the surviving worker and reports zero unfinished workers.

The panic remains contained by the existing worker-thread boundary. Component code
still has no Runtime, transport, Recorder, controller, or OutputAuthority handle and
cannot fabricate completion or physical evidence.

### Required-work progress and starvation guarantee

M11.2 did not introduce fairness or OS thread-priority policy. The accepted bounded
owner order remains:

```text
Recorder receipt poll
-> safety, controller watchdogs, safe output and transport completions
-> stopping/configuration fences
-> thermal/native measurement work
-> configured Metakon READ admission
-> References
-> native controllers
-> periodic managed sources
-> observation-driven managed transforms
-> semantic fact admission
-> Recorder receipt poll
```

Every lower phase yields when safety becomes due. Missed periodic slots coalesce to
one actual-time opportunity rather than replaying a backlog. In server mode one Host
turn precedes Recorder/history result polling and a bounded, rotated batch of at
most four client requests.

The progress guarantee is therefore:

> Given owner turns and a cooperative required worker, bounded managed, client,
> history, Recorder-query, emulator, or property-operation pressure cannot be
> admitted ahead of due safety/native work indefinitely.

It is not a guarantee of equal service among noncritical tasks, nor can it make an
OS/device call or trusted component that never returns complete. Intentional
configuration quiescence and shutdown fences also deliberately stop ordinary
acquisition; that is lifecycle behavior, not starvation.

The combined evidence is:

- the new real-panic test preserves unrelated acquisition/controller/fact progress;
- `host_scheduler` freezes safety-first order, coalescing and native controller
  progress;
- configured resource-scoped reconnect proves a faulted bus does not stop an
  unrelated physical resource;
- M9B.8 pressure proves emulator/API/event-gap/client isolation;
- `recorder_isolation` proves native controller and transport progress under held
  SQLite, history jobs and a nonreading client;
- configuration tests prove atomic rejection and deliberate safe quiescence rather
  than accidental partial progress.

### Shutdown under faults

Existing and new tests jointly freeze:

- a timing-out READ and protocol recovery reach a finite terminal resource state;
- reconnect/open retry uses one worker, one original deadline and a fixed attempt
  cap; shutdown during retry is finite;
- `TransportShutdown::Pending` receives later owner turns;
- a dead managed worker is already terminal and does not count as unfinished;
- a hung managed worker remains honestly unfinished but cannot delay the owner past
  the accepted shutdown grace;
- Recorder and transport cleanup progress independently;
- active/slow client delivery is best effort inside its separate finite window.

`shutdown requested` still does not mean every external resource is closed. Success
requires the existing safe, transport, managed-worker and Recorder evidence; no
unfinished state is hidden.

### Bounds re-confirmed

| Resource | Bound and exhaustion behavior |
|---|---|
| COM/open worker | One worker and at most one OS-open call per candidate |
| Reconnect open | At most 64 attempts, 100-ms spacing, one original open deadline |
| ResourceExecutor | 32 ordinary transactions plus one reserved safe-output slot; no unbounded retry queue |
| Physical transaction | Validated queue TTL and execution timeout; generic READ retry at most once after proven recovery; WRITE never retried |
| Managed executor | Exactly two fixed workers; one job and one completion mailbox per slot; no backlog or respawn |
| Managed execution | Independent 100-ms acceptance deadline; dead/hung capacity is never replaced automatically |
| Owner/reactor mailboxes | 64 messages each; bounded failure/detach behavior |
| Application operations | 64 pending globally, 8 per scope; retained outcomes are separately bounded |
| Client work per server turn | At most four rotated requests after required owner progress |

No capacity was increased and no unbounded fallback was added.

### Failure classification input for M11.7

| Failure | Classification | Recovery |
|---|---|---|
| Clean configured READ | Automatically continuing | Next periodic slot |
| CRC, malformed frame, silence, or disconnect on production COM | Requires explicit reconnect | Retire/open/probe/rebind/activate under existing M8 lifecycle |
| Incompatible compatibility probe | Requires explicit reconnect or corrected deployment | New validated candidate; failed generation remains quiesced |
| Stale completion | Automatically contained | Discard; current generation continues |
| Managed implementation typed failure | Scoped permanent component failure | Explicit component replacement/reconfiguration |
| Managed worker panic/channel death | Scoped component failure plus permanent loss of that fixed slot | Process restart; surviving slot continues |
| Non-cooperative managed hang | Scoped component deadline failure; worker completion not guaranteed | Process restart; shutdown reports unfinished if still running |
| Owner/reactor terminal channel loss | Process-level failure | Existing finite failed shutdown and process restart |

### Deferred work

At M11.2 completion, M11.3 remained the separate combined control/output recovery
gate, especially ambiguity plus reconnect plus proof of no automatic rearm. That
gate is now complete in the M11.3 section above. Recorder/platform hardening is
complete in M11.4 above; bounded diagnostic logging remains M11.5, and additional
full-process capacity/soak work remains M11.6-M11.7. No logging subsystem or Arduino
support was started in M11.2-M11.4.

### Verification

Concurrency-sensitive oracles were repeated independently:

```text
managed_executor::lifecycle_tests                         10/10 PASS
configured Metakon CRC/short/silence/disconnect matrix   10/10 PASS
```

Focused suites passed for serial open/retirement, configured physical acquisition,
M3 codec/transport/runtime recovery, M8 reconnect, native components, Host
scheduling, controller/OutputAuthority, Recorder quality/isolation, M9B.8 fault
acceptance, M9D physical output, configuration and shutdown.

Full gates passed:

```text
cargo fmt --all -- --check                               PASS
cargo test --workspace                                   PASS
cargo test --workspace --release                         PASS
cargo clippy --workspace --all-targets -- -D warnings    PASS
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps PASS
```

The exact 42-operation/25-capability registry regression remained green as part of
both workspace runs. No hardware test was run; accepted M8, post-M9C and M9D evidence
remains applicable and unchanged.
