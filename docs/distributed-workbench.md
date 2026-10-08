# Distributed Workbench preview (M18)

Runtime owns experiments, acquisition, controller/output authority and Recorder.
Workbench owns presentation and one bounded Application client. A script connects
directly to Runtime with its own scope and separately to the local Workbench API
when it needs to change plots. It never relays measurements into Workbench.

```text
Script -------- WS/WSS Application API --------> Linux Runtime
Windows GUI --- WS/WSS Application API --------> Linux Runtime
Script -------- loopback Workbench API --------> Windows GUI presentation
```

TCP compatibility is unchanged: Workbench accepts numeric loopback TCP addresses.
WS/WSS uses the same worker, scope, sequencer, admission, recovery journal and
rebuild barrier. This guide does not extend Runtime's existing loopback-only WS
listener. Direct LAN WS needs separate transport-policy review; it is not yet an
accepted two-computer deployment. A local test proxy is test infrastructure only.

## Linux Runtime

Install the existing Linux x86_64 GNU Runtime package and use its example
unit in the [Linux Runtime guide](linux-runtime.md#optional-systemd-deployment)
with an absolute executable path, dedicated working directory and separate SQLite
path. Do not run against a production DB
for acceptance. The virtual-demo command is:

```bash
./lab-runtime --serve --profile virtual-demo --port 8765 \
  --record-db /var/lib/lab-runtime/preview.sqlite \
  --ws-port 8766 --ws-origin http://127.0.0.1:3000
```

The process must have write access to the DB directory. Recorder is independent of
GUI/script lifetime. Its SQLite connection is exclusive: inspect live recording
through Application history/status queries; stop Runtime before raw SQLite checks.

```bash
sudo systemctl start lab-runtime
sudo systemctl status lab-runtime --no-pager
sudo journalctl -u lab-runtime -f
sudo systemctl restart lab-runtime
sudo systemctl stop lab-runtime
ss -ltn '( sport = :8765 or sport = :8766 )'
sqlite3 /var/lib/lab-runtime/preview.sqlite 'PRAGMA integrity_check;'
```

WS remains bound to `127.0.0.1` and requires the configured exact Origin, Host and
`lab-runtime.application.v1` subprotocol. Origin is a browser policy, not client
authentication. Do not expose a managing endpoint publicly without authentication.
Service supervision is an OS concern; it is not Runtime application logic.

WSL2 verifies Linux binaries/SQLite/systemd plus Windows clients on one laptop. It
does not prove two physical machines, USB/RS-485, bare-metal timing or power loss.
WSL localhost forwarding and systemd lifetime depend on WSL availability; a systemd
unit inside WSL does not make the distribution independent of Windows/WSL shutdown.

## Windows Workbench

```powershell
cargo build --release -p lab-workbench --locked
./target/release/lab-workbench.exe --connect 127.0.0.1:8765 --workspace ./target/tcp-preview
./target/release/lab-workbench.exe --connect ws://127.0.0.1:8766/application/v1 `
  --observe --workspace ./target/ws-preview --workbench-listen 127.0.0.1:8767
```

`--observe` is explicit and defaults off. It disables GUI Runtime controls and
rejects Runtime mutations and Exact Retry before submission to the worker mailbox,
including mediated mutations through the Workbench API. Read queries, subscriptions,
manual Check Status and presentation operations remain available. It is local
client policy, **not server authorization**: another client can still mutate Runtime.
`history_read` is an asynchronous mutation/work job in the frozen API and is also
blocked by this conservative mode. Recent in-memory history and existing history
page queries remain queries; use normal mode for explicit archive selection jobs.

`--ws-origin` defaults to `http://127.0.0.1:3000` and must match Runtime policy.
Non-loopback plaintext WS requires `--allow-insecure-ws` and prints a warning; use
only on an explicitly trusted network. There is no automatic WSS downgrade.
WSS validates certificates and hostname against OS roots. `--ws-ca-file PATH` adds
a bounded PEM trust bundle for a private CA; it does not bypass verification.
TLS certificate/hostname errors fail the connection. The GUI thread owns no socket.

## Tuna access control (configure only after approval)

The existing endpoint is
`wss://fresh-hedgehog-9022.ru.tuna.am/application/v1`, with local upstream
`http://127.0.0.1:8766`. A previous hello/latest is not proof of current key-auth.
At M18 inspection, no Tuna process was found and Upgrade probes returned HTTP 404;
protected internet acceptance remains pending. Existing configuration was not changed.

Official [Tuna HTTP tunnel documentation](https://tuna.am/docs/tunnels/http/) supports
WebSocket Upgrade, `--key-auth`, `X-Token`, and the `TUNA_KEY_AUTH` environment variable.
The account token authenticating the Tuna agent is different from the client tunnel
key. Configure an independent random access key. Never put it in a URL or command
line; use a protected file/environment and disable tunnel inspection.

On Linux, after approval and using the existing assigned subdomain/account:

```bash
# Store only the client access key in this root/user-readable file (mode 0600).
IFS= read -r TUNA_KEY_AUTH < /etc/lab-runtime/tuna-access-key
export TUNA_KEY_AUTH
tuna http 127.0.0.1:8766 --subdomain=fresh-hedgehog-9022 --location=ru \
  --request-header='host:127.0.0.1:8766' --inspect=false --log-level=warn
unset TUNA_KEY_AUTH
```

This command is a proposed protected launch, not evidence that the currently assigned
tunnel is authorized or running. Verify the installed Tuna version's `http --help`
and the assigned subdomain before replacing a working agent. Do not change its
configuration as part of a client-only test. For systemd, put the access key in a
permissions-restricted environment file and reference it with `EnvironmentFile=`;
do not put literal keys in a unit or journal. Keep Runtime upstream on loopback.

On Windows, use a restricted file containing the access key, optionally ending in
one LF or CRLF. Workbench reads at most 514 bytes and accepts a nonempty ASCII header
value no larger than 512 bytes, with no embedded CR/LF:

```powershell
./target/release/lab-workbench.exe `
  --connect wss://fresh-hedgehog-9022.ru.tuna.am/application/v1 `
  --ws-token-file C:/Users/user/.secrets/tuna-runtime-key `
  --observe --workspace ./target/wss-preview --workbench-listen 127.0.0.1:8767
```

Alternatively use `--ws-token-env LAB_RUNTIME_TUNNEL_TOKEN`. The two secret sources
are mutually exclusive. Header values never enter Application JSON, recovery or
presentation documents. HTTP response bodies/headers are not included in client
errors. Raw HTTP Upgrade trace logging in Tungstenite is disabled at compile time.

Before sending any public mutation, establish that missing and deliberately wrong
`X-Token` requests cannot upgrade, while the correct key can complete hello and
read-only queries. Do not print request headers or response bodies during these tests.
Check TLS errors and incorrect hostname with a local TLS fixture, never by turning
verification off. Tunnel authorization is not per-operation Runtime authorization.
Tuna terminates public TLS and can see Application traffic; the Tuna service/operator
and local upstream are inside the experiment's trust boundary.

## Independent Babashka client

Babashka 1.13.220 includes Cheshire and Java's asynchronous WebSocket client. The
following bounded acceptance example requires **virtual-demo/test** instances. It
creates its own Runtime scope, performs exactly one safe Reference retune, waits
for authoritative accepted/completed evidence, verifies the resulting snapshot,
then adds a temperature plot through the local Workbench presentation API:

```powershell
bb clients/babashka-smoke/distributed.clj ws://127.0.0.1:8766/application/v1 8767
# Only after protected Tuna is confirmed:
bb clients/babashka-smoke/distributed.clj `
  wss://fresh-hedgehog-9022.ru.tuna.am/application/v1 8767 `
  --token-env LAB_RUNTIME_TUNNEL_TOKEN
```

The script never forwards observations, calls mediated Runtime mutations, or retries
an ambiguous mutation. Workbench acquires measurements directly from Runtime and
updates its own subscribed/rebuilt observations. The original TCP smoke examples
remain available. The example is not an unattended experiment/recovery SDK.

## Recovery

Unexpected continuity loss uses the existing bounded retained-scope episode; hello
does not prove Fresh. Queries/subscriptions are rebuilt through the authoritative
barrier; ambiguous mutations, status and Exact Retry are never replayed automatically.
Explicit Disconnect cancels reconnect. After `instance_changed`, use **Connect new
scope** in the same workspace. Only this explicit action chooses a new scope.
Unresolved quarantined evidence remains visible and blocks new mutations. Manual
Check Status/Exact Retry follow existing eligibility; observation mode also disables
Exact Retry. Closing Workbench never stops Runtime or Recorder.

If the local Workbench API is unavailable, direct scripts can still use Runtime;
presentation changes require reconnecting to Workbench. Do not turn an unacknowledged
UI call or Runtime timeout into an automatic mutation retry.

```powershell
# The Runtime remains usable when Workbench is closed.
bb clients/babashka-smoke/distributed.clj ws://127.0.0.1:8766/application/v1 --runtime-only
```

## Operational blocker found during M18

The unchanged final Linux Runtime binary was exercised with required Recorder.
It failed with `recorder ingress capacity exhausted` on both WSL-mounted NTFS and
native ext4 while two clients/GUI or a WS subscriber plus mutation were active.
It failed closed, reported `coverage: gap`, confirmed virtual safe output and exited
with an incomplete Recorder shutdown rather than claiming successful flush. The
first failed database passed integrity and foreign-key checks. A separate ext4 run
without Workbench remained recording for 90 seconds and completed an explicit stop
and process shutdown successfully. These results do not establish the exact cause
of the ingress saturation; filesystem choice alone does not explain it.

A narrower headless reproduction uses TCP recording_start, one WS subscriber and
one TCP Reference retune. Recording failed after 68 ms on the unchanged accepted
Linux executable. A coherent SQLite backup including WAL preserves all 4,795 old
measurements, two new measurements and a durable gap; integrity and foreign keys
pass. The terminal retune audit was not admitted, and the failed boot is unsealed.
The four-group ingress credit and writer's 100 ms fact coalescing window are the
leading capacity explanation; the rejection's exact receipt interleaving was not
instrumented. A healthy restart/continued recording result is not claimed.

This is an operational acceptance blocker, not a Workbench transport PASS. Runtime
production and Recorder limits/semantics were not changed. Sustained recording with
concurrent clients requires separate diagnosis/review before preview publication.
