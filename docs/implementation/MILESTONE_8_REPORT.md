# M8 implementation report

Status: `M8_HARDWARE_ACCEPTANCE_PENDING`; the reviewed software corrections are
complete under SOL_HIGH and COM5 has not been reopened, 2026-09-16.

Design authority: [MILESTONE_8_DESIGN.md](MILESTONE_8_DESIGN.md). M7 was
externally accepted at `f3ff456`; the design-only checkpoint was committed as
`443fa4e`. This report records actual implementation evidence. Planned tests
are not reported as passing evidence.

## Acceptance implementation map

| ID | Principal named evidence | Status |
| --- | --- | --- |
| C1 | `configuration_validation`, `configuration_startup`, `demo` | Software pass |
| C2 | `configuration_validation`, `configuration_startup`, `configured_physical`, `managed_script_reload` | Software pass |
| C3 | `configuration_validation`, `definitions` | Software pass |
| C4 | `configuration_validation`, `configuration_startup` | Software pass |
| C5 | `configuration_provenance`, `configuration_reload`, `runtime_lifecycle_operations` | Software pass |
| C6 | `runtime_lifecycle_operations`, `configured_physical` | Software pass |
| C7 | `configuration_reload`, `recorder_failure`, `recorder_transactions`, `recorder_process_reopen` | Software pass |
| C8 | `runtime_lifecycle_operations`, `request_deduplication`, `recorder_required` | Software pass |
| C9 | `managed_script_reload` | Software pass |
| C10 | `managed_script_reload`, M5 `runner`/`workers` regressions | Software pass |
| C11 | `model_restart`, `managed_script_reload`, `runtime_lifecycle_operations` | Software pass |
| C12 | `windows_com_transport`, `configured_physical` | Software pass; actual COM5 open/settings/close observed |
| C13 | `windows_com_transport`, accepted M3 transport suites | Software pass |
| C14 | `windows_com_transport`, `configured_physical` | Software pass; real disconnect pending |
| C15 | `configured_physical`, `windows_com_transport` | Software pass |
| C16 | Actual Windows COM + Metakon read-only bench | Correction software pass; corrected physical rerun pending |
| C17 | Actual observations, public history and SQLite reopen | First durable evidence retained separately; corrected physical evidence pending |
| C18 | `babashka_reconnect`, `host_scheduler`, configured acquisition suites | Software pass; corrected hardware continuation pending |
| C19 | `com_recorder_shutdown`, `recorder_shutdown`, `runtime_shutdown` | Actual clean COM/Recorder close passed; physical disconnect/reconnect subcase not run |
| C20 | `recorder_reload_budget`, Recorder bounds/fault/time/process suites and final gates | Software pass |

## Actual sequence

1. Recovered clean accepted implementation HEAD `f3ff456`; `53880ca` is an
   ancestor. The only incoming changes were the expected uncommitted Astra design
   and coordination documents. `git diff --check` passed.
2. Read all required coordination, project, release-plan and M8-design files.
   The initially supplied donor path `D:\rust\com_port_reader` was unavailable;
   the actual machine-local path was provided and inspected read-only during the
   later correction review recorded below. Committed the documentation-only
   design checkpoint as `443fa4e`.
3. C1-C4 first acceptance test compile was red because
   `lab_runtime::configuration` did not exist. An unrelated fixture byte-string
   encoding error was corrected and red was repeated, leaving only the missing
   API. Added the host-only parser and exact-byte bundle with `toml` 1.1.6,
   structural checks before artifact reads, strict unknown/duplicate rejection,
   cross-reference limits, read-only definition safety validation and fixed
   aggregate source limits. Targeted run: four tests green. The first green C4
   fixture accidentally made the diagnostic parameter invalid while constructing
   an output definition; it was corrected to a valid output-only M3 definition,
   after which the intended read-only rejection and duplicate COM tests passed.
   Targeted clippy passed. The initial fmt check reported formatting diffs only;
   formatting was then applied before commit.
4. C5-C8 acceptance compile was red because the deployment lifecycle module did
   not exist. Added one active/one-candidate bounded lifecycle, checked revision
   and candidate counters, a 30-second monotonic expiry, explicit union diff and
   restart-required rejection before side effects. Targeted tests prove a
   live-safe commit, barrier-before-commit ordering, and that barrier failure
   retains old configuration without rearming the deliberately revoked probe
   authority. Three targeted tests pass. This slice is not yet claimed as the
   final asynchronous Host/Recorder integration.
5. C12-C15 acceptance compile was red because no host serial module existed.
   Added pinned `serialport` 4.10.1 with default features disabled and a fixed
   one-request/one-completion worker outside Core. The first green attempt exposed
   an acceptance-fixture assumption that an asynchronous open was immediately
   Online: three tests expected positive admission rather than allowed `Ok(0)`.
   The tests were corrected to poll boundedly, preserving the nonblocking owner
   contract. Four tests then passed for settings, partial same-frame continuation,
   no timeout write retry, disconnect recovery proof, oversize rejection and
   binding-generation replacement. Concrete open reads settings back and rejects
   driver substitution. This is software evidence only, not C16/C17/C19 hardware.
6. C1/C2 configured-startup acceptance compile was red because
   `ServiceHost::loaded_configuration` and configuration-driven composition did
   not exist. Added the exclusive `--serve --config <path>` form, completed all
   parsing and artifact freezing before entropy/listener/Recorder side effects,
   constructed the explicit native observation graph, retained exact loaded
   provenance, and bound the listener only after a safe/disarmed owner existed.
   Both targeted tests pass. This slice intentionally does not yet claim managed
   component, controller, or physical-resource startup composition.
7. C11 native-model restart acceptance was red because Core had no restart
   command or generation result. Added a checked thermal-model replacement that
   preserves the logical instrument ID, rejects stale generation and active
   dependent control/output work, advances generation exactly once, clears old
   signal currency and records later samples with the new generation. Two
   targeted Core tests and warning-free targeted clippy pass. Public operation
   composition and managed-model restart remain separate work.
8. C12-C17 configured physical-composition acceptance was red because Host had
   no transport-injected deployment builder. Added exact frozen-definition
   composition into the existing M3 `ResourceExecutor`, scheduled only the
   trusted read-only temperature primitive and published its decoded value
   through the normal Signal/fact path with resource/binding correlation. The
   first green run exposed exact binary floating-point comparison in the test;
   the oracle was corrected to a tight numeric tolerance. Two software tests,
   configured startup regression and warning-free targeted clippy pass. The
   production service now creates concrete COM workers before listener binding.
   This is deterministic software evidence only; no actual COM port or Metakon
   was opened and C16/C17 hardware remain pending.
9. C5/C8/C11 lifecycle-operation acceptance was red because Service exposed no
   distinct reload/restart API. Added live-safe configuration reload from the
   original path into one frozen candidate, checked staging/revision apply,
   display/cadence commit without catch-up or graph replacement, and native model
   restart without rereading TOML or managed sources. Added three separate,
   deduplicated wire operations. Three targeted tests pass, including retained
   configuration revision across model restart and no stale current observation.
   Managed reload currently reports an explicit failure when components exist;
   C9/C10 implementation remains. Safe-barrier and transport-rebind diffs are not
   falsely treated as live apply.
10. The configuration-provenance acceptance test was added after the frozen-byte
    production path already existed in the configured-startup slice, so no red
    result is claimed. The process test starts with SQLite enabled, mutates the
    TOML pathname after readiness, shuts down cleanly, reopens the database and
    verifies the `runtime_toml` provenance content is byte-for-byte the originally
    loaded candidate. The targeted test passed.
11. C9 configured managed-source acceptance was red at startup because the graph
    builder rejected managed components. Added startup composition from frozen Lua
    bytes, using the existing two-worker supervisor and one Core staging slot,
    plus a distinct source-only reload that leaves TOML hash/revision unchanged.
    The first green attempt exposed that an old committed generation made the
    wait predicate immediately true; it was corrected to wait for a checked
    generation advance. C9 single-component reload and C10 invalid-source
    preservation pass, as do targeted lifecycle regression and clippy. The
    current sequential path does not yet satisfy C9's atomic multi-component
    commit and is not reported as full C9 completion.
12. The first full debug workspace regression reached the new managed tests after
    all earlier suites passed, then failed because the two independent tests ran
    concurrently while each correctly requested the process-wide fixed two Lua
    worker slots. The test module now serializes only its ownership of that shared
    production capacity (no test ordering or shared domain state); startup still
    uses its finite Busy retry. The full regression must be rerun before a gate is
    claimed.
13. C9 multi-component acceptance was red because the sequential reload committed
    generation 2 for the first source before invalid Lua in the second source was
    known. Core now retains up to the existing eight-component bound of validated
    init results outside the active graph while preserving the accepted single
    staging slot and two Lua workers. One checked owner command validates every
    identity/generation again and commits the complete batch; failure discards all
    prepared candidates. Old-generation steps may progress during preparation,
    so their state revision is refreshed only at the fenced commit while the
    component generation remains the stale-work boundary. The three managed
    tests now prove invalid two-source atomicity, subsequent all-source generation
    advance, single-source reload and invalid-source preservation. Full Core tests
    and warning-free targeted clippy pass.
14. C19 transport shutdown acceptance was red because `ByteTransport` had no
    explicit retirement progress contract and Host could not distinguish a closed
    adapter from a detached worker. Added a nonblocking Pending/Complete shutdown
    step to the existing M3 ownership boundary, implemented COM retire/drain/join
    only after `is_finished`, and made Host close resources only after output-safe
    obligations resolve. Shutdown status now reports COM cleanup independently
    from managed workers and Recorder flush. Software tests cover clean and
    indefinitely stalled transports plus concurrent active Recorder close and
    successful SQLite reopen. Two targeted tests and warning-free clippy pass;
    actual hardware close remains C19-pending.
15. Configured physical readiness coverage was red because Host had no explicit
    compatibility-probe operation. Each Metakon definition must now contain the
    trusted read-only channel-type primitive; Host queues it once through the
    normal M3 executor and requires a validated integer value 3. Production
    startup services that bounded probe up to the configured open deadline before
    binding the listener. Temperature acquisition remains a separate scheduled
    read and cannot substitute for compatibility. The first test response used
    the I16 flag for a U8 value and correctly failed; the oracle was fixed to the
    protocol's U8 flag. Physical composition/startup regressions and clippy pass.
16. Live reload provenance coverage initially existed only for startup activation.
    Recorder now accepts the same bounded activation message after startup and
    receipts it with a checked per-boot activation generation while preserving
    run/interval identity and all M7 ingress limits. Configuration reload closes
    an already-open Required gate before domain commit, queues the exact committed
    frozen bundle, and reopens only after the matching durable receipt; a gate
    that was already closed is never spuriously opened. Postcommit storage failure
    leaves the new active identity and faults Required recording. The extended
    process/reopen test observes activation generation 2 and both exact TOML byte
    versions in SQLite. Targeted provenance, Required and clippy suites pass.
17. C5 managed-source separation was red: a label-only configuration reload had
    frozen the newly modified Lua pathname even though it did not advance the
    active component generation. Configuration loading still validates the whole
    candidate, then replaces artifacts for byte-for-byte unchanged managed
    declarations with the prior active frozen bytes before staging. The extended
    C9 test proves label/TOML revision changes retain source bytes and generation
    until the distinct script reload, which then advances the generation while
    preserving that TOML hash. All managed reload tests and clippy pass.
18. C1/C4 declarative native-control startup was red because the typed TOML
    schema did not contain the accepted PID/EMA/lease fields and Host explicitly
    rejected every Reference/controller/safe-profile graph. Added strict bounded
    schema fields and validation, then composed existing Core Fixed/Ramp,
    SafeProfile and native PID objects in safe dependency order. Startup proves
    old-profile safe evidence before readiness, prepares the controller as Ready,
    and leaves the output Disarmed with no lease. A separate red C4 test then
    showed pure validation admitted a mismatched Reference/input unit and a
    measurement parameter disguised as an output; both now reject before any
    artifact read. Three startup tests, five validation tests and warning-free
    targeted clippy pass. No controller is started or armed by configuration.
19. C6 Host integration was red because the accepted lifecycle seam always
    returned `RequiresSafeBarrier`. Core first gained checked complete Reference
    and controller replacement commands; their red tests prove one revision
    advance, reset algorithm/progress state and no authority acquisition. Host
    now pauses Warming/Running controllers, revokes manual/native authority,
    proves safe under the old profile, and only then installs changed model,
    Reference, safe-profile and full PID/EMA/timing values in one serialized
    owner turn. A first manual-owner green attempt exposed that the commit time
    had been captured just before the barrier time, correctly triggering Core's
    monotonic `Output(InvalidTime)` guard; commit now uses the barrier timestamp.
    Host tests prove Warming control becomes Paused and a stale manual lease is
    rejected after reload; both remain safe/disarmed without implicit rearm.
    Full Core, targeted lifecycle/provenance suites and workspace all-target
    warning-denied clippy pass.
20. C5 explicit public staging was red at strict wire decode because only the
    convenience reload operation existed. Added distinct deduplicated
    `stage_configuration` and `apply_configuration` operations with canonical
    candidate/base-revision arguments and bounded diff-effect results. Staging
    performs no active mutation; apply uses the same safe/Recorder lifecycle as
    convenience reload. Moving the Required recording fence into the apply port
    also ensures an unknown/stale/expired candidate is rejected before closing
    recording admission. The public API test observes revision one after stage
    and revision two only after the matching apply. Protocol, deduplication,
    lifecycle and warning-denied clippy suites pass.
21. C14/C15 Host rebind acceptance was red because the stable M3 resource owner
    could only be shut down, not replaced. Core now replaces an adapter only
    after the old executor reports `TransportShutdown::Complete`; Pending keeps
    the old slot quarantined and creates no second owner. Host advances the
    Metakon binding/mapping generations, publishes explicit Unavailable samples
    at the fence, resets the compatibility probe, and retains the logical
    resource ID. The first green attempt exposed that RebindMetakon had not
    invalidated prior Good samples; Core now records Transport-unavailable facts
    under the new generation before reacquisition. Deterministic software tests
    then recovered a different real decoded value through the replacement
    adapter. Added a public deduplicated `reconnect_resource` operation which
    requires resource ID plus expected binding generation and opens only the
    configured port—never an enumerated substitute. COM/physical/Core targeted
    tests and warning-denied workspace clippy pass. No real port was opened, so
    C16/C17 and the hardware part of C14/C19 remain pending.
22. C1 canonical-order acceptance was red because equivalent arrays of instrument
    tables retained source order in the effective graph. The loader now sorts
    every bounded object class by explicit identity before validation and
    scheduling while retaining the exact original TOML bytes/hash separately.
    Managed cross-reference validation was also made order-independent and now
    rejects dependency cycles explicitly. The reordered fixture has equal
    effective graphs and distinct exact-content hashes; all six configuration
    validation tests pass.
23. C18 configured-process coverage was added after the public reload and
    configured-recorder paths existed. The actual Babashka 1.13.220 A process
    connects to a `--config` Runtime, starts Required recording and is killed;
    acquisition/SQLite progress continues without that client. Babashka B resumes
    the same scope, proves a later durable prefix, invokes the explicit
    configuration reload, and requests clean shutdown. Rust then reopens SQLite
    under a new boot and reads at least three archived normal Signal rows. The
    first run exposed that A had not explicitly opened a run, so its `run_id` was
    null; the scenario now uses the public `recording_start` operation rather
    than assuming configuration implies an active experiment run. The targeted
    real-process test passes. This is configured virtual acquisition software
    evidence; it is not the C16/C17 real Metakon bench.
24. C1 repository-example coverage was red because no deployable TOML examples
    existed. Added a complete Required-recording virtual PID deployment and a
    conservative read-only Metakon/COM deployment with a strict trusted-operation
    JSON definition. Both parse through the production loader; the physical
    example is documentation/configuration evidence only and was not opened.
25. The first full debug gate exposed a pre-existing M7 process-level race rather
    than an M8 limit shortfall: the actual recorded Babashka A-kill/B-reconnect
    oracle failed when the already-completed safe `controller_pause` terminal
    arrived while all four ordinary Recorder groups were charged. Diagnostic
    reproduction measured a 418-byte terminal with four groups, six records and
    7,618 bytes outstanding; no byte/record limit was close. The accepted safe
    action must not be undone, and the four-group limit must not be enlarged.
    Host now retains only that safe-reducing terminal in its existing bounded
    pending-operation slot and retries admission after a receipt; ordinary and
    authority-increasing overflow behavior remains fail-closed. A deterministic
    held-writer test fills exactly four groups, proves Required stays Recording,
    releases the worker and verifies the deferred terminal in SQLite. The full
    `recorder_operations` suite and the actual Babashka 1.13.220 recorded A/B
    Pause/Stop/Shutdown/reopen oracle pass after the change. Production followed
    the real process red reproduction; the additional deterministic coverage was
    added afterward, and this sequence is recorded honestly.
26. C20 fixed-ingress coverage was red at compile because Recorder had no live
    activation reservation or typed configuration lifecycle record. The prior
    live activation used a control slot and therefore did not count against the
    accepted four ordinary groups. Recorder now reserves one record/group and
    its bounded payload bytes from the unchanged 1024-record/4-MiB/four-group
    credit before the owner commit. At 4/4 it returns bounded backpressure
    without mutation or failure; after credit releases, the reservation fences
    later record identities and suppresses periodic record allocation until it
    is filled or cancelled. The worker commits exact provenance/object baseline
    and one version-1 `configuration_lifecycle` record in the same SQLite
    transaction, including candidate operation, base/committed revisions, exact
    TOML hash, activation root/generation, affected IDs and monotonic commit time.
    Host quiesces new producers, keeps native safety/transport polling alive,
    drains prior facts, reserves before domain mutation and resumes acquisition
    only after the matching receipt. No M7 limit changed. The new held-writer
    C20 test proves 4/4 rejection, later one-of-four reservation, atomic durable
    fact and exact credit release. Configuration provenance/reload, the actual
    configured Babashka A/B process test and warning-denied clippy pass.
27. C6 definition/address rebind coverage was red because Host exposed only a
    same-definition reconnect helper and Core could not replace a Metakon mapping.
    Core now validates the expected old binding/mapping revisions, requires the
    exact next generations, rejects live dependent control and M8 actuator
    definitions, drops old pending-read correlations, atomically installs the
    read-only definition/binding and publishes explicit Transport-unavailable
    samples under the new identity. Host replaces the already-retired adapter,
    updates temperature/probe operation mappings from the candidate's frozen
    definition and retains the stable resource/instrument IDs. Configuration
    diffing now includes exact definition-content identity, not only its path,
    and resource topology changes remain restart-required. The deterministic
    test changes address 1 to 2 and scale 0.1 to 0.2, observes binding generation
    two and decodes the new-address response only through the new mapping. The
    production apply port now finitely retires each affected old resource before
    opening the exact configured replacement; failure ends quiescence but never
    restores old authority. Configured physical, lifecycle, full Core and
    warning-denied workspace clippy suites pass. This is software evidence only;
    no real COM port was opened.
28. C9/C11 recorded-lifecycle acceptance was red after a successful managed
    source reload because Recorder activation generation remained one. Script
    reload, model restart and explicit reconnect now use the same global
    Required fence, producer quiescence, unchanged four-group reservation and
    atomic activation/lifecycle receipt as configuration apply, while retaining
    their distinct operation kinds and unchanged deployment revision. Managed
    restart prepares every component from the active frozen bytes, commits the
    batch generation together, resets schedules/warm-up and never rereads the
    mutable source path. The red test records Required history, reloads source
    generation 1->2, corrupts the pathname, restarts successfully to generation
    3 from frozen bytes, observes activation generations 2 and 3, unchanged TOML
    hash, two durable lifecycle facts and clean SQLite shutdown. Restart now
    includes configured managed models as well as native thermal models and
    crosses the safe barrier without rearming controllers. Explicit reconnect
    reserves the same durable lifecycle boundary before retiring the old handle
    and releases Required only after the compatibility probe and matching
    activation receipt. Four managed tests, lifecycle regressions, Recorder
    budget coverage and warning-denied workspace clippy pass.
29. Additional C3/C4 boundary coverage was added after the bounded parser and
    configured graph builder already existed, so no production-red sequence is
    claimed. The tests reject nesting depth nine, assignment 4,097, integer
    overflow and instrument 65 before artifact reads; the exact 64-instrument
    maximum parses and composes into the authoritative Host graph. C5 provenance
    coverage now explicitly stages a candidate, mutates the pathname to invalid
    content, and applies the retained candidate while SQLite records the staged
    exact bytes. Both targeted suites pass.
30. The final software gate ran on implementation HEAD `0a42d73`. Both
    `cargo test --workspace` and `cargo test --workspace --release` passed all
    388 named Rust tests, including three actual Babashka process scenarios and
    the recording-enabled crash/reconnect/Pause/Stop/Shutdown/SQLite-reopen
    acceptance. Formatting, warning-denied all-target clippy and warning-denied
    rustdoc passed. The finite `cargo run -p lab-runtime` demo exited normally;
    actual Babashka 1.13.220 `bb test-client` passed 8 tests/13 assertions; and
    `git diff --check` passed. No default test was environment-skipped.

## Resolved versions and fixed bounds

The completion environment used Rust/Cargo 1.95.0 and Babashka 1.13.220.
Relevant resolved crates are `toml` 1.1.6, `serialport` 4.10.1 (default features
disabled), `mlua` 0.11.6, `rusqlite` 0.40.2, `libsqlite3-sys` 0.38.2,
`serde_json` 1.0.151 and `sha2` 0.11.0.

The loader admits at most 64 KiB of TOML, 4,096 syntactic values, depth 8,
128 frozen artifacts and 1 MiB aggregate exact source bytes. Deployment object
limits remain 8 resources, 64 instruments, 8 managed components, 8 References,
8 controllers and 8 safe profiles. Lifecycle staging retains one candidate for
30 monotonic seconds. Lua retains the accepted two workers/one Core stage. COM
retains one request and one completion slot. Recorder retains the M7 limits of
1,024 records, 4 MiB and four causal groups; activation uses one of those groups
and does not add hidden capacity. One lifecycle fact admits at most 256 affected
identities and 64 KiB charged representation.

## Actual Metakon 513 bench attempt — 2026-09-16

The operator supplied a Metakon 513 on COM5, address 5, thermocouple input,
one channel, 9600 8N1/no flow control, unknown firmware, with the actuator load
physically disconnected. The device display was reported as approximately
21 °C. The exact read-only deployment is
`examples/runtime.metakon-513-com5.toml`, SHA-256
`fc849d3586fbc06a1b2465d877100074edda1e6468e4c8f6d5f0a7f2f292ec2c`.
Its frozen definition hash is
`0260b37bce4c72795ad2a55188f4dbbf2b923f8b85ffd177078e27edb4b9921b`.
The definition exposes only `channel_type` register 0 and `temperature`
register 1 as read-only operations; no actuator parameter is present.

The user-approved relative launch command failed before COM composition with
`StorageError("storage path must be a local absolute path")`. Passing the same
unchanged config file by absolute path made its relative Recorder path absolute
and started successfully. This is a configuration-path handling defect, not a
device response. No COM port was opened by the failed attempt.

The absolute-path launch opened only COM5 and reached `ready` under boot
`5c9239c5365b1a20560e2d77a4b9b000`. Reaching readiness proves that the strict
read-only channel-type compatibility probe returned the required U8 value 3 at
address 5/channel 0/register 0. Repeated address 5/channel 0/register 1 reads
passed exact address/function/type/length/CRC validation and published generation
1/revision 1 `Good` signals, but their values were 2.3–2.4 °C rather than the
approximately 21 °C physical display. Five public Babashka samples were
`2.4, 2.4, 2.4, 2.3, 2.3 °C`; the durable run contains twelve such rows with
minimum 2.3 and maximum 2.4 °C.

The accepted codec does not expose or persist a raw-frame log. Its strict decode
and the resulting raw I16 values uniquely constrain the accepted frames below;
these are deterministic reconstructions, not an independent wire capture:

```text
channel_type request:        05 00 00 00 6A
channel_type accepted reply: 05 00 00 00 41 03 00
temperature request:         05 00 01 00 AE
raw 23 accepted reply:       05 00 01 00 44 17 00 2F
raw 24 accepted reply:       05 00 01 00 44 18 00 37
```

Required recording was explicitly started through the public API. It committed
run 1/interval 1 with complete coverage and no first error; its durable prefix
advanced while observations continued. Offline read-only SQLite inspection found
12 `measurement` rows for instrument 1/parameter 2, all `good`, no unavailable
rows, no gaps and zero `output_events`. Exact `runtime_toml` and instrument
definition hashes match the files above. The closed SQLite file is
`examples/metakon-513-com5-history.sqlite`, SHA-256
`91b1da3f76222389ab77d0f4355f5da6ed8b2f0dbb2271ada2a3532b54fb1eec`.

Multiple actual Babashka clients disconnected and reconnected while acquisition
continued. Resource snapshots reported logical resource 1, generation/binding 1,
completed transactions and an empty queue between polls. Shutdown then completed
with `safe_confirmed=true`, `transports_closed=true`,
`recorder_flushed=true`, zero unfinished transports/workers and exit success.
The sealed interval and run both have complete coverage.

No actuator/configuration write, alternate register/address, output test,
implicit reset or experimental command was attempted. On the value discrepancy,
the Runtime was stopped before the physical cable disconnect/reconnect test, as
required. C16/C17 cannot pass until the accepted Metakon 513 register/scaling
knowledge is reviewed against authoritative device documentation or read-only
donor evidence; no speculative scale or register change is authorized.

## Reviewed bench corrections — 2026-09-16

External review accepted the stop and localized the discrepancy to
engineering-value interpretation, without requiring an architecture redesign.
The supplied official protocol conclusion is that Metakon measurement registers
carry raw values without one universal decimal-point position; the polling
application must apply the configured device/sensor interpretation. The strict
frame decoder and signed I16 raw semantics were therefore preserved unchanged.

The initially supplied donor path `D:\rust\com_port_reader` was wrong for this
machine. The user later supplied the actual path
`D:\rust_projects\com_port_reader`. Read-only verification found a clean
`feature/rust-core-api` checkout at
`8226aecafaecbb243d21d0a10e8653a7564ad484`, described as
`v0.1.0-33-g8226aec`. No donor file was modified and donor tests were not run,
because doing so would write build artifacts into the read-only repository.

The donor protocol path is direct and does not contain an implicit decimal-point
rule. `src/protocol/metakon.rs` uses `ReadRegisterRequest::encode`,
`decode_response`, `calculate_crc` and `parse_register_value`; type code 4 is
decoded as little-endian signed I16. `src/instrument/metakon_5x3.rs` uses
`Metakon5x3::verify_channel_type` to require register 0 U8 value 3, maps
`Measurement` to register 1 with `RegisterDataType::Int`, and passes an external
`process_value_scale` through `Metakon5x3Register::engineering_scale`.
`src/lua_api/metakon.rs` obtains that explicit deployment scale from
`app.metakon`, defaulting through `DEFAULT_METAKON_SCALE`; the default in
`src/data/series.rs` is 1.0. Finally,
`SerialCommandSource::read_instrument_value` and `scale_instrument_value` in
`src/acquisition/serial_command_source.rs` compute engineering value as raw times
that supplied scale. Register 0 does not select the scale, and this driver does
not read a separate decimal-position register.

The relevant donor tests are
`matches_single_byte_reference_values`, `encodes_device_one_read_request`,
`encodes_device_two_read_request`, `parses_positive_int_response`,
`parses_negative_int_response`, `rejects_corrupted_crc`,
`describes_all_channel_registers`, `creates_read_request`,
`adds_metakon_series_by_parameter_key`, `reads_metakon_parameter_from_lua` and
`exposes_metakon_parameter_descriptors`. Together they fix CRC/framing, raw I16
decode, register/type mapping and explicit caller-selected scaling. The 0.1 test
cases verify that a caller-selected tenths profile is propagated; they do not
make 0.1 a universal Metakon temperature scale.

More importantly, the donor Git history contains the actual physical deployment.
Immediately before deletion commit `ca25185`, tree
`ecd5fc8cce1b6bf21602703cab2b02824f07c7a9` tracked
`profiles/three_metakon.lua` with COM5/9600/8N1/no-flow and
`lua_scripts/three_metakon_control.lua` with addresses 1, 2 and 5, channel 0,
explicit `scale = 1.0`, and a periodic `measurement` series. Machine-local
ignored logs provide direct execution evidence: `logs/application 2026-08-17.log`
records the profile on COM5 and device 5 measurement reads of 24, 26, 27 and 28
with `scale 1`; line 311 specifically records device 5/register-1 engineering
measurement 24. This explains why the old application displayed a value near the
front panel: raw I16 24 was multiplied by 1.0. The packaged ignored copy in
`dist/com_port_reader-0.1.0-windows-x86_64` retains the same COM5/address-5/
scale-1 deployment, but is supporting machine-local evidence rather than a file
tracked at current HEAD.

The donor therefore independently confirms the reviewed correction: valid
register-1 raw I16 values 23/24 for this Metakon 513 thermocouple deployment are
23/24 °C, not 2.3/2.4 °C. No further software or profile-representation change is
required before the corrected read-only rerun.

Correction A began with red acceptance
`c16_metakon_513_thermocouple_scale_preserves_raw_degree_values`. The actual
bench TOML and strict responses produced `Some(Float(2.3000000000000003))`
instead of `Some(Float(23.0))`. Production Core and the codec did not change.
A new explicit `metakon-513-thermocouple.json` profile applies scale 1.0 only to
this verified deployment. The earlier scale-0.1 definition was renamed
`metakon-temperature-tenths-fixture.json` and its name now states that it is a
fixture-specific tenths profile, not a universal Metakon fact. The targeted C16
test and all five `configured_physical` tests pass. Commit: `27080b2`.

Correction B began with red acceptance
`c1_relative_and_absolute_config_paths_resolve_the_same_recorder_database`.
Relative invocation failed before equivalent startup with `storage path must be
a local absolute path`. The loader now lexically makes the supplied TOML path
absolute once and resolves deployment-owned relative paths from that file's
absolute parent. It does not canonicalize away `..`; the additional
`c3_relative_deployment_resolution_does_not_admit_parent_traversal` test proves
parent traversal still rejects before artifact reads. Relative and absolute
invocation now retain identical exact TOML bytes/hash and resolve the same
Recorder database. All four `configuration_startup` and ten
`configuration_validation` tests pass. Commit: `d330020`.

The original evidence database remains unchanged at
`metakon-513-com5-history.sqlite`, SHA-256
`91b1da3f76222389ab77d0f4355f5da6ed8b2f0dbb2271ada2a3532b54fb1eec`.
It still honestly contains the first attempt's 2.3/2.4 values. The corrected
TOML uses the new, not-yet-created
`metakon-513-com5-corrected-history.sqlite`, so a rerun cannot rewrite or append
to the first archive. Commit: `867f985`.

Corrected exact source identities are:

```text
runtime.metakon-513-com5.toml:
  fd47daf2cac9f8200a6b1ce5d212b9d872572bb006b16c045816577e712bd815
metakon-513-thermocouple.json:
  b631a78a13b9126c430c50732ac1fb3f0739c3e7da7664ef1591e4ce178c65eb
metakon-temperature-tenths-fixture.json:
  c4bb74ac28dcfd8dc7825b0d7e78a6acca60166232f3eabf09e855fef8671610
```

Final correction verification found 391 named Rust tests. Debug and optimized
release workspace suites pass, as do formatting, all-target Clippy with warnings
denied and rustdoc with warnings denied. `git diff --check` passes. The first
debug workspace attempt had one transient timing failure in the pre-existing M7
Recorder test
`start_acceptance_commits_before_held_terminal_then_pause_from_new_client_reopens_once`;
its exact isolated rerun, complete test binary and a second full debug workspace
run all passed. No default test was skipped and COM5 was not opened during these
corrections.

## Current limitations

The first COM5 open/settings/compatibility probe, repeated syntactically valid
reads, Required durability, client independence and clean COM/Recorder close are
retained real hardware evidence, but the values remain evidence of the old
incorrect profile. The corrected profile has not yet opened COM5. Corrected
temperature, physical disconnect/reconnect and final public-history/provenance
evidence for C14/C16/C17/C19 remain pending. Firmware remains unknown and no
independent wire capture exists.

M8 stops at `M8_HARDWARE_ACCEPTANCE_PENDING`; it is not ready for external
review and does not authorize M9. M8 performed no physical actuator write and
makes no physical-output-safety, power-loss, remote-security, GUI or long-soak
certification claim.
