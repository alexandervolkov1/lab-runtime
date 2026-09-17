# Current work — M8 external review gate

```text
STATUS: READY_FOR_EXTERNAL_REVIEW
Current model: SOL_HIGH
M9: NOT AUTHORIZED
```

This is the only current authorization.

## Completed evidence

- Final read-only COM5/Metakon run accepted as successful physical evidence.
- Successful immutable archive:
  `examples/metakon-513-com5-transient-open-retry-history.sqlite`, SHA-256
  `63ecb8575be5ac82ef968b929e8b3098b051a0f30a55c190b7fdc1dcaf64dc80`.
- One public reconnect reached generation 2 in the required Ready -> rebind -> probe
  -> durable lifecycle -> ordinary Good order.
- Babashka A-kill/B-resume, harmless live-safe revision, zero outputs, zero gaps and
  clean finite shutdown passed.
- The final release-only failure was diagnosed as a test-harness synchronization
  defect with first error `recorder ingress capacity exhausted`.
- Test synchronization now waits for the durable pre-reconnect receipt; production
  code and Recorder semantics are unchanged.
- Repeated focused release tests and all final debug/release/static/client/demo gates
  pass.

## Current restrictions

- Do not open COM5.
- Do not run more physical tests.
- Do not modify or reuse either preserved hardware archive.
- Do not start M9A or M9B.
- Do not perform unrelated release hardening, including the deferred `.gitattributes`
  LF pinning, during this gate.

## Next step

STOP for external M8 review. A later explicit review decision is required before any
new implementation milestone or hardware run.
