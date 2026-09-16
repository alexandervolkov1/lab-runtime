Product roadmap

ai/HANDOFF.md contains the current review/model gate.

ai/WORK.md contains only the currently authorized model task.

The persistent detailed path to the first stable release is:

docs/implementation/RELEASE_PLAN_M7_TO_V0_1.md
Completed
M1  domain foundation
M2  central OutputAuthority
M3  bounded transport + narrow Metakon + data-driven definitions
M4  native control pipeline + corrected Warming/renewal
M5  bounded disposable Lua components
M6  autonomous headless Runtime + local API + real Babashka process
M7  Runtime-owned Recorder/SQLite + durable provenance + bounded raw history

M6 was verified at the supplied checkpoint with 189 named Rust tests plus real Babashka A/B acceptance and has now passed the external release-planning review.

Current gate

M7 Recorder/SQLite design and the tests-first D1–D18 acceptance contract are
complete in docs/implementation/MILESTONE_7_DESIGN.md.

M7 is externally accepted at f3ff456, including D1-D18 and the recorded 345-test
debug/release gates. Its reported limitations remain accepted limitations.

STATUS: WAITING_FOR_REVIEW
Current model: SOL_HIGH
Current phase: Recorder-corrected physical reconnect failed before rebind.
M8 software implementation and the 388-test debug/release completion gates are
complete at `0a42d73`; details are in `MILESTONE_8_REPORT.md`.
Do not start M9.

An actual COM5 bench passed compatibility, strict frame decoding, Required
recording and clean shutdown, but exposed a deployment-specific decimal scale
error. The reviewed tests-first corrections are complete at `867f985`: the
actual thermocouple profile is explicit, relative deployment paths are stable,
and the first SQLite archive is preserved. Corrected real 27/28 °C readings agree
with the contemporaneous 28 °C front panel and are durably recorded with Good
quality and zero output events. Actual disconnect then left one transaction and
COM worker recovering beyond the configured timeout, without an Unavailable
Signal; finite shutdown could not confirm transport close. The reviewed narrow
correction is complete at `71d4e56`: recovery is finite, admission/fencing and
Unavailable publication are owner-enforced, and shutdown starts retirement from
every executor state. The next real run proved that correction, then exposed a
failed reconnect whose installed replacement incorrectly resumed acquisition.
The reviewed resource-scoped reconnect gate is complete at `0ae7b3c`; target
ordinary reads now remain quiesced until compatibility and lifecycle success,
while failed replacements retain their advanced generation and are retired. A
407-entry debug workspace gate passes. The corrected rerun proved finite
disconnect and stopped ordinary acquisition after a failed reconnect, but the
reconnect still returned `invalid_configuration` and Required Recorder failed
with `fact record reservation mismatch`. The resulting unsealed evidence archive
is preserved at the hash recorded in `MILESTONE_8_REPORT.md`. The reviewed
Recorder correction is complete at `fdf8a73`: lifecycle work reserves capacity
without assigning a future FIFO identity, compatibility facts remain contiguous,
and activation takes the current tail identity only when immediately enqueued.
Cancellation creates no rewind or gap; Required failure remains fail-closed. The
debug workspace now passes 414 named tests plus actual Babashka A/B, fmt, Clippy
and diff gates. A new absent archive is selected at `2ec104b`. Await explicit
continuation of the read-only hardware gate. That run reconfirmed real Good
acquisition, one durable Unavailable, finite Offline and healthy contiguous
Recorder operation. Its single explicit reconnect nevertheless failed before a
generation-2 binding or probe was installed. Clean shutdown sealed complete
evidence with zero outputs. The archive hash and exact facts are in
`MILESTONE_8_REPORT.md`. External review is required before another COM5 open or
software correction; M8 is incomplete and M9 is not authorized.

First stable release sequence
M7
    Runtime-owned Recorder
    SQLite
    durable provenance
    bounded history query

M8
    runtime.toml deployment configuration
    validation/staging/reload
    managed script reload
    virtual model/emulator restart
    Windows COM adapter
    real Metakon read-only smoke

M9
    optional persistent Lua workspace/fallback scripts
    presentation/workspace model
    Babashka presentation operations

M10
    separate Rust egui/eframe GUI client
    reuse/adapt successful v1 plot behavior
    series sidebar
    logs
    control panels
    minimal reload controls

M11 / release candidate
    integration/fault hardening
    repository cleanup
    test-file naming cleanup
    final architecture/user/developer documentation
    tutorials
    Windows packaging
    SHA256
    release review

v0.1.0

Each milestone requires:

ASTRA_HIGH design
    ↓
manual model switch
    ↓
SOL_HIGH tests-first implementation
    ↓
external review

No model crosses a gate automatically.

Release character

v0.1.0 is a usable and extensible laboratory Runtime, not full historical
com_port_reader feature parity.

Full On/Off/Furnace/filter parity, richer recipes, Arduino and other extensions
continue after release in an educational explain -> code -> test -> commit workflow.

Babashka is an optional external client and bb.exe is not a Runtime dependency.

Lua has two distinct roles:

managed disposable bounded components
+
small optional persistent local workspace for fallback experiment scripting

Neither role owns physical safety.

The GUI is a separate client and must work through the same public Runtime API.
