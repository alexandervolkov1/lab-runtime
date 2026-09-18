# Milestone 9D — physical Metakon output integration

## Status

```text
M9D: READY_FOR_EXTERNAL_REVIEW
M10: NOT AUTHORIZED
```

Date: 2026-09-18 (Europe/Moscow).

M9D was inserted before M10 to complete the audited partial Metakon output path
without changing the accepted OutputAuthority architecture. The software path and
the read-only preflight correction are complete. The final bounded real-device run
proved initial safe-zero, exactly one authority-gated +10 percent controller output,
strict ACK, distinct matching register-6 readback, normal pause to verified zero,
Recorder sealing and clean shutdown. M9D is ready for external review; M10 remains
unauthorized.

## Software implementation

Initial read-only preflight HEAD:

```text
ea49a61715e94dfd559a5d360e928410e0116b82
```

Initial read-only preflight artifacts:

```text
target/release/lab-runtime.exe
SHA-256 533661e9d850593966191c26b97e803ef2e12a8afe661c7967831e22f0e56232

examples/runtime.metakon-513-m9d-read-preflight.toml
SHA-256 50d1d26d515aa021e7522499e404c6729f2456a159983bdd77dc3a03d7464a7f

examples/runtime.metakon-513-com5-output.toml
SHA-256 82aaea40998965fc021dd08c9aea254e9c514bcb41f1b73ace31037b924061d8

examples/definitions/metakon-513-output.json
SHA-256 08cc2b1c3b7a945167297c4eccc5ed5c3cd7eab22edb8e48d54159d3667e2492
```

Commits:

```text
45f477436a98a36443b8cb2df34ab54a7aef9cef  feat: integrate authority-gated Metakon output
ea49a61715e94dfd559a5d360e928410e0116b82  test: add read-only M9D hardware preflight
b6e184077b968c61a0923b58539a26925f1f39fe  fix: await finite transport retirement on shutdown
```

The active path is:

```text
native controller
-> OutputProposal
-> OutputAuthority reservation and lease/epoch checks
-> bounded ResourceExecutor
-> final authority and binding-generation recheck
-> Metakon WRITE 0x01, register 0x06, signed I8
-> strict five-byte ACK
-> separate Metakon READ 0x00, register 0x06
-> signed-I8 readback comparison
-> ReadbackVerified or explicit bounded failure
```

The output definition uses percent at scale 1.0 and raw range -100..=100. Active
controller limits are 0..=10 and safe value is 0. The same authority-gated transport
path carries normal proposals and safe-zero. ACK remains distinct from readback and
neither is called physical effect. A mismatching readback records the reported value,
fails the dispatch and requests safe state. A readback timeout is ambiguous; the
original write is not retried blindly.

The concrete Windows COM module is crate-private. Application API callers,
controllers, ordinary instruments and managed components have no raw register-write
surface. The Application operation registry was not extended with `set_output`,
`raw_write` or `write_register`.

## Software verification

Before COM5 was opened, these gates passed:

```text
cargo fmt --all -- --check
cargo test --workspace
cargo test --workspace --release
cargo clippy --workspace --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
```

Focused acceptance also passed for exact signed-I8 frames at 0, +10, -10 and limits
+/-100; range rejection; strict ACK validation; ACK/readback separation; mismatch;
readback timeout without write retry; stale authority/generation before first byte;
generic configured controller binding; safe-zero through the physical path; virtual
ThermalPlant regression; protocol inventory; emulator/physical isolation; and M9B.8
fault acceptance.

One pre-existing Recorder timing test produced one transient failure during an
earlier debug run and then passed unchanged in the complete rerun and release suite.
No test relaxation or production workaround was made.

## Read-only hardware preflight

Setup:

- Metakon 513 on COM5, address 5, channel 0;
- 9600 baud, 8N1, no flow control;
- READ opcode 0x00, CRC initial value 0xFF;
- heater/load physically disconnected as confirmed by the operator;
- read-only definition with no actuator and Recorder disabled.

The second bounded preflight boot was
`2d5cd99eda6c48663218bc76c571061d`. It proved:

```text
COM5 open: successful
resource generation: 1
resource acquisition state: idle between transactions
channel_type: Good integer 3
temperature attempt 1: Good 28.0 degC at 24050565800 ns
temperature attempt 2: Good 28.0 degC at 25051834100 ns
temperature attempt 3: Good 28.0 degC at 26053839800 ns
temperature attempt 4: Good 28.0 degC at 28052517400 ns
```

The first harness run requested shutdown while an ordinary serial transaction was
pending and ended with incomplete cleanup. The corrected read-only run observed the
authoritative resource state `idle` immediately before requesting normal shutdown.
It still returned:

```text
operation state: failed
safe_confirmed: true
recorder_flushed: true
unfinished_workers: 0
unfinished_transports: 1
transports_closed: false
cleanup_complete: false
exit_success: false
process exit code: 1
```

Both processes exited and released COM5. The repeat after the real idle predicate
makes this a meaningful preflight blocker rather than treating the first harness
ordering error as acceptance evidence.

## Read-only shutdown root cause and correction

The unfinished object was the only configured transport for resource 1: its single
`ResourceExecutor`, binding generation 1, transport generation 1, and Windows worker
`lab-com-1`. The read-only deployment creates no output definition, safe profile or
controller, so there was no `OutputAuthority`, output reservation, pending output
operation, ACK/readback state, or second writer lifecycle. There remained one
physical resource, one bounded executor and at most one OS-open worker.

Shutdown is deliberately nonblocking. The first `ComTransport::try_shutdown` call
publishes the persistent stop intent, changes the adapter to `Closing`, and may
return `Pending` before the owner has observed the worker's terminal completion.
`HostCore` therefore correctly kept resource 1 out of `closed_resources`, which made
`unfinished_transports = 1`. The defect was in `ServiceHost::shutdown_step`: when
Recorder was already flushed, it froze that first ordinary `Pending` observation as
the terminal result instead of giving the transport a subsequent owner turn. The OS
worker then stopped and COM5 was released as the process unwound, but the already
frozen public result could not become successful.

Commit `45f4774` did not create a second executor or COM owner for the read-only
deployment. Its larger composition/timing changes exposed this pre-existing
scheduler-luck assumption. The correction in `b6e1840` waits for transport closure
after safety is confirmed, while retaining the existing absolute two-second bound
for a genuinely stuck transport. It does not add a second grace period after an
already failed safety deadline.

Two deterministic software oracles cover the exact distinction. A transport that
returns `Pending` once and `Complete` on the next owner turn now closes successfully;
a transport that never retires still produces an honest failed terminal result at
the finite deadline. The first oracle failed before the production correction and
passed afterward. Existing shutdown, physical configuration, full debug/release,
clippy and warning-denied rustdoc gates also passed.

A single corrected read-only COM5 run used boot
`815774ba12b3b5752326d7eb0558573f`. It observed resource 1 idle at binding and
transport generation 1, `channel_type = 3`, and four distinct Good temperature
samples at 28.0 degrees Celsius. Normal shutdown then completed with:

```text
safe_confirmed: true
recorder_flushed: true
unfinished_workers: 0
unfinished_transports: 0
transports_closed: true
cleanup_complete: true
exit_success: true
process exit code: 0
```

The process emitted no stderr and released COM5. The run loaded only
`runtime.metakon-513-m9d-read-preflight.toml`; physical WRITE count remained zero.

## Hardware write acceptance

The authorized acceptance attempt used implementation HEAD
`ecf5520021be2f25fc48d2c8e36845fa555db75b`, release binary SHA-256
`1fd6597fb5f565ce10136bab99d19645e07c8e33dfb0e083caab4cd0c8d369ef`, the accepted
COM5 output deployment, and the physically disconnected heater/load.

Production startup opened COM5, completed the channel-type compatibility gate and
established safe zero through the normal `OutputAuthority -> WRITE register 0x06 ->
ACK -> separate READ register 0x06` path before publishing readiness. Thus the only
physical write in this attempt was one successful safe-zero command. No controller
was started, no lease for nonzero authority was acquired, and no nonzero proposal or
write occurred.

Immediately after readiness, the temporary PowerShell acceptance harness failed
before its first Application API query because a helper parameter used PowerShell's
reserved automatic name `$args`. Parameter binding therefore tried to convert an
automatic `System.Object[]` to a hashtable. This is a harness defect, not a Runtime,
serial, protocol, ACK or readback failure. The failure handler terminated the
process; normal Runtime shutdown was not reached. The process and listener are gone
and COM5 was released. Because the acceptance failure rule prohibits further
physical exercise after the first meaningful failure, no corrected hardware rerun
was attempted.

The incomplete Recorder files were preserved separately as failure evidence:

```text
examples/metakon-513-m9d-write-smoke-failed.sqlite
size: 4096 bytes
SHA-256: da63a52bdf62b3fc863995d79f497584be3ba97c1de4c55d1d301afbafab6cfc

examples/metakon-513-m9d-write-smoke-failed.sqlite-wal
size: 399672 bytes
SHA-256: 8885b44a1c5198a47d9e8d96ab10b7b7c166a88a9fca5fcc12581fa06fd68fe3

SHM: absent
SQLite integrity check on an isolated copy: ok
boot: 47183b55695a8a6f1e1c35dbb23ed84b, active/unsealed
runs: 0
operation events: 0
controller events: 0
persisted output events: 0
```

The empty persisted event sets corroborate that the harness did not reach any API
mutation or controller start. They do not replace the startup readiness contract as
durable safe-zero evidence: the process was terminated before Recorder flush and
the WAL is deliberately retained. This archive is not acceptance evidence. The
clean acceptance path remains absent:

```text
examples/metakon-513-m9d-write-smoke.sqlite: absent
```

### Corrected harness attempt

The reserved `$args` collision was removed and the exact request/response helper was
validated without COM5 against the virtual Runtime. A normal offline run exercised
hello, queries, Recorder start/stop, reference and PID configuration DTOs,
controller start/pause and clean shutdown. A second offline run injected a failure
after controller start and proved the harness cleanup path performed pause, Recorder
stop and Runtime shutdown with exit code 0.

Exactly one newly authorized physical run then used that helper. Production startup
again completed safe-zero WRITE/ACK/separate-readback before readiness, and the
previous parameter-binding defect was gone: the first Application API output query
returned successfully. Its truthful settled snapshot was:

```text
state: disarmed
safe_confirmed: true
epoch: 2
sent: 0 at 170000500 ns
acknowledged: 0 at 310000200 ns
readback: 0 at 310000200 ns
reported_readback: 0 at 310000200 ns
outcome: readback_verified
requested: null
pending: false
in_flight: false
```

The harness incorrectly required `requested = 0` after settlement and required the
readback timestamp to be strictly greater than the ACK timestamp. Current Runtime
semantics clear the pending requested value after settlement, and two distinct
semantic stages may be committed in one owner turn with the same monotonic timestamp.
The separate `acknowledged` and `readback` fields and ordered Recorder stages, not a
strict timestamp inequality, establish their distinction. This was another harness
interpretation defect, not a Runtime, serial, protocol or device failure.

The harness stopped before Recorder start, reference/PID mutation, controller start,
lease acquisition or any nonzero proposal. Its accepted cleanup used normal
`runtime_shutdown`, which requested and verified safe zero again before closing the
transport. Therefore this attempt issued two physical writes, both register-6 zero:
startup safe-zero and shutdown safe-zero. Nonzero, ambiguous and mismatching writes
were all zero. The process exited 0 and released COM5.

The second diagnostic archive is cleanly closed but is not acceptance evidence:

```text
examples/metakon-513-m9d-write-smoke-failed-2.sqlite
size: 196608 bytes
SHA-256: cac2639d34b0158cad3844598876c96dabe568c343adca2104fa6d11c473d915
WAL: absent
SHM: absent
SQLite integrity check: ok
boot: b7a6db20096b443db5276d27fc530dce, sealed
coverage: complete
final output: disarmed, safe_confirmed, readback 0
resource: offline with zero unfinished transport work
runs/operation events/controller events/output events/gaps: 0
```

The lack of output-event rows is expected because the harness failed before starting
the recording run; the sealed boot exit summary retains the final safe state. No
further hardware attempt was made under that authorization.

### Final acceptance run

The final harness correction changed no production Rust or tests. Settled output
assertions now use the authoritative semantic fields rather than requiring transient
`requested` storage, and accept `readback_timestamp >= acknowledged_timestamp`.
They still require distinct sent, acknowledged and readback fields/stages, terminal
`readback_verified`, no pending or in-flight work, and the appropriate safe state.

Before reopening COM5, the complete harness path passed against software/virtual
infrastructure: startup, initial zero, measurement, P-only configuration, controller
start, nonzero settlement, pause to zero, Recorder stop and clean shutdown. A
separate injected register-readback mismatch was rejected and its normal pause,
Recorder and shutdown cleanup all completed. The harness contains no PowerShell
automatic/reserved-variable collisions. No arbitrary sleep is used as an acceptance
predicate.

Exactly one final hardware run used implementation HEAD and artifacts:

```text
HEAD: 693f5862147f42a7cc7b3b60fb7650ac7200a93e
target/release/lab-runtime.exe:
  SHA-256 1fd6597fb5f565ce10136bab99d19645e07c8e33dfb0e083caab4cd0c8d369ef
examples/runtime.metakon-513-com5-output.toml:
  SHA-256 82aaea40998965fc021dd08c9aea254e9c514bcb41f1b73ace31037b924061d8
examples/definitions/metakon-513-output.json:
  SHA-256 08cc2b1c3b7a945167297c4eccc5ed5c3cd7eab22edb8e48d54159d3667e2492
boot: a2e853dbc3e3d019f643713b33c2a350
```

The Metakon 513 was powered on COM5 at address 5/channel 0, 9600 baud, 8N1,
no flow control. The heater/load remained physically disconnected. The deployment
declared only register 0x06 writable: signed I8, scale 1.0, range -100..=100 percent,
safe value 0. Startup reached Ready, read channel type Good integer 3 and observed
four distinct generation-1 Good temperature samples at 29.0 degC.

Initial safe-zero settled through the production path:

```text
state: disarmed
safe_confirmed: true
epoch: 2
sent: 0 at 170000300 ns
acknowledged: 0 at 310000500 ns
readback: 0 at 310000500 ns
reported_readback: 0 at 310000500 ns
outcome: readback_verified
pending: false
in_flight: false
owner/lease: absent
```

ACK and readback remained distinct fields and protocol transactions. Their equal
owner timestamp is valid because both terminal observations were processed during
one owner turn.

The controller was configured before start using the authoritative current value:

```text
measurement: 29.0 degC
reference: 39.0 degC
signed error: reference - measurement = +10.0 degC
Kp: 1.0 percent/degC
Ki: 0
Kd: 0
unclamped output: +10.0 percent
output limits: 0..=10 percent
expected bounded output: +10.0 percent
```

The first intended nonzero proposal was the only nonzero physical command. It was
authorized at output epoch 3, binding generation 1, output revision 1, under the
finite automatic lease for controller instance 1 (expiry 8151583900 ns). The final
authority/generation recheck passed immediately before first possible output byte.
The command encoded signed I8 raw 10 (`0x0A`), WRITE started at 5061101500 ns, the
strict ACK event committed at 5103485900 ns, and the separate register-6 readback
event committed value 10 at 5151583900 ns. Terminal outcome was
`readback_verified`; proposed, authorized, ACKed and reported values were all +10.

The harness immediately paused the controller. Normal authority revocation advanced
to epoch 4 and sent safe raw I8 zero at 5185033700 ns. ACK zero committed at
5223720400 ns and the separate register-6 readback zero committed at 5270323700 ns.
The terminal state was disarmed and safe-confirmed, the controller remained paused,
and 19 subsequent bounded state observations showed no automatic rearm or later
nonzero command.

The full process issued four physical register-6 writes: startup zero, exactly one
+10 command, pause zero, and the normal shutdown zero. Nonzero writes were exactly
one; blind retries, ambiguous writes and readback mismatches were all zero. The
active Recorder interval contains the +10 and pause-zero output lifecycles; startup
and shutdown zero occurred outside that interval.

Recorder stop drained accepted facts, committed its transaction and sealed run 1 and
interval 1 with complete coverage and zero gaps. Shutdown retained final readback
zero and controller Paused, then reported:

```text
unfinished_transports: 0
transports_closed: true
cleanup_complete: true
recorder_flushed: true
exit_success: true
process exit code: 0
stderr: empty
COM5: released
```

Clean acceptance evidence:

```text
examples/metakon-513-m9d-write-smoke.sqlite
size: 221184 bytes
SHA-256: 14ec74be2a33cb795b29dcc295acc323a0dd4d8cf44b2cc5f6f64fd29e775c32
WAL: absent
SHM: absent
SQLite integrity check: ok
boot: sealed
run 1: sealed
interval 1: sealed
coverage: complete
gaps: 0
```

The archive contains three Good 29.0 degC samples and ordered requested, authorized,
send-started, acknowledged and readback-verified facts for +10, followed by the safe
send-started, safe-acknowledged and safe-readback-verified zero facts. This proves the
register command, ACK and reported register value. It does not prove physical heater
effect because the load was deliberately disconnected.

## Preserved evidence

Historical M8 evidence remained byte-identical:

```text
examples/metakon-513-com5-transient-open-retry-history.sqlite
SHA-256 63ecb8575be5ac82ef968b929e8b3098b051a0f30a55c190b7fdc1dcaf64dc80

examples/metakon-513-com5-prepared-reconnect-history.sqlite
SHA-256 1396421e62b5a1abb834b4178689b3303277a88d46174e0353d2710c2ab17023
```

The post-M9C read-smoke archive was not modified:

```text
examples/metakon-513-post-m9c-smoke.sqlite
SHA-256 098f2fdc31805cbcdfe46d04055a205c9f980fcf6955ccf8ede6043cbc00a9fc
```

## External-review remediation

The first external review found that an already-started safe WRITE could be sent
again after its terminal result became ambiguous. The exact interleaving was:

```text
safe WRITE 0 accepts a nonempty byte prefix
-> timeout / transport uncertainty
-> recovery completes
-> dispatch becomes Ambiguous
-> the next safety-service turn sees safe_needed
-> a second WRITE 0 is queued
```

The root cause was that `safe_needed` represented both the continuing safety
obligation and permission to initiate another safe dispatch. Failure handling tried
to suppress a retry by clearing that flag in one path, but recovery reasserted it;
retaining the obligation therefore also re-enabled admission.

`OutputAuthority` now retains two independent facts. `safe_needed` remains true,
while the private `safe_resend_blocked` latch records that an already-started safe
command has an unknown physical outcome. In that state the public snapshot is
fault-latched, has no ordinary lease, reports `safe_confirmed = false` and retains
the terminal `Ambiguous` outcome. Both automatic `begin` and trusted transport
reservation reject another write. Repeated safety turns and repeated safe requests
may advance lifecycle identity but cannot queue protocol bytes or rearm ordinary
output. Epoch, generation and final-send fencing are unchanged.

A failure definitively before the first byte does not set the latch and may use the
existing later safe-delivery policy. A normal ACK followed by matching separate
readback still confirms safe state. M9D does not add an automatic post-ambiguity
read transaction: after transport recovery the current model has no explicit,
identity-bound reconciliation operation that can safely distinguish a new evidence
action from the ambiguous dispatch. It therefore fails closed until a fresh
authority/rebind or a future explicit reconciliation policy. It neither clears the
safety obligation nor claims safe state. Ordinary ambiguous nonzero output retains
its existing rule: the nonzero command is never retried, and a distinct safe action
is requested.

The deterministic regression accepts a two-byte prefix of the safe frame, reaches
timeout, completes recovery and terminalizes `Ambiguous`, then executes repeated
safety turns and an additional safe request. The physical WRITE count stays exactly
one, the executor queue stays empty, no lease or nonzero output appears,
`safe_confirmed` remains false, generation fencing remains intact and transport
shutdown completes finitely. A companion oracle proves that a failure before
`send_started` can later deliver and verify safe zero. Existing M2 tests now retain
the same obligation/block distinction for failed and ambiguous safe dispatches.

The review also reported two unstable debug tests:

* `control_api::lifecycle_uses_runtime_safe_path_and_disconnect_does_not_undo_transition`
  observed `state = failed` instead of `completed`. It was an invalid timing
  assumption: controller start validates input age against the authoritative service
  clock, while workspace scheduling could age the fixture's startup sample beyond
  500 ms. The fixture now services due native work immediately before start and
  still requires the original completed lifecycle result.
* `recorder_process_reopen::killed_retune_after_durable_acceptance_has_unknown_terminal_and_is_not_replayed`
  reported `child did not reach held SQLite batch: Disconnected`. It was a harness
  synchronization defect. The child spun on an atomic barrier while the parent used
  an unrelated four-second scheduler timeout and discarded child stderr. The
  WriterBarrier now signals its exact reached predicate with a condition variable;
  the child retains its finite two-second hang guard, and the parent waits for either
  the marker or process EOF and reports status/stderr.

The two tests do not share production state or one production defect. Each passed
10/10 in isolation after correction; their binaries passed together and with
`--test-threads=1`. No sleep was enlarged, no assertion was weakened and the
workspace was not serialized.

Successive full-gate runs exposed three more pre-existing test assumptions of the
same class. Recorder history and start/stop tests repeatedly serviced periodic
producers while waiting for a storage predicate, allowing the tests themselves to
exhaust the four-group bounded ingress under scheduler delay. They now wait on the
durable Recorder state/FIFO ordering at a fixed authoritative Runtime instant. The
measurement projection test now establishes its required Good sample explicitly
instead of assuming the first scheduled refresh already ran. These are test-only
corrections; the history and lifecycle tests each passed 20/20, as did the
measurement precondition test.

The successful physical archive remains applicable. That run contained zero
ambiguous writes and zero mismatches, and all four writes reached strict ACK and
separate matching readback. The correction changes only the unexercised ambiguous
safe-WRITE branch; wire encoding, successful settlement, 0 -> +10 -> 0 evidence and
shutdown semantics are unchanged. No additional hardware run was performed.

## Final verification and remaining gate

After external-review remediation, the following gates passed:

```text
cargo fmt --all -- --check
cargo test --workspace                 PASS (two consecutive runs)
cargo test --workspace --release       PASS
cargo clippy --workspace --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
```

The complete M9D physical-output suite passed 10 consecutive runs. Focused M2
OutputAuthority, M3 Metakon transport, M4 controller, controller configuration,
configured physical output, Runtime shutdown, COM/Recorder shutdown and M9B.8 fault
acceptance suites passed. Historical M8, post-M9C and M9D archive hashes remain
byte-identical; the M9D archive has no WAL/SHM.

The read-only shutdown blocker remains resolved, the external-review blockers are
corrected, and final physical acceptance remains valid. M9D is ready for external
review. M10 remains unauthorized and was not started.
