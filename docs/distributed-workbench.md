# Runtime and Workbench on two computers

[User manual](README.md) · [Linux installation](linux-runtime.md)

Run Runtime on a Linux computer near the instruments and view it with Workbench
on a Windows computer. SQLite stays on Linux; no shared network folder is needed.
Start with the virtual experiment before connecting hardware.

**Warning:** raw TCP has no TLS encryption or authentication. Anyone allowed to
connect can send experiment commands. Use only a trusted isolated local network,
never direct Internet access or router port forwarding.

## 1. Choose addresses

This example uses:

| Setting | Example — replace with your own |
|---|---|
| Linux Runtime address | `192.168.1.50` |
| Windows Workbench address | `192.168.1.60` |
| Runtime TCP port | `7420` |
| Linux network interface | `eno1` |

On Linux use `ip -4 address`; on Windows use `ipconfig`. Choose the actual local
network addresses, preferably fixed or reserved in your router. `127.0.0.1` means
this computer, so it cannot identify the other computer.

## 2. Restrict the firewall first

Ask the network administrator to allow incoming TCP **only** from the Windows
address to the Linux address on port 7420, and block other sources to that port.
Keep any existing remote-administration access intact. Neither program configures
the firewall for you.

If Linux already uses **active UFW with default-deny incoming traffic**, first
review `sudo ufw status verbose`. With the sample addresses above, add:

```sh
sudo ufw allow in on eno1 proto tcp from 192.168.1.60 to 192.168.1.50 port 7420
sudo ufw status numbered
```

There must not be another broader rule allowing this port. These commands do not
set up an inactive firewall. If you use a different firewall, apply the same
source/destination/port restriction through its administrator. Do not flush rules,
disable the firewall, or enable a new firewall blindly over a remote session.
See the [UFW manual](https://manpages.ubuntu.com/manpages/noble/man8/ufw.8.html)
for that tool's rule syntax.

## 3. Start Runtime on Linux

From the extracted executable's directory, after completing the Linux installation:

```sh
mkdir -p state/logs
export LAB_RUNTIME_LOG_DIRECTORY="$PWD/state/logs"
./lab-runtime --serve --profile virtual-demo \
  --bind 192.168.1.50 --allow-remote-tcp --port 7420 \
  --record-db "$PWD/state/history.sqlite" --record-policy required
```

Use the Linux computer's own address. Runtime binds just that interface, not all
interfaces. Wait for `"state":"ready"`. This is a replacement for the loopback
command, not a second Runtime to run beside it.

In another Linux terminal, you can check the listener:

```sh
ss -ltn '( sport = :7420 )'
```

Expected: a listening socket at `192.168.1.50:7420`.

## 4. Connect Windows Workbench

In PowerShell, from the extracted Windows package directory:

```powershell
Test-NetConnection -ComputerName 192.168.1.50 -Port 7420
.\lab-workbench.exe --connect 192.168.1.50:7420 --allow-remote-tcp `
  --workspace .\lan-workspace
```

Expected: `TcpTestSucceeded : True`, then Workbench reaches **Fresh**. The port
check alone does not prove that the correct Runtime or measurements are ready.

Select **Signal 1/1** to view the virtual temperature. Use the same Reference and
Recorder controls as in [Your first experiment](getting-started.md#5-change-the-virtual-reference).
For a watch-only station, add `--observe`; that station cannot start/stop recording
or change the experiment. No inbound Workbench firewall rule is needed for this
outgoing connection.

## 5. Finish and locate your data

Stop recording in Workbench and check the completed result. Close Workbench.
Runtime continues on Linux until you press Ctrl+C in its own terminal and wait
for it to exit. The archive remains at `state/history.sqlite` under the Linux
executable directory. Follow [backup instructions](recording.md) before copying it.

If connection fails, check the selected addresses, port, firewall and that Runtime
still runs. Do not widen the firewall to every address as a diagnostic shortcut.

## Other supported arrangements

Runtime can also run on a Windows computer using the same `--bind` and
`--allow-remote-tcp` options; use `lab-runtime.exe` and a Windows absolute database
path. Restrict its Windows inbound firewall rule to the Workbench computer's IP.
A Workbench on that same Runtime computer must also connect to the selected LAN
address, since this command does not additionally listen on loopback.

Workbench also supports WS/WSS connections. Those require a separately configured
secure endpoint; see the [technical transport reference](reference/transports.md).
The separate Workbench presentation API stays loopback-only and must not be
forwarded as a remote-control service.

These instructions do not claim completed physical two-computer testing or
hardware safety qualification. Check the actual network and equipment before use.
