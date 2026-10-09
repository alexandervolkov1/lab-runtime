# M18 implementation candidate: stopped for Recorder admission review

**Historical review record.** The Recorder remediation was subsequently approved
and committed; current M18 scope, verification and remaining review limits are in
[M18_CONSOLIDATED_REVIEW.md](M18_CONSOLIDATED_REVIEW.md). Earlier restrictions,
HEAD/status claims and failed gates below describe their dated diagnostic stage.

Date: 2026-10-09. Branch: `feature/m18-distributed-workbench`.
Base HEAD: `b546354bddf035c15f78f1f1d1ea1d22eb84ac76`.
No new commit, merge, push, tag or release. This is an incomplete working-tree
candidate, not M18 acceptance. Evidence is under
`target/m18-implementation-20261009/`; original diagnostics remain intact.

## Implemented candidate and proof boundary

The existing writer high-water correction is preserved: after successful SQL
commit, `confirmed_submission` is the maximum original submission timestamp in
the committed FIFO prefix. `persisted_through_sequence` and cumulative released
credits identify that prefix; the timestamp is not a claim that later-enqueued
records with earlier timestamps have committed. Owner receipt checks are unchanged.

`worker/reference_completion.rs` reserves one existing ordinary group, two records
and a bounded byte envelope before synchronous Reference dispatch. One Reference
fact and the corresponding terminal audit share a SQL transaction. Original times,
scope and request sequence are retained. No record IDs are assigned until FIFO
transfer. Failed domain commands cancel only the unused fact reservation. A failed
capacity reservation returns existing `recording_unavailable` before the Reference
side effect; it does not rewrite an executed command into a failure.

This covers `reference_retune` and `reference_configure` only. It does not protect
arbitrary asynchronous mutations or the preceding accepted audit. The default
four groups / 1,024 records / 4 MiB and per-group limit are unchanged.

`worker/reserved_facts.rs` separately reserves the existing maximum causal-group
envelope for rebind baselines and resource compatibility probe results. Conversion
to the actual bounded group happens on the sole owner, without intervening work;
unused capacity has no assigned record IDs. Transferred capacity still requires a
post-commit receipt. The lifecycle activation reservation remains separate. This
candidate is not sufficient proof for all reconnect paths: the full Linux test
below still fails and the changes must not be accepted as a complete remedy.

A related panic-window regression was diagnosed and corrected in fact ingress:
receiver destruction can precede `JoinHandle::is_finished()`. A disconnected writer
now preserves `unknown_tail`, matching the existing panic contract, rather than
classifying the unknown SQL tail as ordinary queue overflow. Full-queue admission
still latches a truthful gap. The existing panic/rebind assertion is preserved.

Six deterministic regressions in `host/reference_completion_tests.rs` prove:

- Two pending groups + accepted audit + completion fit the original four slots;
  fact and terminal audit persist together, stop/finish and reopen succeed.
- Failed Reference domain validation releases only the unused fact allowance.
- Three pending groups exhaust the reservation after accepted audit, return
  `RecordingUnavailable`, retain Reference revision 1 and preserve a durable gap.
- A real deferred-FK COMMIT failure rolls back both fact and terminal and releases
  no credit for them; the earlier accepted audit remains durable.
- Holding after COMMIT but before receipt retains completion credit and the old
  owner prefix. Duplicate polls release no credit twice; generation stays fixed.
- Separate fact reservations backpressure at four slots, reject double cancel,
  use fresh tokens, transfer exact credit and preserve contiguous FIFO identities.

All six passed 10 repeated Debug runs on each platform (60 evaluations per OS).
The complete Windows reconnect subset passed 43/43 once. These passes do not waive
the following process and full-workspace failures.

## Remaining Recorder blocker: the approved grouping is insufficient

The new Arch process failure is **not lost receipts** and is not a flaky assertion:

1. Recording starts and independently commits its accepted/completed audits.
2. Ordinary acquisition publishes three groups while writer coalescing/SQL is
   pending: records 6–7 (measurement + Reference), 8 (managed measurement), and
   9–10 (measurement + Reference). Distinct `captured_at` values identify the three
   actual transfers. Writer coalescing does not release their owner credits.
3. `reference_retune` is admitted into SessionStore. Its accepted audit takes
   group four, record 11.
4. The requested completion reservation would be group five. It correctly fails
   before side effect. The client gets accepted followed by
   `failed / recording_unavailable`; Reference revision remains 1 and target 50.
5. Record 12 is the durable `recorder_gap` with reason
   `recorder Reference completion capacity exhausted`. The accepted audit exists,
   but there is no terminal operation audit. SQLite integrity is `ok`.

Exact evidence: `arch-debug-a-control-ws-observer-tcp-3-transcript.jsonl`, its
`stderr.log`, and `arch-failed-rows.json`. The isolated database is
`/home/alex/lab-runtime-m18-implementation-20261009/target/matrix-a/arch-debug-a-control-ws-observer-tcp-3/history.sqlite`
inside Arch WSL2. The harness terminated only its own failed process after the
assertion; clean shutdown/reopen is **not** claimed for that failed case.

This is one ordinary retune following recording/subscription, not a mutation
flood. The two-pending-group proof does not cover three pending groups. Once
SessionStore has accepted and the independent accepted audit consumes the last
slot, the present contract leaves no capacity for the mandatory terminal audit.
Increasing credits, shortening sleeps, blind retry or hiding the missing audit
would not establish the requested invariant.

### Minimal next decision for external review (not implemented)

For a **new** recorded Reference mutation, reserve both the accepted-audit slot
and the completion slot atomically **before SessionStore admission**. If those
ordinary credits are unavailable, return a synchronous `busy` without consuming
the mutation sequence, changing Reference, or failing Recorder. Known exact
requests must retain their existing replay/status behavior even when full; failed
admission must cancel both reservations without assigning FIFO record IDs. Both
messages retain separate original timestamps and commit in FIFO order.

This adds Recorder capacity as an Application admission condition. The existing
wire `busy` shape alone is not proof that changing admission is authorized or
correct. It requires review of mutation sequence/recovery/BestEffort behavior and
new deterministic tests before implementation. An alternative is combining the
accepted audit into the completion transaction, but that changes the established
independent accepted-evidence crash boundary and is not proposed as a silent fix.

Reconnect also remains blocked: on the fresh Linux build,
`api_provisioned_signals_use_generic_application_history_recorder_and_shared_reconnect`
returns `RecordingUnavailable` at the reconnect assertion in `service.rs`.
The new Metakon probe/baseline budget does not establish complete protection for
SimpleDevice reconnect and continued unrelated acquisition. No generation rollback,
global acquisition pause or weakened assertion was introduced to conceal this.

## TCP LAN changes and validation

- Both Runtime startup modes accept `--bind IPv4 --allow-remote-tcp`. Loopback is
  the default; non-loopback without opt-in, wildcard, multicast, broadcast, invalid
  and duplicated arguments are rejected. Config mode retains the TOML port; the
  override is startup-only and does not change the deployment DTO.
- WebSocket loopback bind, Host, Origin and framing remain unchanged.
- Workbench uses the existing worker/transport with `--allow-remote-tcp`; remote
  numeric IPv4 endpoints are validated at parsing and worker construction. Its
  local API remains loopback-only. Cross-protocol flag combinations are rejected.
- Bundled `runtime.clj PORT --host IPv4 --allow-remote-tcp` retains local defaults.
  Octets are validated without DNS or unsupported Babashka reflection. No external
  `thermal-plant-analysis` changes were made.
- Runtime startup/CLI tests passed on Windows and Ubuntu WSL2, including real
  hello/latest and clean shutdown on selected `192.168.0.105` and `172.31.158.229`
  interfaces. Loopback default rejects access via the other interface.
- Native Windows Babashka passed hello/Reference query against Windows Runtime on
  loopback and `192.168.0.105`, with clean shutdown; missing remote opt-in is rejected
  before socket use. See `bb-tcp-results.json` and `bb-tcp-check.py`.
- This does not prove physical two-PC LAN or Windows Workbench → Linux GUI E2E.

The deployment guide documents explicit interface selection, trusted-source
firewall allowlisting, lack of raw TCP TLS/authentication, and authenticated WSS
for Internet use. No firewall/system/Tuna configuration was changed.

## Verification results for this candidate

| Check | Result |
| --- | --- |
| Windows `cargo test --workspace` | 857 passed, 0 failed, 13 ignored |
| Linux `cargo test --workspace --no-fail-fast` | 857 passed, 1 failed, 13 ignored |
| Windows Clippy `--workspace --all-targets -- -D warnings` | PASS |
| Linux equivalent Clippy | FAIL: unchanged Windows-only `OwnershipError::{AlreadyActive, Platform}` are dead code on Linux |
| `cargo fmt --all -- --check`, `git diff --check` | PASS |
| Six new reservation regressions | 10/10 Debug runs per OS, 60 test evaluations each |
| Linux isolated `disconnect_never_owns_start_stop_and_reconnect_rebuilds_current_state` | 20/20 Debug repeats; an earlier full run failed with ingress exhaustion while waiting for stop |
| Windows/Linux Release workspace gates | Not run for this incomplete candidate after the architecture stop; previous-version PASS does not qualify it |

The Linux command uses fresh `CARGO_TARGET_DIR=/root/lab-runtime-target/m18-implementation-20261009`;
default test parallelism is retained. `--no-fail-fast` only collects all failing
targets. Windows totals exclude two nested Workbench child-test summaries already
counted by the parent. Linux Workbench passed 231 tests / 11 ignored; Windows
Workbench passed 232 / 11 ignored.

Test-only fixture changes coordinate existing process-wide ManagedExecutor users
within each integration-test process and create the intentionally invalid journal
parent only after initial load/hello. They preserve all semantic assertions. These
two diagnosed platform issues did not fail in the completed Linux diagnostic run.

The initial full Linux run also failed `recorder_api::disconnect_never_owns_start_stop_and_reconnect_rebuilds_current_state`
with `recorder ingress capacity exhausted`. It then passed 20 isolated runs and the
next complete run. This remains an unresolved saturation/scheduling concern, not
evidence that the failure was harmless or fixed.

### Process matrix (Debug candidate, intermediate build)

| Controller / subscriber | Windows | Arch WSL2 |
| --- | --- | --- |
| TCP / TCP | 3/3 healthy | 3/3 healthy |
| TCP / WS | 3/3 healthy | 3/3 healthy |
| WS / TCP | 3/3 healthy | 2 healthy, 1 reservation failure |
| WS / WS | 3/3 healthy | Not reached after failure |

All 20 healthy processes retained terminal audits, had no gaps, reported a complete
Recorder flush, passed SQLite integrity/FK checks, shut down cleanly and
reopened the same DB with the original prefix preserved and no retune replay.
Exact zero-credit assertions are in the reservation regressions; the process
harness did not sample a separate post-flush credit counter.
They use intermediate Debug executables; this is diagnostic evidence, not a final
Release acceptance matrix. Exact executable hashes are in each matrix JSON.
The prior 53/80 failure matrix used a different Release candidate and cannot be
treated as a controlled statistical comparison with this smaller Debug run.

Full commands and per-test logs are retained under the evidence directory. Main
commands: `cargo test -p lab-runtime --lib reference_completion`,
`cargo test -p lab-runtime --lib reconnect`,
`cargo test -p lab-runtime --test runtime_startup --test tcp_lan`,
`cargo test --workspace`, and the Linux `--no-fail-fast` diagnostic above.

## Remaining acceptance limits

No new active Recorder + Windows GUI + Babashka end-to-end acceptance or visible
graph proof is claimed. No new Linux Release build/package or protected Tuna E2E
was performed; the existing Tuna configuration and user Runtime were not altered.
Physical two-PC LAN, real USB/RS-485 and bare-metal behavior remain untested.
Prior successful client transport tests do not waive these current Recorder gates.
No broader admission/recovery/authority change will be implemented without the
requested external review.
