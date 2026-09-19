# M11 failure, recovery and bounds matrix

This is the engineering source for the developer-preview gate. It records the
post-M11 software contract; it is not a user manual or a claim of production or
hardware certification. Source and tests remain authoritative.

## Guarantee legend and evidence boundary

- **GUARANTEED** means the behavior is an explicit invariant with deterministic
  implementation and test evidence inside the stated software/process boundary.
- **BEST EFFORT** means Runtime attempts the behavior, but loss or failure may occur
  without violating experiment correctness.
- **NOT GUARANTEED** means the physical/world state cannot be inferred from the
  available evidence.

These distinctions are normative:

```text
requested != authorized != send_started != ACK != readback != physical_effect
ACK != readback
readback != physical effect
process shutdown != proof hardware physically became safe
fact admitted != fact durably committed
archive structurally readable != experiment semantically complete
```

## Consolidated failure and recovery matrix

`Explicit rearm` refers to the controller/output lifecycle, not merely restoring a
resource. A dash means the failure has no controller rearm consequence.

| Failure | Detection | Authoritative Runtime / Recorder state | Measurement consequence | Controller consequence | Physical-output consequence | Automatic retry? | Recorder consequence | Application consequence | Recovery | Explicit rearm? | Guarantee | Regression / evidence |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| COM open failure | open worker result/deadline | candidate never becomes Ready; active generation is not replaced | configured signal remains never-observed/unavailable | no authority gained | no output generation installed | bounded open attempts only | bounded resource/configuration fact where lifecycle reaches Runtime | structured transport/startup failure | correct device/settings and explicit reconnect or restart | yes if prior controller faulted | **GUARANTEED** | `configuration_startup`, `configured_physical`, service reconnect tests |
| Read silence/timeout | transaction deadline | resource reaches truthful recovery/offline state | one `Unavailable(Transport)`; no Good sample | unusable input fails active control through normal policy | authority revoked; safety requested when applicable | READ only after proven clean recovery; production COM otherwise requires reconnect | unavailable fact; no fabricated sample | resource/current show unavailable/offline | explicit reconnect | yes | **GUARANTEED** | configured Metakon fault matrix |
| CRC failure | strict codec validation | current generation remains truthful; request terminal | no corrupt Good commit | same as unavailable input | no evidence/authority fabricated | no frame reuse or hidden retry | unavailable fact | bounded transport-unavailable projection | explicit reconnect | yes if failed | **GUARANTEED** | `configured_physical` CRC oracle |
| Truncated/malformed frame | strict length/field validation plus deadline | terminal transport result; queue drains | no partial or later-frame Good commit | same as unavailable input | no compatibility/output evidence fabricated | no carry-over into a later transaction | unavailable fact | bounded failure | explicit reconnect | yes if failed | **GUARANTEED** | `configured_physical` short-frame matrix |
| Disconnect | transport error/recovery deadline | resource Offline; generation not silently replaced | exactly one logical unavailable transition | active controller fails closed | lease revoked; safe obligation remains truthful | no unbounded/parallel replacement; explicit reconnect | resource and measurement lifecycle remain ordered | resource/current show Offline/unavailable | explicit reconnect | yes | **GUARANTEED** | M8 reconnect and M11.2 configured matrix |
| Incompatible compatibility probe | typed channel probe | replacement stays quiesced/offline; no Core rebind | no ordinary acquisition released | no authority acquired | no output adapter released | no | bounded reconnect failure | structured operation/resource failure | corrected deployment plus explicit reconnect | yes if recovering a faulted controller | **GUARANTEED** | configured reconnect/probe tests |
| Stale generation completion | correlation and binding-generation check | discarded; current generation unchanged | no stale measurement commit | no lifecycle change | cannot confirm or send for current authority | no | no false fact | no false current state | current generation continues | no | **GUARANTEED** | M3/M8 generation fencing, configured matrix |
| Transport shutdown `Pending` | `TransportShutdown` state | remains nonterminal and receives bounded owner turns | ordinary work is quiesced | shutdown/safety policy remains authoritative | no false retired/safe claim | bounded polling until deadline | shutdown facts/status stay truthful | terminal shutdown result reports incomplete cleanup if needed | cooperative retirement or process exit | no | **GUARANTEED** | `com_recorder_shutdown`, serial shutdown tests |
| Truly hung transport | shutdown deadline | unfinished resource remains visible | no further trusted sample | control cannot rely on it | physical state remains uncertain | no infinite wait | terminal status records incomplete cleanup where available | unsuccessful finite shutdown | process restart; external inspection | yes before later output | Runtime bound **GUARANTEED**; physical state **NOT GUARANTEED** | serial stuck-worker and runtime shutdown suites |
| Never-observed input | absence of a committed sample | signal is never-observed | no cached value promoted | cannot warm/start normal control | no lease/output | normal polling only | truthful quality/lifecycle | current projection says never observed | obtain a fresh Good sample | normal explicit lifecycle still applies | **GUARANTEED** | measurement/controller startup tests |
| Unavailable input | committed quality/failure | signal current is Unavailable | last Good is not treated as current usable input | active controller fails; lease revoked | safe transition requested | source-specific acquisition recovery only | unavailable fact | generic current/event projection | repair source/resource, then explicit controller recovery | yes | **GUARANTEED** | controller quality, configured fault, Recorder quality tests |
| Stale Good-looking cached input | monotonic freshness deadline | value remains historical but unusable | no new Good commit | controller fails on stale input | lease revoked; safe transition | no synthetic refresh | controller/output lifecycle fact | status shows failed/no lease | fresh measurement plus explicit recovery | yes | **GUARANTEED** | controller freshness/timing tests |
| Generation replacement/late old sample | generation fence | new generation sole current binding | old completion discarded | old authority unusable | old lease/proposal cannot pass final check | no | no false new-generation fact | current generation remains truthful | normal new-generation acquisition | yes if controller was faulted | **GUARANTEED** | M3/M8/M11.3 fencing oracles |
| Invalid/non-finite component result/state | Runtime validation | affected component Failed; output invalidated | component signal unavailable/unchanged truthfully | dependent controller sees unusable input | component has no OutputAuthority access | no implicit retry | scoped failure fact | component/current status is failed/unavailable | explicit replacement/reconfiguration | yes only for dependent failed controller | **GUARANTEED** | managed-component validation suites |
| Controller tick gap/non-finite calculation | Core monotonic/config/result checks | controller Failed; lease revoked | source measurement remains independently truthful | Failed | safe obligation requested | no controller auto-restart | controller/output facts | status exposes Failed/no lease | safe evidence, `reset_failed`, `resume`, fresh warm-up | yes | **GUARANTEED** | controller lifecycle/timing suites |
| Lease expiry | final authority time check | lease invalid; epoch/authority state remains truthful | none | cannot renew/send under expired lease | queued proposal rejected before byte zero; safety policy applies | no resurrection | lifecycle/output fact where emitted | status has no valid lease | accepted lifecycle creates a fresh finite lease | yes | **GUARANTEED** | OutputAuthority lease/final-recheck tests |
| Output rejected before `send_started` | final authority/generation recheck or pre-send failure | reservation released; no byte claim | none | ordinary intent is not resurrected | physical effect impossible through this attempt | only existing unstarted safe-delivery policy | exact pre-send outcome fact | output status is rejected/failed | correct authority/resource; new normal lifecycle intent | yes for normal output | **GUARANTEED** inside adapter boundary | M9D authority-recheck tests |
| Ordinary WRITE started, no valid ACK | send-started evidence plus bounded transport outcome | Ambiguous; lease revoked; controller Failed; safe obligation recorded | subsequent resource/input may become unavailable | Failed | original write never resent; physical effect unknown | **No** blind retry | send-started/ambiguous/safe-request facts | output exposes ambiguity and unconfirmed safety | clean reconnect/rebind, safe evidence, explicit recovery | yes | software state **GUARANTEED**; physical effect **NOT GUARANTEED** | M9D and configured output recovery |
| ACK without usable readback | strict ACK then separate read result | ACK retained; outcome Ambiguous/Unavailable; controller Failed | readback is not a measurement substitute | Failed | no verified register state; no blind nonzero resend | no | separate ACK/readback failure facts | output shows ACK and absent readback | safe transition; reconnect if required; explicit recovery | yes | distinction **GUARANTEED**; physical effect **NOT GUARANTEED** | M9D readback timeout |
| Readback mismatch | compared typed register value | mismatch/Failed; controller Failed | none | Failed | original nonzero not repeated; a distinct safe attempt may be allowed because non-safe value was observed | no blind retry | mismatch plus safe lifecycle facts | requested/reported values remain visible | matching safe readback, then explicit recovery | yes | mismatch state **GUARANTEED**; physical effect **NOT GUARANTEED** | M9D mismatch suite |
| Ambiguous safe WRITE | safe send-started without decisive result | `safe_obligation_pending=true`, `ambiguous_safe_resend_blocked=true`, `safe_confirmed=false`, FaultLatched | none | cannot rearm | repeated safety turns do not resend; physical safety unknown | **No** | ambiguity/obligation facts | safe-unconfirmed/fault-latched projection | validated rebind creates a new safe-establishment lifecycle | yes | no-resend **GUARANTEED**; physical safety **NOT GUARANTEED** | ambiguous-safe-write regression 10/10 |
| Safe WRITE ACK plus matching readback | strict ACK and separate reg06 read | authority reaches confirmed safe/disarmed state | none | remains paused/failed until explicit action | register safe value verified; physical effect still not proven | no duplicate write | safe evidence fact | safe-confirmed visible | `reset_failed` then `resume` and warm-up as applicable | yes | register evidence **GUARANTEED**; physical effect **NOT GUARANTEED** | M9D safe transition suites |
| Resource loss while controller active | transport/resource failure | controller Failed, old authority revoked, safe obligation truthful | signal unavailable | Failed | stale generation cannot write | no blind retry | ordered resource/controller/output facts | statuses expose Offline/Failed/unconfirmed safety | reconnect/probe/rebind/safe establishment | yes | **GUARANTEED** | configured output end-to-end oracle |
| Reconnect after output fault | explicit reconnect lifecycle | fresh generation/authority; controller remains Failed | Good acquisition may resume | no auto-rearm from Ready/Good/safe evidence | nonzero count cannot increase without explicit recovery | ordinary acquisition resumes, output does not | reconnect/safe facts | Ready and Failed remain distinguishable | `reset_failed -> Paused -> resume -> Warming -> fresh lease` | yes | **GUARANTEED** | `configured_physical_output` 10/10 |
| Managed worker panic/channel disconnect | completion channel disconnect | slot Dead, no pending correlation; affected component Failed | affected output unavailable | unrelated controller continues | component cannot access output authority | no respawn | scoped failure; unrelated Recorder continues | component status failure | process restart restores fixed capacity | dependent controller only | **GUARANTEED** | managed-executor lifecycle 10/10 |
| Dead managed capacity | explicit Alive/Dead slots | dead slot never counts as pending/healthy; survivor accepts one job | unrelated native acquisition continues | unrelated control continues | none | no replacement explosion | no ingress ownership change | new admission returns Executor when no live slot | process restart | no | **GUARANTEED** | two-dead-worker regression |
| Non-cooperative managed hang | independent 100-ms acceptance deadline | component Failed; worker slot remains honestly unfinished/quarantined | unrelated acquisition continues | unrelated control continues | no authority access | no kill/respawn loop | unrelated Recorder continues | component failure | process restart; shutdown may report unfinished | no | owner liveness **GUARANTEED**; worker completion **NOT GUARANTEED** | managed timeout/shutdown tests |
| Recorder unavailable/unconfigured | attachment/start state | no Recording state or durable gate | acquisition policy-dependent but measurements stay truthful | Required cannot start/continue authority; BestEffort native work may continue | Required fails closed; BestEffort has no added authority | no hidden storage retry | no fake run/start | bounded recording-unavailable/start failure | correct storage and restart/reattach | yes under Required fault | **GUARANTEED** | Recorder startup/Required suites |
| Required Recorder ingress/progress/worker failure | credit rejection, sticky worker error, or 2-s committed-progress deadline | Recorder Failed; committed prefix fixed | acquisition may still publish truthfully | active controller fails; lease revoked | safety transition requested | no silent continuation | gap or `unknown_tail`; no false receipt | Failed status/operation | correct storage and new process/worker, then explicit control recovery | yes | **GUARANTEED** | `recorder_required`, backpressure/isolation |
| BestEffort Recorder failure | same storage signals | Recorder Failed; prefix fixed | native acquisition continues | otherwise valid controller may continue | no output authority transferred to Recorder | no fake receipt | gap/unknown tail | failed recording status | new worker/process | no solely due to Recorder | **GUARANTEED** | Recorder isolation/failure suites |
| SQLite busy/locked | 100-ms SQLite busy timeout/bounded query result | open/query fails or bounded job remains scoped | Runtime owner does not block | policy consequences only if writer fails | no direct authority effect | no unbounded retry | prefix unchanged; no second owner/boot | startup failure or `history_busy` | release lock, retry through accepted lifecycle | policy-dependent | **GUARANTEED** | external lock and held-SQLite tests |
| Write/transaction/seal failure | SQL statement/commit result | sticky Failed, never false Idle/success | no forged sample | Required fails closed; BestEffort split applies | Required safety path | no | transaction rolls back; prefix cannot cross it; gap/unknown tail | failed operation/status | inspect prefix; correct storage; new process | Required: yes | **GUARANTEED** | Recorder transaction/failure/shutdown tests |
| Filesystem/open/incompatible archive | pre-readiness validation/open result | no attached Ready worker or half-started run | serving never claims Ready | no experiment authority gained | none | no | existing bytes not rewritten | bounded startup failure | correct path/permissions/archive and restart | no | **GUARANTEED** (read-only oracle Windows-specific) | Recorder startup/reopen tests |
| Process kill | OS termination; later SQLite recovery | old owner cannot complete; lifecycle active until validated reopen marks interrupted | committed prefix only | no restored controller/lease | no shutdown/safety claim | no operation replay | WAL may retain committed prefix; run/interval unsealed, `unknown_tail` | later history shows interrupted/incomplete | reopen for inspection under new boot | yes before any new control | software reopen **GUARANTEED**; power loss **NOT GUARANTEED** | `recorder_process_reopen` |
| Explicit gap/incomplete coverage | durable gap or conservative unknown tail | coverage not complete | surviving measurements remain readable | no completeness inference | none | no | gap/incomplete lifecycle durable where possible | history/status truthful | inspect; start a new run after correction | policy-dependent | **GUARANTEED** | Recorder gap/quality/reopen suites |
| Malformed/invalid/unknown request | bounded UTF-8/JSON/envelope/DTO/registry validation | no Runtime mutation | none | none | none | client may submit a new valid frame only per connection policy | no semantic fact for rejected noise | bounded error or scoped close | correct request/reconnect | no | **GUARANTEED** | `client_isolation`, M9B.8 |
| Oversized/deep/large-value frame | hard 16-KiB/depth/value/string checks | no Runtime mutation | none | none | none | no | none | affected peer closes/bounded rejection; others continue | reconnect with bounded frame | no | **GUARANTEED** | real malformed-frame matrix |
| Slow reader/writer | bounded queues, nonblocking I/O, fixed 2-s deadline | only peer/session detaches | native progress continues | native progress continues | safety/transport completion continues | no infinite socket retry | Recorder independent | scoped detach; healthy peers continue | reconnect | no | **GUARANTEED** | client isolation pressure tests |
| Subscription overflow/replay gap | 16-event queue and 1,024-event ring cursor check | Runtime state unchanged; subscription removed/gapped | current snapshot remains authoritative | none | none | no guaranteed delivery | durable Recorder independent | explicit `event_gap`/`resync_required` | snapshot/history plus fresh subscription | no | loss signal **GUARANTEED**; delivery **BEST EFFORT** | `subscription_recovery`, M9B.8 |
| Client disconnect | reactor generation/detach | client-owned delivery state removed; experiment persists | native progress continues | accepted mutation continues | accepted physical/domain operation is not rolled back | retained mutation outcome follows accepted scope rules | Recorder independent of client lifetime | reply may be lost; later observation/replay remains truthful | reconnect/reattach | no | experiment isolation **GUARANTEED**; reply delivery **BEST EFFORT** | request deduplication/client isolation |
| History/job/cursor pressure | fixed job/page/cursor admission | Runtime/Recorder authority unchanged | acquisition continues | control continues | none | no unbounded queue | durable history only exposes committed pages | `history_busy`/bounded cancellation | consume/release/disconnect/expiry | no | **GUARANTEED** | `recorder_history_api`, isolation |
| Emulator pressure/invalid target/stale generation | target kind, generation, finiteness/range validation | bounded synchronous virtual commit or rejection | only declared virtual signal may change | generic dependent control sees truthful virtual samples | cannot fabricate physical Ready/ACK/readback/output evidence | no buffered retry | normal generic fact path for accepted virtual sample | ordinary result/error category | valid current virtual target request | no | **GUARANTEED** | `emulator_api`, M9B.8 |
| Client/session/operation capacity exhausted | explicit admission counters | experiment state unchanged unless work was already accepted | native progress continues | native progress continues | safety work retains priority | no unbounded fallback | independent | existing capacity/busy category; peer may detach by queue policy | terminal completion, detach, TTL, or later retry | no | **GUARANTEED** | M9B.8/M11.6 capacity tests |
| TCP bind/startup failure | bind/readiness handshake | service never claims ready | experiment process does not half-start | no controller authority | no output authorization | no | Recorder/transport startup unwind truthfully | process startup fails explicitly | correct configuration/address and restart | no | **GUARANTEED** | `process_readiness`, startup tests |
| Diagnostic directory/file unavailable | sink create/open/write result | no Runtime/Recorder transition | none | none | none | no recursive file retry; stderr remains best effort | Recorder unaffected | no API change | correct path and restart | no | **BEST EFFORT** diagnostics; experiment isolation **GUARANTEED** | diagnostic logging tests |
| Diagnostic queue overflow | lossy `try_send` counter | no Runtime/Recorder transition | native progress continues | native progress continues | safety path does not wait | drops/coalesced bounded indication | Recorder unaffected | no API change | automatic when sink progresses | no | delivery **BEST EFFORT**; nonblocking bound **GUARANTEED** | held-writer/native-progress tests |
| Diagnostic writer stuck/fails | sink error or 250-ms drain observation | authoritative result already independent | none | none | none | file side disables or handle detaches | Recorder unaffected | no API change | restart after correcting destination | no | flush **BEST EFFORT**; finite Runtime shutdown **GUARANTEED** | diagnostic failure/shutdown tests |
| Shutdown before diagnostic flush | bounded close/drain window | Runtime terminal state frozen first | none | safety result remains truthful | no stronger physical claim | no | Recorder shutdown result independent | process result does not claim log completeness | inspect retained prefix; restart | no | diagnostic tail **BEST EFFORT** | diagnostic shutdown tests |

## Bounds master table

Values below are the active source constants at the M11.7 gate. “Bounded failure”
means the subsystem reports/rejects/detaches according to its existing contract; it
does not allocate an unbounded fallback.

| Resource | Capacity / bound | Owner | On-full / expiry behavior | Release condition | Principal test |
|---|---:|---|---|---|---|
| Active clients | 8 | socket reactor | new peer closes | exact detach | `client_isolation` |
| Owner-to-reactor / reactor-to-owner mailbox | 64 each | process reactor/owner | nonblocking pause, scoped detach, or terminal owner failure | receiver progress/detach | reactor saturation tests |
| Inbound admitted requests | 8/client | peer/owner queue | socket read pauses | reply or detach | M9B.8 |
| Reply frames | 8/client | peer | affected peer detaches | write/detach | slow reader |
| Event frames | 16/client | peer | affected peer detaches with recoverable state | write/detach | event flood |
| Subscription | 1/client; 8 kinds; 16 targets | Application | busy/validation error | unsubscribe/gap/detach | subscription recovery |
| Event replay | 1,024 records; 4 KiB/record; scan 32/offer 4 per turn | Host/Application | old cursor gets gap/resync | eviction/current snapshot | M9B.8 gap |
| NDJSON/JSON | 16 KiB frame; depth 16; 1,024 values; 512-byte strings | peer parser | bounded rejection/close | reconnect | malformed matrix |
| Slow peer deadlines | 2 s hello/partial/write; 200 ms terminal-delivery window | reactor/process | scoped close; terminal delivery may be lost | reconnect/process exit | client isolation/shutdown |
| Session scopes | 16; detached TTL 1,800 s | Application | capacity exhausted | terminal eviction/TTL | dedup tests |
| Pending mutations | 8/scope; 64 global | Application | busy/capacity exhausted | terminal outcome | operation capacity |
| Retained outcomes | 32/scope; 256 global; 4 KiB each; TTL 600 s | Application | oldest terminal record evicts; ID never silently reused | expiry/eviction | request deduplication |
| Runtime recent history | configured ring; query max 128 | Runtime signal | oldest evicts | later samples | measurement API |
| Durable history | 8 jobs, 8 pages, 8 cursors; 128 rows/8 KiB page | Recorder/Application | `history_busy`/bounded page error | completion, expiry, detach | Recorder history API |
| Configuration staging | 1 candidate, 32 overlays, 30-s lifetime | Service/Application | busy/capacity/revision error; atomic rejection | apply/replacement/expiry | configuration API |
| Runtime objects | 64 instruments, 8 resources, 64 references, 64 controllers | Core Runtime | validated capacity error | configuration replacement/process lifetime | configuration/runtime tests |
| Managed components | 8 definitions; state 4 KiB; implementation ID 64 bytes | Core Runtime | bounded validation/capacity failure | reconfiguration | managed-component tests |
| Managed executor | 2 fixed workers; 1 job and 1 completion/slot; 100-ms deadline; 2-MiB stack/worker | ManagedExecutor | Busy/Executor; dead slot not respawned | completion or process restart | lifecycle tests |
| ResourceExecutor | 32 ordinary transactions plus 1 reserved safe-output lane | Core Runtime | bounded capacity failure; safety lane remains distinct | terminal completion | M3/M9D tests |
| Transport resources/transactions | 8 resources; one active transaction/resource; validated max duration 60 s | Runtime/adapter | bounded admission/deadline | terminal completion/rebind | transport suites |
| Reconnect/open | 1 worker/candidate; 64 attempts; 100-ms spacing; one original deadline | Service | finite failure, no parallel explosion | success/failure/retirement | reconnect suites |
| Recorder ingress | 4 groups; 1,024 records; 4 MiB total; 512 KiB/group | RecorderWorker | sticky failure/gap path; no buffering fallback | durable receipt | backpressure tests |
| Recorder causal group | 256 facts; 256 KiB | Core/Host | bounded admission failure | Recorder receipt | Recorder facts tests |
| Required Recorder progress | 2 s monotonic deadline | Runtime | fail closed | committed progress or explicit recovery | Recorder Required tests |
| Recorder SQLite | 1 GiB main quota with 5% reserve; WAL checkpoint threshold 16 MiB | SQLite worker | checked write/checkpoint failure | storage correction/new process | quota/checkpoint tests |
| Diagnostic queue/entry | 1,024 records; 8 KiB/record | diagnostic facade | lossy drop and bounded indication | sink progress | diagnostic pressure |
| Diagnostic files | 4 MiB/file × 4 files = 16 MiB | diagnostic sink | rotate oldest; file side disables on I/O failure | rotation/restart | production rotation soak |
| Shutdown | 2 s safety-unknown grace; 200 ms post-safe worker grace; 2 s Recorder/transport grace | Host/Service | finite unsuccessful status if proof/cleanup absent | terminal evidence/deadline | runtime/Recorder/transport shutdown suites |
| Diagnostic shutdown | 250 ms drain observation; drained library handshake bounded by 100 ms enqueue + 1,000 ms acknowledgement | diagnostics | detach; tail may be lost | drain or process exit | diagnostic shutdown tests |

## Bounded qualification and soak classification

### Mandatory automated

Ordinary debug/release workspace tests cover the configured acquisition fault
matrix, controller/OutputAuthority fencing, Required/BestEffort Recorder policy,
process-kill reopen, client isolation, history/cursor cleanup, managed-worker death,
diagnostic nonblocking behavior, API registry and finite shutdown. They exclude
only explicitly ignored longer qualification workloads.

### Bounded manual / preview acceptance

- `preview_soak::virtual_control_and_recorder_cycles_remain_bounded_and_close_cleanly`
  runs 2,000 100-ms monotonic turns across eight recording cycles, then checks
  controller progress, Recorder sealing, SQLite integrity and clean sidecars.
- `diagnostics::test_support::production_rotation_retains_exactly_the_documented_storage_window`
  writes beyond six file windows and checks exactly four retained files and at most
  16 MiB.
- Repeated focused configured transport/output, crash/reopen, client isolation,
  M9B.8 and shutdown suites exercise lifecycle churn without hardware.

These tests run for seconds/minutes, not hours, and are ignored so ordinary
`cargo test` remains suitable for development.

### Later extended soak

Multi-day acquisition/recording, millions of samples, thousands of reconnect and
start/stop cycles, long-lived subscriptions, physical-volume exhaustion, real power
loss and exhaustive USB/driver faults remain later manual qualification. No hard
real-time, throughput, RSS or latency promise is inferred from bounded soak data.

## Developer-preview technical gate

The gate requires all of the following:

- no known critical production correctness or safety blocker and no unexplained
  accepted test flake;
- deterministic mandatory debug/release, Clippy and warning-denied rustdoc gates;
- ambiguous output fails closed, is never blindly retried, and reconnect/input
  recovery never auto-rearms control;
- lease, epoch, generation and final-send authority checks remain enforced;
- configured acquisition faults and reconnect are finite and scoped;
- Required Recorder failure fails closed, clean archives seal and pass integrity,
  crash archives remain truthfully interrupted/incomplete, and history is bounded;
- malformed, slow and dead clients are isolated, all client-owned queues are
  bounded, and native work progresses under pressure;
- diagnostics have a lossy 1,024-record queue and strict 16-MiB file retention;
  diagnostic failure never becomes experiment failure;
- shutdown is finite and does not strengthen physical or durability evidence;
- the accepted M9D archive remains unchanged at SHA-256
  `14ec74be2a33cb795b29dcc295acc323a0dd4d8cf44b2cc5f6f64fd29e775c32`.

## Residual limitations and Arduino decision

Developer preview is not production certification. There is no multi-day unattended
soak, physical disk-full test, real power-loss validation, exhaustive OS/USB-driver
fault injection, hardware fault-injection Arduino, hard real-time guarantee, remote
network security model, or final release-polished documentation. M9D deliberately
did not prove physical heater effect because the load was disconnected.

Arduino hardware is **not required before developer preview**. Deterministic
software oracles establish the normative CRC, short-frame, silence, disconnect,
applied-WRITE-without-ACK, readback mismatch, fencing and no-retry semantics.
Arduino fault injection would add useful USB/driver/physical evidence later, but it
would not replace those software contracts.
