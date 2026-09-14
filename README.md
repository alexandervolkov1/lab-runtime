# lab-runtime

Milestones 1–3: a synchronous Rust owner with typed descriptors,
stable local identity, deterministic virtual measurements, pure queries and bounded
recent signal state, bounded output authority, strict Metakon framing and a
single-owner fault-injected byte executor. **No physical hardware I/O was tested.**

## Run and verify

The workspace uses Rust edition 2024; verified with rustc/cargo 1.95.0 on Windows.
There are exactly two packages and no third-party dependencies.

```powershell
cargo run -p lab-runtime
cargo test --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

The executable prints generic introspection and five explicit virtual measurements,
then exits. A three-sample window demonstrates oldest-first eviction. There is no
background service, clock polling, sleep, serial port or network listener.

## Contract and scope

- All mutations use local Commands; Queries return owned snapshots without refresh.
- Display names are not identity. Parameter IDs are scoped to an InstrumentId;
  a SignalId is their typed pair. Duplicate display names are allowed.
- Samples use caller-supplied elapsed monotonic time, strictly increasing per signal.
  Initial latest is unknown. A failed refresh stores Unavailable without a value.
- Invalid configuration is atomic. Configuration preserves previous observations
  until the next refresh. MeasurementUnavailable is a typed error **after** storing
  the failed observation, unlike validation errors.
- History capacity is 1..=4096 per instrument, with at most 64 instruments.
- Generic parameter configuration rejects actuation. Explicit output commands use
  a safe profile, exclusive manual/automatic lease, epoch and deadline checks at
  simulated send. Requested, sent, ACK and readback are separate observations.
- Output time is explicit and nondecreasing. Commands tick the watchdog before
  producer validation; even rejected producer work may expire an existing lease.

No controller, physical serial adapter, Lua, Babashka/IPC, recorder or GUI is included. The executable
remains the finite M1 demo; M2 is exercised with `cargo test -p lab-core --test milestone2`.
This is not an autonomous runtime and does not prove physical safety or multi-day
operation. M3 uses deterministic fake byte adapters; M4 is the current SOL_HIGH scope.

See [M1 design](docs/implementation/MILESTONE_1_DESIGN.md),
[completion report](docs/implementation/MILESTONE_1_REPORT.md),
[M2 design](docs/implementation/MILESTONE_2_DESIGN.md),
[M2 report](docs/implementation/MILESTONE_2_REPORT.md),
[M3 contract](docs/implementation/MILESTONE_3_DESIGN.md),
[M3 report](docs/implementation/MILESTONE_3_REPORT.md),
[target architecture](docs/architecture/HIGH_LEVEL_ARCHITECTURE.md),
[migration analysis](docs/migration/V1_TO_LAB_RUNTIME_MAP.md), and
[donor baseline](docs/migration/DONOR_BASELINE.md).
