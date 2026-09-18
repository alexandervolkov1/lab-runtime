# Milestone 9D — physical Metakon output integration

## Status

```text
M9D: HARDWARE ACCEPTANCE BLOCKED
M10: NOT AUTHORIZED
```

Date: 2026-09-18 (Europe/Moscow).

M9D was inserted before M10 to complete the audited partial Metakon output path
without changing the accepted OutputAuthority architecture. The software work is
complete, but M9D is not ready for external review because its mandatory read-only
hardware preflight did not close the transport cleanly. In accordance with the stop
gate, no physical output write was attempted.

## Software implementation

Hardware-test HEAD:

```text
ea49a61715e94dfd559a5d360e928410e0116b82
```

Exact tested artifacts:

```text
target/release/lab-runtime.exe
SHA-256 533661e9d850593966191c26b97e803ef2e12a8afe661c7967831e22f0e56232

examples/runtime.metakon-513-m9d-read-preflight.toml
SHA-256 50d1d26d515aa021e7522499e404c6729f2456a159983bdd77dc3a03d7464a7f

examples/runtime.metakon-513-com5-output.toml
SHA-256 82aaea40998965fc021dd08c9aea254e9c514bcb41f1b73ace31037b924061d8

examples/definitions/metakon-513-output.json
SHA-256 08cc2b1c3b7a945167297c4eccc5ed5c3cd7eab22edb8e48d54159d3667e2492
```

Commits:

```text
45f477436a98a36443b8cb2df34ab54a7aef9cef  feat: integrate authority-gated Metakon output
ea49a61715e94dfd559a5d360e928410e0116b82  test: add read-only M9D hardware preflight
```

The active path is:

```text
native controller
-> OutputProposal
-> OutputAuthority reservation and lease/epoch checks
-> bounded ResourceExecutor
-> final authority and binding-generation recheck
-> Metakon WRITE 0x01, register 0x06, signed I8
-> strict five-byte ACK
-> separate Metakon READ 0x00, register 0x06
-> signed-I8 readback comparison
-> ReadbackVerified or explicit bounded failure
```

The output definition uses percent at scale 1.0 and raw range -100..=100. Active
controller limits are 0..=10 and safe value is 0. The same authority-gated transport
path carries normal proposals and safe-zero. ACK remains distinct from readback and
neither is called physical effect. A mismatching readback records the reported value,
fails the dispatch and requests safe state. A readback timeout is ambiguous; the
original write is not retried blindly.

The concrete Windows COM module is crate-private. Application API callers,
controllers, ordinary instruments and managed components have no raw register-write
surface. The Application operation registry was not extended with `set_output`,
`raw_write` or `write_register`.

## Software verification

Before COM5 was opened, these gates passed:

```text
cargo fmt --all -- --check
cargo test --workspace
cargo test --workspace --release
cargo clippy --workspace --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
```

Focused acceptance also passed for exact signed-I8 frames at 0, +10, -10 and limits
+/-100; range rejection; strict ACK validation; ACK/readback separation; mismatch;
readback timeout without write retry; stale authority/generation before first byte;
generic configured controller binding; safe-zero through the physical path; virtual
ThermalPlant regression; protocol inventory; emulator/physical isolation; and M9B.8
fault acceptance.

One pre-existing Recorder timing test produced one transient failure during an
earlier debug run and then passed unchanged in the complete rerun and release suite.
No test relaxation or production workaround was made.

## Read-only hardware preflight

Setup:

- Metakon 513 on COM5, address 5, channel 0;
- 9600 baud, 8N1, no flow control;
- READ opcode 0x00, CRC initial value 0xFF;
- heater/load physically disconnected as confirmed by the operator;
- read-only definition with no actuator and Recorder disabled.

The second bounded preflight boot was
`2d5cd99eda6c48663218bc76c571061d`. It proved:

```text
COM5 open: successful
resource generation: 1
resource acquisition state: idle between transactions
channel_type: Good integer 3
temperature attempt 1: Good 28.0 degC at 24050565800 ns
temperature attempt 2: Good 28.0 degC at 25051834100 ns
temperature attempt 3: Good 28.0 degC at 26053839800 ns
temperature attempt 4: Good 28.0 degC at 28052517400 ns
```

The first harness run requested shutdown while an ordinary serial transaction was
pending and ended with incomplete cleanup. The corrected read-only run observed the
authoritative resource state `idle` immediately before requesting normal shutdown.
It still returned:

```text
operation state: failed
safe_confirmed: true
recorder_flushed: true
unfinished_workers: 0
unfinished_transports: 1
transports_closed: false
cleanup_complete: false
exit_success: false
process exit code: 1
```

Both processes exited and released COM5. The repeat after the real idle predicate
makes this a meaningful preflight blocker rather than treating the first harness
ordering error as acceptance evidence. Root cause is not established; it may be a
hardware-facing shutdown/serial-environment issue and must be classified before a
write acceptance run.

## Hardware write acceptance

Not started. No WRITE opcode 0x01 frame, including safe-zero, was sent to COM5. No
nonzero proposal was armed. No M9D Recorder archive or acceptance hash exists:

```text
examples/metakon-513-m9d-write-smoke.sqlite: absent
```

## Preserved evidence

Historical M8 evidence remained byte-identical:

```text
examples/metakon-513-com5-transient-open-retry-history.sqlite
SHA-256 63ecb8575be5ac82ef968b929e8b3098b051a0f30a55c190b7fdc1dcaf64dc80

examples/metakon-513-com5-prepared-reconnect-history.sqlite
SHA-256 1396421e62b5a1abb834b4178689b3303277a88d46174e0353d2710c2ab17023
```

The post-M9C read-smoke archive was not modified.

## Remaining gate

Classify and resolve the finite read-only shutdown failure without exercising output
against the connected device. Only after a clean read-only preflight may one bounded
production-path sequence proceed: safe-zero, authorized +10, normal safe transition
to zero, ACK plus distinct register-6 readback, and clean shutdown. M10 remains
unauthorized.
