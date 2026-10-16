# Recording and keeping your data

[User manual](README.md) · [First recording walkthrough](getting-started.md#6-record-and-finish-a-run)

Runtime writes recordings to a SQLite database on the Runtime computer. Workbench
starts and stops runs, but does not own the database. A live graph is not a saved run.

## Make recording available

Start Runtime with an absolute local `--record-db` path, as in the first experiment,
or set `[recording]` in a [configuration file](configuration.md#choose-the-recording-path).
Use a writable local disk with free space, not a network share.

`required` is recommended when data loss must stop experiment-critical control.
If storage fails, Runtime blocks that control and follows its safety handling.
Changing the policy to hide a failure does not recover lost data.

## Start and stop

1. Wait for Workbench to be **Fresh** and Recorder to show `idle`.
2. Enter a nonempty label in the Recorder text box, click **Start recording**,
   review the confirmation and click **Confirm**.
3. Wait for **Operation completed**, click **Acknowledge**, and check for
   `authoritative state: recording`.
4. If a controller uses required recording, pause it through its reviewed safe
   procedure first. Then click **Stop recording**, review and **Confirm**.
5. Wait for **Operation completed**. In its result, check `run_sealed: true` and
   `transaction_committed: true`. Then **Acknowledge** and check for `idle`.

The result is shown as structured text, not a separate success dialog. After an
action, acknowledge its result to restore the controls. If stop fails or the answer
is lost, do not assume the run was saved: keep the files and follow
[recovery guidance](workbench.md#loss-of-connection-and-recovery).

You can store several runs in one database. A label helps identify a run; using the
same label does not overwrite an earlier run. Data outside a started run is not a
substitute for deliberate recording.

## Keep and copy a recording

For a simple, consistent backup:

1. Finish the run and check its stop result.
2. Stop Runtime with Ctrl+C and wait for it to exit normally.
3. Copy the database and any same-name `-wal` and `-shm` files together to a new
   backup folder. Keep the originals until the backup has been checked.

For the first-experiment paths, after Runtime has stopped:

```powershell
New-Item -ItemType Directory -Path .\first-run-backup -ErrorAction Stop | Out-Null
Copy-Item -Path .\first-run\history.sqlite* -Destination .\first-run-backup\
```

If Runtime crashed or storage failed, preserve all these files before attempting
recovery. Never delete a WAL file just because the main database exists. Do not
copy only the main file while Runtime is writing, and do not modify a live database
with another program.

## View a saved run

This Workbench version has no saved-run browser or export command. Completing the
walkthrough needs no extra software: check the stop result and saved file location.
For deeper inspection, give a copy to someone familiar with SQLite, or open a copy
with a SQLite viewer in read-only mode. Keep your original archive unchanged.

A database file's size alone does not prove complete data, and a successful
recording does not certify physical equipment as safe.
