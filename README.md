# lab-runtime

lab-runtime collects measurements from laboratory instruments, displays them in
live plots, and records experiments in a local database. It also supports
controlled changes to instrument settings and setpoints.

Start with the built-in virtual instrument. It needs no connected equipment.

## Two programs

- **Runtime** talks to instruments, runs the experiment, and writes recordings.
  It runs in a terminal and must stay open while the experiment is running.
- **Workbench** is the Windows application for viewing measurements and sending
  operator commands. Closing Workbench does **not** stop Runtime or recording.

They can run on one Windows computer. You can also run Runtime on Linux and use
Workbench from a Windows computer on the same trusted local network.

## Download

Open [GitHub Releases](https://github.com/alexandervolkov1/lab-runtime/releases/tag/v0.1.0-preview.5)
and expand **Assets**. This guide accompanies `v0.1.0-preview.5`, a pre-release.

| Computer | Download |
|---|---|
| Windows x64 | `lab-runtime-v0.1.0-preview.5-windows-x86_64.zip` — Runtime and Workbench |
| Linux x86_64 | `lab-runtime-v0.1.0-preview.5-linux-x86_64.tar.gz` **and** `lab-runtime-v0.1.0-preview.5-linux-x86_64.licenses.tar.gz` |

Download the matching `.sha256` files too. Linux users should also keep the
`.build.json` file with the downloads. Do not select GitHub's **Source code**
archives if you want ready-to-run programs.

The Windows package requires no programming tools. Extract the whole ZIP into a
writable folder; do not run the programs from inside the ZIP. If Windows reports
a missing runtime DLL, follow [installation troubleshooting](docs/troubleshooting.md#program-will-not-start).

The Linux binary requires glibc 2.34 or newer. Follow the
[Linux instructions](docs/linux-runtime.md) for checks and extraction.

## Try a virtual instrument on Windows

Open the extracted folder containing `lab-runtime.exe` and `lab-workbench.exe`.
In File Explorer's address bar, type `powershell` and press Enter.

Start Runtime:

```powershell
.\lab-runtime.exe --serve --profile virtual-demo --port 7420
```

Wait for a line containing `"state":"ready"`. Keep this terminal open.

Open a second PowerShell window in the same folder and start Workbench:

```powershell
.\lab-workbench.exe --connect 127.0.0.1:7420 --workspace .\quickstart-workspace
```

Wait for **Fresh**, then select **Signal 1/1** under **Discovery** to see the
virtual temperature plot. A flat line near 20 degrees is normal: the virtual
controller starts inactive.

This short example does not enable recording. Close Workbench, then press
**Ctrl+C** in the Runtime terminal and wait for the prompt to return.

## Learn the workflow

Start with [Your first experiment](docs/getting-started.md): view a signal,
change a virtual setpoint, record a run, and shut down cleanly.

The [User manual](docs/README.md) then covers:

- daily [Workbench use](docs/workbench.md);
- [recordings and backups](docs/recording.md);
- [configuration and read-only instruments](docs/configuration.md);
- [Linux Runtime](docs/linux-runtime.md) and [two-computer setup](docs/distributed-workbench.md);
- [common problems](docs/troubleshooting.md).

Writing integrations or building the software? Use the separate
[Developer and API documentation on GitHub](https://github.com/alexandervolkov1/lab-runtime/blob/v0.1.0-preview.5/docs/developer/README.md).
It needs an Internet connection and is not needed for the walkthrough.

## Safety and license

This is a developer preview, not a safety-certified control system. Test with
virtual instruments first. Before connecting real equipment, verify the wiring,
device settings, safe limits, and independent emergency-stop procedure.

Never assume that a lost connection means a command did not run. Do not blindly
repeat an uncertain physical command. Closing Workbench is not an emergency stop.

Raw TCP connections have no encryption or authentication. Use them only locally
or on a trusted isolated network, never directly over the Internet.

The project uses the [MIT license](LICENSE). Distributed packages also contain
third-party license notices; keep those materials with any copies you distribute.
