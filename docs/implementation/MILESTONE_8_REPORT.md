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
| C5-C8 | Staged diff, atomic owner apply, safe barrier, Required durability fence and no rearm | Live/cadence Host apply and public operation green; safe/Required integration remains |
| C9-C11 | Separate managed-source reload and virtual-model restart with generation fencing | Single-component C9/C10 and native C11 green; atomic multi-component reload remains |
| C12-C15 | Bounded read-only Windows COM worker, M3 adapter semantics, disconnect/reconnect/rebind fencing | Software adapter and configured Host acquisition green; reconnect/rebind Host lifecycle remains |
| C16-C17 | Actual Windows COM + Metakon read-only acquisition and durable SQLite inspection | Hardware pending |
| C18 | Public API/Babashka process independence under configured acquisition | Pending |
| C19 | Finite COM + Recorder shutdown in software fault cases and actual hardware | Software pending; hardware pending |
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

Red/green test names, commands, defects and resolved dependency versions will be
added after each logical slice.

## Current limitations

No M8 software or physical hardware acceptance has passed yet. In particular,
fake transports will not be counted as C16/C17 or the hardware portion of C19.
M8 makes no physical-output, power-loss, remote-security, GUI or long-soak claim.
