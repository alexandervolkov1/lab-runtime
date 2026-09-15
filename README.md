# lab-runtime

Milestones 1–5 are implemented and externally reviewed: a synchronous Rust owner with typed descriptors,
stable local identity, deterministic virtual measurements, pure queries and bounded
recent signal state, bounded output authority, strict Metakon framing and a
single-owner fault-injected byte executor. M4 adds a deterministic thermal plant and
native EMA/Reference/PID control lifecycle, corrected multi-sample Warming and finite
native lease renewal. M5 adds bounded Lua model observations and transforms on two
isolated worker slots. The reviewed checkpoint has **123 workspace tests**.

The [M6 design and H1–H28 acceptance contract](docs/implementation/MILESTONE_6_DESIGN.md)
are complete for the autonomous headless host, local API and real Babashka slice.
Implementation awaits the explicit **ASTRA_HIGH -> SOL_HIGH** user switch.
Follow [ai/WORK.md](ai/WORK.md) and [ai/HANDOFF.md](ai/HANDOFF.md) for the active gate.

## Run and verify

The workspace uses Rust edition 2024; verified with rustc/cargo 1.95.0 on Windows.
There are three workspace packages:

- `lab-core`: std-only domain state, algorithms, transport and output authority.
- `lab-lua`: bounded disposable Lua VMs and fixed workers; `mlua 0.11.6`, Lua 5.4,
  vendored, with default features disabled.
- `lab-runtime`: executable composition, strict data-definition loading and finite
  integration demonstrations.

```powershell
cargo run -p lab-runtime
cargo test --workspace
cargo test --workspace --release
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

The executable prints generic introspection and five explicit virtual measurements,
then exits. A three-sample window demonstrates oldest-first eviction. It is still
the finite M1 executable; autonomous scheduling and the local API are M6 work.
The finite M5 integration test can be run with
`cargo test -p lab-runtime --test milestone5_demo`.

## Contract and scope

- All mutations use local Commands; Queries return owned snapshots without refresh.
- Display names are not identity. Parameter IDs are scoped to an InstrumentId;
  a SignalId is their typed pair. Duplicate display names are allowed.
- Samples use caller-supplied elapsed monotonic time. Normal publication is strictly
  increasing; managed failure/reload may invalidate Good at the same time. Derived
  samples retain original observation freshness. Initial latest is unknown and a
  failed observation stores Unavailable without a value.
- Invalid configuration is atomic. Configuration preserves previous observations
  until the next refresh. MeasurementUnavailable is a typed error **after** storing
  the failed observation, unlike validation errors.
- History capacity is 1..=4096 per instrument, with at most 64 instruments.
- Generic parameter configuration rejects actuation. Explicit output commands use
  a safe profile, exclusive manual/automatic lease, epoch and deadline checks at
  simulated send. Requested, sent, ACK and readback are separate observations.
- Output time is explicit and nondecreasing. Commands tick the watchdog before
  producer validation; even rejected producer work may expire an existing lease.
- Native controllers have Created/Ready/Warming/Running/Paused/Failed lifecycle. Warming
  accumulates distinct fresh inputs without a lease or PID output. They consume
  fresh typed samples, evaluate an independent Reference, and can affect the M4 plant
  only by proposing through the same lease/epoch/final-dispatch authority as clients.
- Pause and controller failure revoke authority and complete the virtual safe action;
  resume resets EMA/PID memory and obtains a new lease epoch.
- Healthy native delivery renews a finite lease through a private Runtime path;
  old tokens become stale. Manual/client authority receives no native renewal privilege.
- Lua can publish bounded model/filter observations, with fresh VMs, resource budgets,
  independent acceptance deadlines and generation fencing. It has no output authority,
  raw transport or physical-evidence capability. Explicit Rust `ServiceSafety` remains
  independent of Lua progress and still requires a host caller.

Babashka/API, Recorder, GUI and real Windows COM deployment are not implemented.
No physical hardware acceptance was performed. M2–M5 are exercised through tests;
the virtual proof does not establish physical safety or multi-day reliability.
The later release sequence is recorded in [ai/ROADMAP.md](ai/ROADMAP.md).

See [M1 design](docs/implementation/MILESTONE_1_DESIGN.md),
[completion report](docs/implementation/MILESTONE_1_REPORT.md),
[M2 design](docs/implementation/MILESTONE_2_DESIGN.md),
[M2 report](docs/implementation/MILESTONE_2_REPORT.md),
[M3 contract](docs/implementation/MILESTONE_3_DESIGN.md),
[M3 report](docs/implementation/MILESTONE_3_REPORT.md),
[M4 contract](docs/implementation/MILESTONE_4_DESIGN.md),
[M4 report](docs/implementation/MILESTONE_4_REPORT.md),
[M4 lifecycle corrections](docs/implementation/MILESTONE_4_LIFECYCLE_REVIEW.md),
[M5 contract](docs/implementation/MILESTONE_5_DESIGN.md),
[M5 report](docs/implementation/MILESTONE_5_REPORT.md),
[target architecture](docs/architecture/HIGH_LEVEL_ARCHITECTURE.md),
[migration analysis](docs/migration/V1_TO_LAB_RUNTIME_MAP.md), and
[donor baseline](docs/migration/DONOR_BASELINE.md).
