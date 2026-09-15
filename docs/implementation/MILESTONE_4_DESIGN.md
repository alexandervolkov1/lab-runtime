# M4 — deterministic native control pipeline design

Status: original M4 implementation contract; the reviewed Warming, native lease
renewal and independent safety-service corrections are specified in
[M4 lifecycle review](MILESTONE_4_LIFECYCLE_REVIEW.md) and reported in
[M4 implementation report](MILESTONE_4_REPORT.md). M3 is complete.
Scope ends after native plant/EMA/Reference/PID/lifecycle verification. No M5 code.

## Components and ownership

Runtime remains the sole mutable application owner. M4 adds a native thermal-plant
instrument, a bounded Reference registry and a native-controller registry. Algorithms
are small reusable value/state types in Core; they own only private mathematical
memory. They receive typed values and monotonic time and have no transport access.

The complete virtual path is:

```text
ThermalPlant temperature Sample -> controller-owned native EMA
    -> native PID <- Runtime-owned Fixed/Ramp Reference
    -> OutputProposal -> existing OutputAuthority
    -> simulated trusted Begin/Complete -> ThermalPlant heater power
```

The virtual effect is applied only when a Dispatch passes BeginDispatch. Its separate
completion then records successful simulated evidence; a failed/ambiguous completion
cannot prove that an already accepted effect did not occur. Generic ConfigureParameter cannot actuate.
This fixture is deterministic test evidence for authority composition, not hardware
safety or a physically accurate furnace model.

## Thermal plant

Add a distinct M4 instrument so the exact M1 time-function fixture remains compatible.
The first-order model uses the exact solution over each uneven interval:

```text
equilibrium = ambient + gain_per_percent * heater_percent
alpha       = 1 - exp(-dt / time_constant)
temperature = previous + alpha * (equilibrium - previous)
```

Configuration requires finite values, positive time constant/gain, initial temperature
inside descriptor limits and percentage output 0..100. Time is monotonic. Validation
and finite-result checking precede state commit. Refresh advances only this plant and
emits a normal typed Sample. Applying heater power is private to Runtime's trusted
post-arbiter virtual dispatcher.

## EMA

For time constant tau and elapsed `dt`, use the numerically stable equation:

```text
alpha = -expm1(-dt / tau)
y     = previous + alpha * (input - previous)
```

The first good input initializes the value without inventing a prior interval. A
configured good-sample count declares WarmingUp until reached; output remains visible
for diagnostics but controllers require Ready. Unavailable input returns explicit
Unavailable and does not advance filter state. Exact unit identity is required.
Nonfinite values/results, invalid quality/value combinations and non-increasing good
timestamps fail without partial mutation. Retuning validates positive finite tau and
preserves value/time/count; no hidden reprocessing or conversion.

## References

FixedReference and RampReference are independent Runtime components with explicit
unit and monotonic time. Fixed returns the same finite value while enforcing time
ordering. Ramp stores current, target, positive absolute rate and last update time.
It integrates elapsed runtime time toward either an upward or downward target and
clamps exactly at target. Retune first computes the continuous value at the supplied
time, then changes target/rate atomically. Controller pause does not pause a Reference;
the next explicit Reference tick/resume observes all elapsed monotonic time. Wall clock
does not enter the API. Reset/pause policy beyond this is not implemented in M4.

## PID

PID configuration contains finite gains and ordered finite output limits. On a good
update:

```text
error       = setpoint - measurement
P           = kp * error
I_candidate = I_previous + ki * error * actual_dt
D           = -kd * (measurement - previous_measurement) / actual_dt
u_raw       = P + I_candidate + D
```

The first sample has no elapsed interval: I is unchanged and D is zero. Derivative
uses measurement, so a setpoint-only step produces no derivative kick. Conditional
anti-windup rejects the candidate integral only when upper saturation and positive
error, or lower saturation and negative error, would push farther outward. Diagnostics
retain P/I/D, unconstrained and clamped output. All candidates must be finite before
committing measurement/time/integral/diagnostics. PID returns a number to the controller;
only the controller can wrap it in an OutputProposal, and only authority can dispatch.

## Controller lifecycle and freshness

States: Created, Ready, Running, Paused, Failed. Registration validates static IDs,
unit compatibility (measurement/reference), algorithm configuration, positive age/gap,
lease and proposal durations. Prepare checks current bindings but acquires nothing.
Start from Ready requires a latest Good finite sample, Ready EMA result, reference and
`age <= max_input_age`; it acquires a new Automatic lease. Tick requires increasing
controller time and `dt <= max_tick_gap`, rechecks freshness/quality, advances Reference,
EMA and PID, then submits exactly one proposal. The proposal TTL is bounded by lease,
input freshness and configured TTL.

Pause stops proposals first, revokes the lease, requests/completes the fixture safe
transition and enters Paused. Resume repeats freshness/reference checks, resets EMA/PID
memory explicitly (no integration across pause), and acquires a new lease/epoch. A
stale/unavailable/wrong-unit/nonfinite input or excessive tick gap enters Failed,
revokes with Trip and performs the independent fixture safe action. Successful safe
evidence leaves OutputAuthority FaultLatched after controller failure and never rearms.
Algorithm state and output state remain separate snapshots.

## Acceptance mapping

- EMA: unequal dt equation, warm-up, unavailable/wrong-unit, finite output, invalid
  update atomicity and state-preserving retune.
- Reference: fixed, upward/downward ramp, exact clamp, target/rate continuity,
  nondecreasing monotonic time and progress independent of measurement arrival.
- PID: first sample, actual-dt integral, derivative on measurement, no setpoint kick,
  upper/lower conditional anti-windup, finite diagnostics and invalid-update atomicity.
- Lifecycle: Created -> Ready -> Running, pause -> revoke -> safe -> Paused, resume
  gets a different epoch and reset memory; stale/unavailable/large-gap -> Failed and
  fault/revoke.
- Closed loop: repeated plant refresh/controller ticks move temperature materially
  toward a Ramp/Fixed target; every heater change corresponds to arbiter sent/evidence,
  and generic configuration still rejects heater power.

Tests are written before production bodies. M1–M3 tests must remain unchanged in
meaning. Completion runs workspace fmt, debug/release tests, clippy -D warnings,
rustdoc, finite demo and diff checks, then writes M4 report and the required
SOL_HIGH -> ASTRA_HIGH handoff. Stop before M5 design or implementation.
