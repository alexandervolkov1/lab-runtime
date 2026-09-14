# AI Handoff

## Current milestone

Milestones 3 and 4 are complete.

Current model: SOL_HIGH (Sol High).
Status: WAITING_FOR_MODEL_SWITCH.
Verified completion checkpoint: `d604981`; this handoff is the following
documentation-only commit. The working tree must be clean before continuation.

## Completed

M3 delivered extensible bounded unit identities, strict pure Metakon framing/scaling,
known-profile data definitions, bounded single-owner fake byte transport, recovery and
the actual-first-byte authority recheck. M4 delivered the deterministic thermal plant,
native uneven-dt EMA, Fixed/Ramp References, actual-dt PID, controller lifecycle,
freshness failure policy and a closed loop whose every heater effect passes the existing
OutputAuthority. M1 and M2 behavior remains green.

Full evidence and limitations are in:

- `docs/implementation/MILESTONE_3_REPORT.md`
- `docs/implementation/MILESTONE_4_DESIGN.md`
- `docs/implementation/MILESTONE_4_REPORT.md`

No M5 design or implementation was started.

## Important decisions

- Runtime remains the single mutable owner. Queries clone bounded snapshots and do not
  advance time, devices, References, controllers or watchdogs.
- Engineering Unit is an extensible fixed-capacity identity/symbol value. OutputAuthority
  trusts the descriptor unit; it has no Percent-specific branch and performs no conversion.
- Physical output queues hold private intents. Authority, instance, epoch, deadline, unit,
  descriptor limits, binding generation and mapping revision are rechecked in the same
  serialized operation as the first byte attempt. Ambiguous writes are never blindly retried.
- PID and EMA are mathematical kernels with no transport/output access. Runtime constructs
  OutputProposal, and OutputAuthority alone creates Dispatch.
- Pause revokes and settles the virtual safe action. Resume resets EMA/PID and acquires a
  new lease epoch. Stale/unavailable input or excessive tick gap enters Failed, Trips output,
  completes safe evidence and remains FaultLatched rather than rearming.
- The M4 immediate simulated dispatcher is accepted only for the M4 thermal plant. Metakon
  remains asynchronous behind M3 transport; M4 does not fabricate physical readback.

## Domain and public API changes

- `Unit` supports bounded custom identities and symbols shared by built-ins/data definitions.
- `metakon`, `transport` and generic Metakon instrument APIs implement M3's deterministic
  codec/resource/binding boundary.
- `Ema`, `FixedReference`, `RampReference` and `Pid` expose validated native algorithms and
  bounded diagnostics.
- `ReferenceId`, `ReferenceConfig`, `ReferenceSnapshot`, `ControllerId`,
  `NativeControllerConfig`, `ControllerState` and `ControllerSnapshot` define M4 state.
- Runtime Commands register/prepare/start/tick/pause/resume controllers and explicitly inject
  the virtual unavailable attempt; Queries inspect controllers/References without mutation.

## Important files

- `crates/lab-core/src/runtime.rs` — sole owner and M3/M4 orchestration.
- `crates/lab-core/src/output.rs`, `output/authority.rs` — unchanged central safety authority.
- `crates/lab-core/src/metakon.rs`, `transport.rs`, `instrument.rs` — M3 protocol/resource model.
- `crates/lab-core/src/plant.rs` — private-actuation first-order M4 fixture.
- `crates/lab-core/src/processing.rs`, `reference.rs`, `control.rs` — native M4 algorithms/state.
- `crates/lab-core/tests/milestone3_*.rs` and host `tests/definitions.rs` — M3 acceptance.
- `crates/lab-core/tests/milestone4_algorithms.rs` and `milestone4_runtime.rs` — M4 acceptance.

## Verification

All exited 0 on Windows with rustc/cargo 1.95.0:

- `cargo fmt --all -- --check`
- `cargo test --workspace`: 72 passed, none failed or ignored.
- `cargo test --workspace --release`: 72 passed, none failed or ignored.
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo doc --workspace --no-deps`: no warnings.
- `cargo run -p lab-runtime`: finite M1 demo, normal exit.
- `cargo tree --workspace --edges all`; `git diff --check`.

Core remains std-only. The host uses serde/serde_json only for bounded strict JSON data
definitions. `D:\rust\com_port_reader` remained read-only and clean at
`50d3d1e3de84c650e1aa0ffbf1625044f794d315`. No physical COM/actuator or long-soak test ran.

## Commits

M3:

- `b50a8a8` phase/reviewer unit correction; `bb629fc` red unit/codec tests;
  `cafe5bf` units/codec; `05c734c` red transport tests; `fcb1a31` executor/recovery;
  `47faf35` red descriptor/unit tests; `b5d0f90` red byte-boundary tests;
  `9115de6` authority/Metakon integration; `9480a22` strict host JSON;
  `5f3e61d` sample/recovery evidence; `efb8dd0` M3 report.

M4:

- `9416c49` design; `a68426f` red algorithm tests; `bb5d5b3` algorithms;
  `53133ed` formatted acceptance; `0d33c14` red lifecycle tests;
  `b27072c` plant/controller runtime; `1721d07` watchdog-safe edge fix;
  `18e4f7a` regression test; `d604981` M4 report/status.

## Known limitations

The transport and plant are deterministic test implementations, not production serial or
physical safety evidence. Runtime has no autonomous scheduler/clock. Native control is a
single numeric EMA/PID loop; no graph editor, cascades, live bumpless retuning, persistence,
interlock or long-soak guarantee exists. The executable remains the finite M1 demo. M5 Lua,
M6 Babashka/IPC, recorder and GUI are absent.

## Review request

None unresolved in M3/M4. The required stop is a trust-boundary model switch. Astra High
must design and review M5 only before any Lua implementation is authorized.

## Model handoff

Current model: SOL_HIGH.
Next model: ASTRA_HIGH.

Reason: M3 transport/protocol and M4 native control pipeline are complete. The next
milestone introduces the embedded Lua code-execution, sandboxing, budget, failure-isolation
and safety trust boundary.

Resume from: M5 design only.

Do not start: M5 implementation.

## Reviewer response

External review previously approved M2 and required the M3 extensible-unit correction.
That correction is implemented and verified. No later external reviewer response has been
received in this phase.

## Next step

The user manually switches Sol High -> Astra High. Astra High reads this file first, then
the current project/plans and produces/reviews only the M5 design and acceptance contract.
Do not begin Lua implementation until the next explicit authorization/model handoff.
