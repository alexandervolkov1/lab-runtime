# Optional furnace API check

This is the project's one external client example. It needs Babashka 1.13.220
(`bb --version`); Runtime, Core, Cargo tests, configuration and application
packages do not depend on it. Install it only to run this check.

Start the built-in furnace with Recorder, then follow the
[user guide](../../docs/README.md#внешняя-проверка) for TCP, WS and WSS commands.
Run `bb clients/babashka-smoke/manual-output.clj --help` for options.
Without `--shutdown` the client leaves Runtime serving after sealing its own run.
With that flag it also requires authoritative `recorder_flushed:true`.

The scenario checks discovery/descriptors, current temperature, several fresh
subscription measurements without polling, 10 → 20 → 15% manual power, lease
renewal and expiry, safe confirmation, fixed setpoint, PID start/pause/resume,
PID/manual conflict, durable run completion and archived measurement/run pages.
It targets only the built-in virtual furnace. It does not qualify physical hardware.

Responses and subscription events are demultiplexed by envelope type and message
ID. Waits have finite deadlines; failure exits with code 1, success prints a JSON
`PASS` record. The transcript preserves sent command identities and results.
The client never reconnects or automatically repeats a command, including status
or history work. After an uncertain failure inspect the last request and rebuild
state deliberately using the [API recovery contract](../../docs/api/events-mutations-and-recovery.md).
Use a fresh furnace process for each complete check.

WSS uses the normal certificate and hostname validation of the HTTP client.
`LAB_RUNTIME_ACCESS_KEY`, when set, supplies tuna's `X-Token` header; the script
does not log it. A key is sent only over WSS or loopback WS.
