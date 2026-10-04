# Application API reference

This compatibility page preserves existing links. The canonical reference for
Application protocol v1 and the current repository/v0.1 product is now:

- [Application API overview](api/README.md)
- [Protocol, envelopes, hello, and sessions](api/protocol-and-sessions.md)
- [Complete operation and capability registries](api/operations.md)
- [Events, mutations, deduplication, and recovery semantics](api/events-mutations-and-recovery.md)
- [Public errors and limits](api/errors-and-limits.md)

The Application API is a supported language-neutral product boundary, not an
implementation detail of Workbench. Runtime currently exposes it over local
TCP/NDJSON and optional loopback WebSocket/JSON; native Workbench uses TCP.

The [Workbench external API](workbench-api.md) is a distinct presentation/client
surface, not another Runtime Application implementation. See the optional
[Babashka wire smoke](../clients/babashka-smoke/README.md) for direct Runtime access
and a separate minimal Workbench example.

```text
direct client -> Runtime API -> authoritative Runtime

external client -> Workbench API -> Workbench dispatcher
                -> existing Runtime client -> Runtime
```

A client that does not need Workbench presentation or client-recovery semantics
should normally use the Runtime API directly. Workbench is neither a transparent
Runtime proxy nor a Runtime event relay.

The authoritative registry contains 43 operations (22 queries and 21 mutations) and
26 structured capabilities. Clients must begin with `hello` and use the operations,
capabilities, limits, scope, next sequence, and event cursors returned by that Runtime
instead of assuming a static composition.
