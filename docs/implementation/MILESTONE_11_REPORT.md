# Milestone 11 implementation report

## M11.3 — controller, OutputAuthority and physical-output recovery hardening

### Status and scope

```text
M8-M10: ACCEPTED
M11.1-M11.2: COMPLETE
M11.3: COMPLETE
M11.4: NOT STARTED
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
gate is now complete in the M11.3 section above. Recorder/platform hardening remains
M11.4, bounded diagnostic logging remains M11.5, and additional full-process
capacity/soak work remains M11.6-M11.7. No logging subsystem or Arduino support was
started in M11.2.

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
