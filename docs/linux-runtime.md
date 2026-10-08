# Headless Runtime on Linux x86_64

The Linux package targets `x86_64-unknown-linux-gnu` and contains only the headless
`lab-runtime` executable. Windows and Linux share the same Rust source, Application
API, controller/output authority and Recorder implementation. Workbench is a
separate Windows application and is not built or included here. ARM is not supported
by this package.

## Extract and run

Keep the accompanying `.licenses.tar.gz` and `.build.json` with the package when
redistributing it. The license archive contains the project license, third-party
texts and a target-specific dependency inventory; it is not needed at runtime.
Package creation does not publish a release or grant formal legal sign-off.

```sh
sha256sum -c lab-runtime-0.1.0-linux-x86_64.tar.gz.sha256
sha256sum -c lab-runtime-0.1.0-linux-x86_64.licenses.tar.gz.sha256
tar -xzf lab-runtime-0.1.0-linux-x86_64.tar.gz
cd lab-runtime-0.1.0-linux-x86_64
mkdir -p "$PWD/state/logs"
export LAB_RUNTIME_LOG_DIRECTORY="$PWD/state/logs"
./lab-runtime --serve --profile virtual-demo --port 7420 \
  --record-db "$PWD/state/history.sqlite"
```

This virtual profile opens no serial hardware. Wait for the JSON `state: ready`
line before connecting. TCP/NDJSON defaults to `127.0.0.1`; the optional WebSocket
listener remains loopback-only and requires an explicit allowed origin:

```sh
./lab-runtime --serve --profile virtual-demo --port 7420 \
  --ws-port 7421 --ws-origin http://127.0.0.1:3000
```

Clients use the unchanged [Application API](api/README.md): complete `hello` before
queries or commands. Ctrl+C, SIGTERM and SIGHUP request the existing Runtime-owned
safe shutdown and Recorder flush. SIGHUP shuts down; it does not reload configuration.
Client disconnect does not stop the experiment. Forced termination cannot perform
safe shutdown or guarantee a sealed Recorder tail.

The executable dynamically links GNU/Linux system libraries. Consult `ldd` and
`required_glibc_symbol_version` in its `.build.json`; builds on a newer distribution
may require a newer glibc than older hosts provide. SQLite is compiled into Runtime
through the existing bundled rusqlite feature; no SQLite executable, database template,
GUI library or libudev package is required. A writable local directory is needed for
Recorder databases and their WAL/SHM sidecars. Use an absolute `--record-db` path.

Set `LAB_RUNTIME_LOG_DIRECTORY` explicitly for unattended deployment. Existing
diagnostic fallback is `$XDG_STATE_HOME/lab-runtime/logs` when that variable is set
(after `LOCALAPPDATA`, if supplied), otherwise a temporary `lab-runtime/logs` directory.
Logging remains bounded and best effort; it is separate from scientific recording.

## Serial deployment

Follow the existing [Configuration Guide](configuration.md). Schema-v1 resource
kind strings remain `windows_com_read_only` and `windows_com` on both platforms;
Linux does not introduce a new DTO or resource kind. On Linux set `port` to an absolute
device path, for example `/dev/ttyUSB0`, `/dev/ttyACM0`, or a stable
`/dev/serial/by-id/...` symlink. Paths preserve case, are limited to 4096 bytes, and
reject control characters, empty segments and `.`/`..` segments. Validation does not
open a device or resolve symlinks. Legacy COM names still validate for compatibility,
but a Windows COM name does not identify a Linux device automatically.

The service account must have access to the device node (often through a distribution's
`dialout` group or a local udev rule). Select that policy on the deployment host.
Do not substitute virtual evidence for physical ACK, readback or safe confirmation.
Linux compilation and virtual tests do not establish USB/RS-485 hardware acceptance;
device permissions, baud/parity/flow control, disconnect/reconnect, adapter latency,
output safety and finite shutdown still need qualification with the actual equipment.

## Optional systemd deployment

Create a dedicated `lab-runtime` account, install the executable as
`/opt/lab-runtime/lab-runtime`, and create `/var/lib/lab-runtime` owned by that account.
An example unit at `/etc/systemd/system/lab-runtime.service` is:

```ini
[Unit]
Description=Headless laboratory Runtime (virtual demo)
After=network.target

[Service]
Type=simple
User=lab-runtime
Group=lab-runtime
WorkingDirectory=/var/lib/lab-runtime
Environment=LAB_RUNTIME_LOG_DIRECTORY=/var/lib/lab-runtime/logs
ExecStart=/opt/lab-runtime/lab-runtime --serve --profile virtual-demo --port 7420 --record-db /var/lib/lab-runtime/history.sqlite
KillSignal=SIGTERM
TimeoutStopSec=15s
Restart=no
UMask=0077

[Install]
WantedBy=multi-user.target
```

Validate with `systemd-analyze verify /etc/systemd/system/lab-runtime.service`, then
use the host's normal `systemctl daemon-reload` and `systemctl start lab-runtime`
commands. `systemctl stop lab-runtime` requests safe shutdown. The example makes no
automatic restart/rearm decision. For physical deployments replace the demo CLI with
`--serve --config /etc/lab-runtime/runtime.toml` and provide validated definitions,
device permissions and a writable local recording path. See
[configuration](configuration.md) and [Recorder](recorder-sqlite.md).
Service management remains outside Runtime application logic.

## Build and package

Use Linux x86_64, Rust 1.95 with Cargo, Python 3.11+, Git, a C compiler/linker, and
binutils. For Ubuntu these host tools are available from `build-essential`,
`binutils`, `git`, `python3` and `curl` (curl is useful for installing Rust).

```sh
cargo build -p lab-runtime --locked --target x86_64-unknown-linux-gnu
cargo build -p lab-runtime --release --locked --target x86_64-unknown-linux-gnu
cargo test -p lab-core -p lab-runtime --all-features --locked --target x86_64-unknown-linux-gnu
cargo test -p lab-core -p lab-runtime --release --all-features --locked --target x86_64-unknown-linux-gnu
cargo fmt --all --check
cargo clippy -p lab-core -p lab-runtime --all-targets --all-features --locked \
  --target x86_64-unknown-linux-gnu -- -D warnings
python3 scripts/package-linux-runtime.py
```

The packager derives the version from workspace Cargo metadata. Outputs in `dist/`:

- `lab-runtime-<version>-linux-x86_64.tar.gz`, containing one executable with mode 0755;
- `.licenses.tar.gz`, containing Runtime-only dependency license evidence;
- `.build.json`, recording target, source commit/dirty state, toolchain, lockfile hash,
  binary/archive SHA-256, shared libraries and required glibc symbol version;
- `.sha256` companions for both archives.

Existing outputs are protected against accidental overwrite. `--allow-dirty` explicitly
records an uncommitted review build. Build products can be kept on a Linux filesystem
with `CARGO_TARGET_DIR` when the source checkout is on a WSL-mounted Windows drive.
The packager uses Cargo's reported executable path rather than assuming `target/`.

The Linux process test completes hello and `latest` through real TCP and WebSocket
connections, checks Application shutdown, and checks SIGINT/SIGTERM/SIGHUP with active
recording plus sealed SQLite rows and integrity. To run that same acceptance against
an extracted package outside the checkout:

```sh
LAB_RUNTIME_SMOKE_BINARY=/absolute/extracted/path/lab-runtime \
  cargo test -p lab-runtime --release --locked --target x86_64-unknown-linux-gnu \
  --test linux_process
```

WSL2 provides Linux build/process evidence, not bare-metal kernel, USB, serial adapter
or physical safety qualification. Review the verification report for the exact build
host, glibc requirement, commands and remaining hardware risks.
