# Current work — SOL_HIGH

Read this file only after:

```text
ai/HANDOFF.md
ai/ROADMAP.md
AGENTS.md
```

Then read in full:

```text
docs/implementation/MILESTONE_4_LIFECYCLE_REVIEW.md
docs/implementation/MILESTONE_5_DESIGN.md
docs/implementation/MILESTONE_4_REPORT.md
docs/implementation/MILESTONE_3_REPORT.md
```

Inspect the relevant current Rust/tests before modifying them.

You are now:

```text
SOL_HIGH
```

Authorized scope:

```text
1. implement the approved M4 lifecycle corrections;
2. fully verify corrected M1–M4;
3. implement M5 bounded Lua according to MILESTONE_5_DESIGN.md;
4. verify and report M5;
5. STOP for external review.
```

Do NOT begin M6 design or implementation.

---

# 1. Establish the `ai/` coordination layout first

Before production changes, make one small documentation-only coordination commit if it has not already been made.

The permanent coordination files are:

```text
ai/HANDOFF.md
ai/ROADMAP.md
ai/WORK.md
```

All are tracked in Git.

Update `AGENTS.md` so new phases read these files first.

`ai/HANDOFF.md` becomes the canonical external-review communication file.

Do not create additional per-model prompt files unless a genuine temporary need exists.

`ai/WORK.md` is intentionally overwritten when the current phase changes.

Do NOT perform the large Markdown cleanup now.

The full repository Markdown cleanup is a pre-release phase after Recorder/runtime hardening.

---

# 2. Baseline

Before production code:

```powershell
git status
git log --oneline -20

cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Expected pre-correction implementation baseline:

```text
72 tests passing
```

The current green suite does NOT mean the reviewed M4 lifecycle defects are fixed.

The design documents define required new red/green evidence.

---

# 3. First implement M4 lifecycle corrections

No Lua dependency may be added until:

```text
W1–W9
R1–R10
```

pass.

Use tests first and preserve the initial red evidence in the logical commit history/report.

---

# 4. Controller Warming correction

Implement the accepted contract from:

```text
docs/implementation/MILESTONE_4_LIFECYCLE_REVIEW.md
```

The intended lifecycle includes explicit Warming/preparation behavior.

Essential invariants:

```text
multi-sample EMA warm-up works

Warming owns no OutputLease

Warming performs no PID integration

Warming creates no normal OutputProposal

only distinct fresh Good observations advance warm-up

reusing the same observation does not count twice

exact freshness expiry is stale according to the approved exclusive boundary

Reference stays independent

Start and Resume use the same reset-and-warm policy

invalid/stale/unavailable input cannot activate output

Pause/Trip/RequestSafe/rebind cancel or fence Warming correctly

competing authority cannot slip through the preparation guard

failed activation cannot leave a guard or lease behind
```

N=1 must retain the useful direct-start behavior.

N>1 must become a supported lifecycle, not an implicit error.

Do not alter EMA mathematics merely to satisfy lifecycle tests.

---

# 5. Native lease renewal correction

Implement the approved private native renewal operation.

Required semantic result after healthy successful Running progress:

```text
same authority instance
same owner
same epoch
expiry = now + finite configured lifetime
new token replaces old token
old token becomes stale
```

Renewal must be available only to the trusted Runtime-owned native controller path.

Do not expose it as a generic public client command.

Do not grant renewal merely because a caller can construct/know:

```text
OutputOwner::Automatic(controller_id)
```

Manual/external owners continue to expire normally.

Renewal must reject:

```text
at expiry
after expiry
after revoke
after pause
after trip
after rebind
stale instance
stale epoch
stale token

pending proposal
transport reservation
in-flight dispatch
```

No renewal after:

```text
stale/unavailable/repeated control input
invalid PID result
tick-gap failure
failed delivery
```

A healthy controller must remain Running across several original lease lifetimes in fake-clock acceptance.

Do not accumulate expiry as:

```text
old_expiry + lifetime
```

Renew from current trusted progress time:

```text
now + lifetime
```

Do not solve the problem using enormous/infinite leases.

---

# 6. Preserve failure diagnostics and fencing

The controller must remain queryable after a lifecycle/output/safe-path failure.

Do not lose the useful failure state merely because settling safe output itself fails.

A stale failed controller must not later revoke or mutate a newer different owner.

Pure Query must not:

```text
renew leases
advance safety
perform warm-up
run watchdogs
```

---

# 7. Independent safety service requirement

The later long-running host needs safety/watchdog progress independent of producer success.

M5 introduces the bounded service seam needed to prove this against stalled Lua.

Do not claim:

```text
finite lease alone = autonomous safety
```

A process that never services time cannot physically execute a safe action by magic.

Preserve the distinction.

---

# 8. M4 correction verification gate

After W1–W9/R1–R10 are green, before adding Lua:

```powershell
cargo fmt --all -- --check
cargo test --workspace
cargo test --workspace --release
cargo clippy --workspace --all-targets -- -D warnings
cargo doc --workspace --no-deps
cargo run -p lab-runtime
git diff --check
```

Update M4 documentation/report so it distinguishes:

```text
original 72-test M4 checkpoint
```

from:

```text
corrected lifecycle implementation
```

Use a clean logical commit boundary before M5.

---

# 9. M5 dependency/package boundary

Only after corrected M4 is green:

add the Lua boundary according to `MILESTONE_5_DESIGN.md`.

Accepted target:

```text
lab-runtime
    ↓
lab-lua
    ↓
lab-core
```

`lab-core` remains free of Lua/mlua.

Current selected embedding target from the accepted design:

```text
mlua 0.11.6
lua54
vendored
```

If the actual dependency/version capability differs materially from the researched design:

```text
STOP
STATUS: WAITING_FOR_REVIEW
```

Do not silently weaken isolation/budget requirements to fit a library.

---

# 10. M5 scope

Implement:

```text
one bounded Lua virtual measurement/model component

one bounded Lua moving-mean filter/transform
```

Do NOT implement in M5:

```text
Lua controller
physical Lua output
Lua COM ownership
raw transport
arbitrary Runtime command access
Babashka
external API
Recorder
GUI
```

The omission of a Lua controller is deliberate.

---

# 11. Lua data boundary

Lua receives only bounded plain data.

No callback receives:

```text
&mut Runtime
mutable Instrument
OutputLease
OutputAuthority
ByteTransport
serial handle
Dispatch completion capability
arbitrary Rust objects
```

Persistent script state is:

```text
bounded plain data
owned/committed by Runtime
```

not persistent arbitrary VM globals/upvalues.

Use fresh VM/job semantics from the accepted design.

All Lua outputs are revalidated in Rust before commit.

---

# 12. Lua capability allowlist

The real environment must prevent reachable access to forbidden capabilities, not merely hide one obvious global.

Acceptance must cover indirect/nested reachability.

Lua must not obtain unrestricted:

```text
filesystem
process execution
environment access
network
FFI
package.loadlib
native module loading
raw transport
raw actuator bytes
safety mutation
```

Follow the exact capability closure described in L1–L22.

Do not expose `debug`, unrestricted `require`, bytecode loading or equivalent escape paths where the design forbids them.

---

# 13. Lua budgets

Implement the actual bounded paths described by the M5 design.

At minimum preserve separate limits for:

```text
instruction/work
heap/allocation
plain-data state/result
diagnostics
host-call count
wall-clock acceptance deadline
worker capacity
component capacity
generation/revision
```

The accepted deadline is:

```text
100 ms
```

according to the M5 design.

The deadline includes the full accepted job path specified by the design, not merely user callback body time.

An instruction hook is not a substitute for an independent wall-clock acceptance deadline.

---

# 14. Worker/isolation model

Use the accepted fixed bounded worker model.

Current design:

```text
two fixed worker slots
```

A stalled job may quarantine capacity, but must not cause unbounded worker spawning.

Runtime/native safety work must never wait synchronously for a stalled VM.

The integration acceptance must prove progress of:

```text
native PID
native lease renewal
Rust safe transition
another transport/recovery path
```

while a Lua worker is deliberately blocked.

Use deterministic barriers/channels for the hard-stall test.

Do not depend on lucky thread scheduling.

---

# 15. Lua processing semantics

Preserve typed:

```text
Sample
SampleQuality
Unit
monotonic observation time
freshness provenance
```

A slow transform does NOT make an old physical observation fresh.

Derived freshness must retain the source observation lineage required by the M5 design.

A missing/unavailable/stale/wrong-unit input should avoid unnecessary VM execution where the contract specifies that behavior.

A retained old filtered/model value must never masquerade as a new fresh Good observation after failure.

---

# 16. State atomicity

Lua may mutate temporary job-local tables/environment while executing.

That must never mutate committed Runtime state until Rust:

```text
validates successful completion
validates types/units
validates finite/range
validates bounds
validates generation/revision/deadline
```

and commits atomically.

If execution throws or returns invalid data:

```text
previous committed config/state remains unchanged
```

No accidental persistent globals/upvalues.

---

# 17. Reload/generation

Valid replacement:

```text
stage
validate/load
quiesce according to contract
commit new generation
reset state
re-warm
invalidate dependent latest state
reject old in-flight result
```

Invalid replacement must preserve the previous committed generation where the design says it can remain usable.

Never half-install a new script.

Checked generation exhaustion fails closed.

Renaming does not create a new generation.

Reload does.

---

# 18. Evidence and safety

Lua has no API path that can create or claim:

```text
Acknowledged
ReadbackVerified
safe_confirmed
DispatchOutcome
CompleteDispatch
```

M3 rule remains:

```text
ACK != readback
```

A Metakon write ACK cannot satisfy a Readback-required SafeProfile.

Lua failure cannot disable Rust-owned safe behavior.

M5 must not invent the still-missing generic physical readback mechanism.

---

# 19. L1–L22 are the acceptance contract

Treat every L1–L22 group from:

```text
docs/implementation/MILESTONE_5_DESIGN.md
```

as mandatory.

Especially preserve tests for:

```text
real capability closure
valid model/filter behavior
warm-up
type/unit/finite/range rejection
plain-data bounds
atomic state
input freshness
instruction budget
memory/parser limit
hard-stall isolation
deadline boundary
bounded workers
isolated failure
safety independence
reload
invalid replacement
same-time invalidation
evidence rejection
identity/generation fencing
bounded cleanup/shutdown
finite demonstration
```

A synchronous Lua call followed by resumed native work does NOT prove isolation.

---

# 20. M5 finite demonstration

Add the finite M5 demonstration required by the design.

It should prove approximately:

```text
Lua model
    ↓
Lua filter
    ↓
native controller
    ↓
OutputAuthority
    ↓
existing Rust virtual plant output
```

while also proving an unrelated native path continues correctly.

Demonstrate:

```text
warm-up
valid processing
failure
reload
generic snapshots
```

Keep the original finite M1 demo green.

This is not yet the M6 long-running service.

---

# 21. Source quality

All Rust:

```text
//! module docs
/// public rustdoc
// explanatory comments
TODO
FIXME
rustdoc examples
```

must remain English.

Explain non-obvious:

```text
VM ownership
capability reachability
budget mechanics
worker quarantine
deadline vs cancellation
state commit
freshness lineage
generation fencing
safe independence
```

Prefer readable explicit state transitions over compressed abstractions.

The user is studying the source.

---

# 22. Git

Use:

```text
one logical change = one commit
```

Recommended broad boundaries:

```text
tests: add M4 warm-up regression cases
feat: implement controller warming lifecycle

tests: add native lease renewal regressions
feat: add private native lease renewal

fix/test: close retained failure/fencing cases

docs: report corrected M4 lifecycle

test: add bounded Lua acceptance foundations
feat: add lab-lua bounded runner
feat: add Lua model/filter components
feat: add Lua worker/isolation/reload integration
test: close L1–L22 acceptance
docs: report M5
```

Exact commit names are not mandated.

Do not squash everything into one commit.

---

# 23. M5 final verification

Before completion:

```powershell
cargo fmt --all -- --check
cargo test --workspace
cargo test --workspace --release
cargo clippy --workspace --all-targets -- -D warnings
cargo doc --workspace --no-deps
cargo run -p lab-runtime
git diff --check
```

Also run every explicit M5 Lua integration/hard-stall/deadline acceptance required by the design.

Audit dependency/features so the Lua environment is exactly the intended one.

Report measured deadline/timeout behavior honestly.

Do not claim OS hard-real-time isolation.

---

# 24. Completion documentation

Write/update:

```text
docs/implementation/MILESTONE_4_REPORT.md
docs/implementation/MILESTONE_5_REPORT.md
ai/HANDOFF.md
```

`ai/HANDOFF.md` must include:

```text
exact HEAD
clean/dirty status
new test count
commands/results
commits
implemented W/R/L acceptance mapping
dependency versions/features
known limitations
review request
```

Do not paste an architecture encyclopedia there.

Point to reports for details.

---

# 25. Required stop

After M5 is completely green:

```text
STATUS: WAITING_FOR_EXTERNAL_REVIEW
```

in:

```text
ai/HANDOFF.md
```

Print:

```text
MODEL HANDOFF

STOP HERE.

Completed:
Corrected M4 controller lifecycle and implemented bounded M5 Lua extensions.

Current model:
SOL_HIGH

Next step:
External architecture/safety review.

Do not start:
M6 design or implementation.
```

Then STOP.

Do not design M6.

Do not implement M6.

Do not perform M7 Recorder work.

Do not perform the pre-release Markdown cleanup yet.
