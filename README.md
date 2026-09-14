# lab-runtime

Milestone 1 domain foundation: a synchronous Rust owner with typed descriptors,
stable local identity, deterministic virtual measurements, pure queries and bounded
recent signal state. **No hardware or output execution.**

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
- Actuator metadata is visible for introspection, but output-affecting commands are
  rejected. Access flags alone are not permission to actuate.

No OutputArbiter, controllers, serial, Lua, Babashka/IPC, recorder or GUI is included.
M1 is not a complete autonomous runtime and does not prove physical safety or
multi-day operation. Later milestones require separate authorization.

See [M1 design](docs/implementation/MILESTONE_1_DESIGN.md),
[target architecture](docs/architecture/HIGH_LEVEL_ARCHITECTURE.md),
[migration analysis](docs/migration/V1_TO_LAB_RUNTIME_MAP.md), and
[donor baseline](docs/migration/DONOR_BASELINE.md).
