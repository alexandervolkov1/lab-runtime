# Current work — M8 physical reconnect review stop

STATUS: WAITING_FOR_REVIEW

Current model: SOL_HIGH.
Current phase: Recorder-corrected physical reconnect failed before generation-2 install.
M7: externally accepted at `f3ff456`.
M8 software implementation: complete at `0a42d73`; reviewed corrections complete
through Recorder FIFO correction commits `fdf8a73` and `2ec104b`.
M9: not authorized.

## Authoritative contract

Read before acting:

```text
ai/HANDOFF.md
ai/ROADMAP.md
ai/WORK.md
AGENTS.md
PROJECT_BRIEF.md
docs/implementation/RELEASE_PLAN_M7_TO_V0_1.md
docs/implementation/MILESTONE_8_DESIGN.md
docs/implementation/MILESTONE_8_REPORT.md
```

## Review checkpoint

The accepted physical run started at HEAD
`5355963d9e4bb3efa5f5214223a1510cc25619e9` with exact expected TOML and
definition hashes. Normal COM5 acquisition passed channel type 3 and produced
165 durable generation-1 Good temperature rows at raw/engineering 29--30 with
scale 1.0, matching the operator's 29-degree display.

The operator-reported device power-off produced exactly one value-less
Unavailable/Transport row, finite Offline, generation 1, queue zero and no later
Good. Required Recorder remained healthy and complete. After the
operator-reported device power-cycle/reset boundary, exactly one public
`reconnect_resource(resource=1, expected_binding_generation=1)` was issued. Its
Accepted and Failed/`invalid_configuration` phases are durable at records 677
and 678.

The Recorder FIFO correction passed this physical causal chain: there is no
`fact record reservation mismatch`, `recording_unavailable`, gap or unknown
tail. However, no generation-2 binding, compatibility measurement or lifecycle
activation was installed. Resource state remained Offline at generation 1.
Current evidence therefore places the failure before the replacement crossed
the rebind fence, without distinguishing old-adapter retirement failure from
configured COM5 reopen failure. Do not guess between those branches.

No reconnect retry, alternative COM open, register/address probe, configuration
operation or physical write was attempted. Normal shutdown completed with
transports closed, Recorder flushed, no unfinished transports/workers and success.
Final M8 Babashka-independence, live-safe reload and release/rustdoc gates were
not run after the reconnect failure.

Preserve the sealed evidence archive unchanged:

```text
examples/metakon-513-com5-recorder-corrected-history.sqlite
SHA-256 49c21ab48a22b4c2d0100357686f1b367d0432ce641dd9c8e21eee3184e4c6de
```

It has complete coverage, no gaps, zero output events and terminal checkpoint
through record 849. Preserve every earlier hardware archive as well.

Await external review. Do not reopen COM5, retry reconnect, change production
code, start M9 or perform post-M8 cleanup without explicit authorization.
