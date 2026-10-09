# M18 closure: successful worker exit misclassified as panic

Date: 2026-10-09. Separate follow-up to the externally approved Recorder commit
`55bbf3006f05b4f8356b863c4c1261b5c4255f9b`; the admission/capacity architecture is
unchanged. This follow-up is submitted with consolidated M18 review, not claimed
to have its own external approval.

## Established invariant violation

The first Linux workspace Debug gate during commit assembly failed
`old_completed_job_ids_cannot_displace_a_live_cancellation`: normal shutdown
reported Failed instead of Closed. Its retained SQLite has a sealed boot,
complete coverage and shutdown evidence, with integrity OK.

`RecorderWorker::reconcile_receipt` read `alive` before
`JoinHandle::is_finished`. The worker can publish the final close receipt, store
`alive=false`, and return between those two observations. The owner then combines
the old `alive=true` with the new `finished=true`, calls `fail("storage worker
panicked")`, and loses the truthful terminal-seal status of a successful close.
The worker did not panic and SQL did not fail. This establishes the invariant
violation required by the user's Recorder freeze exception.

The deterministic regression
`successful_close_between_liveness_observations_is_not_a_worker_panic` holds the
real writer immediately before finish, releases it after the owner's liveness
sample, and waits for actual thread completion at that exact test-only seam.
Before the production change it fails with the same false panic. A coherent copy
of that RED database independently confirms `sealed`, integrity OK and no foreign
key violations. No test expectation was relaxed.

## Minimal change and unchanged contracts

In [reconcile_receipt](../apps/lab-runtime/src/recorder/worker/lifecycle.rs), sample
thread completion before loading liveness, with an Acquire fence between them.
The installed Rust 1.95.0 source implements `is_finished` using a relaxed load of
the thread packet's Arc count. The fence acquires the thread's released final
reference before reading `alive`; load ordering alone would not give that memory
visibility guarantee on a weakly ordered target. Both tested targets are x86_64.
A close during those observations can
defer completion detection to the next poll; it cannot combine an earlier alive
sample with a later completion sample. The test hook and held barrier are compiled
only for tests. The worker still publishes its final receipt and clears liveness
before normal return in [RecorderWorker::open_internal](../apps/lab-runtime/src/recorder/worker.rs).

There is no join, wait or added blocking operation in production polling. Genuine
panics still bypass the final liveness store and are detected as Failed. SQL close
errors remain failures. Reservation ownership, 13 groups / 1545 records / 4 MiB,
Reference admission, cumulative SQL receipts, FIFO, record/submission identities,
generation fences and credit-release code are unchanged. No successful SQL commit
is inferred from reservation or thread completion.

## Evidence

Root: `target/m18-consolidated-20261009/`.

- `linux-workspace-debug-before-close-fix.log` and `cancel-failure-sqlite.json`:
  original full-gate failure and coherent SQL evidence.
- `close-observation-before-fix.log`, `close-observation-before-fix.sqlite` and
  `close-observation-before-fix-sqlite.json`: deterministic RED and SQL inspection.
- `windows-close-{debug,release}.log`, `linux-close-{debug,release}.log`:
  14 worker regressions PASS in each profile/platform, including zero final
  credits, final seal/Closed, stale receipts, exact accounting and the new race.
- `*-close-integrations-{debug,release}.log`: 18 PASS per platform/profile for
  real writer panic, Required fail-closed, shutdown, flush and SQL close/commit
  failure suites.
- Final workspace gates and fresh TCP/WS matrix outcomes are recorded in
  `M18_CONSOLIDATED_REVIEW.md`; all prior matrix evidence remains intact.
