# Explicit manual actuator control

This additive Application v1 extension exposes existing OutputAuthority manual
ownership and delivery. It sets a declared actuator value such as heater power;
it does not retune a PID Reference. No arbitrary bytes, register addresses, owner
IDs, authorization tokens, ACKs or readback evidence are accepted from clients.
Use `hello.operations` and the `manual_output` capability to detect support.

## Discovery and identity

Use `discover`, `describe`, `latest` and `output` before changing anything.
The descriptor supplies parameter role, access, exact value type, unit and limits.
`output` adds the following compatible fields:

- `manual_supported`: an executable native thermal-plant, Metakon or SimpleDevice
  adapter exists. This is capability, not permission. Metadata-only virtual
  instruments do not acquire a manual adapter.
- `authority_instance`: an always-present decimal-string process-local fence.
  Existing nullable `instance` retains its previous lease-only meaning.
- `profile`: installed `min`, `max`, `safe_value`, `max_lease_ns`,
  `max_proposal_ttl_ns` and `required_evidence`, or null without a policy.
- `manual_attempt`: the latest bounded manual delivery receipt, or null.

Existing fields, operations, protocol envelope version and Recorder schema remain
unchanged. Older clients can ignore new fields and advertised operations. Pair
process-local identities with `hello.boot_id`; never reuse fences across restart.

## Operations

Every mutation uses ordinary `request_id:{scope,seq}`. All four take
`actuator:{instrument,parameter}`, `expected_instance` from
`output.authority_instance`, and `expected_epoch` from `output.epoch`.
IDs, epochs and durations are canonical decimal strings. Unknown fields are rejected.

| Operation | Additional arguments | Completion meaning |
|---|---|---|
| `output_acquire` | `lease_lifetime_ns` | Finite exclusive lease issued to this attached scope; output has not changed. |
| `output_renew` | `lease_lifetime_ns` | Explicit extension of the current idle lease from Runtime monotonic now; new expiry must be later and within the profile maximum. |
| `output_write` | finite JSON number `value`, `proposal_ttl_ns` | One correlated delivery settled; inspect its evidence. Value uses the declared actuator unit. |
| `output_safe` | none | Authority revoked and safe procedure requested; this does **not** confirm delivery of safe output. Query subsequent `safe_confirmed` and observations. |

Descriptor, profile, exact physical encoding, availability, Required Recorder gate,
lease, epoch, binding/mapping generation and final pre-byte checks remain effective.
A string such as `"20"` is not numeric power. Fractional values must be exactly
representable under the declared encoding; the API does not round requested output.

Example after acquisition and a fresh output query:

```json
{"v":1,"msg_id":"power-1","op":"output_write","request_id":{"scope":"<hello scope>","seq":"2"},"args":{"actuator":{"instrument":"1","parameter":"2"},"expected_instance":"<authority_instance>","expected_epoch":"<epoch>","value":20.0,"proposal_ttl_ns":"200000000"}}
```

## Ownership and loss

The server-issued scope's boot-local counter identifies `OutputOwner::Manual`.
A different scope cannot write, renew or release its lease. Warming/running PID
conflicts with manual acquisition; manual authority conflicts with PID start.
Explicitly pause the PID and wait for safe evidence before acquiring manual control.
Queries and reconnect never transfer ownership or renew a lease.

Disconnect preserves admitted work and does not extend authority. The retained
scope can explicitly reattach and inspect its original operation. Without explicit
renewal the watchdog revokes the lease at its exclusive monotonic expiry and
services configured safe output. Disconnected hardware or Runtime/process loss
cannot be guaranteed to reach physical safe state; software delivery evidence is
not a hardware safeguard.

Before-send failure has no send evidence and never retries the ordinary write.
Existing transport recovery does not create another write or renew authority.
After any possible output byte, timeout is ambiguous and ordinary authority is
revoked. A distinct configured safe transition may follow. An ambiguous **safe**
write retains its obligation but blocks automatic resending. Existing explicit
`reconnect_resource` requires retirement and fresh safe evidence and never rearms
the replacement authority. Fault latches and controllers retain their deliberate
recovery rules. There is no new automatic reset.

## Accepted, delivered and observed

`accepted` only records bounded admission. Physical writes remain accepted while
queued, awaiting ACK or awaiting required separate readback. A completed
`output_write` contains its own terminal receipt: `attempt_id`, requested value,
optional dispatch identity, `terminal`, `outcome`, `sent`, `acknowledged`, `readback`,
`reported_readback` and `readback_failure`. `evidence_source` distinguishes
`virtual_simulation` from `transport_protocol`.

ACK-only profiles can complete with `acknowledged`, without a readback claim.
Readback profiles require `readback_verified`. Mismatch, unavailable readback or
uncertain delivery produces a failed operation retaining the receipt. ACK/readback
are protocol/register evidence, not independent physical-effect proof. Native plant
completion is explicitly simulated in Recorder.

Safe service does not overwrite the receipt. Current `output.readback` can be safe
zero while `manual_attempt.readback` retains prior delivered power. The next explicit
manual write replaces the terminal receipt; original results remain in bounded
session history. There is one unresolved manual write per actuator and the existing
64 global / 8 per-scope pending limits. Application waits at most 30 seconds for
settlement. Unsettled delivery or authority replacement reports an explicitly
unknown result, not success, cancellation or retry permission. A later query can
show more evidence without rewriting the original operation result.
The failed operation uses `timeout` for the finite wait or `instance_changed` for
replacement/loss of the original receipt, with `result.outcome:"unknown"`.

Exact-ID duplicates return original accepted/terminal state without another write.
Changed payload under the same ID conflicts. Expired evidence retains the existing
high-water protection against executing an old ID again. Status and Exact Retry
remain manual client choices.

## Recording and example

Existing operation records capture accepted/terminal manual actions. Existing
output facts retain attempt/dispatch identity, value, authority/binding lineage and
evidence source. Temperature follows normal acquisition/measurement recording.
Required Recorder failure still gates ordinary output before the byte boundary;
safety service remains available. There is no SQLite schema change.

See the [Babashka example](../../clients/babashka-smoke/manual-output.clj) and
[launch instructions](../../clients/babashka-smoke/README.md#manual-heater-power-direct-runtime).
Its default uses the native virtual plant. Virtual and byte-transport fixture
results do not imply a physical Arduino run.
