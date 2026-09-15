# AI Handoff

This is the single persistent communication file between:

* the current implementation/design agent;
* the external ChatGPT reviewer;
* the user.

Detailed architecture and implementation reasoning belongs in normal project documentation.
This file must remain concise and current.

Before starting a new phase, the active model must read:

1. `ai/HANDOFF.md`
2. `ai/ROADMAP.md`
3. `ai/WORK.md`
4. the specific milestone design/report files referenced below.

---

## Current state

Current milestone:

```text
M4 lifecycle correction implementation + M5 implementation
```

Current model transition:

```text
ASTRA_HIGH -> SOL_HIGH
```

Status:

```text
WAITING_FOR_MODEL_SWITCH
```

Verified implementation baseline before the Astra design phase:

```text
72 workspace tests passing
```

The M4 lifecycle corrections and M5 Lua architecture are designed but are NOT yet implemented.

The authoritative design contracts are:

```text
docs/implementation/MILESTONE_4_LIFECYCLE_REVIEW.md
docs/implementation/MILESTONE_5_DESIGN.md
```

---

## Completed

Implemented before the current phase:

```text
M1 domain foundation
M2 OutputAuthority / OutputArbiter
M3 bounded transport + narrow Metakon + data-driven instruments
M4 thermal plant + EMA + Reference + native PID
```

The M3/M4 baseline preserves:

```text
Runtime single mutable ownership

central Rust output authority

actual-first-byte authority revalidation

partial-write ambiguity

no blind actuator-write retry

ACK != readback

extensible engineering units

independent Reference

native PID/EMA without transport access

safe failure on stale/unavailable control input

query purity
```

Astra High subsequently designed two mandatory M4 corrections:

```text
1. explicit controller Warming lifecycle for EMA warmup_samples > 1

2. bounded renewal of Runtime-owned native-controller output leases
```

Astra High also completed the M5 bounded Lua trust/isolation design.

No M5 production implementation has been completed yet.

---

## Current accepted M4 correction decisions

### Warming

Controller startup/resume must support multi-sample EMA warm-up.

During Warming:

```text
no output lease
no PID integration
no normal OutputProposal
no ordinary actuator authority
```

Only distinct fresh Good samples may advance warm-up.

The Reference remains an independent Runtime component and is not paused merely because a controller is warming.

Start and Resume use the same explicit reset-and-warm policy.

Invalid/stale/unavailable input cannot activate the controller.

All W1–W9 acceptance cases in `MILESTONE_4_LIFECYCLE_REVIEW.md` are mandatory.

### Native lease renewal

Runtime-owned native controllers use finite leases.

A healthy Running native controller may renew its lease only through a private trusted Runtime path after successful control/output progress.

Accepted semantics:

```text
same authority instance
same owner
same epoch
new finite expiry
old token immediately stale
```

Renewal:

```text
must occur before expiry
must not fabricate evidence
must not transition through safe/arm states
must reject pending/reserved/in-flight authority work
must stop on stale input, failure, pause, revoke or trip
```

Manual owners and client/external Automatic owners do NOT inherit native renewal privilege.

A Babashka client that starts a Runtime-owned PID does not become the PID owner and its later disconnect must not stop that healthy PID.

All R1–R10 acceptance cases are mandatory.

---

## Current accepted M5 decisions

M5 proves only a bounded embedded Lua extension layer.

The implementation scope is deliberately:

```text
Lua-backed virtual measurement/model component
+
Lua moving-mean processing/filter component
```

M5 does NOT include:

```text
Lua controller
physical Lua instrument writes
raw transport
raw COM
arbitrary actuator operations
orchestration API
Babashka
recorder
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

`lab-core` remains independent of Lua.

Current selected embedding target:

```text
mlua 0.11.6
Lua 5.4
vendored
```

The Lua environment is capability-allowlisted.

Scripts must not receive unrestricted:

```text
filesystem
network
process execution
environment access
FFI
native dynamic loading
raw transport
OutputLease
OutputAuthority
hardware evidence completion
mutable Runtime objects
```

Execution model:

```text
fresh VM per job
bounded plain-data persistent state committed by Runtime
two fixed worker slots
bounded instruction/work budget
bounded heap/data/state/diagnostic sizes
bounded host-call count
independent 100 ms acceptance deadline
generation/revision fencing
```

A failed/stalled Lua worker must not block:

```text
native PID
native lease/watchdog progress
OutputAuthority safe transition
transport recovery
unrelated Runtime work
```

Late results, including late results from the CURRENT generation after deadline, are discarded.

Reload:

```text
stage candidate
validate
increment generation on successful replacement
reset state
re-warm
reject old/late results
```

An invalid staged replacement leaves the previous committed generation intact.

Lua may never fabricate:

```text
Acknowledged
ReadbackVerified
safe evidence
Dispatch completion
```

All L1–L22 acceptance groups in `MILESTONE_5_DESIGN.md` are mandatory.

---

## Current next work

SOL_HIGH must:

1. verify the clean baseline;
2. implement the approved M4 lifecycle corrections tests-first;
3. make W1–W9 and R1–R10 green;
4. run the complete M1–M4 verification gate;
5. only then add Lua dependencies;
6. implement M5 according to `MILESTONE_5_DESIGN.md`;
7. satisfy L1–L22;
8. write/update implementation reports;
9. update this file;
10. STOP for external review.

SOL_HIGH must NOT begin M6 design or implementation.

---

## Reviewer response

The external reviewer writes recommendations in this section.

The active model must read and consume this section before continuing after a review checkpoint.

Current reviewer status:

```text
APPROVED_TO_PROCEED_WITH_SOL_HIGH_M4_CORRECTIONS_AND_M5_IMPLEMENTATION
```

Reviewer constraints:

* do not weaken M2/M3 output-authority or physical-evidence semantics;
* do not reinterpret ACK as readback;
* do not give Lua raw transport or safety authority;
* do not make controller lifetime depend on client lifetime;
* do not solve native lease lifetime using effectively infinite leases;
* do not expand M5 into a Lua controller or physical Lua driver milestone;
* keep native Rust a first-class extension path.

---

## Permanent product direction

Native Rust is a first-class extension mechanism.

Recompilation is acceptable and expected for:

```text
performance-critical algorithms
high-rate controllers
complex mathematical models
real hardware drivers
new physical protocols
safety-critical components
reliability-sensitive code
```

Dynamic extension is a convenience, not a religion.

A component should be movable from Babashka/Lua experimentation to native Rust without forcing the surrounding domain model to be redesigned.

Typical implementation levels:

```text
Native Rust
    fast / deterministic / hardware / safety / complex protocols

Data-driven
    new devices using trusted known protocol primitives

Lua
    bounded embedded local models/transforms/adapters

Babashka
    external orchestration, REPL work, recipes,
    supervisory logic and later experimental models/controllers
```

All actuator-affecting implementations still converge on Rust OutputAuthority.

---

## Later release goal

The first stable project release is NOT immediately after M6.

Before the first release the project must additionally complete:

```text
M7 durable Recorder + SQLite

release/runtime hardening

real Windows serial/COM adapter

safe real Metakon read-only smoke path

repository Markdown cleanup/consolidation

final documentation

end-to-end tutorial

release packaging
```

See `ai/ROADMAP.md`.

---

## Model handoff

Current model:

```text
ASTRA_HIGH
```

Next model:

```text
SOL_HIGH
```

Resume from:

```text
Implement M4 lifecycle corrections first.
Then implement M5.
```

Required stop:

```text
After M5 implementation and verification, before M6 design.
```
