# AI Handoff

## Current milestone

M3 complete; M4 design and implementation not started.
Model used: SOL_HIGH.
Status: APPROVED_FOR_SOL_HIGH_M4.
Verified M3 checkpoint: 5f3e61d plus the following report commit.
Working tree: clean at the M3 report boundary; verify with git status.

## Completed

Read the full current project and plans. Implemented deterministic M2 authority,
preserved M1, added English teaching documentation and missing-docs enforcement.
60 tests pass after M3. Runtime now owns bounded byte executors, strict Metakon
framing/mapping, declarative known-profile instruments and the actual-byte output
gate. Units are bounded extensible identities. No physical COM test was performed.

## Important decisions

Runtime is the single mutable owner. Each actuator has one lease, one pending normal
proposal, one in-flight dispatch and reserved safe work. Instance/epoch/deadline
checks precede simulated send. Safe recovery needs correlated evidence, never
auto-rearms, and does not assume zero is universally safe. Query is pure; explicit
output commands advance the watchdog even if producer validation later fails.

M3 must recheck authority at actual byte initiation, not reuse an M2 Dispatch as a
transport permit. An ambiguous real write is unresolved until recovery establishes
the boundary; no blind retry or safe claim from timeout. Detailed choices and test
gates are in MILESTONE_3_DESIGN.md, not duplicated here.

The direct user instruction requires M3 design/acceptance before this handoff and
overrides IMPLEMENTATION_ROADMAP_M2_M6.md section 16's prohibition on M3 design.
The supplied roadmap is preserved unchanged; its M5 questions end at `Answ`.
Do not infer missing later model-switch instructions.

## Domain/public API changes

New lab_core::output actuator/profile/state/owner/opaque lease/proposal types,
instance-scoped dispatch IDs, evidence snapshots, commands/results/errors.
Command::Output and Query::Output integrate with Runtime. Generic configuration
still cannot actuate. Complete is trusted simulation input, not an external API for
clients to fabricate hardware evidence. No new packages or external dependencies.

## Important files

- docs/implementation/MILESTONE_2_REPORT.md — full evidence and limitations.
- docs/implementation/MILESTONE_2_DESIGN.md — implemented authority contract.
- crates/lab-core/src/output.rs and output/authority.rs — API and state machine.
- crates/lab-core/src/runtime.rs — ownership, watchdog and command/query integration.
- crates/lab-core/tests/milestone2.rs — 14 integration tests; authority.rs has 2 unit tests.
- docs/implementation/MILESTONE_3_DESIGN.md — fixed design and 20 acceptance gates.
- NEXT_IMPLEMENTATION_PLAN.md and IMPLEMENTATION_ROADMAP_M2_M6.md — sequence/model gates.

## Verification

All exit 0: fmt --check; debug/release workspace tests (38 passed each, none failed
or ignored); clippy --workspace --all-targets -- -D warnings; cargo doc --workspace
--no-deps (no warnings); cargo run -p lab-runtime (finite M1 demo);
cargo tree --workspace --edges all (two packages, no third-party dependencies);
git diff --check. Exact commands and test mapping are in the M2 report.

Donor D:\rust\com_port_reader remained read-only and clean at
50d3d1e3de84c650e1aa0ffbf1625044f794d315. No physical COM testing was performed.

## Commits

96e9582 design/plan; 3707809 English M1 rustdoc; 5545c55 red M2 tests;
9abdd4f implementation; 37ecdce regression tests/fix; 8f79150 final-epoch defenses;
5fa25b2 M2 report. This file, supplied roadmap and M3 contract form the following
documentation-only handoff commit (inspect git log for its hash).

## Known limitations

No autonomous clock, controllers, transport, Lua, Babashka service, recorder or GUI.
An unfinished simulated operation stays unresolved. No physical safety/long-soak
claim. Trip is a notification, not a persistent hardware interlock. Instance IDs
are process-local, not durable authentication. The host still runs only the M1 demo.

## Review request

None currently. This stop is the required model switch, not an unresolved architecture
fork. If implementation reveals a contract conflict, record it here and use
WAITING_FOR_REVIEW before changing the safety boundary.

## Model handoff

Current model: ASTRA_HIGH (Astra High).
Next model: SOL_HIGH (Sol High).
Reason: M2 and M3 design/acceptance are complete; implementation belongs to Sol.
Resume: M3 implementation, beginning with baseline verification and red acceptance tests.
Do not continue M3 with the current model or cross later handoffs automatically.
After Sol completes M3 and M4, the supplied roadmap requires another STOP and switch
to Astra High for M5 design only, not immediate Lua implementation.

## Reviewer response

No external response received.

## M3 phase update

M3 verification is complete; see docs/implementation/MILESTONE_3_REPORT.md. Continue
directly under SOL_HIGH with M4 design, tests first and M4 implementation. This is
not a model handoff. The final handoff section below remains the prior record until
M4 replaces it.

## Next step

User manually switches Astra High -> Sol High. Then read this file, AGENTS.md, current
project/plans and MILESTONE_3_DESIGN.md; verify the 38-test baseline and implement M3
sequentially with tests first. Current Astra phase is finished and must stop here.

External review: M2 APPROVED.

Mandatory M3 correction:
- engineering units must become extensible;
- OutputAuthority must not hard-code Unit::Percent;
- expected actuator unit must come from trusted descriptor/binding metadata;
- data-driven instruments must be able to introduce units such as sccm, mA, rpm, mbar without modifying lab-core;
- no dimensional analysis or automatic unit conversion in M3.

Detailed Sol instructions:
SOL_M3_M4_INSTRUCTIONS.md

STATUS: APPROVED_FOR_SOL_HIGH_M3_M4
