# Roadmap to v0.1.0

## Current state

```text
M8–M11: ACCEPTED
Developer-preview technical gate: PASSED
Core technical implementation for v0.1: FUNCTIONALLY COMPLETE
Current phase: Developer Preview Preparation
```

The accepted headless Runtime core is implemented and technically hardened for a
developer preview. Remaining work concerns reference material, practical integration
validation and release preparation, not an unplanned feature milestone.

## Developer Preview Preparation

The compact developer-facing reference is complete:

1. architecture and concepts guide;
2. Application API reference;
3. Recorder/SQLite archive reference;
4. safety/failure/recovery cheat sheet;
5. configuration and instrument/component extension notes;
6. getting-started/build/run guide;

Preview packaging/build remains not started and is the next separate preparation
step.

This is not yet the final polished release-documentation pass. Each execution slice
requires explicit authorization in `WORK.md`.

## Practical learning and integration

After preview preparation, the planned exercise is:

```text
Arduino thermal plant
→ Rust physical instrument integration
→ experiment-specific Clojure client
→ Clay live browser view
→ thermal experiments
→ sealed SQLite archives
→ Clojure/Clay system-identification notebook
```

The Arduino remains an external instrument/emulator and is not required to accept
M11 retroactively. Runtime continues to own experiment semantics; the Clojure/Clay
client owns presentation semantics.

TCP/NDJSON is sufficient for the first workflow. A future WebSocket transport may
only adapt the same Application sessions, operations and DTOs; it is not current
preview scope.

## Final release gate

The later final documentation/package audit will require polished architecture,
protocol, archive, configuration, safety/recovery and operational documentation plus
a reproducible checksummed Windows package. It is not authorized by the current
phase transition.

## Explicit non-goals

- no GUI or Presentation API;
- no bundled scripting runtime or first-party client SDK;
- no dynamic plugin framework;
- no new convenience API merely for documentation;
- no schema or accepted semantic changes;
- no claim of production certification, hard real-time or exhaustive physical
  qualification.
