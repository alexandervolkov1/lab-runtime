# Your first experiment

[User manual](README.md) · [Workbench guide](workbench.md)

This walkthrough uses the Windows portable package and a simulated temperature
instrument. It opens no serial ports and controls no physical equipment.
No programming tools or source checkout are needed.

## 1. Extract the Windows package

Download the Windows ZIP from the [release page](https://github.com/alexandervolkov1/lab-runtime/releases/tag/v0.1.0-preview.5).
Use **Extract All**, then open the folder containing both `.exe` files.
Keep the accompanying folders and notices together.

Type `powershell` in that folder's File Explorer address bar and press Enter.
All Windows commands below run from this folder, not from `docs`.

If you already ran the README example, close its Workbench and stop its Runtime
with Ctrl+C first. Only one Runtime can listen on port 7420 at a time.

## 2. Start Runtime with recording available

Create a new folder for this walkthrough. If `first-run` already exists, choose
a different name here and in the Workbench command; do not delete an old run.

```powershell
$run = [IO.Path]::GetFullPath((Join-Path $PWD "first-run"))
New-Item -ItemType Directory -Path $run -ErrorAction Stop | Out-Null
.\lab-runtime.exe --serve --profile virtual-demo --port 7420 `
  --record-db (Join-Path $run "history.sqlite") --record-policy required
```

Expected: Runtime stays running and prints a line containing
`"port":7420` and `"state":"ready"`. The `first-run\history.sqlite` file is
created. The database is ready, but a recording run has not started yet.

Keep this terminal open. Do not close it to stop the program; use Ctrl+C when
you reach the last step.

## 3. Open Workbench and wait for Fresh

Open another PowerShell window in the same extracted package folder:

```powershell
.\lab-workbench.exe --connect 127.0.0.1:7420 --workspace .\first-run\workspace
```

Expected: the Workbench window opens. Its connection goes through startup states,
and the top row eventually shows **Fresh**. This means the displayed information
has caught up with Runtime. Wait if it says **Rebuilding**; do not act on **Stale**
values. If it never becomes Fresh, use [troubleshooting](troubleshooting.md).

## 4. Find the virtual instrument and plot

1. In the **Discovery** list, select **Instrument 1**. Its details identify the
   virtual thermal plant.
2. Select **Signal 1/1**, its temperature measurement.
3. Look for **Fresh observation** and the live graph in the main pane.

Selecting the signal displays its plot; there is no **Add graph** button.
The line is normally near 20 degrees and can be flat. The virtual controller
has not been started. Values here are simulated, not measurements from a device.

## 5. Change the virtual Reference

A Reference is a setpoint: the value a controller can aim for.
This example uses a changing setpoint, or ramp.

1. Select **Reference 1** in Discovery.
2. Under **Reference controls**, enter `30` in **Target** and `1` in **Rate**.
   Here the units are degrees Celsius and degrees Celsius per second.
3. Click **Retune**.
4. Under **Operator action**, read **Confirmation required**, then click
   **Confirm** once.
5. Wait for **Operation completed** and check the Reference details for target
   `30` and rate `1`. Click **Acknowledge** to return to the controls.

The Reference changes, but temperature will not follow until a controller runs.
Leave the controller inactive for now: with required recording, recording must
start before the controller. The next step shows the correct order.

If the draft becomes stale, click **Cancel**, wait for Fresh and prepare it again.
If the outcome is unknown, do not submit another command: follow
[connection recovery](workbench.md#loss-of-connection-and-recovery).

## 6. Record and finish a run

1. Find the **Recorder** section below the operator controls. It should show
   `authoritative state: idle`. Scroll the main pane if needed.
2. Enter `First virtual run` in the text box next to **Start recording**.
3. Click **Start recording**, review, then **Confirm** once.
4. Wait for **Operation completed**, then click **Acknowledge**. Recorder should
   now show `authoritative state: recording`.
5. Optionally, for this virtual example only, select **Controller 1**, click
   **Start**, review and **Confirm**, wait for completion, then **Acknowledge**.
   Return to **Signal 1/1** to see the simulated temperature respond. Do not use
   this as a procedure for starting physical equipment.
6. Let it record for about ten seconds. If you started the virtual controller,
   select **Controller 1**, click **Pause**, review and **Confirm**, wait for
   completion and **Acknowledge** before stopping the recording.
7. Click **Stop recording**, review and **Confirm** once. Wait for
   **Operation completed**. Its result includes `run_sealed` and
   `transaction_committed` set to `true`; then click **Acknowledge**.
8. Check that Recorder has returned to `authoritative state: idle`.

Do not treat an **Accepted / in progress** message or the existence of a database
file as proof that recording finished. On an error, preserve the files and use
[Recording](recording.md). Workbench does not have a saved-recording browser.

## 7. Close Workbench

Close the Workbench window normally. The Runtime terminal remains running.
Closing the viewer does not stop the experiment, its controller or an active
recording. This is intentional: Runtime does not depend on the viewer staying open.

You can reopen Workbench with the same command while this Runtime is still running
and wait for Fresh again. Close it before the next step.

## 8. Stop Runtime and check the file

In the Runtime terminal, press **Ctrl+C** once and wait for the PowerShell prompt
to return. Runtime shuts down its experiment and recording. Do not force-close
the terminal while it is stopping.

From the package folder, check the archive:

```powershell
Get-Item .\first-run\history.sqlite | Select-Object FullName, Length, LastWriteTime
```

Expected: a nonempty SQLite file at the path used in step 2. The completed stop
result in step 6 is the recording result; this file check only confirms its
location. Keep any matching `-wal` and `-shm` files too. See
[safe backups](recording.md#keep-and-copy-a-recording) before moving data.

You have now viewed a measurement, changed a virtual setpoint, recorded a run and
closed both programs. Continue with [daily Workbench use](workbench.md) or
[your own configuration](configuration.md).
