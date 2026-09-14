# lab-runtime — implementation plan M2–M6

## Goal

Continue `lab-runtime` from the completed Milestone 1 through the first real Babashka vertical slice.

Authorized scope:

```text
M2 OutputArbiter + simulated dispatcher
M3 transport/protocol + narrow Metakon + data-driven instrument
M4 signal processing + Reference + native PID
M5 bounded embedded Lua extensions
M6 real local Babashka / external API slice
STOP
```

Do NOT proceed to:

```text
M7 recorder
GUI
physical production deployment
24/72 hour soak
full product parity
```

after M6.

A review is required at that boundary.

---

# 0. Read the current project first

Before changing code, read:

```text
AGENTS.md
README.md

docs/implementation/MILESTONE_1_DESIGN.md
docs/implementation/MILESTONE_1_REPORT.md

docs/architecture/HIGH_LEVEL_ARCHITECTURE.md
docs/architecture/EXTENSION_MODEL.md
docs/architecture/RUNTIME_AND_SAFETY_MODEL.md
docs/architecture/POC_PLAN.md
docs/architecture/OPEN_QUESTIONS.md

docs/migration/V1_TO_LAB_RUNTIME_MAP.md
docs/migration/REUSE_PLAN.md
docs/migration/MIGRATION_RISKS.md
docs/migration/POC_REFINEMENT.md
```

Also inspect all current Rust source and tests.

Run:

```powershell
git status
git log --oneline -20

cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

The current Milestone 1 behavior is the baseline.

Do not redesign it merely because a later milestone introduces more concepts.

---

# 1. Create one permanent AI handoff file

Create in repository root:

```text
AI_HANDOFF.md
```

This is the single dedicated communication file between the implementation agent and the external ChatGPT reviewer.

Keep it concise and current.

Use this structure:

```markdown
# AI Handoff

## Current state

Current milestone:
Status:
Current HEAD:
Working tree:

## What was completed

Short factual summary.

## Important implementation decisions

Only decisions that materially affect architecture or future work.

## Public/domain API changes

New or changed important types, Commands, Queries, Events and contracts.

## Files worth reviewing

A short list of the most important files and why.

## Verification

Commands run and exact results.

## Commits

Recent logical commits for the current milestone.

## Deviations from plan

None, or a precise explanation.

## Review request

None.

Or, if review is required:

STATUS: WAITING_FOR_REVIEW

Exact question:
Relevant files/symbols:
Options considered:
Agent recommendation:
Why the decision matters:

## Reviewer response

Leave the previous external reviewer response here until it has been consumed.

## Next step

Exactly what will be done next.
```

Rules:

1. Read `AI_HANDOFF.md` before beginning every new milestone.
2. Update it after every completed milestone.
3. Do not turn it into a full changelog.
4. Keep detailed design/history in normal docs; handoff only points to them.
5. If a genuine architectural fork cannot be resolved from existing architecture, write it under `Review request`, set `WAITING_FOR_REVIEW`, and stop at a safe commit boundary.
6. Do not ask for review for ordinary implementation choices that fit the existing architecture.
7. After consuming a `Reviewer response`, record the resulting decision in the appropriate design document and then continue.
8. The user may also provide the reviewer with a fresh dircat of the repository.

---

# 2. Documentation and teaching quality are now a hard requirement

The user will study the Rust source code.

Therefore source readability is a first-class requirement, not cleanup for later.

All Rust documentation and source comments must be written in English.

This includes:

```text
//! module documentation
/// public API documentation
/// documentation for important private abstractions
// explanatory implementation comments
TODO / FIXME text
examples inside rustdoc
```

Do not write Russian comments in Rust source.

## Rustdoc requirements

Every public:

```text
struct
enum
trait
type alias
constant where meaning is not obvious
public field
public function
public method
```

must have useful rustdoc.

Document:

```text
what the item represents
who owns its state when relevant
important invariants
error/failure semantics
whether an operation mutates state
time semantics where relevant
safety significance where relevant
```

Do not write useless documentation such as:

```rust
/// Gets the value.
fn value(...)
```

when the non-obvious question is what that value means.

For important modules use `//!` at the top explaining their role in the architecture.

For non-obvious private functions/types, add `///` or concise `//` comments where they help a learner understand the design.

## Comment requirements

Prefer comments explaining WHY:

```rust
// Re-check the epoch immediately before dispatch so a proposal that was
// authorized before a revoke cannot reach the actuator afterwards.
```

not comments restating syntax:

```rust
// Increment epoch.
epoch += 1;
```

Particularly comment:

```text
ownership transitions
state-machine transitions
generation/epoch fencing
deadline/TTL logic
unsafe ambiguity of hardware writes
scheduler decisions
signal quality propagation
PID state updates
Lua isolation/budget decisions
IPC reconnect/gap semantics
```

Keep code straightforward.

Prefer explicit intermediate variables and clear types over clever compressed iterator chains when the latter make the code harder to study.

No premature generic abstractions.

## Enforce documentation

Add an appropriate missing-docs lint for public Rust API, preferably at workspace/crate level.

The exact severity may be `warn` combined with the existing `-D warnings` verification, or an equivalent setup.

Do not silence documentation warnings broadly.

---

# 3. Per-milestone workflow

For every M2–M6 milestone:

1. Update `AGENTS.md` current phase.
2. Write a short design document BEFORE production implementation:

```text
docs/implementation/MILESTONE_N_DESIGN.md
```

3. Update `AI_HANDOFF.md` with the planned boundary.
4. Write acceptance tests first for the public/domain behavior where practical.
5. Implement in small logical commits.
6. Add/update source rustdoc and explanatory comments as part of the implementation, never as a postponed cleanup pass.
7. Run the milestone checks.
8. Write:

```text
docs/implementation/MILESTONE_N_REPORT.md
```

9. Update `AI_HANDOFF.md`.
10. Commit the report/handoff.
11. Proceed to the next authorized milestone unless:

* tests are not green;
* architecture is contradicted;
* the milestone acceptance gate is not met;
* `AI_HANDOFF.md` is in `WAITING_FOR_REVIEW`.

Use:

```text
1 logical change = 1 commit
```

Do not produce one giant M2–M6 commit.

---

# M2. Central OutputArbiter with deterministic simulated dispatch

## Purpose

Prove that no actuator-affecting operation can bypass one Rust authority boundary.

M1 already exposes actuator metadata but rejects actuator writes.

M2 turns the virtual actuator into an executable path only through OutputArbiter.

No physical hardware is involved.

---

## M2.1 Domain concepts

Introduce only the concepts now required.

Likely concepts include:

```text
ActuatorId / ActuatorHandle
OutputState
OutputOwner
OutputLease
OutputProposal
SafeProfile
OutputEvidence / DispatchOutcome
epoch
proposal TTL / deadline
```

Use the existing architecture vocabulary where it remains appropriate:

```text
Unverified
SafePending
Disarmed
ArmedManual
ArmedAuto
FaultLatched
```

Do not build persistence/network representations for these concepts yet.

Do not add cursor/request-ID/deduplication.

---

## M2.2 Simulated dispatcher

Implement a deterministic simulated dispatcher.

It must allow tests to model:

```text
successful output
failed output
ambiguous output
delayed/pending output
readback/acknowledgement distinction where needed
```

No threads are required merely to simulate concurrency.

An explicit deterministic queue/step API is acceptable and may be preferable for tests.

The crucial invariant:

> Authorization at enqueue time is insufficient.

Immediately before the simulated physical send, the dispatcher must re-check that the permit/epoch/deadline is still current.

---

## M2.3 Safe profile

Do not assume:

```text
safe == 0
```

Define a small explicit SafeProfile for the virtual actuator.

For the current virtual heater fixture, zero power may be selected as its specific simulated safe value, but the API must not encode zero as the universal concept of safety.

---

## M2.4 Ownership

Support at least deterministic test producers representing:

```text
manual owner
automatic owner
```

Controllers themselves do not exist yet.

Automatic owner in M2 is only a producer identity used to prove the authority contract.

One actuator has at most one current owner.

Lease expiry and epoch revoke must invalidate stale proposals.

---

## M2.5 Required acceptance tests

At minimum cover:

```text
output cannot execute while unverified/disarmed without authority

explicit safe procedure reaches Disarmed only after required success/evidence

one owner excludes another

manual and automatic use the same final validation path

wrong type/unit/range/nonfinite value is rejected

proposal after lease expiry is rejected

queued proposal authorized before revoke is rejected at final dispatch check

old epoch cannot execute after ownership transition

late result cannot resurrect old authority

safe failure does not become Disarmed

ambiguous dispatch is not reported as successful safe state

removing/releasing owner does not silently fabricate physical safety

existing ConfigureParameter cannot write heater_power around the arbiter
```

Use deterministic injected time.

---

## M2.6 Scope cut

Do NOT add:

```text
serial
Metakon
PID
filter graph
Lua
IPC
recorder
GUI
```

M2 ends when the virtual output safety path is convincing.

---

# M3. Transport, protocol boundary and narrow Metakon/data-driven slice

## Purpose

Prove:

```text
Runtime owns transport
Protocol owns bytes/framing
Instrument owns domain semantics
OutputArbiter remains the only actuator authority
```

Use the donor Metakon knowledge instead of inventing a fake claim of existing Modbus support.

---

## M3.1 Keep core independent of OS serial

`lab-core` must not depend on Windows COM or `serialport`.

Define only the narrow core/application contracts actually needed.

Concrete serial I/O belongs outside core, in the host/adapters.

A trait/interface becomes justified here because tests need a fake byte transport and production will later need a serial transport.

Keep it narrow.

---

## M3.2 Single-owner transport executor

Create a bounded single-owner transport/executor model.

It must guarantee:

```text
one transaction at a time on a serial resource
no interleaving between instruments sharing that resource
bounded queue
queue/transaction deadlines
clear ownership of reconnect/rebind
explicit cancellation/expiry semantics
```

Do not pretend that cancelling software undoes bytes already transmitted.

Document this prominently in English.

---

## M3.3 Transaction semantics

Separate:

```text
read operation
idempotent operation
non-idempotent / output-affecting operation
```

Do not blindly preserve the donor's generic three retries.

A timeout after sending an actuator write may have:

```text
unknown physical effect
```

and must not be automatically treated as "nothing happened".

---

## M3.4 Narrow Metakon codec

Extract/rewrite only a narrow tested Metakon slice from donor knowledge.

Preserve provenance in documentation/tests.

At minimum verify:

```text
CRC reference vectors
frame encode/decode
device/channel/register matching
response type/length validation
corrupted CRC rejection
measurement scaling
sensor sentinel -> explicit unavailable/fault quality
representability of an output write
```

Do not migrate the old concrete `SerialConnection` dependency into the codec.

Do not implement every register merely for parity.

---

## M3.5 Native Metakon Instrument adapter

Expose the selected Metakon subset through the same domain Instrument/descriptors used by M1.

The generic client must not need:

```text
if instrument == Metakon
```

for ordinary introspection.

Parameter roles and side effects remain explicit.

---

## M3.6 Minimal data-driven instrument proof

Add a deliberately small declarative definition path for an already-supported operation profile.

The purpose is to prove:

> A new instrument using known protocol primitives can be described without changing Rust Core.

Do not invent a programming language inside JSON/TOML.

Choose one simple representation and validate it strictly.

It should be capable of describing a useful small fixture such as:

```text
parameter key
type
unit
role
access
range
protocol operation mapping
scaling
known side effect
```

Complex protocol behavior remains native Rust.

---

## M3.7 Final permit check

For any output-affecting transaction:

```text
OutputProposal
    ↓
OutputArbiter
    ↓
permit
    ↓
transport queue
    ↓
FINAL PERMIT RECHECK
    ↓
bytes
```

This path must be visible and documented.

No alternative raw output path.

---

## M3.8 Testing

Use fake/fault-injected transport for automated tests.

Cover:

```text
two instruments share one bus without interleaving

different buses are independent at the model level

queue deadline

transaction timeout

partial response

bad CRC

late response after timeout

recovery before next transaction

no blind repeat of ambiguous actuator write

rebind/restart invalidates stale work

final arbiter epoch check prevents stale queued write
```

Physical COM smoke testing is optional and must not be presented as completed unless actual hardware was used.

No dangerous physical actuator tests.

---

# M4. Signal processing, Reference and native PID loop

## Purpose

Build the first real native control pipeline:

```text
Virtual thermal plant
       │
       ▼
temperature Signal
       │
       ▼
native EMA
       │
       ▼
native PID ◄──── Fixed/Ramp Reference
       │
       ▼
OutputProposal
       │
       ▼
OutputArbiter
       │
       ▼
virtual heater power
```

No Lua yet.

---

## M4.1 Upgrade the virtual fixture

The simplistic M1 time function has served its purpose.

Add a deterministic small virtual thermal plant suitable for closed-loop tests.

It is a test/runtime fixture, not a declaration that the simulated plant perfectly represents real hardware.

Prefer a simple understandable model.

If donor furnace equations are reused later, document exact provenance and differences.

---

## M4.2 Minimal signal processing

Do not create a universal graph editor.

Add only enough graph/component infrastructure for:

```text
measurement source
one native EMA
controller input
diagnostic signals
```

Establish the future SignalNode boundary without implementing every filter.

Signal quality must propagate explicitly.

---

## M4.3 EMA

Extract the useful EMA mathematics from donor knowledge.

Preserve:

```text
uneven dt support
numerically stable alpha
explicit time constant
validate-before-state-update
```

Add explicit quality/warm-up semantics.

Reject non-finite computed output.

Document retuning/state-retention behavior.

---

## M4.4 Independent Reference

Implement:

```text
FixedReference
RampReference
```

as independent runtime components.

Reference progress uses monotonic runtime time.

It is NOT driven implicitly by arrival timestamps of one measurement.

Preserve useful donor behavior:

```text
ascending and descending ramps
target clamp
target/rate changes can continue from the current reference value
```

Controller pause must not implicitly pause an unrelated shared Reference.

---

## M4.5 Native PID

Extract the donor PID numerical kernel without the old registry/InstrumentValue wrapper.

Preserve and test:

```text
actual dt
derivative on measurement
no setpoint derivative kick
conditional anti-windup
candidate validation before state commit
diagnostics P/I/D/output/unconstrained
```

PID returns an OutputProposal.

It has no transport dependency.

It has no right to write hardware.

---

## M4.6 Controller lifecycle

Implement only the lifecycle needed now:

```text
Created
Ready
Running
Paused
Failed
```

or an equally clear minimal representation consistent with architecture.

Starting/resuming a controller does not magically restore old output authority.

Pause must revoke authority and drive the output through the safe transition.

Resume must:

```text
validate fresh input
validate Reference
obtain new authority
use explicit state reset/reinitialize policy
```

Do not silently preserve an integrator merely for v1 compatibility.

---

## M4.7 Freshness

A previously valid measurement must eventually become unusable for control.

Add explicit freshness policy based on monotonic time.

Controller must fault/revoke rather than drive indefinitely from stale input.

This must be independent of "three serial failures" or similar acquisition retry policy.

---

## M4.8 Required tests

Cover at least:

```text
EMA with unequal dt
filter warm-up/quality
Reference independent time
ramp up/down/clamp
ramp target/rate continuity
wall-clock irrelevant to Reference/control tests

PID first sample
integral actual dt
D on measurement
no setpoint kick
upper/lower anti-windup
invalid update atomicity

controller start
pause -> safe output
resume with new authority
stale measurement revokes/faults
bad quality revokes/faults
large dt policy

full virtual closed loop moves temperature toward reference
all actuator writes still pass OutputArbiter
```

---

# M5. Bounded embedded Lua extensions

## Purpose

Prove Lua's intended role:

```text
embedded local extension
```

not:

```text
second application/runtime API
```

Add Lua only after the native authority/control path exists.

---

## M5.1 Packaging

A separate `lab-lua` crate is justified at this milestone if it keeps the VM dependency outside `lab-core`.

Preferred dependency direction:

```text
lab-lua
   ↓
lab-core
```

The core must not depend on Lua.

---

## M5.2 Lua scope

Implement the smallest useful Lua extension surface:

```text
one Lua-backed virtual/model instrument or parser component
one Lua filter/transform
one tiny Lua controller proof if the bounded contract is already robust
```

Do not recreate the old global:

```text
app
```

API.

Do not move orchestration into embedded Lua.

---

## M5.3 Component contract

Lua code should receive plain bounded data such as:

```text
configuration
typed input value(s)
quality
dt/time
small bounded state
```

and return:

```text
typed value/result
new bounded state if applicable
diagnostics
OutputProposal for a controller proof
```

Runtime validates all returned values again.

Lua never receives mutable Rust runtime objects.

---

## M5.4 Restrictions

Embedded Lua must NOT receive unrestricted:

```text
COM handles
filesystem
network
process spawning
native dynamic modules
FFI
OS library
raw arbitrary actuator bytes
safety policy mutation
```

Use the minimum standard-library allowlist required.

Document exactly what is exposed.

---

## M5.5 Budgets

Implement and test practical limits for the supported execution path:

```text
instruction/work budget
wall-clock deadline/watchdog
memory/state/output size budget where supported
bounded diagnostics
bounded host calls
```

An instruction hook alone is not sufficient protection against an unbounded native host call.

If the selected Lua embedding cannot safely bound some required path, document the limitation and narrow Lua's permitted role rather than pretending the risk vanished.

---

## M5.6 Failure isolation semantics

Test:

```text
Lua exception
infinite loop
oversized result/state
invalid returned type
NaN/Inf output
timeout
reload/replacement
late result from old generation
```

A failed Lua filter invalidates its dependent branch according to policy.

A failed Lua controller loses authority.

An unrelated native PID must continue to run.

Safe action must not require a functioning Lua VM.

---

## M5.7 Lua instrument operations

For physical-like named operations, Lua must operate through a bounded Rust-defined catalog.

Conceptually:

```text
temperature.read
power.set(value)
```

The Rust side defines:

```text
resource binding
operation kind
argument type/range
side effects
deadline
framing/template restrictions
result expectations
```

Lua may parse/transform allowed data.

Lua must not be able to replace the address/raw actuator bytes after OutputArbiter has authorized a particular value.

---

## M5.8 Reload

Reload creates a new generation.

Old results are rejected.

For stateful processing:

```text
default = reset/re-warm
```

unless an explicit validated migration policy exists.

Do not attempt automatic arbitrary Lua-state migration.

---

# M6. Real Babashka / local external API vertical slice

## Purpose

At the end of this milestone a real Babashka process must connect to a running Rust Runtime and interact with the same domain semantics used internally.

This is the stop point for this plan.

---

## M6.1 Runtime becomes a real long-lived host

`apps/lab-runtime` now needs an actual headless runtime process.

It must own:

```text
domain Runtime
native control tick
virtual fixture
OutputArbiter
Lua host where configured
external API adapter
```

Closing a client connection must not stop the Runtime.

Native PID must continue to execute without Babashka.

Keep GUI absent.

---

## M6.2 External protocol

Unless implementation evidence exposes a strong problem, use:

```text
loopback TCP + bounded NDJSON
```

for the first slice.

Reasons:

```text
simple cross-language protocol
easy Babashka client
existing donor framing/error tests can inform behavior
remote-ready without exposing the network today
```

Bind by default only to:

```text
127.0.0.1
```

Do not bind to `0.0.0.0`.

The transport encoding is an adapter.

Do not make JSON DTO structs become the internal domain model.

---

## M6.3 Keep protocol small

Implement only what the vertical slice needs.

At minimum:

```text
hello / version / capabilities

discover instruments
describe instrument
get runtime/controller/reference snapshot

get latest signal value

subscribe
unsubscribe

configure existing Reference
configure existing PID settings as allowed

start native controller
pause native controller

operation status/outcome
```

Do not implement the future complete product API.

Do not implement recording yet.

---

## M6.4 Command semantics

External requests must preserve the domain distinction:

```text
request received
accepted
applied/completed
failed
outcome unknown
```

Do not tell Babashka that a command was physically applied merely because JSON parsing succeeded.

Queries remain snapshot-only.

Hardware/virtual refresh remains an explicit operation.

---

## M6.5 Request IDs and bounded deduplication

Now, and only now, introduce request IDs/deduplication.

Requirements:

```text
bounded in-memory retention
same retained request ID + different payload is rejected
reconnect/restart does not imply durable exactly-once behavior
evicted IDs may have outcome_unknown semantics
```

Document these limits in English.

---

## M6.6 Events/subscriptions

Add bounded subscriptions sufficient for the slice.

Need:

```text
sequence/cursor
bounded retention
slow-client policy
explicit gap detection
resync through snapshot
```

A slow client must not backpressure PID/acquisition/safety.

If it falls behind:

```text
gap / resync
```

is preferable to blocking Runtime indefinitely.

---

## M6.7 Babashka client

Create a small real Babashka client/helper.

Keep it thin.

A reasonable repository location is something like:

```text
clients/babashka/
```

unless the existing workspace layout suggests a clearer non-Rust location.

It should contain:

```text
bb.edn
small client namespace
small REPL-friendly domain helper namespace
acceptance/example script
```

Do not create a large Clojure application yet.

Do not embed nREPL in Rust.

---

## M6.8 Desired Babashka experience

The first usable interaction should conceptually feel like:

```clojure
(require '[lab.client :as lab])

(def c (lab/connect! {:host "127.0.0.1"
                      :port 9000}))

(lab/hello c)
(lab/instruments c)
(lab/describe c ...)
(lab/latest c ...)

(lab/set-reference! c ...)
(lab/configure-pid! c ...)
(lab/start! c ...)
(lab/pause! c ...)
```

Names may differ.

Optimize for:

```text
REPL clarity
plain Clojure data
small number of concepts
good error information
```

not object-oriented wrappers.

---

## M6.9 Real acceptance scenario

Automate, as far as practical, this scenario with an actual Babashka process:

```text
1. Start Rust Runtime with virtual thermal plant.
2. Connect Babashka.
3. hello/capabilities.
4. Discover instrument without hardcoded Rust-type knowledge.
5. Describe parameters.
6. Read latest temperature.
7. Subscribe to temperature/controller diagnostics.
8. Configure Ramp Reference.
9. Configure native PID.
10. Start native PID.
11. Observe temperature moving under virtual control.
12. Disconnect/kill Babashka.
13. Verify native PID continues in Runtime.
14. Reconnect a new Babashka process.
15. Obtain a fresh snapshot.
16. Verify controller/reference/output state.
17. Safely pause the controller.
18. Verify output reaches the virtual safe state.
```

Also test:

```text
malformed message
oversized frame
unknown operation
invalid config
slow subscriber
subscription gap
duplicate request ID
client disconnect during pending operation
protocol-version mismatch
```

---

## M6.10 External/manual authority test

Where practical, add one small test showing that an external/manual owner is different from a Runtime-owned native PID.

If the external producer owns an actuator and its client disappears:

```text
lease expires
    ↓
authority revoked
    ↓
safe transition
```

But if Babashka merely started a Runtime-owned native PID and then disconnects:

```text
native PID continues
```

This distinction is central to the architecture.

---

## M6.11 No remote security claim

M6 is local-only.

State explicitly:

```text
not safe for direct untrusted network exposure
```

Do not implement Internet TLS/authentication merely to complete this milestone.

Keep the protocol/session boundary ready for future identity/permissions.

---

# 4. Coding style for M2–M6

This section is mandatory.

The user is learning from this source.

Prefer code that teaches the architecture.

## Module docs

Every substantial Rust module begins with useful `//!` documentation explaining:

```text
what this module owns
what it does not own
how it fits into the data/control flow
important invariants
```

## Public APIs

Every public API has meaningful English rustdoc.

For non-obvious functions include sections where useful:

```text
# Errors
# Panics
# Safety
# Invariants
# Examples
```

Do not mechanically add all headings when they add no information.

## State machines

Document transitions close to their implementation.

A learner reading OutputArbiter should be able to understand WHY each state exists without first reading five architecture documents.

## Algorithms

For PID, EMA, CRC, ramp and thermal model:

```text
explain equations
explain state
explain time assumptions
explain validation-before-commit
cite donor provenance where reused
```

Do not merely paste equations from v1.

## Concurrency

Whenever introducing threads/channels/tasks explain:

```text
who owns mutable state
who may block
queue capacity
overflow policy
shutdown behavior
why a channel is bounded
```

## Avoid clever code

Do not optimize for minimum line count.

Readable:

```rust
let error = setpoint - measurement;
let proportional = config.kp * error;
```

is often preferable in this project to compressing domain logic into a dense expression.

---

# 5. Tests and verification throughout

For every milestone run:

```powershell
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo doc --workspace --no-deps
git diff --check
```

Run release tests at each major milestone boundary:

```powershell
cargo test --workspace --release
```

Also verify `cargo run -p lab-runtime` where the host behavior is relevant.

At M5 run Lua integration tests.

At M6 run the actual Babashka acceptance flow, not only a Rust fake client.

If Babashka executable is unavailable:

```text
do not claim M6 complete
record the blocker in AI_HANDOFF.md
```

---

# 6. Git strategy

Use small logical commits.

Suggested shape, not mandatory exact names:

```text
M2:
docs: design milestone two output authority
test: add output arbiter acceptance cases
feat: add output authority model
feat: add deterministic simulated dispatcher
docs: report milestone two

M3:
docs: design transport protocol milestone
test: add transaction and metakon vectors
feat: add bounded transport executor
feat: add metakon codec and native adapter
feat: add minimal data-driven instrument definitions
docs: report milestone three

M4:
docs: design native control pipeline
feat: add native ema
feat: add fixed and ramp references
feat: add native pid
feat: connect virtual control loop through arbiter
docs: report milestone four

M5:
docs: design bounded lua host
feat: add bounded lua component host
feat: add lua processing extension
test: cover lua failures and reload fencing
docs: report milestone five

M6:
docs: design local external api
feat: add bounded loopback protocol adapter
feat: add subscriptions and operation outcomes
feat: add babashka helper client
test: add real babashka vertical acceptance
docs: report milestone six
```

Never sacrifice logical history just to match these names.

---

# 7. Milestone gates

Do not proceed from one milestone to the next unless:

```text
tests are green
clippy is clean
docs match implementation
AI_HANDOFF.md is current
working tree is understood
milestone report exists
acceptance gate is actually demonstrated
```

If architecture changes materially, record the issue rather than silently bending the design.

---

# 8. Final stopping point

After successful M6:

1. Update `AI_HANDOFF.md`.
2. Write `docs/implementation/MILESTONE_6_REPORT.md`.
3. Run all workspace checks.
4. Run actual Babashka acceptance.
5. Show the exact Rust and Babashka commands needed to reproduce the demo.
6. Provide the commit range M2–M6.
7. Mark:

```text
STATUS: READY_FOR_EXTERNAL_REVIEW
```

in `AI_HANDOFF.md`.

Then STOP.

Do not begin:

```text
recorder
SQLite schema
GUI
hardware acceptance
long soak
rich Babashka workflow library
full Lua compatibility
```

The next architectural/product review happens here.

---

# Definition of done for this plan

This plan is complete when:

```text
M1 remains green

M2:
one Rust OutputArbiter controls the virtual actuator
stale/revoked output cannot pass final dispatch

M3:
transport/protocol/instrument are separated
narrow Metakon path exists
minimal data-driven instrument path is proven
output path still passes arbiter

M4:
native EMA + Fixed/Ramp Reference + PID operate a virtual thermal process
freshness/failure stops unsafe control
PID has no transport access

M5:
Lua can provide a bounded local extension
Lua failure cannot disable Rust safety/native control
Lua cannot access raw output/COM/OS capabilities

M6:
a real Babashka process connects to a real long-running Rust Runtime
REPL can discover, inspect, configure and operate the native virtual control loop
Babashka disconnect does not stop a Runtime-owned PID
reconnect reconstructs state
external/manual authority expires safely
slow/malformed clients do not block control

All:
public Rust API is documented in English
important internal invariants are commented in English
source is suitable for a Rust learner to study
AI_HANDOFF.md accurately communicates current state to the external reviewer
```
