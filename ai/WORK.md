# Current work — M6 model handoff

## Gate and authority

```text
Current state: M6 design complete; waiting for explicit user model switch
Completed model: ASTRA_HIGH
Requested next model: SOL_HIGH
Authorized after user switch: M6 implementation only
```

**ASTRA_HIGH must STOP here.** This file is a resumption instruction, not evidence
that the user has switched models. Do not implement M6 until that explicit switch.
Do not begin M7 design or implementation. This file is authoritative for the phase;
legacy root handoff/roadmap instructions are historical compatibility material.

Read first:

- `ai/HANDOFF.md`, `ai/ROADMAP.md`, `ai/WORK.md`, `AGENTS.md`.
- `PROJECT_BRIEF.md`.
- `docs/implementation/MILESTONE_6_DESIGN.md` in full, including H1–H28 and section 12.
- M4 lifecycle review/report and M5 design/report in `docs/implementation`.

Use the architecture/migration documents linked by the design as context; do not
implement future target concepts ahead of this milestone. The external reviewer
approved corrected M4/M5, not an implemented M6. README/AGENTS synchronization is
complete; the full incoming Astra mandate is preserved in commit `a8c68ff`.

## Sol baseline and implementation sequence

After the user switches, inspect status/log before edits and verify the reviewed
123-test baseline and clippy. Preserve unrelated changes and the pre-existing
untracked `ai/project_snapshot.txt`. Do not weaken an accepted test to hide a
regression. No donor edits/copying architecture or donor workspace dependency.

Implement M6 design section 12 in order, tests first for each slice:

1. Core Reference evaluation/continuous retune and atomic safe PID configuration
   with domain revisions and pure config snapshots.
2. Autonomous virtual host and explicit monotonic scheduler; managed shutdown
   begin/poll seam, stop barrier and bounded safe/recovery shutdown.
3. Bounded typed protocol, request/outcome/session store and loopback network
   reactor; pure queries, authority exclusions, fair isolated clients.
4. Semantic events, frozen snapshot/cursor/pages, subscription replay/gap/resync
   and restart handling.
5. Real fixed-worker Lua integration; both blocked workers must leave independent
   native renewal, safety, M3 recovery and external reads serviceable.
6. Thin Babashka client and actual-process reconnect acceptance, including killed
   bb A, more than three finite leases without a client, bb B reconciliation and
   safe pause. Missing/unusable bb is incomplete acceptance, not a skipped pass.
7. Full verification, H1–H28 evidence report, coordination update and external-review stop.

One owner mutates Runtime. No shared blocking Runtime lock or owner waits on
network/Lua. Core stays std-only. Host serialization types are not Core types or
a Recorder schema. Finite native leases renew only after trusted successful
progress; renewal preserves instance/owner/epoch. No API direct output, manual
producer, arbitrary Lua deployment, client clock or evidence completion.

Use the exact bounds/ordering/replay/shutdown policies in the design. Any genuine
unresolved architectural contradiction requires `WAITING_FOR_REVIEW` in HANDOFF,
with the precise fork, then STOP. Do not silently redesign accepted boundaries.
Routine implementation choices within this contract do not require another gate.

## Verification and report

New test files/functions use descriptive snake_case names, as suggested in the
H1–H28 matrix. No mass rename of old milestone tests. Record failing-before and
passing-after evidence for each meaningful slice. Use deterministic clocks and
worker/readiness barriers; never substitute arbitrary startup sleeps or a Rust
fake for the actual bb process.

```powershell
cargo fmt --all -- --check
cargo test --workspace
cargo test --workspace --release
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p lab-runtime
bb --version
cargo test -p lab-runtime --test babashka_reconnect -- --nocapture
git diff --check
git status --short
```

Create `docs/implementation/MILESTONE_6_REPORT.md` with each H ID mapped to named
passing tests, tool versions, actual process evidence, configured hard bounds,
shutdown/reconnect observations and remaining limits. Preserve M1 behavior,
M4 W1–W9/R1–R10 and M5 L1–L22. A design or the 123-test baseline is not M6 proof.

Use small logical commits; documentation/baseline commits stay separate from
production implementation. This phase permits those commits. Do not push without
user authorization. Document public APIs and important ownership/time/safety
invariants in English; maintain missing-docs and warning-free checks.

## Completion stop

After verified M6, synchronize README/AGENTS and ai coordination with implemented
facts. In `ai/HANDOFF.md`, mark `READY_FOR_EXTERNAL_REVIEW`, link the report and
STOP for external review. Do not design or implement M7 Recorder/SQLite, GUI,
physical production controls, real COM, long soak or full product parity.
