# Current work — M8 final hardware rerun pending

STATUS: READY_FOR_HARDWARE_RERUN

Current model: SOL_HIGH.
Current phase: M8 software corrections complete; final hardware rerun pending.
M7: externally accepted at `f3ff456`.
M8 software implementation: complete at `0a42d73`; reviewed reconnect
preparation correction complete at implementation HEAD `6f641e2`.
M9: not authorized.

## Authoritative contract

Read before acting:

```text
ai/HANDOFF.md
ai/ROADMAP.md
ai/WORK.md
AGENTS.md
PROJECT_BRIEF.md
docs/implementation/RELEASE_PLAN_M7_TO_V0_1.md
docs/implementation/MILESTONE_8_DESIGN.md
docs/implementation/MILESTONE_8_REPORT.md
```

## Completed software correction

The Recorder-corrected physical run is preserved unchanged at:

```text
examples/metakon-513-com5-recorder-corrected-history.sqlite
SHA-256 49c21ab48a22b4c2d0100357686f1b367d0432ce641dd9c8e21eee3184e4c6de
```

Deterministic source-level reproduction proved that its reconnect stopped at
`retire_old_begin`: Host supplied cached monotonic time older than Core's latest
transport poll, so Core returned `Transport(InvalidTime)` before retirement,
replacement spawn, actual Windows open or rebind. The correction supplies the
current owner time explicitly.

The mailbox lost-stop hypothesis is separately confirmed as a latent adapter
defect, not the physical root cause. Retirement now has persistent coalesced
intent independent of the fallible ordinary mailbox. Candidate preparation
waits for actual Windows open and configured-settings readback Ready before Core
rebind. Bounded stage diagnostics and stable public transport failure mapping
are implemented. Generation remains old at every pre-rebind failure; Recorder
health and finite shutdown behavior are covered by named regressions.

Software gates pass: focused Windows COM, configured physical, reconnect
lifecycle/public error, COM/Recorder shutdown, Recorder Required/FIFO, all Core
transport/Metakon tests, actual Babashka A/B, complete debug workspace, fmt,
warning-denied workspace all-target Clippy and diff check. Release-profile and
warning-denied rustdoc remain reserved for the successful final hardware gate.

## Next authorized work

Only after explicit user authorization and device access, run the final real M8
read-only hardware rerun. Do not start M9. Do not issue physical writes, retry
automatically, enumerate alternative ports, probe other registers or reuse any
existing SQLite evidence archive.

The next archive main/WAL/SHM paths are confirmed absent:

```text
examples/metakon-513-com5-prepared-reconnect-history.sqlite
```

Approved hashes:

```text
runtime TOML  8688bf121b27a6ffc88a73eb35fc23def9c0787330eaf2168899198c41f5186c
definition    b631a78a13b9126c430c50732ac1fb3f0739c3e7da7664ef1591e4ce178c65eb
```

Launch command:

```powershell
cargo run -p lab-runtime -- --serve --config .\examples\runtime.metakon-513-com5.toml
```

COM5 was not opened during the software correction.
