# Milestone 5: bounded Lua observations — implementation report

## Result and scope

The corrected M1–M4 verification gate passed at 92 tests before any Lua dependency
was introduced; the original checkpoint was 72 tests. M5 adds a language-neutral
Core component boundary and a separate `lab-lua` host. The finite application test
executes a real Lua temperature model, a three-observation Lua moving mean, a native
EMA/PID, Rust OutputAuthority and an existing virtual thermal plant. A separate
native loop stays Running when the model deliberately fails. Reload replaces both
Lua generations and re-warms the branch; the faulted controller resumes only after
Rust virtual safe readback, deliberate fault acknowledgement and reset.

M5 has **123 workspace tests**, passing in debug and release. The existing M1 finite
executable still exits normally. Tests do not touch a physical actuator or the v1
donor. Lua has read-only observation capabilities; physical output arbitration,
dispatch and evidence remain Rust-owned.

## Ownership and scheduling

`Runtime` owns bounded manifests, config/state maps, one staged candidate, eight
committed components, accepted samples, original freshness lineage, attempts,
revisions and generation correlation. `ComponentExecutor` exposes only nonblocking
submit/poll/cancel/expiry. Source init publishes no fake Good. Transform captures
one typed fresh Good input before admitting work and checks it again at acceptance.
Successful Step publishes at trusted service time while retaining its original
source/input observation time for freshness, EMA and native lease decisions. Equal-
time failure/reload may replace a Good latest sample with Unavailable; an old
snapshot stays unchanged and a late Good cannot restore that time.

The host creates exactly two fixed worker threads, each with a 2 MiB stack, fixed
one-job/one-result mailboxes and its own disposable Lua VM. A process-global atomic
reservation prevents concurrently constructed pools from splitting the capacity;
blocked worker slots stay counted until they actually exit. Runtime never waits for
the VM or acquires a worker-held output lock. Each job has a host `Instant` deadline
of 100 ms from admission through VM construction, loading, callback, result
conversion and VM destruction. A completed result polled at/after its deadline is
rejected; Core also expires pending work independently on its next service turn.
Worker cancellation is advisory and does not certify that a stalled native call has
ended. Supervisor shutdown closes admission and returns its unfinished-worker count
after at most 200 ms of grace; it joins only finished threads. A test-only controlled
runner blocks both actual slots, verifies the incomplete count, releases them and
joins them during harness cleanup.

The real Lua VM is constructed with `StdLib::NONE`; scripts receive only copied
`phase`, finite times, unit ID, plain config/state and optional finite typed input,
plus strictly counted `type`, bounded `error` and a handful of finite scalar `math`
wrappers. The guest `_ENV` has no standard-library fallback. A private arity gate
counts even extra `nil` arguments without exposing its trusted `select` upvalue.
Guest chunks are text only; each invocation uses a new VM so guest globals and
upvalues cannot persist. Results contain only status, optional finite number,
exact unit, bounded plain state and bounded diagnostics. Unknown fields (including
ACK, readback, bytes, actuator and safety claims), nested/cyclic/sparse data and
oversized returns reject the entire candidate before a state commit.

`lab-core` still has no external dependency. `lab-lua` alone depends on exactly
`mlua 0.11.6` with `default-features = false` and only `lua54`, `vendored` enabled.
The lock resolves `mlua-sys 0.10.0`; its vendored build-dependency metadata also
lists `lua-src 550.0.0` and `luajit-src 210.6.6+707c12b`, while the configured VM
feature is Lua 5.4. No mlua VM/handle crosses the Core component port.

## L1–L22 acceptance evidence

The referenced tests live in `crates/lab-core/tests/milestone5_managed.rs`,
`crates/lab-core/src/runtime.rs`, `crates/lab-lua/tests`,
`crates/lab-lua/src/supervisor.rs` and `apps/lab-runtime/tests/milestone5_*.rs`.

| Gate | Verified result |
| --- | --- |
| L1 packaging | Core std-only dependency tree, pinned isolated host/features, English docs and warning-free clippy/rustdoc. |
| L2 generic model | Two differently configured source definitions register through generic discovery and publish finite observations; the real Lua fixture computes both model variants. No Good on Init. |
| L3 filter | Actual Lua mean returns Warming twice and mean 2 on input values 1,2,3 at unequal times 0,2,5; Core/native M4 warm-up and permitted PID output are separately tested. |
| L4 closure | Real VM global/indirect string-metatable probes and private-arity tests reject standard libraries, `_G`, load/debug/bytecode and indirect capability reachability. |
| L5 validation | Real VM rejects numeric text, Boolean, wrong unit, nonfinite value, premature-value shape and forbidden fields; Core rejects invalid state/finite/range and premature Ready atomically. |
| L6 bounds | Core checks exact/+1 plain text, numeric array and source sizes; real VM rejects overfull keys/array, diagnostics, cyclic/sparse/nested state and heap allocation quota. |
| L7 atomic state | Real VM discards mutated input/config/globals between jobs; forged/invalid candidates leave Core committed state unchanged. Invalid replacement keeps generation 1 usable. |
| L8 freshness | Core rejects absent/unusable or exact-expiry input before a VM job; accepted slow transform retains original observation time instead of refreshing old evidence. |
| L9 budget | Real Lua infinite loop stops via hooked instruction quota under a 3 s outer child-process watchdog; 120 wrapper calls succeed, 129 fail; arity and private-hook reachability checked. |
| L10 allocation/parser | Real Lua allocation growth hits the 8 MiB VM ceiling; text-only invalid bytecode fails compilation; return size caps reject independently of heap. |
| L11 hard stall | Worker barrier keeps a model job blocked until the test releases it; during the block native renews from t0 through t16, Rust virtual safe/expired manual authority and M3 bus recovery progress. |
| L12 deadline | Real supervisor quarantines results polled after 110 ms, adapter clock distinguishes before/expired and fences late *current*-generation completion; actual blocked slots independently expire while still executing. |
| L13 capacity | Two slots cannot admit a third, including two expired quarantined workers; atomic concurrent-pool test, eight committed component cap and one stage slot; release restores capacity without auto-restarting Failed components. |
| L14 isolated failure | Deliberate real Lua model exception fails its dependent filter/controller, revokes lease and leaves virtual output FaultLatched with safe readback; unrelated native loop continues Running. |
| L15 safety independent | With the barrier worker blocked, native stale/gap service trips a controller, expired independent manual lease reaches simulated safe confirmation and an unrelated Metakon resource recovers; service does not wait on Lua. |
| L16 reload | Both real Lua generations increment to 2, invalidate dependents, reset count and rewarm; controller requires acknowledgement/reset/resume; a valid old in-flight result cannot restore Good. |
| L17 invalid replacement | Real syntax candidate and malformed configuration leave the old committed definition and Good sample intact; stage Busy/quota also rejects before admission; post-quiescence failure remains a visible fault. |
| L18 same-time | Core Good-to-Unavailable replacement at the same Duration and late-result fencing preserve immutable old snapshots; M1 normal same-time push still rejects. |
| L19 evidence | Real VM rejects ACK/bytes/actuator/safety fields; M3/M4 evidence suites preserve ACK != readback and allow only trusted Rust simulated virtual readback. |
| L20 identity | Forged Runtime/component/generation/attempt/revision, duplicate terminal and expired results cannot commit; old-generation in-flight result is fenced; checked generation exhaustion rejects without mutation. |
| L21 cleanup | The *actual* supervisor reports two unfinished blocked slots in about its 200 ms grace, refuses pool recreation, then releases/joins them; Drop never waits on workers. |
| L22 finite run | Full real Lua model/filter/native/authority/virtual-plant pipeline, fault, reload and generic snapshots pass alongside unrelated native control; original finite M1 demo remains green. |

## Verification and boundaries

On Windows with Cargo 1.95.0, `cargo fmt --all -- --check`,
`cargo test --workspace` and `cargo test --workspace --release` (123 in each
profile), `cargo clippy --workspace --all-targets -- -D warnings`,
`cargo doc --workspace --no-deps`, `cargo run -p lab-runtime`, all explicit
Lua/barrier/deadline integration tests and `git diff --check` exit 0.
`cargo tree -p lab-core` is std-only; `cargo tree -e features -p lab-lua` confirms
the feature boundary. The real worker deadline test sleeps 110 ms, after the
exclusive 100 ms acceptance time; bounded shutdown uses a 200 ms grace and the
test asserts less than 300 ms elapsed to allow scheduling overhead. The finite
M5 run settles in roughly 60–70 ms on this test machine, not as a real-time bound.

This is a software-only, in-process virtual proof. If the process, OS, native
Lua library or Runtime caller itself stops being serviced, the `Instant` deadline
does not force thread termination or prove hardware safe state. Physical safe
completion still needs trusted readback; an ACK is insufficient. The runtime has
no autonomous scheduler, COM deployment, physical output test, durable Recorder,
long soak or product parity. M6 design and implementation await external review.
