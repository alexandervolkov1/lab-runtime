# M3 completion report — 2026-09-15

Status: complete. Scope: OS-independent bounded transport, narrow Metakon protocol,
generic instrument mapping, extensible units and trusted actual-byte output dispatch.
M1/M2 remain green. No M4 or M5 code is included in this report boundary.

## Delivered

- `Unit` is a fixed-capacity value object: canonical ID up to 32 printable ASCII
  bytes and nonblank display symbol up to 16 UTF-8 bytes. Equality uses identity.
  Common constants and configuration-created `sccm`/`rpm` use the same type. There
  is no conversion or dimensional analysis.
- OutputAuthority stores the descriptor's trusted numeric scalar unit. SafeProfile
  numbers inherit that unit. Percent is no longer a branch in authority logic;
  inconsistent binding metadata fails before registration.
- The pure Metakon codec owns CRC/framing/type/address/length rules and exact signed
  representability. It has no transport dependency. Register 1 sentinel becomes
  unavailable SensorFault, never a number. Register 6 output cannot truncate/clamp.
- ResourceExecutor owns one ByteTransport, one active transaction, at most 32 queued
  ordinary transactions and a separate one-item safe slot. Frame max is 38 bytes;
  Runtime owns at most eight resources. Queue/execution durations are at most 60 s.
  Polling performs bounded nonblocking attempts with partial offsets and no history.
- Reads may retry once only when explicitly marked and only after successful adapter
  recovery within the original deadline. Writes never retry after possible bytes.
  Successful fake recovery promises to discard the old response generation.
- Runtime holds physical binding generation and mapping revision. Rebind creates new
  authorities in Unverified and stale queued output cannot send even when logical IDs
  are equal. Queries are still pure; physical reads are explicit Commands.
- The output queue contains a private OutputIntent, not an M2 Dispatch. Runtime
  validates current instance/lease/epoch/deadline/unit/limits/binding/revision in the
  same serialized poll that calls the first bounded write attempt. `Ok(0)` records no
  send and must revalidate. An accepted prefix records in-flight send; later revoke
  cannot undo it. Timeout revokes immediately, then remains unresolved until recovery
  permits terminal Ambiguous evidence. Safe recovery does not rearm.
- Host-side strict JSON supports only `metakon-5x3-v1` operations. It bounds input
  to 16 KiB, nesting to 8 and parameters to 16; serde rejects duplicate/unknown
  struct fields. Raw registers/frames/expressions have no schema path. Core has no
  serialization, COM or OS dependency.

## Acceptance evidence

Tests were committed before their corresponding implementation. Initial red runs
failed on absent Unit/Metakon, transport and instrument APIs. M3 adds 22 tests:

| Area | Evidence |
| --- | --- |
| Extensible/bounded units U1–U6 | milestone3_codec_units; milestone3_instruments; definitions |
| Donor CRC and exact frames C1 | milestone3_codec_units golden-vector tests |
| Strict response/type/address/CRC C2 | read_and_ack_decoding_is_strict |
| Scaling/sentinel/representability C3–C4 | measurement_scaling_and_output_representation_are_explicit; milestone3_metakon_runtime |
| One owner, no interleave, independent resources T1 | milestone3_transport partial_io and separate_resource tests |
| Queue/frame/time bounds and exact expiry T2–T3 | queue_capacity_and_exact_deadline_are_explicit plus constructor validation |
| Partial I/O and recovery T4–T6 | timeout_after_a_prefix; bad_crc_and_late_bytes; generation assertions |
| One read retry/no write retry T7 | retryable_read_gets_at_most_one_retry; partial_write_is_not_retried |
| Actual-byte final gate A1–A4 | milestone3_output_transport revoke/WouldBlock/partial tests; unchanged M2 evidence tests |
| Generation/revision G1–G2 | rebind_fences_queued_intent; atomic binding mismatch |
| Generic discovery/strict data D1–D3 | milestone3_instruments and host definitions tests |

## Verification

All exited 0 on Windows:

- `cargo fmt --all -- --check`
- `cargo test --workspace`: 60 passed, none failed/ignored.
- `cargo test --workspace --release`: 60 passed, none failed/ignored.
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo doc --workspace --no-deps`: no warnings.
- `cargo run -p lab-runtime`: finite M1 descriptor demo unchanged, normal exit.
- `git diff --check`; clean status at the report boundary.

Dependencies added only to the host package: serde/serde_json for strict JSON. Core
remains std-only and OS-independent. Donor remained clean/read-only at HEAD
50d3d1e3de84c650e1aa0ffbf1625044f794d315. No hardware or physical COM test ran.

## Commits

- b50a8a8 — accept SOL phase and reviewer unit correction.
- bb629fc — red unit/codec acceptance.
- cafe5bf — extensible units and pure codec.
- 05c734c — red bounded transport acceptance.
- fcb1a31 — single-owner executor/recovery.
- 47faf35 — red data-instrument/unit-authority acceptance.
- b5d0f90 — red actual-byte final-gate acceptance.
- 9115de6 — generic Metakon instruments and trusted dispatch integration.
- 9480a22 — strict host JSON loader.
- 5f3e61d — sample/sentinel/bad-CRC/late-byte recovery evidence.

## Known limitations / deviations

This is a deterministic synchronous executor model, not a serialport implementation.
An actual OS adapter must prove its calls are bounded and its recovery semantics; no
physical safety claim follows from the fake. Protocol-invalid read responses recover
but are not automatically retried. Only one narrow profile and one numeric scalar
actuator are supported. No arbitrary commands, config writes, Boolean/enum actuators,
durable IDs, autonomous scheduler or long-soak guarantee was added.

M3 design originally described JSON generally; implementation chose the permitted
serde host adapter and added the reviewer-mandated extensible unit correction. No
unresolved authority, ambiguity, lifecycle or trust-boundary contradiction was found.

Next: create M4 design, then implement the native thermal plant/EMA/Reference/PID
pipeline under SOL_HIGH. Do not switch models after M3 and do not begin M5.
