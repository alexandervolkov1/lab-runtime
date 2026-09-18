# Current work — M10.1 structural/studyability audit

```text
M8: ACCEPTED
M9A: ACCEPTED
M9B: ACCEPTED
M9C: ACCEPTED
POST-M9C HARDWARE SMOKE: PASS
M9D: ACCEPTED
M10: AUTHORIZED
Current phase: M10 — core cleanup and studyability
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

## Authorized next task

M10 is authorized, but implementation does not start immediately. The first task is
`M10.1 structural/studyability audit`. It must map the actual post-M9D codebase,
responsibilities, main execution paths, architectural seams and candidate cleanup
before any refactoring slice is proposed or implemented.

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
