# Safety and failure behavior

This is a developer-preview operational reference, not hardware certification.
Source and deterministic tests define the exact implementation.

## Guarantee vocabulary

- **GUARANTEED**: an explicit software/process invariant with deterministic
  implementation and regression evidence inside the stated boundary.
- **BEST EFFORT**: attempted, but loss or failure can occur without violating
  experiment correctness.
- **NOT GUARANTEED**: cannot be inferred from available software evidence.

The evidence boundaries are normative:

```text
requested != authorized != send_started != ACK != readback != physical_effect
fact admitted != fact durably committed
archive structurally readable != experiment semantically complete
process shutdown != proof that hardware physically became safe
```

## Failure and recovery matrix

| Failure | Detection | Runtime response | Controller/output response | Retry | Recovery | Guarantee |
|---|---|---|---|---|---|---|
| COM/open failure | bounded open result/deadline | candidate never becomes Ready; active generation is not silently replaced | no authority is gained | bounded open attempts only | correct settings/device, then explicit reconnect or restart | **GUARANTEED** |
| read silence/timeout | transaction deadline | resource becomes truthfully recovering/offline; signal receives `Unavailable(Transport)` | active control fails closed; authority is revoked and safe procedure requested | no hidden retry; normal physical recovery is explicit | `reconnect_resource`, then explicit controller recovery | **GUARANTEED** |
| CRC or malformed/truncated response | strict length/field/CRC decoder | transaction terminates without committing a false `Good` observation or poisoning the next frame | no output/probe evidence is fabricated | no frame reuse or blind retry | explicit reconnect when resource is offline | **GUARANTEED** |
| disconnect | transport error and bounded recovery deadline | resource Offline; one current generation remains authoritative | active controller becomes Failed; safe obligation remains visible | no parallel unbounded replacement | explicit reconnect/probe/rebind | **GUARANTEED** |
| incompatible probe | typed compatibility read | replacement remains quiesced/offline; acquisition is not released | no output adapter/authority is released | no | correct deployment/device and reconnect | **GUARANTEED** |
| stale transport completion | transaction and binding-generation correlation | discarded; current state is unchanged | cannot authorize or confirm current output | no | current generation continues | **GUARANTEED** |
| never observed / unavailable input | absence or explicit failed attempt | no cached value is promoted as current | cannot warm normally; active controller fails closed | source-specific polling only | obtain a fresh `Good`, then lifecycle recovery if failed | **GUARANTEED** |
| stale cached `Good` input | monotonic freshness deadline | old value remains historical but unusable | controller Failed, lease revoked, safe requested | no synthetic refresh | fresh measurement plus explicit recovery | **GUARANTEED** |
| invalid component result or dead worker | result validation or channel disconnect | affected component becomes Failed/unavailable; dead slot terminalizes | component cannot access transport/output; dependent control sees unusable input | no unbounded respawn | correct/reconfigure component or restart process | **GUARANTEED** |
| controller tick gap/non-finite result | Core time/math validation | controller enters Failed | lease revoked; safe obligation requested | no automatic restart | safe evidence, `reset_failed`, then `resume` | **GUARANTEED** |
| lease/epoch/generation expired or stale | final authority check | stale intent is rejected before byte zero | no output; old authority never resurrects | no | accepted lifecycle creates fresh authority | **GUARANTEED** |
| output rejected before `send_started` | final check or pre-send transport failure | no send-start evidence exists | physical effect is impossible through that attempt inside the adapter boundary | only the existing unstarted safe-delivery policy | correct authority/resource; submit new valid intent | **GUARANTEED** inside software adapter boundary |
| write started, no valid ACK | send-start record plus bounded outcome | output is Ambiguous; authority revoked | controller Failed; safe obligation retained; physical effect unknown | **no blind retry** | reconnect/safe establishment, then explicit controller recovery | software fail-closed **GUARANTEED**; physical effect **NOT GUARANTEED** |
| ACK but readback unavailable | strict ACK then failed read phase | ACK retained separately; result unverified/ambiguous | no verified output; no blind nonzero resend | no | safe/reconnect path and explicit recovery | distinction **GUARANTEED**; physical effect **NOT GUARANTEED** |
| readback mismatch | typed register comparison | mismatch and reported value remain visible | controller Failed; distinct safe attempt may follow policy | no blind retry of original command | matching safe evidence, then explicit recovery | mismatch detection **GUARANTEED**; physical effect **NOT GUARANTEED** |
| ambiguous safe write | safe send started without decisive result | safe obligation remains pending, resend is blocked, `safe_confirmed=false` | normal output prohibited and fault latched | **no automatic resend** | a validated rebind starts a new safe-establishment lifecycle | no-resend **GUARANTEED**; physical safety **NOT GUARANTEED** |
| matching safe readback | strict ACK plus separate matching register read | authority reaches confirmed-safe/disarmed evidence state | failed/paused controller still does not rearm | no duplicate send | explicit controller lifecycle | register evidence **GUARANTEED**; physical effect **NOT GUARANTEED** |
| Required Recorder failure | admission rejection, sticky worker/SQL failure, or committed-progress deadline | Recorder Failed and committed prefix fixed | active control fails closed; safety path runs | no silent continuation | correct storage/new process, then explicit control recovery | **GUARANTEED** |
| BestEffort Recorder failure | same storage signals | Recorder Failed and prefix fixed; unrelated native work may continue | no new authority comes from Recorder | no fake receipt | correct storage/new worker or process | **GUARANTEED** |
| SQLite lock/write/open failure | bounded open/busy/transaction result | no false recording/durable state | policy-specific Required/BestEffort behavior | no unbounded storage retry | release lock/correct path and use accepted lifecycle | **GUARANTEED** |
| process kill during recording | later SQLite recovery/reopen | committed prefix may survive; old boot/run/interval remain interrupted/incomplete | no controller/lease restoration and no shutdown claim | no operation replay | reopen for inspection under a new boot | process-kill recovery **GUARANTEED**; power-loss behavior **NOT GUARANTEED** |
| malformed/oversized/slow client | bounded parser, queues, and 2 s absolute deadline | peer is rejected/detached; experiment persists | native acquisition/control/safety continue | client may reconnect with valid bounded input | reconnect, `hello`, resnapshot | isolation **GUARANTEED**; reply delivery **BEST EFFORT** |
| subscription/replay overflow | 16-frame client event queue and 1,024-event ring | Runtime state unchanged; gap is explicit | no control effect | no guaranteed delivery | current snapshot/history and fresh subscription | loss signal **GUARANTEED**; delivery **BEST EFFORT** |
| bounded capacity exhausted | admission check | operation is rejected/scoped; no unbounded fallback | no implicit authority change | retry only when advertised and capacity has released | consume/release/expire or reconnect | **GUARANTEED** |
| diagnostic destination or queue failure | file I/O result/drop counter | no Runtime or Recorder state change | no authority/safety effect | diagnostics may fall back to stderr or be dropped | correct directory/restart if logs are needed | experiment independence **GUARANTEED**; log completeness **BEST EFFORT** |

## Output certainty model

| Stage | What it establishes | What it does not establish |
|---|---|---|
| requested | a producer proposed a value | permission, transport activity, or effect |
| authorized | current lease/epoch/range policy accepted intent | bytes were sent |
| `send_started` | the final authority/generation check passed and the first possible byte boundary was crossed | complete frame, ACK, device state, or physical effect |
| ACK | the strict protocol acknowledgement was decoded | a matching register value or physical effect |
| readback | a separate typed read returned a value; `readback_verified` means it matched | independent physical heater/load effect |
| physical effect | real-world plant behavior | **NOT GUARANTEED** by software ACK/readback |

Once `send_started` exists, timeout or transport loss cannot prove that nothing
happened. The original write is therefore not blindly retried. This applies to
ordinary writes and to an ambiguous safe write.

An ambiguous safe write preserves two different truths:

```text
safe obligation pending = true
ambiguous safe resend blocked = true
safe confirmed = false
normal output prohibited
```

An obligation is not permission to repeat a command whose outcome is unknown.

## Controller recovery

Healthy transport or input does not imply armed output:

```text
input healthy != controller armed != output authorized
```

The accepted fault recovery sequence is:

```text
fault
  -> ControllerState::Failed
  -> resource recovery / explicit reconnect and compatibility probe
  -> new generation and safe-state evidence where obtainable
  -> controller remains Failed
  -> controller_reset_failed
  -> ControllerState::Paused
  -> controller_resume
  -> ControllerState::Warming
  -> distinct fresh Good observations
  -> fresh authority, epoch, and finite lease
```

`controller_reset_failed` is accepted only after the authority has valid safe
evidence and no outstanding safe obligation. It acknowledges the failure into
Paused; it does not resume. Reconnect, configuration reload, elapsed time, or a new
`Good` observation never automatically rearm control.

## Recorder and archive failures

Required recording is part of the experiment policy. Admission exhaustion, worker
failure, transaction failure, or loss of committed progress fails closed and does
not fabricate a seal or durable result. BestEffort uses the same truthful Recorder
state but does not stop unrelated native work solely because storage failed.

SQLite can recover a committed WAL prefix after process termination. That proves
neither real power-loss survival nor semantic completeness. An unsealed or
interrupted run remains incomplete even when `PRAGMA integrity_check` succeeds.
See [Recorder and SQLite](recorder-sqlite.md).

## Client recovery

One client cannot become the experiment owner. Malformed or slow peers are scoped;
accepted mutations continue if the peer disappears. Mutation deduplication is
process-local and bounded. After a subscription gap or process-instance change:

1. perform `hello` and inspect the current boot/cursors;
2. query frozen discovery/current/status projections;
3. use durable history where required;
4. create a fresh subscription from a valid cursor.

Do not bridge a reported gap by assuming missing events were contiguous or harmless.

## Diagnostic logging

Diagnostics use `tracing`, `tracing-subscriber`, and a bounded nonblocking
`tracing-appender` worker. Defaults:

| Property | Value |
|---|---|
| level | `INFO` (`ERROR`, `WARN`, `INFO`, `DEBUG`, `TRACE` supported) |
| Windows directory | `%LOCALAPPDATA%\lab-runtime\logs` |
| files | active `lab-runtime.log` plus rotations; four files total |
| storage | 4 MiB/file, at most 16 MiB retained |
| producer queue | 1,024 complete records, lossy on overflow |
| record size | 8 KiB maximum; oversized formatted records are truncated |
| fallback | stderr mirror; one bounded warning when file output fails |
| shutdown | best-effort 250 ms observable drain; it cannot indefinitely delay Runtime shutdown |

Logs deliberately omit routine measurements and controller ticks at normal levels.
They may be lost during overflow or shutdown. No state transition, Recorder policy,
or operation result depends on a log write. Scientific/audit history belongs in the
Recorder, not in text logs.

## Shutdown

```text
shutdown requested != every worker/resource already retired
```

Shutdown revokes ordinary authority, services safety obligations, retires
transports, drains/seals Recorder work, and cleans up workers/clients for bounded
owner turns. A pending transport is nonterminal and continues receiving bounded
turns until completion or deadline. Truly hung resources are reported unfinished;
the process does not wait forever.

The terminal Application result reports `safe_confirmed`, unfinished workers and
transports, Recorder completion/error, output projections, and `exit_success`.
Unresolved ambiguity remains visible. A finite process exit is not proof that the
physical plant reached a safe value.

## Preview limitations

The preview has no multi-day unattended qualification, physical disk-full test,
real power-loss qualification, exhaustive USB/driver fault injection, Arduino
hardware fault-injection evidence, hard-real-time guarantee, physical-heater-effect
proof, or remote/network-security qualification. These limitations do not weaken
the stated software invariants; they define the boundary outside them.
