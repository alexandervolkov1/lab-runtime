# Configuration guide

[User manual](README.md) · [Troubleshooting](troubleshooting.md)

Use a configuration file when you want to keep instrument and recording settings
between launches. Stop Runtime before editing the file; editing it does not change
a running experiment. The GUI does not have a configuration-file reload button.

## Where to keep files

The Windows package contains safe examples in `examples`. Keep those originals.
Create your own writable folder beside the executables:

```powershell
New-Item -ItemType Directory -Path .\my-config -ErrorAction Stop | Out-Null
Copy-Item .\examples\runtime.minimal.toml .\my-config\runtime.toml
.\lab-runtime.exe --serve --config .\my-config\runtime.toml
```

The supplied example chooses a free port. Read `port` from the ready message and
use that number with Workbench. It writes `my-config\history.sqlite`.

Do not combine `--config` with `--profile`, `--record-db` or `--port`. In this mode,
edit the file to select the port and database. All commands assume the package
folder is your current directory.

## Minimal virtual configuration

For a fixed port, this complete `runtime.toml` gives one simulated temperature
signal without opening hardware:

```toml
schema_version = 1

[runtime]
key = "minimal-virtual"
display_name = "Minimal virtual laboratory"

[server]
host = "127.0.0.1"
port = 7420

[recording]
enabled = true
path = "history.sqlite"
policy = "required"

[[instruments]]
id = 1
key = "temperature"
kind = "virtual_measurement"
display_name = "Virtual temperature"
history_capacity = 64
base_temperature = 20.0
poll_period_ms = 100
```

Save as UTF-8 without BOM, with a `.toml` extension rather than `.toml.txt`.
Start it with the same `--serve --config` command above. Connect Workbench to
`127.0.0.1:7420` and select **Signal 1/1**. This small configuration has no controller
or Reference. For the virtual controller example, copy `examples/runtime.virtual.toml`
instead; use a different directory so each example has its own database.

## Choose the recording path

In the `[recording]` section:

```toml
[recording]
enabled = true
path = "history.sqlite"
policy = "required"
```

A relative path is based on the configuration file's folder, not the executable's
folder. The containing directory must exist and be writable. Use local storage;
network shares and `..` parent traversal are rejected.

To use a Windows absolute path, forward slashes avoid backslash escaping in TOML:
`path = "D:/LabData/run-01.sqlite"`. Create `D:\LabData` first and substitute your
actual drive. With command-line profile startup, `--record-db` must already be an
absolute path. See [Recording](recording.md) for run labels, completion and backups.

## A real instrument, read-only

This example is for a **Metakon 513 with a matching whole-degree thermocouple
configuration**, not for an arbitrary serial device. Have the device owner verify
the model, channel type, address, scale, wiring and communication settings first.
Read-only still opens the port and sends read requests. It does not certify the
connected apparatus as safe.

Create a new `metakon-read-only` folder beside the executables. Using a text editor,
save the following two files there as UTF-8 without BOM. Both full contents are
provided here; no source checkout or extra package files are required.

### runtime.toml

```toml
schema_version = 1

[runtime]
key = "metakon-read-only"
display_name = "Metakon read-only temperature bench"

[server]
host = "127.0.0.1"
port = 7420

[recording]
enabled = true
path = "metakon-history.sqlite"
policy = "required"

[[resources]]
id = 1
key = "temperature-bus"
kind = "windows_com_read_only"
port = "COM3"
baud_rate = 9600
data_bits = 8
parity = "none"
stop_bits = 1
flow_control = "none"
read_timeout_ms = 50
write_timeout_ms = 50
open_timeout_ms = 2000
recovery_timeout_ms = 2000

[[instruments]]
id = 1
key = "metakon-temperature"
kind = "metakon"
definition = "metakon-temperature.json"
resource_id = 1
address = 1
poll_period_ms = 1000
queue_timeout_ms = 1000
transaction_timeout_ms = 1000
```

In Windows Device Manager, find the adapter under **Ports (COM & LPT)** and replace
`COM3` with its actual port. Match baud rate, data bits, parity, stop bits, flow
control and device `address` to the instrument. These sample values are not auto-detected.
Keep `kind = "windows_com_read_only"` and do not add controllers or output settings
for this procedure.

### metakon-temperature.json

```json
{
  "schema_version": 1,
  "profile": "metakon-5x3-v1",
  "id": 1,
  "name": "Metakon 513 thermocouple whole-degree temperature",
  "parameters": [
    {
      "id": 1, "name": "channel_type", "value_type": "integer",
      "unit": { "id": "1", "symbol": "1" }, "min": 0, "max": 255,
      "role": "diagnostic", "access": "read_only",
      "operation": "channel_type", "scale": 1, "write_effect": "none"
    },
    {
      "id": 2, "name": "temperature", "value_type": "float",
      "unit": { "id": "degC", "symbol": "°C" }, "min": -999.0, "max": 9999.0,
      "role": "measurement", "access": "read_only",
      "operation": "temperature", "scale": 1.0, "write_effect": "none"
    }
  ]
}
```

This definition uses a temperature scale of `1.0`. Do not substitute it for a
different sensor/channel format or assume all devices report the same units.

### Start only after the hardware review

Stop any other Runtime on port 7420, then run:

```powershell
.\lab-runtime.exe --serve --config .\metakon-read-only\runtime.toml
```

After ready, connect Workbench to `127.0.0.1:7420`. Select **Signal 1/2** for the
measured temperature. Compare it with the instrument's display before trusting or
recording it. Missing readings or an implausible scale are reasons to stop and
review the definition, not to guess at write commands. Ctrl+C stops Runtime.

## When configuration fails

Runtime prints an error and does not announce ready. Check the filename, saved
encoding, quotation marks, field spelling, COM port and definition path. Unknown
fields are rejected. Do not remove safety settings to work around a validation error.

Keep each configuration, its JSON definitions and database location together.
See [Troubleshooting](troubleshooting.md#runtime-does-not-report-ready) for common
failures. The separate [Configuration reference on GitHub](https://github.com/alexandervolkov1/lab-runtime/blob/v0.1.0-preview.5/docs/reference/configuration.md)
contains all CLI options, field types and limits for technical configuration work.
