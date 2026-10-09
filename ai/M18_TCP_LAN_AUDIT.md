# M18 TCP LAN audit

**Historical review record.** The Recorder remediation was subsequently approved
and committed; current M18 scope, verification and remaining review limits are in
[M18_CONSOLIDATED_REVIEW.md](M18_CONSOLIDATED_REVIEW.md). Earlier restrictions,
HEAD/status claims and failed gates below describe their dated diagnostic stage.

Read-only network audit, 2026-10-08. No network production code, firewall, Windows
port forwarding, Tuna configuration or proxy installation changes.

## Direct answers

| Question | Actual implementation |
|---|---|
| TCP bind address | Defaults to numeric `127.0.0.1`; virtual-demo CLI accepts explicit IPv4 through `--bind`. The listener binds that selected address. |
| WebSocket bind address | Always `127.0.0.1`, independently of TCP `--bind`. |
| CLI TCP bind supported? | Yes: `--serve --profile virtual-demo --bind <IPv4> --port <port>`, in that order. `0.0.0.0` selects all IPv4 interfaces. |
| Configuration-file bind supported? | No current configurable address: `--serve --config PATH` has fixed argument shape and startup selects loopback with the configured TCP port. Additional `--bind` is not accepted in this mode. |
| Raw TCP loopback admission/framing filter? | None. The reactor accepts `(stream, _)`, applies common capacity/framing/deadline bounds and does not filter the peer IP. |
| Raw TCP Host validation? | None: TCP is NDJSON Application JSON, not HTTP. Host validation is only part of WebSocket Upgrade. |
| Clojure on another physical PC? | A TCP client with a configurable host can directly use `192.168.x.x:port` if Runtime binds a reachable Linux interface and routing/firewall allows it. No protocol proxy is needed. Physical reachability was not tested. |
| Current bundled TCP Babashka example? | `runtime.clj` accepts only PORT and hardcodes `127.0.0.1`; it does not currently expose a remote-host option. A host-capable diagnostic client proves the underlying path. `distributed.clj` accepts WS/WSS, not raw TCP. |
| Current Workbench same remote TCP endpoint? | No. `RuntimeEndpoint::parse` rejects non-loopback numeric TCP addresses before spawning GUI/worker. The current CLI has no opt-in to override this. |

## Source anchors

- `ServiceOptions::parse`, `apps/lab-runtime/src/service.rs:78`: fixed config mode,
  numeric IPv4 `--bind` parsing at 101, default loopback at 110; getter at 188.
- `ServiceHost` trusted startup binds options at `service.rs:827`; normal startup
  chooses config loopback or profile options at 913-920, then binds that address.
- `bind_websocket`, `service.rs:1208-1214`: explicit IPv4 loopback only.
- Reactor accept, `apps/lab-runtime/src/server.rs:284-312`: ignores peer address,
  admits through `ConnectionCoordinator`, constructs a bounded `Peer`.
- `apps/lab-runtime/src/wire.rs`: bounded transport-neutral JSON/NDJSON decoding,
  duplicate-field/schema checks; no socket address or HTTP Host policy.
- `HandshakePolicy`, `apps/lab-runtime/src/server/websocket_peer.rs:258`: exact
  configured loopback Host, followed by Origin and subprotocol checks.
- `RuntimeEndpoint::parse`, `apps/lab-workbench/src/client/endpoint.rs:35-43`:
  numeric TCP parsing and mandatory client loopback check.
- `clients/babashka-smoke/runtime.clj:9-12`: PORT-only CLI and loopback socket;
  `distributed.clj:39-44`: WS/WSS endpoint and explicit remote plaintext WS opt-in.

The server module's historical "loopback" comments and log label are descriptive
defaults, not TCP peer admission checks. Workbench's loopback restriction is real.

## Actual listener and client evidence

The user's existing Arch WSL2 Runtime PID 1087 listens at `127.0.0.1:8765`; it was
not changed or stopped. Arch's current virtual Ethernet IPv4 is `172.31.158.229`.
An isolated new Runtime launched with `--bind 172.31.158.229 --port 0 --ws-port 0`:

```text
ss -ltnp
TCP  172.31.158.229:43809  lab-runtime PID 5410
WS   127.0.0.1:42227      same isolated Runtime
```

Port/PID exact observations are in `tcp-lan-listeners.log`. Windows Babashka connected directly to the WSL IP
and completed hello/reference using raw TCP, without HTTP Host or proxy. Windows
Workbench rejected the same endpoint with its loopback policy error. One explicit
Runtime shutdown completed with safe/flush/cleanup evidence and exit zero.
This proves Windows-to-WSL-IP transport/admission, not two physical LAN machines.

## What is actually needed for physical LAN

For this virtual profile on a Linux PC with that assigned LAN address:

```bash
./lab-runtime --serve --profile virtual-demo --bind 192.168.0.50 --port 8765
```

Use the Linux PC's actual assigned address. A host-configurable Clojure client
connects its socket to `192.168.0.50:8765` and uses unchanged Application NDJSON.
Restrict OS firewall/routing to the explicitly trusted clients; no Runtime server
code or proxy is necessary for that TCP case. Raw TCP is plaintext and has no
tunnel X-Token/HTTP authentication. It must not be described as protected WSS or
server-authorized read-only access; scope/identity are not login credentials.

Workbench direct LAN TCP requires a separately authorized client endpoint-policy
change and validation; config-file deployment bind needs a separately reviewed
configuration/startup change. Neither is made here. Direct LAN WS needs its own
M12 bind/Host/security review. A proxy remains one deployment option for WS/WSS,
not a mandatory bridge for Runtime's already host-bindable raw TCP.

Windows 10 WSL2 currently uses a NAT virtual subnet. A physical peer reaching the
Windows Wi-Fi IP does not automatically reach the WSL listener: Windows inbound
firewall/port forwarding would be a separate system action. No such rule is added,
and a second physical PC/LAN reachability is not claimed.
