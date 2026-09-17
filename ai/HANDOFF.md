# AI handoff

## Current state

```text
STATUS: READY_FOR_HARDWARE_RERUN
Current phase: M8 final read-only hardware acceptance
```

M7 Recorder/SQLite is externally accepted at `f3ff456`.

M8 software-side implementation and subsequent reconnect/recovery/Recorder corrections
are complete.

The latest reconnect production correction is `a55acb1`.

The most recently reported full repository HEAD before the planned documentation
cleanup was:

```text
282d046b808aa4092358a48bc980242509492f4a
```

After documentation cleanup, use the actual clean HEAD rather than resetting to this
historical checkpoint.

## Verified software evidence

Latest reported software verification:

- 436 named debug workspace tests;
- all 136 `lab-core` tests;
- focused reconnect/Windows COM/configured-physical/Recorder/shutdown suites;
- actual Babashka 1.13.220 A/B: all three process tests;
- `cargo fmt --all -- --check` passed;
- workspace all-target Clippy with warnings denied passed;
- `git diff --check` passed.

Release-profile tests and warning-denied rustdoc are intentionally reserved for the
successful final hardware gate.

No real COM port was opened during the latest software correction.

## Latest reconnect correction

The 2026-09-17 physical failure oracle proved clean old-worker retirement and one
replacement worker spawn, but the first actual Windows open returned typed
`SerialError::Disconnected`. The old implementation treated that first completed
attempt as terminal even though the unchanged 2000 ms `open_timeout_ms` deadline had
time remaining.

The corrected path now:

- retains the accepted current-time and persistent/coalesced retirement corrections;
- keeps all attempts inside one candidate worker and one public lifecycle operation;
- retries only typed `Disconnected` at 100 ms cadence, with a fixed 64-attempt cap;
- uses one original absolute `open_timeout_ms` deadline which retries never extend;
- treats `InvalidSettings`, `Timeout` and `Other` as terminal;
- requires actual Windows open + configured-settings readback Ready before Core rebind;
- retains bounded diagnostics with attempt count and last typed error;
- uses stable public failure codes;
- preserves Required Recorder and generation fencing.

The successful deterministic chain proves:

```text
old Offline
-> clean retirement
-> candidate actual-open Ready
-> exactly one Core rebind to generation 2
-> Good channel_type 3
-> durable lifecycle
-> only then ordinary acquisition release
```

The remaining task is to demonstrate this same chain on real Windows COM + Metakon.

## Next hardware evidence target

Next archive:

```text
examples/metakon-513-com5-transient-open-retry-history.sqlite
```

At the software checkpoint, main/WAL/SHM were absent.

Expected hashes:

```text
runtime.toml
39715f3d70391154935f3ab6b25a1e78162f2b711f038238496f8853dc729c09

instrument definition
b631a78a13b9126c430c50732ac1fb3f0739c3e7da7664ef1591e4ce178c65eb
```

Approved hardware launch command after explicit operator authorization:

```powershell
cargo run -p lab-runtime -- --serve --config .\examples\runtime.metakon-513-com5.toml
```

## Hardware deployment

Frozen read-only bench:

```text
COM5
9600
8N1
no flow control
Metakon address 5
channel 0
thermocouple input
scale 1.0
actuator/heater load physically disconnected
```

No physical writes.
No alternate register/address probing.
No automatic substitute-port selection.

The operator may power the Metakon OFF/ON while leaving USB-RS485 connected. Record
those boundaries as operator-reported physical events, not Rust-observed evidence.

## Next step

Execute only `ai/WORK.md`: final real M8 hardware acceptance and final M8 verification.

Do not start M9 until external M8 acceptance.

Detailed chronological evidence remains in:

```text
docs/implementation/MILESTONE_8_REPORT.md
```
