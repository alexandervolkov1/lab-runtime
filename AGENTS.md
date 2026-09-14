# Agent Instructions

This repository is an architecture-first greenfield project.

## Current phase

The current phase is:

```text
high-level architecture design
```

Do not implement the runtime yet.

Do not create Rust crates, Cargo workspace structure, Lua modules, Babashka modules or GUI code unless the user explicitly moves the project to the implementation phase.

## Source of truth

Read:

```text
PROJECT_BRIEF.md
```

before doing any architectural work.

## Relationship to v1

The existing `com_port_reader` project is a separate stable v1 implementation.

Do not copy its architecture automatically.

At this phase the purpose is to design lab-runtime from the product requirements.

A later dedicated phase will compare lab-runtime architecture with v1 code and decide what should be reused.

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

During the initial architecture phase, modify only documentation and architecture decision files.

Do not commit automatically unless the current user instructions permit commits.

## Language

Conversation and architecture-review output may be in Russian during the design phase.

Production source documentation can be standardized to English when implementation begins.
