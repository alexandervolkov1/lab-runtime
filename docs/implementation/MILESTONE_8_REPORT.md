# M8 implementation report

Status: `READY_FOR_HARDWARE_RERUN`; bounded transient-open correction after the
accepted 2026-09-17 physical failure oracle is complete. COM5 has not been reopened.

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
| C14 | `windows_com_transport`, `configured_physical` | Corrected software pass; actual finite disconnect passed, reconnect rerun pending |
| C15 | `configured_physical`, `windows_com_transport` | Software pass |
| C16 | Actual Windows COM + Metakon read-only bench | Corrected acquisition and disconnect pass; corrected reconnect rerun pending |
| C17 | Actual observations, public history and SQLite reopen | Good/Unavailable/failed-reconnect evidence retained; successful reconnect archive pending |
| C18 | `babashka_reconnect`, `host_scheduler`, configured acquisition suites | Software pass; live clients observed acquisition/disconnect/failed reconnect, final A/B phase pending |
| C19 | `com_recorder_shutdown`, `recorder_shutdown`, `runtime_shutdown` | Corrected software and two clean hardware shutdowns pass; final successful-reconnect shutdown pending |
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

## Corrected real acquisition checkpoint — 2026-09-16

After explicit operator approval, the exact relative command in `ai/WORK.md`
loaded corrected TOML hash
`fd47daf2cac9f8200a6b1ce5d212b9d872572bb006b16c045816577e712bd815`
and definition hash
`b631a78a13b9126c430c50732ac1fb3f0739c3e7da7664ef1591e4ce178c65eb`.
Only COM5 at 9600/8N1/no-flow, address 5/channel 0 was opened. Readiness under
boot `dd5ac8d6362a2322046e4ad20b23932c` proves that the strict register-0
compatibility response decoded as U8 channel type 3.

Six consecutive public observations were 28.0 °C/`Good`; later observations
alternated between 27.0 and 28.0 °C. With the verified explicit scale 1.0, the
corresponding strict register-1 signed-I16 raw values are 28 for the six-sample
sequence and 27/28 later. The adapter intentionally retains no separate raw-frame
log, so these raw values are uniquely recovered from the strict decoded integer
and scale rather than claimed as an independent wire capture. The operator then
reported the contemporaneous front-panel value as 28 °C, a plausible exact
agreement with the current public 28.0 °C observation.

The public snapshot reported logical resource 1 idle with an empty queue,
resource generation 1 and latest completed transaction generation 1. Durable
`object_snapshots` recorded device 5/channel 0, binding generation 1 and mapping
revision 1. Durable measurement rows and public history both reported generation
1/revision 1 and `Good` quality.

Required recording started durably as database
`769be53d8fb9cd4dcbb49bf70880cdbe`, run 1/interval 1, label
`M8 corrected Metakon 513 COM5 read-only`. Its live status remained `recording`
with complete coverage, no first error or missing fact, and zero outstanding
groups/records/bytes at the contemporaneous checkpoint; the committed prefix had
advanced to record sequence 647. Public `history_read` returned a frozen complete
raw page of real 27/28 °C observations. A read-only immutable SQLite checkpoint
contained 106 temperature rows at the earlier inspection point, all 106 `Good`,
with range 27–28 °C, and zero `output_events`. Its durable provenance rows contain
the exact TOML and definition hashes above.

The corrected evidence is separate at
`examples/metakon-513-com5-corrected-history.sqlite`. The original erroneous
archive remains unchanged at SHA-256
`91b1da3f76222389ab77d0f4355f5da6ed8b2f0dbb2271ada2a3532b54fb1eec`.
No actuator/output write, configuration operation, alternative register/address,
reset or COM enumeration was performed. At that checkpoint, the corrected
Runtime and Required run remained active for the explicitly operator-cued
physical disconnect step; the run was not yet sealed.

## Actual disconnect contradiction — 2026-09-16

After the operator explicitly reported that the device/cable was physically
disconnected, only public pure queries were issued. No reconnect, write,
configuration operation, alternate register/address, reset or COM enumeration
was attempted. The resource promptly changed from idle to `recovering`, with
active transaction 498 and prior completed transaction 497. The last temperature
remained the old `Good` 28.0 °C sample at monotonic timestamp 495054820600 and
never advanced, so no fabricated new Good sample was published.

The required explicit failure transition did not occur. Across an initial ten
one-second observations and eight further two-second observations—far beyond the
configured 2,000-ms recovery timeout—the same transaction 498 remained active,
the resource never reached `offline`, and the Signal never became `Unavailable`.
The bounded transport queue grew from 21 to its fixed limit of 32 and then stayed
at 32. Required recording itself remained healthy with complete coverage and
continued committing clock/progress facts, but it had no failure observation to
record.

Under the reviewed discrepancy rule, reconnect was not attempted. Public
`runtime_shutdown` was accepted and terminated finitely with
`cleanup_incomplete`: `recorder_flushed=true`, `recorder_unfinished=false`,
`safe_confirmed=true`, `unfinished_workers=0`, but
`unfinished_transports=1`, `transports_closed=false`, `cleanup_complete=false`
and `exit_success=false`. The process exited with code 1 and reported
`safe shutdown incomplete`. The terminal output list was empty.

Offline SQLite inspection found a sealed complete run and interval with no gaps,
381 temperature rows, all `Good`, range 27–28 °C, and zero `Unavailable` rows.
There are still zero `output_events`. The durable boot seal honestly records
resource 1 as `recovering`, generation 1, active transaction 498, latest completed
transaction 497. The corrected database SHA-256 after close is
`d726efe8f562656361c65c970cdb6b78823963861e507026a12c4c3c408593c2`;
the original erroneous archive remains unchanged at
`91b1da3f76222389ab77d0f4355f5da6ed8b2f0dbb2271ada2a3532b54fb1eec`.
The corrected SQLite, empty WAL and SHM files are retained as separate evidence.

This is not a scaling or Metakon frame discrepancy. It is an actual Windows COM
disconnect/recovery and close-boundary contradiction against C14/C16/C17/C19.
M8 stops at `WAITING_FOR_REVIEW`; physical reconnect must not be attempted until
the recovery behavior is reviewed.

## Reviewed recovery correction — 2026-09-16

External review authorized one narrow SOL_HIGH correction without changing the
accepted M3/M8 architecture. Source inspection confirmed the reported causal
chain. `ResourceExecutor::Recovery` carried no deadline; transaction and protocol
`RecoveryStatus::Pending` could therefore remain Recovering forever. The validated
deployment `recovery_timeout_ms` was used only by explicit replacement/reconnect
loops, ordinary admission remained open during recovery, read invalidation waited
for a terminal event, and `try_shutdown` did not call the adapter unless the
executor was already Idle or Offline.

Commit `71d4e56` routes each validated deployment timeout into Core as a monotonic
`Duration`; no TOML, OS or serial dependency enters `lab-core`. Both transaction
and protocol recovery now receive checked exclusive deadlines. Pending remains
Recovering before the deadline. At the deadline, the active correlation becomes
honestly Failed, the resource becomes Offline, and resource generation does not
advance. Only an adapter-proved `RecoveryStatus::Complete` advances generation
and permits the existing at-most-one retry.

The resource owner now rejects ordinary admission while Recovering, Offline,
closing or closed. Work queued behind the failed boundary is fenced and cannot
execute after recovery/rebind; the fixed queue limit remains 32. A bounded owner
event queue drains the active failure and at most the fixed queued set before a
new state transition, so the correction adds no unbounded accumulation. The first
active read failure publishes one `Unavailable` sample immediately through the
normal Signal and Recorder fact path. The prior real Good row remains history,
no numeric replacement is invented, and a clean retry or later explicit rebind
may publish a new Good sample normally.

Shutdown now fences active and queued correlations and starts nonblocking adapter
retirement from Active, Recovering and ProtocolRecovering. Runtime consumes those
terminal/fence events before replacement, preserving started/ambiguity and Signal
facts. No worker is joined unless the adapter already reports its thread finished.
A cooperative COM worker closes normally; a deliberately held worker remains
Pending without blocking, preserving the existing finite
`cleanup_incomplete`/nonzero contract.

The initial owner-boundary acceptance compile was red because
`ResourceExecutor::with_recovery_timeout` and
`TransportError::ResourceUnavailable` did not exist. The protocol and Recorder
acceptance compiles were separately red because Runtime did not expose the
bounded-time registration seam. Production then made those tests green. The
configured periodic-host regression was added after the minimal Core fix; its
first run failed because the assertion incorrectly prohibited the legitimate
pre-failure one-entry queue. The oracle was narrowed to Recovering/Offline and
then proved zero queue growth plus the exact configured 500-ms test deadline.
This production-before-host-coverage ordering is recorded honestly.

New or materially strengthened named regression evidence is:

```text
pending_recovery_deadline_fences_queue_and_enters_offline_without_generation_advance
shutdown_from_recovering_starts_adapter_retirement_without_waiting_for_recovery
shutdown_from_active_fences_started_correlation_and_retires_adapter
timeout_after_a_prefix_fences_old_queue_before_fresh_post_recovery_work
protocol_recovery_pending_is_finite_and_does_not_advance_generation
shutdown_from_protocol_recovery_initiates_adapter_retirement
clean_transaction_recovery_retry_can_publish_new_good_after_unavailable
c14_periodic_reads_do_not_fill_queue_while_finite_recovery_reaches_offline
c14_explicit_rebind_after_offline_fences_old_session_and_resumes_new_generation
physical_read_failure_records_one_unavailable_without_fabricated_good
c19_disconnected_com_worker_retires_cooperatively_without_owner_join
c19_stuck_com_worker_shutdown_attempt_is_finite_and_never_block_joins
```

All 136 `lab-core` tests passed. The focused Windows COM/configured physical/
Recorder quality/COM+Recorder shutdown/Runtime shutdown suites passed 26 tests;
the existing M3 output ambiguity suite also passed. Formatting, targeted
all-target warning-denied Clippy and `git diff --check` passed. The complete debug
workspace gate passed 402 named tests, including actual Babashka A/B process
acceptance. Per the review instruction, full release and warning-denied rustdoc
are deferred until the corrected M8 hardware gate rather than repeated in this
slice. COM5 was not opened.

Both existing bench archives retain their pre-correction hashes. Commit `2672a0a`
changes only the next bench Recorder pathname to
`examples/metakon-513-com5-recovery-corrected-history.sqlite`; that file does not
yet exist. The new exact TOML SHA-256 is
`5fec1c546f81b8a29f7a21360deb33575fe98a87c987fbbd1a28094bd8e01def`.
The definition remains unchanged at
`b631a78a13b9126c430c50732ac1fb3f0739c3e7da7664ef1591e4ce178c65eb`.
The next explicitly authorized bench launch remains:

```powershell
cargo run -p lab-runtime -- --serve --config .\examples\runtime.metakon-513-com5.toml
```

## Actual reconnect lifecycle contradiction — 2026-09-16

The post-recovery-correction bench used boot
`a7bcb055e144ec70b1e071701241a3e5`, exact runtime TOML hash
`5fec1c546f81b8a29f7a21360deb33575fe98a87c987fbbd1a28094bd8e01def`
and unchanged definition hash
`b631a78a13b9126c430c50732ac1fb3f0739c3e7da7664ef1591e4ce178c65eb`.
Only the approved COM5/9600/8N1/no-flow, address-5, channel-0 channel-type and
temperature reads were used. Startup compatibility succeeded with channel type
3. Real pre-disconnect temperature observations were 23--25 degrees Celsius with
`Good` quality and explicit scale 1.0; the operator reported 25 degrees Celsius
on the front panel. Resource generation was 1 and the ordinary queue was empty.

After the operator physically removed the USB-RS485 adapter, the first failed
read published exactly one durable `Unavailable` temperature observation at
record 758 with `failure=Transport` and no value. The previous Good rows remained
in history. The resource reached Offline by the configured 2,000-ms recovery
deadline, transaction 431 became terminal Failed, generation remained 1 and the
queue remained zero. Repeated public snapshots showed the same Unavailable
timestamp and no fabricated Good. Required recording retained complete coverage
and no error.

After the operator physically reconnected the same adapter, the public operation
`reconnect_resource(resource=1, expected_binding_generation=1)` was durably
recorded as accepted at record 1002 and failed with
`invalid_configuration` at record 1003. No second reconnect, configuration
operation, alternate probe or write was attempted. Contrary to that terminal
result, the replacement executor continued ordinary acquisition. Public state
became idle with an empty queue and new real Good observations appeared. Durable
measurement rows after the failed operation carry generation 2/revision 2 and
contain 26--27 degrees Celsius; the replacement executor correlation counter was
distinct from the retired session. This establishes a real failed-lifecycle /
acquisition-ready contradiction rather than stale-value replay.

Source inspection confirmed the exact sequence. `ServiceHost::reconnect_resource`
retires the old resource, opens the replacement `ComTransport`, calls
`rebind_configured_transport()` (installing the new executor and advancing its
binding/mapping identity), then calls the global `begin_configured_probes()` and
waits in `configured_probes_ready()`. Normal host scheduling is already able to
admit periodic temperature reads after rebind. On probe error or deadline,
`cancel_recorded_lifecycle()` records failure but does not quiesce or retire the
installed replacement. The old session was not restored; the defect was that
ordinary acquisition became usable before compatibility and lifecycle success.

Before any production correction, a final public snapshot showed real Good
27.0 degrees Celsius, an idle resource, zero queue and healthy Required recording
with complete coverage. Normal `runtime_shutdown` then completed successfully:
`transports_closed=true`, `recorder_flushed=true`, zero unfinished transports or
workers, `cleanup_complete=true` and `exit_success=true`. The sealed boot reports
complete coverage; the output event count is zero. Offline read-only SQLite
inspection independently confirmed the Unavailable boundary, the accepted and
failed reconnect records, generation-2/revision-2 Good rows after failure, exact
loaded provenance hashes and zero outputs.

This evidence is preserved at
`examples/metakon-513-com5-recovery-corrected-history.sqlite`, final SHA-256
`6afe7924e64326a0dedc37b3d860428a4a351c8f35803c56d51ba271f8caa0e4`.
It must not be reopened for a later bench run. External review authorized a
narrow resource-scoped reconnect-quiescing correction; the next bench must use a
new archive.

## Reviewed reconnect correction — 2026-09-16

External review accepted the physical contradiction as a narrow implementation
defect. The first three acceptance tests were added before production and failed
to compile because the resource-scoped reconnect gate, targeted probe operations,
activation seam and failed-replacement retirement seam did not exist. They
reproduce successful probe gating, wrong-channel-type failure and finite probe
timeout. The multi-resource isolation and shutdown-during-probe cases were added
after the minimal Host implementation and their production-before-coverage order
is recorded honestly. The real failed-reconnect SQLite archive above remains the
end-to-end red evidence for the Service lifecycle path.

Commit `0ae7b3c` adds one bounded `BTreeSet<ResourceId>` owned by Host. Before old
session retirement, explicit reconnect inserts only its target resource. The
periodic Metakon scheduler skips ordinary reads bound to that resource while all
unrelated resources, native safety, recovery and Recorder work continue. Queue
capacity remains unchanged. The rebind-generated Unavailable observation is
captured as the compatibility probe baseline, so it means Pending rather than an
immediate probe failure. Only a later distinct channel-type observation may
complete the probe, and it must be `Good` integer 3.

Reconnect now queues probes only for the target resource. It does not enter the
global configuration safe barrier or global acquisition quiesce. After a valid
probe, the existing durable lifecycle activation is completed before the
resource gate is released. The periodic schedule restarts at one period after
the current monotonic time, preventing a catch-up burst; the compatibility probe
itself never publishes temperature. Application terminal recording occurs in the
same serialized owner turn before any subsequent scheduler service can admit the
first ordinary temperature read.

Wrong channel type, Unavailable/protocol/transport failure or deadline retains
the resource gate, leaves the advanced replacement binding/mapping generation
authoritative, retires the replacement through the existing finite nonblocking
shutdown contract and leaves the temperature Unavailable. No old session or
generation is restored. A later explicit reconnect must therefore use the
actually current generation; deterministic coverage advances the next successful
replacement from generation 2 to 3. Stale-session fencing remains the accepted
M3 replacement rule. A stuck adapter still produces the existing honest finite
cleanup diagnostics and is never block-joined.

New named regression evidence is:

```text
c14_reconnect_replacement_stays_quiesced_until_probe_and_lifecycle_activation
c14_failed_post_install_probe_keeps_new_generation_quiesced_and_offline
c14_probe_timeout_never_releases_ordinary_reconnect_acquisition
c14_reconnect_probe_is_resource_scoped_while_unrelated_resource_continues
c19_shutdown_during_reconnect_probe_fences_work_and_closes_finitely
```

All 12 `configured_physical` tests pass. The focused Core Metakon/output and
Recorder evidence suites pass, as do Windows COM, configured lifecycle,
configuration reload, Recorder quality, COM-plus-Recorder shutdown and Runtime
shutdown suites. The complete debug workspace gate passes; `cargo test
--workspace -- --list` reports 407 named test entries. Workspace all-target
Clippy with warnings denied, formatting and `git diff --check` pass. Full release
and warning-denied rustdoc remain deferred to the final corrected M8 hardware
gate as authorized.

Commit `2a03358` changes only the next bench archive to
`examples/metakon-513-com5-reconnect-corrected-history.sqlite`, which is absent.
The resulting exact runtime TOML SHA-256 is
`200acfe4a2cd36b4375213db9369518d751e0ad43c8401f0b456cb0a03ef6b7c`.
The definition remains unchanged at
`b631a78a13b9126c430c50732ac1fb3f0739c3e7da7664ef1591e4ce178c65eb`.
COM5 has not been reopened after this correction.

## Corrected reconnect hardware review stop — 2026-09-16

The next read-only run started from HEAD `bb62808` with boot
`0f392f7a6928adac89f458f0a9b71188`. Before COM5 was opened, the new archive was
confirmed absent and the exact runtime TOML and definition hashes were confirmed
as, respectively,
`200acfe4a2cd36b4375213db9369518d751e0ad43c8401f0b456cb0a03ef6b7c` and
`b631a78a13b9126c430c50732ac1fb3f0739c3e7da7664ef1591e4ce178c65eb`.
The service opened only the approved COM5/9600/8N1/no-flow, address-5, channel-0
deployment and reached ready after its trusted read-only channel-type probe
returned 3. No output, configuration-register, alternate-register, reset,
enumeration or implicit control-line operation was requested.

Required recording started as database `9ca1316c11d71c50f4c1f944f3227b75`, run 1,
interval 1. Consecutive strict register-1 results decoded as raw I16 28 and,
through the explicit deployment scale 1.0, published as `Good` 28.0 degrees
Celsius. The operator contemporaneously reported 28 degrees Celsius on the
front panel. Resource generation was 1, the queue stayed at zero apart from a
snapshot taken during one active read, Recorder coverage was complete and the
durable watermark advanced normally.

After the operator physically removed the USB-RS485 adapter, public state and
durable history showed the corrected causal boundary. Records 223 and 225 are
the last `Good` 28.0-degree observations; record 227 is exactly one
`Unavailable`, `failure=Transport`, with no numeric value. Resource generation
remained 1, transaction 171 became terminal Failed, the resource reached Offline,
the queue stayed zero through repeated polling and no later Good observation was
published. Required recording remained healthy and complete through this
disconnect boundary.

After operator-reported physical reconnection, the client issued exactly one
`reconnect_resource(resource=1, expected_binding_generation=1)`, request scope
`0f392f7a6928adac89f458f0a9b71188:1`, sequence 7. The Babashka client did not
receive an initial reply within its fixed two-second frame timeout, so it did not
retry. Reconciliation through `operation_status` returned terminal
`failed`, code `invalid_configuration`. Public state remained acquisition-safe:
temperature and channel type were Unavailable, resource state was Offline,
resource generation was 1, the ordinary queue was zero and no post-reconnect
Good temperature was published. Thus the new resource-scoped gate prevented the
previous escaped-acquisition defect, but the real reconnect still did not pass
compatibility/lifecycle activation.

The same reconnect exposed a second failure. Required Recorder became `failed`
with first error `fact record reservation mismatch`, coverage `unknown_tail`,
worker closed, durable prefix through record 472, three outstanding groups and
four outstanding records. Offline SQLite contains the reconnect `accepted`
event at record 472 but no durable terminal reconnect event. The application
reported terminal failure to the reconciled public client, so this is an
explicit durability gap rather than a claim that the failed outcome was recorded.
Required fail-closed behavior kept acquisition disabled.

Normal `runtime_shutdown`, request sequence 8, was accepted. Its terminal reply
did not arrive within the client timeout and the process exited finitely with
code 1 and `safe shutdown incomplete`; success was not fabricated. The archive
has no remaining WAL or SHM sidecar after process exit, but it is intentionally
unsealed: the boot and run remain active/recording, with a complete committed
prefix rather than a terminal complete-run claim. Independent read-only SQLite
inspection found 54 generation-1 Good 28.0-degree rows, the single generation-1
Unavailable disconnect row, zero output events, no declared gaps, the accepted
reconnect prefix, and exact runtime-TOML/definition provenance hashes above.

The evidence is preserved at
`examples/metakon-513-com5-reconnect-corrected-history.sqlite`, SHA-256
`ef16177a913216ad4e39a9c101e7ab749994f337ece6db2525c6bc396a45d377`.
It must not be reused or rewritten. Babashka-independence, live-safe reload,
successful generation-2 acquisition, clean terminal shutdown and the final M8
software gate were not attempted after the failed reconnect. No production code
or tests changed during this bench run. This observed contract failure requires
external review before another COM5 run or correction phase.

## Reviewed Recorder ordering correction — 2026-09-16

External review authorized a narrow SOL_HIGH correction without reopening COM5.
Current-source inspection confirmed the proposed causal chain exactly. The old
`try_reserve_live_activation()` implementation reserved bounded capacity and a
generation, but also called `planned_range(1)`, advanced `reserved_through` and
stored that future record identity before sending an Activation message. During
resource-scoped reconnect, the trusted channel-type compatibility probe could
then emit a normal fact while that reservation remained open. The owner assigned
the probe the following identity and sent it first. The single SQLite worker
still expected the unsent lifecycle identity and correctly rejected the probe
group with `fact record reservation mismatch`.

The preserved physical archive is consistent with this chain: its committed
prefix ends at record 472 with the reconnect Accepted event, while the reserved
lifecycle identity and later probe/lifecycle facts are absent. Required Recorder
therefore failed with an unknown tail before lifecycle durability. Source tracing
also confirmed that the public `failed/invalid_configuration` result masked this
Recorder/owner failure; it was not evidence that the returned channel type was
wrong. A deterministic reconnect test proves that the same Good integer value 3
passes compatibility when Recorder ordering remains healthy. The existing wire
category `recording_unavailable` is now used for this lifecycle storage failure;
other lifecycle validation failures retain `invalid_configuration`.

Commit `fdf8a73` changes the reservation to charge only its existing one record,
serialized byte size, one group and activation generation. It does not assign a
record identity or advance `reserved_through`. Ordinary facts, including the
compatibility probe, can therefore continue through the one owner FIFO while the
credit remains unavailable to competing work. At commit, the owner revalidates
the matching generation, assigns the current FIFO-tail identity and immediately
`try_send`s that Activation on the same sender. Only successful enqueue advances
`reserved_through`; the reserved capacity is not charged twice. Cancellation
releases capacity and the pending generation without rewinding any identity.
The SQLite worker's exact contiguous-ID checks remain unchanged.

The worker receipt also preserves the maximum already-confirmed monotonic
submission time when a later FIFO Activation carries an earlier lifecycle start
time. Periodic Recorder clock facts are no longer globally suppressed by an open
resource lifecycle reservation. No queue, thread, schema or retention bound was
added: the limits remain 1,024 records, 4 MiB and four groups with one pending
activation. Resource-scoped reconnect quiescing, Required fail-closed behavior,
generation fencing and finite shutdown are unchanged. If activation enqueue or
durability fails after replacement installation, Recorder fails, lifecycle
fails, the advanced generation remains authoritative, and ordinary acquisition
stays quiesced/offline.

Tests-first evidence was recorded honestly. The three new worker-level cases
initially failed with the exact reservation mismatch or cancellation mismatch
before production changed. After the correction they passed. The Host causal
tests were then added; their first fixture used timing shorter than the accepted
C14 profile and its bad-probe oracle expected an integer 2 although the normal
fact path publishes Unavailable for that failure. Those fixture/oracle defects
were corrected without changing production. The public error-mapping test was
compile-red until the internal `RecordingUnavailable` lifecycle category and
mapping existed. Named new evidence is:

```text
live_activation_capacity_allows_fifo_fact_before_commit_without_identity_hole
cancellation_after_intervening_fact_groups_releases_credit_without_sequence_rewind
unavailable_fact_before_lifecycle_cancellation_remains_contiguous_and_recorded
reconnect_probe_fact_precedes_durable_activation_and_later_good_temperature
failed_probe_cancels_after_recorded_fact_without_gap_or_good_temperature
required_recorder_failure_after_rebind_stays_quiesced_and_never_rolls_generation_back
recorder_lifecycle_failure_uses_existing_recording_unavailable_wire_code
```

The successful Host chain proves Good/Unavailable generation-1 history, explicit
replacement generation 2, durable Good channel-type 3 before lifecycle
Activation, durable lifecycle completion before release, and only then a Good
temperature. Record IDs remain contiguous with no mismatch or unknown tail. The
failed-probe variant records its Unavailable probe fact, cancels without a gap,
keeps generation 2 quiesced/offline and emits no temperature Good. The injected
Required-storage failure variant remains fail-closed with an honest unknown tail
and never rolls generation back.

Targeted Recorder/Required/backpressure/reopen/history, configured physical,
Windows COM, shutdown, lifecycle and Core transport/Metakon suites passed. All
136 `lab-core` tests passed. The complete debug workspace rerun passed and
`cargo test --workspace -- --list` reports 414 named tests. Actual Babashka A/B
process acceptance passed all three `babashka_reconnect` tests with Babashka
1.13.220. The first full workspace attempt transiently observed two new Host
tests enter Recorder Failed during setup; isolated reruns, 20/20 parallel
repetitions and the subsequent complete workspace run all passed without code
change. Formatting, workspace all-target Clippy with warnings denied and
`git diff --check` pass. Clippy first required moving the new application test
module after production helpers. Full release and warning-denied rustdoc remain
deferred until successful final hardware acceptance as authorized.

Commit `2ec104b` changes only the next bench archive pathname to
`examples/metakon-513-com5-recorder-corrected-history.sqlite`. The archive was
confirmed absent and was not created or opened. Its exact runtime TOML SHA-256 is
`405d99056fd4ba265bb8776f18360a33cd4be81e65c7e03976196acbf374056b`.
The definition remains
`b631a78a13b9126c430c50732ac1fb3f0739c3e7da7664ef1591e4ce178c65eb`.
The prior defect archive retains SHA-256
`ef16177a913216ad4e39a9c101e7ab749994f337ece6db2525c6bc396a45d377`.

## Recorder-corrected physical reconnect review stop — 2026-09-16

The accepted run started at HEAD `5355963d9e4bb3efa5f5214223a1510cc25619e9`
after confirming that the new archive did not exist. Runtime TOML and definition
SHA-256 values matched, respectively,
`405d99056fd4ba265bb8776f18360a33cd4be81e65c7e03976196acbf374056b` and
`b631a78a13b9126c430c50732ac1fb3f0739c3e7da7664ef1591e4ce178c65eb`.
The prior Recorder-defect archive retained SHA-256
`ef16177a913216ad4e39a9c101e7ab749994f337ece6db2525c6bc396a45d377`.

The process boot was `4827bf27ac961d59e482dd2837ee21bb`; Required database
identity was `f68d0f82bdf1559ebc36ba63183a7503`. Only the approved
COM5/9600/8N1/no-flow, address-5, channel-0 read-only deployment was used.
Startup compatibility committed channel type integer 3. Strict register-1 reads
decoded raw signed I16 values 29/30 and the explicit scale 1.0 published 29/30
degrees Celsius with Good quality. The operator contemporaneously reported 29
degrees Celsius. SQLite contains 165 generation-1 Good temperature rows, records
61 through 389, with range 29--30 degrees Celsius. Required recording was
healthy, complete and bounded; public history independently exposed durable rows.

The operator then powered the device off. This is recorded only as
`operator-reported device power-off`, not as a Rust-observed power state. The
last Good row is record 389. Record 391 is exactly one generation-1
`Unavailable`, `failure=Transport`, with no numeric value. Resource transaction
224 became terminal Failed after starting, the resource reached Offline,
generation remained 1, the queue stayed zero and no later Good was published.
Repeated public snapshots retained the same Unavailable timestamp. Required
Recorder remained Recording with complete coverage, no error and no outstanding
credit. No automatic reopen or rebind occurred.

The operator then powered the same device on; this is recorded as
`operator-reported device power-cycle/reset boundary`. Exactly one public
`reconnect_resource(resource=1, expected_binding_generation=1)` was issued under
request ID `4827bf27ac961d59e482dd2837ee21bb:13/1`. SQLite records its Accepted
phase at record 677 and its terminal Failed phase at record 678 with public code
`invalid_configuration`. No retry was attempted.

This failure is not the corrected Recorder ordering defect. Recorder remained
healthy and complete with no `fact record reservation mismatch`, no
`recording_unavailable`, no gap and no unknown tail. The resource remained
Offline at generation 1; channel type retained its old generation-1 observation,
temperature retained the disconnect Unavailable, and SQLite contains no
generation-2 measurement or configuration-lifecycle activation. These facts
localize the failure before a replacement binding crossed the generation fence.
The current public and durable diagnostics do not distinguish whether the old
adapter retirement boundary failed to complete or the configured COM5 reopen
failed before `rebind_configured_transport()`. The report does not guess between
those source branches and no alternate open, probe, port, register or reset was
attempted.

After the stop decision, newly allocated diagnostic client scopes reached the
bounded session admission limit and `hello` returned `busy`. Resuming the known
original scope `4827bf27ac961d59e482dd2837ee21bb:2` worked and issued the sole
normal shutdown. Shutdown completed successfully with
`transports_closed=true`, `recorder_flushed=true`, zero unfinished transports
and workers, `cleanup_complete=true`, `exit_success=true` and no outputs. The
boot, run and interval are sealed with complete coverage. SQLite has no gaps,
zero output events, Accepted/Failed reconnect evidence and a terminal durable
checkpoint through record 849.

The new evidence archive is preserved at
`examples/metakon-513-com5-recorder-corrected-history.sqlite`, final main-file
SHA-256
`49c21ab48a22b4c2d0100357686f1b367d0432ce641dd9c8e21eee3184e4c6de`.
Its WAL is empty after clean exit; the SQLite sidecars and main file are retained
unchanged. Babashka client-independence, live-safe configuration revision and the
final release/rustdoc gates were not attempted after reconnect failure. No
production source or test changed during this run.

## Reviewed pre-rebind correction — 2026-09-16

External review authorized this software-only M8 correction and prohibited a
COM5 open. Source plus deterministic reproduction localize the preceding
physical failure exactly to `retire_old_begin`, before an old-worker retirement
attempt, replacement worker spawn, actual Windows port open or Core rebind.
`HostCore::prepare_configured_transport_replacement()` supplied its cached
`last_now` to `Runtime::shutdown_transport()`. A preceding owner service turn
can poll transports at a later `clock.now()` than that cache, so Core correctly
rejected the backwards timestamp as `Transport(InvalidTime)`. The first
service-level retirement regression observed `RetireOldFailed` instead of its
intended timeout branch for precisely this reason. That failure path explains
the physical generation remaining 1 and proves that asynchronous COM5 reopen
was never reached.

`reconnect_retirement_uses_current_monotonic_time_after_transport_poll` was
compile-red because the old Host API accepted no current owner time. The
correction passes the reconnect turn's current monotonic time explicitly; no
generic `ResourceExecutor` contract or `lab-core` code changed. The test now
advances Core transport time ahead of the old Host cache and proves finite clean
retirement at the nondecreasing time.

The requested lost-stop hypothesis was also audited independently. It is a real
latent adapter defect but was not the cause of the preserved physical run: the
old `ComTransport::retire()` changed state to Closing, used fallible
`try_send(Request::Stop)`, ignored `Full`, and never retried because subsequent
calls returned early. The tests-first worker regression was compile-red before
production because there was no persistent stop intent or worker-loop seam.
`occupied_mailbox_retirement_cannot_lose_stop_or_start_next_data_operation` now
holds one bounded device call, fills the one ordinary mailbox, proves the Stop
wake-up hint is Full, releases the call and verifies that persistent coalesced
retirement prevents the queued read, emits Stopped and finitely exits. Stop is
checked before the next request and after each bounded OS call; there is still
one worker, one ordinary slot, no thread kill and no blocking join.

Worker completion is now honest even when the bounded Stopped completion is
missing or its channel closes: only `JoinHandle::is_finished()` permits Closed,
covered by `finished_worker_without_stopped_completion_is_honestly_closed` and
`finished_worker_with_closed_completion_channel_is_honestly_closed`. A held OS
call remains Pending under the existing
`c19_stuck_com_worker_shutdown_attempt_is_finite_and_never_block_joins` oracle.

`ComTransport::open_windows()` still deliberately returns after spawning a
worker in `Opening`; spawn is not OS-open evidence. The new nonblocking
`open_status()` distinguishes Opening, typed asynchronous failure and Ready.
Ready means `SerialPortDevice::open()` completed and its configured serial
settings readback matched. Explicit reconnect now waits through that finite
preparation before transferring the candidate to Core. Open failure preserves
generation 1, cancels reserved lifecycle credit, leaves Required Recorder
healthy and retires the candidate. A still-running open is retained in the one
quarantined candidate slot, blocks another replacement and participates in
finite shutdown; it is never silently detached and replaced.

One replace-in-place `ReconnectDiagnostic` records no OS-owned strings and
retains the resource, current/target generation, exact bounded stage, bounded
COM state, typed serial error, old-worker completion, replacement spawn,
actual-open confirmation and Core-rebind fence. Its stages distinguish safe
barrier, Recorder reservation, old retirement begin/pending/timeout/failure,
settings, worker spawn, actual Windows opening/failure/readiness, install, Core
rebind, resource-specific probe enqueue/wait/failure, lifecycle Recorder commit
and lifecycle durability. Failed public reconnect terminals include this bounded
detail and remain deduplicatable/reconcilable.

Transport retirement, worker-spawn and actual-open failures now use one stable
`transport_unavailable` wire code rather than `invalid_configuration`.
Invalid candidates retain `invalid_configuration`; stale expected generation
retains `revision_conflict`; Required lifecycle storage failure retains
`recording_unavailable`. Probe incompatibility remains a configuration/domain
failure with the precise stage detail rather than being mislabeled as an open
failure.

New named regression evidence is:

```text
reconnect_retirement_uses_current_monotonic_time_after_transport_poll
occupied_mailbox_retirement_cannot_lose_stop_or_start_next_data_operation
finished_worker_without_stopped_completion_is_honestly_closed
finished_worker_with_closed_completion_channel_is_honestly_closed
asynchronous_open_failure_is_distinct_from_ready
reconnect_retirement_timeout_reports_exact_pre_rebind_stage
reconnect_worker_spawn_failure_keeps_old_generation_and_recorder_healthy
stale_expected_generation_is_conflict_without_prior_failure_diagnostics
asynchronous_candidate_open_failure_never_crosses_generation_fence
unfinished_candidate_open_is_quarantined_until_worker_finishes
failed_probe_is_distinct_and_keeps_installed_generation_quiesced
ready_candidate_installs_once_then_probe_and_lifecycle_release_acquisition
reconnect_transport_failure_and_stale_generation_keep_distinct_public_codes
```

The service regressions use a real `ServiceHost`, Required SQLite Recorder and
the production COM-worker abstraction over deterministic devices without
opening an OS port. Every pre-rebind failure retains binding generation 1,
healthy Recorder state and zero configured outputs; every cooperative failure
shutdown closes and flushes, while the deliberately unfinished old worker
returns finite unsuccessful shutdown. The successful chain proves old Offline,
clean retirement, candidate actual-open Ready, one Core rebind to generation 2,
Good channel type 3, durable lifecycle and only then reconnect release. Existing
stale-session, probe, FIFO, Required failure and later Good-temperature tests
remain green.

Focused verification passed the 31-test `lab-runtime` library, all 136 Core
tests, Windows COM, 13 configured-physical, COM/Recorder shutdown, Runtime
shutdown, Recorder Required/FIFO/reload-budget and lifecycle/public-operation
suites. The first complete workspace run had one transient setup failure in
`reconnect_probe_fact_precedes_durable_activation_and_later_good_temperature`;
its exact isolated rerun, complete 31-test library rerun and subsequent complete
workspace rerun all passed without a code change. The latest workspace lists
427 named tests. Actual Babashka 1.13.220 A/B passed all three process tests.
Formatting, workspace all-target warning-denied Clippy and diff checks pass.
As explicitly authorized, release-profile tests and warning-denied rustdoc remain
for the final successful hardware gate.

Production correction commit is `6f641e2`. The next archive path is
`examples/metakon-513-com5-prepared-reconnect-history.sqlite`; main, WAL and SHM
are all absent. Exact runtime TOML SHA-256 is
`8688bf121b27a6ffc88a73eb35fc23def9c0787330eaf2168899198c41f5186c`.
The definition bytes are unchanged at SHA-256
`b631a78a13b9126c430c50732ac1fb3f0739c3e7da7664ef1591e4ce178c65eb`.
Both supplied immutable evidence archives retain their recorded hashes. The next
authorized work is only an explicitly approved read-only hardware rerun using:

```powershell
cargo run -p lab-runtime -- --serve --config .\examples\runtime.metakon-513-com5.toml
```

COM5 was not opened during this correction. M9 remains unauthorized.

## Transient Windows-open failure oracle and correction — 2026-09-17

The final read-only rerun used the exact LF deployment hash
`8688bf121b27a6ffc88a73eb35fc23def9c0787330eaf2168899198c41f5186c`
and unchanged definition hash
`b631a78a13b9126c430c50732ac1fb3f0739c3e7da7664ef1591e4ce178c65eb`.
Initial COM5 acquisition was healthy: actual open/configuration succeeded, channel
type was 3, repeated generation-1 Good temperatures of 23/24 degrees Celsius agreed
with the front panel, Required Recorder was healthy, durable/public history contained
real rows and no output event existed.

After the operator-reported power-off, exactly one value-less
Unavailable/Transport was recorded and no later Good was fabricated. The resource
reached Offline, generation remained 1, the ordinary queue remained empty and the
old worker retired. Following the operator-reported power-on, exactly one public
reconnect was accepted. It spawned one replacement worker, whose first actual Windows
open failed with `SerialError::Disconnected`. No Core rebind or compatibility probe
occurred, generation stayed 1, Required Recorder remained complete, shutdown was
finite and clean, and the archive contains zero output events.

That immutable failure oracle is
`examples/metakon-513-com5-prepared-reconnect-history.sqlite`, main-file SHA-256
`1396421e62b5a1abb834b4178689b3303277a88d46174e0353d2710c2ab17023`.
Its main/WAL/SHM files are retained and must not be modified or reused.

Source review localized the cause to `SerialPortDevice::open()`, `worker_main()` and
`ServiceHost::reconnect_resource_with_factory()`: Windows/serialport `NoDevice` was
truthfully mapped to `Disconnected`, but one failed asynchronous factory/open attempt
immediately terminated the worker and cancelled the lifecycle. Therefore the
validated `open_timeout_ms` bounded waiting for one attempt rather than providing a
bounded transient-absence grace.

Tests were added red-first for transient-first-then-Ready, several transient failures
then success, finite absence, terminal invalid/non-retryable errors, hung-open worker
uniqueness, sequential retirement, generation/probe/acquisition fences, Required
Recorder health and finite shutdown. Production commit `a55acb1` keeps retry inside
one candidate worker: one public reconnect, one candidate, one worker and at most one
active OS-open call. Completed `Disconnected` attempts retry every 100 ms, capped at
64 attempts and by the single original `open_timeout_ms` absolute deadline. The
deadline is never reset. `InvalidSettings`, `Timeout` and `Other` are terminal. A hung
OS open creates no concurrent replacement and remains governed by existing
timeout/quarantine/shutdown semantics. No protocol byte or compatibility probe can
occur before actual-open/settings-readback Ready.

The post-correction software gate passed all 436 named debug workspace tests, all 40
`lab-runtime` library tests, the focused reconnect/Windows COM/configured-physical/
Recorder/shutdown suites, actual Babashka 1.13.220 A/B (three process tests), Babashka
client tests (8 tests, 13 assertions), formatting, workspace all-target Clippy with
warnings denied and `git diff --check`. COM5 was not opened during correction.
Release-profile tests and warning-denied rustdoc remain reserved for the eventual
successful final hardware gate.

The next new archive is
`examples/metakon-513-com5-transient-open-retry-history.sqlite`; main/WAL/SHM were
confirmed absent. Its exact LF runtime TOML SHA-256 is
`39715f3d70391154935f3ab6b25a1e78162f2b711f038238496f8853dc729c09`;
the definition hash remains unchanged. Exact deployment provenance currently depends
on checkout line endings when `core.autocrlf=true`; later release hardening should pin
relevant provenance-bearing text files to LF via `.gitattributes`. That unrelated
hardening is deliberately not included before this hardware rerun.

## Current limitations

The first archive remains evidence of the old incorrect profile; the separate
corrected run establishes plausible real temperature and durable history. The
pre-fix disconnect and failed-reconnect contradictions are preserved as evidence.
The latest run proves corrected finite disconnect and acquisition quiescing, but
actual reconnect stopped at a transient Windows open failure before Ready. The
preserved archive has a complete Recorder prefix and clean shutdown. Firmware remains
unknown and no independent wire capture exists.

M8 is `READY_FOR_HARDWARE_RERUN`, not ready for acceptance and does not authorize
M9. The transient-open failure is localized and corrected in software, with bounded
single-worker retry diagnostics ready for the rerun. Successful physical reconnect,
Babashka independence during that physical run, harmless live-safe reload and
the final release/rustdoc gates remain open. M8 performed no physical actuator
write and makes no physical-output-safety, power-loss, remote-security, GUI or
long-soak certification claim.
