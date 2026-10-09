# Configuration examples, not experiment archives

The TOML/JSON files here are source examples and test inputs. Review addresses,
ports, device definitions and safe profiles before using a physical instrument;
the virtual examples do not require hardware.

| Files | Purpose |
|---|---|
| [runtime.minimal.toml](runtime.minimal.toml), [runtime.virtual.toml](runtime.virtual.toml) | Safe virtual startup and full virtual deployment |
| [runtime.metakon-read-only.toml](runtime.metakon-read-only.toml), [runtime.metakon-513-com5.toml](runtime.metakon-513-com5.toml) | Generic and bench-specific Metakon read-only templates |
| [runtime.metakon-513-m9d-read-preflight.toml](runtime.metakon-513-m9d-read-preflight.toml) | Physical read preflight template |
| [runtime.metakon-513-com5-output.toml](runtime.metakon-513-com5-output.toml) | Explicit physical-output bench template; not safe to launch on guessed hardware |
| [definitions/](definitions/) | Metakon input/output definitions and a decoder fixture |
| [simple-device/](simple-device/) | Declarative read-only/writable definitions and deployments |

The `recording.path` entries are output destinations. A previous experimental
database is not required to parse these deployments or run their deterministic
tests. Choose a fresh absolute path outside the source tree for real experiments;
do not overwrite a prior run. Recorder can create a new database or reopen a
compatible archive, without restoring controller authority.

The repository deployment-loader and configured-physical integration tests use
these configurations and fake transports, not historical bench measurements.
The separate `test-data/m16_unknown_reconnect_application.json` is an actual
source-consumed Runtime/Workbench regression fixture and remains tracked.

## Historical hardware data

Twenty original SQLite/WAL/SHM files (ten database families, 6,237,088 bytes) were
historical Metakon acquisition/reconnect/output smoke evidence, including failed
runs. They were not required regression fixtures. The exact bytes and their
contemporary reports remain in
[published preview.4 source](https://github.com/alexandervolkov1/lab-runtime/tree/cc32f827708ee9fefe63394245dcce1aa437e520/examples).
Restore complete database/WAL/SHM families from that commit into a new external
directory; do not restore only a main file with an outstanding WAL.

Before removing them from the current tree, an independent local preservation
archive was made: `metakon-hardware-originals-cc32f827.zip` (616,756 bytes), SHA-256:

```text
58a8614be4d361439435f80f30ca608caffd28bfbb83000dec80401c3ab05ec5
```

Its `MANIFEST.json` records every original path, byte count, SHA-256, Git blob and
source commit `cc32f827708ee9fefe63394245dcce1aa437e520`. All twenty files passed
archive round-trip comparison. All ten extracted database copies passed read-only
integrity and foreign-key checks with WAL included; the two nonempty WALs passed
frame/salt/checksum checks (97 and 195 frames). The original files were never opened
through SQLite and were hash-checked unchanged after the audit.

Perform analysis on extracted copies using SQLite `mode=ro` and `query_only`;
SHM may be recreated there. Do not use immutable mode to skip necessary WAL
recovery. Structural integrity does not mean a failed/unsealed experiment completed
or that hardware became safe. See [Recorder semantics](../docs/recorder-sqlite.md)
and [current qualification limits](../docs/developer/project-status.md).
