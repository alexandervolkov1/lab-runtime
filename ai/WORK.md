Current work — ASTRA_HIGH M7 Recorder/SQLite design

Read first:

ai/HANDOFF.md
ai/ROADMAP.md
ai/WORK.md
AGENTS.md
PROJECT_BRIEF.md

docs/implementation/RELEASE_PLAN_M7_TO_V0_1.md

docs/implementation/MILESTONE_6_DESIGN.md
docs/implementation/MILESTONE_6_REPORT.md

docs/architecture/HIGH_LEVEL_ARCHITECTURE.md
docs/architecture/RUNTIME_AND_SAFETY_MODEL.md
docs/architecture/EXTENSION_MODEL.md

docs/migration/V1_TO_LAB_RUNTIME_MAP.md
docs/migration/REUSE_PLAN.md
docs/migration/MIGRATION_RISKS.md

You are:

ASTRA_HIGH

Authorized phase:

M7 RECORDER / SQLITE DESIGN ONLY

Do not implement production M7 code.

Do not begin M8.

Baseline verification

Before design:

git status
git log --oneline -20
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings

Expected reviewed baseline:

M1–M6 complete
189 named Rust workspace tests at supplied M6 checkpoint
real Babashka A/B acceptance complete
Recorder/SQLite not implemented

Preserve the pre-existing user file:

ai/project_snapshot.txt

if it remains present/untracked/staged outside your task.

M7 design obligations

Create:

docs/implementation/MILESTONE_7_DESIGN.md

Use RELEASE_PLAN_M7_TO_V0_1.md as the product contract.

Inspect the actual v1 Recorder/SQLite implementation only as a read-only donor
for behavior/tests, not architecture.

Resolve explicitly:

Recorder ownership and thread/service boundary

record/event vocabulary

Runtime boot vs experiment run vs recording interval

configuration/script provenance

monotonic + wall-clock timestamps

bounded ingress by count and bytes

batching

SQLite durability policy

schema versioning

measurement storage

quality/unit/generation/revision

controller/reference lifecycle records

requested/sent/ACK/readback/failed/ambiguous evidence

structured logs/annotations

BestEffort vs Required recorder policy

Recorder-required control failure -> revoke/safe/fault

shutdown/flush outcome

database reopen/recovery semantics

bounded history query for future GUI

paging/downsampling contract

interaction with M6 semantic events without turning the event ring into the
durable database model

Do not design M8 configuration/COM yet except where a minimal M7 extension seam is
required.

Do not design GUI yet.

Do not expand Lua.

Required acceptance contract

Give M7 tests stable IDs D1–D18 or a stricter equivalent covering all requirements
listed in the release plan.

The acceptance contract must be concrete enough that SOL_HIGH can implement M7
without inventing persistence/safety architecture.

Require tests-first implementation.

M7 dependency choices

Research/select the smallest practical SQLite Rust adapter only if necessary for
the design.

Keep SQLite dependencies out of lab-core.

Do not introduce an ORM/framework without a concrete need.

Documentation quality

All proposed Rust source docs/comments remain English.

Design for code that the user can later study.

Explain why persistence, client events and Runtime authority remain separate.

End of Astra phase

Update:

ai/HANDOFF.md
ai/WORK.md
AGENTS.md

with:

Current model: ASTRA_HIGH
Next model: SOL_HIGH
Resume from: M7 implementation against MILESTONE_7_DESIGN.md
Do not start: M8

Then print:

MODEL HANDOFF

STOP HERE.

Completed:
M7 Recorder/SQLite architecture and tests-first acceptance contract.

Switch from:
ASTRA_HIGH

Switch to:
SOL_HIGH

Resume with:
M7 implementation only.

Do not begin M8.

Then STOP.
