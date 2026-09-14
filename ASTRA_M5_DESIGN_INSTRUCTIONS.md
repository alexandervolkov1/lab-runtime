# Astra High — M4 lifecycle correction review + M5 design

Read first:

```text
AI_HANDOFF.md
AGENTS.md

docs/implementation/MILESTONE_2_DESIGN.md
docs/implementation/MILESTONE_2_REPORT.md
docs/implementation/MILESTONE_3_DESIGN.md
docs/implementation/MILESTONE_3_REPORT.md
docs/implementation/MILESTONE_4_DESIGN.md
docs/implementation/MILESTONE_4_REPORT.md

NEXT_IMPLEMENTATION_PLAN.md

docs/architecture/HIGH_LEVEL_ARCHITECTURE.md
docs/architecture/EXTENSION_MODEL.md
docs/architecture/RUNTIME_AND_SAFETY_MODEL.md

docs/migration/V1_TO_LAB_RUNTIME_MAP.md
docs/migration/REUSE_PLAN.md
docs/migration/MIGRATION_RISKS.md
```

Read the complete external reviewer response in `AI_HANDOFF.md`.

You are **ASTRA_HIGH**.

This phase is DESIGN / ARCHITECTURE REVIEW ONLY.

Do not implement production M5 code.

Do not add Lua dependencies yet.

Do not begin M6.

---

# 1. Verify current baseline

Before design work:

```powershell
git status
git log --oneline -20

cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Expected current baseline:

```text
72 passing workspace tests
M1–M4 implemented
M5 not started
```

Do not modify completed M3 safety semantics merely to simplify M5.

---

# 2. First resolve the two M4 lifecycle issues

Before designing Lua components, analyze and resolve the two issues identified by external review.

Document the chosen corrections in:

```text
docs/implementation/MILESTONE_4_LIFECYCLE_REVIEW.md
```

Do not implement them during this Astra phase.

They will become mandatory implementation prerequisites for the following SOL_HIGH phase.

---

# 3. M4 issue A — EMA warm-up and controller startup

Current behavior has a contract mismatch:

```text
EmaConfig permits warmup_samples > 1

but

StartController
    resets EMA
    processes one sample
    requires EmaStatus::Ready
```

Therefore a controller using a multi-sample EMA warm-up cannot start.

Decide the proper lifecycle.

Strongly consider a model conceptually similar to:

```text
Created
    ↓
Ready
    ↓
Warming / Preparing
    ↓ enough fresh Good samples
Running
```

but do not add a state merely because this example names one.

Choose the smallest coherent solution.

Required invariants:

```text
no actuator authority during warm-up

warm-up consumes only fresh valid samples

bad/stale/unavailable input cannot complete warm-up

output remains safe/disarmed during warm-up

authority is acquired only once all prerequisites are ready

PID must not integrate before the controller is actually allowed to run

resume/reinitialize follows an explicit equivalent policy

Reference semantics remain explicit and independent
```

Evaluate whether Reference should advance during warm-up.

Do not accidentally make controller warm-up pause a shared independent Reference.

If the better design deliberately restricts controller EMA to:

```text
warmup_samples == 1
```

justify it clearly and require validation at configuration time.

Do not leave the current accidental unsupported configuration.

Write exact acceptance tests for SOL_HIGH.

---

# 4. M4 issue B — bounded lease renewal for Runtime-owned controllers

Current native controllers receive a finite `OutputLease`.

The project is intended to support experiments lasting hours or days.

A healthy native controller must not inevitably fail when its initial lease expires.

Design a bounded renewal mechanism.

Do NOT solve this with:

```text
infinite lease
u64::MAX duration
extremely large hidden timeout
client session owning the native controller
```

Required properties:

```text
lease remains finite

Runtime-owned controller may renew only while Running and healthy

renewal is driven by trusted Runtime progress

freshness/tick-gap/controller checks happen before renewal is allowed

failed or stalled controller stops renewing

revocation/fault/pause prevents later renewal

renewal cannot resurrect an old epoch after revoke

external/manual lease does not auto-renew

Lua/external controller ownership does not receive native-controller privilege by accident

Babashka session lifetime does not determine a Runtime-owned native PID lifetime
```

Decide whether renewal:

```text
extends the same lease/epoch
```

or:

```text
issues a replacement token under a controlled authority operation
```

and explain why.

Prefer the smaller design that preserves fencing.

Specify exact boundary/deadline tests, especially:

```text
renew before expiry

renew exactly at expiry -> reject

revoke then delayed renewal -> reject

pause then renewal -> reject

stale input -> no renewal

healthy controller runs across several original lease lifetimes

external owner does not receive automatic renewal
```

---

# 5. Physical readback limitation

M3 correctly distinguishes:

```text
write ACK
!=
readback
```

The physical Metakon path currently proves acknowledged delivery but does not provide a complete generic post-write `ReadbackVerified` workflow.

This is acceptable at the current POC boundary.

Do not weaken evidence semantics.

For M5:

```text
Lua must never manufacture ACK/readback/safe evidence.
```

If a Lua-backed physical operation is considered, Rust-owned transport/protocol/catalog code must remain the source of delivery evidence.

If this cannot be demonstrated safely within M5, restrict the M5 physical Lua scope rather than broadening trust.

---

# 6. M5 purpose

Design the bounded embedded Lua extension layer.

Lua's role is:

```text
embedded local extension
close to Runtime
```

not:

```text
application orchestrator
second Runtime API
safety authority
raw hardware owner
```

Babashka remains the future external orchestration layer.

---

# 7. Decide exact M5 proof scope

The M5 implementation should prove the minimum useful extension model.

Preferred proof:

```text
one Lua-backed virtual/model component
+
one Lua transform/filter
+
optional small Lua controller proof only if the trust boundary remains simple
```

Do not require a Lua controller merely to tick a feature checkbox.

If adding a Lua controller materially complicates safety/isolation before M6, design M5 without it and explain why.

Do not recreate the donor's global `app` API.

---

# 8. Packaging boundary

Evaluate a separate:

```text
crates/lab-lua
```

or the smallest equivalent isolation boundary.

Preferred dependency direction:

```text
lab-runtime / host
        ↓
     lab-lua
        ↓
     lab-core
```

`lab-core` must not depend on Lua/mlua.

Lua-specific value conversion and VM ownership must stay outside the pure domain core.

Avoid crate proliferation.

---

# 9. Lua component contract

Design a small data-only invocation contract.

Lua may conceptually receive:

```text
validated configuration

typed input value(s)

unit identity

sample quality

explicit monotonic dt/time

small bounded persistent component state

small operation catalog if required
```

Lua may return only bounded typed data such as:

```text
value/result

quality/result status

bounded new component state

bounded diagnostics

possibly OutputProposal data for an explicitly authorized controller proof
```

Rust validates every returned value again.

Lua never receives:

```text
&mut Runtime
mutable Instrument
OutputLease internals
OutputAuthority
serial handle
ByteTransport
arbitrary Rust object graph
```

---

# 10. Sandbox and capability model

Explicitly decide what Lua standard facilities exist.

By default embedded Lua must NOT receive unrestricted:

```text
filesystem
network
process execution
OS library
environment mutation
FFI
native dynamic modules
raw COM
raw socket
arbitrary Rust callback registry
raw actuator bytes
safety configuration mutation
```

Prefer an allowlist model.

Document what IS available, not only what is forbidden.

For example:

```text
basic arithmetic
tables
strings
selected math functions
bounded host functions
```

if justified.

---

# 11. Resource budgets

Instruction counting alone is not enough.

Design explicit bounds for every supported callback path.

Consider:

```text
instruction/work budget

wall-clock deadline

memory limit or defensible allocator/state-size bound

input size

returned value size

persistent state size

diagnostic count/length

host-call count

nested callback/reentrancy prevention
```

If the chosen Lua embedding library cannot reliably enforce one of these, narrow the supported Lua role.

Do not document a limit that the implementation cannot actually enforce.

Investigate actual capabilities of the selected Rust Lua library before freezing the design.

This is architecture research, not production implementation.

---

# 12. VM ownership and scheduling

Define:

```text
who owns each Lua VM

whether one VM hosts one component or several

whether callbacks are serialized

whether Lua may call Runtime synchronously

whether reentrancy is forbidden

what happens if one callback stalls

how unrelated native PID work continues
```

Safety-critical Runtime progress must not depend on successful Lua execution.

A hung Lua callback must not block:

```text
native controller watchdog

OutputArbiter safe transition

transport recovery

unrelated native controller
```

If this requires an execution/isolation boundary, design it now.

Avoid introducing Tokio merely because scripting exists.

---

# 13. Lua transform/filter semantics

Design one useful filter/transform proof.

It should integrate with existing:

```text
Sample
SampleQuality
Unit
monotonic time
```

Define:

```text
warm-up behavior

unavailable input

wrong unit/type

NaN/Inf result

exception

timeout

state reset

reload
```

Do not allow a Lua filter to relabel incompatible engineering units arbitrarily unless the component descriptor explicitly declares the output unit.

No implicit unit conversion.

---

# 14. Apply the M4 warm-up lesson to Lua

Do not repeat the M4 warm-up lifecycle ambiguity.

For every stateful Lua processing component define explicitly:

```text
Created/configured

Warming / not yet usable

Ready output

Failed/unavailable

reset/reload
```

or a smaller equivalent contract.

A retained previous value after an unavailable callback must not be presented as a fresh output.

---

# 15. Lua virtual/model component

Design one Lua-backed virtual/model component that proves:

```text
script can provide useful runtime behavior

generic descriptor/domain clients do not branch on Lua origin

script has no physical transport ownership

reload is fenced by generation

bad script cannot mutate committed configuration halfway
```

Prefer a virtual/model proof rather than physical output for M5.

---

# 16. Lua physical instrument / named-operation boundary

Design the future-compatible boundary, even if M5 implementation does not exercise all of it.

Lua may use only Rust-defined named operations such as conceptually:

```text
temperature.read

power.set(value)
```

The trusted Rust operation catalog owns:

```text
resource binding

protocol primitive/template

argument type

unit

limits

side-effect classification

deadline

result shape

output-authority requirement
```

Lua may not choose:

```text
arbitrary COM port

arbitrary device address

arbitrary register

arbitrary frame bytes

arbitrary output unit

arbitrary safety classification
```

For output-affecting named operations:

```text
Lua candidate
    ↓
typed OutputProposal
    ↓
OutputAuthority
    ↓
trusted operation mapping
    ↓
transport
```

Lua must not be able to mutate the authorized value/address after authority validation.

---

# 17. Lua controller decision

Critically decide whether M5 should include a Lua controller proof.

If yes:

```text
Lua computes candidate control output

Rust owns lifecycle

Rust owns freshness

Rust owns lease/renewal

Rust owns failure handling

Rust wraps validated result in OutputProposal

OutputAuthority remains final authority
```

Lua must not receive an OutputLease or issue actuator commands itself.

A timeout/exception/nonfinite return must:

```text
invalidate controller result

stop renewal

revoke authority

run Rust-owned safe procedure
```

If no Lua controller is included in M5, document the extension point sufficiently to show the existing design does not prevent it later.

---

# 18. Generation and reload

Reload/replacement must create a new component generation.

Old asynchronous/late results must be rejected.

Default stateful reload policy:

```text
reset
re-warm
```

not arbitrary state migration.

Do not serialize and restore arbitrary VM internals.

If configuration is invalid:

```text
old committed generation remains valid
```

or another explicitly defined atomic replacement policy must be used.

Never leave a half-installed component.

---

# 19. Failure isolation

Define exact semantics for:

```text
Lua syntax/load error

runtime exception

instruction budget exhaustion

wall-clock timeout

memory exhaustion / state-size violation

invalid returned type

wrong unit

NaN/Inf

oversized diagnostics

forbidden host operation

reload while old callback is in flight

late old-generation result
```

For each state whether:

```text
only component fails

dependent signal becomes Unavailable

controller fails and revokes output

whole Runtime faults
```

Avoid global Runtime failure for ordinary extension errors.

Safe output must remain executable when the Lua VM is unusable.

---

# 20. Acceptance contract for Sol High

Write a concrete tests-first contract.

At minimum include tests for:

```text
Lua cannot access filesystem/network/process/FFI through exposed environment

Lua cannot access raw transport

Lua cannot bypass OutputAuthority

Lua cannot fabricate ACK/readback evidence

finite valid transform succeeds

wrong-type result rejected

wrong-unit result rejected

NaN/Inf rejected

exception isolated

infinite loop/work-budget exhaustion isolated

oversized state/result rejected

reload changes generation

late old-generation result rejected

unrelated native PID remains operable after Lua failure

Rust-owned safe transition works with Lua failed

invalid replacement does not destroy previous committed component
```

Also include the selected M4 lifecycle correction tests from sections 3 and 4.

---

# 21. Keep M5 bounded

Do NOT design or implement:

```text
general-purpose Lua application API

experiment orchestration language

Lua access to all Runtime Commands

Lua networking

Lua filesystem workflows

Lua GUI APIs

dynamic native plugins

WASM

recorder

Babashka client

external TCP API

general Signal graph editor
```

The purpose is to prove one safe extension boundary.

---

# 22. Teaching-quality requirement

The future implementation will be studied by the user.

The M5 design must require English source documentation explaining:

```text
VM ownership

sandbox/capability boundary

budget enforcement

generation fencing

failure propagation

why safety remains outside Lua

why host calls are bounded

state reset/reload semantics
```

Prefer code that exposes these ideas clearly rather than clever abstractions.

---

# 23. Deliverables

Create:

```text
docs/implementation/MILESTONE_4_LIFECYCLE_REVIEW.md
docs/implementation/MILESTONE_5_DESIGN.md
```

Update:

```text
AGENTS.md
AI_HANDOFF.md
```

No production Lua code.

No M5 implementation tests are required to compile yet unless a tiny throwaway library-capability spike is absolutely necessary.

If such a spike is needed, clearly separate it from production code and do not leave it as accidental architecture.

---

# 24. Model handoff

When the M4 correction design and M5 design/acceptance contract are complete:

update `AI_HANDOFF.md`:

```text
Current model:
ASTRA_HIGH

Next model:
SOL_HIGH

Reason:
M4 controller lifecycle corrections and the M5 Lua trust/isolation contract
are fixed. The next work is bounded implementation against those contracts.

Resume from:
1. implement the approved M4 lifecycle corrections and their regression tests;
2. verify M1–M4;
3. implement M5 according to MILESTONE_5_DESIGN.md.

Do not start:
M6 design.
```

Print a clear:

```text
MODEL HANDOFF

STOP HERE.
```

Then STOP.

Do not implement the M4 corrections yourself.

Do not implement M5.

Do not design M6 in this phase.
