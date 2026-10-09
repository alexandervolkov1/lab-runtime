# Agent rules

`lab-runtime` is an architecture-first, headless laboratory automation Runtime.
`v0.1.0-preview.4` is published. Read [project status](docs/developer/project-status.md)
for current capabilities, qualification limits and possible next directions.
The current user request defines task scope; historical reports are not standing
authorization or a reason to resume an old milestone.

## Ownership and contracts

- Runtime is the sole authoritative mutable experiment owner. Workbench and other
  clients own presentation, not experiment state. Client/GUI lifetime is not
  experiment lifetime; client loss does not cancel admitted Runtime work.
- Keep one language-neutral Application API: queries return committed snapshots;
  commands/operations request mutation or work. Transport adapters share its
  session, scope, sequencing, deduplication and recovery semantics.
- Workbench has one bounded Application worker and one presentation owner.
  External Workbench calls go through `WorkbenchDispatcher::dispatch()`;
  `client()` is for internal GUI/operator workflows, not an adapter bypass.
- Preserve the dependency direction: `lab-core` is independent of OS serial APIs,
  SQLite, wire encoding, deployment filesystems, scripting and presentation.
  Managed components are trusted compile-time Rust behind the bounded neutral
  invocation/result contract, without transport or output authority.

## Physical safety and bounded progress

- Every physical side effect goes through central `OutputAuthority`. Keep
  requested, authorized, send-start/first possible byte, ACK, readback and physical
  effect distinct. Recheck finite lease/epoch/generation authority immediately
  before the first possible output byte.
- Timeout after send-start is ambiguous: no blind retry of ordinary or safe
  writes. Reconnect, reload, fresh input and restart never imply automatic rearm.
  Only explicitly virtual instruments accept virtual observations; clients never
  fabricate physical measurements or safety/transport evidence.
- Configuration follows parse, validation, cross-reference/safety validation,
  staging and Runtime-owned apply. Invalid candidates do not partially publish.
  Prepared topology is physically inert until its accepted activation lifecycle;
  publication and participating-resource reconnect fences must remain intact.
- Use monotonic time for scheduling, freshness, control, leases and deadlines;
  wall time is for human-facing history. Do not replay missed scheduling ticks.
- Every long-lived queue/worker has explicit ownership, capacity, overflow,
  shutdown and failure behavior. Required acquisition, control, safety and Recorder
  progress must not wait indefinitely for clients, disk or managed components.
  This is an architectural priority, not an OS thread-priority guarantee.

## Recorder, recovery and transport security

- Recorder is Runtime-owned durable scientific/audit history; diagnostics are
  bounded, best-effort and non-authoritative. Application API, Recorder contract
  and SQLite schema are separate contracts.
- Required Recorder failures fail closed. Reservation/admission is not durability;
  only validated post-COMMIT receipts advance the prefix and release submitted
  credit. Do not infer `recorder_flushed` from thread completion or a readable DB.
  Preserve the documented finite capacity and pre-effect admission fences.
- Mutation recovery is manual. Never automatically replay a mutation, status
  request or Exact Retry. Preserve exact evidence and quarantine old boot/scope
  identities; do not turn uncertain outcomes into success or a new identity.
- Explicit Workbench Disconnect cancels reconnect. Unexpected loss permits only
  the bounded retained-scope episode. Fresh requires the authoritative rebuild
  barrier, not just a socket, hello, cursor or terminal mutation outcome.
- TCP is plaintext, unauthenticated and loopback by default; trusted-LAN access
  requires explicit opt-in. Runtime WS stays loopback behind an authenticated WSS
  tunnel. Preserve key, certificate and hostname validation. The separate local
  Workbench presentation API stays loopback-only; observation mode is client
  policy, not a Runtime authorization role.

## Repository and verification discipline

- Make one logical change per commit, preserve unrelated user changes, and do not
  rewrite history or modify published releases as a cleanup step.
- Keep Rust comments/docs in English, warning-denied builds and missing-docs
  enforcement. Explain non-obvious ownership, safety, time, bounds and failure.
- Use deterministic regression tests for defects and targeted checks for affected
  behavior. Run formatting and diff checks; reuse unchanged-source gate evidence
  rather than automatically repeating heavy builds for documentation-only edits.
- Keep examples and regression fixtures distinct from experimental data. Before
  removing scientific data, verify an exact recoverable copy including WAL/SHM.
  Never clean user caches, worktrees or environments without scoped authorization.
- Preserve exact-version third-party license evidence and release provenance.
  New toolchains/dependencies need a corresponding license/safety review.
- The `com_port_reader` donor is read-only, never a workspace dependency.

## Current references

- [Architecture](docs/architecture.md) and [safety/failures](docs/safety-and-failures.md)
- [Application API](docs/api/README.md) and [Workbench API](docs/workbench-api.md)
- [Recorder/SQLite](docs/recorder-sqlite.md) and [recovery](docs/recovery-and-faults.md)
- [Distributed deployment](docs/distributed-workbench.md)
- [Extension boundaries](docs/extending-runtime.md)
- [Release license evidence](docs/release-license-evidence.txt)
