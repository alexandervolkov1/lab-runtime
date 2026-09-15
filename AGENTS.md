# Permanent coordination files

The permanent coordination files are:

ai/HANDOFF.md
ai/ROADMAP.md
ai/WORK.md

Read all three before each new phase.

ai/HANDOFF.md is the only file used for external reviewer communication.
ai/WORK.md contains the current phase instruction and is replaced at a phase handoff.
ai/ROADMAP.md contains the persistent product roadmap.

Legacy root-level model instruction/handoff files are temporary compatibility
material and will be removed during the mandatory pre-release Markdown cleanup.


# Agent Instructions

This repository is an architecture-first greenfield project.

## Current phase

The current phase is:

```text
M6 implemented and verified against H1–H28 at 189 workspace tests.
READY_FOR_EXTERNAL_REVIEW: SOL_HIGH has stopped for reviewer response.
Do not begin M7 design or implementation before a separate reviewed gate.
```

Follow NEXT_IMPLEMENTATION_PLAN.md sequentially from M2 through M6. Preserve M1 behavior; finish and verify each milestone before beginning the next.

Earlier milestone gates were crossed explicitly. Current model/review gates are
recorded in `ai/HANDOFF.md` and `ai/WORK.md`; legacy root instructions are historical
compatibility material. Authorization through M6 never bypasses a required stop.

M1–M5 are implemented. SOL_HIGH corrected and verified M4 at 92 tests before adding
bounded Lua model/transform support, then verified M5 at 123 tests. The external
reviewer approved this checkpoint and the user authorized ASTRA_HIGH M6 design.
The workspace has lab-core, lab-lua and lab-runtime; Warming owns no lease, native
renewal follows successful trusted delivery, and Lua cannot bypass Rust safety.
The default executable remains finite. Explicit `--serve --profile virtual-demo`
starts the autonomous M6 host and bounded loopback API. Actual Babashka A/B
process acceptance passed. Recorder, real COM deployment and hardware acceptance
remain outside this checkpoint.

Follow `ai/WORK.md` as the authoritative current-phase instruction. The fixed M6
architecture and acceptance are in `docs/implementation/MILESTONE_6_DESIGN.md`;
implemented H1–H28 evidence is in `docs/implementation/MILESTONE_6_REPORT.md`.
The user explicitly crossed ASTRA_HIGH -> SOL_HIGH, and SOL_HIGH completed M6.
The present external-review stop is mandatory. Preserve accepted M1–M5 contracts
and do not begin M7 design or implementation or cross this review gate automatically.

Read `ai/HANDOFF.md` before each milestone. It is the only dedicated external-review
handoff. Stop for unresolved architectural forks with WAITING_FOR_REVIEW; after M6
mark READY_FOR_EXTERNAL_REVIEW and stop.

Do not implement M7 recorder, GUI, physical production deployment, long soak or full product parity. No dangerous physical actuator tests; donor remains read-only.

Implement only concepts required by the current milestone. A concept in the target architecture is not a reason to implement it ahead of time.

## Source of truth

Read:

```text
PROJECT_BRIEF.md
```

before doing any architectural work.

## Relationship to v1

The existing `com_port_reader` project is a separate stable v1 implementation.

Do not copy its architecture automatically.

Architecture and migration analysis in docs/architecture and docs/migration form the target baseline, not permission to copy the donor structure.

The donor is read-only and never a workspace dependency. Milestone 1 uses newly written production code; document reused test intent/validation knowledge.

## Design priorities

In order:

1. safety;
2. conceptual clarity;
3. extensibility;
4. interactive development;
5. reliability for long-running experiments;
6. testability;
7. simplicity;
8. performance.

Do not optimize for hypothetical extreme throughput.

Typical workloads are small compared with general data-acquisition systems.

## Architecture style

Prefer:

* explicit domain boundaries;
* dependency inversion where it solves a real problem;
* data-driven capabilities;
* descriptors/introspection;
* composition;
* clear state ownership;
* clear lifecycle semantics.

Avoid:

* speculative abstractions;
* large generic frameworks;
* trait proliferation;
* unnecessary micro-crates;
* dependency on a specific GUI;
* dependency on one scripting language;
* Rust ABI plugin systems without a demonstrated need.

## Extension philosophy

The system should support several extension levels:

* native Rust;
* data-driven definitions;
* embedded scripting;
* external orchestration;
* possibly compiled plugins later.

These levels should share one coherent domain model rather than becoming independent parallel applications.

## Safety

No extension mechanism may bypass the central output-safety/arbitration layer.

Physical output control remains authoritative in Rust.

## Git

Use:

```text
1 logical architectural change = 1 commit
```

Keep documentation/baseline commits separate from implementation commits. Preserve unrelated user files and stop after the authorized milestone.

Do not commit automatically unless the current user instructions permit commits.

## Language

Conversation and architecture-review output may be in Russian during the design phase.

All Rust module docs, rustdoc, source comments, TODO/FIXME and documentation examples must be in English. Document every public API and important private ownership/state/time/safety invariant meaningfully. Enforce missing-docs; prefer clear teaching code over compressed cleverness.
