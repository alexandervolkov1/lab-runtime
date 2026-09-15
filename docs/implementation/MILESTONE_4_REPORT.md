# M4 completion report — 2026-09-15

Status: original 72-test checkpoint followed by corrected 92-test M1–M4 lifecycle
implementation. Scope: deterministic virtual thermal plant, native EMA and PID,
independent Fixed/Ramp References, explicit controller lifecycle and freshness-driven
safe failure. M1–M3 remain green. No M5 implementation is included.

## Corrected controller lifecycle and output ownership

The original 72-test checkpoint described one-sample Start/Resume and a finite lease
with no renewal. The [reviewed correction](MILESTONE_4_LIFECYCLE_REVIEW.md) is now
implemented. With EMA warmup greater than one, a controller enters Warming: it owns
no lease, performs no PID update and holds a bounded Runtime preparation guard.
Start and Resume each reset EMA/PID and consume at most one distinct fresh Good
observation per service. N=1 retains direct startup. Freshness expires at equality.

Successful Running virtual delivery is the only native renewal proof. Runtime calls
a crate-private checked authority operation after delivery settles. A replacement
copy keeps authority instance, owner and epoch and expires at the current progress
time plus the finite configured lifetime. The old token is immediately stale.
Manual/client Automatic owners expire normally. Pending, reserved or in-flight work
prevents renewal. Safe/pause/failure paths retain the controller even when safe
dispatch cannot complete; a stale controller cannot release a newer owner.

Explicit `ServiceSafety { at }` services watchdog and stale native dependencies,
attempts known Rust virtual safe actions, and polls bounded M3 resources independently
of producer callbacks. It does not renew or run algorithms. This host service seam
needs an actual caller; a finite software lease alone cannot physically send safe.
Physical Metakon still requires trusted Rust evidence, with ACK distinct from
readback.

| Contract | Focused correction evidence |
| --- | --- |
| W1–W4 | Three-sample direct-start, repeated input/exclusive expiry, invalid initial/late input and uneven service tests; existing EMA uneven-dt, unit and nonfinite tests |
| W5–W7 | Guard/competing controller, multi-sample pause/resume reset, RequestSafe/Trip cancellation and guarded profile tests |
| W8–W9 | Ramp spanning pause/warm-up, safe-dispatch Busy snapshots and later manual owner fencing |
| R1–R4 | Thirty-second finite replacement-token run, exact-expiry, stale input/gap, failed delivery and private authority revoke tests |
| R5–R8 | Six original lease lifetimes, current-time expiry, pending/reserved/in-flight rejection, manual owner independent expiry; M3 byte/partial-write tests unchanged |
| R9–R10 | Explicit safety service fake-clock tests, pure queries, retained controller snapshots and Runtime ownership independent of any client session |

Typed Runtime-produced observations reject nonfinite/wrong-unit input at their
existing construction/descriptor boundary. The independent EMA tests exercise those
cases directly. No OS hard-real-time or physical hardware-safety claim follows.

## Delivered at original 72-test checkpoint

- Runtime owns a distinct M4 ThermalPlant instrument so the M1 fixture retains its
  exact behavior. The plant integrates the first-order process with the exact uneven-
  interval exponential solution. Its heater mutator is crate-private and is called
  only after OutputAuthority creates a trusted Dispatch. Generic parameter writes
  cannot reach the heater.
- EMA uses `alpha = -expm1(-dt / tau)`, explicit units, a configured good-sample
  warm-up count and atomic state commits. Unavailable attempts never masquerade as a
  fresh retained value. Invalid time, quality, unit or finite checks do not partially
  mutate filter state.
- FixedReference and RampReference use caller-supplied monotonic Runtime time rather
  than measurement arrival or wall clock. Ramps move in either direction, clamp at
  target and preserve continuity when target/rate is retuned.
- PID uses actual elapsed time, derivative on measurement, no setpoint derivative
  kick and conditional upper/lower anti-windup. Its bounded snapshot exposes P/I/D,
  unclamped/clamped output and private memory. The kernel has no actuator or transport
  access; Runtime alone turns its scalar result into an OutputProposal.
- Native controllers have Created, Ready, Running, Paused and Failed states. Prepare
  checks descriptor role/type/unit, Reference unit, actuator limits and current M4
  virtual-dispatch support without acquiring authority. Start requires fresh Good
  float input and a valid Reference, initializes EMA/PID explicitly and acquires an
  Automatic lease.
- Each Tick requires increasing time within the configured gap, revalidates the latest
  attempt and freshness, advances Reference/EMA/PID, and submits one bounded proposal.
  Proposal lifetime is clipped by configured TTL and remaining input freshness.
- Pause revokes the current lease and completes the independent virtual safe action.
  It also settles safe output if the watchdog revoked the lease first. Resume checks
  fresh input, resets EMA/PID instead of integrating across the pause, and obtains a
  distinct authority epoch.
- Stale or unavailable input and excessive tick gaps enter Failed, clear the controller
  lease, Trip OutputAuthority and complete the safe action. Successful safe readback
  leaves the output FaultLatched; it never silently rearms.
- Queries return bounded owned Reference and controller snapshots without advancing
  time. Runtime registries are capped at 64 instruments, 64 References and 64 native
  controllers. Core remains std-only.

## Acceptance evidence at original checkpoint

Tests were committed before the corresponding algorithm and Runtime implementations.
M4 adds 12 tests, bringing the workspace total from 60 to 72:

| Area | Evidence |
| --- | --- |
| EMA unequal dt, warm-up and finite result | `ema_uses_uneven_dt_and_explicit_warmup` |
| EMA unavailable/unit/time atomicity | `ema_unavailable_and_invalid_inputs_do_not_mutate_state` |
| Fixed/Ramp up/down/clamp/time | `fixed_and_ramp_references_use_only_monotonic_time` |
| Ramp retune continuity/atomicity | `ramp_retune_is_continuous_and_invalid_time_is_atomic` |
| PID first sample, actual dt, D on measurement/no setpoint kick | `pid_uses_actual_dt_and_derivative_on_measurement` |
| PID two-sided anti-windup and atomic invalid update | `pid_conditional_anti_windup_and_atomic_validation_cover_both_limits` |
| Start/pause/safe/resume/new epoch/reset | `controller_start_pause_and_resume_use_distinct_output_authority` |
| Pause after watchdog expiry | `pause_settles_safe_output_when_watchdog_already_expired_the_lease` |
| Stale input and excessive gap fail safe | `stale_input_faults_controller_revokes_and_completes_safe`; `large_tick_gap_faults_even_with_a_fresh_sample` |
| Unavailable latest attempt is never replaced by old Good data | `unavailable_input_faults_without_reusing_old_good_value` |
| Complete plant → EMA → PID/Reference → authority → heater loop | `closed_loop_moves_temperature_toward_reference_through_arbiter` |

The closed-loop test advances the virtual process for 30 explicit seconds. Temperature
moves materially from 20 °C toward a 60 °C ramp target, normal sent/readback evidence
exists, and a generic ConfigureParameter attempt against the heater is rejected.

## Verification at original checkpoint

All commands exited 0 on Windows with rustc/cargo 1.95.0:

- `cargo fmt --all -- --check`
- `cargo test --workspace`: 72 passed, none failed or ignored.
- `cargo test --workspace --release`: 72 passed, none failed or ignored.
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo doc --workspace --no-deps`: no warnings.
- `cargo run -p lab-runtime`: finite M1 descriptor demo unchanged, normal exit.
- `cargo tree --workspace --edges all`: two workspace packages; Core has no external
  dependency, host dependencies remain serde/serde_json and their transitive crates.
- `git diff --check`; clean status before the completion-document edits.

The donor `D:\rust\com_port_reader` remained read-only and clean at
`50d3d1e3de84c650e1aa0ffbf1625044f794d315`. No physical COM or actuator test ran.

## Corrected verification

After the lifecycle and independent service corrections, all M1–M4 gate commands
exit 0: `cargo fmt --all -- --check`, `cargo test --workspace`,
`cargo test --workspace --release` (92 tests in each profile),
`cargo clippy --workspace --all-targets -- -D warnings`,
`cargo doc --workspace --no-deps`, `cargo run -p lab-runtime` (the original finite
M1 demo) and `git diff --check`.

## Commits

- `9416c49` — deterministic M4 design/acceptance contract.
- `a68426f` — red EMA/Reference/PID mathematics tests.
- `bb5d5b3` — native EMA/Reference/PID kernels.
- `53133ed` — formatted algorithm acceptance tests.
- `0d33c14` — red controller lifecycle/closed-loop tests.
- `b27072c` — Runtime-owned plant/controller lifecycle and authority integration.
- `1721d07` — safe settlement after watchdog-first revocation.
- `18e4f7a` — regression evidence for watchdog-expired pause.

M3 commits and its unit-extension correction are listed in
`MILESTONE_3_REPORT.md`; both milestone histories remain separate and reviewable.

## Known limitations and deviations of the M4 virtual proof

- The M4 trusted completion is deterministic simulation evidence, not hardware ACK,
  physical readback or a safety certification. A Dispatch effect may have happened
  even if later completion evidence fails; the authority model does not infer absence.
- Native controller delivery is deliberately limited at Prepare to the M4 thermal
  plant. Claiming immediate simulated readback for a Metakon/physical actuator would
  violate M3's transport boundary. A later scheduler must coordinate asynchronous
  physical completion without weakening authority.
- Runtime is synchronous and has no autonomous scheduler/clock. Callers explicitly
  refresh measurements, tick controllers and service safety. The executable remains the finite M1
  teaching demo rather than an autonomous closed-loop service.
- M4 implements numeric scalar EMA/PID and a single first-order plant. It does not add
  cascades, general graph editing, bumpless live controller retuning, persistent state,
  long-soak guarantees or a physical interlock.
- SafeProfile compatibility beyond descriptor/PID limits is enforced by the existing
  authority when acquiring/proposing. A mismatch fails the controller safely instead
  of being silently clamped.

The corrected M4 gate permits the already reviewed, bounded M5 Lua model/filter
implementation. Do not broaden physical actuation or skip the later external review.
