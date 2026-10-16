# Runtime agent rules

The current user request defines scope. This repository contains only the headless
`lab-runtime` application and platform-neutral `lab-core`. Read README.md,
ORIGIN.md and docs/extraction-review.md for provenance and actual qualification.
Historical reports and client/GUI milestones are not standing tasks.

## Ownership and contracts

- Runtime is the sole authoritative mutable owner of experiments, instruments,
  resources, measurements, controllers and recording. Client loss never cancels
  admitted Runtime work; presentation is client-owned.
- Maintain one language-neutral Application API. Queries return committed bounded
  snapshots; commands request mutations/work. TCP/WS share session, scope,
  sequencing, deduplication and recovery semantics. No client-language branches,
  scripting dependency or special client protocol. Optional external examples are
  never prerequisites for ordinary build, Rust tests or startup.
- Keep lab-core independent of OS serial, SQLite, wire encoding, deployment
  filesystems, scripting and presentation. Managed components are trusted
  compile-time Rust behind bounded invocation/result contracts; no transport or
  output authority is delegated to them.

## Safety and finite progress

- Route every physical side effect through OutputAuthority. Requested,
  authorized, first possible byte/send-start, ACK, readback and physical effect
  remain distinct. Recheck finite lease/epoch/generation immediately before bytes.
- Timeout after send-start is ambiguous. Never blindly retry ordinary or safe
  writes. Reconnect, restart, reload and fresh input never automatically rearm.
  Only explicitly virtual instruments accept virtual observations/evidence.
- Configuration follows parse, validation, cross-reference/safety validation,
  staging and Runtime-owned apply. Invalid candidates never partially publish.
  Preserve inert prepared topology, publication and reconnect fences.
- Use monotonic time for schedules, freshness, leases and deadlines; wall time
  is human-facing history. Never replay missed ticks.
- Every long-lived queue/worker has an owner, capacity, overflow, shutdown and
  failure policy. Required acquisition, control, safety and Recorder progress
  cannot wait indefinitely for clients, disk or managed components.

## Recorder, recovery and transport

- Recorder is Runtime-owned durable scientific/audit history. Diagnostics are
  bounded best-effort and non-authoritative. API, Recorder and SQLite schema
  are separate contracts. Required Recorder failures fail closed.
- Admission is not durability. Only validated post-COMMIT receipts advance the
  durable prefix and release submitted credit. Preserve pre-effect fences; thread
  exit or readable SQLite does not prove recorder_flushed.
- Mutation recovery is manual. Never automatically replay a mutation, status
  check or Exact Retry. Preserve exact uncertain evidence and quarantine old
  boot/scope identities. Reconnect/hello is not an authoritative rebuild.
- TCP is plaintext, unauthenticated and loopback by default. Trusted-LAN TCP
  requires explicit opt-in. Runtime WS remains loopback behind an authenticated
  WSS tunnel; preserve key, certificate and hostname checks. Client observation
  policy is not a server authorization role.

## Development

- One logical change per commit; preserve unrelated changes, releases and history.
- Rust comments/docs stay English. Preserve warning-denied gates and missing-docs
  enforcement. Explain non-obvious ownership, safety, time, bounds and failure.
- Use deterministic defect regressions and targeted checks. Run fmt and diff
  checks; reuse unchanged-source evidence for documentation-only changes.
- Keep scientific data separate from fixtures. Before deleting data verify an
  exact recoverable copy including WAL/SHM. Do not clean user caches or environments.
- Preserve pinned dependencies and exact-version licenses. New toolchains or
  dependencies require license/safety review. Package from explicit file lists;
  verify extracted files, license evidence, startup and graceful Recorder closure.
- Never infer physical qualification from simulation, readback or passing tests.

References: [architecture](docs/architecture.md), [safety](docs/safety-and-failures.md),
[API](docs/api/README.md), [configuration](docs/reference/configuration.md),
[Recorder](docs/recorder-sqlite.md), [recovery](docs/recovery-and-faults.md),
[extensions](docs/extending-runtime.md), [packaging](docs/packaging.md).
