# AI Handoff

This is the single persistent external-review communication file. Read this,
`ai/ROADMAP.md` and `ai/WORK.md` before any subsequent phase. Details of the
implementation and evidence belong in the milestone reports.

## Current state

```text
STATUS: WAITING_FOR_EXTERNAL_REVIEW
Current model: SOL_HIGH
Completed: corrected M4 controller lifecycle and bounded M5 Lua extensions
Next: external architecture/safety review
Do not begin M6 design or implementation before a reviewed next-phase instruction.
```

Exact HEAD of the verified implementation **and M5 report, immediately before
this final handoff coordination commit**:
`edcfa699d9c7ebe84c387d271215d65173b2b358`.
The only other workspace status entry is the pre-existing, untracked
`ai/project_snapshot.txt`; it is user-owned and untouched. The final handoff
commit itself advances Git HEAD, which reviewers can inspect with `git log -1`.
No uncommitted project implementation or report change remains.

## Verification

The uncorrected Astra review checkpoint had 72 passing tests and identified the
M4 lifecycle defects. Tests-first M4 W1–W9/R1–R10 corrections passed at 92 tests
before Lua was added. The finished M5 workspace has **123 tests**, passing in both
debug and release. Windows/Cargo 1.95.0 gate, all exited 0:

- `cargo fmt --all -- --check`
- `cargo test --workspace`; `cargo test --workspace --release`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo doc --workspace --no-deps`
- `cargo run -p lab-runtime` (original finite M1 demo)
- Explicit real-Lua, deadline, supervisor hard-stall, finite M5 and M3 recovery tests
- `cargo tree -p lab-core` (no external deps);
  `cargo tree -e features -p lab-lua`; `git diff --check`

M4 correction W1–W9 and R1–R10 map to
`docs/implementation/MILESTONE_4_LIFECYCLE_REVIEW.md` and the corrected section
of `docs/implementation/MILESTONE_4_REPORT.md`. M5 L1–L22 are mapped with named
evidence and measured timing in
`docs/implementation/MILESTONE_5_REPORT.md`. A deliberate real Lua exception
faults only its dependent controller/output, while a second native loop and
Rust-owned safe/Metakon recovery remain serviceable. Reload never rearms a
faulted controller without Rust virtual safe readback and explicit reset.

Dependency: `lab-lua` pins `mlua = 0.11.6`, default features off, only
`lua54` and `vendored`; lock resolves `mlua-sys 0.10.0`.
Lua source has no filesystem, process, raw transport, physical evidence or
output-authority binding. Core stays std-only. The two fixed worker slots use a
100 ms independent admission-to-cleanup acceptance deadline; the real
late-poll probe waits 110 ms, while a controlled two-slot stall yields two
unfinished slots after approximately 200 ms bounded shutdown grace and permits
test-harness release/join.

## Reviewable commit sequence

- `e60bb84` — canonical ai coordination files and current AGENTS routing.
- `34cf8fc`, `986eba0`, `a97f20a` — M4 red regressions, production corrections
  and separate corrected M4 documentation.
- `6672284`, `26919d3`, `5d6fb96` — managed signal/port red evidence and Core
  language-neutral component seam.
- `b20b453`, `aad363c` — pinned Lua/VM red evidence and red finite run.
- `10b7021`, `a37dad3`, `10a3879` — fenced Core recovery, real bounded Lua
  workers and finite independent-safety integration.
- `82f7e3d` — quota, deadline, identity and controlled stalled-shutdown closure.
- `edcfa69` — separate M5 acceptance report.

The M4 original baseline and earlier M1–M3 commit history remain in milestone
reports. The donor remains read-only; no physical actuator test was performed.

## Known limits and external review request

The proof is in-process and virtual. Lua instruction hooks, memory quota and
result validation isolate ordinary script failures, but a stalled native call
may remain physically active after its 100 ms acceptance deadline. The deadline
cannot preempt OS/process/Runtime stalls or certify hardware safe state.
Rust ServiceSafety must be called explicitly; there is no autonomous scheduler.
Physical ACK is not readback and cannot satisfy a Readback-required SafeProfile.
No physical Lua driver/controller, COM deployment, Recorder, long soak or M6 work
was implemented.

Please review the W1–W9/R1–R10 lifecycle corrections and the L1–L22 bounded
Lua implementation against their design contracts. In particular assess
native-only renewal privilege, same-time Unavailable fencing, worker quarantine,
deadline acceptance, explicit failed-controller recovery, Rust-owned evidence
and the virtual-only safety boundary. Record the reviewer decision below before
any next phase is authorized.

## Reviewer response

Current request: **M4 corrections + M5 implementation; external decision pending**.

The previous reviewer explicitly approved the SOL_HIGH M4-correction/M5 phase
with these continuing constraints: do not weaken M2/M3 output authority or
ACK/readback semantics, give Lua raw transport or safety authority, tie
controller lifetime to client lifetime, use effectively infinite native
leases, expand M5 into physical Lua drivers/controllers, or demote native Rust.

Next reviewer decision and unresolved architectural forks: **awaiting response**.
Do not replace this status with permission to proceed until the external
decision and a new user-authorized phase instruction are recorded.
