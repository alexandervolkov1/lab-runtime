File: ai/HANDOFF.md
AI Handoff

Read before every phase:

ai/HANDOFF.md
ai/ROADMAP.md
ai/WORK.md
AGENTS.md
PROJECT_BRIEF.md
docs/implementation/RELEASE_PLAN_M7_TO_V0_1.md
Current state
STATUS: READY_FOR_HARDWARE_RERUN

Current model:
SOL_HIGH

Current phase:
M8 software corrections complete; final read-only hardware rerun pending.

Completed work:
M7 Recorder/SQLite architecture, implementation and D1-D18 acceptance contract,
externally accepted by the user. M8 design is frozen and its software
implementation/completion gate is complete at `0a42d73`. The reviewed
pre-rebind correction is complete at implementation HEAD `6f641e2`.

Implementation contract:
docs/implementation/MILESTONE_8_DESIGN.md (after explicit implementation handoff)

M7 production implementation is complete and audited against D1-D18. The exact
acceptance mapping, red/green history, versions, bounds, limitations and final
latest-head verification are recorded in MILESTONE_7_REPORT.md.

External review result:
M7 accepted as the implementation baseline at f3ff456. The external review found
no architectural contradiction or blocking D1-D18 defect. Accepted evidence
includes the complete M7 report and 345-test debug/release gates. The reported
hardware, physical-output, power-loss, remote-security and long-soak limitations
remain limitations, not new claims of acceptance in those areas.

Authorized work:
Only the final real M8 hardware rerun after explicit user authorization and
device access. Do not reopen COM5 before that authorization, reuse prior evidence
archives, issue physical writes, probe other registers, begin M9 or perform
post-M8 cleanup.

Model gate:
The earlier ASTRA_HIGH -> SOL_HIGH gate applied to M7 and was crossed.
The M8 ASTRA_HIGH -> SOL_HIGH implementation gate was crossed explicitly.

Do not start:
M9 or later work.

M8 software correction ready for hardware rerun — 2026-09-16

Source and deterministic regression localized the preceding physical reconnect
failure to `retire_old_begin`. Host reconnect preparation passed cached
`last_now` after the owner had already polled the transport with a later
monotonic time; Core correctly rejected the backwards timestamp as
`Transport(InvalidTime)`. The failure was before old-worker retirement,
replacement spawn, actual Windows port opening and Core rebind. The correction
passes the reconnect turn's current owner time explicitly.

The lost-stop hypothesis was independently confirmed as a latent COM adapter
defect, but rejected as the cause of that physical run. Retirement now sets a
persistent coalesced intent outside the fallible one-slot data mailbox; the Stop
request is only a wake-up hint. The worker checks the intent after a bounded OS
call and before another data operation. Only a proven finished worker permits
honest Closed when its Stopped completion is missing.

`ComTransport::open_windows()` remains spawn-only and returns Opening. New
bounded preparation diagnostics distinguish worker spawn, actual OS open
pending, typed open failure, configured-settings readback Ready, install, Core
rebind, probe and lifecycle durability. Reconnect does not cross the generation
fence until actual open/readback Ready. Failed or unfinished candidates remain
bounded, finitely retired or quarantined, and do not trigger automatic retry.

Transport retirement/spawn/open preparation now maps to stable public
`transport_unavailable`; invalid candidates remain `invalid_configuration`,
stale generation remains `revision_conflict`, and Required Recorder failure
remains `recording_unavailable`. Implementation commit is `6f641e2`. The final
debug workspace, focused reconnect/COM/Recorder/Core suites, actual Babashka A/B,
fmt, warning-denied all-target Clippy and diff checks pass; exact chronological
evidence and named regressions are in `MILESTONE_8_REPORT.md`.

The next confirmed-absent archive is
`examples/metakon-513-com5-prepared-reconnect-history.sqlite`. Runtime TOML
SHA-256 is
`8688bf121b27a6ffc88a73eb35fc23def9c0787330eaf2168899198c41f5186c`;
the unchanged definition SHA-256 is
`b631a78a13b9126c430c50732ac1fb3f0739c3e7da7664ef1591e4ce178c65eb`.
Launch only after explicit hardware authorization:

```powershell
cargo run -p lab-runtime -- --serve --config .\examples\runtime.metakon-513-com5.toml
```

COM5 was not opened during this correction. M9 remains unauthorized.

M8 corrected reconnect hardware review stop — 2026-09-16

The read-only run from HEAD `bb62808` passed startup channel type 3 and recorded
54 real generation-1 Good 28.0-degree temperature rows matching the operator's
28-degree front panel. Physical USB-RS485 removal produced exactly one durable
value-less `Unavailable/Transport` row, finite Offline, unchanged generation,
zero queue growth and no fabricated Good.

After operator reconnection, one public
`reconnect_resource(resource=1, expected_binding_generation=1)` was issued. The
client timed out before its first reply and did not retry; reconciliation of the
same request ID returned terminal `failed/invalid_configuration`. Ordinary
acquisition remained quiesced, Offline and Unavailable, so the prior escaped-Good
defect did not recur. However, Required Recorder simultaneously failed with
`fact record reservation mismatch`, `unknown_tail`, worker closed and a durable
prefix through record 472. SQLite contains reconnect Accepted but not its
terminal failure.

Normal shutdown was accepted, finished finitely and exited nonzero with
`safe shutdown incomplete`; no successful close was fabricated. The unsealed
archive is preserved unchanged at
`examples/metakon-513-com5-reconnect-corrected-history.sqlite`, SHA-256
`ef16177a913216ad4e39a9c101e7ab749994f337ece6db2525c6bc396a45d377`.
It contains 54 Good rows, one Unavailable row, exact approved TOML/definition
provenance and zero output events. Full evidence and uncompleted acceptance steps
are in `MILESTONE_8_REPORT.md`. No production code or tests changed, the final M8
gate was not run, and COM5 must not be reopened before external review.

M8 reviewed Recorder ordering correction — 2026-09-16

External review authorized a narrow SOL_HIGH correction and COM5 remained
closed. Source and the preserved archive confirmed that live lifecycle capacity
reservation assigned record N and advanced `reserved_through` before Activation
N entered the FIFO. The resource-scoped channel-type probe then entered first as
N+1, so the SQLite worker correctly rejected the hole with
`fact record reservation mismatch`. Required Recorder failure caused the public
reconnect failure and was masked as `invalid_configuration`; the compatibility
value itself was not the cause.

Commit `fdf8a73` reserves only the unchanged record/byte/group capacity and
activation generation. Intervening facts receive contiguous identities. Commit
assigns the Activation its current FIFO-tail identity only when immediately
enqueuing it, then advances `reserved_through`; cancellation releases capacity
without rewinding IDs. Exact SQLite sequence validation, one writer, 1,024-record
/ 4-MiB / four-group limits, resource-scoped reconnect quiescing and Required
fail-closed behavior remain unchanged. Recorder lifecycle failure now surfaces
through the existing `recording_unavailable` wire code.

Seven new named regressions cover worker interleaving/cancellation, successful
and failed compatibility probes, post-install Required failure and public error
mapping. Targeted suites and all 136 Core tests pass. The complete debug workspace
passes 414 named tests, actual Babashka A/B passes three tests under Babashka
1.13.220, and fmt, warning-denied workspace all-target Clippy and diff checks
pass. Full release and warning-denied rustdoc remain deferred to the successful
final hardware gate as authorized. Exact red/green history is in the report.

Commit `2ec104b` selects the confirmed-absent next archive
`examples/metakon-513-com5-recorder-corrected-history.sqlite`. Runtime TOML hash
is `405d99056fd4ba265bb8776f18360a33cd4be81e65c7e03976196acbf374056b`;
the unchanged definition hash is
`b631a78a13b9126c430c50732ac1fb3f0739c3e7da7664ef1591e4ce178c65eb`.
The defect archive remains unchanged at its recorded SHA-256. STATUS returns to
`M8_HARDWARE_ACCEPTANCE_PENDING`; await explicit continuation and do not begin
M9.

M8 Recorder-corrected physical reconnect review stop — 2026-09-16

The accepted HEAD `5355963d9e4bb3efa5f5214223a1510cc25619e9` run used exact
approved configuration hashes and a previously absent archive. COM5 startup and
channel type 3 succeeded. Strict raw I16 29/30 with explicit scale 1.0 produced
165 durable generation-1 Good rows matching the operator's 29-degree display.

The operator-reported device power-off produced exactly one durable value-less
Unavailable/Transport row, finite Offline, unchanged generation 1, zero queue
growth and no later Good. Required Recorder stayed healthy and complete. After
the operator-reported device power-cycle/reset boundary, exactly one explicit
reconnect request was accepted at record 677 and failed at record 678 with
`invalid_configuration`.

The Recorder FIFO correction held: there was no reservation mismatch,
recording_unavailable, gap or unknown tail. No generation-2 binding, measurement
or lifecycle activation was installed; resource state remained Offline at
generation 1. Evidence therefore localizes the failure before the replacement
crossed its rebind fence, but existing diagnostics cannot distinguish incomplete
old-adapter retirement from configured COM5 reopen failure. No alternative open,
probe, address, register, reset or reconnect retry was attempted.

Normal shutdown through a resumed bounded client scope was completely clean:
transports and Recorder closed, unfinished transport/worker counts were zero and
exit succeeded. The sealed complete SQLite archive is preserved at
`examples/metakon-513-com5-recorder-corrected-history.sqlite`, SHA-256
`49c21ab48a22b4c2d0100357686f1b367d0432ce641dd9c8e21eee3184e4c6de`,
with zero output events. Full evidence is in `MILESTONE_8_REPORT.md`. No
production/test code changed and final M8 gates were not run. STATUS is
`WAITING_FOR_REVIEW`; do not reopen COM5 or begin M9.

M8 design completion — 2026-09-16

Recovered clean HEAD f3ff456, containing 53880ca and all later accepted M7 work.
Read the permanent coordination files, project brief, release plan and relevant
accepted M3-M7 contracts/source. The design preserves Runtime ownership, Core's
OS/storage independence and accepted safety/evidence/Required semantics.

The new MILESTONE_8_DESIGN.md fixes strict declarative loading, exact loaded TOML
identity, staged diff/atomic commit, irreversible safe-barrier failure semantics,
distinct configuration/script/model operations, fencing, fixed Recorder ingress
budgets, read-only worker-backed Windows COM, conservative recovery and C1-C20.
Recorder bounds are not increased. Fixed-topology live reload rejects topology,
listener and Recorder policy/path changes as restart-required.

Actual Windows/Metakon read-only C16/C17/C19 evidence remains mandatory. Fake
transport cannot replace it. Hardware, port/wiring/firmware identity and a
justified clean-response boundary were unavailable, so M8 remains
`M8_HARDWARE_ACCEPTANCE_PENDING`; software completion does not authorize M9.

M8 software implementation completion — 2026-09-16

Recovered the accepted M7 baseline and Astra design checkpoint, then implemented
the C1-C20 software contract in small logical commits. The implementation adds a
strict bounded `runtime.toml` loader and immutable source bundle, staged classified
diffs, safe/fenced activation, distinct configuration/script/model/reconnect
operations, atomic managed generation changes, exact durable lifecycle provenance,
and a worker-owned read-only Windows COM adapter over the existing M3 transport
model. No M7 Recorder bound was enlarged and no COM/SQLite/TOML/OS dependency was
added to `lab-core`.

The completed report is `docs/implementation/MILESTONE_8_REPORT.md`. Final
software verification on implementation HEAD `0a42d73` passed 388 named Rust
tests in both debug and release, including the actual M6/M7/M8 Babashka process
scenarios and SQLite reopen/crash coverage. Formatting, warning-denied all-target
clippy, warning-denied rustdoc, finite demo, `bb test-client` (8 tests, 13
assertions) and `git diff --check` also passed. No default test was skipped.

No real Metakon was connected, no operator-confirmed COM port or recovery boundary
was provided, and no real port or physical operation was attempted. C16, C17 and
the hardware subgate of C19 therefore remain open, along with the bench portions
of C12/C14/C18. Deterministic transports are only software evidence. No actuator
write or physical-output safety claim was made. The initially supplied donor
path was unavailable; the actual path was inspected later as recorded below.

M8 real Metakon review stop — 2026-09-16

The operator connected an actual Metakon 513 on COM5, address 5, thermocouple
input, 9600 8N1/no flow control, with the actuator load disconnected and display
approximately 21 °C. Exact config, definition and SQLite hashes are recorded in
`MILESTONE_8_REPORT.md`. No actuator/configuration write, alternate address or
register, reset or experimental operation was attempted.

The strict register-0 channel-type probe returned the required U8 value 3 and the
service reached ready. Register-1 responses passed exact address/function/type/
length/CRC checks and produced raw I16 values 23/24. With the accepted definition
scale 0.1 they became twelve durable `Good` observations of 2.3–2.4 °C, which is
not plausible against the approximately 21 °C display. Required SQLite recording
has complete coverage, no gaps and zero output events. Actual COM plus Recorder
shutdown completed cleanly. The physical disconnect/reconnect test was not begun.

The first user-approved relative config launch also exposed a pre-COM defect:
the relative config parent left the Recorder path relative and startup rejected
it as non-absolute. Launching the same unchanged bytes with an absolute config
path succeeded. Neither this defect nor the value discrepancy was hidden.

The adapter does not retain independent raw-frame logs. Strict-decoder frame
reconstructions, public snapshots, SQLite rows and shutdown results are in the
report. Review authoritative Metakon 513 documentation or existing read-only
donor evidence before authorizing any mapping change. Do not guess a scale or
probe another register. At that review stop, STATUS was `WAITING_FOR_REVIEW`;
the reviewed correction checkpoint below supersedes it.

Documentation only: no production code, tests or dependencies changed; no COM
opened; no accepted M7 test gate rerun or new hardware evidence claimed. The
initially supplied donor path was unavailable on this computer. The later actual
donor verification below supersedes that path finding. No unrelated cleanup or
ai/project_snapshot.txt changes were made.

M8 reviewed correction checkpoint — 2026-09-16

External review approved a small SOL_HIGH correction phase. The strict decoder
and raw signed-I16 semantics remain unchanged. Tests-first correction commit
`27080b2` gives the actual Metakon 513 thermocouple deployment an explicit
whole-degree scale while retaining the old 0.1 behavior under a clearly named
fixture-only profile. Commit `d330020` resolves relative deployment-owned paths
from the absolute runtime.toml parent while preserving exact loaded bytes and
parent-traversal rejection. Commit `867f985` directs the corrected Required run
to a fresh SQLite file, preserving the original bench archive unchanged.

The actual donor at `D:\rust_projects\com_port_reader` was subsequently inspected
read-only: clean HEAD `8226aecafaecbb243d21d0a10e8653a7564ad484`, describe
`v0.1.0-33-g8226aec`. Its codec decodes register 1 as raw little-endian I16 and
applies an explicit caller-selected engineering scale; register 0 only validates
channel type 3. Historical tracked COM5 configuration includes address 5 with
`scale = 1.0`, and ignored machine-local execution logs record device 5
measurements 24/26/27/28 with scale 1. This independently confirms the M8
whole-degree profile; no further software change is required. Exact symbols,
tests and provenance are in `MILESTONE_8_REPORT.md`. No donor file was modified.

Final correction gates pass with 391 named tests in debug and release, fmt,
all-target warning-denied Clippy, warning-denied rustdoc and diff check. The
report records one transient old Recorder timing-test failure and its passing
isolated, binary and complete-workspace reruns. COM5 was not reopened.

The corrected TOML hash is
`fd47daf2cac9f8200a6b1ce5d212b9d872572bb006b16c045816577e712bd815`;
the actual profile hash is
`b631a78a13b9126c430c50732ac1fb3f0739c3e7da7664ef1591e4ce178c65eb`.
STATUS is `M8_HARDWARE_ACCEPTANCE_PENDING`. Await explicit user continuation,
then use the existing read-only command and stop again after corrected readings
before physical disconnect. Do not begin M9.

M8 corrected physical acquisition checkpoint — 2026-09-16

The explicitly authorized corrected launch opened only COM5 with the frozen
9600/8N1/no-flow, address 5/channel 0 read-only deployment. Strict channel type 3
readiness passed. Consecutive register-1 results decoded to raw 28 and engineering
28.0 °C, followed by raw/engineering 27/28; all public Signal observations were
`Good`. The operator contemporaneously reported 28 °C on the front panel.

Required database `769be53d8fb9cd4dcbb49bf70880cdbe`, run 1/interval 1, has
complete coverage and a committed prefix through at least record 647. Public
history and an immutable SQLite checkpoint contain real `Good` rows; durable
provenance matches both corrected hashes and `output_events` is zero. Logical
resource 1, resource/binding/mapping generations 1/1/1 remain stable. The old
SQLite archive retains its original hash. No write, reset, alternative register,
configuration operation or COM enumeration occurred.

At that checkpoint, the corrected Runtime was intentionally left running and its
separate SQLite run was unsealed while the explicit physical-disconnect cue was
awaited. Do not begin M9.

M8 actual disconnect review stop — 2026-09-16

After the operator disconnected the device/cable, resource 1 entered
`recovering` with active transaction 498. For more than 25 seconds—well beyond
the configured 2-second recovery timeout—it never became Offline, the Signal
never became Unavailable, and transaction 498 remained active. The last Good
sample retained its old timestamp and no later Good sample was fabricated. The
bounded queue reached its fixed capacity 32 and stopped growing.

No reconnect or other physical operation was attempted. Bounded shutdown
returned `cleanup_incomplete`: Recorder flushed and safe confirmation held, but
one transport remained unfinished and not closed; the process exited with code
1. Offline SQLite is sealed complete with no gaps, 381 Good temperature rows,
zero Unavailable rows and zero output events. Its boot seal records the resource
still recovering with transaction 498 active. Corrected database SHA-256 is
`d726efe8f562656361c65c970cdb6b78823963861e507026a12c4c3c408593c2`;
the original archive hash remains unchanged.

This is a genuine C14/C16/C17/C19 Windows COM recovery/shutdown contradiction,
not a scaling or protocol-frame discrepancy. STATUS is `WAITING_FOR_REVIEW`.
Do not reconnect, reopen COM5 or begin M9.

M8 reviewed recovery correction checkpoint — 2026-09-16

External review authorized a narrow SOL_HIGH implementation correction. Commit
`71d4e56` routes the validated per-resource recovery timeout into the existing
Core executor, gives transaction and protocol recovery finite monotonic deadlines,
fences failed-generation queued work, rejects ordinary admission while unavailable,
publishes one immediate `Unavailable` read fact, and initiates nonblocking adapter
retirement from Active/Recovering/ProtocolRecovering. Failed recovery does not
advance generation or become Complete. Existing output ambiguity and no-join
semantics remain intact.

The new causal regressions cover deadline-to-Offline, unchanged generation,
terminal active correlation, owner-boundary admission rejection, zero periodic
queue growth, durable Good-to-Unavailable recording, finite protocol recovery,
all shutdown entry states, cooperative/stuck COM retirement and explicit rebind
with stale-session fencing. Targeted suites, all 136 Core tests, focused 26 host
tests, warning-denied targeted Clippy, formatting and diff checks pass. The full
debug workspace gate passes 402 named tests including actual Babashka A/B process
acceptance. Full release/rustdoc are deferred to the corrected hardware gate as
authorized. Exact red/green sequence is in `MILESTONE_8_REPORT.md`.

Commit `2672a0a` changes only the next bench database pathname to
`examples/metakon-513-com5-recovery-corrected-history.sqlite`; it does not exist
yet. New TOML SHA-256 is
`5fec1c546f81b8a29f7a21360deb33575fe98a87c987fbbd1a28094bd8e01def`;
the unchanged definition hash is
`b631a78a13b9126c430c50732ac1fb3f0739c3e7da7664ef1591e4ce178c65eb`.
Both prior SQLite archives retain their recorded hashes and remain untracked.
COM5 has not been reopened. STATUS returns to `M8_HARDWARE_ACCEPTANCE_PENDING`;
await explicit operator continuation and do not begin M9.

M8 reviewed reconnect correction checkpoint — 2026-09-16

The corrected physical disconnect run reached Offline on deadline and durably
recorded exactly one value-less Unavailable row. After operator reconnection,
however, `reconnect_resource(resource=1, expected_binding_generation=1)` was
durably accepted then failed with `invalid_configuration`, while its installed
generation-2 replacement continued ordinary acquisition and recorded new Good
26/27 °C rows. Normal shutdown was completely clean. That sealed archive is
preserved unchanged at
`examples/metakon-513-com5-recovery-corrected-history.sqlite`, SHA-256
`6afe7924e64326a0dedc37b3d860428a4a351c8f35803c56d51ba271f8caa0e4`,
with complete coverage, exact provenance and zero output events.

External review authorized a narrow implementation correction. Commit `0ae7b3c`
adds a bounded resource-scoped reconnect gate, target-only probe operations and
finite failed-replacement retirement. Ordinary reads for the target remain
quiesced from before old-session retirement until a distinct Good channel-type 3
probe and durable lifecycle completion. Unrelated resources and Recorder/safety
progress continue. Probe failure preserves the advanced generation, keeps the
Signal Unavailable, retires the replacement and requires another explicit
reconnect using the actual current generation. Successful release resets the
periodic deadline without catch-up. No codec, scale, COM setting, queue or
Recorder bound changed.

The first three acceptance cases were red before production because the scoped
gate/probe/activation APIs did not exist. The multi-resource and probe-shutdown
cases were added after the minimal Host implementation; the report records this
honestly. All 12 configured-physical tests and focused Core/COM/Recorder/lifecycle
suites pass. The full debug workspace passes and lists 407 named test entries;
workspace all-target warning-denied Clippy, fmt and diff checks pass. Release and
warning-denied rustdoc remain deferred to the final hardware gate.

Commit `2a03358` selects the new absent archive
`examples/metakon-513-com5-reconnect-corrected-history.sqlite`. Runtime TOML
SHA-256 is
`200acfe4a2cd36b4375213db9369518d751e0ad43c8401f0b456cb0a03ef6b7c`;
the definition remains
`b631a78a13b9126c430c50732ac1fb3f0739c3e7da7664ef1591e4ce178c65eb`.
COM5 has not been reopened after the correction. STATUS remains
`M8_HARDWARE_ACCEPTANCE_PENDING`; await explicit operator continuation and do not
begin M9.

Design validation passed: exactly 20 distinct C1-C20 matrix rows, resolving local
design links, balanced Markdown fences, no trailing whitespace in all five
changed documents, and git diff --check. Changes are documentation-only and left
uncommitted; the accepted implementation HEAD remains f3ff456.

MODEL HANDOFF

STOP HERE.

Switch:
ASTRA_HIGH -> SOL_HIGH

Resume with:
M8 tests-first implementation against MILESTONE_8_DESIGN.md.

Historical design handoff only; the implementation gate was subsequently crossed
and the software work above completed. Do not start M9.

M7 design checkpoint — 2026-09-15

Inspected baseline: edb436a (main), initially clean working tree.
Baseline verification completed successfully:

git status
git log --oneline -20
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings

cargo test --workspace -- --list confirmed 189 named Rust tests.
The workspace suite included actual Babashka A/B process acceptance.
Release-profile verification was not repeated in this documentation phase.

The design fixes Runtime ownership, bounded Core fact capture and storage ingress,
separate boot/run/interval/activation identities, immutable provenance, monotonic
and wall-clock mapping, typed output evidence, BestEffort/Required policies,
Required revoke/fault/safe behavior, WAL/FULL single-connection SQLite, versioned
schema/reopen, explicit bounded raw-history operations and finite shutdown/flush.

The history service implements the release plan's bounded paged raw-observation
option; downsampling is not advertised. M6 transient replay remains separate.

Donor inspected read-only at 50d3d1e3de84c650e1aa0ffbf1625044f794d315.
No donor tests were run or donor files changed. The original design checkpoint
added no production code, tests or dependencies. A later continuation produced
the uncommitted M7 implementation now under SOL_HIGH audit; its test sequence
must be reported honestly. No M8 design, GUI or Lua expansion was added.
ai/project_snapshot.txt was absent and was not created or modified.
No commits were made at that design checkpoint. The user subsequently authorized
small logical commits during SOL_HIGH work.

Documentation validation: all 18 distinct D1–D18 acceptance rows are present,
local design links resolve, Markdown fences balance, no trailing whitespace,
and git diff --check passes. Only MILESTONE_7_DESIGN.md and the four authorized
coordination/instruction files changed.

No unresolved architectural contradiction was found at the design checkpoint.
That checkpoint requested a model handoff, not external acceptance. SOL_HIGH must map D1–D18
to actual red/green evidence in MILESTONE_7_REPORT.md, run the completion gates,
then set READY_FOR_EXTERNAL_REVIEW here and STOP before M8.

M6 review conclusion

The supplied M6 checkpoint is accepted.

The reviewed evidence reports:

189 named Rust workspace tests

debug and release workspace tests passing

fmt/clippy checks passing

real Rust host

real Babashka A/B process acceptance

Babashka A killed without Pause

native PID remained autonomous for more than three finite lease lifetimes

Babashka B reconciled the same Runtime process

safe virtual pause and shutdown completed

This external review is based on the supplied repository snapshot/source/report
and recorded verification evidence. The reviewer did not independently execute
Cargo or hardware commands on the user's workstation.

Preserve all accepted M1–M6 contracts.

Release direction approved

The detailed release roadmap is:

docs/implementation/RELEASE_PLAN_M7_TO_V0_1.md

The approved product direction includes:

M7 Recorder + SQLite

M8 declarative runtime configuration, safe reload, real Windows COM and
   conservative physical Metakon read-only smoke

M9 persistent optional Lua fallback workspace plus server-owned presentation model

M10 separate egui GUI client using the public API

M11 release hardening, cleanup, documentation, tutorials and packaging
Product clarifications from the user
Babashka

Babashka is an optional external orchestration client.

Do not bundle or require bb.exe for Runtime operation.

Keep the thin client/examples in the repository/release examples.

Lua

The previous migration recommendation that all persistent embedded application
REPL behavior should simply become Babashka is superseded by the user's explicit
product requirement.

The final product should retain:

M5 disposable bounded Lua VMs
    for model/filter/component execution

plus

a separate small persistent Lua workspace
    for local fallback experiment scripting

The fallback requirement is:

With only lab-runtime, configuration/scripts and instruments, a user should be
able to automate a useful experiment without installing Babashka or GUI.

Do not recreate the entire v1 global Lua application API.

The persistent workspace remains outside the Runtime owner lane and may only use
validated Runtime application/domain operations.

No raw COM, OutputLease, OutputAuthority or physical-evidence bypass.

A GUI Lua REPL/editor is optional after the first release or as a later small
addition. Preserve an easy seam for a separate Tools -> Lua Workspace window.

Configuration

The application must not depend on executable Lua configuration.

Use a declarative deployment configuration, preferably runtime.toml.

Parsing configuration must be side-effect free.

Reload uses validate/stage/safe/apply semantics.

Invalid candidates leave the committed configuration intact.

Support explicit operations for:

Reload configuration
Reload managed scripts
Restart virtual emulator/model

reload != rearm.

Presentation and GUI

The GUI is a separate client.

Server-owned semantic presentation state must be configurable both from GUI and
Babashka.

Support:

workspaces
plot panels
traces
trace labels
colors
visibility
panel assignment/order
time window
follow/live
axis policy
control panels
logs panel

The user explicitly considers the old v1 plot/series UX good.

Before implementing M10, inspect the donor read-only and preserve/adapt where
appropriate:

plot rendering
egui_plot interaction
time-axis labels
pan/zoom
follow/live
Y auto/manual behavior
legend/multiple series
series colors/visibility
plot sizing
downsampling
series sidebar
series labels/panel assignment

Do not preserve old shared-state ownership.

Global GUI buttons should remain minimal.

Prominent infrastructure actions:

Reload configuration
Reload scripts
Restart emulator/model

Normal controller/reference/recording actions belong in contextual control panels.

Documentation and learning

The first stable release is not complete without clear architecture and developer
documentation plus reproducible tutorials.

After v0.1.0 the default development mode should become a smaller educational
workflow rather than large autonomous agent milestones.

Current model gate
APPROVED_FOR_ASTRA_HIGH_M8_DESIGN

Current model: ASTRA_HIGH. M7 is externally accepted. M8 design is complete;
implementation is not yet authorized. Await the explicit handoff above.

Historical M7 implementation pause checkpoint — 2026-09-16

The user requested an interruption after finishing the current logical commit
and recording completed and unfinished work. The latest Recorder acceptance
commit is 53880ca: an actual blocked-writer server process exits unsuccessfully
after its finite flush watchdog; a fresh SQLite reopen reports the old run's
unknown tail. Its named debug/release tests, formatting and focused clippy pass.

The design, incoming production audit and substantial Runtime-owned Recorder,
Required/BestEffort, SQLite and bounded-history coverage are complete or in
place. D1-D17 still have edge acceptance cases listed in ai/WORK.md and the
in-progress MILESTONE_7_REPORT.md. The final D18 latest-head verification and
version/limit inventory were not complete. At that pause the status remained
M7_IMPLEMENTATION_IN_PROGRESS; it was not an external-review request or a model
handoff. The completion record below supersedes that historical state.

M7 implementation completion — 2026-09-16

The pause checkpoint was recovered exactly, including 53880ca, and all recorded
remaining D1-D17 cases were closed. The final report maps D1-D18 to named tests
and records the honest sequence where production preceded coverage or a fixture
expectation was corrected.

Final verification on the latest source HEAD passed:

- 345 named Rust tests in debug and optimized release;
- formatting, warning-free clippy and rustdoc with warnings denied;
- finite demo;
- actual M6 and recording-enabled M7 Babashka process tests;
- `bb test-client`: 8 tests, 13 assertions;
- real recording A-kill/B-reconnect/Pause/Stop/Shutdown/SQLite-reopen history;
- `git diff --check` and a clean tree before these handoff documentation edits.

Resolved versions include Rust/Cargo 1.95.0, Babashka 1.13.220, rusqlite 0.40.2,
libsqlite3-sys 0.38.2, bundled SQLite 3.53.2 and sha2 0.11.0. No donor files,
M8 work or unrelated cleanup were performed. No architectural contradiction was
found. At that checkpoint external review was required before later work; the
M7 acceptance and M8 design authorization above now supersede that old gate.
