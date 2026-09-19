# lab-runtime

`lab-runtime` is a long-running, headless Rust laboratory automation Runtime with
periodic acquisition, native control, central output safety, durable SQLite recording
and a bounded local Application API.

## Status

```text
M8–M11: ACCEPTED
Developer-preview technical gate: PASSED
Core technical implementation for v0.1: FUNCTIONALLY COMPLETE
Current phase: Developer Preview Preparation
```

The core is ready for developer-preview preparation. “Functionally complete” means
the accepted v0.1 Runtime behavior is implemented and no known preview-blocking
correctness, safety or durability defect remains. It is not production certification
or exhaustive physical qualification.

## Product boundary

```text
Runtime owns experiment semantics.
Clients own presentation semantics.
```

The Runtime owns authoritative experiment state, acquisition, controller lifecycle,
output authority and recording. Clients use the local Application API and own all
workspace, plotting and other presentation behavior.

The v0.1 core includes:

- physical and virtual instruments with periodic acquisition;
- current measurements, bounded recent history and subscriptions;
- Reference, native PID/controller lifecycle and central `OutputAuthority`;
- Recorder/SQLite durable history, provenance, gaps and sealing;
- validated deployment configuration and explicit reconnect;
- compile-time native managed components;
- bounded diagnostic logging;
- a bounded TCP/NDJSON Application API.

It does not include a GUI, Presentation API, scripting runtime, first-party client
SDK or remote-network security model.

## Safety and evidence boundary

Physical output keeps request, authorization, send start, ACK, readback and physical
effect distinct. Ambiguous started writes are not blindly retried, and reconnect does
not automatically rearm a controller.

The accepted M9D evidence proves the tested Metakon command/ACK/readback sequence and
clean shutdown with the heater/load disconnected. It does not prove physical heater
effect or generic hardware safety.

## Recorder and diagnostics

Recorder/SQLite is durable scientific and experiment-audit history. Diagnostic logs
are separate bounded, lossy troubleshooting information.

```text
Application API != Recorder contract != SQLite schema
```

## Repository layout

```text
crates/lab-core        domain, safety and authoritative Runtime state
apps/lab-runtime       process orchestration, adapters, API and Recorder
examples               example deployment data and accepted evidence artifacts
ai                     active coordination and selected internal engineering history
```

Developer-facing architecture, API, archive and extension references are the next
preview-preparation deliverables. They are not yet the final release documentation.
