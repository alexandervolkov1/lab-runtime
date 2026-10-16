# Transport reference

Runtime exposes one [Application contract](../api/README.md) through bounded
transport adapters. Client languages and presentation are independent of it.

| Transport | Endpoint | Security boundary |
|---|---|---|
| TCP/NDJSON | Numeric IPv4, loopback by default | Plaintext and unauthenticated; remote trusted-LAN bind requires explicit opt-in |
| WS/JSON | Loopback `/application/v1` | Requires `lab-runtime.application.v1` subprotocol and exact configured Origin |
| WSS tunnel | External authenticated TLS tunnel to loopback WS | Tunnel owns access control/TLS; clients must validate key, certificate and hostname |

See [listener options](configuration.md#tcp-and-websocket-listeners). Runtime
does not itself provide a public TLS listener. Do not expose its plaintext WS
endpoint as a substitute for authenticated WSS.

The same scopes, sequencing, deduplication, event gaps and manual mutation recovery
apply to both adapters. Frame/upgrade failures are transport-specific. A socket
or hello cannot establish mutation success or a current complete snapshot.
Loss of a client does not cancel Runtime work or grant new output authority.
