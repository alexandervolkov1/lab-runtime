# Current work — M9C remove Lua and obsolete client baggage

```text
M8: ACCEPTED
M9A: ACCEPTED
M9B: ACCEPTED
M9C: AUTHORIZED
Current phase: M9C — remove Lua and obsolete client baggage
M10+: NOT AUTHORIZED
```

M9C is the only current implementation authorization. Do not begin M10, M11, M12
or any post-v0.1 work automatically. This coordination update authorizes the next
implementation slice but does not itself perform production removal.

M9B's coherent bounded Application API was externally accepted at
`d228d697c01a4b77d55333655129f95ee784c07c`. Preserve its laboratory semantics,
ownership, safety, bounds, structured errors, reconnect/resynchronization behavior
and generic extension model while simplifying the active product.

## Goal

Remove Lua and obsolete first-party client baggage from the active v0.1 product
without replacing them with another scripting language or client layer.

The target active architecture is:

```text
Application API
      ↓
authoritative Runtime
      ├── native instruments
      ├── native managed components
      ├── native controllers / OutputAuthority
      ├── Recorder
      └── virtual instruments / emulator API
```

Runtime remains the sole authoritative mutable experiment owner. Clients remain
disposable requesters/observers. No embedded or user scripting language is required
for v0.1.

## Authorized removal scope

Remove active Lua product implementation and vocabulary:

- the `lab-lua` crate and `mlua` dependency;
- implementation identity `lua.v1`;
- Lua runtime, worker and sandbox machinery;
- Lua source/configuration paths and active Lua-only fixtures;
- active Lua-specific Application API vocabulary;
- `reload_managed_sources` and capability `managed_source_reload`;
- other active Lua-only names identified by the accepted M9B review.

Remove obsolete first-party client baggage:

- the active Babashka client implementation and product/user workflow;
- Babashka-specific process acceptance that exists only to validate the removed
  client;
- client SDK/example material that is not part of the headless v0.1 product.

Replace coverage that remains necessary with implementation-neutral Rust,
raw-protocol or process integration tests. Do not weaken accepted API, Runtime,
Recorder, control, safety, reconnect or isolation guarantees merely because a
temporary client or adapter is removed.

Inspect the transitional `runtime_snapshot`, `snapshot_page` and
`snapshot_release` family. Remove it if accepted discovery/current/domain-specific
surfaces fully supersede it. Retain only genuinely necessary neutral semantics; old
Babashka convenience is not a reason to preserve it.

## Required neutral architecture

Do not remove or specialize the generic managed-component contract. Preserve as
appropriate:

- neutral `ComponentDefinition` and `ComponentImplementationId`;
- the BuiltIn/native implementation path;
- `Invocation`, `ComponentResult` and `ComponentCompletion`;
- `ComponentExecutor` and the neutral bounded executor architecture;
- bounded `PlainData`;
- generation/revision fencing and scoped failure;
- native components including `native.moving_mean.v1`;
- neutral managed-component configuration and provenance.

Ordinary native components must continue to project through generic discovery,
current measurement, history, subscription and property/configuration surfaces
without component-specific Application API operations.

## Historical compatibility and evidence

Active product cleanup is not historical erasure. Preserve:

- historical M5 and later milestone documentation;
- immutable SQLite archives and hardware evidence;
- the existing Recorder schema and old recorded data;
- historical provenance vocabulary such as `managed_lua_source` wherever needed to
  understand or validate immutable archives and evidence;
- read/inspection compatibility that is genuinely required for historical data.

Do not rename old schema fields, rewrite archives or alter evidence merely to remove
the word Lua.

```text
active product surface != historical compatibility/evidence
```

## Invariants to preserve

- `lab-core` remains OS-, storage-, presentation- and scripting-runtime-independent.
- Required acquisition, native control, OutputAuthority, Recorder and lifecycle
  work retain priority over external/extension work.
- No client or extension can fabricate transport completion, ACK, readback, safe
  evidence, an `OutputLease` or an `OutputAuthority` decision.
- Finite leases, generation/epoch fencing, final authority recheck, no blind retry
  after ambiguous output and no automatic rearm remain unchanged.
- Recorder remains Runtime-owned, bounded and independent of client lifetime.
- Application API, Recorder contract and SQLite schema remain separate contracts.
- All queues, histories, workers and retained state remain explicitly bounded.
- Historical M8 evidence remains unchanged; COM5 is outside this work.

## Non-goals

Do not add:

- GUI or Presentation API;
- Steel, Python or a replacement scripting language;
- a new first-party client or SDK;
- new laboratory domains or convenience API operations;
- new physical instrument protocols or hardware acceptance;
- broad Core studyability refactoring assigned to M10;
- M11 hardening/logging redesign;
- M12 tutorials, packaging or final release documentation;
- a post-v0.1 roadmap.

M9C is simplification, not substitution.

## Completion and review gate

Inventory active versus historical Lua/Babashka references before removal. Keep
changes logically separated, preserve required neutral tests, and document the
final removed/retained surface and migration implications.

M9C completion requires focused replacement/regression tests and the full
debug/release/fmt/Clippy/warning-denied-rustdoc gates, followed by explicit external
review. Do not authorize or begin M10 automatically.
