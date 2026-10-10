# Project status and development map

## Historical published baseline

[v0.1.0-preview.4](https://github.com/alexandervolkov1/lab-runtime/releases/tag/v0.1.0-preview.4)
is published and its seven uploaded assets were downloaded and hash-verified.
Its exact source is `cc32f827708ee9fefe63394245dcce1aa437e520`; the final license
packaging commit changes no production Rust/Cargo inputs from the M18 merge.
The Cargo package version remains `0.1.0`. A developer preview is not production
certification or hardware safety qualification.

The release provides Windows Runtime + Workbench and a Linux x86_64 GNU headless
Runtime. The Linux executable must travel with its matching license archive.
This binary requires GLIBC symbols through **2.34** and was built/smoke-tested on
Arch Linux WSL2 with glibc 2.44; this is not universal Linux compatibility.
See [Linux deployment](../linux-runtime.md) and the
[license evidence review](../release-license-evidence.txt). Preserve pinned
`third-party-licenses/` materials, including original serialport source and the
separate Rust standard-library inventory.

## Preview.5 documentation and packaging

The `v0.1.0-preview.5` changes concern user documentation and packaging only,
not product behavior. User packages contain the manual and two safe virtual
configuration examples. Experimental clients, hardware/protocol examples and
engineering references remain in the source repository.

Check [GitHub Releases](https://github.com/alexandervolkov1/lab-runtime/releases)
for the current publication status and downloadable assets. This source document
does not assert whether preview.5 has been published. The preview.4 commit above
identifies the historical published source, not a moving latest-release pointer.

## Implemented boundaries

- Runtime owns experiment state, acquisition, controllers, central output
  authority, configuration and durable Recorder history. Workbench owns
  presentation. Neither Workbench nor script lifetime owns the experiment.
- One Application semantic model serves TCP/WS/WSS clients. Workbench uses one
  bounded worker, with explicit observation-only policy and manual recovery.
- TCP defaults to loopback and requires explicit trusted-LAN opt-in; it has no
  TLS or authentication. Runtime WS remains loopback; authorized WSS/Tuna access
  requires the key and normal TLS/certificate/hostname validation.
- The local Workbench presentation API is opt-in and loopback-only. Remote direct
  Runtime access does not provide remote Workbench presentation control.
- Declarative SimpleDevice and native Rust instruments share the ordinary
  Application, controller, Recorder and Workbench paths. Prepared topology does
  not become public authority before atomic activation. API overlays do not
  survive Runtime restart.
- Required Recorder failure is fail-closed. The finite pre-admission budget,
  causal ordering, committed receipts and manual mutation reconciliation remain
  architectural contracts, not performance promises.

The authoritative current explanations are [architecture](../architecture.md),
[safety](../safety-and-failures.md), [Recorder](../recorder-sqlite.md),
[recovery](../recovery-and-faults.md), [Application API](../api/README.md),
[Workbench API](../workbench-api.md) and [transport reference](../reference/transports.md).
The separate [user manual](../README.md) covers all-local Windows and Linux Runtime
with Windows Workbench. Programmable clients remain in the developer documentation.

## Evidence and open qualification limits

- Windows/Ubuntu workspace Debug/Release gates for the unchanged M18 Rust inputs
  passed historically. Later Arch targeted Recorder and Workbench Debug/Release
  suites, WSS stress (20/20 per profile), formatting and scoped Clippy passed.
  Ubuntu evidence is historical, not a new Arch run.
- The released Windows/Linux extracted packages passed their smoke checks;
  Linux additionally passed measurement, shutdown/seal and SQLite integrity
  checks. The packaging license evidence regressions passed separately.
- Rare WSS test instability remains **unresolved**. Passing repetitions do not
  establish its root cause or justify weakening authentication/TLS assertions.
  No authentication/TLS bypass was established by that review.
- WSL2 is not physical two-computer E2E. There is no exhaustive physical hardware
  safety, power-loss, disk-full, multi-day or USB/driver fault qualification.
  ACK/register readback is not proof of downstream physical effect.
- Recovery does not automatically replay mutations, status or Exact Retry, and
  has no Discard/Forget action. Quarantined uncertainty remains visible.
- No embedded scripting VM or dynamic plugin loader is shipped. Earlier Steel
  evaluation was stopped by dependency-safety findings; any future proposal
  needs a fresh exact-version review, not a standing historical prohibition.

Possible next directions are physical two-host qualification, investigation of
the WSS fixture instability, and explicitly scoped device/automation integration.
These are directions for discussion, not authorization to start another milestone.

## Source layout and verification

| Location | Purpose |
|---|---|
| `apps/lab-runtime/` | Runtime host, storage/transport adapters and integration tests |
| `apps/lab-workbench/` | Native client, presentation, recovery and local API |
| `crates/lab-core/` | Platform-neutral experiment and safety contracts |
| `clients/` | Independent Babashka/ClojureScript Application-boundary demonstrations |
| `examples/` | Safe virtual deployments and developer-oriented SimpleDevice protocol examples |
| `test-data/` | Deterministic regression inputs, including internal Metakon deployments under `fixtures/metakon/` |
| `scripts/` | Windows/Linux packaging, license evidence gates and their tests |
| `third-party-licenses/` | Pinned redistribution evidence; not a disposable cache |
| `docs/` | Current user, API, architecture and contributor documentation |

Choose targeted tests from affected owners; `apps/lab-runtime/tests/README.md`
maps Runtime integration suites. Keep `cargo fmt --all -- --check` and
`git diff --check` clean. Full workspace Debug/Release gates belong to changes that
need them, not every documentation cleanup. Packaging uses an explicit public-file
inventory: new README/document links must also resolve inside the extracted package.
Run packaging, documentation and evidence checks as appropriate; license tests do
not require rebuilding Rust. Never treat successful archive creation alone as a
complete license audit.

## Historical evidence

Historical milestone reports, external reviews, original bench databases and
temporary execution permissions are not current development instructions. They
remain byte-recoverable from the immutable published source:

- [Engineering reports at preview.4](https://github.com/alexandervolkov1/lab-runtime/tree/cc32f827708ee9fefe63394245dcce1aa437e520/ai)
- [Hardware examples/data at preview.4](https://github.com/alexandervolkov1/lab-runtime/tree/cc32f827708ee9fefe63394245dcce1aa437e520/examples)

Use `git show v0.1.0-preview.4:<path>` for a historical text, or `git archive` into
a new external destination for binary files. Recover each database together with
its matching WAL/SHM; never overwrite an active database or substitute a failed
run for a successful one. Old machine-local ignored logs may not exist on another
computer; a historical report is not a claim that those raw logs were revalidated.
The published tag, assets, checksums and original six packaging scripts remain
available at that exact commit; current-tree documentation hygiene does not alter
the released binaries or imply byte-identical rebuilt archives.
