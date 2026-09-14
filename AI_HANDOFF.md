# AI Handoff

## Current state

Current milestone: M4 correction design + M5 design complete; implementation pending.
Current model: ASTRA_HIGH.
Status: WAITING_FOR_MODEL_SWITCH — ASTRA_HIGH -> SOL_HIGH.
Verified implementation baseline: `21d5cde`.
Latest design checkpoint: `3fda62c`; this handoff is the following documentation commit.
Working tree: checked clean at the final handoff commit; verify before continuing.

## What was completed

Read the new external reviewer response and authoritative
`ASTRA_M5_DESIGN_INSTRUCTIONS.md`, required implementation/architecture/migration
documents, and relevant lifecycle/authority/signal code. Verified the existing
72-test baseline. Designed both mandatory M4 corrections, then the M5 Lua
trust/isolation boundary and concrete tests-first acceptance.

- [M4 lifecycle review](docs/implementation/MILESTONE_4_LIFECYCLE_REVIEW.md):
  chosen behavior, existing code findings, W1–W9 and R1–R10.
- [M5 design](docs/implementation/MILESTONE_5_DESIGN.md):
  scope, researched embedding capabilities, ownership, limits, failure/reload
  contracts and L1–L22 acceptance groups.

No production Rust, Cargo dependency, Lua script, spike or implementation test
changed. The original M4 defects are still present until Sol implements corrections.
The new contracts are not claims of passing implementation acceptance.

## Important design decisions

- Add Warming with a Runtime preparation guard but no lease/proposal/PID integration.
  Count distinct fresh Good samples; Start/Resume share reset-and-warm policy.
  Reference remains independent. Use exclusive freshness, including exact boundaries.
- Native renewal is private, after a successful Running delivery: replacement token
  with same ownership epoch, expiry = now + finite lifetime, old token invalid.
  Renewal at expiry/revoke rejects. No pending/reserved/in-flight work may be renewed.
  Manual/external Automatic identities gain no native renewal privilege.
- Preserve queryable controller failure state on safe-path errors and fence stale
  controllers from affecting a later owner; these are part of M4 lifecycle closure.
- M5 proves a Lua virtual measurement model and moving-mean filter. No Lua controller
  or callable physical Lua operations. M3 ACK is still not ReadbackVerified.
- lab-lua depends on Core; Core stays std-only. Research selected mlua 0.11.6,
  lua54+vendored only. Versioned primary sources are linked in M5 section 3.
- Fresh VM per job, persistent state as bounded Runtime-committed plain data, explicit
  capability allowlist and two fixed workers. Instruction/heap/call quotas plus an
  independent 100 ms acceptance deadline; quarantine never spawns unlimited workers.
- Native/safety service never waits for a VM. Derived freshness retains original
  observation time; reset/reload uses new generations and re-warm. Invalid staged
  replacement preserves the previous committed generation.

## Planned public/domain changes

Warming/controller diagnostics and private renewal are M4 prerequisites. M5 adds
language-neutral managed-component registration, bounded data/executor seam,
component snapshots, independent safety service and Sample freshness provenance.
Lua values, leases, raw transport and evidence completion are absent from callback
contexts. These APIs are designed, not implemented.

## Verification

At baseline `21d5cde`:
- `git status`: only user-added reviewer response and Astra instruction file.
- `git log --oneline -20`: inspected.
- `cargo test --workspace`: 72 passed, none failed or ignored.
- `cargo clippy --workspace --all-targets -- -D warnings`: exit 0.

Design phase: `git diff --check`, Markdown link/fence checks and a changeset-scope
audit. Every changed path is Markdown; Cargo manifests/lockfile and Rust sources/
tests are unchanged. No need to repeat unchanged-code release/hardware checks.
Lua capability research used primary documentation/source, not a runtime spike.
Donor was neither accessed nor modified during this phase.

## Commits

- `c5399a1` — preserve external review/instructions and authorize Astra design phase.
- `b291714` — M4 warm-up and bounded native renewal correction design.
- `3fda62c` — M5 trust/isolation design and Sol acceptance.
- Following documentation commit — final phase gate and this handoff.

Earlier M3/M4 implementation histories remain in their milestone reports.

## Deviations and limits

M5 intentionally omits the optional Lua controller and physical-operation proof.
Fresh per-job VMs and explicit data-state commits simplify atomicity. Workers bound
waiting dependencies and ordinary script failures, not native crash/system OOM or
hard OS timing. Physical generic post-write readback remains unimplemented.
M4's exact freshness boundary is resolved as exclusive; no safety weakening.
No M6 design or implementation has started.

## Review request

No unresolved design fork. Implementation evidence is still required for all new
acceptance gates. The prior reviewer response below has been consumed into the
two design contracts; no external approval of this new design is claimed.

## Model handoff

Current model: ASTRA_HIGH.
Next model: SOL_HIGH.

Reason: M4 controller lifecycle corrections and the M5 Lua trust/isolation contract
are fixed. The next work is bounded implementation against those contracts.

Resume from:
1. Implement approved M4 lifecycle corrections and regression tests first.
2. Verify M1–M4 and update corrected M4 implementation evidence.
3. Implement M5 according to MILESTONE_5_DESIGN.md and satisfy L1–L22.
4. Write the M5 report, update this file and stop for external review.

Do not start: M6 design or implementation.

## Reviewer response

The following externally supplied response is preserved verbatim. Its requested
design work is complete; its implementation prerequisites remain pending.

External review: M3 accepted. M4 accepted as the current deterministic POC, with two mandatory lifecycle issues to resolve before M5 implementation.

STATUS: APPROVED_FOR_ASTRA_HIGH_M5_DESIGN_WITH_M4_CORRECTIONS

### M4 issue 1: EMA warm-up lifecycle

The standalone EMA correctly supports `warmup_samples > 1`, but `StartController` currently resets EMA, submits exactly one sample, and requires `EmaStatus::Ready`.

Therefore a controller configured with `warmup_samples > 1` cannot successfully start.

Astra High must define the intended lifecycle before further implementation.

Preferred direction:

* do not acquire output authority while processing warm-up samples;
* represent warm-up/preparation explicitly enough that the controller can accumulate required Good samples before becoming Running;
* stale/unavailable input during warm-up must not acquire or retain output authority;
* resume/reinitialize must use the same explicit policy.

If the architecture deliberately chooses to support only `warmup_samples == 1` for controller-owned EMA, this must instead be validated explicitly at configuration time and documented as a deliberate limitation. Do not leave the current implicit contradiction.

### M4 issue 2: long-running controller lease renewal

A native Runtime-owned controller obtains a bounded OutputLease with `lease_lifetime`, but the current controller lifecycle has no explicit renewal mechanism.

A healthy native controller must not inevitably fail merely because the original lease reaches its expiry during a long-running experiment.

Astra High must define bounded lease renewal semantics.

Required properties:

* leases remain finite;
* renewal must occur only while the Runtime-owned controller is healthy and Running;
* renewal must not create a new way to bypass freshness, controller lifecycle or OutputAuthority;
* stale/failed controller execution must stop renewal and allow expiry/revoke;
* external/manual client leases must NOT inherit automatic Runtime-controller renewal semantics;
* Babashka starting a Runtime-owned PID later must not make that PID dependent on the Babashka session lifetime;
* epoch/ownership fencing must remain valid;
* do not solve this by assigning effectively infinite leases.

Prefer a small explicit renewal/heartbeat contract rather than rebuilding OutputAuthority ownership.

### Physical evidence constraint

M3 correctly keeps protocol acknowledgement distinct from readback. Preserve this.

Do not allow Lua or any later external API to fabricate `ReadbackVerified`.

If an actual physical SafeProfile requires readback, successful write ACK alone must remain insufficient until a trusted Rust-owned readback/reconciliation path exists.

### General review result

Preserve:

* Runtime as the single mutable owner;
* extensible engineering units;
* actual-first-byte authority revalidation;
* partial-write ambiguity;
* no blind write retry;
* independent Reference;
* PID/EMA without transport/output access;
* safe failure on stale/unavailable input;
* English teaching-oriented rustdoc/comments.

Next model:
ASTRA_HIGH

Authorized next work:

1. design the two M4 lifecycle corrections above;
2. design the M5 bounded Lua trust/isolation contract;
3. write acceptance criteria for Sol High;
4. STOP before production implementation.

Do not begin M5 implementation.
