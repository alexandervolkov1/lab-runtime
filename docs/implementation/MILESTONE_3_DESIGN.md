# M3 — transport and narrow Metakon design / acceptance contract

Status: design fixed for implementation after the Astra High -> Sol High handoff.
This document is not implementation or passing-test evidence. Read AI_HANDOFF.md
first. Baseline: completed [M2](MILESTONE_2_REPORT.md), 38 passing tests.

## 1. Scope and ownership

Implement NEXT_IMPLEMENTATION_PLAN.md M3.1–M3.8, preserving M1/M2 contracts. Prove
Runtime-owned bounded transport, pure framing, native instrument semantics and one
output authority with a fake byte transport. No controllers, plant/EMA/PID, Lua,
Babashka, network service, recorder, GUI or physical actuator testing in M3.

Keep two packages. `lab-core` contains OS-independent domain state, a narrow byte
transport contract, deterministic executor model, pure codec and instrument mapping.
Concrete I/O and declarative-file loading belong in the host/adapters, outside core.
No Windows COM/serialport dependency in core. Actual physical serial integration is
optional, not an acceptance prerequisite or permission for dangerous testing.

Runtime owns registration, binding/catalog generations, output authorities and bus
executor state. Each resource has exactly one executor and at most one active
transaction. Two instruments on one bus cannot interleave bytes; separate resources
progress independently. Neither protocol code nor Instrument owns a serial handle.
The host supplies bounded byte adapters and explicit elapsed time. A narrow trait is
justified by fault-injected tests; no generic plugin framework or new micro-crates.

Refactor the currently virtual-specific internal registry only as needed for the
common descriptor/query boundary. Native virtual, native Metakon and declarative
Metakon definitions expose the same typed IDs, units, roles, effects and quality.
No generic discovery client branches on an instrument family. Parameter configuration
remains distinct from actuation, and queries never initiate I/O.

## 2. Byte executor and bounded work

The byte adapter exposes nonblocking/bounded attempt-to-write, poll-read and recovery
progress. It reports Pending separately from accepted bytes, EOF and errors. The
executor stores partial offsets, uses caller time, and does bounded work per poll;
never sleep or wait indefinitely inside Runtime. A future OS adapter must prove its
own call/deadline bounds before claiming the same behavior. Do not hold safety state
across an unbounded I/O call or expose the adapter to producers.

Initial hard limits (named constants, test boundaries): up to 8 bus resources,
32 queued ordinary transactions per bus, one active transaction, maximum frame
38 bytes, one reserved/coalesced safe request per actuator (existing maximum 64
instruments), one bounded latest result per registered parameter. One executor tick
performs at most one write attempt and one read/recovery poll per bus. No growing
completed-transaction log. Queue-full rejects explicitly without evicting safe work.
Per-actuator normal output pending capacity remains one.

Transactions carry typed operation, instrument/parameter, resource ID, immutable
mapping revision, binding generation, absolute queue deadline and execution timeout.
Durations must be positive, checked for overflow and bounded by a declared maximum
of 60 seconds each; deadlines expire at `now >= deadline`. Output queue deadline
also cannot outlive the proposal or lease. Evidence/result IDs are internal scoped
IDs, not authority conveyed by client strings.

The executor has explicit Idle, InFlight, Recovering and Offline states; its bounded
queue is separate. Cancellation before any bytes removes work. Once any byte was
accepted, cancellation only fences future work; it cannot undo transmitted bytes.
Safe work takes priority over queued ordinary work but cannot overtake an unresolved
in-flight frame or unsafe recovery. Bound safe backlog by actuator coalescing;
repeated requests must not allocate an unbounded queue.

## 3. The final output check is the physical initiation boundary

```text
typed proposal -> arbiter validation -> immutable queued intent
    -> current owner/epoch/expiry/binding/mapping check -> first byte attempt
    -> correlated delivery evidence -> optional verified safe confirmation
```

The queue holds intent plus opaque authority correlation, never a reusable Dispatch
permission. M2 BeginDispatch already records a simulated send: do NOT invoke it,
queue that Dispatch and later write bytes without revalidation. Extract/reuse private
validation logic behind one trusted coordinator; preserve the M2 simulation API and
its tests. Codec encode functions may return bytes, but only the owning executor
has access to a byte adapter. There is no raw-write command or arbitrary external
completion command for actual hardware bindings.

Immediately before the first possible byte, recheck actuator instance, lease owner,
epoch, lease/proposal expiry, active state, resource/binding generation, mapping
revision and immutable value/profile bounds. Safe actions use the reserved authority
path and current safe generation, not a producer lease. The single owner serializes
this decision with initiating a bounded write attempt; no intervening command may
revoke between check and initiation. WouldBlock with zero accepted bytes is not
sent evidence and requires a fresh check on the next attempt. A partial write is
in flight, not a newly queued proposal or permission for a second operation.

Revocation during an already-started frame cannot retract its prefix. The trusted
executor must complete that same frame or recover the resource according to adapter
semantics; no new ordinary operation can start. Snapshot evidence distinguishes
admitted/started/completed rather than claiming a complete frame on first-byte start.
If existing public observations need an additive delivery-state type, document it;
do not redefine ACK as readback or quietly weaken M2 final-check guarantees.

## 4. Failure, retry, recovery and rebind

Operation classification is native and explicit: read-only, explicitly idempotent,
output-affecting/non-idempotent. An idempotent label alone is NOT a retry permission.
Only whitelisted read-only operations may get one retry, after verified recovery
and within their original total deadline. Default writes, including apparently
idempotent setpoints and safe writes, have no automatic retry once any byte may
have been sent. Do not copy the donor's generic three-attempt wrappers.

Transaction timeout, partial response, CRC/type/address error or I/O failure produces
typed failure/unknown evidence, not a successful old sample with a new timestamp.
An ambiguous write revokes authority and latches fault; requested/sent/ACK/readback
remain distinct. No timeout or mere buffer clear proves physical safety.

On uncertain framing/delivery, enter Recovering before taking another transaction.
Metakon has no transaction ID on the wire: matching address/register is insufficient
to distinguish a delayed old response. Recovery must establish a clean response
boundary and that old operations cannot create future effects. A fake adapter may
explicitly guarantee reset/discard of its old generation; an unproven physical reset
must leave the resource Offline/faulted pending external reconciliation. Do not
invent a universally safe silent interval or equate reconnect with unique device
identity. Bound recovery attempts and time (one attempt, at most 60 seconds); on
failure remain Offline, never busy-loop. Other buses must continue progressing.

M2 terminal Complete assumes no future effect from that operation. Do not feed a
raw real timeout into that terminal assumption. Keep unresolved delivery explicit
until recovery establishes the boundary; a late ACK cannot authorize a new owner
or confirm a newer safe request. Safe output waits for settled/reconciled prior work.
Readback verifies the commanded register value, not the physical plant's condition.

Rebind/restart first revokes leases, increments checked binding/resource generations,
invalidates old queued work/results and marks observations unavailable. Validate
the replacement definition before committing it; identical display name, descriptor
or address does not preserve authority. Bring the new binding online only after
recovery/probe and an explicit safe procedure. Changing scaling/mapping increments
revision and also fences queued work; rename alone retains identity and mapping.

## 5. Pure Metakon slice and provenance

Source knowledge: read-only donor `D:\rust\com_port_reader`, HEAD
`50d3d1e3de84c650e1aa0ffbf1625044f794d315`,
`src/protocol/metakon.rs` (calculate_crc and frame tests) and
`src/instrument/metakon_5x3.rs` (selected register semantics). This is Metakon's
protocol, NOT Modbus. Rewrite the small pure codec; do not import SerialConnection,
retry wrappers or the donor as a dependency. Explain vector provenance in English
test comments, and validate against these known bytes, not round trips alone.

CRC starts at 0xFF. For each byte, consume eight least-significant bits. Feedback is
the low bit of data XOR CRC; if set, XOR CRC with 0x18, shift CRC right, then insert
feedback at bit 7. There is no final XOR. Freeze these donor reference vectors:

| Meaning | Hex bytes (final byte is CRC for frames) |
| --- | --- |
| CRC of single 00 / 01 / FF | 35 / 6B / 00 |
| Read device 1, channel 0, register 1 | 01 00 01 00 A0 |
| Read device 2, channel 0, register 1 | 02 00 01 00 28 |
| Read i16 1234 from device 1, channel 0, register 1 | 01 00 01 00 44 D2 04 F1 |
| Write i16 1234, device 15, channel 0, register 2 (codec fixture only) | 0F 00 02 01 04 D2 04 7E |
| Write i8 -25, device 15, channel 0, register 6 | 0F 00 06 01 02 E7 2B |
| ACK for device 15, channel 0, register 2 | 0F 00 02 01 BE |

Read request: device/channel/register/00/CRC, five bytes. Read response adds type
flags and data before CRC; validate exact length for expected type (minimum seven,
absolute bound 38). Write request uses function 01, type and data; write ACK is the
five-byte device/channel/register/01/CRC frame. Integer data is little-endian.
Type low nibble: Bool=0, U8=1, I8=2, U16=3, I16=4; flags readable=0x40,
writable=0x80. Reject unsupported types/flags, invalid bool and length mismatches.
Check device, channel, register, function, expected type and CRC before accepting.

Native profile `metakon-5x3-v1` exposes only:

- Register 0: channel type U8, expected 3 for this profile. This is a compatibility
  probe, NOT a physical serial number or proof that a reconnected device is the same.
- Register 1: measurement I16, normal raw range -999..9999; sentinel -32768 means
  explicit sensor fault/unavailable before normal range checking. Domain Float in
  Celsius, `engineering = raw * scale`, finite positive scale, no offset in M3.
- Register 6: output I8, raw -100..100, domain Float percent with unit scale 1.
  Reject fractional/unrepresentable output; never silently truncate/clamp to a byte.
  A fixture safe profile may restrict 0..100 and choose zero; that is an explicitly
  chosen simulated fixture, not a universal hardware rule.

Register 2 above is an encoding golden only, not permission to expose another native
operation. Probe/measurement/output-readback are read-only operations. Output write
must be Actuator + OutputAffecting and always pass through the arbiter.

## 6. Small declarative proof

Use strict JSON in the host, converted into a validated core definition. Serde plus
serde_json in the host is acceptable if needed; keep serializer/OS dependencies out
of core. Limit input before parsing to 16 KiB, depth to 8, parameters to 16, names
to 128 UTF-8 bytes. Reject duplicate JSON keys, unknown fields, unsupported schema
versions, duplicate parameter keys/IDs, nonfinite numbers and invalid units/ranges.
Do not rely on a map parser that silently replaces duplicate keys.

Schema version 1 contains `profile: "metakon-5x3-v1"`, a display name and parameters.
Each parameter declares a local numeric key, name, value kind, unit, role, access,
range, operation mapping, scale and side effect. The supported operation enum is
`channel_type`, `temperature`, `output`; it selects native register/type/function
semantics above, not arbitrary addresses or bytes. The validator cross-checks every
declared property against that trusted profile. Output scale must be 1; duplicate
aliases of the same operation are rejected. Output cannot be relabeled configuration
or read-only to gain a write path. Readback does not imply generic write permission.

Separate instance binding carries logical instrument ID, resource ID, device address
and channel. This is deployment wiring, not an expression language. A definition
may select a useful subset, rename fields, restrict ranges, and choose measurement
scale; it cannot add a new protocol or broaden output semantics. Rebind validation
is atomic, and safe profile binding remains explicit and separate.

Prove the extension claim with two validated definition fixtures using the same
profile but different names, IDs/scales and bindings. Register and interrogate both
without changing Rust core or writing family checks in a client. This proves a new
instrument description using known primitives, not an arbitrary protocol language.

## 7. Required executable acceptance (write tests first after switch)

These are requirements, not current passing tests. Group into codec, executor and
domain integration tests; keep original 38 tests passing throughout.

| Gate | Required assertions |
| --- | --- |
| C1 CRC/frames | All golden vectors above; encode/decode valid signed values; no dependence on a serial connection |
| C2 strict decode | Bad CRC, wrong device/channel/register/function/type, truncated/excess/oversized frame and unsupported flags reject |
| C3 measurement | Positive scale, sentinel yields fresh unavailable quality, out-of-range/nonfinite results reject, previous good sample not relabeled fresh |
| C4 output | Exact i8 boundaries, negative fixture vector, fractional/out-of-range/nonfinite/wrong-unit rejection before any bytes |
| T1 bus ownership | Two instruments on one bus never interleave; stalled bus A does not prevent bus B progress |
| T2 bounds | Queue full, max resources/frame, per-tick work and duration overflow reject/bound deterministically |
| T3 queue deadline | Exact deadline sends zero bytes, including time spent waiting behind another instrument |
| T4 delivery | Partial writes/reads assemble one transaction; zero-byte WouldBlock is not sent evidence; timed-out prefix is not undone |
| T5 recovery | Timeout/bad CRC/partial frame enter recovery; no next transaction before recovery; recovery failure becomes Offline |
| T6 stale bytes | Late response after timeout, including same address/register, cannot complete a newer transaction; old generation completion ignored |
| T7 retry | One explicitly permitted read retry only after recovery within total deadline; ambiguous ordinary/safe write never automatically repeated |
| A1 final gate | Queue output, then revoke/expire/change owner before first byte: zero unauthorized bytes; include TTL expiry during transport backlog |
| A2 pending send | WouldBlock, then revoke before retry: zero bytes; revoke after prefix leaves uncertain delivery without rearm or second operation |
| A3 safe recovery | Safe priority survives full ordinary queue; unresolved old write blocks safe confirmation; late old ACK cannot confirm new safe |
| A4 evidence | ACK alone does not become readback; mismatched readback does not confirm safe; generic setters/raw commands cannot write |
| G1 binding | Rebind/restart with equal IDs/descriptors fences old leases/queued work/results; fresh binding is unverified; rename alone preserves identity |
| G2 revision | Mapping/scaling change fences old work; invalid replacement leaves committed definition unchanged |
| D1 discovery | Native virtual, native Metakon and both declarative fixtures share introspection without family-specific client code |
| D2 strict data | Unknown/duplicate keys, depth/size/count bounds, bad type/unit/range/scale, unknown operation and output-role disguise reject atomically |
| D3 extension | Second definition loads and works through fake bytes with no Rust core changes; no arbitrary executable expressions or raw frames |

Use deterministic fake time and a scripted fault-injected byte adapter with observable
transmitted bytes, not sleeps or only domain mock results. Cover actual final-check
ordering at the adapter boundary. Recovery's fake generation guarantee must be
explicitly documented; do not misrepresent it as physical COM acceptance.

## 8. Implementation order and completion gate

After the user switches to Sol High: read AI_HANDOFF, applicable instructions, this
contract and current code; rerun the 38-test baseline. Write bounded executor/codec
acceptance tests first and record the red run. Implement small logical commits:
pure codec; bounded executor/recovery; trusted final output integration; native
descriptor mapping; strict host definition loader and second fixture. Keep code,
tests and architecture comments English and enforce missing-docs.

Before M3 completion run fmt, debug/release workspace tests, clippy -D warnings,
rustdoc, finite host regression and diff checks. Write M3 report with exact acceptance
mapping, dependencies, limits, provenance, recovery assumptions and hardware-not-run
statement; update AI_HANDOFF. Internal naming is not frozen; changing the safety,
recovery or trust contract requires recording a review question and stopping rather
than quietly weakening the gate.

This document does not authorize crossing any model gate. Per the supplied roadmap,
Sol implements M3 then M4, then stops for Sol High -> Astra High to design M5 only.
The current Astra phase ends here: no M3 test or production-code commit before switch.
