# Agent Instructions

This repository is an architecture-first greenfield project.

## Current phase

The current phase is:

```text
handoff: M4 correction and M5 designs fixed; waiting for ASTRA_HIGH -> SOL_HIGH
```

Follow NEXT_IMPLEMENTATION_PLAN.md sequentially from M2 through M6. Preserve M1 behavior; finish and verify each milestone before beginning the next.

Mandatory user model gate: after M2 completion and the M3 design/acceptance contract,
update AI_HANDOFF.md, explicitly request Astra High -> Sol High, and STOP. Do not
implement M3 before the user switches models. This direct user instruction overrides
IMPLEMENTATION_ROADMAP_M2_M6.md section 16's prohibition on M3 design. Later model
handoffs also require an explicit stop; authorization through M6 never bypasses them.

The user crossed the M4 gate. ASTRA_M5_DESIGN_INSTRUCTIONS.md is authoritative for
this phase. Consume the external reviewer response in AI_HANDOFF.md, design the two
M4 lifecycle corrections, then the M5 Lua boundary and Sol acceptance contract.
Do not implement M4 corrections, add Lua dependencies, implement M5, or design M6
during this Astra phase. Finish the design, update AI_HANDOFF.md, request
ASTRA_HIGH -> SOL_HIGH and STOP. Sol must fix/verify M4 before implementing M5.

The Astra design is complete. Next Sol implementation is governed by
docs/implementation/MILESTONE_4_LIFECYCLE_REVIEW.md, then
docs/implementation/MILESTONE_5_DESIGN.md. Its W1–W9/R1–R10 prerequisites precede
Lua dependencies; M5 must satisfy L1–L22. The current 72-test baseline still has
the reviewed M4 lifecycle defects; a design document is not an implementation fix.
After M5, update AI_HANDOFF.md and stop for external review. Do not begin M6 design
or implementation or cross the present model gate automatically.

Read AI_HANDOFF.md before each milestone. It is the only dedicated external-review handoff. Stop for unresolved architectural forks with WAITING_FOR_REVIEW; after M6 mark READY_FOR_EXTERNAL_REVIEW and stop.

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
