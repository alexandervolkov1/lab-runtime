# Agent Instructions

This repository is an architecture-first greenfield project.

## Current phase

The current phase is:

```text
implementation: milestone 1 domain foundation
```

Implement only Milestone 1 from FOUNDATION_MILESTONE1_PLAN.md: minimal Rust workspace, native virtual Instrument, typed descriptors/validation, stable identity, deterministic time, bounded Signal state and local Commands/Queries.

Do not proceed to Milestone 2 or implement OutputArbiter, output execution, serial/Metakon runtime, controllers, Lua, Babashka, IPC, recorder or GUI without a separate user instruction.

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

Production source documentation can be standardized to English when implementation begins.
