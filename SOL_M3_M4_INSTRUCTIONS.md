Read `AI_HANDOFF.md` first, including the external reviewer response, then read:

* `AGENTS.md`
* `docs/implementation/MILESTONE_2_DESIGN.md`
* `docs/implementation/MILESTONE_2_REPORT.md`
* `docs/implementation/MILESTONE_3_DESIGN.md`
* the current M2–M6 implementation roadmap
* the relevant architecture and migration documents referenced by those files

You are now **SOL_HIGH**.

Your authorized scope is:

```text
M3 implementation
then
M4 design + implementation
then
STOP
```

Do not begin M5.

Before changing production code:

1. verify `git status`;
2. inspect recent commits;
3. rerun the existing M2 baseline;
4. confirm that the expected baseline is 38 passing workspace tests;
5. preserve all M1/M2 safety contracts unless a concrete contradiction is found.

If implementation exposes an unresolved architectural problem involving output authority, transport ownership, write ambiguity, safety evidence, lifecycle or trust boundaries:

* stop at a clean commit boundary;
* update `AI_HANDOFF.md`;
* set `STATUS: WAITING_FOR_REVIEW`;
* recommend `ASTRA_HIGH`;
* describe the exact conflict;
* do not silently redesign the architecture.

---

## Mandatory M3 correction: engineering units must become extensible

The current closed Rust enum:

```rust
Unit {
    Celsius,
    Percent,
    Pascal,
    Unitless,
}
```

was an acceptable M1 simplification, but it must **not** become the permanent extensibility boundary.

M3 is the milestone that proves data-driven instruments can introduce new parameter metadata without requiring changes to Rust Core. Therefore a new engineering unit such as:

```text
V
mV
A
mA
W
rpm
sccm
slm
mbar
Torr
K
mm
```

must be representable by a validated data-driven instrument definition **without adding a new Rust enum variant and recompiling `lab-core`**.

### Required unit model

Replace or evolve the closed unit representation into a small extensible domain representation.

The exact Rust type is your implementation decision, but it should conceptually provide:

```text
stable/canonical unit identity
human-readable symbol
bounded validated representation
cheap equality comparison
```

A design similar in spirit to:

```rust
UnitId("degC")
UnitId("percent")
UnitId("Pa")
UnitId("sccm")
```

is appropriate, but the exact names/types are not prescribed.

Convenient predefined constants or constructors for common units are welcome if they improve readability, for example conceptually:

```rust
Unit::CELSIUS
Unit::PERCENT
Unit::PASCAL
Unit::UNITLESS
```

provided they use the same extensible representation as data-driven units rather than reintroducing a closed enum.

### Explicit non-goals

Do NOT implement:

```text
dimensional analysis
SI type algebra
automatic unit conversion
prefix conversion
Pa ↔ Torr conversion
°C ↔ K conversion
a third-party physical-units framework unless independently justified
```

For M3, units are explicit domain metadata and equality is intentional.

If two signals use different unit identities, they are different units until some future explicit transform converts them.

---

## OutputAuthority must not know about Percent

The central output authority currently contains a special assumption equivalent to:

```rust
proposal.unit == Unit::Percent
```

Remove that assumption.

The expected actuator unit must come from the **trusted actuator descriptor/binding metadata**.

The authority should conceptually validate:

```text
proposal unit
    ==
trusted actuator unit
```

not:

```text
proposal unit
    ==
Percent
```

For the current virtual heater:

```text
heater_power -> %
```

so `%` remains correct for that particular actuator.

For a future data-driven actuator:

```text
gas_flow -> sccm
```

the same arbiter must be capable of validating an `sccm` proposal without any `sccm`-specific Rust branch.

### SafeProfile semantics

Do not give `SafeProfile` its own competing unit identity unless there is a demonstrated need.

Its numeric:

```text
min
max
safe_value
```

should be interpreted in the trusted actuator's declared unit.

The binding between:

```text
ActuatorId
descriptor
expected unit
value specification
SafeProfile
```

must be validated when the actuator/output authority is configured.

A profile cannot silently reinterpret a `%` actuator as `sccm`, or vice versa.

The trusted descriptor/binding remains authoritative for parameter type/unit semantics.

---

## Keep actuator value scope intentionally narrow

Do not use this correction as an excuse to build a generic actuation language.

For M3/M4, supporting a **numeric scalar actuator** is sufficient.

Do not prematurely generalize output execution to:

```text
Boolean actuator
Enum actuator
structured action
multi-value actuator
arbitrary commands
```

unless a current acceptance test actually requires it.

If a descriptor declares an actuator value kind unsupported by the current output path, reject it explicitly during binding/admission rather than accepting it partially.

---

## Required unit acceptance tests

Add regression/acceptance coverage proving all of the following.

### U1. Existing heater behavior remains correct

A heater actuator declared as:

```text
unit: %
```

accepts a valid `%` proposal.

### U2. Wrong unit is rejected

The same actuator rejects a proposal carrying:

```text
°C
```

or another nonmatching unit.

### U3. Custom data-driven unit works

A data-driven fixture introduces a unit not compiled as a special Rust enum variant, for example:

```text
sccm
```

and generic discovery exposes it correctly.

A numeric actuator declared with that unit accepts a proposal with the same canonical unit identity through the normal authority path.

### U4. No Core modification per new unit

Demonstrate that a second custom engineering unit can be supplied by data/configuration without adding another Rust code branch or enum variant.

### U5. Unit metadata is bounded and validated

Reject unreasonable unit metadata according to a small explicit policy, for example:

```text
empty identity
blank symbol where disallowed
overlong strings
invalid representation if your selected format defines one
```

Do not overdesign the validation grammar.

### U6. Profile/binding mismatch is rejected atomically

If the trusted output binding/profile is inconsistent with the actuator descriptor, registration/configuration fails before authority becomes usable.

No partially configured authority may remain armed.

---

# Preserve these M2 invariants

The unit refactor must not weaken M2.

Keep all of the following:

```text
Runtime is the sole mutable authority owner.

Generic ConfigureParameter cannot actuate.

One actuator has at most one current owner.

Lease/epoch/instance fencing remains intact.

Proposal queueing is not physical sending.

Revocation invalidates ordinary queued work.

Authorization is checked again at the actual send boundary.

Safe output is not universally zero.

Requested, started/sent, acknowledged and readback remain distinct.

Ambiguous physical effect never becomes success.

Safe recovery never automatically rearms an actuator.
```

`Dispatch` from M2 is still simulation evidence, **not** a reusable transport permission.

`Complete` from M2 is still trusted deterministic simulation input. Do not expose a future wire/client operation that lets a caller fabricate:

```text
ReadbackVerified
```

or any other hardware evidence.

---

# M3 implementation

Implement the already fixed `MILESTONE_3_DESIGN.md` contract.

Use acceptance tests first.

The M3 implementation must prove:

```text
Transport owns byte movement and resource serialization.

Protocol owns framing, CRC and response interpretation.

Instrument owns domain semantics.

Runtime owns transport/executor lifecycle.

OutputArbiter remains the only authority for output-affecting operations.
```

## Byte transport

Use a narrow OS-independent byte adapter boundary suitable for:

```text
scripted fake/fault-injected adapter now
real serial adapter later
```

Do not add Windows COM dependencies to `lab-core`.

The fake adapter must expose actual attempted/accepted bytes so tests verify the real byte-initiation boundary rather than merely mocking domain success.

## Executor

Implement a bounded single-owner executor per resource.

Preserve the M3 design limits/policies unless implementation demonstrates a real problem.

Important behavior:

```text
one active transaction per serial resource
bounded queue
queue deadline
execution deadline
partial-write accounting
partial-read accounting
explicit recovery
binding generation
mapping revision
no unbounded completed log
safe capacity cannot be crowded out by ordinary work
```

## Actual output final gate

This is mandatory:

```text
typed OutputProposal
        ↓
OutputArbiter
        ↓
queued trusted intent
        ↓
transport backlog
        ↓
CURRENT AUTHORITY CHECK
CURRENT INSTANCE CHECK
CURRENT EPOCH CHECK
CURRENT DEADLINE CHECK
CURRENT BINDING GENERATION CHECK
CURRENT MAPPING REVISION CHECK
CURRENT UNIT/TYPE/LIMIT CHECK
        ↓
FIRST POSSIBLE BYTE
```

The safety check and beginning of the first bounded write attempt must be serialized by the trusted owner.

A revoke must not fit between the final check and byte initiation.

### WouldBlock / zero bytes

If a write attempt accepts **zero bytes**:

```text
no physical send evidence exists yet
```

Before retrying that attempt, revalidate authority again.

### Partial write

Once at least one byte was accepted:

```text
physical operation may have started
```

Revocation cannot undo that prefix.

Do not start a second operation.

Finish/recover according to the executor/recovery semantics while authority remains revoked for new normal work.

---

# Failure and retry semantics

Keep the M3 design strict.

Do not copy generic donor retry logic.

Distinguish:

```text
read-only operation
explicitly retryable read
output-affecting/non-idempotent operation
```

A write timeout does NOT prove that the effect did not occur.

After any possible output byte was transmitted:

```text
timeout
partial response
CRC failure
I/O error
late response ambiguity
```

must not cause blind write retry.

An ambiguous write:

```text
revokes ordinary authority
latches fault as specified
requires reconciliation/recovery
cannot confirm safe state
```

M2's terminal simulated `Complete` assumption does not apply directly to a raw real transport timeout.

---

# Metakon

Implement only the narrow M3 Metakon slice described by the design.

Preserve donor provenance in tests/docs.

Cover at least:

```text
CRC vectors
frame encoding
frame decoding
device/channel/register matching
length/type validation
bad CRC rejection
scaling
sentinel -> unavailable/fault quality
output representation
```

Do not copy donor runtime wrappers or concrete serial ownership.

The codec must remain pure with respect to transport ownership.

---

# Data-driven instrument proof

Implement one small strict definition format for already-supported primitives.

Its purpose is architectural proof, not creation of a new programming language.

It should be able to describe:

```text
parameter ID/name
value type
engineering unit
range
role
access
operation mapping
scaling
side-effect classification
```

Engineering units in this format must use the new extensible unit model.

Unknown/duplicate/invalid fields should be rejected according to the M3 design.

No arbitrary executable expressions.

No arbitrary raw output frames.

---

# M3 completion

Before declaring M3 complete:

```powershell
cargo fmt --all -- --check
cargo test --workspace
cargo test --workspace --release
cargo clippy --workspace --all-targets -- -D warnings
cargo doc --workspace --no-deps
cargo run -p lab-runtime
git diff --check
```

Write:

```text
docs/implementation/MILESTONE_3_REPORT.md
```

Update `AI_HANDOFF.md`.

If all M3 gates are green, continue directly to M4 under SOL_HIGH.

Do not switch models after M3.

---

# M4 implementation

Create/finalize:

```text
docs/implementation/MILESTONE_4_DESIGN.md
```

then implement M4.

M4 target:

```text
deterministic virtual thermal plant
        ↓
temperature Signal
        ↓
native EMA
        ↓
native PID ◄──── Fixed/Ramp Reference
        ↓
OutputProposal
        ↓
OutputArbiter
        ↓
virtual heater actuator
```

No Lua.

No Babashka.

No recorder.

No GUI.

## EMA

Keep the algorithm easy to study.

Document in English:

```text
equation
time constant
uneven-dt alpha
warm-up behavior
quality behavior
validation-before-state-commit
```

Preserve useful donor mathematical behavior, not donor architecture.

## Reference

Implement:

```text
FixedReference
RampReference
```

Reference uses monotonic Runtime time and is independent of measurement arrival.

Document:

```text
start
progress
target
rate
clamp
retune continuity
pause/reset policy where implemented
```

Do not bake ramp logic into PID.

## PID

Implement native PID independent of transport.

Preserve and test:

```text
actual dt
derivative on measurement
no setpoint derivative kick
conditional anti-windup
validation before state commit
P/I/D diagnostics
effective/authorized output feedback where needed
```

PID produces `OutputProposal`.

It never writes the actuator directly.

## Controller lifecycle

Implement only the required lifecycle.

Controller state and output authority remain separate.

Pause must revoke authority and request safe transition.

Resume requires fresh valid input, valid Reference, a new authority generation/lease and explicit algorithm-state policy.

Do not silently integrate across a long pause.

## Freshness

A Good sample is not usable forever.

Add explicit monotonic freshness policy.

Stale/unavailable/bad-quality control input must result in the defined controller fault/revoke behavior.

---

# M4 acceptance

Cover at least:

```text
EMA unequal dt
EMA warm-up
EMA unavailable input
EMA finite output

FixedReference
Ramp upward
Ramp downward
Ramp clamp
Ramp retune continuity
Reference monotonic time independent of wall clock

PID first sample
integral with actual dt
derivative on measurement
no setpoint derivative kick
upper anti-windup
lower anti-windup
invalid update does not partially mutate PID state

controller start
controller pause -> revoke -> safe
controller resume -> new authority
stale measurement -> fault/revoke
unavailable measurement -> fault/revoke

closed loop moves virtual temperature toward Reference
all actuator effects still pass OutputArbiter
```

Update English rustdoc/comments as part of each logical implementation step.

The user is studying this code. Prefer explicit readable intermediate calculations over compressed clever code.

---

# Documentation requirements

All Rust:

```text
//! module docs
/// public rustdoc
// explanatory comments
TODO
FIXME
rustdoc examples
```

must be in English.

Document public API meaningfully.

For important private state-machine/algorithm helpers, explain non-obvious ownership, state, time and safety invariants.

Do not write comments that merely restate the code.

Keep `missing_docs` enforcement active.

For PID/EMA/CRC/Ramp/transport recovery, comments and rustdoc should make the implementation educational enough that a Rust learner can understand both:

```text
what the code does
why the architecture requires it
```

---

# Git

Use small logical commits.

Do not squash M3/M4 into one giant commit.

Do not modify the donor repository.

Do not introduce M5 code.

---

# Required stop after M4

After M4 is complete and all checks are green:

1. write `docs/implementation/MILESTONE_4_REPORT.md`;
2. update `AI_HANDOFF.md`;
3. include all relevant M3/M4 commits and verification results;
4. record any deviations;
5. set the model handoff:

```text
Current model: SOL_HIGH
Next model: ASTRA_HIGH

Reason:
M3 transport/protocol and M4 native control pipeline are complete.
The next milestone introduces the embedded Lua trust/isolation boundary.

Resume from:
M5 design only.

Do not start:
M5 implementation.
```

6. print a clear `MODEL HANDOFF`;
7. STOP.

Do not begin M5 design or implementation.

The user will return to the external reviewer before continuing.
