# Workbench guide

[User manual](README.md) · [First experiment](getting-started.md)

Workbench is the Windows window onto a running experiment. Runtime must be started
separately. Closing Workbench does not stop instruments, controllers or recording.

## Connect to Runtime

From the extracted Windows package folder:

```powershell
.\lab-workbench.exe --connect 127.0.0.1:7420 --workspace .\my-workspace
```

Use the port reported by Runtime. Keep one Workbench per workspace folder.
For another computer, use the [local-network guide](distributed-workbench.md).

Workbench connects at startup. **Disconnect** leaves Runtime running and does not
reconnect automatically. Click **Connect** when you want to connect again.

Add `--observe` if you only want to watch. This disables this Workbench's experiment
commands, including recording start/stop, but still lets you view signals and
inspect command status. It does not prevent another connected operator from acting.

## Read the status

| Display | What to do |
|---|---|
| **Fresh** | The displayed data has caught up; available controls can be used |
| **Rebuilding** | Wait while Workbench refreshes its view |
| **Stale** or **Cached last observation (stale)** | Treat displayed values as old; do not use them to judge current equipment state |
| **Unknown** | No value is available yet |
| **Unresolved in the current Runtime view** | That item is not present in the current experiment |

The connection may say **Ready** before the display becomes Fresh. Wait for Fresh.

## Find instruments and measurements

The **Discovery** pane lists instruments, signals, References, controllers,
resources, configuration properties and Recorder.

Select **Instrument 1** to inspect its details. Select **Signal 1/1** to see
parameter 1 from instrument 1. Other configurations use their own numbers.
The details area currently displays structured text, including names, values and
units; it is not a separate instrument dashboard.

## View and adjust plots

Select a **Signal** in Discovery to display its live graph. The standard view also
creates a default live plot when signals become available. No add-plot action is
needed for this workflow.

Drag inside a plot to pan. Double-click the plot to restore automatic bounds.
These changes affect only your view, not measurement or recording.

This GUI has no arbitrary plot/trace editor or **Save Layout** button. You cannot
use it to add named multi-signal dashboards or edit trace colors and stored axis
settings. Do not look for menus that are not present in this preview.

**Waiting for good finite signal events.** means no usable live samples have
arrived for that signal. A flat line can simply mean a steady value. Old display
points eventually leave the plot; **Display window truncated** is not a Recorder
data-loss message. Reconnecting can clear the live graph.

Live graphs are not saved recordings. Workbench has no archive browser or CSV
export button; see [Recording](recording.md).

## Reference and operator controls

A **Reference** is a setpoint. Select one to see **Reference controls**:

- **Configure fixed** uses the **Fixed value** field.
- **Configure ramp** uses **Target** and a positive **Rate**.
- **Retune** changes a Reference that is already a ramp.

For every action, read **Confirmation required**, press **Confirm** once, and
wait. **Accepted / in progress** is not completion. After **Operation completed**,
check the updated item details, then press **Acknowledge** to return to the controls.
**Cancel** discards an unconfirmed draft.

A running physical controller may use the Reference immediately. Review the
equipment's safe limits before changing it. Command completion is not independent
proof that physical equipment reached the requested state.

### Other available actions

| Selected item | Available controls |
|---|---|
| Controller | **Start**, **Pause**, **Resume**, **Reset failed**, **Configure PID** when applicable |
| Writable Integer or Text property | **Configure property** |
| Reconnect-capable resource | **Reconnect** |

Use controller and PID controls only with a reviewed equipment procedure. Pause,
reconnect and reset are not substitutes for an emergency stop, and reconnect does
not automatically rearm a controller. Some displayed properties are read-only.
With required recording, start a recording run before **Start** or **Resume**.
Pause the controller through its reviewed procedure before ending that recording.

A disabled button can mean data is stale, the current state does not allow the
action, observation mode is on, or a previous command is unresolved. Do not work
around it by opening another workspace to repeat the command.

## Recorder

The **Recorder** section shows `authoritative state`. Enter a label, use
**Start recording**, and confirm. After completion, **Acknowledge** restores the
controls. Use **Stop recording** and confirm when finished.

Check the completed stop result and the return to `idle` as described in
[Recording](recording.md). Closing or disconnecting Workbench does not stop a run.

## Workspace and saving

`--workspace` selects the folder Workbench uses for its local settings and command
recovery information. Reopen the same folder to retain that information. Without
the option, Windows normally uses `%LOCALAPPDATA%\lab-runtime\workbench`.

The GUI does not provide **Save**, **Save Layout** or **Save As** for a workspace.
It loads saved presentation settings when available, but does not automatically
save every graph adjustment or persist live plot samples. Keep this distinction
in mind before closing it.

To back up a workspace, close Workbench normally and copy the whole workspace
folder. Do not edit or delete its recovery files to bypass an unresolved command.
The Recorder database is separate and stays on the Runtime computer.

## Loss of connection and recovery

Workbench may briefly show **Reattaching** after an unexpected connection loss.
Wait for Fresh if it reconnects. If it remains **Disconnected**, check Runtime and
the network, then use **Connect**. Reconnecting never repeats a command automatically.

If you see **Unknown / reconciliation required** or **Ambiguous**, a command may
have run even though its answer was lost. Do not press its ordinary action button
again. Open **Recovery / reconciliation** and use **Check Status** when available.
It checks once; it does not keep polling.

`outcome_unknown` means the result is still unknown, not that nothing happened.
**Exact Retry…** is a separate, manual recovery action. It may execute a command
that did not previously run. Do not use it for uncertain physical work without
reviewing the equipment state and the retained command with a responsible operator.

After Runtime restarts, **Connect new scope** may be offered. It does not erase
old uncertainty. **Quarantined recovery evidence** means old commands cannot be
resolved against this running instance, and controls may remain blocked. There is
no **Discard** or **Forget** button. Preserve the workspace and recording and get
help; do not delete files or start a fresh workspace to bypass the block.

## Common problems

For a failed command, read **Operation failed** and its details before acknowledging
it. If a draft became stale, cancel it and review current values. Do not repeat
an operation whose result is unknown.

For startup failures, missing signals, locked workspaces and recording errors,
use [Troubleshooting](troubleshooting.md). Stop Runtime from its own terminal with
Ctrl+C; the Workbench window has no Runtime shutdown button.
