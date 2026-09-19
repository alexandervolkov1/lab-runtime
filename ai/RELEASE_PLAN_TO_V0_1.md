# Release plan to v0.1.0

## Current status

```text
M8–M11: ACCEPTED
Developer-preview technical gate: PASSED
Core technical implementation for v0.1: FUNCTIONALLY COMPLETE
Current phase: Developer Preview Preparation
```

Functionally complete means the accepted Runtime behavior is implemented and no
known preview-blocking correctness, safety or durability defect remains. It does not
mean production certification, exhaustive hardware qualification or final release
readiness.

## Preview preparation

The preview package needs concise, current developer material:

- architecture/concepts;
- complete Application API semantics and bounds;
- Recorder/SQLite schema, provenance, gaps, sealing and read-only inspection;
- safety/failure/recovery guarantees and evidence limits;
- configuration and instrument/component extension paths;
- build, run and getting-started workflow;
- bounded diagnostics collection;
- reproducible preview packaging.

This preparation must not change the accepted 42-operation / 25-capability API,
SQLite schema, scheduler, output safety or Runtime ownership.

## Practical validation after preview preparation

```text
Arduino thermal plant
→ Rust physical instrument integration
→ experiment-specific Clojure client
→ Clay live browser view
→ thermal experiments
→ sealed SQLite archives
→ Clojure/Clay system-identification notebook
```

This validates extension and client boundaries in practice. It is not a prerequisite
for M11 acceptance. WebSocket is optional future transport over the same Application
semantics and is not required for the first workflow.

## Final release gate

Before v0.1.0, complete the polished reference/tutorial set, supported-platform and
recovery guidance, archive documentation, licensing/checksum review, clean Windows
build/package acceptance and external release review.

The final package should contain the executables, example deployment material,
instrument definitions, developer documentation, applicable license files and
checksums. It must not bundle historical AI reports, development fixtures, obsolete
Lua/Babashka clients or a GUI.

## Residual qualification limits

- no multi-day unattended soak;
- no physical disk-full or real power-loss qualification;
- no exhaustive USB/driver or Arduino fault-injection evidence;
- no hard-real-time guarantee;
- no proof of physical heater effect;
- no remote/network-security qualification.

These limits are explicit and do not currently block developer preview.
