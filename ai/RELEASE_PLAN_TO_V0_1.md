# Release plan to v0.1.0

## Current status

```text
M8–M11: ACCEPTED
Developer-preview technical gate: PASSED
Core technical implementation for v0.1: FUNCTIONALLY COMPLETE
v0.1.0-preview.1: PUBLISHED
M12.1 WebSocket architecture audit: ACCEPTED
M12.2 transport-neutral server seam: ACCEPTED
M12.3 bounded WebSocket/JSON transport: ACCEPTED
M12.4 transport parity/fault acceptance: ACCEPTED
M12.5 browser/ClojureScript smoke acceptance: ACCEPTED
M12 consolidated external review: READY FOR EXTERNAL REVIEW
M13: NOT AUTHORIZED
Current phase: M12 consolidated external review
```

The accepted M12.2 implementation commit is
`0e1bcb228f068eb1fcec6116eebc64f3352e520d`.
The accepted M12.3 implementation commit is
`6fb867389426fa75033f54312fda8c5556c52ed7`.
The accepted M12.4 implementation commit is
`9b58e92cffa79087f60d78032bc34b89499ae961`.
The accepted M12.5 implementation/evidence commit is
`f291edf35a7805245ce19ea088b9ee899d57f30e`.

The consolidated M12 source, bounds, security, parity and real-browser evidence is
recorded in `M12_CONSOLIDATED_EXTERNAL_REVIEW.md` and is ready for external review.
M13 remains unauthorized.

Functionally complete means the accepted Runtime behavior is implemented and no
known preview-blocking correctness, safety or durability defect remains. It does not
mean production certification, exhaustive hardware qualification or final release
readiness.

## Completed preview preparation

The preview package needs concise, current developer material:

- architecture/concepts;
- complete Application API semantics and bounds;
- Recorder/SQLite schema, provenance, gaps, sealing and read-only inspection;
- safety/failure/recovery guarantees and evidence limits;
- configuration and instrument/component extension paths;
- build, run and getting-started workflow;
- bounded diagnostics collection;
- reproducible preview packaging.

This preparation did not change the accepted 42-operation / 25-capability API,
SQLite schema, scheduler, output safety or Runtime ownership.

## Current path to v0.1.0

```text
v0.1.0-preview.1
    ↓
M12 WebSocket transport
    ↓
M13 external Steel scripting host
    ↓
M14 GUI / Workbench / client-owned PresentationDocument
    ↓
thermal-plant-uno end-to-end integration validation
    ↓
final documentation / packaging / review
    ↓
v0.1.0
```

M12 adds a bounded loopback WebSocket/JSON adapter to the same Application API,
session state and global client budget as TCP/NDJSON. It must not create transport-
specific operations, DTOs, errors, history or experiment semantics.

M13 hosts Steel outside Runtime as an Application client. Steel crash or script
termination must not destroy authoritative experiment state:

```text
script lifetime != experiment lifetime
```

M14 keeps GUI/Workbench behavior in a separate client. A future
`PresentationDocument` is client-owned and transport/language-neutral; it is not a
Runtime experiment-semantic DTO.

Across M12-M14:

```text
Runtime owns experiment semantics.
Client owns presentation semantics.
```

No UI or scripting semantics may enter Runtime core.

## End-to-end integration validation

```text
Arduino thermal plant
→ Rust physical instrument integration
→ experiment-specific Clojure client
→ Clay live browser view
→ thermal experiments
→ sealed SQLite archives
→ Clojure/Clay system-identification notebook
```

This validates the completed transport, external scripting-client and presentation
boundaries in practice. It is not retroactive evidence for M8-M11 acceptance and
does not change physical-evidence limits.

## Final release gate

Before v0.1.0, complete the polished reference/tutorial set, supported-platform and
recovery guidance, archive documentation, licensing/checksum review, clean Windows
build/package acceptance and external release review.

The final package should contain the executables, example deployment material,
instrument definitions, developer documentation, applicable license files and
checksums. It must not bundle historical AI reports, development fixtures or
obsolete Lua/Babashka clients. GUI/Workbench packaging, if provided, remains a
separate client artifact and does not enter Runtime core.

## Residual qualification limits

- no multi-day unattended soak;
- no physical disk-full or real power-loss qualification;
- no exhaustive USB/driver or Arduino fault-injection evidence;
- no hard-real-time guarantee;
- no proof of physical heater effect;
- no remote/network-security qualification.

These limits are explicit and do not currently block developer preview.
