# Current work — M9D external review gate

```text
M8: ACCEPTED
M9A: ACCEPTED
M9B: ACCEPTED
M9C: ACCEPTED
POST-M9C HARDWARE SMOKE: PASS
M9D: READY_FOR_EXTERNAL_REVIEW
M10: NOT AUTHORIZED
M11+: NOT AUTHORIZED
```

M9D is the only active milestone. Its implementation preserves the accepted
OutputAuthority architecture. The read-only shutdown correction is committed at
`b6e1840`; the final focused, debug/release workspace, clippy and warning-denied
rustdoc gates pass.

The final bounded COM5 acceptance used the production native-controller path. It
proved startup safe-zero, one authority-gated +10 percent output, strict ACK,
separate matching register-6 readback, immediate normal pause to verified zero,
Recorder sealing and clean finite shutdown. There was exactly one nonzero write,
zero ambiguous writes and zero mismatches. The heater/load was physically
disconnected, so register command/readback is proven but physical heater effect was
not tested.

The two earlier harness failures remain documented diagnostic evidence. They did not
require production or test changes: terminal `requested` state need not remain
populated, and distinct ACK/readback stages may share one owner timestamp. The final
harness asserts semantic fields and non-decreasing authoritative time, and its full
normal and mismatch-cleanup sequences passed offline before COM5 was opened.

The clean acceptance archive is
`examples/metakon-513-m9d-write-smoke.sqlite`, SHA-256
`14ec74be2a33cb795b29dcc295acc323a0dd4d8cf44b2cc5f6f64fd29e775c32`.
The bounded evidence and exact gate results are in
`docs/implementation/MILESTONE_9D_REPORT.md`. Historical M8 and post-M9C evidence
remain immutable. Do not begin M10 unless M9D is externally accepted and M10 is
explicitly authorized.
