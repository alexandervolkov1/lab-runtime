# Milestone 11.1 hardening and failure-model audit

## Status and scope

```text
M8-M10: ACCEPTED
M11: AUTHORIZED
M11.1 hardening and failure-model audit: COMPLETE
M11 implementation: NOT STARTED
M12+: NOT AUTHORIZED
```

This is a source-and-test-derived audit of accepted post-M10 HEAD
`a6e0e3e712eb7fbba23e6d308027ecb295cf9348`. Current source and accepted tests are
authoritative. No production code, tests, Cargo metadata, deployment or instrument
definitions, SQLite evidence, COM port, or hardware was changed or exercised.

The system already has unusually strong deterministic failure coverage. M11 should
therefore harden specific gaps, not redesign scheduling, OutputAuthority, Recorder,
the Application contract, or the SQLite schema.

Guarantee labels used below mean:

- **GUARANTEED**: enforced by current code and a deterministic regression, within
  the stated software boundary;
- **BEST EFFORT**: Runtime makes a bounded attempt, but completion depends on an OS,
  device, filesystem, or non-cooperative worker;
- **NOT GUARANTEED**: the current architecture deliberately does not claim it.

In particular, ACK, readback, and physical effect remain different facts. Even an
ACK plus matching register readback does not prove an independent physical heater
effect.

## Authoritative current fault model

The failure model follows the existing ownership hierarchy:

```text
Runtime
  owns authoritative experiment state, controller lifecycle,
  OutputAuthority, signal quality, transport scheduling and Recorder gating

HostCore
  gives Runtime bounded monotonic service opportunities and transfers
  semantic facts to Recorder without waiting for disk

ServiceHost
  owns startup, deployment, explicit reconnect and finite process shutdown

Application/server
  owns bounded client/session/projection state; client failure is not
  experiment failure

workers
  own blocking serial, managed computation, SQLite, or socket mechanics;
  their results are non-authoritative until the owner validates them
```

The normal containment rule is:

```text
detect at the narrowest owner
-> publish or retain an honest unavailable/failed/ambiguous state
-> revoke unsafe authority where required
-> do not retry work whose physical effect may already have happened
-> require an explicit lifecycle boundary for recovery when identity changed
```

Monotonic `Duration`/`Instant` drives scheduling, freshness, PID, leases,
transaction deadlines, reconnect and shutdown. Wall clock is sampled only by the
Recorder for human/export metadata. The time tests prove that forward, backward and
failed wall reads do not change control or output decisions.

## Physical acquisition and transport matrix

The principal code is `serial.rs`, `lab_core::transport::ResourceExecutor`,
`runtime/physical_io.rs`, `host/scheduler.rs`, and `service/reconnect.rs`.

| Failure | Detection | Authoritative transition | Measurement | Controller / physical output | Retry, recovery, rearm | Recorder / API | Guarantee and coverage |
|---|---|---|---|---|---|---|---|
| COM disconnect during READ | Serial worker returns `Disconnected`, completion channel closes, or an executor call fails | Active read emits `ReadUnavailable`; ordinary queue is fenced; executor enters recovery then `Offline` if no clean boundary | Matching current generation commits `Unavailable(Transport)` | Dependent controller fails on the next safety opportunity; any lease is revoked and safe is requested | Production COM does not silently reopen. Generic read retry is at most once only if an adapter proves recovery before the original deadline. Normal recovery is explicit `reconnect_resource`; controller rearm remains explicit | Unavailable measurement is a semantic fact when recording; resource/current state and operation failure are public | **GUARANTEED** containment. M3 transport, configured physical C14, M8 reconnect, Recorder quality/order tests |
| Initial OS open failure | Asynchronous `ComOpenStatus::Failed` or open deadline | Startup fails before readiness; no Runtime binding is advertised ready | No fabricated sample | No controller/output activation | Only `Disconnected` is transient during a reconnect candidate; startup does not claim readiness | Process startup returns failure; no archive mutation is claimed beyond successfully opened Recorder boot work | **GUARANTEED** software behavior. `configuration_startup`, serial open tests |
| Transient `Disconnected` while opening replacement | Candidate worker classifies failed attempt | Same candidate remains `Opening`; old binding is already quiesced/retired | Existing signal remains unavailable; no candidate acquisition | No output or controller rearm | Up to 64 attempts, 100 ms spacing, under one original `open_timeout_ms`; deadline is never reset | Reconnect remains accepted/pending then completes or fails with bounded public category; diagnostic snapshot retains attempts/latest class | **GUARANTEED** retry bound. Service and serial transient-open regressions; M8 hardware evidence |
| Invalid settings, timeout, or other open error | Typed `SerialError` | Candidate becomes failed; no pre-rebind generation advance | No candidate sample | No authority is granted | Terminal for that candidate; explicit corrected configuration/reconnect is required | Operation fails; bounded reconnect diagnostic retains stage/class | **GUARANTEED**. `invalid_settings_open_is_terminal...` and open policy tests |
| READ timeout or silence | Executor execution deadline; short response never reaches expected length | `ReadUnavailable`, queue fence, bounded recovery, then offline if recovery does not complete | One current-generation `Unavailable(Transport)` attempt; old Good is not reused as fresh | Safety service treats Unavailable as unusable and fails an active controller | No physical frame retry after production COM fault. Explicit reconnect is the ordinary recovery | Unavailable fact is durable if admitted; API current/event exposes quality/failure | **GUARANTEED** state semantics. M3 timeout/recovery, configured physical, Recorder quality tests |
| Malformed frame, wrong length/address/type, or CRC | Strict Metakon decode after a completed transaction | Publishes transport failure and enters protocol recovery; queued ordinary work is fenced | `Unavailable(Transport)` for the matching read | Active dependent control fails closed on safety service | Failed logical read is not replayed. Production COM has no implicit protocol-reset proof, so it reaches `Offline` unless an explicit clean/rebind boundary occurs | API does not expose raw bytes; Recorder retains the failed observation, but today not the detailed codec class | **GUARANTEED** decode/fail-closed semantics. Codec and M3 bad-CRC tests. **Gap:** configured-COM integration does not exercise every codec class |
| Wrong compatibility probe/channel type | Resource-scoped probe observes a new non-Good or non-integer-3 result | New binding generation remains authoritative but quiesced; replacement is retired, never rolled back to the old generation | Rebind baseline remains Unavailable; ordinary polling is not released | No controller or output activation | Another explicit reconnect/configuration correction is required | Reconnect operation fails; diagnostic stage is `ProbeFailed` | **GUARANTEED**. configured physical failed-probe/timeout tests |
| Stale binding/mapping or executor generation completion | Correlation and binding/mapping comparison in Runtime | Stale result is discarded; it cannot mutate the replacement | No stale Good publication | Cannot renew a lease, ACK a new dispatch, or confirm new safe state | No retry based on stale result; current generation proceeds independently | No false fact/result is projected | **GUARANTEED**. M3 rebind and M8 generation-fence regressions |
| Serial worker hangs in an OS call | Owner deadline expires while worker handle remains unfinished | Runtime/executor becomes unavailable or shutdown-pending; owner never joins the worker | Pending read becomes unavailable when executor deadline is serviced | Safe delivery becomes best effort; no success is invented | Rust cannot kill the thread. It is quarantined/detached and replacement is bounded so concurrent open workers are not spawned | API/shutdown reports unfinished transport and unsuccessful cleanup | Owner liveness is **GUARANTEED**; worker termination and device state are **NOT GUARANTEED**. Stuck-worker and shutdown tests |
| Reconnect retirement fails or times out | Repeated nonblocking `try_shutdown` until configured recovery deadline | No replacement is installed before old retirement proof; resource remains quiesced | Unavailable remains current | No rearm/output release | Explicit later retry only after the quarantined worker is resolved; no generation rollback | Reconnect fails with stage-specific in-process diagnostic | **GUARANTEED**. service reconnect retirement tests |
| Queue full or resource unavailable | Admission checks fixed ordinary queue / executor state | Command rejects without hidden work; safe queue remains separately reserved | No new observation from rejected admission | No ordinary output bypass; safe obligation remains pending | Periodic scheduling skips admission while reconnect-quiesced/recovering. It does not grow the queue | Bounded domain/public error; no fake Recorder fact | **GUARANTEED**. queue-capacity and configured-recovery tests |
| Shutdown during open, retry, recovery, or read | Stop barrier plus persistent serial stop intent | New work fenced; active correlation terminalized honestly; adapter retirement receives later owner turns | No post-fence Good from old work | Safety is attempted first; started output keeps ambiguity | Bounded owner deadlines; no join of a non-cooperative worker | Shutdown status distinguishes unfinished transport and exits unsuccessful when cleanup is incomplete | **GUARANTEED** owner/process bound, **BEST EFFORT** device/worker close. configured physical and COM/Recorder shutdown tests |

Two qualifications matter. First, production `ComTransport::try_recover` requires a
trusted clean boundary; it does not treat elapsed time as proof that a disconnected
or protocol-confused serial session is clean. Second, the generic
`ResourceExecutor` one-read retry exists for adapters that can prove recovery, but
physical output transactions are never retryable.

## Measurement freshness and quality matrix

| Failure | Detection | Authoritative transition | Measurement | Controller / output | Recovery / rearm | Recorder / API | Guarantee and coverage |
|---|---|---|---|---|---|---|---|
| Never observed | Signal has no latest sample | State remains registered but unobserved | API projects unavailable/not-observed; no cached value is invented | Start/tick rejects input | Wait for a real observation; controller start/resume remains explicit | No measurement fact exists merely for a query | **GUARANTEED**. M1 and measurement API tests |
| Explicit Unavailable | Sample quality is not `Good` | Unavailable becomes the latest attempt, including at equal publication time where allowed | Prior Good stays in history but is not current input | Warming/Running fails closed; lease cannot be renewed | A later distinct Good may restore the signal, but not the failed controller | Unavailable quality/failure is durable and public | **GUARANTEED**. M1, M4, Recorder quality tests |
| Stale Good-looking cached value | `at - freshness_at >= max_input_age` (exclusive age) | Signal stays Good as history/current data, but control input validation fails | Value remains queryable with its original time | Controller becomes Failed; safe requested; no reuse for PID | Fresh observation plus explicit `reset_failed` and `resume` | Controller failure/output facts plus original measurement time | **GUARANTEED**. M4 exact-age/stale-input tests |
| Instrument generation replacement | Rebind/restart checks expected generation | New generation commits an Unavailable baseline; old pending work is fenced | Current cannot remain old-generation Good | Dependent active controllers must already be stopped or are invalidated/fail closed according to path | Fresh new-generation measurement; explicit controller lifecycle | Generation/revision is projected and recorded | **GUARANTEED**. M8 rebind and model restart tests |
| Late old-generation completion | Correlation generation/revision/runtime identity mismatch | Completion discarded | No publication | Cannot restore a dependent controller or authority | None; wait for current generation | No relabelled fact | **GUARANTEED**. managed and transport stale-completion tests |
| Non-finite/out-of-range managed result or invalid bounded state | Runtime validates result, state, unit, shape and status | Component becomes Failed; latest signal is invalidated/unavailable | `Unavailable(ComponentFailure)` | Dependent managed branch and active controller fail; component has no transport/output authority | Replacement/reconfiguration is explicit; no automatic restart | Failed signal/component state is public; measurement lineage/failure is recordable | **GUARANTEED**. M5 invalid-result and M9A tests |
| Virtual/emulator Unavailable | Validated virtual-only command with `value=None` | Ordinary virtual signal commit | Latest is Unavailable/Disabled | Same freshness/quality rules as native input | Later valid virtual publication; controller recovery explicit | Generic event/history/Recorder paths | **GUARANTEED**. emulator API tests |

The central invariant is therefore implemented: a fresh `Good` observation is not
equivalent to a stale cached value whose payload still looks Good.

## Controller failure and recovery matrix

| Failure | Detection | Authoritative controller state | Output consequence | Retry / recovery | Explicit rearm? | Recorder / API | Guarantee and coverage |
|---|---|---|---|---|---|---|---|
| Invalid controller/PID configuration | Core validates units, bounds, timing, revision and safe lifecycle state before commit | Unchanged | Unchanged; no lease | Submit a corrected full configuration | N/A | Structured public rejection; no partial configuration fact | **GUARANTEED**. controller configuration/control API tests |
| Invalid lifecycle transition | Exact state match for start/pause/resume/reset | Unchanged | No hidden authority mutation, except watchdog service remains allowed where explicitly documented | Use the valid operation for current state | Yes where applicable | Typed/structured failure | **GUARANTEED** |
| Unavailable input | `control_input` finds no current Good finite value | Warming or Running becomes Failed | Running lease revoked; safe requested | Restore input, then `reset_failed`, then `resume` | **YES** | Controller/output and unavailable measurement facts | **GUARANTEED**. M4 and host scheduler tests |
| Stale input | Exclusive freshness check | Failed | Same fail-closed safe transition | Fresh observation plus explicit recovery | **YES** | Same as above | **GUARANTEED** |
| Tick gap exceeds `max_tick_gap` | Safety service compares actual monotonic service time | Failed | Lease revoked; safe requested | Correct scheduling/availability, reset, resume | **YES** | Controller/output facts; status Failed | **GUARANTEED**. host scheduler and M4 tests |
| Non-finite/invalid PID or EMA computation | Algorithm returns typed error | Failed | No proposal succeeds; authority trips safe | Correct configuration/input, reset, resume | **YES** | Bounded controller failure, no non-finite public output | **GUARANTEED** by algorithm/runtime validation tests |
| Output delivery/readback failure | Physical completion cannot settle matching output | Failed for ordinary controller dispatch | Safe requested; ambiguous/mismatch truth retained | Resolve transport/safety, reset, resume | **YES** | Correlated output stages and controller state | **GUARANTEED**. M9D suite |
| Pause | Explicit command | Warming/Running to Paused | Active lease released; safe evidence required for physical output | Resume begins Warming, not Running | **YES**, `resume` | Lifecycle and output facts | **GUARANTEED** |
| Resume | Explicit command from safely Paused and authority `can_prepare` | Warming | No lease until distinct Good warm-up completes | Continue bounded scheduling | It is the explicit rearm | API returns accepted/completed warming, never claims physical output | **GUARANTEED** |
| `reset_failed` | Explicit command only from Failed, with safe/disarmed authority | Paused with reset algorithms | Does not acquire a lease or produce output | Follow with explicit resume | **YES**, two deliberate steps | Public lifecycle result/status | **GUARANTEED** |
| Lease expires | OutputAuthority monotonic watchdog at exact expiry | Running loses valid ownership and then fails on safety service | `SafePending`; queued ordinary work fails final validation | Establish safe evidence, reset, resume | **YES** | Revoked/safe-requested facts retain epoch/attempt | **GUARANTEED**. M2/M4 exact expiry tests |
| Component/instrument replacement or generation change | Checked generation/revision and safe configuration barrier | Active dependents are paused/failed before replacement as defined | Old lease/proposal cannot cross generation | Complete replacement/probe/safe boundary, then explicit lifecycle | **YES** | New generation plus configuration/provenance facts | **GUARANTEED** |

No accepted controller fault automatically rearms a controller. Queries are pure and
do not refresh input, advance Reference/PID, renew leases, or service watchdogs.

## Physical WRITE matrix

The following distinctions are normative:

```text
requested
!= authorized
!= send_started
!= ACK
!= readback
!= physical_effect
```

| WRITE outcome | Detection and authoritative transition | Controller / output consequence | Retry and recovery | Recorder / API | Guarantee |
|---|---|---|---|---|---|
| Failure before `send_started` | Final authority/generation check rejects, queue expires, or adapter accepts no positive prefix; reservation is aborted | No dispatch is considered started; safe obligation may remain eligible for a later delivery | Ordinary request is not silently resurrected. A safe request may use the existing later-delivery policy because no possible output byte was admitted | Expired/failed attempt is recorded without fabricated send/ACK/readback | **GUARANTEED** no adapter-admitted byte; M2/M3 and M9D safe-before-send test |
| `send_started`, no valid ACK | Timeout, I/O failure, invalid ACK, protocol recovery, or terminal failure after positive adapter admission | Outcome becomes `Ambiguous`; ordinary controller fails and distinct safe is requested | **NO blind retry** of the WRITE. Recovery may clean transport but cannot reinterpret the original effect | Send-started, uncertainty/ambiguous facts remain correlated; API never reports success | **GUARANTEED** software ambiguity/no-retry; physical effect **NOT GUARANTEED** |
| ACK valid, separate readback unavailable/invalid | Strict ACK is recorded, then nonretryable reg06 READ fails or cannot decode | Outcome becomes Ambiguous; ordinary controller fails; safe transition requested | WRITE is not retried. A safe WRITE that reaches this ambiguity blocks another automatic safe resend | ACK and Ambiguous are separate durable/public evidence | **GUARANTEED**. M9D readback-timeout test |
| ACK valid, readback mismatches | Typed reg06 readback differs from authorized value | Outcome Failed/Mismatch; ordinary controller fails and safe is requested; a safe mismatch remains unconfirmed | The original WRITE is not retried. Because mismatch is positive evidence of a non-safe register value rather than unknown effect, the existing safe policy may attempt a distinct safe transition | Requested/ACK/reported value/failure are retained | **GUARANTEED**. M9D mismatch test |
| ACK and matching readback | Strict ACK followed by separate typed reg06 READ | `ReadbackVerified`; ordinary physical controller lease may renew; safe dispatch becomes `safe_confirmed`/Disarmed | No duplicate WRITE | Distinct ACK/readback stages and values | Software register evidence **GUARANTEED**; independent physical effect **NOT GUARANTEED** |
| Safe-zero becomes ambiguous after start | `transport_uncertain`, failed readback, or ambiguous terminal for a safe dispatch | `safe_obligation_pending=true`, `ambiguous_safe_resend_blocked=true`, `safe_confirmed=false`, FaultLatched, no normal output | Repeated safety turns and safe requests cannot emit another WRITE. A fresh authority created by validated rebind can start a new safe-establishment lifecycle; it still does not rearm a controller | Ambiguous safe evidence remains visible/durable | **GUARANTEED**. exact M9D partial-prefix/no-retry regression |

## OutputAuthority and lease failure matrix

| Failure | Detection / transition | Physical-output consequence | Recovery and rearm | Guarantee and coverage |
|---|---|---|---|---|
| Expired lease | `at >= expires` in watchdog/final checks | Pending ordinary proposal displaced; safe requested | Safe evidence, explicit controller reset/resume | **GUARANTEED** |
| Stale authority epoch/runtime instance | Token/dispatch identity mismatch | Rejected; cannot ACK/complete current authority | None from stale caller | **GUARANTEED** by M2 identity tests |
| Stale binding or mapping generation | Final pre-byte validation and completion correlation | Before-start rejection or late result ignored; cannot confirm replacement | Explicit current-generation operation only | **GUARANTEED** |
| Final recheck fails after queueing | `AuthorizationStep::Validate` immediately before first possible byte | `started=false`; no ordinary send | New deliberate proposal/safe lifecycle as appropriate | **GUARANTEED** by revoke-after-queue tests |
| Controller pauses while ordinary output is queued | Pause/revoke advances epoch before final validation | Queued ordinary send is fenced; safe intent owns new epoch | Resume only after safe confirmation | **GUARANTEED** |
| Safe obligation while transport unavailable | Safe reservation/queue is retained or repeatedly attempted without claiming success; offline transport cannot complete it | No normal output; `safe_confirmed=false` | Hardware availability plus explicit rebind may establish a fresh safe authority; completion is **BEST EFFORT** while hardware is absent | Containment **GUARANTEED**, physical safe effect **NOT GUARANTEED** |
| Unresolved ambiguous safe state | Separate resend-block latch | No further WRITE and no normal lease | No current in-place reconciliation operation. Validated rebind creates fresh authority; controller remains non-running and still needs explicit reset/resume | **GUARANTEED** no retry/no rearm |

## Recorder and SQLite failure matrix

The semantic boundary is `Runtime facts -> Host admission -> RecorderWorker ->
SQLite worker`. Runtime never performs SQL and never waits for disk.

| Failure | Detection | Authoritative Recorder transition | Acquisition / control / output | Durable consequence | API / recovery | Guarantee and coverage |
|---|---|---|---|---|---|---|
| Recorder unavailable or unconfigured | No worker is attached, or the attached worker has entered Failed/Closed | Public state is `unconfigured`, `failed`, or `closed`; no new recording admission | Acquisition continues. A deployment requiring Recorder fails before serving; a Required worker failure trips active authority/control | No facts are invented; an already committed prefix remains readable | `recording_start`/history reject as appropriate; correct storage and start a new process/worker | **GUARANTEED**. Recorder API/startup/failure/Required tests |
| Start failure | Start admission rejects, startup deadline, or SQLite transaction fails | Starting to Failed, or request rejected before transition | BestEffort does not own control. Required remains closed and cannot start authority | No successful run/interval boundary is claimed | Structured failed operation/status; process restart or corrected storage/configuration | **GUARANTEED** |
| Ingress capacity exhausted | Pre-send group/record/byte credit check or full/disconnected channel | Sticky Failed; known loss schedules one reserved FIFO gap seal | Acquisition continues. Required immediately fails controllers/trips outputs; BestEffort may continue control | Committed prefix retained; gap may be durable, otherwise `unknown_tail` and `failure_persisted=false` | `recording_status` exposes failure, coverage, watermark and pending counts; new process/storage required | **GUARANTEED**. backpressure/failure/Required suites |
| Core fact outbox exhausted | 256 facts or 256 KiB bounded capture overflow | Host disables capture, Recorder fails with known gap | Required fails closed; BestEffort continues with truthful loss | First lost fact/count retained when representable | Failed status/gap | **GUARANTEED** by Core and Recorder backpressure tests |
| Required progress deadline | No strictly newer durable receipt within 2 s from original submission | Sticky Required gate failure | Acquisition continues; Warming/Running controllers fail; all outputs trip safe | Prefix remains; tail may be pending/unknown | Recording failed; no automatic reopen/rearm | **GUARANTEED**. exact-deadline and held-writer tests |
| SQLite busy/locked or second owner | 100 ms SQLite busy timeout / exclusive ownership check | Open or worker operation fails | Same policy split as storage failure | No invented commit; prior prefix remains | Bounded failure; later process/open after lock release | Generic SQLite error handling **GUARANTEED**; concurrent-owner startup covered. Exact live external-lock classes need one focused acceptance oracle |
| Write/transaction/constraint/checkpoint failure | Transaction/commit/health call returns error | Worker receipt Failed; storage thread exits | Required fails closed; BestEffort acquisition/control may continue without Recorder | Whole failed transaction rolls back; durable checkpoint cannot advance over it; coverage becomes gap/unknown tail | Public error is bounded `recording_failed`, not raw SQL | **GUARANTEED**. transaction, checkpoint, failure suites |
| Storage worker panic | Join handle becomes finished while `alive` stayed true | Owner latches `storage worker panicked`; no fabricated receipt or credit release | Required fails closed; owner remains nonblocking | Only already committed prefix is durable | Failed status; process restart/reopen | **GUARANTEED**. worker-panic and Required-host tests |
| History job capacity exhausted | Eight active queued/executing/retained jobs or mailbox contention | Recording lifecycle is unchanged | Acquisition/control/output continue | No archive mutation | `history_busy`; cancel/disconnect/TTL releases capacity | **GUARANTEED**. history API/cancellation tests |
| Connection-local history cursor capacity exhausted | Eight retained cursors per Application | No Recorder failure | No effect | No effect | `history_busy`; disconnect or 5 s expiry releases cursor | **GUARANTEED** |
| Filesystem/disk error | SQLite/open/write/metadata/close returns error | Same sticky Failed path | Policy split as above | Prefix only; failure seal may be impossible | Correct path/space/permissions and restart | Handling is **GUARANTEED** by typed/injected storage failures; exact Windows permission/disk-full behavior is **BEST EFFORT** and lacks a platform acceptance test |
| Process killed before commit | OS terminates process | No owner transition can complete | No shutdown safe-effect guarantee after process death | SQLite transaction is absent/rolled back on reopen | New boot marks old active run/interval interrupted/unknown-tail | **GUARANTEED** committed-prefix interpretation; process survival is not claimed |
| Process killed after commit before receipt | WAL commit exists but owner receipt is lost | Old process state is unknowable | New process never restores controller lease/output authority | Committed row reopens under old boot; no duplicate control replay | New boot; old operation terminal may remain unknown | **GUARANTEED**. process-reopen tests |
| Unsealed boot/run/interval on reopen | Active/starting/stopping rows found during validated open | Prior rows become `interrupted`, coverage `unknown_tail`; new boot identity | No old authority/controller state restored | Archive remains structurally queryable but semantically incomplete | History exposes interrupted/coverage state | **GUARANTEED**. reopen/process tests |
| Clean stop | Stop accepted only after prior admitted FIFO facts; transaction commits interval/run seals | Recording returns Idle | Required control must already be paused/safe; stop closes gate but does not destroy worker | Accepted prefix drained; interval/run sealed and committed | Successful terminal operation; worker may remain alive | **GUARANTEED** |
| Clean process shutdown | Finish transaction commits terminal boot seal and SQLite close succeeds | Closed; `worker_closed=true` | Safety is handled independently before terminal success | Terminal boot seal, committed prefix; normal close checkpoints/removes active sidecars | `recorder_flushed=true`, successful process exit only with all criteria | **GUARANTEED** for accepted path |
| Gap/incomplete coverage | Known admission loss or unconfirmed tail | `gap` with known metadata, or conservative `unknown_tail` | Required fails closed; BestEffort may continue | Completeness is explicitly false even when SQLite is structurally readable | Status/history expose coverage/loss | **GUARANTEED** |

### Crash/recovery guarantee boundary

Current code and tests establish:

- clean close: terminal boot seal committed, connection closed, accepted prefix and
  run/interval seals reopen;
- process kill: committed WAL prefix may survive even when its owner receipt did
  not; uncommitted work is not promoted;
- reopen: old active boot/run/interval becomes interrupted with unknown-tail
  coverage, and a new boot is created;
- schema/application/index/provenance compatibility is validated before recovery;
- authority, lease, controller-running state and safe evidence are never restored
  from SQLite.

They do **not** establish that every crash leaves no WAL/SHM, that every filesystem
or power-loss mode preserves a readable file, or that a structurally readable
archive is semantically complete. Clean evidence may require no sidecars; crash
evidence may legitimately require WAL recovery. A complete archive requires sealed
boot/run/interval state and complete coverage, not merely a successful SQLite open.

## Application, client and emulator failure matrix

| Failure | Detection / state transition | Experiment consequence | Retry / recovery | Recorder / API consequence | Guarantee and coverage |
|---|---|---|---|---|---|
| Malformed JSON, duplicate key, excessive depth/value/string, invalid UTF-8 | Bounded wire decoder rejects before owner | None | Client opens a new valid connection | One bounded best-effort rejection, then affected peer closes | **GUARANTEED**. protocol/client/M9B.8 tests |
| Oversized/unterminated frame | 16,384-byte frame cap or 2 s partial-frame deadline | None | Reconnect | Peer closes; no owner request | **GUARANTEED** |
| Unknown operation | Central registry lookup | None | Correct request on same session | Structured nonfatal rejection | **GUARANTEED** |
| Invalid DTO/domain input | Strict keys/types/bounds then Core validation | Atomic rejection; no partial mutation | Correct request/revision | 12-category public mapping, bounded message | **GUARANTEED** |
| Duplicate live `msg_id` | Per-peer pending ID set | No second dispatch | Wait for terminal or reconnect with request scope | Affected peer is rejected/closed; accepted work remains | **GUARANTEED** |
| Duplicate mutation request ID | Session scope, sequence and normalized payload | Exact replay returns retained state; changed payload conflicts; old evicted ID is unknown and never re-executed | Reconcile using operation status/new consecutive ID | Accepted mutation survives disconnect | **GUARANTEED** |
| Client disconnect after accepted mutation | Reactor detach | Mutation is not rolled back; Runtime continues | Reconnect same live scope and query retained outcome | Recorder operation fact follows owner lifecycle, not socket lifetime | **GUARANTEED** |
| Slow/nonreading client | Reply/event queue limits or 2 s blocked-write deadline | Native owner and other clients continue | Reconnect/resync | Peer detaches; subscription/history resources cancel | **GUARANTEED** |
| Full event queue | 16 per-client queued event frames | No experiment backpressure | Reconnect and resync current/replay | Client detaches; no Recorder gap because transient event replay is separate | **GUARANTEED** |
| Replay ring gap | Cursor older than retained 1,024-event ring | None | Explicit current/discovery resync and new subscription | `event_gap`, subscription abandoned | **GUARANTEED** |
| Per-session/global operation capacity | 8 pending per scope / 64 global; 32/256 terminal retention | Rejected work never runs | Wait for completion/TTL or use retained status | `busy`; pending records never evicted | **GUARANTEED** |
| Too many clients/scopes | 8 socket clients; 16 logical scopes | Experiment continues | Capacity releases on detach/expiry | Extra socket is dropped or scope returns capacity error | Code is bounded; exact MAX_CLIENTS process-level oracle is a **coverage gap** |
| History pressure/leaked cursor | Eight jobs and eight connection cursors, cancellation/TTL/generation fences | No native-work blocking | Finish/cancel/disconnect/expiry | `history_busy`; late result cannot occupy reused client capacity | **GUARANTEED** |
| Server reactor channel loss/panic | Owner sees disconnected incoming channel, or final join reports panic | Fatal bounded shutdown, not continued unsupervised serving | Process restart | Nonzero process outcome; today diagnostics are sparse | Containment **GUARANTEED** by fatal-path tests; root-cause observability is missing |
| Shutdown with active clients | Stop barrier blocks new hello/mutations; shutdown terminal has a 200 ms delivery window | Safety/Recorder progression remains owner-priority | Clients reconnect only to a new process | Best-effort terminal replies, then sockets close | **GUARANTEED** finite owner path; delivery to dead/slow clients **NOT GUARANTEED** |
| Fast `emulator_publish` | Wire/session bounds and serialized owner dispatch | Host service runs before each client batch; Recorder pressure follows selected policy | Client receives busy/failure or continues bounded publishing | Generic measurement/event/history/Recorder path | **GUARANTEED** isolation in M9B.8 pressure test; long soak is missing |
| Publish to physical/nondeclared target | Host `emulator_targets` allowlist | Atomic rejection; no physical state/evidence | Use a declared virtual target | `not_allowed`-class public failure; no physical fact | **GUARANTEED** |
| Non-finite/out-of-range/wrong generation publication | Wire/Core validation | No signal mutation | Correct value/current generation | Structured failure | **GUARANTEED** |
| Multiple emulator clients | Session requests serialize through one owner; signal time/order checks remain authoritative | Committed publications follow owner order, not client ownership | Disconnect does not undo a committed sample | Ordinary generic facts | Containment is **GUARANTEED**; a dedicated multi-publisher ordering stress test is missing |

The invariant is preserved:

```text
bad / slow / dead client != bad / slow / dead experiment
```

An emulator command can create only a configured virtual observation. It cannot
construct physical transport completion, ACK, readback, output-safe evidence, or a
physical instrument observation.

## Managed-component failure matrix

| Failure | Detection / transition | Signal and controller consequence | Recovery | Authority boundary | Guarantee and coverage |
|---|---|---|---|---|---|
| Both slots busy | Two fixed workers, one job/completion mailbox each, no backlog | Invocation rejects Busy; existing committed state is unchanged | Retry on a later scheduler opportunity if component not failed | Component has no Runtime/transport/output handle | **GUARANTEED** |
| 100 ms deadline expires | Runtime polls `try_expire` | Component Failed; signal unavailable; dependent controller fails safe | Explicit component replacement/reconfiguration | No output authority | **GUARANTEED** by adapter-deadline test |
| Cooperative algorithm deadline/cancel | Native implementation checks deadline/cancel flag | Typed failed completion, then same Runtime failure path | Explicit replacement | No authority | **GUARANTEED** for registered implementations |
| Non-cooperative worker hangs | Deadline marks component failed; worker remains unfinished | Required native work continues; one slot is permanently occupied until process exit | Process restart; no thread kill/replacement | Cannot seize Runtime or block owner | Owner isolation **GUARANTEED**; worker completion **NOT GUARANTEED**. Shutdown fake covers reporting, but real executor hang needs a focused test |
| Worker panics | Completion channel disconnects | Busy correlation eventually expires and component fails, but `ManagedExecutor::try_poll` currently treats disconnected like empty and does not retire/reconstitute the dead slot | Process restart; remaining slot may continue | Cannot mutate Runtime after panic | Safety containment is present; explicit worker-death status/capacity behavior is a **real hardening gap** |
| Invalid result/state/unit/non-finite value | Runtime validation | Component Failed, Unavailable signal, dependent branch/controller failed | Explicit replacement | No authority | **GUARANTEED** |
| Stale/forged completion | Full correlation including Runtime instance, component, generation, attempt, revision | Ignored | Current work continues | Cannot commit or affect output | **GUARANTEED** |
| Generation replacement | Old job canceled and completion fenced | Replacement starts Warming; old Good is invalidated | Normal explicit replacement lifecycle | Dependent controllers do not auto-rearm | **GUARANTEED** |
| Shutdown | Admission closes, cancellation set, jobs senders dropped; no join unless naturally finished | No new commits; unfinished count remains honest | Process exit | Safety/transport work proceeds independently | **GUARANTEED** finite owner path; worker return **BEST EFFORT** |

## Shutdown matrix

`shutdown requested` does not mean that all resources are already closed. Terminal
success requires honest evidence from independent subsystems.

| Condition | Progress and terminal rule | Guarantee / test |
|---|---|---|
| Normal safe output | Stop producers, service safety until every configured output is readback-confirmed safe and has no lease/pending/in-flight work | **GUARANTEED** virtual and M9D software paths; runtime shutdown tests |
| Safe obligation cannot complete | Safety turns continue for 2 s, then Recorder cleanup may proceed; terminal status remains unsuccessful and does not claim safe | Finite owner exit **GUARANTEED**; physical safe effect **NOT GUARANTEED** |
| `TransportShutdown::Pending` | It is nonterminal and receives subsequent owner turns; ordinary one-turn retirement is not frozen as failure | **GUARANTEED**, dedicated M9D regression |
| Truly hung transport/open candidate | Persistent stop intent, no join; unfinished transport/candidate remains in status until deadline | Finite owner/process path **GUARANTEED**; thread/device close **NOT GUARANTEED** |
| Managed worker incomplete | Safe work proceeds; after safe confirmation a 200 ms grace prevents indefinite wait; unfinished count makes exit unsuccessful | **GUARANTEED** |
| Recorder flush pending | Nonblocking stop/finish progression for up to 2 s after safety phase | **GUARANTEED** finite; durable completion **BEST EFFORT** |
| Recorder failed or close failed | No successful flush claim; already committed terminal seal is distinguished from successful connection close | **GUARANTEED** |
| Ambiguous physical write | Ambiguity remains; no retry or fabricated safe evidence; shutdown can terminate unsuccessful after bounds | **GUARANTEED** truthfulness; physical state **NOT GUARANTEED** |
| Active/slow API clients | Stop admission, continue bounded owner/reactor turns, offer terminal response for 200 ms, then stop reactor | Finite server shutdown **GUARANTEED**; client receipt **NOT GUARANTEED** |
| Fatal owner/reactor fault | Enters the same safety/Recorder grace and returns nonzero | **GUARANTEED** by fatal shutdown tests |

Maximum normal shutdown is phase-bounded rather than one single deadline: up to 2 s
for unresolved safety, a 200 ms managed-worker grace once safe, up to 2 s for
Recorder/transport cleanup, and a 200 ms terminal-client delivery window. These are
owner bounds, not promises that an OS-blocked detached worker has terminated.

## Recorder facts versus diagnostic logs

Semantic events that affect experiment interpretation should remain durable when a
Recorder interval is active:

- every Good/Unavailable measurement attempt and generation identity;
- controller configuration/lifecycle/failure;
- output requested, authorized, send-started, ACK, readback, mismatch, uncertainty,
  ambiguity and safe transitions;
- configuration activation/revision and successful reconnect generation boundary;
- accepted/terminal Application operations;
- Recorder start/stop/failure, gap, coverage and sealing state;
- component activation identity and failed/unavailable derived observations.

M11 should evaluate adding a bounded semantic resource transition record using the
existing version-one record/runtime-event mechanism (no schema change) for an
autonomous physical resource loss/offline boundary. Today the Unavailable
measurement is durable, but the precise resource/protocol failure class is mostly a
volatile snapshot or failed operation detail.

Diagnostic-only information should normally include:

- serial open attempt number, OS error class and recovery polling;
- raw codec rejection class, bounded frame lengths and queue high-water marks;
- worker start/stop/panic and mailbox pressure;
- TCP accept/detach, malformed-peer reason and reactor errors;
- SQLite busy/checkpoint/internal error text and latency;
- routine scheduler turn, poll iteration and successful low-level byte chatter.

Do not duplicate each diagnostic line into SQLite. A log line does not become
experiment evidence, and a Recorder fact must not depend on the log sink succeeding.

## Diagnostic logging audit and target design

### Current state: missing

No logging facade or subscriber is present in Cargo metadata. Production has no log
levels, structured diagnostic records, file destination, rotation, retention, byte
bound, startup/shutdown log, panic hook, or component-specific transport/Recorder/
server diagnostics. `main.rs` writes only the serve readiness line and the legacy
deterministic demo output to stdout. Production source has no `eprintln!`; test
processes use stdout markers only for harness coordination.

Current bounded snapshots and public errors are valuable state projections, but
they are not a troubleshooting log and do not preserve the causal detail of a
failed OS/device/storage/network step.

### Minimal M11 logging direction

M11.5 should add one process-wide, static, structured diagnostic path with these
properties:

1. levels `ERROR`, `WARN`, `INFO`, `DEBUG`; default `INFO`, explicit configuration
   for `DEBUG`, and no unbounded per-byte trace;
2. human-readable line-delimited file records containing UTC display time,
   monotonic elapsed time, level, subsystem, stable bounded event name and bounded
   fields;
3. one explicit destination resolved before readiness; startup failure to create
   the configured required destination fails startup, while later write failure is
   reported once to stderr and never changes experiment authority;
4. hard size rotation, proposed 4 MiB per file and four files including current
   (16 MiB total), rotate-before-write, truncate/delete the oldest, and cap each
   record (proposed 8 KiB);
5. a fixed nonblocking producer queue (proposed 1,024 records) owned by one log
   writer; owner/worker paths use `try_send`, never wait for log I/O, and count
   dropped DEBUG/INFO/WARN/ERROR records by level;
6. one bounded synthetic overflow summary when the writer becomes writable; no
   recursive logging through the same full queue;
7. startup/build/config/archive identity, readiness, shutdown phase/result, panic,
   serial/reconnect, transport/protocol, controller/output safety, Recorder/storage,
   and server/reactor lifecycle events;
8. shutdown closes admission and gives the logger a short bounded flush opportunity
   after authoritative Recorder/safety status is frozen; logger failure cannot turn
   an unsafe shutdown into success or vice versa;
9. secrets/raw arbitrary client payloads are not logged; protocol fields and paths
   are bounded/redacted as appropriate;
10. collection is the current file plus numbered rotations; M12 later documents the
    user workflow.

The exact dependency/facade choice should be made in M11.5 after a small prototype.
The acceptance contract is the bounds and failure behavior above, not a particular
logging crate. A daily-only appender is insufficient because it does not provide a
hard size bound.

## Bounded-resource matrix

| Resource | Capacity / deadline | Owner | Exhaustion behavior | Release / recovery | Existing test |
|---|---|---|---|---|---|
| Runtime instruments / resources / references / controllers / components | 64 / 8 / 64 / 64 / 8 | Runtime | Atomic registration rejection | Reconfiguration/restart; identities do not silently recycle | Core registration/capacity tests |
| Signal history | 1..=4,096 per signal | Runtime signal buffer | Oldest-first eviction | Automatic bounded eviction | M1 history tests |
| Instrument parameters | 16 per definition | Core definition | Definition rejection | Correct definition | definition tests |
| Transport ordinary queue | 32 per resource | ResourceExecutor | `QueueFull`; no admission | Completion/fence/rebind | M3 queue tests |
| Transport safe queue | one reserved safe transaction | ResourceExecutor | Busy; obligation remains | Settle/fence/rebind | M2/M9D tests |
| Transport transaction/recovery | max 60 s generic; deployment supplies smaller validated values | ResourceExecutor | Timeout, recovery, then Offline | Proven recovery or explicit rebind | M3/M8 tests |
| COM request/completion | one each; OS call settings 1..=250 ms | COM worker | Owner sees no progress/timeout; never adds backlog | Completion or retirement; non-cooperative thread detached | serial tests |
| Reconnect open attempts | 64, 100 ms spacing, one original open deadline | candidate COM worker / ServiceHost | Candidate failure/quarantine | explicit later reconnect after cleanup | serial/service tests |
| Managed workers | two slots; one job and one completion each; 100 ms job deadline | ManagedExecutor | Busy; timed-out component fails | Later free slot or process restart for stuck/dead slot | M5 deadline/capacity; real worker-death gap |
| Runtime Recorder fact outbox | 256 facts and 256 KiB | Runtime | Sticky loss; Required fails closed | New recording/process after failure | Core Recorder tests |
| Recorder ingress | 4 groups, 1,024 records, 4 MiB total, 512 KiB/group, 256 facts/group, 64 KiB/record charge | RecorderWorker | Sticky failure plus reserved gap attempt | Process restart/new worker | backpressure tests |
| Recorder storage batch | up to 4 groups, 256 records, 512 KiB, 100 ms coalescing | SQLite worker | Stops coalescing; transaction remains atomic | Next FIFO command | backpressure tests |
| Recorder command channel | ordinary credits plus reserved lifecycle/failure/finish slots (`4 + 8 + 6`) | RecorderWorker | Lifecycle failure; no owner block | Worker progress or fail | Recorder lifecycle tests |
| Recorder history jobs | 8 | RecorderWorker | `history_busy` | Completion/cancel/disconnect | history tests |
| Durable history rows | 1..=128 measurements; 1..=32 runs | SQLite query layer | Invalid/busy or shorter bounded page | continuation cursor | history tests |
| SQLite main file | 1 GiB hard page cap; writes stop at 95% reserve | SqliteStore | Storage failure before unsafe growth | Free space/new archive; no migration in M11 | transaction/quota tests |
| SQLite WAL | 16 MiB checkpoint threshold, may overshoot by one bounded transaction | SqliteStore | Required checkpoint failure stops writer | Reopen/repair outside live Runtime | WAL tests |
| Wire frame / JSON | 16,384 bytes, depth 16, 1,024 values, string 512 | wire/reactor | Reject/close peer | reconnect | protocol tests |
| Socket clients | 8 | reactor | Additional accepted socket dropped | detach | code bound; exact process test missing |
| Owner/reactor mailboxes | 64 each | server | Backpressure/detach/fatal channel loss, never blocking owner | drain/detach/restart | server unit/M9B.8 |
| Per-client requests/replies/events | 8 / 8 / 16 | reactor | stop reading or detach | drain/reconnect/resync | server/client tests |
| Network sweep | 8 KiB and four frames/IO iterations; 5 ms loop sleep | reactor | Work continues next turn | next turn | server unit tests |
| Client hello/partial/write | 2 s absolute | reactor | detach | reconnect | client isolation tests |
| Session scopes / pending / terminal | 16 / 64 / 256 global; 8 / 32 per scope | SessionStore | structured capacity failure; pending never evicted | completion, 600 s terminal TTL, 1,800 s detached TTL | dedup/M9B.8 |
| Event replay / event size / scan | 1,024 / 4 KiB / 32 | EventLog | oldest eviction; explicit gap; oversize is owner error | resync | subscription tests |
| Frozen projections | bounded records, 5 s expiry | Application | busy/expired projection | release/expiry | measurement/config API tests |
| Connection history cursors | 8, 5 s retention; worker result cache 30 s | Application | `history_busy` | consume/disconnect/expiry | history API tests |
| Staged deployment / overlays | one candidate for 30 s / 32 overlays | DeploymentLifecycle | busy/expired/validation rejection | apply/expiry/restart | configuration tests |
| Configuration input/artifacts | TOML 64 KiB, 4,096 values, depth 8; 128 artifacts, 1 MiB total | loader | candidate rejection before activation | corrected candidate | validation tests |
| Shutdown | 2 s safety, 200 ms worker grace, 2 s flush, 200 ms client terminal window | ServiceHost/server | unsuccessful honest terminal, no indefinite join | process restart/manual hardware reconciliation | shutdown suites |
| Diagnostic log | **none today** | none | no retained troubleshooting evidence | M11.5 | missing |

No clearly unbounded production queue or retained collection was found. Apparent
dynamic maps/vectors are bounded by topology, session, frame, operation, or Recorder
limits. The important non-memory unboundedness is external duration: an OS/device or
trusted component can ignore its cooperative deadline. The owner remains bounded by
detaching/quarantining the worker, but the worker's actual completion is not
guaranteed. Diagnostic logging must not introduce a new unbounded queue or file.

## Priority and starvation audit

One `HostCore::service` turn currently executes:

```text
Recorder receipt poll
-> due safety / controller watchdogs / safe delivery / all transport progress
-> stopping and configuration fences
-> due thermal-model measurements
-> due Metakon read admission
-> due References
-> due controllers on distinct Good input
-> periodic managed sources
-> observation-driven managed transforms
-> semantic fact admission
-> Recorder receipt poll
```

Each lower-priority section checks whether safety became due and yields if so.
`Periodic::take` advances from the prior deadline and coalesces missed slots into one
actual-time opportunity; it does not replay a catch-up burst. The process server
runs this Host turn before polling Recorder/history replies and before up to four
rotated client requests, so clients cannot indefinitely precede native safety work.
The socket reactor and serial/managed/SQLite workers have independent bounded
mailboxes and do not borrow Runtime.

Existing tests prove missed-deadline coalescing, safety-before-native work, stale
input/tick-gap failure, held SQLite isolation, two managed-slot isolation, emulator
pressure, slow clients, M3 recovery and finite shutdown. They do not prove a
quantitative worst-case scheduling latency under the maximum simultaneous topology,
eight clients, full Recorder ingress, reconnect work and OS scheduling load. M11
should add a bounded stress acceptance and a non-default soak; it should not change
the scheduling algorithm unless that oracle finds a defect.

## Existing coverage and concrete missing coverage

### Strong existing oracles to retain

- M8 reconnect: `configured_physical.rs`, `configuration_reload.rs`, transport unit
  tests and `com_recorder_shutdown.rs` cover bounded recovery/offline, old-worker
  retirement, transient open retry, probe-before-release, generation fencing and
  durable lifecycle ordering;
- M9B.8: `m9b8_fault_acceptance.rs`, `request_deduplication.rs`,
  `client_isolation.rs` and `subscription_recovery.rs` cover reconnect replay,
  pending limits, malformed traffic, emulator pressure and server recovery;
- M9D: `crates/lab-core/tests/milestone9d_physical_output.rs` and
  `configured_physical_output.rs` cover final authority recheck, ACK/readback
  distinction, mismatch, readback timeout, safe-before-start policy and ambiguous
  safe no-resend;
- M4/Core control: `milestone4_runtime.rs`, `milestone4_algorithms.rs` and
  `host_scheduler.rs` cover unavailable/stale input, tick gap, lease expiry,
  lifecycle and no automatic rearm;
- Recorder: the `recorder_*` integration binaries plus `writer_barrier_tests` cover
  admission credit, gap/seal, Required deadline, worker panic, SQL rollback,
  WAL/checkpoint, process kill/reopen, cursor cancellation, the lost-wake oracle,
  held-writer isolation and finite failed shutdown;
- managed components: `milestone5_managed.rs`, `milestone9a_native.rs` and adjacent
  executor unit tests cover bounded PlainData/result validation, deadlines,
  stale/forged completion, generation fencing and fixed component capacity;
- server: `api_protocol.rs`, `protocol_foundation.rs`, `client_isolation.rs` and
  server unit tests cover wire bounds, trickle/write deadlines, event overflow
  detach, owner-mailbox detach retention and response-size fallback.

### Prioritized gaps

| Priority | Current deficiency | Risk | Nearby code/test | Smallest M11 correction |
|---|---|---|---|---|
| **BLOCKING FOR PREVIEW** | No bounded diagnostic logging exists | Real field failures cannot be reconstructed beyond current snapshots/Recorder semantics; panic/OS details disappear | `main.rs`, server, serial, Recorder and reconnect snapshots | Implement the bounded rotating M11.5 sink and targeted structured events; prove saturation, rotation, sink failure and finite shutdown |
| **BLOCKING FOR PREVIEW** | No single end-to-end oracle combines physical resource loss or safe ambiguity, explicit reconnect, fresh safe establishment, and proof that the controller never automatically rearms | Individually correct subsystems could compose incorrectly at the recovery boundary | M8 reconnect + M9D output + runtime lifecycle tests | Add software-only configured physical test covering loss/ambiguity through reconnect and explicit reset/resume; change production only if red |
| **IMPORTANT** | Managed worker channel disconnect/panic is treated like an empty completion; a dead slot remains occupied and two panics can permanently remove component capacity | Silent degradation and poor diagnostics; dependent control eventually fails but executor health is unclear | `ManagedExecutor::try_poll`, M5 deadline, runtime shutdown fake | Detect finished/disconnected slots, report a typed terminal completion/health state without spawning unbounded replacements; add real panic and never-return tests |
| **IMPORTANT** | Automatic physical codec failure is tested in generic fake transport, but not through configured COM composition and public resource state for every class | Fault class/recovery expectations can drift between adapter and configured lifecycle | codec/M3 tests, `configured_physical` | Add table-driven configured-software cases for CRC, short/silence, disconnect and wrong probe; verify Unavailable, Offline/quiesced, no queue growth and explicit reconnect rule |
| **IMPORTANT** | Exact Windows permission/read-only/disk-full behavior and an explicit crash-archive integrity acceptance are absent | Platform error classification or archive collection could surprise preview users | transaction/quota/checkpoint/process-reopen suites | Add deterministic filesystem/open error cases where portable; run an offline `PRAGMA integrity_check` acceptance on clean and killed-process fixtures without adding a live schema/migration feature |
| **IMPORTANT** | Exact eight-client saturation, reactor failure with active Recorder/control, and shutdown with all client queues pressured are not one process-level oracle | Capacity code is bounded but full-system release behavior is not frozen | server unit, client isolation, M9B.8 | Add one process-level max-client/slow-peer/native-progress/shutdown test |
| **IMPORTANT** | No bounded soak covers long cadence, repeated reconnect, long Recorder growth, long subscription/resync or future log rotation | Unit tests do not expose drift, handle/file growth or rare scheduling races | host scheduler, M8, Recorder/process tests | Add opt-in bounded soak profiles and a short CI stress analogue; do not put 24 h tests in default `cargo test` |
| **NICE TO HAVE / DEFER** | Autonomous resource loss has durable Unavailable measurements but no dedicated durable resource-transition fact with low-level class | Archive interpretation may require inference from measurements and operations | Runtime measurement facts, host resource events | Consider one bounded semantic runtime/resource event using existing schema; keep byte/error detail diagnostic-only |
| **NICE TO HAVE / DEFER** | No automatic ambiguous-write reconciliation | Hardware could be known safe only by an explicit future policy | M9D fail-closed latch | Defer. Current no-retry/no-rearm behavior is safer and accepted |

The first two items are release-gate gaps. The managed-worker issue is a concrete
implementation deficiency but is not a new physical safety authority: a dead
component cannot commit state or bytes, and its dependent controller fails closed.

## Software versus Arduino fault plan

All normative M11 behavior can be established in deterministic Rust without an
Arduino:

- bad CRC, wrong address/type/length and short frame;
- finite delay, silence and read timeout;
- disconnected adapter and failed recovery;
- WRITE admitted/started but ACK suppressed;
- valid ACK with absent, invalid, delayed or wrong readback;
- generation replacement and late old completion;
- queue pressure, worker hang, reconnect failure and shutdown deadlines.

A later controllable Arduino/USB serial fault emulator would provide useful
supplementary physical evidence for:

- Windows COM disappearance/re-enumeration and cable disconnect timing;
- real USB/driver timeout and buffered-byte behavior;
- device applies WRITE but intentionally suppresses ACK;
- real delayed/wrong readback and device reset during a transaction;
- repeated physical reconnect/handle-release cycles.

Arduino is **not required before developer preview**. It cannot replace the
software oracles because physical timing is nondeterministic, and M8/M9D already
provide accepted real-device baseline evidence. It may be separately authorized as
test infrastructure later; the educational Arduino furnace remains outside M11.

## Proposed M11 implementation sequence

### M11.2 — acquisition, executor and owner-progress hardening — SOL_HIGH

**Exact deficiency:** configured fault-class coverage is fragmented, managed worker
death is not explicitly surfaced, and maximum-load owner latency is not frozen.

**Scope:** `serial.rs`, `managed_executor.rs`, `host/scheduler.rs`,
`runtime/managed_components.rs`, configured physical/host scheduler/runtime shutdown
tests. Add only typed internal health/failure handling required by red tests.

**Deliver:** configured CRC/short/silence/disconnect matrix; real managed panic/hang
oracles and bounded dead-slot semantics; combined required-work starvation test;
unchanged poll cadence/order, retry policy and public API.

**Risk:** HIGH reasoning/concurrency, hence SOL_HIGH.

### M11.3 — control and physical-output recovery acceptance — SOL_HIGH

**Exact deficiency:** individual M8/M9D guarantees are strong, but the combined
loss/ambiguity -> reconnect -> safe -> explicit controller recovery lifecycle is
not one oracle.

**Scope:** Core controller/OutputAuthority/physical-I/O tests and configured physical
output integration tests. Prefer tests and rustdoc; production changes only for a
reproduced correctness defect.

**Deliver:** complete before-start/started/ACK/readback matrix; offline safe
obligation; reconnect does not clear controller failure or auto-rearm; explicit
reset/resume required; no M9D semantic change.

**Risk:** HIGH safety boundary, SOL_HIGH.

### M11.4 — Recorder/storage failure and recovery acceptance — SOL_HIGH

**Exact deficiency:** core durability coverage is already extensive, but platform
filesystem classes and explicit integrity acceptance are not consolidated.

**Scope:** Recorder fault harness and integration/process tests; no schema or
transaction-policy change.

**Deliver:** deterministic lock/permission/quota/checkpoint/write/close classification
where reliable; clean and killed-process reopen plus offline integrity check;
explicit structurally-readable versus semantically-complete assertions; preserve
Required and BestEffort policies.

**Risk:** HIGH concurrency/durability, SOL_HIGH. This combines Recorder, SQLite and
crash recovery; separate submilestones would add ceremony without a new owner.

### M11.5 — bounded diagnostic logging — SOL_HIGH

**Exact deficiency:** the subsystem is absent.

**Scope:** one process-level logging module, startup/CLI-or-deployment selection,
bounded writer, rotation, panic hook and focused call sites in Service/server,
serial/reconnect, OutputAuthority orchestration and Recorder. Core should remain
logging-facade-independent unless a minimal neutral diagnostic event is justified.

**Deliver:** fixed queue, record and file/retention bounds; level filtering;
rotation; sink failure/drop accounting; startup/shutdown/panic/failure records;
no Runtime blocking and no Recorder log dumping.

**Risk:** MEDIUM implementation but HIGH cross-cutting failure-boundary reasoning;
use SOL_HIGH for the first implementation slice. Mechanical call-site completion
can use SOL_MEDIUM only after the sink contract is frozen.

### M11.6 — Application/process pressure hardening — SOL_MEDIUM

**Exact deficiency:** exact socket-capacity and all-queues-pressured process shutdown
are not frozen end to end; new diagnostics also need server integration tests.

**Scope:** server/reactor/Application tests, not new operations or DTOs.

**Deliver:** MAX_CLIENTS+1, slow reply/event peers, malformed flood, history and
emulator pressure, reactor failure, disconnect cleanup and finite shutdown while
native/Recorder work advances. Preserve 42 operations, 25 capabilities and all
bounds.

**Risk:** MEDIUM, SOL_MEDIUM.

### M11.7 — consolidated failure matrix and preview gate — SOL_MEDIUM

**Exact deficiency:** guarantees are distributed across milestone reports and no
bounded soak/preview manifest exists.

**Scope:** focused short stress tests, opt-in/manual soak scripts or documented
commands, this matrix updated to final code, diagnostic collection acceptance and
evidence/hash verification.

**Deliver:** deterministic mandatory suites; optional 24 h polling/Recorder,
repeated reconnect, long subscription and log-rotation soaks; final
GUARANTEED/BEST-EFFORT/NOT-GUARANTEED table.

**Risk:** MEDIUM verification/integration, SOL_MEDIUM.

### M11.8 — external review — SOL_HIGH

Independent safety, durability, boundedness, diagnostics and API review. No M12 gate
is crossed automatically.

No stage should be skipped. The sequence is already minimized by combining managed
executor/starvation with acquisition in M11.2, all Recorder/SQLite/crash work in
M11.4, and API/emulator/process pressure in M11.6. M11.3 remains separate because a
small diff in ambiguity or rearm semantics deserves an isolated safety review.

## Developer-preview technical acceptance criteria

The system is functionally complete and hardened enough for developer preview when:

1. there is no known critical correctness, ownership, durability, boundedness or
   physical-output safety defect;
2. Runtime remains the sole authoritative mutable experiment owner and every worker
   remains non-authoritative;
3. mandatory debug/release/lint/rustdoc and focused fault suites are deterministic;
4. configured transport loss, malformed protocol, stale generation and explicit
   reconnect have one accepted software matrix;
5. unavailable/stale input, tick-gap and lease expiry fail controllers closed;
6. `send_started + ambiguous` is never blindly retried, ambiguous safe remains
   latched, and reconnect/safe transition never automatically rearms a controller;
7. shutdown is finite and reports incomplete safe, transport, managed-worker or
   Recorder cleanup honestly;
8. Required Recorder failure trips authority/control, BestEffort loss is truthful,
   and committed-prefix/gap/seal/crash expectations are explicit;
9. clean and killed-process archives pass the defined reopen/integrity acceptance,
   while semantic completeness still requires seals and complete coverage;
10. diagnostic logs have fixed queue/record/file/retention bounds, rotate, survive
    sink pressure without blocking Runtime, and remain separate from Recorder;
11. malformed, slow, full and dead clients cannot stop native work; session,
    subscription, history and emulator pressure release capacity as specified;
12. managed worker panic/hang cannot silently restore state, seize authority, block
    Runtime, or make shutdown wait indefinitely;
13. short stress and selected opt-in soak checks show no cadence catch-up burst,
    queue/file growth beyond bounds, stale-generation commit, or resource leak;
14. the exact 42-operation, 25-capability, protocol/error/bounds contract, SQLite
    schema, scheduler order, Metakon wire behavior and accepted evidence hashes are
    unchanged;
15. M12 documentation polish and packaging remain a later explicit gate.

## Final M11.1 conclusion

The accepted post-M10 architecture is already fail-closed at the important control,
output and Required Recorder boundaries. M11 should not redesign it. The minimum
hardening program is to add bounded diagnostics, close the managed-worker death
hole, consolidate configured physical fault/recovery oracles, add targeted
Recorder/platform and full-process pressure acceptance, and finish with bounded
stress/soak evidence. Arduino hardware is supplementary, not a prerequisite.
