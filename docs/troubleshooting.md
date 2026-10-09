# Troubleshooting

[User manual](README.md)

Start with the message shown in the Runtime terminal or Workbench. Preserve data
before trying repairs. Never repeat an uncertain physical command just to see
whether it works a second time.

## Program will not start

- Extract the entire Windows ZIP and run from the folder containing both `.exe`
  files. Do not use GitHub's source archive.
- If Windows reports `VCRUNTIME140.dll` or a similar missing runtime DLL, install
  the x64 package from [Microsoft's Visual C++ Redistributable page](https://learn.microsoft.com/en-us/cpp/windows/latest-supported-vc-redist).
  Do not download individual DLLs from third-party sites.
- Double-clicking Runtime without arguments runs a short demonstration and exits.
  Use the terminal command in [Getting started](getting-started.md).
- On Linux, a `GLIBC_2.34 not found` error means this binary cannot run on that
  system. Use a compatible system; do not replace system libraries manually.

## Runtime does not report ready

| Message or symptom | Check |
|---|---|
| Address already in use | Another Runtime may be running. Stop your previous instance, or choose an unused port in both programs |
| Usage or invalid argument | Copy the command exactly; Runtime arguments have a required order. Do not mix `--config` and `--profile` |
| Cannot open recording database | Use a writable local directory, free disk space and an absolute path with `--record-db` |
| Configuration or definition cannot be read | Check filenames, extensions and paths relative to the configuration file |
| Invalid configuration | Check spelling, types, unique IDs and UTF-8 without BOM; compare with the supplied virtual example |
| Cannot open COM/device port | Check the selected port, permissions, cable and whether another program owns it |

Do not weaken recording policy or physical limits simply to make startup pass.
For a clean software-only check, use the virtual walkthrough with a new data folder.

## Workbench cannot connect or stays Stale

Check that Runtime is still running and reported ready. The address and port must
match it. On one computer use `127.0.0.1`; from another computer use Runtime's
actual LAN address, not `127.0.0.1`.

For LAN access, both commands need the explicit remote-TCP option. Check the
[firewall and connection steps](distributed-workbench.md). Do not disable the
firewall or expose the port to the Internet as a workaround.

After **Disconnect**, click **Connect** explicitly. **Ready** is not yet **Fresh**.
If a command was interrupted, use [Workbench recovery](workbench.md#loss-of-connection-and-recovery)
instead of submitting it again.

## Empty plot or disabled button

Select a **Signal**, not only its Instrument. Wait for **Fresh observation**.
**Waiting for good finite signal events.** can indicate unavailable or bad readings.
Check the instrument/resource details. A flat virtual temperature near 20 degrees
is expected until its controller is explicitly started.

Click **Acknowledge** after a completed operation to restore the controls. Other
reasons for disabled controls include `--observe`, stale information, an unsuitable
device state or unresolved commands. Read the warning; do not bypass it.
If controller start reports `recording_unavailable` with required recording,
check Recorder: it must be recording, not merely configured or idle. Start a run
and check its completion before preparing another controller-start action.

## Workspace problem

Only one Workbench can use a workspace at a time. Close its other window before
reopening it. Check directory permissions if opening still fails.

If the workspace is damaged or recovery information is quarantined, preserve the
whole folder and seek help. Do not delete recovery files or switch workspaces to
make uncertain operations disappear.

## Recording failed

Stop issuing new experiment commands. Check free space, permissions and the
Recorder message. On physical equipment, follow the laboratory's safe-stop
procedure. A viewer disconnect is not a safe stop.

Keep the database and all sidecars, plus the workspace. See
[Recording](recording.md); neither a visible file nor an accepted stop request
proves that the run finished successfully.

## Collect information for help

Include the package version from `BUILD.txt` on Windows, your startup command
with secrets removed, the error text, and whether the setup is virtual or physical.
Do not post private recordings, access keys or device addresses publicly.

Runtime diagnostic logs normally live at `%LOCALAPPDATA%\lab-runtime\logs` on
Windows. On Linux, use the `state/logs` directory configured by the Linux guide.
Logs help diagnosis but are not recordings. Preserve them without editing the
original experiment data.
