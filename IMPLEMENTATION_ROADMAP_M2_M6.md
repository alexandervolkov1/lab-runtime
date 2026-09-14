# lab-runtime — model-aware implementation roadmap, M2–M6

## Goal

Continue the completed Milestone 1 through the first real Babashka vertical slice.

The work is intentionally divided between two model roles:

```text
Astra High
    architecture
    safety-critical semantics
    trust boundaries
    concurrency/lifecycle design
    design review

Sol High
    bounded implementation
    algorithms
    codecs
    tests
    refactoring inside an accepted design
    documentation of implemented code
```

The agent must NEVER silently continue across a model handoff.

At every handoff:

1. finish the current authorized scope;
2. make the working tree clean;
3. update `AI_HANDOFF.md`;
4. print a clear `MODEL HANDOFF` message to the user;
5. STOP;
6. wait for the user to manually switch the model and start a new session/continuation.

---

# 1. Permanent communication file

Use exactly one dedicated communication file:

```text
AI_HANDOFF.md
```

This is the only special file used to communicate state between:

```text
Astra
Sol
external ChatGPT reviewer
user
```

Normal design reports remain in `docs/`.

The user may additionally send the external reviewer a fresh repository dircat.

---

# 2. Required AI_HANDOFF.md format

Keep:

```markdown
# AI Handoff

## Current milestone

Milestone:
Model used:
Status:
HEAD:
Working tree:

## Completed

Concise factual summary.

## Important decisions

Only decisions that materially affect later work.

## Domain/public API changes

Important types, Commands, Queries, Events, state machines and contracts.

## Important files

Files the next model or reviewer should inspect first.

## Verification

Exact commands and results.

## Commits

Logical commits produced in this phase.

## Known limitations

Anything intentionally incomplete.

## Review request

None.

Or:

STATUS: WAITING_FOR_REVIEW

Question:
Relevant files:
Options:
Recommendation:
Reason:

## Model handoff

Current model:
Next model:
Reason:
Resume from:
Do not start:

## Reviewer response

External reviewer may place a response here.

## Next step

Exactly one next authorized step.
```

Do not turn `AI_HANDOFF.md` into another architecture encyclopedia.

Keep detailed reasoning in:

```text
docs/implementation/
docs/architecture/
```

---

# 3. Mandatory model handoff message

Whenever a model change is required, print to the user:

```text
MODEL HANDOFF

STOP HERE.

Completed:
<scope>

Switch from:
<current model>

Switch to:
<next model>

Reason:
<short reason>

Before continuing:
Read AI_HANDOFF.md.

Resume with:
<exact next phase>

Do not continue this phase with the current model.
```

Then stop.

Do not begin even one commit from the next phase.

---

# 4. Coding documentation requirements

The user is studying the Rust code.

All Rust source documentation and implementation comments must be written in English.

This includes:

```text
//! module documentation
/// rustdoc
// implementation comments
TODO
FIXME
examples inside rustdoc
```

## Public Rust API

Every important public:

```text
struct
enum
trait
type
function
method
constant
public field
```

must have useful English rustdoc.

Rustdoc should explain non-obvious semantics such as:

```text
ownership
state
mutation
error behavior
time semantics
safety role
lifecycle
invariants
```

Bad:

```rust
/// Returns the state.
pub fn state(&self) -> State
```

Better:

```rust
/// Returns the current authorization state of the output channel.
///
/// This value describes permission to issue new actuator operations. It does
/// not prove that the physical actuator currently matches the requested value.
pub fn state(&self) -> OutputState
```

## Internal comments

Comments should explain WHY, not repeat syntax.

Good:

```rust
// Re-check the epoch immediately before dispatch. A proposal may have been
// authorized before a manual takeover or fault revoked its authority.
```

Bad:

```rust
// Check the epoch.
```

## Learning-oriented code

Prefer readable domain code.

Avoid compressed cleverness where explicit intermediate values teach the algorithm better.

This is particularly important for:

```text
PID
EMA
ramp/reference
CRC
transaction recovery
OutputArbiter
lease/TTL
Lua budgets
IPC event recovery
```

---

# 5. Documentation lint

Enable enforcement for missing public documentation.

Prefer:

```text
missing_docs = warn
```

or equivalent workspace/crate lint, with the existing CI/local:

```powershell
cargo clippy --workspace --all-targets -- -D warnings
```

so missing docs cannot quietly accumulate.

Do not broadly suppress documentation warnings.

---

# PHASE A — ASTRA HIGH

# M2: OutputArbiter

Current model:

```text
Astra High
```

This phase starts from completed M1.

This phase implements M2 and NOTHING from M3.

---

# 6. M2 purpose

Create the central Rust output authority before any real transport or controller exists.

Current M1 state:

```text
heater_power
    exists as actuator metadata
    but cannot be written
```

M2 introduces the only legitimate actuator path:

```text
producer
    ↓
OutputProposal
    ↓
OutputArbiter
    ↓
authorization / permit
    ↓
simulated dispatcher
    ↓
virtual actuator
```

No physical serial I/O yet.

---

# 7. M2 design first

Before production code create:

```text
docs/implementation/MILESTONE_2_DESIGN.md
```

Update:

```text
AGENTS.md
AI_HANDOFF.md
```

Current phase must become:

```text
implementation: milestone 2 output authority
```

---

# 8. M2 minimal concepts

Implement only concepts required for deterministic authority testing.

Likely concepts:

```text
ActuatorId / ActuatorHandle

OutputState

Unverified
SafePending
Disarmed
ArmedManual
ArmedAuto
FaultLatched

OutputOwner

OutputLease

OutputProposal

SafeProfile

OutputEpoch

Proposal TTL

DispatchPermit

DispatchOutcome / minimal evidence
```

Do not add network representation.

Do not add:

```text
cursor
wire request ID
network deduplication
IPC session
```

---

# 9. M2 fundamental invariant

Authorization at queue insertion is NOT sufficient.

Required flow:

```text
proposal
   ↓
arbiter validates
   ↓
permit
   ↓
queued simulated dispatch
   ↓
FINAL AUTHORITY / EPOCH / DEADLINE RECHECK
   ↓
effect
```

A proposal authorized before revoke must not execute after revoke merely because it is already queued.

This is one of the most important invariants in the project.

Document it clearly in source code.

---

# 10. M2 simulated dispatcher

Use deterministic simulation.

No OS thread is required merely to manufacture concurrency.

Tests must be able to hold/release dispatch intentionally and model:

```text
success
failure
ambiguous result
delayed result
late result after revoke
```

Make state transitions reproducible without sleeps.

---

# 11. SafeProfile

Safety is explicit.

Do not encode:

```text
safe == zero
```

as a universal rule.

The current virtual heater may define:

```text
safe power = 0 %
```

as its fixture-specific SafeProfile.

Other future actuators may define something else.

---

# 12. Ownership

Prove at least:

```text
manual producer
automatic producer
```

They use the same final validation path.

One actuator has at most one owner.

Ownership is time-bounded.

Epoch change fences previous authority.

---

# 13. M2 tests

Cover at least:

```text
new actuator begins Unverified

normal output cannot execute while Unverified

safe transition can establish Disarmed

failed safe transition does not produce Disarmed

ambiguous safe outcome does not claim known-safe state

one owner excludes another

manual and auto proposals share the same safety path

wrong type rejected

wrong unit rejected

nonfinite rejected

outside allowed range rejected

expired proposal rejected

expired lease rejected

revoke invalidates queued proposal

old epoch rejected immediately before dispatch

late success from old epoch cannot restore authority

ownership transition cannot resurrect old proposals

release/removal does not fabricate physical safety

existing configuration command cannot write heater_power around arbiter
```

Use injected monotonic time.

---

# 14. M2 quality checks

Run:

```powershell
cargo fmt --all -- --check
cargo test --workspace
cargo test --workspace --release
cargo clippy --workspace --all-targets -- -D warnings
cargo doc --workspace --no-deps
git diff --check
```

---

# 15. M2 report

Create:

```text
docs/implementation/MILESTONE_2_REPORT.md
```

Update:

```text
AI_HANDOFF.md
```

Include:

```text
state machine implemented
authority invariants
permit semantics
test list
known limitations
commits
verification
```

---

# 16. FIRST REQUIRED STOP

After M2 is completely green:

STOP.

Do NOT design or implement M3 in this Astra run.

`AI_HANDOFF.md` must contain:

```text
## Model handoff

Current model:
ASTRA_HIGH

Next model:
SOL_HIGH

Reason:
The central output authority and its safety semantics are now fixed.
The next phase is bounded transport/protocol implementation against the
existing architecture and migration contracts.

Resume from:
Milestone 3.

Do not start:
Lua, Babashka, recorder or GUI.
```

Print:

```text
MODEL HANDOFF

STOP HERE.

Completed:
Milestone 2 — OutputArbiter and deterministic simulated dispatch.

Switch from:
Astra High

Switch to:
Sol High

Reason:
M2 establishes the safety-critical authority contract. M3 and M4 are now
primarily bounded implementation work against committed contracts.

Before continuing:
Read AI_HANDOFF.md.

Resume with:
Milestone 3.

Do not continue with Astra in this run.
```

Then stop.

---

# PHASE B — SOL HIGH

# M3 + M4

The user manually switches to:

```text
Sol High
```

The first instruction to Sol should be:

```text
Read AI_HANDOFF.md, AGENTS.md and the current implementation reports.

Continue from the exact recorded next step.
Do not reinterpret completed Astra decisions unless implementation exposes
a concrete contradiction.
```

---

# 17. Sol responsibility rule

Sol should implement accepted architecture.

Sol MAY make ordinary local implementation decisions.

Sol MUST NOT silently invent a new high-level architecture.

If implementation reveals a real unresolved issue in:

```text
output authority
trust boundary
transport ownership
hardware ambiguity
failure state
lifecycle semantics
```

then:

1. stop at a clean commit boundary;
2. update `AI_HANDOFF.md`;
3. set:

```text
STATUS: WAITING_FOR_REVIEW
Next model: ASTRA_HIGH
```

4. describe the exact ambiguity;
5. stop.

Do not improvise an architectural framework to escape the ambiguity.

---

# M3 — Transport / Protocol / Instrument

## 18. M3 design

Sol creates:

```text
docs/implementation/MILESTONE_3_DESIGN.md
```

This is a bounded implementation design based on already accepted architecture.

It is not a fresh architecture exercise.

---

# 19. M3 goal

Prove:

```text
Transport owns byte movement/resource serialization

Protocol owns frames/CRC/request-response semantics

Instrument owns domain parameters/operations

OutputArbiter owns authority
```

---

# 20. M3 transport boundary

`lab-core` remains independent of Windows COM libraries.

Introduce only the narrow contract necessary to substitute:

```text
fake byte transport
real serial adapter later
```

The concrete OS serial adapter belongs around Core.

Do not introduce a generic transport framework for hypothetical protocols.

---

# 21. M3 transaction executor

Implement:

```text
single resource owner
bounded queue
one active transaction per serial resource
queue deadline
transaction deadline
clear response bounds
recovery semantics
```

Multiple logical instruments may share one resource.

They must not interleave transactions.

---

# 22. M3 write ambiguity

Document clearly:

```text
timeout != proof that a physical write did not occur
```

Separate:

```text
read retry
safe idempotent retry
ambiguous actuator write
```

Do not copy generic "retry three times" behavior from v1.

---

# 23. M3 narrow Metakon

Implement a narrow native Metakon codec/adapter using donor test knowledge.

Preserve provenance.

Need:

```text
CRC vectors
read/write frame construction
device/channel/register matching
type/length checks
corrupted CRC rejection
scaling
sensor sentinel -> explicit bad/unavailable quality
output representability
```

Do not copy old runtime wrappers.

---

# 24. M3 data-driven proof

Implement one deliberately small data-driven definition format for known protocol primitives.

It should prove a new device can define things like:

```text
parameter
type
unit
range
role
access
operation mapping
scaling
side effect classification
```

without changing Core.

Do not evolve the data format into a programming language.

---

# 25. M3 final dispatch protection

Every output-affecting transaction:

```text
proposal
→ arbiter
→ permit
→ transport queue
→ final permit check
→ bytes
```

No raw output bypass.

---

# 26. M3 tests

Cover:

```text
one resource owner
no interleaving
two resources remain independent
bounded queue
queue expiry
transaction timeout
partial frame
bad CRC
wrong response
late response
session recovery
no blind retry of ambiguous write
generation/rebind fences stale work
revoked queued write fails final permit check
```

No dangerous physical actuator testing.

---

# 27. M3 finish

Create:

```text
docs/implementation/MILESTONE_3_REPORT.md
```

Update handoff, but DO NOT switch models yet.

Continue directly with M4 under Sol High if all gates are green.

---

# M4 — Signals, Reference and native PID

# 28. M4 design

Create:

```text
docs/implementation/MILESTONE_4_DESIGN.md
```

---

# 29. M4 virtual plant

Upgrade the virtual fixture into a deterministic small thermal process.

Keep the model understandable.

The purpose is closed-loop behavior, not perfect furnace simulation.

---

# 30. M4 processing

Implement only enough processing infrastructure for:

```text
temperature source
    ↓
native EMA
    ↓
native PID
```

plus diagnostics.

Do not build a full visual graph editor.

---

# 31. EMA

Preserve donor mathematical intent:

```text
uneven dt
stable exponential alpha
explicit time constant
validate before state commit
```

Document equations and timing assumptions in English.

Handle bad quality and warm-up explicitly.

---

# 32. Reference

Implement:

```text
FixedReference
RampReference
```

as independent Runtime components.

Reference uses monotonic runtime time.

Reference must not be implicitly driven by one measurement stream.

Preserve useful continuity behavior for target/rate changes.

---

# 33. PID

Extract/rewrite the native donor PID kernel.

Preserve:

```text
actual dt
D on measurement
no setpoint derivative kick
conditional anti-windup
validate-before-commit
P/I/D diagnostics
```

PID returns:

```text
OutputProposal
```

PID has no transport access.

---

# 34. Controller lifecycle

Implement minimal lifecycle consistent with architecture:

```text
Created
Ready
Running
Paused
Failed
```

Output authority is a separate concern.

Pause:

```text
stop controller proposals
revoke output authority
safe transition
```

Resume:

```text
validate current input
validate Reference
obtain new lease/epoch
apply explicit reset/reinitialize policy
```

Do not blindly preserve old integrator state.

---

# 35. Freshness

Introduce explicit monotonic input freshness.

A controller cannot run forever on the last valid measurement.

Stale or invalid input must revoke/fault according to the defined policy.

---

# 36. M4 full pipeline

Acceptance target:

```text
virtual thermal process
        ↓
temperature
        ↓
EMA
        ↓
PID ← Ramp Reference
        ↓
OutputProposal
        ↓
OutputArbiter
        ↓
virtual heater
```

The virtual temperature should move toward the reference.

---

# 37. M4 tests

Cover:

```text
EMA uneven dt
EMA bad quality
EMA warm-up
EMA finite output

FixedReference
Ramp up
Ramp down
Ramp clamp
Ramp target/rate continuity
Reference monotonic time

PID first sample
integral using actual dt
D on measurement
no setpoint kick
anti-windup upper/lower
invalid update atomicity

controller start
pause -> safe transition
resume with new authority
stale input -> revoke/fault
bad quality -> revoke/fault

closed loop moves toward target
every actuator action goes through arbiter
```

---

# 38. M4 finish and SECOND REQUIRED STOP

Create:

```text
docs/implementation/MILESTONE_4_REPORT.md
```

Run all checks.

Update `AI_HANDOFF.md`:

```text
Current model:
SOL_HIGH

Next model:
ASTRA_HIGH

Reason:
The native safety/control pipeline is now established. The next milestone
introduces an embedded code-execution and trust/isolation boundary.

Resume from:
Milestone 5 design only.

Do not start:
Milestone 5 implementation.
```

Print:

```text
MODEL HANDOFF

STOP HERE.

Completed:
Milestones 3 and 4.

Switch from:
Sol High

Switch to:
Astra High

Reason:
Milestone 5 introduces embedded Lua, including sandboxing, execution budgets,
failure isolation and safety trust boundaries.

Resume with:
Design and review Milestone 5 only.

Do not begin Lua implementation with Sol yet.
```

STOP.

---

# PHASE C — ASTRA HIGH

# M5 DESIGN ONLY

# 39. Astra role for M5

Astra does NOT implement M5 production code in this phase.

It designs and reviews the Lua trust boundary.

Read all M2–M4 reports/code relevant to:

```text
OutputArbiter
transport
signals
controllers
Runtime ownership
```

---

# 40. M5 design questions

Resolve explicitly:

```text
What exactly may Lua implement?

What data enters a Lua callback?

What data may it return?

What persistent state may it own?

What libraries are available?

What filesystem/network/process/FFI capabilities are absent?

How are instruction limits implemented?

How is wall-clock execution bounded?

How is memory/state/output size bounded?

How are host calls bounded?

How does failure affect signal quality?

How does Lua controller failure revoke authority?

How does reload/generation fencing work?

How does safe action remain executable with Lua dead?

Can Lua ever produce arbitrary actuator bytes?

Answ
```
