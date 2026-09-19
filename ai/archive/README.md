# Internal engineering archive

This directory contains only historical material that remains useful for safety
rationale, regression investigation or accepted-evidence interpretation. Superseded
milestone diaries, POC plans, migration inventories and obsolete Lua/Babashka design
documents were removed from the current tree and remain available through Git
history.

## Design rationale

- `design/0001-runtime-ownership-and-domain-boundary.md` — origin of authoritative
  Runtime ownership and the client-lifetime boundary.
- `design/0002-central-output-authority.md` — origin of centralized physical-output
  authority and ambiguous-delivery handling.
- `design/RUNTIME_AND_SAFETY_MODEL.md` — early detailed safety/lifecycle rationale;
  historical terminology must not override current source or active coordination.

## Accepted milestone evidence

- `milestones/MILESTONE_8_REPORT.md` — deployment, reconnect and read-only hardware
  acceptance context.
- `milestones/POST_M9C_HARDWARE_SMOKE.md` — supplementary physical read evidence.
- `milestones/MILESTONE_9D_REPORT.md` — accepted physical-output architecture and
  evidence interpretation.
- `milestones/MILESTONE_11_REPORT.md` — final technical hardening and bounded-soak
  evidence.

## Audits

- `audits/MILESTONE_11_HARDENING_AUDIT.md` — failure-model audit that drove M11.
- `audits/MILESTONE_11_FAILURE_MATRIX.md` — engineering input for the future public
  safety/failure reference.

Archived documents are not current authorization. Use `../WORK.md`, `../HANDOFF.md`
and current source/tests for authoritative behavior.

The unchanged historical comment in `crates/lab-core/tests/milestone1.rs` cites the
former M1 design path. That superseded design was deleted from HEAD under the explicit
no-test-edit constraint; it remains available in Git history.
