# Current work — M8 final real-hardware acceptance

```text
STATUS: READY_FOR_HARDWARE_RERUN
Current model: SOL_HIGH
M9: NOT AUTHORIZED
```

This is the only detailed currently authorized implementation task.

## Read first

```text
AGENTS.md
PROJECT_BRIEF.md
ai/HANDOFF.md
ai/ROADMAP.md
ai/WORK.md
docs/implementation/RELEASE_PLAN_TO_V0_1.md
docs/implementation/MILESTONE_8_DESIGN.md
docs/implementation/MILESTONE_8_REPORT.md
```

## Software baseline

Latest reported M8 software evidence:

- reconnect production correction `6f641e2`;
- 427 named debug workspace tests;
- all 136 Core tests;
- focused reconnect/COM/Recorder/configured-physical suites;
- actual Babashka A/B three process tests;
- fmt pass;
- warning-denied all-target Clippy pass;
- diff check pass.

Release-profile tests and warning-denied rustdoc remain for the successful final
hardware gate.

If production source changed after the accepted software checkpoint, stop and review
before using real hardware.

## Hardware candidate

Next evidence archive:

```text
examples/metakon-513-com5-prepared-reconnect-history.sqlite
```

Before opening COM5, main/WAL/SHM must not exist.

Expected hashes:

```text
runtime.toml
8688bf121b27a6ffc88a73eb35fc23def9c0787330eaf2168899198c41f5186c

definition
b631a78a13b9126c430c50732ac1fb3f0739c3e7da7664ef1591e4ce178c65eb
```

Approved deployment:

```text
COM5
9600
8N1
no flow control
address 5
channel 0
read-only
scale 1.0
Required Recorder
```

No output write.
No other register/address probe.
No automatic port substitution.

## Operator authorization barrier

Preflight may run without hardware.

Before the first OS COM open, stop and obtain explicit user authorization confirming:

- Metakon connected/powered;
- heater/actuator load physically disconnected;
- front-panel temperature observed;
- COM5 free;
- read-only M8 run authorized.

Only then run:

```powershell
cargo run -p lab-runtime -- --serve --config .\examples\runtime.metakon-513-com5.toml
```

## Acceptance sequence

### A. Initial acquisition

Require:

- actual COM open/configuration;
- channel type 3;
- repeated Good temperature;
- plausible agreement with operator display;
- generation 1;
- Required Recorder healthy;
- durable/public history;
- zero output events.

### B. Device power-off

Ask operator to power OFF the Metakon while leaving USB-RS485/COM5 connected.

Record as `operator-reported device power-off`.

Require:

- exactly one value-less Unavailable/Transport;
- finite Recovering -> Offline;
- generation remains 1;
- queue does not grow;
- no later Good;
- Recorder healthy;
- no automatic reconnect.

Failure -> preserve evidence, `WAITING_FOR_REVIEW`, STOP.

### C. Device power-on

Ask operator to power ON the Metakon.

Record as `operator-reported device power-cycle/reset boundary`.

Read the operator-reported display temperature.

### D. One explicit reconnect

Issue exactly one:

```text
reconnect_resource(resource=1, expected_binding_generation=<actual current generation>)
```

No automatic retry.

Prove order:

```text
old Offline/quiesced
-> old retirement begins
-> old worker proven closed
-> replacement spawn
-> actual Windows open pending
-> settings readback Ready
-> exactly one Core rebind to generation 2
-> resource-specific channel-type probe
-> Good channel_type 3
-> durable reconnect lifecycle completion
-> only then ordinary generation-2 temperature Good
```

Require no stale generation-1 result after rebind.

Require Recorder healthy/contiguous and zero output events.

### E. Babashka A/B

On the same real process:

- Babashka A observes state/history;
- disconnect/kill A;
- Runtime acquisition/Recorder continue;
- Babashka B reconnects/resyncs;
- current generation/latest/history are correct.

### F. Harmless live-safe change

Perform one already-supported metadata/live-safe configuration change through normal:

```text
validate -> stage -> apply -> durable terminal
```

Do not change physical binding, address/register, Recorder path/policy, topology,
safe profile or write capability.

Require no unnecessary rebind and no auto-rearm.

### G. Clean shutdown

Use normal public shutdown.

Require:

- transports closed;
- workers finished;
- Recorder flushed/closed;
- no unknown tail;
- exit success;
- honest safe status;
- zero physical output events.

## SQLite audit

After shutdown:

- reopen the new archive;
- verify complete coverage;
- verify Good -> Unavailable -> reconnect lifecycle -> generation-2 Good;
- verify exact provenance;
- verify zero outputs;
- verify durable shutdown;
- compute SHA-256;
- preserve older evidence archives unchanged.

## Final software gate

Only after successful hardware acceptance:

```powershell
cargo fmt --all -- --check
cargo test --workspace
cargo test --workspace --release
cargo clippy --workspace --all-targets -- -D warnings
```

Run the repository's established warning-denied rustdoc command.

Also run:

- actual Babashka acceptance;
- finite demo/smoke that does not reopen COM5 unexpectedly;
- `git diff --check`.

Record any first transient red result honestly before reruns.

## Stop conditions

Immediately stop and preserve evidence on:

- implausible initial temperature;
- unexpected write/probe;
- recovery deadline failure;
- queue growth while offline;
- more than one automatic reconnect;
- generation change before actual-open Ready;
- ordinary Good before compatibility + durable lifecycle completion;
- Recorder failure/gap/unknown tail;
- stale generation result mutating the new binding;
- incomplete shutdown.

Do not patch and rerun automatically.

## Completion

If all hardware and final software gates pass:

```text
STATUS: READY_FOR_EXTERNAL_REVIEW
```

Update M8 report and concise coordination files, then STOP.

External review is required before M9.
