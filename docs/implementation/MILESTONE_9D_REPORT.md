# Milestone 9D — physical Metakon output integration

## Status

```text
M9D: HARDWARE ACCEPTANCE BLOCKED
M10: NOT AUTHORIZED
```

Date: 2026-09-18 (Europe/Moscow).

M9D was inserted before M10 to complete the audited partial Metakon output path
without changing the accepted OutputAuthority architecture. The software path and
the read-only preflight correction are complete. M9D is not ready for external
review because its real-device write acceptance stopped on a harness failure before
the controller or any nonzero proposal was started. A corrected attempt reached the
first Application API output query, but a second over-strict harness assertion again
stopped before controller start. Production safe-zero completed; the requested
nonzero and controller-pause sequence was not run.

## Software implementation

Hardware-test HEAD:

```text
ea49a61715e94dfd559a5d360e928410e0116b82
```

Exact tested artifacts:

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
third hardware attempt was made.

## Preserved evidence

Historical M8 evidence remained byte-identical:

```text
examples/metakon-513-com5-transient-open-retry-history.sqlite
SHA-256 63ecb8575be5ac82ef968b929e8b3098b051a0f30a55c190b7fdc1dcaf64dc80

examples/metakon-513-com5-prepared-reconnect-history.sqlite
SHA-256 1396421e62b5a1abb834b4178689b3303277a88d46174e0353d2710c2ab17023
```

The post-M9C read-smoke archive was not modified.

## Remaining gate

The read-only shutdown blocker remains resolved. Both physical attempts stopped on
harness defects before controller start, with only verified zero writes. A fresh
explicit decision is needed before retrying the bounded production-path sequence:
startup safe-zero, one authorized +10, normal safe transition to zero, ACK plus
distinct register-6 readback, Recorder seal and clean shutdown. M9D remains blocked
from external review and M10 remains unauthorized.
