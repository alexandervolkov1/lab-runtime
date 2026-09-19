# Extending the Runtime

Extensions are explicit compile-time Rust composition. There is no dynamic plugin
loader, scripting runtime, filesystem driver discovery, or dependency-injection
framework.

## Add a native managed component

The production example is `native.moving_mean.v1`:

| Concern | Source |
|---|---|
| bounded algorithm, stable implementation ID, properties | `apps/lab-runtime/src/managed_executor/moving_mean.rs` |
| one compile-time registration point | `apps/lab-runtime/src/managed_executor/registry.rs` |
| bounded two-slot executor | `apps/lab-runtime/src/managed_executor.rs` |
| language-neutral contracts | `crates/lab-core/src/managed.rs` |
| deployment DTO/validation | `apps/lab-runtime/src/configuration.rs` |

Normal recipe:

1. Add a sibling implementation module. Implement composition, invocation
   validation, and bounded execution over `Invocation`/`ComponentResult`.
2. Assign a stable `ComponentImplementationId` string and declare configuration
   property metadata beside the implementation.
3. Add one explicit `Registration` and match arm in `registry.rs`.
4. Add tests for configuration, Init, Step/state, units, bounds, warm-up, timeout,
   stale completion, and generic projections.
5. Add a deployment instance only when the component should be instantiated.

The implementation must:

- check its finite deadline/cancellation at meaningful work boundaries;
- return finite values in the declared unit/range;
- keep `PlainData` configuration and state within the Core bounds;
- model warm-up explicitly rather than fabricate an initial value;
- accept only the declared input shape and generation/revision semantics;
- avoid all Runtime, transport, SQLite, client, and `OutputAuthority` access.

The static registry supplies constructor/dispatch identity and generic property
metadata. The ordinary component definition registers a normal signal, so discovery,
current measurements, recent history, subscriptions, controller input, Recorder
facts, durable history, and native build/configuration provenance follow without
component-specific Application, Recorder, or SQLite code. The test-only
`managed_executor/reference_component.rs` proves a second registration through
these real paths.

## Add a physical instrument

Metakon is the current physical reference implementation. Its extension path is
explicit rather than hidden behind a universal protocol trait:

| Concern | Existing reference source |
|---|---|
| semantic instrument/parameter descriptors and binding | `crates/lab-core/src/instrument.rs` |
| strict codec and typed values | `crates/lab-core/src/metakon.rs` |
| Runtime transaction admission/completion/measurement commit | `crates/lab-core/src/runtime/physical_io.rs` |
| deployment kind, validation, and neutral properties | `apps/lab-runtime/src/configuration.rs` |
| compile-time instrument composition | `apps/lab-runtime/src/host/instruments.rs` |
| cadence/admission/completion orchestration | `apps/lab-runtime/src/host/scheduler.rs` |
| serial worker and settings | `apps/lab-runtime/src/serial.rs` |
| output safety adapter path | `crates/lab-core/src/output/authority.rs` and Runtime physical I/O |

Normal recipe for a new ordinary physical instrument:

1. Define stable semantic identity and parameter descriptors, including signal,
   unit, value range, role, access, and write effect.
2. Implement a strict protocol codec and typed transaction outcomes. Keep
   protocol-specific framing in its adapter/module.
3. Add an `InstrumentDto` variant and validation. Declare its ordinary properties
   through neutral `InstrumentPropertyMetadata`; do not import Application JSON DTOs.
4. Add one explicit composition arm in `host/instruments.rs` that registers the Core
   instrument/signals and its physical resource binding.
5. Add protocol-specific periodic scheduling and completion correlation. Preserve
   finite queue/deadline and stale-generation checks.
6. If the device has output, integrate the trusted authority-gated adapter described
   below. A read-only device needs no fake output hook.
7. Add deployment examples/definitions and deterministic codec, configuration,
   fault, acquisition, reconnect, and (if applicable) output tests.

The `InstrumentDto` configuration layer may dispatch among concrete instrument
kinds because it owns their configuration. `configuration_api` consumes only
neutral property metadata and must not add a concrete-instrument match. Ordinary
signal descriptors enter the existing generic paths automatically:

```text
instrument signal
  -> discovery
  -> measurements_current / latest
  -> Runtime recent history
  -> subscriptions
  -> controller input
  -> Recorder facts
  -> durable history
```

An ordinary instrument therefore should not require an instrument-specific
Application handler, generic history/subscription change, Recorder branch, or SQLite
schema change. New public operations are appropriate only for genuinely new domain
semantics, not for ordinary signals or properties.

For an Arduino-like thermal furnace, the expected changes are dominated by its
codec/typed transactions, `InstrumentDto` variant and validation/property metadata,
host composition, protocol-specific schedule/completion adapter, optional
authority-gated output adapter, deployment definitions, and tests. The educational
Arduino integration itself is outside this preview reference.

## Physical output extension

A writable adapter must not expose a raw generic write capability. The only accepted
path is:

```text
controller / trusted proposal
  -> Runtime-owned OutputAuthority
  -> bounded ResourceExecutor reservation
  -> final authority + epoch + generation + mapping + deadline check
  -> typed transport WRITE
  -> strict ACK
  -> separate typed readback
```

The final check belongs immediately before the first possible byte. The adapter must
report the `send_started` boundary and preserve immutable correlation through ACK
and readback. Once a write may have started, an uncertain result is ambiguous and
must not trigger blind retry. A safe obligation does not itself authorize resending
an ambiguous safe command.

Application clients and managed components never receive leases, transport intents,
raw byte access, ACK authority, or a way to fabricate physical evidence.

## Configuration and provenance checklist

Before accepting a new extension, verify:

- stable IDs and keys do not depend on collection order;
- every signal has an explicit type, unit, bounds, generation, and history capacity;
- property access/mutation class (`live_safe`, `ordinary_live`, `reinitialize`, or
  read/deployment-only as applicable) matches lifecycle behavior;
- candidate validation is atomic and cross-references resources/controllers/safe
  profiles before activation;
- reconnect/replacement advances generation and fences old completions;
- semantic implementation/configuration/build identity reaches Recorder provenance
  through the generic activation path;
- all queues, transactions, state, errors, and shutdown behavior remain bounded;
- no extension changes public API, Recorder, or schema merely for presentation.

Use [Architecture and concepts](architecture.md) for ownership and
[Safety and failure behavior](safety-and-failures.md) for the authority/evidence
contract.
