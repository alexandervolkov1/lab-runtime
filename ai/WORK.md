# Current work — M10 ready for external re-review

```text
M8: ACCEPTED
M9A: ACCEPTED
M9B: ACCEPTED
M9C: ACCEPTED
POST-M9C HARDWARE SMOKE: PASS
M9D: ACCEPTED
M10: READY_FOR_EXTERNAL_REVIEW
M10.1: COMPLETE
M10.2: COMPLETE
M10.3: COMPLETE
M10.4: COMPLETE
M10.5: COMPLETE
M10.6: COMPLETE
M10.7: COMPLETE
M10.8: READY_FOR_EXTERNAL_REVIEW
Current phase: M10 — external re-review after property-projection correction
M11+: NOT AUTHORIZED
```

M9D software integration, physical acceptance and external review are complete. Its
accepted production path is:

```text
native controller
→ OutputProposal
→ OutputAuthority
→ bounded ResourceExecutor
→ final authority/generation recheck
→ Metakon WRITE reg06
→ strict ACK
→ separate reg06 readback
```

The clean acceptance archive is
`examples/metakon-513-m9d-write-smoke.sqlite`, SHA-256
`14ec74be2a33cb795b29dcc295acc323a0dd4d8cf44b2cc5f6f64fd29e775c32`.
It proves `0 → ACK → readback 0`, `+10 → ACK → readback 10`, `0 → ACK →
readback 0` and clean shutdown. The load/heater was physically disconnected, so
physical heater effect was intentionally not tested. Existing M8, post-M9C and M9D
archives remain immutable.

## Completed slice

`M10.1 structural/studyability audit` is complete. Its authoritative design record is
`docs/implementation/MILESTONE_10_STRUCTURE_AUDIT.md`. M10.2 implemented its
low-risk terminology, archaeological and source-index slice. M10.3 organized the
Application facade, delivery/operation lifecycle and semantic domain handlers while
preserving the authoritative 42-operation registry and public wire contract. M10.4
split Runtime, HostCore and ServiceHost implementations by orchestration
responsibility while retaining the exact structs, fields, owners and progression
order. M10.5 separated semantic Recorder types, the single bounded `RecorderWorker`
and direct SQLite persistence into focused modules while preserving all lifecycle,
schema, transaction, provenance and history behavior. The implementation record is
`docs/implementation/MILESTONE_10_REPORT.md`.

M10.6 centralized the compile-time native component registry, proved a second
test-only component through generic API surfaces, and extracted configured instrument
composition into one explicit Host module. M10.7 tightened internal visibility,
removed an unused managed-executor test barrier, improved architectural rustdoc and
made high-value regression oracles easy to locate. The first M10 external review
found one blocker: the generic Application property projection enumerated concrete
`InstrumentDto` variants. The correction moved typed neutral property metadata and
the static provider seam into `configuration.rs`; `configuration_api` now converts
that metadata without knowing concrete instrument variants. Exact current DTOs and
a second test-only property provider are covered by focused regressions. M10 is ready
for external re-review. M11 work is not authorized.

M10 may split oversized or mixed modules by responsibility, improve internal names,
remove safe dead compatibility/code, tighten public/private boundaries, improve
rustdoc on architectural seams, make main paths easier to follow, simplify native
instrument/component extension where semantics remain unchanged, and reorganize
tests for readability without reducing coverage.

M10 must preserve:

- the accepted 42-operation Application API and its capability/error/bounds
  semantics;
- periodic acquisition and physical/virtual instrument behavior;
- native managed components and Reference/controller/PID behavior;
- OutputAuthority and all accepted M9D physical-output semantics;
- Recorder schema, semantics and provenance;
- reconnect/generation fencing, API backpressure and client isolation;
- the emulator safety boundary and all hardware-facing Metakon behavior.

M10 must not add product features, change public Application API semantics or the
SQLite schema, redesign scheduling or OutputAuthority, change transport/protocol
behavior, add GUI/Presentation or scripting, add the M11 logging subsystem, or begin
the final tutorials/documentation work planned for M12.

Runtime remains the sole authoritative mutable owner. Physical writes require
OutputAuthority; raw transport write bypass is not an ordinary production surface.
Finite leases, epoch/generation fencing and final authority recheck before the first
possible output byte remain mandatory. ACK is not readback, readback is not physical
effect, and timeout after send-started is not proof that no write occurred. There is
no blind retry. An ambiguous started safe WRITE remains latched and non-retriable,
its safe obligation remains recorded, unresolved ambiguity prohibits normal output,
and safe transition does not automatically rearm the controller.
