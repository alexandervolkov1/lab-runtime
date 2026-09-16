# M8 implementation report

Status: implementation in progress under SOL_HIGH, 2026-09-16.

Design authority: [MILESTONE_8_DESIGN.md](MILESTONE_8_DESIGN.md). M7 was
externally accepted at `f3ff456`; the design-only checkpoint was committed as
`443fa4e`. This report records actual implementation evidence. Planned tests
are not reported as passing evidence.

## Acceptance implementation map

| IDs | Tests-first implementation slice | Evidence status |
| --- | --- | --- |
| C1-C4 | Strict bounded TOML loader, immutable artifact bundle, schema/cross-reference/safety validation and zero-side-effect rejection | Initial slice green; broader graph/startup cases remain |
| C5-C8 | Staged diff, atomic owner apply, safe barrier, Required durability fence and no rearm | Live/cadence activation reservation, atomic lifecycle provenance and Required fence green; transport-rebind apply remains |
| C9-C11 | Separate managed-source reload and virtual-model restart with generation fencing | Atomic bounded managed batch and native restart software tests green |
| C12-C15 | Bounded read-only Windows COM worker, M3 adapter semantics, disconnect/reconnect/rebind fencing | Software adapter and configured Host acquisition green; reconnect/rebind Host lifecycle remains |
| C16-C17 | Actual Windows COM + Metakon read-only acquisition and durable SQLite inspection | Hardware pending |
| C18 | Public API/Babashka process independence under configured acquisition | Pending |
| C19 | Finite COM + Recorder shutdown in software fault cases and actual hardware | Clean/stalled software transport and active Recorder close green; hardware pending |
| C20 | Fixed Recorder budgets and complete M1-M7/M8 regression gate | Pending |

## Actual sequence

1. Recovered clean accepted implementation HEAD `f3ff456`; `53880ca` is an
   ancestor. The only incoming changes were the expected uncommitted Astra design
   and coordination documents. `git diff --check` passed.
2. Read all required coordination, project, release-plan and M8-design files.
   The donor path `D:\rust\com_port_reader` is unavailable on this computer;
   no donor file was modified. Committed the documentation-only design checkpoint
   as `443fa4e`.
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

Red/green test names, commands, defects and resolved dependency versions will be
added after each logical slice.

## Current limitations

No M8 software or physical hardware acceptance has passed yet. In particular,
fake transports will not be counted as C16/C17 or the hardware portion of C19.
M8 makes no physical-output, power-loss, remote-security, GUI or long-soak claim.
