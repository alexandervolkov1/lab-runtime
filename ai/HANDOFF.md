# AI Handoff

This is the single persistent external-review communication file.

Every new phase must read:

```text
ai/HANDOFF.md
ai/ROADMAP.md
ai/WORK.md
AGENTS.md
```

Detailed architecture and implementation evidence belongs in normal project documentation.

---

## Current state

```text
STATUS: APPROVED_FOR_ASTRA_HIGH_M6_DESIGN

Completed model:
SOL_HIGH

Completed:
M4 lifecycle corrections
M5 bounded embedded Lua

Next model:
ASTRA_HIGH

Next authorized work:
M6 design only
```

Reviewed implementation checkpoint:

```text
corrected M4 gate: 92 workspace tests
completed M5 gate: 123 workspace tests
```

M6 production implementation has not started.

---

## Completed milestones

```text
M1  domain foundation

M2  central OutputAuthority / OutputArbiter

M3  bounded transport executor
    narrow Metakon
    extensible engineering units
    data-driven known-protocol instruments

M4  virtual thermal plant
    native EMA
    Fixed/Ramp Reference
    native PID
    corrected controller Warming lifecycle
    bounded native-controller lease renewal

M5  bounded embedded Lua
    virtual/model observation
    moving-mean transform
    fixed worker isolation
    capability allowlist
    execution/resource budgets
    generation/reload fencing
    independent Rust safety service seam
```

---

## External review result

Corrected M4 and completed M5 are APPROVED.

The external review accepted the implementation against:

```text
docs/implementation/MILESTONE_4_LIFECYCLE_REVIEW.md
docs/implementation/MILESTONE_4_REPORT.md
docs/implementation/MILESTONE_5_DESIGN.md
docs/implementation/MILESTONE_5_REPORT.md
```

The reviewer did not independently execute Cargo commands on the user's machine.
Approval is based on the supplied repository snapshot, implementation source and recorded verification evidence.

---

## Accepted M4 invariants

### Controller warm-up

`Warming` is preparation, not output ownership.

During Warming:

```text
no OutputLease
no PID integration
no ordinary OutputProposal
no actuator ownership
```

Only distinct fresh Good observations may advance warm-up.

The independent Reference continues according to Runtime monotonic time.

Activation obtains authority only after preparation is complete.

Start and Resume use the explicit reset-and-warm policy.

Invalid/stale/unavailable input cannot activate the controller.

### Native controller lease renewal

Runtime-owned native controllers use finite leases.

Renewal is:

```text
crate-private
Runtime-controlled
after successful Running delivery
```

A replacement token keeps:

```text
authority instance
owner
epoch
```

and receives:

```text
expiry = current trusted progress time + finite configured lifetime
```

The old token becomes stale immediately.

Renewal must stop on:

```text
expiry
revoke
pause
trip
stale/unavailable input
invalid algorithm result
failed output delivery
lost ownership
```

Manual and external/client Automatic owners do not receive this privilege.

Merely knowing or supplying the same numeric controller ID never grants native-controller renewal authority.

A future Babashka session that starts a Runtime-owned native PID does not own the PID lifetime.

---

## Accepted M5 invariants

M5 Lua remains deliberately limited to:

```text
virtual/model observations
+
observation transforms/filters
```

M5 does NOT contain:

```text
Lua controller
physical Lua instrument driver
raw COM access
raw ByteTransport access
OutputLease access
OutputAuthority access
physical evidence completion
Babashka
Recorder
GUI
```

Dependency direction:

```text
lab-runtime
    ↓
lab-lua
    ↓
lab-core
```

`lab-core` remains free of Lua dependencies.

Current embedding:

```text
mlua 0.11.6
Lua 5.4
vendored
default features disabled
```

Lua VMs use an explicit capability allowlist.

Guest code cannot obtain unrestricted:

```text
filesystem
network
process execution
environment access
FFI
native module loading
raw transport
output authority
hardware evidence
```

Execution is bounded by:

```text
two fixed worker slots

fresh VM per job

instruction budget

Lua heap budget

plain-data state/result limits

diagnostic limits

host-call limit

100 ms acceptance/dependency deadline

generation/revision/correlation fencing
```

A stalled worker remains quarantined.

Runtime never spawns unlimited replacement workers.

Runtime never waits synchronously for a Lua VM.

A late result cannot commit, including a late result from the current generation after its deadline.

Reload:

```text
validate candidate
commit new generation atomically
reset bounded script state
re-warm
reject old/late results
```

An invalid staged replacement cannot partially destroy the previously committed component.

Lua may never fabricate:

```text
Acknowledged
ReadbackVerified
safe_confirmed
Dispatch completion
physical delivery evidence
```

M3 rule remains:

```text
ACK != readback
```

---

## Known limits after M5

The implementation is still an in-process software/virtual proof.

A stalled native call inside a Lua worker may continue physically after the 100 ms acceptance deadline.

The deadline means:

```text
result/dependency acceptance fence
```

not:

```text
forced thread termination
hard real-time guarantee
hardware safety certification
```

`ServiceSafety` must currently be called explicitly.

There is not yet an autonomous Runtime scheduler.

There is no:

```text
Babashka/API
long-running host
Recorder/SQLite
real Windows COM adapter
physical hardware acceptance
generic physical post-write readback
GUI
long soak
```

---

## Mandatory documentation synchronization

Before doing material M6 architecture work, ASTRA_HIGH must update the stale:

```text
README.md
AGENTS.md
```

The current files still describe the previous external-review/M1–M4 era in places.

This is a small documentation-only synchronization step.

The updated documents must state factually:

```text
M1–M5 implemented

three workspace packages:
    lab-core
    lab-lua
    lab-runtime

corrected Warming/native renewal lifecycle

bounded Lua model/transform proof

123 workspace tests at reviewed checkpoint

finite executable is not yet the autonomous M6 host

no Babashka API yet
no Recorder yet
no real COM deployment/hardware acceptance yet
```

Do not perform the large pre-release Markdown cleanup now.

---

## Reviewer authorization for M6

M6 is the first:

```text
autonomous long-running headless Runtime
+
real local external API
+
actual Babashka vertical slice
```

ASTRA_HIGH must design M6 before SOL_HIGH implements it.

The M6 design must explicitly resolve:

```text
Runtime/host state ownership

autonomous monotonic scheduler

ServiceSafety cadence and priority

native controller scheduling

measurement/virtual plant scheduling

Lua component scheduling

transport/recovery scheduling

drift and bounded catch-up policy

network/client isolation

bounded request admission

loopback TCP + bounded versioned NDJSON

protocol/version handshake

request IDs

bounded in-memory deduplication

accepted/completed/failed/outcome_unknown semantics

bounded event retention

subscriptions

cursor/sequence semantics

slow-client policy

gap detection

snapshot + cursor reconnect consistency

multi-client isolation

native PID independence from Babashka lifetime

external/manual authority expiry where exposed

graceful shutdown and safe transition

actual Babashka-process acceptance

process-restart semantics before persistence exists
```

---

## Continuing architectural constraints

Preserve:

```text
Runtime = single authoritative mutable owner

Query = pure snapshot

Command/Operation = mutation/work

ACK != readback

client lifetime != experiment lifetime

Lua lifetime != Runtime lifetime

Lua lifetime != safety lifetime

Babashka lifetime != Runtime-owned PID lifetime
```

No extension mechanism may bypass Rust OutputAuthority.

---

## Native Rust extension policy

Native Rust remains a first-class extension path.

Recompilation is expected and acceptable for:

```text
high-rate algorithms
controllers
complex models
physical drivers
new protocols
safety-critical code
reliability-sensitive code
```

Dynamic extensibility is a convenience, not a requirement.

Do not introduce a dynamic native Rust plugin ABI merely to avoid recompilation.

A useful algorithm prototyped in Lua/Babashka should later be movable into Rust without redesigning the surrounding domain model.

---

## M6 scope boundary

ASTRA_HIGH is authorized to:

```text
synchronize README/AGENTS documentation

design M6

write a concrete tests-first acceptance contract

update ai coordination files

request ASTRA_HIGH -> SOL_HIGH
```

ASTRA_HIGH must NOT:

```text
implement M6 production code

add the real TCP server

add the Babashka client implementation

begin M7 Recorder design

implement SQLite

add real COM deployment

perform pre-release Markdown cleanup
```

---

## First-release direction after M6

The current product roadmap remains:

```text
M6
    long-running Runtime
    local API
    real Babashka slice

external review

M7
    Runtime-owned Recorder
    SQLite backend
    durable experiment history

runtime / Windows COM / Metakon hardening

repository cleanup
    including obsolete Markdown
    and human-readable test filenames

final documentation

virtual-furnace tutorial

release verification

first stable GitHub release
```

After that release the project moves into live experimentation with:

```text
Babashka
Lua
native Rust extensions
real Metakon
future Arduino instrument
```

---

## Test file naming policy

New test files use descriptive snake_case names.

Examples:

```text
host_scheduler.rs
request_deduplication.rs
subscription_recovery.rs
babashka_reconnect.rs
runtime_shutdown.rs
```

Use:

```text
step_01_<name>.rs
step_02_<name>.rs
```

only for intentionally sequential human-readable acceptance/tutorial stages.

Test correctness must never depend on file or test execution order.

Do not mass-rename old test files during M6.

Existing historical test filenames are cleaned up during the pre-release repository cleanup.

---

## Reviewer response

```text
STATUS: APPROVED_FOR_ASTRA_HIGH_M6_DESIGN
```

External reviewer conclusion:

* corrected M4 lifecycle accepted;
* bounded M5 Lua implementation accepted;
* no blocking architectural defect identified before M6 design;
* current M5 limitations are explicit and acceptable for this POC boundary;
* M6 must turn the explicit host-service seams into a real autonomous long-running Runtime;
* M6 must be designed by ASTRA_HIGH before SOL_HIGH implementation.

---

## Model handoff

```text
Current model:
SOL_HIGH

Next model:
ASTRA_HIGH

Reason:
M4 corrections and M5 implementation passed external review.
M6 introduces autonomous scheduling, network/client lifecycle,
subscriptions/reconnect and the first real Babashka process boundary.

Resume from:
M6 design only.

Required stop:
After M6 design and acceptance contract,
before M6 production implementation.

Do not start:
M7 Recorder.
```
