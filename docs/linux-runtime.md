# Run Runtime on Linux

[User manual](README.md) · [Connect Windows Workbench](distributed-workbench.md)

The Linux package runs Runtime without a desktop. Workbench is the separate
Windows application. The supplied Linux binary is for x86_64 GNU/Linux, not ARM
or musl-only systems.

## Download and check

From the [release page](https://github.com/alexandervolkov1/lab-runtime/releases/tag/v0.1.0-preview.5),
download these files into the same directory:

- `lab-runtime-v0.1.0-preview.5-linux-x86_64.tar.gz`;
- `lab-runtime-v0.1.0-preview.5-linux-x86_64.licenses.tar.gz`;
- their two `.sha256` files;
- `lab-runtime-v0.1.0-preview.5-linux-x86_64.build.json`.

The binary needs glibc **2.34 or newer** and system `libgcc_s`, `libm`
and `libc`. It was checked on Arch Linux WSL2 with glibc 2.44; this is not a promise
of compatibility with every Linux distribution. Check the local system:

```sh
uname -m
getconf GNU_LIBC_VERSION
sha256sum -c lab-runtime-v0.1.0-preview.5-linux-x86_64.tar.gz.sha256
sha256sum -c lab-runtime-v0.1.0-preview.5-linux-x86_64.licenses.tar.gz.sha256
```

Expected: `x86_64`, glibc at least 2.34, and `OK` for both archives. On a mismatch,
do not run the archive. Download it again from the release page.

## Extract the package and licenses

Run in the download directory. Use a new directory if `lab-runtime-preview5`
already exists:

```sh
mkdir lab-runtime-preview5
tar -xzf lab-runtime-v0.1.0-preview.5-linux-x86_64.tar.gz -C lab-runtime-preview5
tar -xzf lab-runtime-v0.1.0-preview.5-linux-x86_64.licenses.tar.gz \
  -C lab-runtime-preview5/lab-runtime-v0.1.0-preview.5-linux-x86_64
cd lab-runtime-preview5/lab-runtime-v0.1.0-preview.5-linux-x86_64
```

Keep the extracted license materials with the executable, especially when copying
it to another computer. `NOTICE.txt` explains why the companion is required.
The Linux archive includes this guide, the user manual
and safe configuration examples, but no Workbench executable. Open `docs/README.md`
for the manual; the built-in virtual profile below needs no configuration file.

## Start a local virtual experiment

```sh
mkdir -p state/logs
export LAB_RUNTIME_LOG_DIRECTORY="$PWD/state/logs"
./lab-runtime --serve --profile virtual-demo --port 7420 \
  --record-db "$PWD/state/history.sqlite" --record-policy required
```

Expected: a line containing `"state":"ready"` and `"port":7420`. Keep the terminal
open. This command opens no serial hardware and listens only on this Linux
computer. Recording is available, but a run starts only when requested in Workbench.

The database is `state/history.sqlite` below the extracted executable. It stays
on Linux even when you view the experiment from Windows. Use a fresh directory
for separate experiments; do not delete old recordings to repeat a walkthrough.

For a Windows computer to connect, stop this local instance and follow
[Two computers on a trusted network](distributed-workbench.md). That guide changes
the listening address explicitly and checks the firewall first.

## Stop Runtime

Press **Ctrl+C** once in its terminal and wait for it to exit. If it is managed by
an administrator, SIGTERM also requests normal shutdown. Do not use a forced kill
as the normal stop procedure. SIGHUP stops Runtime; it is not a reload command.

Closing Windows Workbench does not stop this process. Follow
[Recording and backups](recording.md) before copying the database and sidecars.

## Serial deployment

The [Configuration Guide](configuration.md#a-real-instrument-read-only) supplies
complete read-only configuration texts that can also be saved on Linux. Replace
`COM3` with the actual device path, such as `/dev/ttyUSB0`, preferably a stable
`/dev/serial/by-id/...` path. Keep the resource kind `windows_com_read_only`:
that spelling is shared by both platforms.

The Runtime account needs permission to open the device; ask the administrator to
configure the host's device-access group or rule. Do not run as root just to avoid
a permissions error. Confirm serial settings and measurement scale with the
instrument owner. Use a writable local recording path.

No physical two-computer or hardware safety qualification is implied by the
virtual walkthrough. See [Troubleshooting](troubleshooting.md) if startup fails.
