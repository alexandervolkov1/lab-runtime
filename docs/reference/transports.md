# Transport reference

For integrators and administrators. Ordinary users should follow the
[local-network guide](../distributed-workbench.md). This is a capability reference,
not a tunnel deployment recipe.

## Supported connections

| Transport | Runtime endpoint | Workbench support | Security boundary |
|---|---|---|---|
| TCP/NDJSON | Loopback by default; one explicit unicast IPv4 LAN bind with opt-in | Numeric socket address; remote TCP needs opt-in | No TLS or authentication |
| WS/JSON | IPv4 loopback HTTP Upgrade listener | `ws://127.0.0.1:PORT/application/v1` locally | Plaintext; exact Host/Origin checks are not user authentication |
| WSS/JSON | A separately administered TLS/authentication endpoint forwards to Runtime's loopback WS listener | `wss://HOST/application/v1` with certificate and hostname verification | Endpoint must enforce authorization; Runtime does not directly terminate TLS |

All use the same Application operations, session rules and recovery contract.
Workbench uses one Application worker across transports; changing transport does
not introduce another experiment or presentation owner.

## Workbench WS/WSS options

Use the [complete CLI grammar](configuration.md#workbench-startup-options) for:

- `--ws-origin`: an exact origin admitted by the Runtime listener;
- `--ws-token-env` or `--ws-token-file`: alternative sources of an access key
  sent as the `X-Token` handshake header, never in a URL;
- `--ws-ca-file`: additional PEM trust anchors for WSS, without disabling
  certificate or hostname checks;
- `--allow-insecure-ws`: explicit plaintext opt-in for non-numeric-loopback hosts.

WS/WSS URLs require exactly `/application/v1`; credentials, queries and fragments
are rejected. The required subprotocol is `lab-runtime.application.v1`. TLS failure
does not trigger an insecure downgrade. An Origin allowlist is not an access-key
check; the administered endpoint must actually reject unauthorized connections.
Do not expose Runtime's raw WS listener directly on the Internet.

## Recovery and local presentation access

Observation mode is a Workbench-side restriction, not server-side authorization.
Connecting, reconnecting or changing transport does not automatically replay
mutations, status requests or Exact Retry. See the
[recovery reference](../recovery-and-faults.md).

`--workbench-listen` is a separate, opt-in IPv4-loopback endpoint. It has no remote
binding or authentication option. Runtime LAN/WSS access does not make this
[presentation API](../workbench-api.md) remotely accessible; do not forward it.
