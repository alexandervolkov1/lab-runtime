# AI Handoff

This is the single persistent external-review communication file. Read
`ai/HANDOFF.md`, `ai/ROADMAP.md`, `ai/WORK.md`, `AGENTS.md` and `PROJECT_BRIEF.md`
before each later phase. Detailed acceptance contracts and implementation
evidence belong in normal project documentation.

## Current state

```text
STATUS: READY_FOR_EXTERNAL_REVIEW
Completed model: SOL_HIGH
Completed work: M6 implementation and H1–H28 acceptance verification
Requested next step: external review of M6 source, tests and report
Authorized implementation now: none
Do not begin: M7 Recorder design or implementation
```

The user explicitly crossed ASTRA_HIGH -> SOL_HIGH and authorized M6 only.
SOL_HIGH completed [the fixed M6 design/H1–H28 contract](../docs/implementation/MILESTONE_6_DESIGN.md)
and wrote [the M6 implementation report](../docs/implementation/MILESTONE_6_REPORT.md).
This status requests review; it does **not** claim external approval of M6 or
authorize another milestone. No unresolved architectural contradiction was found.

## Previously approved implementation baseline

The external reviewer approved corrected M4 and completed M5 at checkpoint
`cb98d55`. M1 is the domain foundation and finite virtual executable; M2 central
OutputAuthority and arbitration; M3 bounded transport, narrow Metakon and
data-driven definitions; M4 thermal plant/native EMA/Fixed/Ramp/PID with corrected
Warming and finite private native renewal; M5 bounded Lua model observations and
moving-mean Transform. The M4 corrections passed 92 tests before Lua
dependencies, and the reviewed M5 baseline passed 123 workspace tests.

Accepted contracts and evidence remain in the
[M4 lifecycle review](../docs/implementation/MILESTONE_4_LIFECYCLE_REVIEW.md),
[M4 report](../docs/implementation/MILESTONE_4_REPORT.md),
[M5 design](../docs/implementation/MILESTONE_5_DESIGN.md) and
[M5 report](../docs/implementation/MILESTONE_5_REPORT.md). The reviewer assessed
supplied source/snapshot and recorded verification; it did not independently run
Cargo on this workstation. The incoming reviewer comments and full Astra mandate
are preserved in commit `a8c68ff`.

## M6 implementation checkpoint for review

M6 has one mutable Runtime owner and explicit 10-ms safety, 100-ms
plant/Reference/native-controller, and 200-ms Lua Source opportunities. Lower
priority work is bounded and cannot hold the owner behind Lua or network I/O.
Two fixed real Lua workers remain isolated. The explicit service profile starts
safe Ready on IPv4 loopback, publishes a bounded port-0 readiness line, and
accepts strict version-one UTF-8 NDJSON. It excludes remote direct output,
manual producer, client-supplied clock and client-supplied physical evidence.

Core gained continuous Ramp retune and safe Ready/Paused PID configuration with
atomic revisions and pure snapshots. Server-issued process scopes/consecutive
request numbers retain accepted/terminal outcomes and expose explicit unknown
after eviction/restart. Semantic events, frozen snapshot pages, subscription
replay/filter progress, gap and resync preserve the owner publication order.
Peer queues, frames, sessions, histories, worker slots and shutdown grace are
bounded. Ctrl-C only sets an atomic stop flag for the Rust owner; the stop
barrier quiesces work, records safe/cleanup evidence before network closure,
and reports unconfirmed output or unfinished workers as failure.

The final workspace lists **189 named Rust tests**. Debug and release
`cargo test --workspace`, `cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets -- -D warnings`, finite
`cargo run -p lab-runtime`, `bb --config clients/babashka/bb.edn run test-client`,
standalone actual-process `babashka_reconnect` and `git diff --check` passed.
The actual built Rust host launched real Babashka A/B: A configured/started and
reached Running, Rust killed A without Pause, and the host ran 6,138 ms with no
ordinary client (>3 complete 2-s leases). B resumed the same boot/scope,
reconciled outcome/replay, observed plant temperature 20.0749 -> 30.6670 °C and
a 6,305-ms finite lease-expiry advance with unchanged authority
instance/owner/epoch, then paused with Rust-confirmed virtual safe readback and
completed clean shutdown. `rustc`/`cargo` are 1.95.0; `bb` is 1.13.220.

The report maps every H1–H28 requirement to named passing tests, records
tests-first red/fix evidence, hard limits, shutdown/reconnect observations and
remaining scope. The M6 proof remains virtual/software. ACK is not readback;
no physical output acceptance, real COM adapter, Recorder/SQLite, GUI, long soak
or full product parity was added. Lua acceptance deadlines fence late results
but do not physically kill a stalled foreign instruction. The separate v1 donor
remains read-only and is not a workspace dependency.

## Review request and gate

Please review [the M6 report](../docs/implementation/MILESTONE_6_REPORT.md),
[fixed design](../docs/implementation/MILESTONE_6_DESIGN.md), implementation
source and named acceptance tests against H1–H28 and the previously accepted
M1–M5 contracts. Record the response in this file. If a genuine architectural
contradiction remains, state the precise fork and set `WAITING_FOR_REVIEW`;
SOL_HIGH has stopped. A later user-authorized model/phase instruction is required
before any M7 design or implementation.

## Workspace preservation and later release work

The user-staged `ai/project_snapshot.txt` pre-existed this phase and remains
untouched. Every M6 commit used scoped paths and excluded that file. After a
reviewed M6 gate, later release work may include M7 durable Recorder/SQLite,
runtime/release hardening, real Windows serial/COM, safe read-only Metakon smoke,
Markdown consolidation, final tutorial and packaging. Listing that sequence is
not permission to begin it now.
