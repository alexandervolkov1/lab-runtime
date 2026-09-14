# M2 — deterministic output authority

Design before production implementation. Scope: virtual outputs only; no transport,
controllers, Lua, IPC or recording. M1 queries and configuration contracts remain.

## Ownership and API

Runtime owns a bounded per-instrument output authority for the existing heater_power
actuator. A typed ActuatorId identifies the instrument/parameter pair; a descriptor
must explicitly declare Actuator + OutputAffecting. ConfigureParameter still rejects
actuation. Output commands join the existing Command boundary; Query returns an
owned output snapshot without ticking clocks or expiring leases.

The authority owns state, epoch, one lease, at most one pending normal proposal,
one in-flight dispatch and a reserved safe-action flag. No background threads.
Manual/automatic are producer modes, not controllers. Both use identical validation.
Tokens and dispatch records have private fields so callers cannot forge permissions
or substitute the authorized value. IDs/epochs are local checked counters, not wire IDs.

## State and time

Unverified starts without an owner or fabricated physical value. Explicit SafeProfile
contains finite percent limits and a selected safe value inside those limits (not
universally zero), maximum lease and proposal lifetimes and required ACK/readback
evidence. These are virtual fixture policies, not hardware safety certification.

Explicit safe request revokes authority and enters SafePending. Safe completion
reaches Disarmed only with required evidence and no unresolved prior in-flight write.
Acquire from Disarmed creates ArmedManual/ArmedAuto with one bounded lease. Release,
expiry and trip revoke/increment epoch first, then request safe. Failure/ambiguity
latches fault; safe recovery may gather evidence but does not automatically clear
the latch or rearm. Fault acknowledgement requires resolved cause and valid safe
evidence; acquiring again is a separate command.

Every output mutation supplies runtime monotonic Duration, nondecreasing globally
for output authority. Zero/equal timestamps are allowed for distinct operations at
one deterministic tick. Backwards time and deadline overflow are rejected.
Tick explicitly expires leases even when no new proposal arrives. Queries are pure.
Proposal deadline is bounded by both profile TTL and lease expiry; expiry uses >=.

## Dispatch and evidence

Queueing does not send. BeginDispatch rechecks owner token, current epoch and deadline
at the last simulated-send boundary. Safe action has a reserved path; it cannot
overtake an unresolved in-flight operation. Stale pending work never reaches the
actuator. CompleteDispatch correlates the exact in-flight ID; late/duplicate results
cannot restore authority. Simulated completions distinguish ACK, readback, failed,
ambiguous. Requested/sent/acknowledged/readback remain separate observations.

An ambiguous terminal simulation result means the effect is unknown, not success.
The deterministic fixture assumes no additional effect after terminal completion;
real transport timeout/recovery must establish that boundary in M3. An old ordinary
completion may update delivery evidence but cannot authorize a producer or confirm
a newer safe procedure. Only completion of the correlated safe operation can do that.
No unbounded trace/history is retained; snapshots expose the latest evidence.

## Tests and teaching quality

Acceptance tests precede implementation: all M2.5 cases plus deadline boundaries,
nonzero safe value, stale result correlation, bounded pending capacity, monotonic
time and safe priority. Original M1 suite remains unchanged semantically.
English module/public API docs explain ownership, transitions, tokens, final checks
and ambiguity. Missing-docs becomes a workspace lint; document the M1 public surface
as a separate foundation change, without redesigning it.

Run fmt, workspace debug/release tests, clippy -D warnings, rustdoc, diff checks
and the existing finite host demo. Then report/update AI_HANDOFF and proceed to M3.
