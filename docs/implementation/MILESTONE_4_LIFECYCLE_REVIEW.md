# M4 lifecycle corrections — design and Sol acceptance contract

Status: design fixed for the next SOL_HIGH implementation phase, 2026-09-15.
Neither correction is implemented by this document. External review accepted M3
and the deterministic M4 POC subject to these prerequisites before M5 code.
This document supersedes the startup/freshness/lease details in
[M4 design](MILESTONE_4_DESIGN.md), not its algorithm or transport contracts.

## 1. Baseline and findings

Baseline HEAD: `21d5cde`. `cargo test --workspace` passes 72 tests and
`cargo clippy --workspace --all-targets -- -D warnings` exits 0. The original
working changes were the user's reviewer response and Astra instructions; they
are preserved in the phase-input commit. No donor access or code change is needed.

Two concrete paths explain the review findings:

- `Runtime::start_or_resume_controller` resets EMA, supplies exactly one sample,
  then rejects any status except Ready. `Ema::new` accepts larger warm-up counts.
  Repeating Start discards the partial progress, so it cannot resolve the mismatch.
- Start acquires one finite lease. `tick_controller` never renews it, while
  `check_output_time` expires it before producer work at `now >= expires`.
  Eventually even healthy native control fails. Making the configured lifetime
  enormous would conceal the absence of a health contract.

Related implementation hazards in the same lifecycle handlers must be covered
while making these corrections: removing a controller from the map followed by
an early `?` can lose its diagnostic state on a safe-path error; a stale controller
must not release/Trip a different owner that acquired the actuator after its
revocation. These are error-path obligations of the corrections, not a redesign
of the M2/M3 authority state machine.

## 2. Chosen warm-up lifecycle

Add exactly one controller state, `Warming`. Existing Ready means descriptor/config
validation succeeded; Warming means an explicit Start/Resume is accumulating input
without actuator authority. Restricting all controllers to one-sample warm-up was
rejected because the reusable EMA deliberately supports a useful multi-sample policy.

```text
Created --Prepare--> Ready --Start--> Warming --N good samples--> Running
                                      |                         |
                                      +--------Pause------------+--> Paused
                                      |                         |
                                      +---input/timing fault-----+--> Failed
Paused --Resume/reset--> Warming
```

For `N == 1`, Start/Resume may complete the Warming transition within the same
command and return Running. This preserves the existing one-sample behavior.
Transition to Running acquires a lease but does not emit a PID proposal. The next
distinct valid sample on a subsequent Tick produces the first PID output, with
I unchanged and D zero. PID must not update at any point during Warming.

### Admission and safe output

Prepare validates the existing descriptor/unit/range bindings plus the duration
rules in section 4. Start/Resume first verifies that the target output is Disarmed,
has required safe evidence, and has no lease, pending/reserved or in-flight work.
It checks a fresh Good Float input, matching units, and evaluable Reference before
resetting algorithm state. A rejected Start/Resume leaves Ready/Paused and its
algorithm snapshot unchanged; the existing global watchdog may still expire other
authority before rejecting the command.

During Warming, Runtime holds a **preparation guard** for that actuator. This is
not a lease, an OutputOwner, or permission to propose/dispatch. Implement it by
bounded inspection of Warming controllers (maximum 64), not a new generic lock
service. Ordinary Acquire and another controller's Start/Resume on that actuator
return Busy. There is no preemption or automatic transfer. Safe requests and Trip
remain admissible and cancel the Warming attempt before changing output state.
RequestSafe cancels it to Paused; Trip cancels it to Failed. Neither runs PID or
implicitly restarts the attempt after safe completion.
Profile/binding mutation must reject while guarded, or require explicit Pause
first. Query and unrelated outputs remain available.

This small admission rule is necessary: merely checking Disarmed at each warm-up
tick would let another client arm the actuator while the controller still reports
Warming. The guard cannot be bypassed through public `Command::Output::Acquire`.
It disappears on Pause, failure, and successful acquisition. At the final sample,
Runtime rechecks all admission conditions, acquires through OutputAuthority, and
changes Warming to Running in the same serialized command. Failure to acquire
ends this attempt as Failed without claiming output authority.

### Samples, time and diagnostics

One Start/Resume/Tick consumes at most one latest sample; do not replay an entire
signal history inside a command. Count only distinct Good samples with increasing
source timestamps. On a Warming tick with the same still-fresh sample, remain
Warming without updating EMA, its count, or PID. Do not call EMA again with the
same timestamp. A later latest Unavailable must never fall back to an older Good.

Use the exclusive freshness rule `0 <= now - sample_time < max_input_age` throughout
Start, Resume, Warming, Running and renewal. Equality has no remaining output TTL
and is stale. This resolves the current discrepancy between config rustdoc, the
`>` test in `control_input`, and the later zero-TTL rejection. Future M5 derived
samples use their input observation time for this same rule, never callback finish
time. Unknown/future-dated/wrong-unit/nonfinite input cannot count as progress.

Warming and Running service times must increase; `dt > max_tick_gap` fails.
Backward/equal command time rejects without consuming a sample or renewing;
an otherwise valid forward service that reveals an excessive gap fails the loop.
Retain separate bounded fields for last service time, last accepted input time,
warm-up count (EMA already has it), activation time and last successful control
output. `last_tick` must have one documented meaning; do not use warm-up service
as evidence of a successful PID update. An unchanged sample may advance Warming
service time, but cannot extend input freshness or obtain a lease.

On a Warming input/timing failure: state becomes Failed, guard is released, no
lease is acquired, no proposal is emitted, and already-confirmed Disarmed output
stays Disarmed. Do not create an unnecessary actuator fault solely because an
algorithm failed before ever owning the output. An explicit Trip is different:
it still invokes the normal fault/safe path. Diagnostics distinguish these cases.

Pause from Warming cancels startup, releases the guard, and enters Paused; the
already-safe actuator needs no duplicate write. Pause from Running first stops
proposals/renewal, then revokes its current lease and drives the existing safe
path. If the watchdog already revoked that same binding, settle its pending safe
work without reacquiring. If another owner is now current, a stale controller
must not release or Trip it. Failure to complete safe work remains visible in
OutputSnapshot; it must not make a Paused/Failed controller disappear from registry.

Resume uses the same reset-and-warm path as Start. Old EMA count/value and PID
integral may remain visible while Paused, but are not reused for activation.
No integration across the pause. Failed is latched: fresh input alone does not
restart it; recovery/reset policy beyond existing commands is not added here.

### Independent Reference

Evaluate Reference at explicit Runtime `now` during valid Warming services and
again at activation. Reference does not depend on counted EMA samples. Missing
samples, Pause and controller failure never reset/freeze the shared Reference.
When next evaluated, Ramp covers the entire elapsed interval, including warm-up
and pause; equal-time evaluation by another consumer remains valid. Query still
does not advance it. An invalid Reference prevents activation and faults a live
attempt. Reference progress is its own committed state; it is not rolled back
because a subsequent controller operation fails.

## 3. Chosen renewal mechanism

Renewal is a crate-private OutputAuthority operation invoked only by Runtime's
native-controller path, after a successful Running tick. No public Renew command,
Lua callback, manual API or `OutputOwner::Automatic(number)` identity grants it.
The coordinator must match an actual registered Running NativeController, its
stored current lease, actuator and successful tick. A matching numeric owner ID
provided through `Command::Output` is insufficient.

Choose a **replacement token within the same ownership epoch**. Preserve the
existing exact-token equality check in `OutputAuthority::check_lease`:

```text
require now < current.expires
require supplied token == stored token (instance, actuator, owner, epoch, expiry)
require ArmedAuto, no fault/safe request, no pending/reserved/in-flight work
require finite validated lifetime L within installed profile maximum
new_expiry = checked_add(now, L), strictly greater than current expiry
replace stored token's expiry; return new token to the registered controller
```

No in-place mutation of a shared token exists: OutputLease is copied data. Old
copies become stale immediately even though the ownership epoch did not change.
Renewal does not bump epoch because the owner and binding have not changed; revoke,
pause, fault and rebind still do. Do not weaken token equality to ignore expiry.
Do not extend any queued proposal's deadline or rewrite an already reserved M3
intent. The empty-work precondition makes this a small M4 virtual-dispatch contract;
asynchronous physical-controller renewal is not added implicitly.

Renew from `now + L`, never `old_expiry + L`. Thus frequent ticks cannot accumulate
unbounded future authority. L remains at most profile.max_lease. A failed checked
addition is rejection, not saturation to Duration::MAX. Tests may reduce L to
seconds; the implementation must not insert an enormous hidden lifetime.

## 4. Health ordering, expiry and failure

Keep the global watchdog before producer work. At exactly expiry, revoke first;
neither a fresh sample nor a delayed successful callback can resurrect the lease.

The Running tick order is fixed:

1. Apply trusted nondecreasing Runtime time and expire old leases.
2. Check Running, exact current owner/token, strictly increasing service time,
   allowed tick gap, distinct fresh Good input and valid matching Reference.
3. Calculate/validate EMA and PID; derive a nonzero proposal TTL clipped to input
   freshness, configured TTL and the **current** lease deadline.
4. Propose, perform the existing final dispatch check, and complete the M4 virtual
   effect/evidence. An unsuccessful proposal/send/completion is not health proof.
5. With work settled, renew through the private authority operation; atomically
   retain the returned replacement token and successful tick diagnostics.

Renew once per successful tick. No separate keepalive timer renews native authority.
A service call with an unchanged sample is not Running progress and does not
renew; preserve the conservative existing Running failure behavior. Queries,
Warming, Paused, Failed and ordinary Output Tick never renew.

Validate `0 < max_tick_gap < lease_lifetime <= profile.max_lease` before Start
and Resume; retain other positive-duration/TTL validation. The strict inequality
allows the largest permitted healthy gap to finish before lease expiry. Host
scheduling still needs real margin; an OS deadline miss is not a reason to renew
late. Existing 100 s lease / 2 s gap fixtures remain admissible.

Stale/unavailable input, PID error, rejected output, or renewal failure stops
renewal, clears controller authority and performs the established Trip/safe path
for the authority that this controller actually owns. Failure after a virtual
dispatch cannot undo its effect; follow with safe output and keep failure evidence.
All exits reinsert/retain the Paused/Failed controller snapshot even if safe
dispatch itself fails or stays unresolved. Do not clear another owner's lease.

Finite authority without a ticking host cannot physically send safe output by
itself. An explicit Runtime safety service must still run independently of Lua,
native producer success and clients. M5 defines that bounded host service seam;
this correction does not claim an autonomous M4 scheduler or hardware watchdog.

Native lifetime belongs to Runtime, not the client that requested Start. A future
Babashka session disappearing does not stop healthy native progress. Conversely,
manual or externally supplied Automatic leases still expire normally. No M6
session protocol or Lua-controller renewal policy is implemented by this change.

## 5. Tests-first contract for Sol

Write regressions and capture the initial red results before editing production
bodies. Keep all 72 baseline tests meaningful. Use injected Duration and existing
virtual authority evidence, not sleep. Names below identify required scenarios;
one table row may need multiple focused tests.

| ID | Scenario and exact required observation |
| --- | --- |
| W1 | N=3, safe fixture, Good at t=0/1/2: Start returns Warming count1; Tick1 count2; Tick2 Running with new lease; no PID update or new normal sent/readback before Tick3. N=1 still starts directly. |
| W2 | Repeated Warming service with unchanged sample: count/value/source time unchanged, lease None; at exact max_input_age -> Failed without acquiring. |
| W3 | Missing, unavailable, future, wrong-unit and nonfinite initial input reject Start/Resume without resetting committed snapshots. During Warming invalid latest attempts fail, release guard and preserve already Disarmed safe evidence. |
| W4 | At a new Warming sample use its actual uneven interval in EMA; a backward/equal service time rejects without partial progress; gap above maximum faults. |
| W5 | During Warming, manual Acquire, forged Automatic Acquire and competing controller Start return Busy; unrelated actuator remains usable; no direct proposal or BeginDispatch gains ordinary authority. |
| W6 | Pause Warming yields Paused, no lease/no duplicate safe write. Resume resets count and warms N samples; after running pause/resume the next ownership epoch is newer, PID I/D have reset semantics. |
| W7 | Explicit RequestSafe/Trip cancels Warming; later delayed ticks cannot activate. Profile/binding changes are rejected while guarded. Failed acquisition cannot leave a guard or lease behind. |
| W8 | Ramp created at t=0 advances through warm-up/pause; a second consumer and equal-time evaluations agree; querying it never advances it. First PID setpoint uses activation's current Reference, not Start's old target. |
| W9 | Exercise safe-path error while Pausing/Failing: controller remains queryable with no claimed lease, output failure/unknown visible. Revoked old controller cannot revoke a newly acquired different owner. |
| R1 | Successful Running delivery at now<expiry renews to now+L; same instance/owner/epoch, new expiry; old token fails and new token works. No safe/arm transition or evidence fabrication occurs. |
| R2 | Exact expiry and one tick after expiry reject despite fresh input; watchdog revokes, controller fails/safe, no ordinary send or replacement lease. |
| R3 | Revoke, Pause, Trip or rebind then delayed renewal: reject; stale instance/epoch/token cannot resurrect ownership. |
| R4 | Stale/unavailable input, repeated sample, gap failure, invalid PID result and failed delivery: no renewal; safe/fault outcome, prior good timestamp never refreshed. |
| R5 | Fake-clock run with L=5 s, max_gap=2 s, fresh 1 s samples for at least 30 s: remains Running across six original lifetimes, each lease finite and expiry<=now+5 s, history bounded. |
| R6 | Rapid healthy ticks do not accumulate old_expiry+L; exact profile limit accepted, excess/zero/overflow rejected; max_gap>=L rejected at admission. |
| R7 | Authority unit tests: pending proposal, transport reservation or in-flight dispatch prevents renewal without changing token, work, deadlines or evidence. M3 partial-write and first-byte tests remain unchanged. |
| R8 | Manual and client-supplied Automatic token expire even while unrelated native PID renews. Merely knowing the same numeric controller owner ID gives no renewal privilege. |
| R9 | Controller after expiry retains failure diagnostics even if safe completion fails. A stalled producer eventually loses lease on independent safety service; pure queries cannot perform renewal. |
| R10 | A healthy registered native loop continues when the initiating test client/handle is dropped; no session object participates in lease renewal. This is a local ownership test, not a Babashka/M6 claim. |

## 6. Implementation boundary and handoff

Expected changed code areas next phase: `control.rs` lifecycle/snapshot fields,
`runtime.rs` admission, warm-up, renewal coordination and retained error state,
`output/authority.rs` private checked renewal, and focused regression tests. EMA/PID
equations, Reference independence, Unit representation, Metakon evidence and final
physical-byte gates remain intact. Document these private invariants in English.

Implement in small logical commits: warm-up tests/behavior, then renewal tests/
behavior, then error/fencing regression closure. Update the M4 design/report to
distinguish the original 72-test checkpoint from the corrected implementation.
Run fmt, debug/release workspace tests, clippy -D warnings, rustdoc, finite demo
and diff checks. Only after W1–W9/R1–R10 pass may Sol add Lua dependencies and
start [M5](MILESTONE_5_DESIGN.md). Astra stops at documentation.
