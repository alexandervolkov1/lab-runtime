# Milestone 11 implementation report

## M11.2 — acquisition, managed-executor lifecycle and starvation hardening

### Status and scope

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

M11.3 remains the separate combined control/output recovery gate, especially
ambiguity plus reconnect plus proof of no automatic rearm. Recorder/platform
hardening remains M11.4, bounded diagnostic logging remains M11.5, and additional
full-process capacity/soak work remains M11.6-M11.7. No logging subsystem or Arduino
support was started in M11.2.

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
