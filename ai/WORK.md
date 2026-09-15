# Current work — ASTRA_HIGH M6 design

Read first:

```text
ai/HANDOFF.md
ai/ROADMAP.md
ai/WORK.md
AGENTS.md
PROJECT_BRIEF.md

docs/implementation/MILESTONE_4_LIFECYCLE_REVIEW.md
docs/implementation/MILESTONE_4_REPORT.md
docs/implementation/MILESTONE_5_DESIGN.md
docs/implementation/MILESTONE_5_REPORT.md

docs/architecture/HIGH_LEVEL_ARCHITECTURE.md
docs/architecture/EXTENSION_MODEL.md
docs/architecture/RUNTIME_AND_SAFETY_MODEL.md

docs/migration/V1_TO_LAB_RUNTIME_MAP.md
docs/migration/REUSE_PLAN.md
docs/migration/MIGRATION_RISKS.md
```

You are:

```text
ASTRA_HIGH
```

Current authorized phase:

```text
M6 DESIGN / ARCHITECTURE REVIEW ONLY
```

Do not implement M6 production code.

Do not begin M7.

---

# 1. Verify baseline

Run:

```powershell
git status
git log --oneline -20

cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Expected reviewed implementation checkpoint:

```text
corrected M4 complete
bounded M5 complete
123 workspace tests
M6 not started
```

Do not reinterpret accepted M2–M5 contracts without a concrete contradiction.

---

# 2. Synchronize stale project docs first

Before material M6 design work, update:

```text
README.md
AGENTS.md
```

The current files contain stale previous-phase statements.

README should factually describe:

```text
M1–M5 implemented

workspace packages:
    lab-core
    lab-lua
    lab-runtime

corrected Warming/native lease renewal

bounded Lua virtual/model + transform proof

123 tests at reviewed checkpoint

finite executable is still not the autonomous M6 host

no Babashka/API
no Recorder
no real COM/hardware acceptance
```

AGENTS must make the current phase:

```text
ASTRA_HIGH — M6 design only
```

and route all future agents through:

```text
ai/HANDOFF.md
ai/ROADMAP.md
ai/WORK.md
```

Do not perform final documentation or repository-wide Markdown cleanup.

Commit this synchronization separately from the M6 design.

---

# 3. M6 goal

Design the first real long-running headless Runtime host and local external API.

Target:

```text
Babashka process
        ↓
bounded local versioned API
        ↓
long-running Rust host
        ↓
single authoritative Runtime owner
        ↓
virtual plant / Lua observations / Reference / native PID
        ↓
OutputAuthority
```

A Runtime-owned native PID must keep running if Babashka disconnects.

Reconnect must reconstruct current state without transferring experiment ownership to the client.

M6 remains a virtual/local software slice.

---

# 4. Authoritative ownership

Design one authoritative mutable Runtime owner.

Network connections, Lua workers and clients must not directly mutate Runtime.

Define clearly:

```text
Runtime owner execution context

network accept/read/write ownership

bounded request channel

bounded reply/event channels

Lua worker relationship

transport relationship

shutdown owner
```

No network or Lua lock may be required for:

```text
ServiceSafety
native PID progress
OutputAuthority
transport recovery
```

Avoid Tokio unless M6 requirements genuinely justify it.

Prefer understandable std threads + bounded channels if sufficient.

---

# 5. Autonomous scheduler

M6 must convert explicit service calls into autonomous host progress.

Use monotonic time for runtime/control scheduling.

Wall-clock persistence belongs later with Recorder.

Design explicit scheduling for:

```text
ServiceSafety

measurement / virtual plant refresh

native controller ticks

managed Lua invocation/poll/expiry

transport/recovery progress

network request handling
```

Do not infer desired controller cadence solely from `max_tick_gap`.

If an explicit period is required, add the smallest clear domain/config representation.

Required properties:

```text
safety work has priority over optional client work

native PID does not depend on a connected client

Lua is never synchronously awaited

slow clients cannot delay control

transport recovery gets bounded progress

drift policy is explicit

no unlimited catch-up after host stalls
```

For periodic work, evaluate deterministic scheduling using:

```text
next_due += period
```

with an explicit lag policy.

Do not replay a large backlog of obsolete PID ticks after a stall.

Define per-work-type policy:

```text
skip
coalesce
bounded catch-up
fault because timing guarantee was lost
```

---

# 6. Host service-turn ordering

Specify the order and bounded amount of work for one host turn.

Consider:

```text
safety/watchdogs

expired component deadlines

component completions

transport/recovery

scheduled measurement/virtual plant work

managed component admissions

native controllers

external commands

events
```

The exact order is a design decision.

Document why it preserves safety and fairness.

Request bursts must not starve native/safety progress.

Set a bounded amount of client work per service turn.

---

# 7. Local external API

Unless a concrete contradiction appears, design:

```text
loopback TCP
+
bounded versioned NDJSON
```

Default bind:

```text
127.0.0.1
```

M6 is:

```text
local-only
not safe for direct untrusted Internet exposure
```

No TLS/authentication requirement yet.

The JSON/network layer is an adapter.

Do not make wire DTO types become the Core domain model.

Prefer host-side implementation around `apps/lab-runtime` unless a separate crate solves a demonstrated dependency problem.

---

# 8. Protocol resource bounds

Specify hard bounds for at least:

```text
message/frame size

JSON depth where relevant

simultaneous clients

pending requests per client

Runtime request queue

reply queue

event queue

retained operation results

retained event history

subscriptions per client

string/error sizes
```

Malformed/oversized clients must not disturb Runtime progress.

A client that stops reading cannot create an unbounded queue.

Choose an explicit slow-client policy:

```text
gap/drop
or
disconnect
```

---

# 9. Version/capability handshake

Design:

```text
hello
protocol version
actual server capabilities
```

Incompatible versions fail explicitly.

Capabilities report implemented features only.

Do not expose Rust enum discriminants/layout as the protocol contract.

---

# 10. Preserve Query / Command / Operation semantics

Query remains:

```text
pure snapshot
no hidden I/O
no scheduler progress
no mutation
```

Typical queries:

```text
discover
describe
latest
runtime snapshot
controller snapshot
reference snapshot
component snapshot
```

Commands/Operations may mutate or schedule work.

External outcomes must distinguish where relevant:

```text
received
accepted
completed/applied
failed
outcome_unknown
```

Successful JSON parsing is not operation completion.

Network clients must never manufacture:

```text
DispatchOutcome
Acknowledged
ReadbackVerified
safe_confirmed
```

---

# 11. Request IDs and bounded deduplication

M6 introduces request IDs.

Design bounded process-local deduplication.

Required behavior:

```text
same retained request ID + same payload
    -> return known retained outcome

same retained request ID + different payload
    -> reject

old ID no longer retained
    -> never claim original operation did not execute

Runtime process restart
    -> no durable exactly-once promise
```

Define:

```text
dedup scope
retention bound
eviction
payload identity/fingerprint
operation-result retention
```

Dedup should survive TCP reconnect within the same Runtime process when appropriate.

Do not implement durable deduplication before persistence exists.

---

# 12. Events and subscriptions

Design bounded subscriptions for the useful vertical slice.

Possible public events:

```text
signal observation

controller lifecycle/diagnostics

Reference change

output-authority state

operation completion/failure
```

Do not mirror every internal Rust mutation as an event.

Use a monotonically ordered process-local event sequence/cursor.

Define:

```text
retention bound

per-client queue

subscription admission

slow-client behavior

gap detection

reconnect

resync
```

A client behind retained history receives an explicit gap/resync condition.

Never silently hide lost events.

---

# 13. Snapshot + cursor consistency

Explicitly solve:

```text
snapshot
event occurs
subscribe
event disappears
```

A reconnecting client needs a coherent:

```text
snapshot + cursor/barrier
```

Design an atomic Runtime-owner operation or equivalent correct scheme.

Describe recovery after:

```text
normal reconnect

slow-client gap

subscription recreation
```

No replay across Runtime process restart is promised before persistence.

---

# 14. Multiple clients

Support a small bounded number of simultaneous local clients.

Define:

```text
connection ownership

request ordering per connection

cross-client serialization

subscription independence

disconnect cleanup
```

One client disappearing must not:

```text
stop Runtime

stop Runtime-owned native PID

destroy experiment state

affect unrelated clients
```

---

# 15. Controller lifetime versus client lifetime

This distinction is mandatory.

Case A:

```text
Babashka requests StartController

Runtime-owned native PID starts

Babashka disconnects

Runtime scheduler keeps PID alive and renews its finite lease
```

Case B, only if an external/manual producer is exposed:

```text
external finite owner

client/heartbeat disappears

no native renewal privilege

lease expires

ServiceSafety drives configured safe transition
```

Caller-supplied `Automatic(id)` must never grant native renewal privilege.

If external direct output is unnecessary for the M6 vertical slice, do not add it merely for demonstration.

---

# 16. Reference and PID configuration

Design the smallest external commands needed for the Babashka vertical slice.

Network adapters must invoke domain operations, never mutate controller fields directly.

Decide:

```text
Reference retune while running

Ramp continuity

PID gain/limit configuration lifecycle

whether PID config is only allowed while Ready/Paused

integrator/reset semantics after config changes
```

It is acceptable for M6 to require PID configuration before Start if that keeps the contract clear.

Do not invent a generic runtime configuration language.

---

# 17. Long-running integration of M5 Lua

M6 integrates the existing M5 Lua boundary.

Do not broaden its authority.

Do not add:

```text
Lua controller
Lua filesystem
Lua network
Lua physical output
Lua generic Runtime command API
```

With both Lua workers stalled, unrelated:

```text
ServiceSafety
native control
transport recovery
external read/query service
```

must remain serviceable.

---

# 18. Host startup/shutdown

Design explicit startup and shutdown ownership.

Startup must not require GUI or Babashka ownership.

Shutdown should evaluate an ordered flow similar to:

```text
stop new external admission

stop/revoke external authority

stop native proposal production

request safe transitions

progress bounded trusted output/transport handling

close subscriptions/network

request Lua worker shutdown without infinite wait

finish process
```

Choose the exact order from accepted authority semantics.

Do not claim physical safety if required evidence was never obtained.

M6 output acceptance remains virtual.

---

# 19. Process restart semantics

M6 reconnect means:

```text
new client
same Runtime process
```

It does NOT mean:

```text
Runtime crashed/restarted
state automatically restored
```

Durable reconstruction belongs to M7 Recorder/persistence work.

Document this explicitly in protocol/client semantics.

---

# 20. Babashka client design

Design a thin real client under approximately:

```text
clients/babashka/
```

Expected later shape:

```text
bb.edn

src/lab/client.clj

small REPL-friendly helper namespace

acceptance/example script
```

Prefer plain Clojure data:

```text
maps
vectors
keywords
small functions
```

Do not build an object hierarchy.

Do not embed nREPL into Rust.

Do not build the full recipe/workflow language yet.

---

# 21. Desired REPL experience

The implementation should later feel approximately like:

```clojure
(require '[lab.client :as lab])

(def c
  (lab/connect! {:host "127.0.0.1"
                 :port 9000}))

(lab/hello c)
(lab/instruments c)
(lab/describe c instrument-id)
(lab/latest c signal-id)

(lab/set-reference! c reference-id ...)
(lab/configure-pid! c controller-id ...)
(lab/start! c controller-id)

(lab/subscribe! c ...)

(lab/pause! c controller-id)
```

Exact names are not fixed.

Optimize for clear interactive REPL use and explicit errors/outcomes.

---

# 22. Actual Babashka process is mandatory acceptance

M6 cannot be completed using only a Rust fake client.

The M6 implementation test must execute an actual `bb` process.

If Babashka is unavailable:

```text
M6 is incomplete
```

Record the blocker.

For automated tests:

```text
use OS-assigned/ephemeral ports

use explicit server readiness

avoid arbitrary startup sleeps
```

---

# 23. Main M6 end-to-end acceptance

Design an automated test for:

```text
start real headless Rust Runtime

wait for explicit ready indication

start actual Babashka process

hello/version/capabilities

discover instruments generically

describe virtual plant

read latest temperature

subscribe to temperature/controller diagnostics

configure Ramp Reference

configure PID as allowed

start native PID

observe virtual closed-loop movement

kill/disconnect Babashka

keep Runtime scheduling

prove PID remains Running
prove finite lease renewal continues

start a new Babashka process

obtain coherent snapshot + cursor

verify controller/reference/output state

pause native PID

verify Rust virtual safe transition
```

Also require focused acceptance for:

```text
malformed NDJSON

oversized message

unsupported protocol version

unknown operation

invalid configuration

duplicate request ID same payload

duplicate request ID different payload

evicted/unknown request outcome

slow subscriber

subscription gap

snapshot/resync after gap

disconnect during pending operation

multiple clients

stalled Lua worker while API/native safety continue

graceful shutdown
```

---

# 24. Test naming

New test filenames use descriptive snake_case.

Preferred examples:

```text
host_scheduler.rs
api_protocol.rs
request_deduplication.rs
subscription_recovery.rs
babashka_reconnect.rs
client_isolation.rs
runtime_shutdown.rs
```

Use:

```text
step_01_<name>.rs
step_02_<name>.rs
```

only for intentionally human-readable stages of one larger tutorial/acceptance scenario.

Tests must remain independently runnable.

Never rely on Cargo execution order.

Do not mass-rename existing historical test files during M6.

That cleanup happens before the first release.

---

# 25. Explicit M6 non-goals

Do not design or implement:

```text
Recorder

SQLite

GUI

real Windows COM deployment

physical Metakon acceptance

Internet exposure

TLS/authentication

programmable Babashka model

programmable Babashka controller

Lua controller

workflow/recipe engine

native dynamic-plugin ABI

WASM

final release docs

repository Markdown cleanup
```

---

# 26. Future Recorder compatibility without implementing it

M7 follows M6.

M6 should preserve semantic events sufficient for future recording of:

```text
command origin

operation accepted/completed/failed

controller lifecycle

Reference changes

signal observations

output-authority changes

Lua failures/reloads
```

Do not design SQLite tables now.

Do not make network DTOs the future persistence schema.

---

# 27. Native Rust stays first-class

Native Rust remains the correct extension path for:

```text
performance-critical controllers

high-rate filters

complex models

physical drivers

new protocols

safety/reliability-sensitive components
```

Recompilation is acceptable.

Do not add a dynamic native plugin ABI merely to avoid rebuilding.

---

# 28. M6 design deliverable

Create:

```text
docs/implementation/MILESTONE_6_DESIGN.md
```

It must contain:

```text
scope and non-goals

ownership/thread model

scheduler/time model

service-turn ordering

queue/resource bounds

startup/shutdown

wire protocol envelope

version/capability handshake

domain/wire boundary

request ID/dedup semantics

operation outcomes

event/subscription model

snapshot/cursor consistency

gap recovery

slow/multi-client policy

Babashka boundary

controller/client lifetime semantics

failure model

tests-first acceptance contract

SOL_HIGH implementation order
```

Give acceptance scenarios stable IDs such as:

```text
H1
H2
...
```

The contract must be precise enough that SOL_HIGH does not need to invent high-level concurrency/lifecycle architecture.

---

# 29. Allowed Astra changes

This Astra phase may change documentation such as:

```text
README.md
AGENTS.md
ai/HANDOFF.md
ai/WORK.md
docs/implementation/MILESTONE_6_DESIGN.md
```

and existing architecture documents only if a concrete M6 design correction requires it.

Do not add production:

```text
network dependencies

TCP server

Babashka implementation

new M6 Runtime production subsystem

Recorder

SQLite
```

Use small logical documentation commits.

---

# 30. Stop rule for architecture forks

If design exposes a genuine unresolved problem involving:

```text
scheduler ownership

OutputAuthority

external authority lifecycle

request replay/dedup safety

snapshot/cursor consistency

shutdown safety

Lua/network isolation
```

update:

```text
ai/HANDOFF.md
```

with:

```text
STATUS: WAITING_FOR_REVIEW
```

describe the exact fork and STOP.

Do not silently invent a new architecture merely to continue.

---

# 31. Required handoff after M6 design

When the M6 design and acceptance contract are complete:

update:

```text
ai/HANDOFF.md
ai/WORK.md
AGENTS.md
```

to request:

```text
ASTRA_HIGH -> SOL_HIGH
```

with:

```text
Reason:
M6 long-running host, scheduler, local API, subscriptions,
reconnect and Babashka acceptance contracts are fixed.

Resume from:
Implement M6 according to MILESTONE_6_DESIGN.md.

Do not start:
M7 Recorder.
```

Print:

```text
MODEL HANDOFF

STOP HERE.

Completed:
M6 architecture/design and tests-first acceptance contract.

Switch from:
ASTRA_HIGH

Switch to:
SOL_HIGH

Resume with:
M6 implementation only.

Do not begin M7.
```

Then STOP.

Do not implement M6 under ASTRA_HIGH.
