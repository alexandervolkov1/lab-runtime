# Optional external manual-output example

`manual-output.clj` is an independent TCP/NDJSON Application client. Babashka is
required only when explicitly running this example. Runtime, Cargo dependencies,
protocol, configuration, ordinary Rust tests and startup have no Babashka/Clojure
dependency. The client is not included in binary application packages.

Install Babashka separately if you choose to run the check (`bb --version`). Build
Runtime with `cargo build -p lab-runtime --locked`, then from the repository root:

```sh
python scripts/run_manual_output.py debug
```

On Linux use `python3`. To select a local evidence/database directory explicitly,
add `--output-root PATH`. For the reviewed Windows C: storage run:

```powershell
python scripts/run_manual_output.py debug --output-root "$env:TEMP/lab-runtime-next-review"
```

The default D: checkout path failed Required Recorder progress on the extraction
host; see the [qualification and retained failures](../../docs/extraction-review.md).
Do not weaken Recorder policy or automatically retry uncertain operations.
The bounded runner starts the built-in virtual plant with
Required Recorder on an ephemeral loopback port and starts this external client.
It discovers/describes the instrument, obtains temperature, explicitly acquires
authority, writes 10%, 20%, 15%, renews the lease, requests safe, verifies evidence,
seals the run and requests fixture shutdown. It checks the closed SQLite archive
and retains transcripts, process exits, identities and measurements in a new
directory under `target/runtime-manual-api`.

For an already-running virtual Runtime with Recorder enabled:

```sh
bb clients/babashka-smoke/manual-output.clj PORT
```

Replace PORT with the Runtime ready port. This form leaves Runtime running.
`--shutdown-fixture` requests Runtime shutdown and is for an owned fixture only.
To test an extracted binary use
`python scripts/run_manual_output.py release --binary ABSOLUTE_EXECUTABLE_PATH`.

The optional `--identified-emulator INSTRUMENT TEMPERATURE_PARAMETER OUTPUT_PARAMETER`
mode requires independently verified hardware identity/definition and operator
authorization. Never guess a COM port or use the `COM256` placeholder. No physical
Arduino acceptance is claimed. No automatic reconnect, mutation replay or rearm
is implemented. See the [API contract](../../docs/api/manual-output.md).
