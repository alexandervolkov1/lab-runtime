# Current work — M9D hardware gate blocked

```text
M8: ACCEPTED
M9A: ACCEPTED
M9B: ACCEPTED
M9C: ACCEPTED
POST-M9C HARDWARE SMOKE: PASS
M9D: HARDWARE ACCEPTANCE BLOCKED
M10: NOT AUTHORIZED
M11+: NOT AUTHORIZED
```

M9D is the only active milestone. Its software implementation is committed at
`ea49a61715e94dfd559a5d360e928410e0116b82` and preserves the existing
OutputAuthority architecture. All focused, debug/release workspace, clippy and
warning-denied rustdoc gates pass.

The mandatory real-device read-only preflight opened COM5, returned
`channel_type = 3`, and produced four consecutive generation-1 Good temperature
samples at 28.0 degrees Celsius. Clean shutdown then failed twice with one unfinished
transport and `transports_closed = false`, including after an authoritative resource
idle observation. Both Runtime processes exited nonzero and released COM5.

No physical Metakon WRITE command was sent. No M9D acceptance archive was created.
Do not perform physical output testing, patch production against the connected
device, declare M9D ready for review, or begin M10 without a new explicit decision.

The bounded evidence and exact software gate results are in
`docs/implementation/MILESTONE_9D_REPORT.md`. Historical M8 and post-M9C evidence
remain immutable.
