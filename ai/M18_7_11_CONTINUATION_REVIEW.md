# M18.7–M18.11 continuation: Recorder and LAN review gates

**Historical review record.** The Recorder remediation was subsequently approved
and committed; current M18 scope, verification and remaining review limits are in
[M18_CONSOLIDATED_REVIEW.md](M18_CONSOLIDATED_REVIEW.md). Earlier restrictions,
HEAD/status claims and failed gates below describe their dated diagnostic stage.

Date: 2026-10-09. Branch: `feature/m18-distributed-workbench`.
Committed base: `b546354bddf035c15f78f1f1d1ea1d22eb84ac76`.
Status: architecture decisions required; not M18 acceptance or implementation of
the proposals below. No merge, push, tag, Release, firewall or Tuna changes.

## Implemented writer correction remains valid

The existing uncommitted correction in `recorder/worker/storage_loop.rs` retains
the maximum original submission time of the committed FIFO prefix. It advances
only after a successful SQLite transaction. Facts use the maximum member time,
not the last member's time; operation, annotation and probe receipts use the same
high-water helper. Original stored submission timestamps are unchanged.

The exact prefix is `persisted_through_sequence`, not a time range. An earlier
timestamp in a later, uncommitted submission is not confirmed by this watermark.
Cumulative committed record/byte/group counts release only their exact outstanding
credits. The owner still rejects future record IDs, future submission times,
regressing receipts and excessive released credits. Activation generation is
receipted separately; a timestamp cannot authorize a future generation. Start,
Stop, Finish, SQLite schema and ordinary limits are unchanged.

The five receipt regressions cover out-of-order facts, mixed original audit times,
batch maximum, gap receipt, exact credit release, held terminal, generation,
stop/flush/reopen and genuine four-group saturation. The deferred-FK COMMIT test
also checks that an actual COMMIT failure confirms/releases nothing uncommitted.
See [the original proof and evidence](M18_RECORDER_RECEIPT_FIX.md).

## Genuine saturation: causal chain

Sources in the current working tree:

- `application.rs:492`: SessionStore admission precedes Recorder intent.
- `application.rs:510`: accepted operation audit is submitted independently.
- `host/lifecycle.rs:249`: command observation and fact admission use the original
  host time, which can legitimately precede the audit's submission time.
- `host/recording.rs:531`: audit recording does not edit the domain result.
- `application.rs:596`: SessionStore commits the truthful terminal domain outcome
  before attempting terminal Recorder audit admission at 603.
- `recorder/worker/ingress.rs:43` and `:112`: fact/audit admission counts each
  separate message as one ordinary group; the fifth unreleased group latches gap.
- `recorder/worker/storage_loop.rs:178`: Facts coalescing waits at most 100 ms from
  the original queue instant. An Operation interrupts coalescing but does not make
  SQL commit/owner receipt synchronous.

An observed eligible burst is:

| Owner step | Outstanding ordinary groups before any release |
| --- | ---: |
| Two acquisition/domain fact groups already pending | 2 |
| Accepted Reference retune audit | 3 |
| Authoritative changed Reference fact | 4 |
| Terminal retune audit attempts admission | 5 requested; only 4 allowed |

The fifth attempt has no admitted record identity. It schedules the reserved gap
seal, makes Recorder failure sticky and invokes Required fail-closed handling.
The accepted audit and Reference fact were already in FIFO and can subsequently
commit. Missing terminal audit is therefore consistent with the committed prefix
and explicit gap, not evidence that the domain command was unexecuted. Changing
an already completed Application result to failed would misrepresent execution.

The prior matrix reproduced this in 53/80 fresh Release processes: Windows 13/40,
Arch WSL2 40/40, across all four TCP/WS controller/subscriber combinations.
All 80 retained the accepted retune and Reference revision 2; only the 27 healthy
recordings retained its terminal audit. All 80 passed SQLite integrity/FK and
same-database reopen/prefix preservation. Failed Recorders eventually released all
charged groups, persisted their gaps and reported unsuccessful Recorder flush on
shutdown. This is genuine saturation, distinct from lost receipts.

This scenario uses two clients, normal virtual-demo acquisition and one retune,
not a mutation flood. Immediate retune after recording/subscription creates a
burst; slower ordinary operation is not equivalent evidence. The five-minute
session retained 6,086 measurements and both retune audits without gaps. That
success does not waive the reproducible burst. WSL scheduling and FULL/WAL commit
latency influence frequency, but SQLite quota was not exhausted and neither
transport framing nor client count changes Recorder's four-group limit.

### Reconnect is a separate publication-budget problem

`service/configuration.rs:727` reserves only one lifecycle group. Resource-scoped
reconnect passes `global_quiesced=false`; other acquisition continues.
`host/configuration.rs:552` crosses Core rebind and produces authoritative changed
binding/baseline evidence. Compatibility probes and subsequent host service can
produce further facts. The lifecycle reservation does not reserve those groups.
`service/configuration.rs:820` services the host while awaiting lifecycle durability;
that progress may generate ordinary facts and exhaust the remaining credits.

The instrumented Linux failure observed four groups, five records, 6,148 bytes,
persisted prefix 7, activation generation 2, `first_missing_fact=5`, and reconnect
stage `LifecycleDurability` after Core rebind and one replacement open. Generation
2 was already receipted; nevertheless Recorder Failed is not valid successful
Required coverage. `service/reconnect.rs:457` truthfully fails and retires the
replacement. The binding generation cannot be rolled back or blindly retried.
Evidence: `target/m18-recorder-fix-20261008/linux-candidate-lib-repeat-1.log`.

## Recorder contract decision and proposed minimum

The present rejection is consistent with the documented contract: nonblocking
bounded producers, four outstanding causal groups, sticky gap, Required fail-closed,
and Application completion distinct from audit durability. It is an operational
M18 blocker under normal bursts, not an SQLite corruption or receipt correctness
defect after the writer correction.

Reducing the coalescing timeout cannot prove adequate capacity under a held writer.
More credits, synchronous disk waits on the owner, dropping audits, replaying the
mutation, or rewriting its terminal outcome are not proposed remedies.

The minimum implementation candidate to review is a **bounded completion causal
group for synchronous Reference mutations**:

1. Keep the original accepted audit and SessionStore identity/outcome rules.
2. Before the Reference side effect, reserve one of the existing four group slots
   and the proven record/byte upper bound for its authoritative Reference facts
   plus terminal audit. Do not allocate record IDs before FIFO transfer.
3. Capture those exact original records within the serialized owner turn and
   transfer one bounded completion group; preserve their original timestamps,
   identities and causal order. Commit the group atomically and release its exact
   credits only through the post-COMMIT receipt. The illustrated burst then uses
   four groups instead of requiring five.
4. On unsuccessful pre-side-effect reservation, do not execute the Reference
   mutation. Use an explicitly reviewed existing failure mapping and preserve
   truthful gap/failure evidence for the already accepted intent. Do not silently
   convert capacity failure into successful admission or consume identity again.
5. Maintain actual overload/SQL-error/Required-failure behavior. Do not claim that
   this small bundle protects arbitrary asynchronous operations or unrelated
   acquisition against a permanently blocked writer.

This is a proposed change to Recorder causal grouping/reservation, not an approved
writer-only patch. Before implementation the bound must be derived from every
fact emitted by the Reference command/observation path, including allocation and
encoding scratch. Core's global 256-record/256-KiB outbox is not itself proof of a
one-Reference completion bound. SQL must support facts and an audit in the same
transaction without changing schema/checkpoint meaning. Cancellation, short/failed
domain paths and reservation ownership must be explicitly tested.

Reconnect additionally needs a reviewed publication budget for rebind/probe facts,
not just the existing lifecycle reservation. Keep probe-before-activation FIFO,
resource quiescence, generation advancement and Required durable reopening. Do not
generalize the synchronous Reference bundle across a potentially two-second
reconnect: a bounded resource-specific reservation/drain design must account for
all participating facts and continued unrelated acquisition. A global pause of
acquisition or post-rebind rollback is not authorized by this proposal.

An alternative is to defer more terminal audits in the existing 64-entry pending
operation tracker (currently used narrowly for safe controller-pause terminals).
That alone does not reserve Reference/rebind facts, and retained terminal payloads
need exact memory/credit accounting plus gap/stop/shutdown treatment. Broadening it
without that proof would change the overflow contract and hide evidence. It is not
implemented as a quick workaround.

### Deterministic regression acceptance plan after review

- Hold the writer before SQL, establish two pending acquisition groups, execute one
  retune, and prove one accepted audit, exact Reference revision/target, exact
  terminal audit, unchanged group/record/byte bounds and no premature durability.
- Hold after SQL/before receipt; a reservation must remain charged until the exact
  receipt. Duplicate receipt must release nothing a second time.
- Exhaust all real slots before mutation: no side effect, truthful selected
  admission/failure mapping, no missing/unaccounted reservation or identity hole.
- Inject real COMMIT failure: no prefix/watermark/credit release for the failed
  transaction; preserve the prior committed accepted intent and explicit failure.
- Exercise every cancellation path and Required/BestEffort policy; stop/flush must
  account for retained records, original timestamps and pending operations.
- Hold resource reconnect at pre-rebind, probe, post-rebind and post-SQL stages;
  prove exact generation/provenance order, resource quiescence, no premature Good,
  no rollback/rearm, and honest post-rebind failure under real overload.
- Repeat the 80-process matrix, concurrent workspace tests and active two-client
  recording. A single short healthy process is insufficient acceptance.

Existing tests cover genuine exhaustion (`recorder_backpressure`), one lifecycle
reservation (`recorder_reload_budget`), held shutdown (`recorder_shutdown`), and
post-rebind failure (`host/reconnect_recorder_ordering_tests`). They do not prove
complete ordinary-operation capacity when acquisition groups are already pending.

## Linux test-only findings and next changes

These are distinct from production saturation:

- `managed_executor.rs:124`: one process-global two-worker pool. Concurrent
  ServiceHost fixtures compete for it; `service.rs:1064` has a two-second startup
  bound, before Recorder opens. Original b546 history-api also failed 6/9 under
  default concurrency. Proposed test-only fixture guard serializes ownership of
  that shared resource inside each test executable, not the workspace test runner
  or unrelated tests. It must cover complete executor lifetimes and cleanup.
- `lab-workbench/src/client/worker.rs:3520`: a regular-file journal parent returns
  Windows NotFound but Linux NotADirectory. Linux emits its startup journal problem
  before Hello; `wait_for(Hello)` discards it, and the mutation correctly rejects
  without emitting a second warning. Move fixture obstruction creation after Hello
  so the test deterministically exercises write failure, retaining warning,
  rejection and no-wire assertions. No production change is needed.
- `service.rs:3015`: post-rebind Recorder saturation is production overload, not a
  fixture completion notification race. Do not hide it by serial test execution.

These test changes are authorized but remain unimplemented at the architecture
stop. No test is disabled or weakened, and no fixed sleep is introduced.

## Direct TCP LAN: concrete separate review proposal

Current config validation requires `server.host == "127.0.0.1"`
(`configuration.rs:935`); config CLI has fixed shape (`service.rs:78`) and startup
chooses loopback (`service.rs:913`). Virtual-demo alone accepts explicit `--bind`.
Raw TCP reactor has no peer-loopback/HTTP Host restriction; Workbench has both
endpoint-parser and worker-spawn loopback checks. The bundled TCP Babashka example
hardcodes loopback. Actual `ss` and Windows-to-WSL-IP hello evidence are in
[the TCP audit](M18_TCP_LAN_AUDIT.md); they do not prove config-mode or physical LAN.

Preferred design for review: a **trusted startup-only TCP override**, leaving the
frozen deployment TOML/DTO/reload meanings unchanged:

```text
lab-runtime --serve --config /etc/lab-runtime/runtime.toml \
  --bind 192.168.0.50 --allow-remote-tcp
lab-workbench --endpoint 192.168.0.50:8765 --allow-remote-tcp --observe
```

These are proposed CLI shapes, not working commands in the current implementation.
Port stays configured by the deployment. Default and absence of explicit opt-in
remain loopback. Bind must select an assigned numeric unicast IPv4 address;
wildcard/multicast/broadcast remote binds are rejected in this proposed mode.
Config reload/apply cannot remotely expose a listener. Listener readiness, shutdown,
capacity, framing, mutation identity and one-worker recovery remain unchanged.
Both Workbench parser and spawn guard must use the same explicit endpoint policy.
The bundled TCP example gains HOST plus an explicit plaintext remote opt-in; no
external `thermal-plant-analysis` file is changed. Workbench API stays loopback.

Runtime WebSocket bind/Host/Origin remain unchanged and loopback-only. Tuna uses
that loopback upstream with verified WSS and mandatory X-Token at the tunnel.
Direct LAN TCP has no encryption, principals or server read-only authorization.
Deployment must explicitly trust the LAN and restrict TCP ingress to nominated
Windows client IPs; prohibit WAN/router forwarding. A private network/firewall is
the proposed trust boundary, not Runtime authentication or the --observe UI flag.

M12 consolidated review lines 139–143 requires a separate threat-model/security
review for non-loopback binding. The user likewise explicitly requires review
before changing config-mode security boundaries. Approve or reject this narrow
trusted-LAN exception before implementation; do not silently relax TOML host,
WebSocket Host checks or all existing clients.

Acceptance after approval: default/config rejection tests, explicit opt-in tests,
same TCP parity/recovery suite, selected-interface listener checks, two independent
clients, firewall allowed/denied peer tests, restart, and real two-physical-PC
reachability. WSL NAT evidence is recorded separately. No obligatory proxy/Caddy.

## Validation and operational limits

Fresh targeted results from this continuation and their exact commands are kept in
`target/m18-continuation-20261009/`; see `validation-summary.json` there. Each
selection contains 38 tests: receipts 5, backpressure 11, reload budget 4,
transactions 18. Test parallelism is unchanged.

The first Linux invocation against the old shared Cargo target failed four receipt
regressions. That linked binary lacks `confirm_submission`; the isolated fresh
Debug binary contains it, uses the identical working-tree source hash and passes
all 38 tests. The old host-target executable is stale evidence, not a reason to
alter assertions or Runtime. Preserve both logs and use isolated target directories
for future baseline/candidate comparisons. Do not clean existing diagnostic builds.

Prior Windows workspace Debug/Release each passed 847 top-level tests, 13 ignored.
Prior complete Linux diagnostics were Debug 837 passed / 11 failed / 13 ignored;
Release 846 passed / 2 failed / 13 ignored. They remain red; targeted success is
not a new full gate. No fresh full workspace acceptance is claimed at this stop.

Prior two-client session proves Runtime observations, presentation API/model,
recording and reopen. White GUI captures do not prove a painted graph. Protected
Tuna E2E, config-mode direct LAN, physical two-machine acceptance and native GUI
rendering remain outstanding. Historical SQLite lock reports are not all closed
by fixing one diagnostic Python connection lifetime. Tuna configuration/secrets,
the user's existing Runtime and unrelated files remain untouched.
