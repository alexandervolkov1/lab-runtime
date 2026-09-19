# Post-M9C real-device hardware smoke

## Result

```text
POST-M9C HARDWARE SMOKE: PASS
M10: NOT AUTHORIZED
```

Date: 2026-09-18 (Europe/Moscow).

This was a narrow supplementary read-only smoke, not a repeat of M8 acceptance.
Production code and tests were not changed. No physical output operation was
configured or issued, and no device power-cycle or fault injection was performed.

## Identity and read-only setup

- Git HEAD: `1e80b04f0f24cf88cb7bc4b2f44a35677e8c1285`.
- Runtime binary SHA-256:
  `1dd34a3455581e25400a93e87af37d6d308ac711efbf41bf63e22533ae169f1e`.
- Final smoke deployment SHA-256:
  `21e11f4d84def992715d2dd7c755c39f4439c09055fc6e5d11573005362adfc9`.
- Instrument definition SHA-256:
  `b631a78a13b9126c430c50732ac1fb3f0739c3e7da7664ef1591e4ce178c65eb`.
- Device: Metakon 513 on COM5, address 5, channel 0.
- Serial settings: 9600 baud, 8N1, no flow control.
- Protocol: custom Metakon read `0x00`, CRC initial value `0xFF`, little-endian.
- Register 0 was the read-only channel-type probe; register 1 was the read-only
  signed temperature value with scale 1.0.
- The definition contains no actuator parameter; both parameters have
  `write_effect = none`.

## Failed diagnostic run

The original assessed run used boot ID `10a83e21032985deea23852b133b9560`.
COM5 opened, the compatibility probe returned Good integer `channel_type = 3`, and
four consecutive generation-1 Good temperatures reached the Application API and
subscription. Required Recorder sealed one run and interval with complete coverage,
three Good rows at 22.0 degrees Celsius, no gaps, and no output events.

Exactly one public reconnect completed and advanced the binding generation from 1
to 2. The repeated channel-type probe was Good. The harness then read the latest
generation-2 temperature as:

```text
observed_at_ns = 6468596900
quality        = Unavailable
failure        = Transport
```

It incorrectly labelled that existing value as the first ordinary temperature poll
and stopped. The deliberately incomplete failure evidence was preserved before any
new run:

```text
examples/metakon-513-post-m9c-smoke-failed.sqlite
size: 4096 bytes
SHA-256: da63a52bdf62b3fc863995d79f497584be3ba97c1de4c55d1d301afbafab6cfc

examples/metakon-513-post-m9c-smoke-failed.sqlite-wal
size: 803432 bytes
SHA-256: d68793297c7786cda025feccf058fa541b5cedce0d7ff6b9f44b77f21459bc37
```

No SHM file remained. The WAL is retained with the main file because the harness
terminated that run instead of performing normal shutdown. These files are
diagnostic evidence, not acceptance evidence.

A bounded Recorder-off diagnostic rerun observed the same explicit generation-2
Unavailable baseline and then at least three consecutive ordinary generation-2 Good
temperature measurements. The resource stayed Ready/idle and acquisition recovered
without a second reconnect. That diagnostic harness requested shutdown too early and
received `cleanup_complete = false`, `transports_closed = false`, and
`unfinished_transports = 1`; it was therefore not used as acceptance evidence. The
final harness synchronized shutdown on the authoritative resource becoming idle,
without an arbitrary sleep.

## Root cause investigation

The raw transport root cause requested by the failed smoke does not exist: the
reported `Transport` value was not a Disconnected, Timeout, CRC, malformed/short
frame, unexpected-register, closed-worker, or other COM5 response. It was the
intentional synthetic invalidation baseline created by Core during rebind.

The exact implementation order is:

1. `Command::ReconfigureMetakon` creates an Unavailable/Transport sample for every
   signal of the replacement generation and clears pending reads for that instrument
   (`crates/lab-core/src/runtime.rs`, rebind path around lines 1742-1755).
2. Ordinary acquisition remains quiesced. The host queues only the compatibility
   probe and explicitly treats an unchanged rebind baseline as pending rather than a
   probe failure (`apps/lab-runtime/src/host.rs`, lines 871-915 and 1182-1224).
3. After the Good integer-3 probe and durable lifecycle commit, the service releases
   the resource (`apps/lab-runtime/src/service.rs`, lines 1668-1702).
4. Release schedules ordinary acquisition at `activation time + poll period`, which
   prevents a catch-up read or overlap (`apps/lab-runtime/src/host.rs`, lines
   1226-1252).

The accepted M8 archive is an independent hardware oracle for the same behavior. Its
first generation-2 measurement records are: channel baseline Unavailable/Transport
at record 741, temperature baseline Unavailable/Transport at record 742, Good
channel-type probe at record 743, then the first ordinary Good temperature at record
748. The post-M9C ordering therefore did not change.

Current tests encode the same contract:
`c14_reconnect_replacement_stays_quiesced_until_probe_and_lifecycle_activation`
requires the temperature to remain explicitly Unavailable through probe/lifecycle
and become Good only after the fresh scheduler deadline;
`reconnect_probe_fact_precedes_durable_activation_and_later_good_temperature` checks
the Recorder ordering.

Classification: **test/smoke harness defect**. The harness confused the authoritative
generation-reset baseline with a newly completed ordinary read. Successful reconnect
guarantees a compatible replacement and release of ordinary acquisition; it does
not rewrite every current signal to Good before the first fresh scheduled poll. The
acceptance requirement itself remains unchanged: a later, distinct ordinary
generation-2 observation must become Good. No production fix or semantics change was
required.

## Final acceptance run

Exactly one clean final acceptance run used boot ID
`04ce94c2b7b1e2b262d1d9db90b42e74` and the same read-only hardware path.

The raw NDJSON Application API exercised `hello`, `discover`, `resource`,
`measurements_current`, `measurement_window`, `subscribe`, `unsubscribe`,
`recording_start`, `recording_status`, `recording_stop`, exactly one
`reconnect_resource`, and normal `runtime_shutdown`. No `runtime_snapshot`, Lua,
Babashka, or product-client path was used.

Initial results:

- resource 1 was the physical COM5 resource, Ready/idle at binding generation 1;
- channel type was Good integer 3;
- four consecutive generation-1 Good temperatures were observed at 24.0 degrees
  Celsius (range 24.0-24.0);
- current, recent history, and live subscription exposed the same physical signal;
- Required Recorder start crossed its documented durable boundary;
- Recorder persisted three new Good temperature rows at 24.0 degrees Celsius;
- stop reported accepted facts drained, transaction committed, run sealed, and
  interval sealed.

Reconnect results:

- the sole reconnect operation completed successfully and advanced generation 1 to
  generation 2;
- the generation-2 channel probe was Good integer 3;
- the explicit temperature rebind baseline was Unavailable/Transport at
  `6354362500 ns`, before the probe at `6400002600 ns`;
- four subsequent ordinary generation-2 temperature polls were Good at 24.0 degrees
  Celsius, spanning `7453457900 ns` through `10451270100 ns`;
- a fresh subscription received another Good generation-2 sample at
  `11451898400 ns`;
- current and recent history reflected generation 2; the queried window contained
  11 Good generation-2 temperature observations;
- no stale generation-1 value re-entered authoritative current state.

Shutdown completed with `cleanup_complete = true`, `exit_success = true`,
`transports_closed = true`, zero unfinished transports/workers, Recorder flushed,
and process exit code 0. COM5 was no longer held; no runtime process or API listener
remained.

The new clean acceptance archive is:

```text
examples/metakon-513-post-m9c-smoke.sqlite
size: 204800 bytes
SHA-256: 098f2fdc31805cbcdfe46d04055a205c9f980fcf6955ccf8ede6043cbc00a9fc
WAL: absent after normal close
SHM: absent after normal close
```

Independent read-only SQLite inspection found one sealed run, one sealed interval,
complete coverage, three recorded Good generation-1 temperature rows, zero gaps,
zero output events, and a sealed Runtime boot. The live API likewise reported zero
output events.

## Preserved M8 evidence

The accepted M8 evidence remained byte-identical after all diagnostics and the final
run:

```text
examples/metakon-513-com5-transient-open-retry-history.sqlite
SHA-256: 63ecb8575be5ac82ef968b929e8b3098b051a0f30a55c190b7fdc1dcaf64dc80

examples/metakon-513-com5-prepared-reconnect-history.sqlite
SHA-256: 1396421e62b5a1abb834b4178689b3303277a88d46174e0353d2710c2ab17023
```

M10 remains unauthorized.
