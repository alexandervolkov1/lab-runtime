# lab-runtime

Headless laboratory automation Runtime. It owns instruments, measurements,
controllers, finite output authority and durable SQLite recording. External
clients use one language-neutral [Application API](docs/api/README.md) over
TCP/NDJSON or loopback WebSocket. Client lifetime does not own an experiment.

This repository contains only `apps/lab-runtime` and `crates/lab-core`.
The application and Cargo package are both named **lab-runtime**.

## Build and test

Use Rust 1.95.0 and a native C toolchain (MSVC on Windows; GCC/Clang on Linux).
SQLite is bundled. The release license gate is pinned to this exact Rust toolchain.

```sh
cargo build --workspace --locked
cargo test --workspace --locked
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
```

No client language, scripting runtime or GUI is required for these commands or
for running Runtime. Python 3.11+ is used only by development/packaging checks.

## Run

From a source checkout, start the built-in virtual thermal plant:

```sh
cargo run -p lab-runtime --locked -- --serve --profile virtual-demo --port 7420
```

Windows binaries require the Microsoft Visual C++ 2015-2022 x64 runtime
(`VCRUNTIME140.dll`), as did the source project.

From an extracted package use `./lab-runtime` (Linux) or `.\lab-runtime.exe`
(PowerShell) with the same arguments. Wait for the JSON `"state":"ready"` line.
The virtual profile opens no serial hardware. Ctrl+C requests bounded safe
shutdown. For recording, add `--record-db` with an absolute path to a writable
local SQLite file and `--record-policy required`; start/stop runs through the API.

For persistent deployment settings use `--serve --config PATH` with a copied
[minimal](examples/runtime.minimal.toml) or [virtual controller](examples/runtime.virtual.toml)
example. Follow the [configuration guide](docs/configuration.md).

## Use and development

- [API and manual output](docs/api/README.md), [configuration reference](docs/reference/configuration.md).
- [Architecture](docs/architecture.md), [safety](docs/safety-and-failures.md),
  [Recorder/SQLite](docs/recorder-sqlite.md), [recovery](docs/recovery-and-faults.md).
- [Driver integration](docs/extending-runtime.md) and [documentation index](docs/README.md).
- Optional [external manual-output example](https://github.com/alexandervolkov1/lab-runtime-next/blob/main/clients/babashka-smoke/README.md)
  exercises discovery, measurements, 10 → 20 → 15% power, explicit lease renewal,
  safe return and recording. Babashka is required only to explicitly run this
  example and is absent from application packages.
- [Origin and extraction inventory](https://github.com/alexandervolkov1/lab-runtime-next/blob/main/ORIGIN.md),
  [packaging](https://github.com/alexandervolkov1/lab-runtime-next/blob/main/docs/packaging.md)
  and [review results](https://github.com/alexandervolkov1/lab-runtime-next/blob/main/docs/extraction-review.md).

Developer preview; virtual and byte-transport tests do not establish physical
Arduino or other hardware safety qualification. Repository extraction does not
publish a release or rename the original repository.
