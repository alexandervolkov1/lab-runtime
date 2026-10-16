# Run Runtime on Linux

Build with Rust 1.95.0, Python 3.11+ for packaging, a C compiler and binutils.
`cargo build -p lab-runtime --release --locked` builds the native application.
The supplied candidate target is x86_64 GNU/Linux; check its `.build.json` for
the measured glibc symbol requirement and actual build host.

Verify both `.sha256` files. Extract the Runtime `.tar.gz` into a new directory,
then its matching `.licenses.tar.gz` into the directory containing `lab-runtime`.
Keep all license materials with the executable when redistributing it.

```sh
mkdir state
./lab-runtime --serve --profile virtual-demo --port 7420 \
  --record-db "$PWD/state/history.sqlite" --record-policy required
```

Wait for `"state":"ready"`. Connect through the [Application API](api/README.md)
and explicitly start a recording run. Ctrl+C or SIGTERM requests normal bounded
shutdown; SIGHUP stops Runtime and is not a reload command. See
[recording and backups](recording.md).

## Serial deployment

For serial deployment use the [configuration guide](configuration.md), replacing
COM placeholders with an operator-verified device path, preferably a stable
`/dev/serial/by-id/...` path. The historical resource spelling
`windows_com_read_only` is shared by both platforms. Configure device permissions
and local storage for the Runtime account. Physical qualification is separate.
