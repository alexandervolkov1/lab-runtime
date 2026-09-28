# Roadmap after the developer preview

## Current state

```text
M8–M11: ACCEPTED
Developer-preview technical gate: PASSED
Core technical implementation for v0.1: FUNCTIONALLY COMPLETE
v0.1.0-preview.1: PUBLISHED
M12.1 WebSocket architecture audit: ACCEPTED
M12.2 transport-neutral server seam: ACCEPTED
M12.3 bounded WebSocket/JSON transport: ACCEPTED
M12.4 transport parity/fault acceptance: ACCEPTED
M12.5 browser/ClojureScript smoke acceptance: READY FOR EXTERNAL REVIEW
M13: NOT AUTHORIZED
Current phase: M12.5 external review
```

The accepted M12.2 implementation commit is
`0e1bcb228f068eb1fcec6116eebc64f3352e520d`.
The accepted M12.3 implementation commit is
`6fb867389426fa75033f54312fda8c5556c52ed7`.
The accepted M12.4 implementation commit is
`9b58e92cffa79087f60d78032bc34b89499ae961`.

The accepted headless Runtime core is implemented and technically hardened for a
developer preview. Remaining work concerns reference material, practical integration
validation and release preparation, not an unplanned feature milestone.

## Developer Preview Preparation

The compact developer-facing reference is complete:

1. architecture and concepts guide;
2. Application API reference;
3. Recorder/SQLite archive reference;
4. safety/failure/recovery cheat sheet;
5. configuration and instrument/component extension notes;
6. getting-started/build/run guide;

Preview packaging/build is complete. The checksummed Windows x86_64 artifact is
ready locally, and `v0.1.0-preview.1` is published as a GitHub pre-release. Its tag
remains fixed at the packaged source commit; later coordination commits are not
release inputs.

This is not yet the final polished release-documentation pass. Each execution slice
requires explicit authorization in `WORK.md`.

## Practical learning and integration

After preview preparation, the planned exercise is:

```text
Arduino thermal plant
→ Rust physical instrument integration
→ experiment-specific Clojure client
→ Clay live browser view
→ thermal experiments
→ sealed SQLite archives
→ Clojure/Clay system-identification notebook
```

The Arduino remains an external instrument/emulator and is not required to accept
M11 retroactively. Runtime continues to own experiment semantics; the Clojure/Clay
client owns presentation semantics.

The published preview contains TCP/NDJSON. The accepted post-preview M12.3 commit on
`main` additionally contains the optional loopback WebSocket/JSON implementation.

## M12 — WebSocket transport

M12 adds WebSocket/JSON as a second bounded loopback transport for the same
Application API:

```text
TCP / NDJSON ------+
                   +-> one shared delivery boundary -> ONE Application
WebSocket / JSON --+
```

- **M12.1:** read-only architecture audit — accepted;
- **M12.2:** transport-neutral server seam, with unchanged TCP behavior — accepted;
- **M12.3:** bounded local WebSocket/JSON transport and browser Origin policy —
  accepted;
- **M12.4:** parity, reconnect, backpressure and fault acceptance — accepted;
- **M12.5:** browser/ClojureScript smoke acceptance — ready for external review;
- **M12 external review.**

M12.1 through M12.3 are accepted. See
`M12_1_WEBSOCKET_ARCHITECTURE_AUDIT.md` and
`M12_2_TRANSPORT_NEUTRAL_SERVER_SEAM.md`; M12.3 evidence is in
`M12_3_BOUNDED_WEBSOCKET_TRANSPORT.md`. M12.4 evidence is in
`M12_4_TRANSPORT_PARITY_FAULT_ACCEPTANCE.md`. M12.5 evidence is in
`M12_5_BROWSER_CLOJURESCRIPT_SMOKE.md` and is ready for external review.

## M13 — external Steel scripting host

Steel is a future external Application client. It must not execute inside Runtime
ownership, acquire transport/output authority, or make script lifetime equal
experiment lifetime. Native real-time components remain native Rust.

## M14 — GUI/Workbench/presentation schema

GUI and any future transport/language-neutral `PresentationDocument` are
client-owned. Runtime must not acquire window, tab, row, column, plot, panel,
layout, widget, slider, button, egui, or other presentation semantics.

## Final release gate

The later final documentation/package audit will require polished architecture,
protocol, archive, configuration, safety/recovery and operational documentation plus
a reproducible checksummed Windows package. It is not authorized by the current
phase transition.

## Explicit non-goals

- no GUI or Presentation API;
- no bundled scripting runtime or first-party client SDK;
- no dynamic plugin framework;
- no new convenience API merely for documentation;
- no schema or accepted semantic changes;
- no claim of production certification, hard real-time or exhaustive physical
  qualification.
