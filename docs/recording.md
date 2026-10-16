# Recording and backups

Runtime owns the SQLite archive and its worker. Enable recording with an absolute
local `--record-db PATH --record-policy required`, or the deployment `[recording]`
section. The database directory must exist and be writable.

After `hello`, query `recording_status`; submit `recording_start` with a label and
the next exact scope/sequence identity. Wait for the terminal operation result.
Use `recording_annotate` for notes and `recording_stop` with the returned run ID.
Stop completion must confirm `accepted_facts_drained`, `interval_sealed` and
`transaction_committed`. Acknowledged admission alone does not prove durability.
Follow the [Recorder tutorial and complete schema](recorder-sqlite.md) and
[Application operations](api/operations.md).

Use the accepted safe/controller lifecycle before stopping Required recording.
Client disconnection does not stop recording. A lost response is uncertain: retain
the exact request and use [manual recovery](recovery-and-faults.md).

For a consistent backup, finish the run, request Runtime shutdown (Ctrl+C or the
API), and verify the reported Recorder drain/flush and normal exit. Copy the
database together with any matching `-wal` and `-shm` files to a new destination;
retain originals until verified. After a crash preserve all files before recovery.
Never modify live SQLite with another program or discard a WAL because the main
file is readable. Inspect a closed copy with read-only SQLite. Integrity success
does not by itself establish sealed runs, complete coverage or physical safety.
