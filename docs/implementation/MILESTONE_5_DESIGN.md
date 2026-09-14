# M5 — bounded Lua components and Sol acceptance contract

Status: ASTRA_HIGH design fixed for implementation after the model handoff,
2026-09-15. This is a specification, not implemented or measured Lua behavior.
Baseline `21d5cde`: 72 workspace tests pass; clippy -D warnings passes. No Lua
dependency, production code, spike or implementation test was added in this phase.

Authority: `ASTRA_M5_DESIGN_INSTRUCTIONS.md` and the external reviewer response
in `AI_HANDOFF.md`. First implement and verify every prerequisite in
[M4 lifecycle review](MILESTONE_4_LIFECYCLE_REVIEW.md). This design preserves the
M2/M3 evidence, ownership, first-byte and partial-write contracts. M6 is out of scope.

## 1. Scope and explicit choices

Implement two useful proofs through one bounded component contract:

1. A Lua virtual measurement model: a deterministic temperature source, with
   configurable baseline/rate and explicit elapsed time. It exposes one read-only
   Float measurement through normal InstrumentDescriptor/Signal queries.
2. A Lua three-sample moving-mean filter: one Float input/output in the same unit,
   explicit three-sample warm-up and a bounded numeric state vector. Registering
   a second script/configuration must need no new Core implementation variant.

No Lua controller in M5. The two proofs already exercise state, units, failures,
asynchronous execution, generation fencing and dependency-driven native safety.
A Lua controller would add scheduling/renewal and actuation admission choices
without strengthening those proofs. Section 12 preserves its future extension
point, with no present callback capability or placeholder implementation.

Both proofs are observation/model components. Neither has an actuator descriptor,
physical operation capability or safe-action callback. A native PID may consume
their validated signal only while driving the existing M4 virtual thermal plant.
The pure native plant/PID path remains independently usable. Script-derived values
are not protective interlocks or permission for physical closed-loop operation.

Named physical operations are a future boundary described in section 12, not a
callable M5 API. No global `app`, arbitrary Runtime commands, network service,
Babashka code, recorder, GUI, general graph engine, plugin process, WASM or Tokio.

## 2. Packaging, ownership and domain integration

Add exactly one package in the Sol implementation: `crates/lab-lua`.

```text
lab-runtime (composition, source deployment, worker supervision)
    -> lab-lua (Lua-specific conversion and worker execution)
    -> lab-core (same domain owner and public command/query boundary)
lab-lua -> lab-core
lab-core -X-> mlua / Lua / OS thread or file APIs
```

Core remains std-only. Lua configuration and source loading live outside Core;
VM values and closures never enter domain snapshots. Do not merge the Lua host
into native PID, instrument drivers or OutputAuthority.

Only the trusted composition/deployment layer reads script source from an explicitly
selected local bundle, enforcing the byte limit while reading. Core receives bounded
source identity/data through the adapter seam; Lua never receives that file handle,
path authority or a source-loading callback. Invocations cannot request more files.

Core adds a small language-neutral managed-component seam, not a general plugin
SDK. Concrete names may vary, but these responsibilities and access rules are fixed:

| Object | Owner and necessary contract |
| --- | --- |
| Component registration | Runtime: logical ComponentId, checked generation, manifest, committed bounded state, warm-up counter, lifecycle, one output signal and optional one input |
| Component manifest | Validated data: schema version1, kind Source/Transform, one Float output descriptor, configured input/output unit, Float limits, warmup count1..64, positive input age, bounded scalar configuration |
| Executor port | Supplied trusted adapter, owned by Runtime; nonblocking `try_submit`, bounded `try_poll`, nonblocking cancellation/shutdown request; no synchronous evaluate method |
| Invocation | Owned immutable data plus Runtime-generated correlation; no Runtime/Instrument references, output token or transport handle |
| Completion | Owned bounded result data or typed failure; never Command, DispatchOutcome or function pointer |
| Worker supervisor | Host/lab-lua: exactly two fixed worker slots, their mailboxes, clocks/cancellation and quarantine accounting; no ownership of domain state |

One executor-port trait is justified by a real Lua adapter and a deterministic
fault-injected test adapter, analogous to M3's narrow ByteTransport seam. Do not
add an interface per algorithm. Public adapter installation/completion polling is
a trusted Rust composition boundary; it is never placed in the Lua environment
or automatically exported as a future external-client command.

Managed source/transform registrations occupy the existing maximum-64 instrument
budget; maximum eight managed components total. Each provides a normal descriptor,
latest/window query and measurement state, so generic consumers do not branch on
Lua. A transform is represented in M5 as a read-only derived-signal instrument;
its binding and processing status are available in a separate component snapshot.
No change to SignalId's instrument/parameter identity representation is needed.
Runtime reuses the existing descriptor validation and bounded SignalBuffer.

Allowed bindings are native or managed Source -> one managed Transform -> native
controller. Transform-to-transform binding, self-binding, cycles, multi-input and
dynamic graph editing are rejected in M5. The resulting dependency walk is bounded
by eight components plus 64 controllers. Lua origin never changes scalar/unit
validation or permits a generic ConfigureParameter write to actuate.

Useful application operations are Stage/Register, explicit Invoke, PollComponents,
Reset/Reload and QueryComponent. They use the existing command/query ownership
semantics. PollComponents is a mutation, not a hidden query effect. Registration
and reload have accepted/pending/committed/rejected results; do not report a staged
candidate as installed. One staged candidate globally, one pending invocation per
component, no unbounded operation history or M6 request/deduplication protocol.

## 3. Selected embedding and verified capabilities

Choose `mlua = 0.11.6`, with default features disabled and only `lua54` and
`vendored`, for the initial implementation. Pin the selected crate in Cargo.lock
and record the resolved Lua/ffi versions in the M5 report; no dependency is added
by this design. Do not enable LuaJIT, Luau, module, async, send, serde, macros or
native module support. VMs are constructed inside their owning worker, so Lua
objects need not cross threads. Feature definitions and minimum Rust version are
verified in the [versioned mlua manifest](https://github.com/mlua-rs/mlua/blob/v0.11.6/Cargo.toml).

The research below concerns the selected version's actual APIs, not an inference
that `Lua::new()` supplies this project's sandbox:

| Capability observed in primary source | Design consequence |
| --- | --- |
| `Lua::new_with`, `set_hook`, `set_memory_limit`; the hook is per Lua thread, memory limit produces MemoryError | Construct with StdLib::NONE, install limits before script loading, expose no coroutine path. These APIs are building blocks, not a hard execution-time guarantee. [Lua API](https://docs.rs/mlua/0.11.6/mlua/struct.Lua.html) |
| `Chunk::set_environment` and `set_mode`; binary chunks are not consistency-checked | Bind a fresh explicit environment and require Text mode; reject supplied bytecode. [Chunk API](https://docs.rs/mlua/0.11.6/mlua/struct.Chunk.html) |
| Count hooks run during Lua function execution | Compilation, C work, conversion, GC and cleanup cannot be bounded by instruction counting alone. External deadline admission and worker isolation are mandatory. [Lua 5.4 manual](https://www.lua.org/manual/5.4/manual.html#lua_sethook) |
| mlua's allocator rejects growth above its quota; actual system allocation failure calls Rust allocation-error handling | The quota bounds accounted Lua heap, not process RSS or survival under system OOM. [Allocator source](https://github.com/mlua-rs/mlua/blob/v0.11.6/src/memory.rs) |
| Lua has Drop/GC behavior and an internally locked state | Never clone/share a VM with Runtime or destroy it on the safety lane. [State source](https://github.com/mlua-rs/mlua/blob/v0.11.6/src/state.rs) |
| Table raw access and explicit Value variants are available | Decode without coercion/metamethod execution and cap traversal before allocation. [Table](https://docs.rs/mlua/0.11.6/mlua/struct.Table.html), [Value](https://docs.rs/mlua/0.11.6/mlua/enum.Value.html) |

`Lua::sandbox` and `set_interrupt` are Luau-only APIs; do not accidentally claim
them for this lua54 build. No executable capability spike was necessary to choose
this design. Sol must verify the actual selected feature build and budget/error
behavior with tests before advertising support. If version resolution or an API
differs, record it; do not silently enable unsafe libraries to obtain a green build.

## 4. Data-only script contract and state ownership

Source is bounded UTF-8 text returning exactly one function accepting one context
table. That function handles `phase = "init"` or `"step"`. Top-level chunk execution
and the function share one per-job work/call/deadline budget. This source convention
is a component callback contract, not an executable application profile.

Every job constructs a **fresh VM**, executes the same immutable source, invokes
its function, validates/copies the result and destroys the VM on its worker. No
mutable Lua global, upvalue, registry object, coroutine or compiled closure persists
between invocations. Persistent state is exclusively the bounded plain-data state
committed by Runtime and copied into the next job. This deliberately trades small
POC throughput for explicit reset, failure atomicity and teachable ownership.

No source/global mutation can modify committed state or another generation. VM
reuse/cached mutable closures would change this guarantee and require later review.
The only retained source is the validated immutable text for a committed/staged job.

### Input context

The script sees only copies of:

- `phase`, configuration and last committed state;
- declared output unit ID; for Transform, one input's numeric value, unit ID,
  Good quality and original observation time;
- Runtime elapsed seconds and dt, finite f64, converted from trusted monotonic
  Duration. First step has dt=0; subsequent Transform dt uses distinct input
  observation times, Source dt uses scheduled model times. Keep authoritative
  Duration outside Lua; no script-supplied timestamp is accepted;
- no IDs/tokens needed for actuation. Full-width generation/attempt correlation
  stays outside the VM to avoid numeric precision/forgery problems.

Configuration/state use a deliberately small grammar: a string-keyed map of at
most 16 fields. Keys are nonblank ASCII, at most32 bytes. Values are finite numeric
scalars, booleans, UTF-8 text at most128 bytes, or dense arrays of at most64 finite
numbers. No nested maps, nested arrays, sparse/mixed keys, metatables, object
graphs, functions, userdata, threads or cycles. Total leaves at most256, encoded
logical size at most4096 bytes per config/state. Size accounting is explicitly
key bytes + scalar tags/payloads + length prefixes + array payloads, independent
of Rust object overhead; both node and byte bounds are checked before copying.
Choose documented checked accounting constants in implementation, not JSON size
after already allocating an unbounded tree. Configuration schema/allowed keys
belong to the trusted manifest supplied by deployment, not to script side effects.

### Result context

Init returns exactly a bounded initial state and diagnostics. It creates no sample,
increments no warm-up count and performs no effect. Step returns exactly:

```text
status: "warming" | "ready" | "unavailable"
value: finite numeric Float candidate only for "ready"; absent otherwise
unit_id: exact declared output unit identity
state: bounded candidate replacement state
diagnostics: zero to four bounded text messages
```

Unknown keys reject, including `command`, `lease`, `bytes`, `ack`, `readback`,
`safe`, addresses or fabricated timestamps. Numeric strings/booleans do not
coerce to Float. Lua Number and exactly representable Integer within +/-2^53
are accepted numeric representations; validate finite/range again in Core.
Unit identity is checked; symbol comes from the trusted descriptor. Lua cannot
declare a unit conversion at return time. M5's selected filter requires identical
input/output units; explicit future unit-changing transforms need manifest policy.

Worker conversion uses raw field iteration/access, rejects metatables and applies
depth/count/byte checks before cloning data. It must stop at limit+1; never collect
the full Lua table and check afterward, invoke tostring, or call metamethods.
Rust then validates the typed result again against the immutable current manifest,
current generation, request, input lineage and deadlines. Only a completely valid
current result commits state, count, diagnostics and sample together.

An unavailable result commits a failed attempt and failure status, not candidate
state. Retained state remains diagnostic only. No partial result or error carries
an implicit default value, output proposal or successful prior reading.

## 5. Allowlisted environment and host calls

Start from `Lua::new_with(StdLib::NONE, ...)`, then execute text in a newly
constructed environment without a fallback to VM globals. Base globals that
mlua/Lua installs internally are not automatically reachable from that environment.
There is no library-table cloning from an unrestricted `Lua::new()` instance.

Available facilities are exactly:

- Lua language arithmetic/comparison/boolean operators, local functions, conditionals,
  loops, plain tables, string literals/concatenation and length;
- `type(value)` as a bounded type classifier;
- `error(message)` as a host wrapper accepting only UTF-8 text up to256 bytes;
- `math.abs`, `floor`, `ceil`, `sqrt`, `exp`, `log`, `sin`, `cos` (one finite numeric
  argument), and `math.min`, `max` (exactly two finite arguments), plus finite `pi`.
  These are small explicit trusted Rust scalar wrappers, not the whole math library.
  Reject nonfinite arguments/results and wrong arity/types without Lua coercion.

All callable wrappers count toward the same at-most128 host-call budget and check
the job's monotonic deadline/cancel flag before work. They cannot accept a Lua
function, call back into Lua, wait on a channel/lock/I/O, or invoke Runtime. Arguments
are read in bounded arity; do not first materialize an unbounded variadic argument
vector. Diagnostics are returned data; no synchronous log/print callback exists.

Absent: `_G`, package/require/load/loadfile/dofile, io, os, debug, coroutine,
pcall/xpcall, collectgarbage, print/warn, string/table/utf8 standard libraries,
metatable/raw-access functions, random/randomseed, FFI, native modules, environment,
filesystem, sockets, process APIs, ByteTransport and all application commands.
In particular, do not load the string library and leave it reachable through a
string metatable after merely removing its global. Without pcall/coroutines/debug,
scripts cannot catch/reset the instruction hook or move execution to an unhooked
Lua thread. Tests must probe transitive capabilities, not only a list of names.

The environment itself can be modified by the invocation, but contains no hidden
capability and is discarded with the VM. A script assigning `_ENV` or shadowing
math cannot change host counters, committed configuration or the next invocation.

## 6. Budgets and their precise guarantees

These are initial M5 POC admission constants, not hardware response guarantees.
Tests cover exact bounds; production settings may lower them. Raising them requires
documenting the new work/memory envelope, not accepting script-selected limits.

| Resource | Bound and enforcement |
| --- | --- |
| Committed components | 8, additionally within existing 64-instrument limit |
| Staged candidates | 1 globally; invalid/rejected stages release their bounded data |
| Workers/VMs | 2 worker slots, at most one VM/job each; quarantine consumes a slot |
| Pending work | 1 per component; 1 job and 1 completion mailbox slot per worker; no FIFO backlog behind an active worker |
| Source | 32 KiB UTF-8, checked by host before admission; Text mode only |
| Invocation data excluding source | 12 KiB and at most512 scalar leaves, checked before marshaling |
| Configuration / persistent state | 4 KiB each, 16 fields/256 leaves/64 array elements, section 4 grammar |
| Entire converted result | 8 KiB/512 leaves, includes state, value and diagnostics |
| Diagnostics | 4 messages, each256 UTF-8 bytes, aggregate1024; excess rejects the result |
| Error returned to Core | Fixed typed cause plus at most256 bytes of safely extracted text, no complete traceback or arbitrary Error Display formatting |
| Lua accounted heap | 8 MiB per VM; set before loading source/marshaling script data; constructor-only trusted allocation precedes this, check used_memory before exposure |
| Lua execution | 100,000 VM instructions for top-level plus init/step; hook every100 instructions; counter never resets between phases; exhaustion rejects |
| Host calls | 128 total for the entire job, including type/error/math wrappers |
| Total job deadline | 100 ms from host admission, including queue wait, VM construction/load/init or step, conversion, validation and cleanup |
| Host maintenance quantum | Target <=10 ms for nonblocking safety/component service in the M5 finite harness; measured observation, not a hard OS guarantee |

Instruction accounting can overshoot by fewer than one hook interval; measure and
document the counter convention. Parser/GC/native work does not consume this
counter reliably. Heap quota bounds Lua allocator accounting, not stacks, Rust
mailboxes, allocator metadata, process RSS or survival under native crash/system
OOM. Two workers imply at most16 MiB accounted Lua heaps; source/state/copy/queue
counts above bound retained extension data separately. Choose a fixed worker stack
size (initially2 MiB); native stack exhaustion is a process-level risk, not a caught
Lua budget success. Recursive-script tests must demonstrate ordinary Lua errors
are isolated under this configuration without claiming arbitrary native crash safety.

The deadline is an **acceptance and dependency deadline**. Hook/wrappers also check
it, but a VM may physically continue after100 ms if native execution/GC stalls.
Runtime independently expires the job on its next service opportunity, invalidates
the dependent branch and discards all late results. No in-process forced thread
termination, exact CPU preemption or hard-real-time completion claim is made.
Failure to service the host at all is a Runtime/OS stall, already outside M4's
software-only safety guarantee.

Use two clocks with explicit roles: Core uses supplied monotonic Duration for
sample freshness/leases; the adapter uses monotonic Instant for the100 ms execution
deadline. Clock reads are trusted host operations and neither is wall civil time.
The finite harness maps one Instant origin to Core time and services all independent
work. Fake-executor tests inject deadline outcomes; real Lua tests exercise actual
hooks/quota plus an outer test timeout. A completion must be accepted before both
the adapter deadline and Core's input/attempt validity expire.

## 7. Scheduling, quarantine and shutdown

The host starts two workers during composition. Runtime submits only to an idle
slot using try_send; no free slot returns Busy without admitting work or allocating
another thread. One component cannot have two outstanding callbacks. The caller
decides when to retry rejected work; no automatic unbounded catch-up queue. Fair
round-robin admission in the finite demonstration suffices for two proof components.

Workers own VM creation, compilation, invocation, all conversion and destruction.
They send only bounded owned data after successful cleanup; a slot is not reusable
until cleanup has completed. A worker may block waiting for its own next job, but
Runtime never waits for it. Mailbox sends from a worker also use nonblocking/bounded
semantics; a cancelled receiver must not strand that worker in a send forever.

Every host service turn performs:

1. Core safety service: advance OutputAuthority watchdogs, controller freshness/gap
   checks and reserved safe work; progress existing transport recovery.
2. Expire component deadlines independently of a result message; atomically mark
   affected outputs unavailable and fault/revoke dependent controllers.
3. Poll at most one completion per worker, validate and commit current results.
4. Service due native controllers/acquisition and admit bounded new component jobs.

Implement a narrow `ServiceSafety { at }` command/helper as needed: at most64
controllers/outputs, no renewal and no Lua wait. It detects missing/stale input or
overdue controller progress even when nobody submits TickController. It drives
Rust virtual safe dispatch and keeps M3 recovery bounded; physical safe completion
remains subject to existing transport/evidence limitations. Ordinary queries stay
pure. Reuse the same failure helper as M4 rather than duplicating controller rules.

Expired slots are quarantined until the worker exits that job, including cleanup.
A cooperative return releases capacity but its result is discarded permanently;
the component remains Failed until explicit Reset/Reload. No replacement workers
are spawned while quarantined ones still exist. If both slots hang, all new Lua
work returns Unavailable/Busy, while native work and safety remain serviceable.
The host owns one process-lifetime supervisor/slot ledger; dropping/recreating a
component or facade cannot bypass this quota. Repeated load/reload/timeout must
not accumulate detached workers or VM generations.

Cancellation is a flag plus invalidation, never the physical undoing of a callback.
No Runtime/authority mutex is shared with a worker. Panic-unwind handling at the
worker boundary may classify worker failure, but cannot claim recovery from abort,
VM memory corruption or native stack exhaustion.

Shutdown closes admission, revokes affected authority and performs Rust safe work
before requesting worker cancellation. It reports unfinished/quarantined workers
after a bounded grace period (initially200 ms) without waiting in Drop/join on the
safety lane. Join only known-finished workers outside critical service. Rust's
[JoinHandle documentation](https://doc.rust-lang.org/std/thread/struct.JoinHandle.html#method.is_finished)
supports nonblocking finished checks; dropping a handle does not stop execution.
The supervisor retains/accountably abandons outstanding slots until process exit;
it must not silently restart an unlimited pool. Healthy native control must remain
usable if the optional Lua host is disabled before whole-process shutdown.

## 8. Quality, warm-up and time integration

Runtime owns component lifecycle `Loading` (candidate), `Warming`, `Ready`, `Failed`.
Loading candidate is not yet a published usable instrument. Successful init commits
an initial generation in Warming with no Good output, count0 and explicit pending
readiness. Source defaults to warmup1; moving mean uses3.

A valid step counts only a new valid input/model time. Before manifest warmup N is
reached, Ready is rejected as premature; Warming carries no control-usable value.
After N, Ready is allowed, but a component may continue reporting Warming. A Ready
component cannot regress to Warming silently; use explicit Reset or fail the result.
PID consuming such a signal uses corrected M4 warm-up/freshness rules and cannot
arm from a partial Lua window. Lua warm-up and native EMA warm-up are separate
counts: neither manufactures the other's readiness.

For Transform, absent/stale/unavailable/wrong-type/wrong-unit input is rejected by
Runtime before invoking Lua. Record Unavailable and fail the component; do not call
Lua to salvage the last Good value. Ordinary recovery is explicit Reset/Reload
and re-warm. A valid returned Unavailable has the same conservative failure policy.
This avoids implementing a speculative gap/hold-last-value policy in M5.

The current Sample has only one timestamp. M5 must add a small additive
`freshness_at()` field/accessor (or equivalently named typed provenance) so that:

- Native M1–M4 samples have freshness_at == at; their behavior remains unchanged.
- A managed output's at is the trusted Runtime publication/attempt time; Source
  freshness_at is its scheduled model time, Transform freshness_at is inherited
  from the input's original freshness time, never callback completion.
- Core constructs these times from retained invocation metadata, validates
  freshness_at<=at, and ignores/rejects script-supplied time fields. Controller
  freshness and renewal use freshness_at; EMA distinct-input timing uses the
  observation time, not artificial publication latency.

Compute dt as a checked Duration difference before converting to Lua seconds; do
not subtract two rounded absolute f64 timestamps and treat that as authoritative.
Keep component generation/lineage in Runtime registration/job metadata; M5 does not
need to add every future event sequence or persistence field to Sample.

Runtime retains the exact input signal/generation/observation stamp with each job.
A later unrelated Good input does not automatically invalidate an earlier job if
the captured input is still fresh, but input reset/reload, an Unavailable attempt,
or any lifecycle failure invalidates it. No pending job is silently replaced; new
admission waits or returns Busy. At completion recheck input lineage/quality and
age, not only result type. Slow computations must not refresh old evidence.

Add generic unavailable reasons for ProcessingWarmup and ComponentFailure instead
of mislabeling all failures Transport or inventing a third Good-like quality.
No value accompanies Unavailable. Failure/reset/reload must invalidate the latest
dependent sample immediately at trusted service time, even if its previous Good
was published at that same Duration. A private SignalBuffer invalidation operation
may replace the current tail with Unavailable on equal time; it must never replace
Unavailable with Good at equal time. Ordinary push/Refresh remains strictly
increasing, preserving M1 tests. Older returned snapshots are immutable copies;
all subsequent queries see invalidation. This bounded window is not durable history.

Successful asynchronous publications still require an increasing output timestamp;
the host services accepted results at increasing times per component. No fabricated
epsilon timestamp is allowed. A late/duplicate/non-increasing result cannot commit
state or a Good sample. Same-time failure remains allowed by the explicit rule above.

Failure propagates through the fixed M5 dependency chain and registered controllers
in the same bounded service turn. Dependent controllers become Failed and stop
renewing, with Rust-owned Trip/safe action; unrelated native controllers continue.
The authority survives unusable or destroyed Lua VMs. A new valid generation never
automatically restarts a failed/paused controller.

## 9. Atomic reset, reload and generation fencing

Correlate every job with Runtime instance, ComponentId, generation, attempt and
expected committed state revision. Counters are checked and never wrap/reuse. Core
retains the correlation outside Lua. A single in-flight invocation per component
keeps state commits serial. No general distributed request ID scheme is introduced.

Reload/configuration replacement is staged:

1. Host bounds source/configuration and Core validates manifest and bindings. Keep
   the current committed generation and its accepted state/observations untouched.
2. Use a spare worker for candidate text compilation and bounded init. No capacity
   means explicit Busy; do not stop the old generation just to make room. Syntax,
   exception, limit or schema failure rejects candidate, preserving old generation.
3. After valid init, Core rechecks expected current generation/binding. Pause any
   affected Warming/Running native controller through the ordinary Rust lifecycle;
   require resolved virtual safe output before committing replacement. If safe
   transition fails, reject replacement and retain the old committed definition
   with honest output fault state, never auto-resume it.
4. In one commit increment generation, invalidate old pending correlation and
   dependent signals, install candidate config/state, reset count and enter Warming.
   Request cancellation of old work, but do not wait for its worker to finish.
5. Old results, even valid and numerically identical, are rejected. New work uses
   only the new generation and state. Publication cannot revive a dependent controller.

Descriptor identities and input/output bindings remain fixed across M5 reload;
only script/configuration of the same validated shape may change. Topology/unit/
actuator mutation is not a script hot-reload capability. Reset follows the same
new-generation staged init policy with the existing source. No state migration or
serialization of arbitrary VM internals. A candidate init is validation, not proof
that all future inputs succeed; a later step error legitimately fails the new
generation. Post-quiescence hardware effects are not transactionally rolled back.

## 10. Failure matrix

| Cause | Committed component and signals | Controller/output effect |
| --- | --- | --- |
| Invalid manifest/config/source size | Reject before worker/admission; old generation intact | None |
| Candidate syntax/load/init exception or budget failure | Reject candidate; old config/state intact; failed worker slot quarantined if still executing | None before quiescence; never arm candidate |
| Committed step exception, forbidden function call, recursion error | Failed; explicit Unavailable; discard candidate state | Dependents fail/revoke; Rust safe action; unrelated native work continues |
| Instruction or host-call budget exhausted | Failed/Unavailable; quota cause retained; no catch/retry in script | Same dependent-only fault |
| 100 ms job deadline, including construction/parse/GC/cleanup stall | Failed/Unavailable at next service; slot quarantined while alive; all late results rejected | Same; no wait for VM or worker |
| Lua quota MemoryError | Failed/Unavailable; no allocation-dependent giant error formatting; VM disposed on worker | Same; not a claim of process-OOM isolation |
| Wrong type/unit, NaN/Inf, forbidden result key, range violation | Reject entire result/state; Failed/Unavailable | Same |
| Oversized/cyclic/sparse state/result or too many diagnostics | Stop bounded traversal, reject all result/state, Failed/Unavailable | Same |
| Warming return during required warm-up | Commit bounded state/count and Unavailable(ProcessingWarmup) | No start/lease from this input; a pre-existing running dependent must fail if readiness is lost |
| Bad/stale input before invoke or stale lineage at completion | No usable output; invalidate pending result, Failed/Unavailable | Same dependent fault |
| Reset/reload while old callback runs | Old valid config until candidate commit; new generation Warming at commit; old result ignored | Explicit safe pause before replacement; no automatic restart |
| Duplicate/late old-generation result | Discard, saturating bounded diagnostic counter; never change current generation | Cannot restore authority or new safe evidence |
| Worker unwind/disconnect | Fail its pending component; retain fixed slot accounting until exited | Native control and independent safe service continue |
| Both slots quarantined | Lua admission unavailable; finite retained data, no thread replacement leak | Existing native paths continue; dependencies expire/fail |
| Native VM/allocator corruption or process abort | Outside embedded exception isolation | Requires process/hardware protection; never represented as successful software safe completion |

There is no global Runtime fault for ordinary extension exceptions or validation
errors. Generation/attempt/state counters saturate diagnostically or fail closed
on identity exhaustion; they never allocate an unbounded fault log.

## 11. Physical evidence boundary stays Rust-owned

M3's write acknowledgement remains acknowledgement. An output SafeProfile requiring
Readback cannot become confirmed from ACK until a trusted Rust readback/reconciliation
path exists. No M5 code implements that missing physical workflow as a Lua parser.

Lua cannot receive, return, or invoke ACK/readback/safe completion types. The port
accepts only component data. Existing `Command::Output::Complete` is trusted M2
simulation input; do not expose it to scripts or future client DTOs. Managed
components cannot register an actuator or pass a fake physical operation result.
Keep all existing Metakon byte/ACK tests and add evidence non-fabrication tests.

Rust's virtual dispatcher may supply simulation ReadbackVerified for the known M4
plant. It must not infer physical readback from the existence of any virtual/model
component, a Lua value, or a descriptor name. The core output query keeps sent,
acknowledged, readback and unknown separate.

## 12. Future extension points, explicitly not M5 APIs

For a future Lua-backed physical adapter, Rust owns an immutable named-operation
catalog: resource binding, address/channel/register or template, argument type/unit/
limits, side effects, deadlines, response bound/shape, retry policy, evidence source
and output-authority requirement. Ordinary Lua can name only a scoped catalog entry
and supply bounded arguments; it cannot change metadata or choose raw bytes.

```text
Lua candidate -> typed Rust OutputProposal -> OutputAuthority
    -> immutable Rust operation mapping -> first-byte recheck -> transport
    -> Rust protocol evidence (ACK distinct from required readback)
```

Rust renders output bytes from the exact authorized value after validation; Lua
cannot serialize a substitute value/address afterward. Safe actions and required
evidence must run with Lua dead. A device whose safe operation/evidence requires
arbitrary Lua remains read/model-only. M5 supplies no callable named-operation
catalog, no physical Lua writes and no arbitrary text parser endpoint.

A future Lua controller can use the same bounded job/result/generation mechanism,
with a distinct trusted Runtime controller registration and result-as-candidate
contract. Runtime must own lifecycle, warm-up, freshness, failure and finite renewal
based on accepted timely results, never a live VM or client session. Script gets
no lease and cannot call renewal. Exceptions/timeouts/nonfinite results invalidate
health, stop renewal, revoke and run Rust safe. Native renewal privilege from the
M4 correction is not automatically granted to every Automatic owner or component.
This future extension requires its own reviewed actuation/renewal acceptance, not
a feature flag in this milestone.

## 13. Tests-first acceptance for Sol

These are required future tests, not passing claims. First complete W1–W9/R1–R10
in the M4 lifecycle review and verify M1–M4. Then capture red domain-contract tests
and real embedding tests before implementing their bodies. Use deterministic
executor tests for ordering, and actual mlua jobs for sandbox/limits/conversion.

| ID | Required assertions |
| --- | --- |
| L1 packaging | Core dependency tree remains std-only; only lab-lua depends on mlua with chosen features; VM types never cross mailboxes/domain API. All code/docs/comments English, missing_docs enforced. |
| L2 generic model | Register two script/config variants without Core branches; source init publishes no fake Good; finite steps produce expected temperature through ordinary discovery/latest/window. Invalid config is atomic. |
| L3 filter proof | Inputs1,2,3 at unequal increasing times produce two explicit warming attempts then ready mean2; count/state bounded. Native EMA's separate warm-up and first permitted PID output are observable. |
| L4 capability closure | Actual scripts cannot reach filesystem/process/env/network/FFI/raw transport, including `_G`, package.loadlib, require, debug, string-metatable access, coroutine, pcall, load and bytecode. Test nested/indirect attempts, not only global nil checks. |
| L5 return validation | Ready numeric valid result succeeds; numeric text/bool, wrong unit, premature Ready, NaN/Inf, out-of-range, unknown evidence/bytes/command keys reject without committing state. |
| L6 plain-data bounds | Exact limits accepted; limit+1 config/state/source/result/diagnostics rejected; cyclic/deep/sparse/function/thread/userdata tables fail with bounded traversal. No unbounded clone/format on error. |
| L7 state atomicity | A callback mutates input config/state/environment then throws or returns invalid data: committed state/config unchanged. Next successful invocation sees only previously committed state; globals/upvalues never persist. |
| L8 input/age | Missing/Unavailable/stale/wrong-unit input avoids VM execution, fails branch; original observation time survives a slow valid transform. At exact freshness expiry even an otherwise timely result is unusable and cannot renew dependent control. |
| L9 instruction/call budget | Infinite loop and repeated wrapper calls fail under actual mlua; top-level and callback share counters; cannot catch/remove hook. Near-limit valid finite script succeeds; counter convention/overshoot documented. |
| L10 memory and parser | Actual Lua allocation growth hits quota; oversized result within heap hits data cap independently; syntax/deep recursion return isolated failure. Parser/top-level stall is subject to external deadline even if no instruction hook fires. |
| L11 hard-stall model | Fake executor blocks until a test-controlled release after deadline; Runtime service returns without waiting, native PID completes several ticks/renewals, authority safe action and another bus recovery progress before worker release. Use barriers/channels, not scheduler-speed guesses. |
| L12 clock/deadline | Before deadline accepted; at/after100 ms rejected; include time spent in construction/load, conversion and cleanup. Late current-generation completion after timeout is discarded, not merely old-generation results. Test Instant via injected adapter clock where practical and real timeout with outer harness watchdog. |
| L13 bounded workers | Two quarantined jobs plus repeated register/reload requests never exceed two worker slots, eight components and one stage; no blocked Runtime send/join. Release workers and show capacity recovery does not auto-restart failed components. |
| L14 isolated failure | Real Lua exception/infinite loop/quota failure affects only its component/dependents; an unrelated native PID remains Running across renewed lease lifetimes. Dependent native PID becomes Failed, lease revoked, virtual safe confirmed and output FaultLatched. |
| L15 safety independence | With all Lua workers failed/stalled, independent ServiceSafety expires a manual lease and native stale dependency, performs Rust virtual safe, and preserves M3 recovery. It does not renew leases or depend on any Lua callback success. |
| L16 reload | Valid staged replacement increments generation, resets state/count, re-warms, invalidates dependent latest and requires deliberate controller restart. Old in-flight valid result cannot commit. |
| L17 invalid replacement | Malformed config, syntax/init failure, oversized state, deadline and Busy leave old committed generation/config/state usable; failed safe quiescence retains old definition plus honest fault, not automatic rollback/rearm. |
| L18 same-time invalidation | Good sample then failure/reset/reload at the same service Duration becomes latest Unavailable with no value; old snapshot copies stay immutable. Ordinary M1 push/refresh still rejects equal time; late Good cannot overwrite invalidation. |
| L19 evidence | Lua result claiming ACK/ReadbackVerified/safe or an actuator descriptor rejects. Fake Metakon write ACK with Readback-required profile remains insufficient. Failed Lua cannot disable Rust safe, and cannot inject Command::Output::Complete through any binding. |
| L20 identity | Wrong Runtime/component/generation/attempt/state revision, duplicate and expired result all reject without mutation; checked generation exhaustion fails closed. Renaming preserves ID; reload changes generation even with identical descriptor. |
| L21 failure cleanup | Cancel/disable/shutdown with a blocked worker returns a bounded incomplete outcome, no join in Drop; worker-count ledger survives component recreation. Test-only blocked worker is released and joined during harness cleanup. |
| L22 finite demonstration | Separate finite M5 example/test runs model -> filter -> native controller -> existing Rust plant output, and an unrelated native path; demonstrates warm-up, fault and reload with generic snapshots. Original M1 demo remains finite and green. |

The finite threaded integration harness must keep servicing native/safety work
while a Lua result is absent. A test that calls Lua synchronously and only resumes
PID after it returns does not satisfy isolation. Real runaway tests need an outer
test-process watchdog so a regression cannot hang the whole verification run;
no subprocess is introduced as the production extension architecture.

## 14. Implementation order, documentation and stop

Sol sequence is fixed:

1. Implement M4 Warming/guard and native renewal corrections with red/green regression
   evidence, retained failure state and unchanged M3 byte/evidence tests. Update M4
   implementation docs and run the full verification gate.
2. Add language-neutral component data/registration/nonblocking port and fake-executor
   tests in Core; add independent safety service and freshness provenance required
   for delayed derived samples. Do not add arbitrary mutable Sample injection APIs.
3. Add lab-lua with the chosen embedding features, fresh-VM runner, strict environment,
   bounds and conversion tests. Keep interpreter errors typed and bounded.
4. Add fixed workers, deadline/quarantine supervision, state commit/reload fences,
   useful model/filter fixtures and dependency safety integration.
5. Complete every L1–L22 gate; run fmt, debug/release workspace tests, clippy -D warnings,
   rustdoc, finite M1/M5 demonstrations, feature/dependency audit and diff checks.
6. Write `docs/implementation/MILESTONE_5_REPORT.md`, update the sole `AI_HANDOFF.md`,
   record exact tests/version/features/limits and any measured deadline overruns.
   Stop after M5 for external review; do not begin M6 design or implementation.

Small logical commits should separate accepted inputs/design, tests, production
subsystems and completion documentation. Do not retrofit claims of passing tests
into this design before implementation. A failed library capability gate may
narrow the implementation only within this approved model/filter contract; changing
trust, quotas, physical capabilities or isolation requires explicit review recorded
in AI_HANDOFF rather than silently relaxing the gate.

English module/rustdoc/private-invariant comments must teach VM and data ownership,
capability reachability, limits and their exceptions, why native calls are bounded,
why Runtime never waits, cancellation versus termination, state atomicity,
freshness lineage, generation fencing and Rust-only evidence/safe execution.
Prefer explicit intermediate state and decisions over compressed abstractions.

## 15. Design review outcome and limitations

The two reviewer lifecycle issues have a concrete prerequisite contract. M5 now
has a bounded useful proof, an implementation path consistent with the inspected
mlua APIs, and independent failure/acceptance semantics. No unresolved architecture
fork requires a decision before handing this implementation contract to Sol.

Deliberate limits: observation/model Lua only, no Lua controller or physical Lua
operations; fresh VM per job; bounded non-preemptible worker quarantine; finite
harness rather than M6's long-lived client service. Embedded workers isolate
ordinary script failures and waiting dependencies, not native crash/system OOM
or OS starvation. Lua-derived measurements remain simulated/untrusted algorithm
data. M3's missing generic physical readback path remains explicit and unchanged.

Handoff: ASTRA_HIGH -> SOL_HIGH. Implement approved M4 corrections, verify M1–M4,
then implement M5 only. Do not cross the next review boundary automatically.
