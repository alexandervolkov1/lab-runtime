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

M9D is the only active milestone. Its software implementation preserves the existing
OutputAuthority architecture. The read-only shutdown correction is committed at
`b6e1840` and all focused, debug/release workspace, clippy and warning-denied rustdoc
gates pass.

The shutdown failure was an owner-lifecycle defect: `ServiceHost` froze the first
nonblocking transport-retirement `Pending` result as terminal merely because Recorder
was already flushed. There was one resource, one executor and one COM worker; the
read-only deployment created no output lifecycle. A deterministic regression test
failed before the fix. The corrected bounded lifecycle passed a real read-only COM5
run with `channel_type = 3`, four Good temperature samples, zero unfinished
transports, clean shutdown and process exit 0.

No physical Metakon WRITE command was sent. No M9D acceptance archive was created.
The remaining gate is the separately authorized physical write acceptance. Do not
perform it, declare M9D ready for review, or begin M10 without a new explicit
decision.

The bounded evidence and exact software gate results are in
`docs/implementation/MILESTONE_9D_REPORT.md`. Historical M8 and post-M9C evidence
remain immutable.
