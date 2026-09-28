# M14.2 minimal native Application client

## Review status

```text
M14.1 Workbench architecture audit: ACCEPTED

M14.2:
READY FOR EXTERNAL RE-REVIEW

M14.3:
NOT AUTHORIZED

STATUS: M14_2_REMEDIATION_READY_FOR_EXTERNAL_REVIEW
```

External review accepted this implementation and its focused remediation on
2026-09-28. The accepted implementation commit is
`bdedf9455305f693a9537f698403c3bbb3840c51`. The original review-ready status above
is retained as the evidence state that was reviewed; active coordination now records
M14.2 as accepted and authorizes M14.3 only.

M14.2 creates the first `lab-workbench` executable and its private bounded native
Application client. It does not create a GUI, presentation model, reusable SDK, new
transport, or Runtime semantic surface.

The accepted M14.1 review-ready commit is
`6340aee32b563000bc6397a52aecd85ee945da1c`. The coordination commit that accepted
M14.1 and authorized M14.2 is
`3a2355450b62aa13122691fea51080674196c19d`.

## External-review remediation

The first M14.2 external review accepted the process/thread/framing structure but
requested four focused corrections. The remediation does not change Runtime or
Application semantics:

1. recovery insertion is now a checked precondition of mutation transmission. Eight
   unresolved records cause local `recovery_capacity` rejection before any request
   bytes are queued. A resolved completed/failed record may be deliberately retired
   to admit later work because it no longer represents an ambiguous or nonterminal
   mutation; unresolved records are never evicted;
2. `RetryMutation` now accepts only `MutationIdentity`. The worker looks up its own
   bounded record and supplies the exact stored `request_id`, operation, and args.
   Unknown, wrong-session, and still-in-flight identities are locally rejected;
3. Reference bootstrap now requires `Ready` before changing bootstrap or cursor
   state. Subscription setup is transactional from the worker's perspective: a
   local enqueue rejection restores the prior cursor and leaves no bootstrap;
4. every retained-scope connect attempt is capped by
   `min(CONNECT_DEADLINE, reattach_deadline - now)`. The same absolute deadline is
   checked while retrying and while awaiting hello, and is never extended by
   `scope_in_use`, connect failure, or partial progress.

The review scenarios are frozen as deterministic regressions described below. No
GUI, presentation, scripting, new protocol operation, or Runtime change was needed.

## Package and module layout

```text
apps/lab-workbench/
    Cargo.toml
    src/
        main.rs
        client/
            mod.rs
            framing.rs
            types.rs
            worker.rs
```

`lab-workbench` is a private binary package, not a client library. `main.rs` is a
small finite headless shell accepting a numeric loopback `--connect` address and an
optional retained `--scope`; it reports only the completed hello summary and then
stops its client. Acceptance orchestration stays in tests rather than becoming a
temporary production CLI.

The package depends only on `serde_json = "1"`, which was already present and
resolved in the workspace. The final locked graph is:

```text
lab-workbench 0.1.0
└── serde_json 1.0.151
    ├── itoa 1.0.18
    ├── memchr 2.8.3
    ├── serde_core 1.0.229
    └── zmij 1.0.23
```

No new third-party package entered `Cargo.lock`; the lockfile gained only the local
`lab-workbench` package record. Its inherited package license is MIT. The resolved
`serde_json 1.0.151` manifest declares `MIT OR Apache-2.0` and Rust 1.71; it is the
same already-locked package and transitive graph used by `lab-runtime`, so M14.2
introduces no new advisory or provenance surface. `cargo audit` is not installed on
this machine; this is not presented as a fresh whole-graph advisory scan. M14.2 adds
no dependency on `lab-runtime`, `lab-core`, eframe, egui, egui_plot, Steel, Lua,
Tungstenite, Tokio, async-std, or a channel framework. The workspace's declared
toolchain policy was not changed in anticipation of the future GUI; verification
used Rust 1.95.0 on Windows 10 22H2 build 19045.

## Process and ownership boundary

```text
lab-workbench main / future GUI
        |
        | bounded commands (32)
        v
ONE Application client worker thread
        |
        | TCP / NDJSON
        v
lab-runtime
        |
        v
ONE Application / ONE SessionStore / Runtime

ONE Application client worker
        |
        | bounded ordered updates (64)
        v
main / future GUI
```

Exactly one worker owns the `TcpStream`, decoder, pending output and byte offset,
connection lifecycle, hello projection, retained scope, authoritative `next_seq`
view, connection-local `msg_id` allocator, pending correlations, aggregate
subscription token, event cursor, reattach state, bootstrap buffer, and bounded
in-memory recovery records. No socket or writer escapes the module. There is no
`Arc<Mutex<TcpStream>>` or shared mutable client aggregate.

The handle contains only bounded channel endpoints, a checked local command-ID
allocator, the finite stop signal, and the join handle. The stop signal exists so
shutdown cannot be trapped behind a full command mailbox; it is not shared
experiment state.

## Frozen bounds and overflow behavior

| Resource | M14.2 bound | Owner and overflow behavior |
|---|---:|---|
| Application JSON body | 16,383 bytes | Framing rejects locally before write/dispatch. |
| TCP NDJSON frame | 16,384 bytes including LF | Fixed-capacity frame accumulation; oversize closes only this client connection. |
| Commands to worker | 32 | `try_send`; the caller receives local `Busy` on the 33rd queued command. |
| In-flight exchanges | 8 | Ninth request is locally rejected before socket write. |
| Ordered worker updates | 64 | No reply/event is silently dropped as continuity: saturation installs one sticky `ResnapshotRequired`, marks state stale, and closes the connection. |
| Active Runtime subscription | 1 | A second active or pending subscribe is locally rejected. |
| Recovery records | 8 | Same bound as in-flight mutation capacity; records are upserted by `request_id`. |
| Reference bootstrap event buffer | 64 | Overflow publishes `ResnapshotRequired`; it never claims a complete snapshot. |
| Socket read work | 8 KiB per worker turn | One fixed stack buffer; no unbounded read accumulation. |
| Commands serviced | 8 per worker turn | Network/deadline work cannot be permanently hidden by command traffic. |

The `std::sync::mpsc::sync_channel` mailboxes are finite. Submission and update
publication never wait indefinitely. The worker polls nonblocking network I/O and
uses a two-millisecond idle poll; it does not introduce an async runtime or a thread
per request.

## Deadlines

All deadlines use `Instant`:

| Activity | Absolute/finite deadline |
|---|---:|
| TCP connect | 2 seconds (`TcpStream::connect_timeout`); each reattach attempt is capped by the smaller remaining overall deadline |
| hello after complete write | 2 seconds |
| incomplete input frame | 2 seconds from its first byte; trickle does not reset it |
| blocked partial output | 2 seconds from first `WouldBlock` until later progress |
| ordinary/operation reply | 5 seconds after complete frame write |
| retained-scope reattach, including `scope_in_use` retries | one overall 3-second deadline |
| worker shutdown/join | 3 seconds |

A timeout or transport failure after any mutation byte was written is ambiguous. It
produces a recovery record and reconciliation update; it is never translated into a
terminal mutation failure. Reattach retry uses a short bounded delay inside the one
overall deadline rather than extending that deadline. Expiry also fences an
already-connected reattach that is still awaiting hello.

## TCP/NDJSON framing

`FrameDecoder` retains partial bytes only in its connection-local buffer. It accepts
the Runtime's LF and CRLF forms, removes only the transport delimiter, validates
UTF-8 and parses one JSON value. A frame at the exact 16,384-byte boundary with a
16,383-byte JSON body is accepted; another byte before LF is rejected.

Encoding streams `serde_json::Value` through a size-limited writer, so an oversized
request is rejected without first constructing an unbounded encoded body. One LF is
then appended. `PendingWrite` retains one frame and its exact byte offset across
partial writes and `WouldBlock`; the corresponding pending exchange remains counted
until its terminal response.

Server-side duplicate-key, nesting, member, string, envelope, operation and
`request_id` validation remains authoritative in the existing Application codec.
The client has not copied that semantic validator or the 42-operation registry.

## Hello, scope, and protocol discovery

A new connection sends `hello {scope:null}`. Reattachment sends
`hello {scope:<retained>}`. Before `Ready`, ordinary commands are locally rejected.
The worker consumes and retains the authoritative hello fields:

```text
boot_id
scope
next_seq
operations
capabilities
limits
event_oldest
event_latest
```

Operations, capabilities, and limits are data received from Runtime; there is no
client registry competing with the server. `scope_in_use` during reattach closes
only that attempt and retries within the original reattach deadline. `scope_unknown`
or `instance_changed` invalidates retained recovery/scope state and surfaces the
unchanged structured public error. No new session rule was added.

## `msg_id` ownership and reply routing

The worker owns a checked `u64` counter encoded as the protocol's decimal string.
It is unique among the at-most-eight in-flight exchanges and never wraps silently.
The pending table correlates replies even when an event arrives between them or
legal replies arrive out of request order. A server reply with an unknown or already
terminal `msg_id` is a connection-local protocol failure.

The table and counter are discarded/reset on reconnect. Reuse of the same textual
value on a later TCP connection has no deduplication meaning. No `msg_id` is stored
in a recovery record.

## Mutation sequence and response model

For Workbench-generated mutations the worker is the only allocator of:

```text
request_id = { scope: hello.scope, seq: hello.next_seq }
```

Sequence allocation is serialized. While admission is unknown, another generated
mutation is locally rejected with `mutation_reconciliation_required`. The local
view advances only after Runtime reports `accepted`, or after authoritative
`operation_status`/retained retry proves the corresponding sequence was admitted.
Encoding rejection before transmission removes the pending record without advancing
the sequence.

`ClientUpdate` keeps separate:

- ordinary result;
- bounded structured public Application error envelope;
- mutation accepted;
- mutation terminal completed;
- mutation terminal failed;
- Application event and subscription progress;
- transport failure;
- local client rejection;
- ambiguous mutation/reconciliation required;
- resnapshot required.

Socket write completion is never reported as admission or operation completion.

## Recovery records and reconciliation

The in-memory recovery record contains:

```text
boot_id
request_id.scope
request_id.seq
exact op
exact args Value
known admission: pending | accepted | ambiguous | completed | failed
```

It is bounded to eight entries and deliberately excludes `msg_id`. The exact record
is inserted before a mutation can enter the output queue. If all eight records are
unresolved, another mutation is locally rejected before wire emission. Checked
upsert results make any internal capacity-invariant violation explicit rather than
silently discarding a record. Completed/failed records are resolved: they remain
available for a subsequent exact retry, but may be retired deliberately when a new
mutation needs the bounded slot. An exact retry's terminal response also retires its
record.

If a connection is lost after mutation bytes may have reached the server, the record
becomes ambiguous. After retained-scope reattach, the caller can request
`operation_status` and ask to retry by identity. Only the worker can supply the
stored operation and args; callers cannot replace either field. Unknown identities,
records from another boot/scope, and a record already represented by a live pending
exchange are rejected locally. The worker never allocates around ambiguity and never
claims an exactly-once guarantee beyond Runtime's existing retained dedup contract.

M14.2 does not persist recovery records. Persistence and validate-before-replace
file semantics remain M14.3 work.

## Subscription, replay, and gap behavior

The worker owns one aggregate subscription token. `subscribe`, `unsubscribe`,
events, and subscription progress all use the same connection owner and ordered
update stream. Events must carry a valid, strictly advancing cursor. Disconnect
discards the connection-local subscription token, pending exchanges, and correlation
state while retaining the last process cursor as observational recovery input.

After reconnect, the caller creates a fresh subscription from the retained cursor.
Runtime remains the authority for replay. A correlated or unsolicited `event_gap`
clears subscription/bootstrap state and emits `ResnapshotRequired` with the original
bounded error envelope. The client never fabricates continuity.

## Representative bootstrap barrier

M14.2 proves the barrier for a Reference projection without adding a Runtime
snapshot transaction:

1. take `hello.event_latest` as cursor `C`;
2. install the aggregate reference subscription with `after=C`;
3. only after subscribe succeeds, query the Reference snapshot;
4. while the query is pending, buffer at most 64 matching reference events;
5. use the snapshot revision and apply only buffered events with a strictly newer
   revision;
6. on buffer overflow, malformed ordering, or `event_gap`, require a resnapshot.

Bootstrap setup first verifies `ConnectionState::Ready`. A disconnected request
returns local `client_not_ready` without installing bootstrap state, a subscription,
or a replacement cursor; a later retained-scope reattach can bootstrap normally.

An event after `C` is either replayed/delivered by the installed subscription or is
already represented by the later query snapshot. Revision filtering prevents
double-applying older events. M14.3 must generalize this operation-specific proof to
each Workbench projection using that projection's stable identity and revision
rules; M14.2 does not pretend that a generic presentation snapshot already exists.

## Shutdown and failure behavior

`ShutdownWorker` and the independent stop flag end reconnect attempts and command
admission. The worker drops the socket best-effort, clears all connection-local
tokens and pending network state, publishes terminal state where mailbox capacity
permits, and joins inside three seconds. Dropping a handle also signals stop.

Disconnect and worker shutdown never synthesize `runtime_shutdown`,
`controller_pause`, or `recording_stop`. Runtime remains alive and authoritative.
Malformed server frames, EOF/reset, deadlines, output pressure, and update pressure
are scoped to this client process/connection.

## Deterministic private-client tests

The production package does not depend on Runtime crates for testing. Test-only
scripted TCP peers and writers cover:

- LF, CRLF, split frames and the exact 16,383/16,384 body/frame boundary;
- oversized body/stream, invalid UTF-8/JSON, incomplete-frame deadline;
- partial write offset and blocked-write identity;
- frozen queue/deadline bounds and the 33rd-command `Busy` result;
- authoritative hello fields, retained scope, and `msg_id` reset on reconnect;
- out-of-order replies with independently routed events;
- ninth in-flight rejection before wire emission;
- accepted mutation sequencing and ambiguous disconnect recovery;
- eight unresolved recovery records rejecting a ninth mutation before wire output,
  followed by exact reconciliation freeing capacity for the next mutation;
- worker-owned exact retry emitting the original stored op/args, with fabricated and
  still-in-flight identities rejected locally;
- bounded `scope_in_use` detach-race retry;
- deterministic connect-timeout calculations at the full, partial, exact-expiry and
  expired reattach boundaries;
- query/mutation retry/status/subscribe/unsubscribe/bootstrap command paths;
- disconnected bootstrap rejection followed by successful bootstrap after reattach;
- structured public errors and explicit `event_gap` resnapshot;
- ordered update saturation closing the connection and requiring resnapshot;
- malformed peer input, EOF/timeout isolation;
- finite worker shutdown with proof that no `runtime_shutdown` operation is sent.

The final focused debug suite ran successfully three consecutive times:

```text
20 passed; 0 failed; 1 ignored (each run)
```

The ignored item is the separately invoked real-process acceptance below.

## Real Runtime process acceptance

The opt-in test starts the built `lab-runtime` executable as a child with
`--serve --profile virtual-demo --port 0`, parses its real readiness JSON, and drives
the private client over TCP without a Rust package dependency on Runtime.

Command:

```powershell
cargo build -p lab-runtime -p lab-workbench --locked
cargo test -p lab-workbench runtime_acceptance::real_runtime_reference_reconnect_reconcile_and_replay --locked -- --ignored --exact --nocapture
```

Observed result:

```text
1 passed; 0 failed
```

The scenario proves hello and advertised operations/limits, Reference query, one
aggregate subscription, `reference_retune` accepted, client disconnect after
admission, Runtime still alive, same-scope reattach, `next_seq` continuity,
authoritative completed `operation_status`, exact retry returning the retained
result, unchanged committed revision after retry (no second execution), a valid
next-sequence mutation, replay from the pre-mutation cursor, unsubscribe, finite
client shutdown, and Runtime still alive afterward. Test cleanup terminates the
child only after those assertions; client shutdown itself does not terminate it.

## Complete verification

Run from the final M14.2 working tree on Windows 10 Pro 10.0.19045 with
`rustc 1.95.0` and `cargo 1.95.0`:

```text
cargo fmt --all -- --check
    PASS

cargo test --workspace --locked
    PASS

cargo test --workspace --release --locked
    PASS

cargo clippy --workspace --all-targets --locked -- -D warnings
    PASS

cargo test -p lab-workbench --locked
    PASS three consecutive debug runs

real Runtime process acceptance
    PASS

git diff --check
    PASS
```

The two pre-existing Runtime opt-in rotation/soak tests remain ignored and unchanged.
The new M14.2 real-process acceptance is also opt-in because it requires the sibling
Runtime executable to be built first; it was run explicitly and passed. No ignored
test was edited merely to alter counts.

## Unchanged Runtime and protocol semantics

M14.2 adds no Application operation, capability, DTO, public error, SessionStore
rule, dedup rule, subscription/history behavior, Runtime/Recorder/SQLite behavior,
OutputAuthority path, or physical-safety claim. It adds no presentation or scripting
semantics anywhere. Runtime remains the sole authoritative experiment owner; all
client state in this package is connection/session coordination or discardable
observational recovery data.

```text
Runtime owns experiment semantics.
Client owns presentation semantics.

client lifetime != experiment lifetime
GUI lifetime != experiment lifetime
script lifetime != experiment lifetime
```

## Remaining M14.3 work

M14.3 is not authorized. Its accepted future scope begins with client-owned
`WorkbenchModel`/`PresentationDocument`, persistence validation, general projection
bootstrap/rebuild policy, and durable recovery-journal design. It must not move
presentation state into Runtime or weaken the M14.2 single-owner client boundary.
GUI dependencies remain deferred to M14.4, and Steel remains blocked by its separate
dependency-safety gate.

## Final status

```text
M12: ACCEPTED

M13.1: ACCEPTED
M13 dependency safety resolution: ACCEPTED
M13.2: BLOCKED / NOT AUTHORIZED

M14.1: ACCEPTED

M14.2:
READY FOR EXTERNAL RE-REVIEW

M14.3:
NOT AUTHORIZED

STATUS: M14_2_REMEDIATION_READY_FOR_EXTERNAL_REVIEW
```
