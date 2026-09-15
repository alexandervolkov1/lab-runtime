# AI Handoff

This is the single persistent external-review communication file. Read
`ai/HANDOFF.md`, `ai/ROADMAP.md`, `ai/WORK.md` and `AGENTS.md` before each phase.
Detailed contracts and implementation evidence belong in normal project docs.

## Current state

```text
STATUS: WAITING_FOR_USER_MODEL_SWITCH
Completed model: ASTRA_HIGH
Completed work: M6 architecture/design and tests-first acceptance contract
Requested switch: ASTRA_HIGH -> SOL_HIGH
Resume after explicit user switch: M6 implementation only
Do not begin: M7 Recorder design or implementation
```

M6 design is complete in
[the M6 design and H1–H28 acceptance contract](../docs/implementation/MILESTONE_6_DESIGN.md).
This is an Astra design checkpoint, not completed M6 implementation or a claim of
external approval of M6. No M6 production code/dependencies/tests were added.
There is no unresolved architectural fork; the pending gate is the user model switch.

## Reviewed implementation checkpoint

Corrected M4 and completed M5 are **APPROVED** by the external reviewer.
The approved implementation checkpoint is `cb98d55`:

- M1: domain foundation and finite virtual executable.
- M2: central OutputAuthority and arbitration.
- M3: bounded transport executor, narrow Metakon and data-driven definitions.
- M4: thermal plant, native EMA/Fixed/Ramp/PID; corrected Warming and finite native
  renewal, verified at 92 workspace tests before adding Lua dependencies.
- M5: bounded Lua model observations and moving-mean transform, verified at 123 tests.

Accepted implementation contracts/evidence:

- [M4 lifecycle review](../docs/implementation/MILESTONE_4_LIFECYCLE_REVIEW.md).
- [M4 report](../docs/implementation/MILESTONE_4_REPORT.md).
- [M5 design](../docs/implementation/MILESTONE_5_DESIGN.md).
- [M5 report](../docs/implementation/MILESTONE_5_REPORT.md).

The reviewer approved supplied source/snapshot and recorded verification evidence;
the reviewer did not independently run Cargo on this machine. Astra independently
reran `cargo test --workspace` (123 passing) and
`cargo clippy --workspace --all-targets -- -D warnings` (passing) before M6 design.
Incoming reviewer comments and the full Astra mandate are preserved in `a8c68ff`.

## Accepted invariants carried into M6

Warming owns no lease, integrates no PID and emits no ordinary proposal. Only
distinct fresh Good observations advance warm-up; activation acquires authority
when preparation finishes. Start/Resume reset and warm; independent Reference
progress follows monotonic Runtime time, including controller pause.

Native renewal remains crate-private and follows successful trusted Running
delivery. It retains authority instance, owner and epoch, with a new finite expiry;
the old token becomes stale. Expiry/revoke/pause/trip, bad input, failed algorithm,
failed delivery or lost ownership stop renewal. Numeric Automatic/controller IDs
never grant clients that privilege. A client starting a native controller does
not own its lifetime.

Lua remains read-only model/transform work with `mlua 0.11.6`, Lua 5.4 vendored,
default features disabled. Core is std-only. Two fixed workers, disposable VMs,
capability allowlist, memory/instruction/data/diagnostic/host-call bounds and
100-ms acceptance/dependency deadlines remain in force. Stalled slots quarantine;
no unlimited replacement or owner wait. Correlation/generation/revision/deadline
fences reject obsolete results. Replacement validates before atomic commit and
re-warms. No Lua raw transport, output authority, OS access or physical evidence.
ACK is not readback. Runtime service must progress independently of workers.

The implementation remains a virtual/software proof: the executable is finite;
there is no M6 autonomous scheduler/API/Babashka client, Recorder/SQLite, GUI,
real COM adapter or hardware acceptance. A Lua deadline fences acceptance, not
physical thread execution. No hard real-time or physical safety certification.

## Completed Astra work

1. Synchronized stale README/AGENTS with reviewed M5 before material M6 design:
   `aaca40f` (separate documentation commit).
2. Preserved the incoming review/phase mandate: `a8c68ff`.
3. Fixed M6 architecture and tests-first acceptance: `1d18317`.
4. Updated README/AGENTS and all three ai coordination files for this model gate.

M6 contract decisions:

- One Runtime owner, explicit monotonic periods, safety-first bounded scheduling;
  std nonblocking network reactor and fixed Lua workers have no shared Runtime lock.
- Loopback-only bounded version-1 NDJSON; separate host DTOs, pure queries and
  serialized domain commands, explicit accepted/completed/failed/unknown outcomes.
- Safe Ready/Paused PID configuration with shared Core validation/revisions;
  continuous live Ramp retune; no remote direct output or evidence capability.
- Server-issued scopes and monotonic request numbers, bounded retained outcomes,
  same-ID conflicts, explicit unknown after eviction/restart and no blind replay.
- Bounded semantic events, frozen snapshot/cursor/pages, ordered replay/filter
  progress, explicit gap/resync and disconnect policy for slow clients.
- Producer stop barrier, nonblocking worker quiesce, safe/recovery grace before
  transport teardown; missing evidence and unfinished workers reported honestly.
- Mandatory real Rust host + actual bb A/B process acceptance, client death,
  autonomous operation beyond three finite leases, reconnect and safe pause.

## Sol resumption and external-review gate

After the user explicitly switches to SOL_HIGH, follow `ai/WORK.md` and M6 design
section 12 sequentially. Write failing behavior tests before each production
slice, satisfy all H1–H28 and preserve M4 W1–W9/R1–R10 plus M5 L1–L22.
Missing actual bb execution leaves M6 incomplete; a Rust fake client or skipped
test cannot satisfy it. The design workstation has `babashka v1.13.220`; that is
availability evidence only.

Record implementation/test/process evidence in `MILESTONE_6_REPORT.md`. After M6,
update this file to `READY_FOR_EXTERNAL_REVIEW` and STOP. If a genuine architecture
contradiction remains unresolved, use `WAITING_FOR_REVIEW`, describe the exact
fork and stop. Do not cross either gate automatically or begin M7.

## Later release work and workspace preservation

After reviewed M6, the release sequence still requires M7 Recorder/SQLite,
runtime/release hardening, real Windows COM, safe read-only Metakon smoke,
Markdown consolidation, final documentation/tutorial and release packaging.
These are future phases, not permission to design or implement them now.
The v1 donor remains read-only and never a workspace dependency.

`ai/project_snapshot.txt` was already an untracked user file at this phase's
start. It was not edited or staged; preserve it.
