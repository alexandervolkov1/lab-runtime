# M18: distributed Workbench transport audit

**Historical review record.** The Recorder remediation was subsequently approved
and committed; current M18 scope, verification and remaining review limits are in
[M18_CONSOLIDATED_REVIEW.md](M18_CONSOLIDATED_REVIEW.md). Earlier restrictions,
HEAD/status claims and failed gates below describe their dated diagnostic stage.

Status: Workbench/client implementation ready for review; M18 acceptance blocked.
Evidence date: 2026-10-08. No merge, push, tag or release publication.
Base: approved GUI recovery commit `c87890be2e3cac3a12300a02fa42673743ebe307`.

## Owners and surfaces

Runtime remains the only experiment, device, controller and Recorder authority.
One Workbench worker owns hello, scope, message IDs, mutation sequence, subscription,
recovery journal, quarantine and bounded reattach. One main-thread dispatcher owns
Workbench observations and presentation revisions. Independent Clojure/Babashka
processes connect directly to Runtime for experiment automation, and to the existing
IPv4-loopback Workbench API for presentation. Measurements flow from Runtime to
Workbench; scripts do not relay or fabricate observations.

## Existing implementation and minimal seam

- `client/worker.rs` has one synchronous worker with nonblocking TCP I/O and a 2 ms
  idle wait, bounded mailboxes, eight pending exchanges and an absolute reattach
  deadline. Transport replacement must not copy or bypass this state machine.
- `client/framing.rs` owns NDJSON decoding and bounded partial output. A private
  transport adapter can own TCP/NDJSON or WebSocket text framing, while delivering
  the same bounded JSON envelopes to `handle_incoming` and preserving send evidence.
- Runtime uses Tungstenite 0.30.0 for bounded RFC 6455 framing. Workbench can reuse
  that exact library with verified TLS support. Ping/Pong, fragmentation and Close
  stay below Application messages. The adapter owns finite handshake, partial-message,
  write and close deadlines and per-turn byte budgets.
- DNS and TLS cannot run on the GUI thread. DNS resolution must have a finite
  deadline rather than an unbounded standard-library resolver in the sole worker.
- Before M18, `main.rs` and `gui/mod.rs` accepted a numeric TCP address. WS/WSS endpoint,
  exact Origin, explicit insecure remote-WS opt-in, trusted CA and X-Token sources
  belong to client transport configuration. URL credentials/query secrets and TLS
  verification bypass are forbidden. Diagnostics expose categories, not requests,
  headers, secret values or reflected HTTP bodies.
- `gui/app.rs`, client submission and the existing dispatcher can enforce an explicit
  local observation policy. Presentation and queries remain available; GUI mutation
  and Exact Retry controls cannot submit. This is not Runtime authorization.
- Existing `clients/babashka-smoke` scripts are bounded loopback TCP wire examples,
  not a reusable remote client. M18 adds a bounded explicit WS/WSS scenario using
  the existing Application messages and the existing Workbench revision precheck.

## Invariants

Transport does not alter operation names, envelope/DTO schemas, admission, `{scope,
seq}`, exact payload, Recorder, central OutputAuthority or Runtime scheduling.
Mutation, status and Exact Retry are not replayed on reconnect. Ambiguous work retains
evidence; quarantine is not cleared on new hello or new scope. New scope requires
explicit action after instance change, including empty history. Fresh requires the
existing complete authoritative rebuild, not a successful socket or hello.
Explicit Disconnect remains fenced/coalesced and never reconnects automatically.
GUI/script loss does not stop Runtime or Recorder. Workbench API stays loopback-only
and uses the same dispatcher/client, without a second Runtime subscription owner.
One unresolved output frame cannot be re-enqueued after a partial WebSocket write.
Queues, protocol buffers, work per turn, DNS, handshake and shutdown remain bounded.

## Runtime LAN boundary requiring review

`ServiceHost::bind_websocket` explicitly binds `127.0.0.1`; the reactor constructs
the expected Host as `127.0.0.1:<port>`. `WebSocketOptions` carries only a port and
Origin allowlist. M12.3 section 12 requires separate review for non-loopback binding,
authentication, TLS, proxy/Host handling, deployment and DoS limits. Origin is a
browser-origin policy, not authentication. The current TCP profile already supports
explicit LAN bind, but that does not expose the WebSocket listener.

Direct LAN WebSocket cannot be made true by a Workbench-only change. The proposed
separate review is an explicit WS bind setting with loopback default, explicit
insecure-LAN opt-in, a finite exact Host policy and unchanged reactor/client bounds.
Neither accepting arbitrary Host nor an obligatory user bridge is an acceptable
substitute. Runtime production remains unchanged while client/proxy work proceeds.

## Tuna trust boundary and evidence

Tuna terminates public TLS and forwards HTTP Upgrade to the Runtime loopback WS
listener. Its upstream Host must be rewritten explicitly to the listener Host.
X-Token/key-auth restricts tunnel access; it does not create Runtime roles or
read-only server authorization. The operator and Tuna service remain trusted.
Workbench verifies the endpoint certificate and hostname and never downgrades.
Secrets come from named environment variables or bounded local files, never URL
parameters, command-line secret values, journals or debug/error output. Request
inspection and secret-bearing debug logging must be disabled at the tunnel.

User-confirmed endpoint: `wss://fresh-hedgehog-9022.ru.tuna.am/application/v1`.
Upstream: `http://127.0.0.1:8766`, Host rewrite `127.0.0.1:8766`.
Existing configuration must not be changed without separate consent. Until key-auth
is verified, public mutation E2E is not authorized. Local authenticated TLS proxy
tests prove adapter mechanics, not real Tuna or physical LAN acceptance.

## Required evidence before M18 acceptance

TCP/WS/WSS parity, bounded handshake/partial frames/control/shutdown, correct and
missing/wrong-token outcomes, untrusted-certificate and hostname rejection, independent
script mutation plus authoritative GUI rebuild, transactional presentation updates,
no duplicate mutations, active Recorder continuity, restart/reconnect/quarantine,
late replies/event gaps/caller loss and clean shutdown. Repeated concurrent tests,
workspace Debug/Release, warnings-denied Clippy, fmt, Windows GUI Release, Linux
Runtime builds and Arch WSL2 smoke are required. Real protected Tuna and a separately
reviewed LAN listener are outstanding acceptance gates, not assumed PASS.

## Implemented client seam

`client/endpoint.rs` validates TCP/WS/WSS configuration and owns sensitive handshake
headers. `client/transport.rs` adapts bounded envelopes and partial output below the
existing worker. Tungstenite 0.30.0 owns RFC 6455; Rustls checks OS/private CA trust
and hostname. Hickory/Tokio resolution runs on a temporary current-thread executor
inside the same worker, with no detached resolver or second Application owner.
DNS, TCP, TLS and Upgrade share the existing finite connect-attempt deadline.

After Upgrade, work remains nonblocking: 8 KiB input/output and eight messages per
turn; 16,383-byte Application messages; 32 KiB retained output; 8 KiB Upgrade input;
finite partial-frame/control-output deadlines; at most eight resolved addresses.
Close is bounded to 200 ms and cannot flush queued Application mutations across an
explicit disconnect fence. Ping/Pong and fragmented text use library framing.
First-byte evidence survives partial-write errors, including the existing TCP path;
an ordered valid response prefix survives a following abrupt WS EOF. No ambiguous
mutation is re-enqueued by a transport flush or reconnect.

`--observe` is immutable local client policy. The GUI disables operator/Exact Retry
actions and the client rejects mutation/retry submissions before command-ID or
mailbox admission, including mediated Workbench API calls. Queries, subscriptions,
Check Status and existing presentation operations remain available. This deliberately
also rejects `history_read`, which the frozen API classifies as a mutation/work job;
recent history and existing history-page queries remain read-only. It creates no
Runtime authorization role and changes no mutation/admission registry.

GUI plots now render the already accepted PresentationDocument from authoritative
observation buffers. Previously a script could commit a plot through the existing
API, but the main view rendered only its selected entity. This local presentation
defect was fixed without adding an API operation or relaying scientific data.

`clients/babashka-smoke/distributed.clj` is a bounded explicit virtual-demo example,
not an autonomous experiment SDK. It opens its own scope, performs one retune,
checks accepted/completed plus a committed snapshot, and separately commits one
presentation revision through loopback. Its transport failure never retries the
mutation. Tokens are supplied through a named environment variable; malformed or
duplicate configuration fails before connecting. Workbench supports a bounded
token file as well. Tokens never enter Application JSON or recovery/presentation.
Tungstenite raw HTTP request tracing is disabled through `log/max_level_info`.

## Verification results

All final workspace gates retained their ordinary test parallelism:

| Check | Result |
| --- | --- |
| `git diff --check` | PASS |
| `cargo fmt --all -- --check` | PASS |
| `cargo clippy --workspace --all-targets -- -D warnings` | PASS |
| `cargo test --workspace` | 842 passed, 13 ignored, 0 failed |
| `cargo test --workspace --release` | 842 passed, 13 ignored, 0 failed |
| Workbench `--test-threads=16`, Debug and Release | 230 passed, 11 ignored per run; 10/10 each |
| Five distributed tests, Debug and Release | 5 passed per run; 20/20 each |
| Windows Workbench Debug/Release build | PASS |
| Ubuntu 24.04 Runtime Debug/Release GNU x86_64 build | PASS |

Totals exclude nested child-fixture summary lines. No ignored acceptance was made
active or silently counted as passed. Eleven new non-ignored regressions cover
endpoint/token policy, CLI, retained partial output, frame deadlines, transport
parity/observation, authenticated TLS and WS recovery. Existing TCP regressions pass.

The five distributed tests prove:

1. One real Runtime serves equivalent TCP/WS hello, discovery, current/latest
   measurements, oldest-first bounded recent history, Reference state, idle Recorder
   status, bootstrap/subscriptions and operation accepted/completed/failed/status.
   Terminal exact retry is rejected locally without re-executing.
2. TCP and WS observation clients reject mutation/retry without allocating identity
   or changing the Reference; read operations remain available.
3. A local WSS proxy rejects missing/wrong X-Token, untrusted CA and wrong hostname;
   the trusted authorized connection completes hello/discovery/Reference queries.
   Counters distinguish TLS rejection from key-auth rejection. No TLS bypass exists.
4. Fragmented hello with interleaved Ping/Pong works. Loss after mutation send keeps
   exact ambiguous evidence, reattaches the same scope, sends no automatic mutation,
   status or retry, and permits only explicit retry with identical identity/payload.
   Close is observed without shutting down Runtime.
5. Instance change with empty or unresolved history exposes explicit new-scope
   action. There is no automatic scope choice; unresolved evidence survives and is
   quarantined, blocking new mutations after the explicit new hello.

The unchanged Runtime transport-parity suite additionally verifies durable history,
event gaps, cross-transport retained operations, peer pressure and shared shutdown.
Existing TCP late-reply, admission/fencing and recovery regressions exercise the
shared worker. They are not evidence of every fault over a real Tuna tunnel.

## Actual Windows + Arch WSL2 operation

Native Windows Release GUI completed WS hello/discovery/Fresh/live observations
against Arch WSL2. Babashka 1.13.220 independently retuned virtual Reference from
revision 1 to 2 / target 51 and committed presentation revision 3. A native screenshot
shows its `Distributed virtual-demo` temperature plot. No measurement relay or
Workbench-owned mutation journal was created. After GUI close, a direct script
completed another mutation while Runtime remained alive; local Workbench API
availability was therefore not an experiment-lifetime prerequisite.

The actual Windows Release GUI also completed verified WSS through a local
authenticated TLS fixture forwarding to Arch Runtime. A bounded manual fixture
initially expired while preparing the GUI; the properly orchestrated second run
passed. This is local proxy evidence, not real Tuna or physical LAN E2E. Babashka
WS was exercised; actual Babashka WSS remains unverified.

Runtime/systemd start, status, restart, journal and stop were exercised using an
isolated temporary `/run/systemd/system` unit and test DBs. Clean baseline shutdowns
returned exit 0 and released both ports. Failed Recorder shutdowns correctly
reported exit 1, no unfinished workers/transports and no lingering process. The
temporary unit was removed; the user's pre-existing Runtime on port 8765 was retained.

Ubuntu builds use Rust/Cargo 1.95.0 and `--locked --target x86_64-unknown-linux-gnu`.
Runtime production sources are unchanged from `8f7cb98de194205c5ffcaaa9292ea5ce6845e623`.
The rebuilt Linux executable SHA-256 is:

```text
8bcb3f1a4da4bb4efecd9280bea75b263b31fa38636732509b54afa38a88d4b4
```

Existing Linux release artifacts are preserved; no M18 release was published.

## Recorder operational blocker: evidence and limits

Required recording failed on both WSL-mounted NTFS (23.5 seconds) and native ext4
(30.1 seconds), with `recorder ingress capacity exhausted`. A separate ext4 baseline
without Workbench recorded successfully for more than 90 seconds, then completed
explicit recording_stop and clean process shutdown. Its 4,795 measurements survived
reopen/restart.

A narrower reproduction removes GUI and heavy compilation: two direct clients,
TCP recording_start, WS subscribe, TCP Reference query and one reference_retune.
At boot `aeb2b54e4dfde4d9407c11dcac31d3ff`, recording became active at monotonic
1307 ms and failed at 1375 ms. The unchanged accepted Linux executable reproduces
the failure; the Workbench adapter is not a necessary cause.

A coherent SQLite backup includes the WAL. It contains start accepted/completed
(record sequences 4/5), measurement/Reference facts (6/7), another measurement (8),
retune accepted (9), its Reference fact (10), then a gap (11): first_missing is null,
known_count is 1, last_confirmed is 10. The retune's terminal operation audit was
not admitted. The failed boot remains unsealed; it must not be described as a clean
recording shutdown. Integrity and foreign-key checks pass; all 4,795 previous rows
survive, and two new measurements persisted, but healthy continued recording FAILS.

Source analysis locates this failure in operation audit admission:
`RecorderWorker::try_admit_operation` charges one whole ingress group per phase,
and `HostCore::record_operation` fails required recording if admission fails.
Credit includes uncommitted/in-flight SQL work. The hard group limit is four;
the SQLite writer coalesces fact groups for up to 100 ms and releases credit only
after commit receipts. The persisted sequence is consistent with two pending fact
groups plus retune acceptance and its fact group filling four credits before the
terminal audit. These tiny records cannot exhaust the 1,024-record/4 MiB limits.
The exact receipt/interleaving at the rejection was not instrumented; do not claim
a proven erroneous credit release or a GUI production race. The observed behavior
is a production capacity/operability blocker with the specified fail-closed outcome.

No Recorder limit, scheduling, durability or Application semantics was modified.
The minimum next diagnostic is a deterministic writer-barrier reproduction of the
ordinary operation burst with exact credit/receipt assertions. Any proposed change
to coalescing, grouping or terminal credit must be reviewed against M7 bounds,
required recording and operation audit guarantees before implementation. Increasing
limits or adding sleeps/retries is not a justified remedy.

## Outstanding acceptance and review

- Direct physical/LAN WS: blocked by the reviewed loopback/Host policy boundary;
  explicit bind/Host-policy proposal above needs external review. No mandatory bridge.
- Protected real Tuna E2E and agent stop/recovery: pending. Read-only local inspection
  found no Tuna process in Windows, Ubuntu or Arch, no key-auth marker in the local
  configuration and no relevant environment variable. Missing/wrong-key verified-TLS
  Upgrade probes both returned 404; no Application message was sent. This does not
  prove configured authorization. Installed CLI help confirms key-auth/X-Token and
  Host/inspection flags; existing configuration was not changed.
- Active required Recorder with simultaneous clients: FAIL, as detailed above.
- Actual Babashka WSS, two simultaneous clients over a real protected tunnel,
  Tuna downtime/recovery and the full physical two-computer fault matrix: not verified.
- The three previously identified admission races in ignored process acceptances
  remain untouched: `real_runtime_reference_reconnect_reconcile_and_replay`,
  `real_runtime_exact_retry_retained_outcome_never_reexecutes`, and
  `real_runtime_exact_retry_outcome_unknown_never_reexecutes` (old main.rs lines
  1480/1620/1718; line numbers moved after CLI additions).
- An earlier `sqlite: database is locked` lacks enough original context for a firm
  diagnosis. Live raw SQLite access conflicts with the accepted exclusive connection;
  this possibility does not explain the current ingress failure. Stopped coherent
  backups pass integrity checks.
- New TLS/DNS dependencies need refreshed release inventory/license review before
  packaging Workbench. Existing release legal blockers are not waived.
- WSL2 is not physical mini-PC, USB/RS-485, power-loss or bare-metal acceptance.

Evidence, transcripts, compiler/build logs, repeat logs, coherent DB backups,
screenshots and the review patch live in ignored
`target/m18-distributed-workbench-20261008/`. Private fixture keys are ephemeral;
only public test certificates and a synthetic access-key file are on disk.
M18 is not complete and no preview/public release acceptance is claimed.
