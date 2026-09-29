# M14.6B2A — recovery quarantine projection and restart classification

## Status and scope

```text
M14.6B1: ACCEPTED

M14.6B2A quarantine projection / restart classification:
READY FOR FINAL EXTERNAL RE-REVIEW

M14.6B2B Exact Retry UI: NOT AUTHORIZED
M14.6B3 automatic fault reattach: NOT AUTHORIZED
M14.6B4 consolidated fault acceptance: NOT AUTHORIZED
M13.2 Steel: BLOCKED / NOT AUTHORIZED

STATUS: M14_6B2A_HANDOFF_READY_FOR_EXTERNAL_REVIEW
```

The accepted B1 implementation is
`1a5c69965fa2ad3907c0314cb61cc3e29798e5ea`; its acceptance and B2A authorization
are recorded by `070975fe48bf0f107f4a86f23c008933c5fb6dd0`.

M14.6B2A changes only the private Workbench client, model, GUI presentation, tests,
and coordination evidence. It adds no dependency, Runtime/Application operation or
DTO, persistence version, automatic reconnect, status polling, Exact Retry UI,
Discard/Forget, or Steel.

## Active and quarantined ownership

The single Application client worker remains the sole recovery owner. It now holds
two independent bounded projections:

```text
active recovery       <= 8 RecoveryRecord
quarantined recovery  <= 8 QuarantinedRecoveryRecord
```

Active records may be reconciled only against the currently attached boot and
scope. A quarantined record preserves the exact old `boot_id`, `(scope, seq)`,
operation, normalized arguments, and known admission, plus one typed reason:

- `InstanceChanged`;
- `ScopeUnknown`;
- `AttachedBootMismatch`;
- `AttachedScopeMismatch`.

`ClientUpdate::RecoveryProjection` is one private atomic Workbench update containing
both bounded collections. It is not an Application DTO and is not stored in
`PresentationDocument`.

The model validates both eight-record bounds and rejects duplicate
boot/scope/sequence identities across the complete candidate before transactionally
replacing either collection. Its
`reconciliation_required` list is still derived only from active records.

## Classification rules

`instance_changed` and `scope_unknown` hello errors move the exact active records
into quarantine, publish one atomic active/quarantine projection, clear current
retry/status authority, and retain the old desired evidence. A successful hello
with a different boot or scope performs the equivalent classification while allowing
observational attachment and rebuild to continue.

A same-boot, same-scope hello leaves records active and quarantine empty. Repeated
classification is identity-deduplicated. Once quarantined in a process, evidence is
never reactivated by a later coincidentally matching hello.

The worker's `operation_status` path now requires an exact active record attached to
the current hello. A quarantined identity is therefore rejected locally before wire
emission even if a non-GUI caller bypasses the B1 renderer-neutral eligibility
layer.

## Journal preservation and restart

Classification never calls journal save, replacement, or retirement. It does not
rewrite boot/scope, manufacture a mutation identity, or serialize the quarantine
reason. While quarantine exists, a later successful new-scope hello also skips the
ordinary active-journal reconciliation write. Mutation admission remains blocked,
so no new record can overwrite the old evidence under the unchanged v1 journal
format.

Tests snapshot the complete journal bytes before classification and compare them
after:

- `instance_changed`;
- `scope_unknown`;
- successful attached-boot mismatch;
- successful attached-scope mismatch;
- manual new-scope attachment after quarantine;
- three restart/classification cycles with the maximum eight records.

All comparisons are exact byte equality. On restart the unchanged v1 journal loads
as candidate recovery evidence and is classified again after authoritative hello;
no new persistence schema is needed.

Valid session quarantine does not set `RecoveryJournalProblem`. Corrupt, unreadable,
or unwritable journal state continues through the distinct existing journal-failure
projection. Both conditions block mutations, but the GUI and model do not conflate
them.

## Mutation and status gating

Any non-empty quarantine blocks new mutation admission inside the worker before
wire emission. The renderer-neutral typed operator readiness boundary independently
requires an empty quarantine, and thin GUI enablement mirrors that rule. Queries and
normal observational rebuild remain available.

The manual GUI connection action is labelled `Connect new scope` while quarantine
is visible and submits `Connect { scope: None }`. This is operator initiated, not
automatic reattach. A new-scope hello may restore observational `Fresh`, but the
quarantine remains and mutation controls remain fail-closed.

B1 Check Status enumerates only the active side of `RecoveryProjection`. When
classification removes active recovery, tracker rows disappear; quarantined evidence
is displayed separately without Check Status or Retry controls.

## External-review handoff remediation

The final B2A remediation makes recovery ownership disposition indivisible at the
model boundary. Every worker recovery change publishes exactly one
`RecoveryProjection { active, quarantined }`; there is no empty-active update that
can outrun a later quarantine update.

The exact successful mismatched-hello order is:

```text
State(Ready)
Hello
RecoveryProjection { active: [], quarantined: [old evidence] }
```

Ready and Hello do not modify the model's prior active recovery projection, so the
old evidence remains visible until the atomic replacement makes quarantine visible.
The exact `instance_changed` / `scope_unknown` order is:

```text
RecoveryProjection { active: [], quarantined: [old evidence] }
Reply(PublicError)
State(Disconnected)
```

Tests apply each emitted update individually and assert after every prefix that the
old record is active, quarantined, or covered by an explicit recovery problem.

If the bounded update queue is full at classification, the atomic projection is not
partially published. The model retains the old active evidence; the worker retains
the quarantine; and the single deferred `ordered_update_queue_full`
`ResnapshotRequired { connection_lost: true }` moves connection state to Stale.
Thus pressure cannot expose empty active plus empty quarantine. No second status,
retry, or mutation request is emitted. Once the atomic projection is delivered, the
B1 tracker immediately removes the active row, treats the identity as missing from
status authority, and sends zero `operation_status` requests.

## GUI semantics

The existing Recovery / reconciliation section retains B1 active rows and adds a
visibly separate `Quarantined recovery evidence` subsection. Each row shows only:

- old boot;
- old scope and sequence;
- operation;
- known admission;
- typed quarantine reason.

Arguments are not editable or rendered as a retry candidate. The UI states that
the record is not attached to the current Runtime session, status/retry are
unavailable, and new experiment mutations remain blocked. It never calls the old
mutation failed, completed, applied, safe, recorded, durable, or resolved.

## Deterministic evidence

Focused tests prove:

- same boot/scope retains active authority and an empty quarantine;
- both hello-invalid codes preserve exact record content and journal bytes;
- successful boot/scope mismatches quarantine rather than erase evidence;
- active and quarantine collections remain independently bounded at eight;
- repeated hello and restart classification does not duplicate or grow entries;
- a manual new-scope rebuild can become observationally Fresh while quarantine and
  mutation blocking remain;
- status and mutation attempts against quarantine emit zero network requests;
- active B1 rows disappear while the separate quarantine projection remains;
- successful boot/scope mismatch and both hello-error paths are prefix-safe when
  their exact ordered updates are applied one at a time;
- update-queue saturation preserves the old active evidence until the fail-closed
  transport-loss resnapshot is visible;
- atomic disposition disables Check Status for the quarantined identity and emits
  zero `operation_status` requests;
- reconciliation identities never include quarantined records;
- valid quarantine and journal I/O failure remain distinct;
- scripted-peer `instance_changed` produces no follow-up request and preserves the
  journal byte for byte.

The focused Workbench suite passed three consecutive debug runs, each with
`110 passed; 0 failed; 2 ignored`. The two existing opt-in real Runtime acceptances
were then run explicitly and passed (`2 passed; 0 failed`).

The workspace declares 634 tests. Both locked debug and release workspace gates ran
630 enabled tests successfully and left the same four opt-in tests ignored; the two
Workbench process acceptances among those four were executed separately as noted
above. The pre-existing rotation and soak workloads remain unchanged and ignored.

## Verification

```text
cargo fmt --all -- --check                                      PASS
cargo test --workspace --locked                                PASS
cargo test --workspace --release --locked                      PASS
cargo clippy --workspace --all-targets --locked -- -D warnings PASS
cargo test -p lab-workbench --locked (three consecutive runs)  PASS, 110/0/2 each
real Runtime Workbench process acceptances                      PASS, 2/0
native Glow GUI smoke                                           PASS
git diff --check                                                PASS
```

The Glow smoke retained the native window, Glow renderer, Fresh/Stale/rebuild
lifecycle, live plot, Reference observation, real mutation lifecycle, clean and
forced Workbench exits, and Runtime survival evidence.

No Cargo manifest/lockfile, Runtime/core source, public API, recovery journal schema,
or accepted capacity changed.

## Non-goals retained

M14.6B2A does not implement Exact Retry UI or wiring, automatic status queries,
automatic transport-fault reattach, automatic mutation replay, quarantine clearing
or reactivation, Discard/Forget, a new persistence format, M14.6B2B–B4, or Steel.
