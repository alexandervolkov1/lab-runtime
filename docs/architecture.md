# System architecture

Applies to Application protocol v1 and the v0.1 product.

`lab-runtime` separates authoritative experiment ownership from operator
presentation:

```text
instruments and resources
           |
        Runtime
  (experiment authority)
           |
    Application API
       /        \
  Workbench   direct clients
 (presentation)
```

```text
Runtime owns experiment semantics.
Workbench owns presentation semantics.

Workbench lifetime != Runtime lifetime.
GUI lifetime != experiment lifetime.
external automation lifetime != experiment lifetime.
```

Closing, crashing, or disconnecting a client does not shut down Runtime, stop the
Recorder, pause a controller, or roll back work already admitted by Runtime.

## Runtime process

### lab-runtime

Runtime is the sole authoritative mutable experiment owner. It owns:

- instruments, resources, current measurements, and bounded recent history;
- References, controllers, output authority, and safe transitions;
- declarative configuration and resource generations;
- Recorder lifecycle and durable scientific/audit facts;
- Application sessions, scopes, mutation sequencing, deduplication, and retained
  operation outcomes;
- event ordering and the authoritative projections served to clients.

Runtime contains no presentation ownership. It does not own windows, panels, plots,
layout, or GUI selection state.

### Declarative SimpleDevice onboarding

The Application API can provision a bounded declarative SimpleDevice through
`stage_simple_device_candidate` followed by the ordinary configuration apply
lifecycle. A valid candidate becomes an ordinary Runtime instrument: its signals,
actuators, controller inputs, recent history, Recorder facts, and Workbench
projections use the same generic paths as native instruments.

Prepared SimpleDevice topology is physically inert and is not publicly discoverable
until publication succeeds. Publication holds the required bounded event capacity
before durable activation; after the durable activation boundary, the prepared
publication is installed as one serialized atomic owner operation. A resource
reconnect is fenced while a SimpleDevice publication is pending.

An API-provisioned overlay is process-local. It remains owned by Runtime across
client or Workbench disconnect, but is not restored after a Runtime restart; the
persistent deployment is restored instead. Failed pre-durable applies do not become
successful activation provenance.

## Client boundary

Clients own presentation and exact recovery evidence, never experiment state.
They consume committed snapshots and submit typed intent through the Application
API. Workbench remains an external client in the original repository.
No GUI, client worker, scripting host or broker is shipped here.

## Runtime layers

| Layer | Responsibility | Explicit non-ownership |
|---|---|---|
| `lab_core::Runtime` | instruments, committed signals/history, References, controllers, `OutputAuthority`, resource/component state, semantic Recorder-fact outbox | sockets, SQLite, OS serial ownership, presentation |
| `HostCore` | composes one Runtime with schedules, adapters, Recorder admission, events, and configuration catalogs | no second experiment state |
| `ServiceHost` | startup, bounded Application listeners, deployment lifecycle, reconnect, and finite shutdown | no controller/measurement semantics |
| `Application` | public projections, sessions, deduplication, operations, pages, subscriptions | no experiment authority |
| `SessionStore` | scopes, next sequence, normalized mutation identity, retained outcomes | no transport or domain execution |
| `RecorderWorker` | bounded ingress and exclusive SQLite worker ownership | no Runtime decisions or output authority |
| diagnostic logger | bounded best-effort troubleshooting | no scientific history or control authority |

Queries clone bounded committed projections. Mutations advance state only through the
serialized Runtime owner. A query does not perform hidden device polling.

## One Application semantic model

There is one production `Application`, one `SessionStore`, one operation registry,
and one DTO/Application JSON semantic model. Session, deduplication, event, operation
outcome, and history semantics are shared. TCP and WebSocket are bounded adapters
around that transport-neutral Application JSON:

```text
TCP bytes -> NDJSON framing -----\
                                  -> Application -> Runtime owner
WebSocket text -> JSON body -----/
```

Once a valid transport message reaches the common Application JSON codec, transport
selection does not change DTOs, operation names, scope identity, request sequencing,
deduplication, event/history semantics, operation outcomes, or semantic public-error
mappings. Transport mechanics remain distinct: TCP owns NDJSON framing, while HTTP
Upgrade, WebSocket Origin/subprotocol checks, binary/control messages, WebSocket UTF-8
validation, and protocol-close behavior occur before Application dispatch and may
fail without an Application error envelope.

The canonical API reference starts at [Application API](api/README.md).

## Transports

### TCP/NDJSON

TCP defaults to IPv4 loopback. Both startup modes allow an explicit
`--bind IPv4 --allow-remote-tcp` on a selected trusted LAN interface.
Remote clients must deliberately select the trusted-LAN endpoint. TCP provides
no authentication or TLS and must not be exposed on the public Internet. Listener
selection is startup-only and deployment reload cannot widen it. Each complete
frame is one UTF-8 JSON object followed by LF; CRLF input is accepted. Framing is
bounded before semantic processing.

### WebSocket/JSON

The optional WebSocket listener is also loopback-only and shares the Runtime's global
client capacity. Its public endpoint contract is:

- path: `/application/v1`;
- required subprotocol: `lab-runtime.application.v1`;
- exact configured Origin allowlist;
- text JSON messages only.

The JSON body is the same Application message used inside an NDJSON frame. See
[Errors and limits](api/errors-and-limits.md) for canonical bounds.

For public access, Tuna terminates WSS with `X-Token` authentication and forwards
Upgrade to Runtime's loopback WS listener with the required Host rewrite.
WSS clients must validate the certificate and hostname and must never downgrade.
Tuna is inside the trust boundary; tunnel authentication does not add Runtime
per-operation roles. The proxy does not change Application semantics.

## Measurements and scheduling

A resource is a bounded transport endpoint. An instrument is a semantic device or
model. A signal is identified by instrument and parameter IDs. Current observations
distinguish value, unit, quality, Runtime observation time, source time, and
generation.

Before the first attempt, state is `not_observed`. An unavailable attempt does not
carry an old successful value. A good cached value may still be too old for control:
quality and freshness are separate facts. Generation changes fence late work from an
old resource/model instance.

Scheduling uses monotonic deadlines. Delayed work receives a current opportunity; it
does not replay missed historical intervals. This is bounded scheduling, not a
hard-real-time guarantee.

## Control and output safety

```text
fresh measurement + Reference
  -> native controller
  -> OutputProposal
  -> Runtime OutputAuthority
  -> bounded resource reservation
  -> final lease/epoch/generation check
  -> typed transport write
  -> ACK
  -> separate readback
```

Intent, authorization, send start, ACK, readback, and physical effect are distinct.
Reconnect and fresh input do not automatically rearm control. See
[Safety and failure behavior](safety-and-failures.md).

## Recorder and time

Runtime emits semantic facts and owns Recorder lifecycle. Recorder admission is not
durable commit; the authoritative committed prefix advances only after the storage
receipt returns to the owner. See [Recorder and SQLite](recorder-sqlite.md).

Reference mutations reserve accepted/completion audit capacity before admission.
The bounded budget is 13 groups / 1,545 records / 4 MiB; only Reference credit is
protected, while reconnect shares ordinary credit with acquisition. Reservation
does not guarantee SQL success or unlimited concurrent admission. Submitted credits
remain charged until a valid cumulative SQL receipt. Required recording failure
rejects side effects through the accepted fail-closed fences; BestEffort exposes
missing recording. Thread completion alone never proves durability or a flush.

Monotonic time governs scheduling, freshness, controller `dt`, leases, and
deadlines. Wall-clock time is human/archive context and does not drive control.

## Trusted extension and scheduling boundaries

Managed components use the bounded language-neutral invocation/result contract;
the shipped implementations are trusted compile-time Rust. They do not own
Runtime, transport, SQLite or output authority. There is no embedded scripting VM
or dynamic plugin loader. Keep `lab-core` independent of these adapters and of
wire/deployment/presentation concerns; see [extension boundaries](extending-runtime.md).

Required acquisition, control, safety and Recorder progress have architectural
priority over clients and managed work. This does not claim OS thread priority,
preemption of an in-flight serial transaction or hard-real-time guarantees.

## Boundedness and shutdown

Every long-lived queue, session, projection, cursor, history, and worker has a fixed
capacity and lifecycle. Exhaustion rejects, evicts by documented policy, reports a
gap, disconnects a faulty client, or fails closed; it does not switch to an unbounded
fallback.

Runtime shutdown is a bounded progression through authority revocation, safety,
transport retirement, Recorder sealing/flushing, and worker cleanup. A shutdown
request is not proof that every resource is already closed or physically safe.

External clients and automation use the same language-neutral Application semantics.
Their lifetime remains independent of experiment lifetime.
