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
M12.5 browser/ClojureScript smoke acceptance: ACCEPTED
M12: ACCEPTED
M13.1: ACCEPTED
M13 dependency safety resolution: ACCEPTED
M13.2 Steel:
BLOCKED ON STEEL DEPENDENCY SAFETY
NOT AUTHORIZED
M14.1 Workbench architecture audit: ACCEPTED
M14.2 bounded native Application client: ACCEPTED
M14.3 WorkbenchModel + PresentationDocument: ACCEPTED
M14.4 minimal native GUI: ACCEPTED
M14.5 operator controls + properties/config: ACCEPTED
M14.6A recovery/fault audit: ACCEPTED
M14.6B1 recovery UI + one-shot operation_status: ACCEPTED
M14.6B2A quarantine projection / restart classification: ACCEPTED
M14.6B2B1 Exact Retry core / evidence lifecycle: ACCEPTED
M14.6B2B2 Exact Retry GUI: ACCEPTED
M14.6B3 bounded fault reattach: ACCEPTED
M14.6B4 consolidated recovery/fault acceptance: ACCEPTED
M14.6 recovery/reconnect/fault acceptance: ACCEPTED
M14 consolidated acceptance: ACCEPTED
M15.1 documentation/productization audit: ACCEPTED
M15.2 README + getting started: ACCEPTED
M15.3 architecture + Application API reference: AUTHORIZED
M15.4+: NOT AUTHORIZED
Current phase: M15.3 architecture + Application API reference
STATUS: M15_3_ARCHITECTURE_API_REFERENCE_AUTHORIZED
```

The accepted M12.2 implementation commit is
`0e1bcb228f068eb1fcec6116eebc64f3352e520d`.
The accepted M12.3 implementation commit is
`6fb867389426fa75033f54312fda8c5556c52ed7`.
The accepted M12.4 implementation commit is
`9b58e92cffa79087f60d78032bc34b89499ae961`.
The accepted M12.5 implementation/evidence commit is
`f291edf35a7805245ce19ea088b9ee899d57f30e`.
The accepted M12 consolidated review-ready commit is
`bb159f674f2cca1c50f903528b268cb070858604`.
The accepted M13.1 audit commit is
`81e303de18021c49c526b12bb1f77f8ea75ae2d9`.
The accepted M13 dependency-safety resolution commit is
`7a150cd8e15d990e00ad62b5c9d9b66c401b7086`.
The accepted M14.1 architecture-audit commit is
`6340aee32b563000bc6397a52aecd85ee945da1c`.
The accepted M14.2 implementation commit is
`bdedf9455305f693a9537f698403c3bbb3840c51`.
The accepted M14.3 implementation commit is
`f01567ba2165b24b9551e3b0acf5b100b0169f32`.
The accepted M14.4 implementation commit is
`116aba631fe47ea24412dd3d4b50e9b00eafc8df`.
The accepted M14.5 implementation commit is
`ab097ed5207ea426cbbd48611015da12ce534a43`.
The accepted M14.6A audit commit is
`4e18801930939404a8e856521b86c55c20d71dc9`.
The accepted M14.6B1 implementation commit is
`1a5c69965fa2ad3907c0314cb61cc3e29798e5ea`.
The accepted M15.1 documentation/productization audit commit is
`8cd89e2b58ba11248c2ce2532c165270a9c60797`. Only M15.2 README + getting started is
accepted at `9a3cd58bc776ffe9939e7f0d97dd753769fd1b15`. Only M15.3 architecture +
Application API reference is authorized; M15.4+ is not authorized.

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
- **M12.5:** browser/ClojureScript smoke acceptance — accepted;
- **M12 external review:** accepted.

M12.1 through M12.5 are accepted. See
`M12_1_WEBSOCKET_ARCHITECTURE_AUDIT.md` and
`M12_2_TRANSPORT_NEUTRAL_SERVER_SEAM.md`; M12.3 evidence is in
`M12_3_BOUNDED_WEBSOCKET_TRANSPORT.md`. M12.4 evidence is in
`M12_4_TRANSPORT_PARITY_FAULT_ACCEPTANCE.md`. M12.5 evidence is in
`M12_5_BROWSER_CLOJURESCRIPT_SMOKE.md` and is accepted. Consolidated evidence is in
`M12_CONSOLIDATED_EXTERNAL_REVIEW.md` and is accepted.

## M13 — external Steel scripting host

Steel is a future external Application client. It must not execute inside Runtime
ownership, acquire transport/output authority, or make script lifetime equal
experiment lifetime. Native real-time components remain native Rust.

The M13.1 source/dependency audit is accepted in
`M13_1_STEEL_EXTERNAL_HOST_ARCHITECTURE_AUDIT.md`. It establishes a
one-process-per-script external host over the existing TCP/NDJSON Application
endpoint. The read-only dependency safety resolution for the rejected
`steel-core 0.8.3` graph is complete. Its report,
`M13_STEEL_DEPENDENCY_SAFETY_RESOLUTION.md`, found no acceptable current release or
upstream commit. M13.2 remains blocked and unauthorized.

The earlier separate `lab-steel` implementation sequence is deferred. Current
sequencing builds the external native Workbench client first. If a later dependency
review accepts Steel, the preferred placement is an optional subsystem inside the
Workbench, using its one Application client boundary. Steel remains neither a
Runtime subsystem nor a managed component.

## M14 — GUI/Workbench/presentation schema

GUI and any future transport/language-neutral `PresentationDocument` are
client-owned. Runtime must not acquire window, tab, row, column, plot, panel,
layout, widget, slider, button, egui, or other presentation semantics.

The read-only M14.1 architecture audit is accepted in
`M14_1_WORKBENCH_ARCHITECTURE_AUDIT.md`. The minimal private bounded native
Application client is accepted in `M14_2_MINIMAL_WORKBENCH_CLIENT.md`; the M14.3
client-owned model and persistence implementation is accepted in
`M14_3_WORKBENCH_MODEL_PRESENTATION.md`. The minimal native GUI implementation and
evidence and overflow/live-continuity remediation are in `M14_4_MINIMAL_GUI.md`,
and accepted. M14.5 operator controls and property/configuration workflows are
accepted in `M14_5_OPERATOR_CONTROLS.md` at implementation commit
`ab097ed5207ea426cbbd48611015da12ce534a43`. The recovery/fault audit in
`M14_6_RECOVERY_FAULT_AUDIT.md` is accepted at commit
`4e18801930939404a8e856521b86c55c20d71dc9`. M14.6B1 recovery status UI and
one-shot `operation_status` are accepted at
`1a5c69965fa2ad3907c0314cb61cc3e29798e5ea`. M14.6B2A quarantine projection
and restart classification are accepted at implementation commit
`f945d910cfeabbe8552634c53d95016e79a6eafa`. The accepted M14.6B2B1 implementation
commit is `21e4a6f623e39c102109703b22a5266a17f5e0e0`. The accepted M14.6B2B2
implementation commit is `eafd73adf43a8a566336bfe1a14370066fe06c5c`. The accepted M14.6B3
implementation commit is `0319b1d1a7917903c1cf0aa6e4852c149543f7fd`. The accepted M14.6B4
implementation commit is `801a4b559d82a903e9232cc08f5e7d27d714b1d5`. M14 consolidated
acceptance is granted. M15.1 is accepted at audit commit
`8cd89e2b58ba11248c2ce2532c165270a9c60797`; only M15.2 README + getting started is
accepted at `9a3cd58bc776ffe9939e7f0d97dd753769fd1b15`; only M15.3 architecture +
Application API reference is authorized.

## Final release gate

The later final documentation/package audit will require polished architecture,
protocol, archive, configuration, safety/recovery and operational documentation plus
a reproducible checksummed Windows package. M15.3 does not authorize that later gate.

## Explicit non-goals

- no GUI or Presentation API;
- no bundled scripting runtime or first-party client SDK;
- no dynamic plugin framework;
- no new convenience API merely for documentation;
- no schema or accepted semantic changes;
- no claim of production certification, hard real-time or exhaustive physical
  qualification.


## Accepted M14 consolidated invariants

Runtime owns experiment semantics and Workbench owns presentation semantics.
Workbench/GUI/future-script lifetime is not Runtime or experiment lifetime; Workbench
failure, clean close, or OS termination does not stop Runtime, controllers, Recorder,
or the experiment.

One private bounded Rust Application client owns the socket, framing, hello/session/
scope, message IDs, mutation sequence, subscription/cursor, recovery-journal
interaction, and bounded retained-scope reconnect. No second Application owner was
introduced. One WorkbenchModel and one PresentationDocument remain client-owned and
never become Runtime experiment authority.

Check Status and Exact Retry are explicit/manual; mutation, status, and Exact Retry
are never replayed automatically. Exact bounded worker-owned recovery evidence and
quarantined old-boot/scope evidence remain visible. Explicit Disconnect has no
automatic reconnect and uses the accepted out-of-band/coalesced command-admission
fence. Unexpected continuity loss gets one retained-scope episode with one absolute
3 s deadline, connect timeout no greater than `min(2 s, remaining)`, and retry
spacing of at least 10 ms.

Fresh requires the complete authoritative rebuild barrier. Hello, process liveness,
socket reconnect, an old event cursor, or mutation terminal outcome alone does not
prove Fresh.

The frozen 30-row M14.6A matrix and A-J scenarios pass. Process evidence comprises
eight real-Runtime acceptances and one self-spawn Workbench plus scripted Application
peer acceptance. Native Glow OS-termination evidence preserves Runtime, workspace,
journal, and Reference authority; controller identity `1`, state `ready`, revision
`1` are unchanged, and Recorder state `idle` with null `active_run`/`run_id` is
unchanged. Recorder evidence intentionally proves the weaker authoritative idle
continuity invariant, not active-run survival.

Accepted verification: Workbench `160 passed; 9 ignored; 0 failed` three times;
workspace debug and release each `680 passed; 11 ignored; 0 failed`; real Runtime
`8 passed; 0 failed`; self-spawn/scripted `1 passed; 0 failed`; A-J and native
Glow PASS; recorded focused fault/race repetitions 10/10; fmt, Clippy with warnings
denied, and diff-check PASS.

M13.2 remains blocked on Steel dependency safety and is not authorized. Discard/
Forget, a later milestone, and automatic release are not authorized.
