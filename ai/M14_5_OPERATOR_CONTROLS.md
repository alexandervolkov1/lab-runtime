# M14.5 — Workbench operator controls and property/configuration workflows

## Status

```text
M14.4: ACCEPTED

M14.5 operator controls + properties/config:
READY FOR FINAL EXTERNAL RE-REVIEW

M14.6: NOT AUTHORIZED

STATUS: M14_5_BOUNDED_ACTIONS_READY_FOR_EXTERNAL_REVIEW
```

The accepted M14.4 implementation commit is
`116aba631fe47ea24412dd3d4b50e9b00eafc8df`; its coordination acceptance commit is
`e0212119de4c70e475f8889a56a2138e87e0eb01`.

M14.5 adds client-side operator intent, confirmation, and observation workflows. It
does not add or change a Runtime operation, DTO, public error, session/dedup rule,
queue capacity, controller/Recorder/output-safety semantic, dependency, transport,
or presentation persistence format.

## Ownership and command boundary

`model/operator.rs` is the one renderer-neutral semantic boundary used by the GUI:

```text
GUI draft
    -> typed OperatorIntent
    -> fresh-observation/readiness validation
    -> PreparedOperatorIntent confirmation
    -> exact existing Application op + args
    -> ONE M14.2 Application client worker
    -> Runtime
```

Widgets never call `ClientHandle::mutation` or construct arbitrary operation names.
The only call from the operator layer to that worker method is the
`MutationSubmitter` implementation. A future Steel `lab/*` adapter can reuse this
same typed boundary without acquiring a second operation registry or session owner.

The client-side layer owns drafts, confirmation text, local validation, and the
visible mutation lifecycle. The M14.2 worker remains the only owner of the socket,
`msg_id`, scope, `request_id.seq`, durable recovery journal, exact retry payload,
subscription, and reconnect. Runtime remains the only experiment-state owner.

## Exact M14.5 operation surface

The encoder emits only this reviewed set, and only if the name is advertised by the
current authoritative `hello.operations`:

```text
reference_configure
reference_retune

controller_start
controller_pause
controller_resume
controller_reset_failed
controller_configure_pid

property_configure
reconnect_resource

recording_start
recording_stop
```

There is no GUI path for `runtime_shutdown`, raw output/register writes,
`emulator_publish`, `virtual_models_restart`, history operations, experiment
annotation, deployment staging/apply/reload, or the full controller policy editor.
A source search confirms that ordinary GUI code contains none of those operation
names and no direct mutation call.

## Readiness and confirmation

Before a draft can be prepared and again immediately before wire submission, the
operator layer requires:

- worker connection `Ready`;
- overall projections `Fresh`;
- the exact target observation `Fresh`;
- operation advertised by the current hello;
- no recovery-journal problem;
- no mutation requiring reconciliation;
- no Pending/Accepted/Ambiguous worker recovery record;
- no other unacknowledged operator workflow.

Every mutation enters `AwaitingConfirmation`. The confirmation contains the Runtime
boot, stable target, frozen revision/generation/run identity, and important proposed
values. Warnings are bounded and truthful: property warnings display the
authoritative `mutation_class`, while PID, resource-reconnect, and Recorder-stop
warnings are neutral operation-specific notices. They are not described as Runtime
metadata when no such metadata exists, and never imply ACK, physical effect, safe
state, or durability.

The GUI derives confirmation currentness every frame. If the boot, target freshness,
or guarded value changes, it visibly reports `Draft stale / authoritative state
changed`, disables Confirm, and retains only Cancel/review behavior. It never waits
for a click to discover staleness and never rebases a draft silently.

`CommandSendError::Busy` leaves the workflow awaiting confirmation. It does not
create a submitted or pending Runtime claim.

## Frozen optimistic-concurrency identities

The prepared draft retains the exact current `boot_id` plus a guard against the
fresh projection used to build it:

| Workflow | Frozen authoritative field |
|---|---|
| Reference configure/retune | `revision` |
| PID configure | controller `revision` |
| Property configure | property/configuration `revision` |
| Resource reconnect | `binding_generation` |
| Recording stop | complete `active_run` identity |
| Controller lifecycle | observed lifecycle state |
| Recording start | observed Recorder state |

Confirm rechecks the boot, target freshness, and guard. A Runtime boot change always
invalidates the draft even if the new instance happens to reuse the same numeric
revision/state/generation. If the boot or observed field changes while confirmation
is open, submission fails locally as a stale draft; it is never silently rebased.
`revision_conflict` from Runtime becomes a terminal failed workflow and never causes
automatic merge or retry.

## Reference and controller workflows

Reference forms support fixed configuration, ramp configuration, and ramp retune.
The client rejects empty/malformed/nonfinite numeric text and nonpositive ramp rate,
but does not invent laboratory bounds. The displayed Reference projection changes
only through an Application query/event, not from the form value or terminal reply.

Controller lifecycle buttons are selected by the observed state as a UX gate:
Ready/Start, Warming-or-Running/Pause, Paused/Resume, and Failed/Reset. Runtime may
still reject for safety, input/output, or Recorder reasons. Button state is never
translated into output-safety evidence or authority.

The PID editor freezes `expected_revision` and the five reviewed fields `kp`, `ki`,
`kd`, `output_min`, and `output_max`. It validates finite numbers and
`output_min <= output_max`; all deeper policy/lifecycle validation remains Runtime
owned. Completion does not restart or resume a controller.

Controller authority is split explicitly. A full `controller` query owns lifecycle,
revision, bindings, and config/PID/policy detail. A narrow controller event owns only
the accepted `controller_json()` fields: `state`, `status`, `failure`, `active`,
`paused`, `revision`, `last_tick`, and `latest_output`. It does not use synthetic
`tick`/`output` aliases. A same-revision event merges those dynamic fields while
preserving query-owned `bindings` and `config` as fresh full detail. A changed
revision leaves the event-owned lifecycle fresh but marks cached full detail stale,
disables PID preparation, and schedules at most one bounded full-detail query per
controller. The refreshed query alone can make PID detail fresh again; neither an
event nor a terminal mutation result synthesizes full controller detail. Realistic
Ready -> Running -> Paused -> Failed regressions verify that status, failure, active,
and paused do not remain contradictory.

## Property editor

Property forms are built from the authoritative property record: stable owner and
property identity, `value_type`, `current`, `access`, constraints, `mutation_class`,
unit, and revision. Number, integer, boolean, and text observations may be rendered,
but the accepted v1 `property_configure` mutation surface is deliberately narrower:
only `Integer(i64)` and `Text(String)` candidates can be encoded.

Only `access:"read_write"` integer/text records can produce `property_configure`.
Read-only and deployment-only records remain visible but disabled. Number, boolean,
and enum-shaped observations cannot produce mutation candidates even if a malformed
or future projection marks them writable. M14.5 does not widen the Runtime DTO.
Generic text editing is bounded to 512 UTF-8 bytes.

No arbitrary-JSON editor exists. A configuration event stales property observations.
The one rebuild coordinator owns `refresh_in_flight` and `dirty` state: the first
event starts one bounded `configuration_properties` refresh, further events while it
is active coalesce into one additional refresh, and completion settles only after
that catch-up refresh. A configuration event during the main subscription catch-up
therefore cannot make property controls fresh prematurely. No second subscription is
installed.

## Resource and Recorder workflows

Reconnect is available only for a fresh resource whose projection says it is
reconnect-capable and when `reconnect_resource` is advertised. Confirmation retains
the exact binding generation. There is no automatic reconnect retry and no claim of
transport success before terminal/projection evidence.

Resource authority is likewise split explicitly. A full `resource` query owns
`binding_generation`, `transport_generation`, capabilities, configuration revision,
identity, and deployment metadata. A partial resource event updates only `state`,
`queue_len`, `active`, `latest`, and maps its `generation` fact to the cached
transport-generation observation; it never maps that fact to `binding_generation`.
The query-owned reconnect metadata is preserved. Matching transport generation keeps
full detail fresh. A changed generation keeps the partial transport fact fresh but
stales reconnect detail, disables Reconnect, and schedules one coalesced bounded
`resource` query when current hello advertises it. Repeated events for that generation
do not duplicate the query. Only a full response restores reconnect authority and
supplies the next `expected_binding_generation`; without advertised `resource`, the
partial fact remains visible but no refresh request or authority claim is made.

This rule is enforced twice for defense in depth, without duplicating authority.
The GUI disables its Reconnect control while full detail is stale, and the reusable
renderer-neutral `OperatorIntent::ReconnectResource` preparation path independently
requires `resource_detail_is_fresh(resource)`. It performs that check before reading
preserved capabilities or `binding_generation`, never derives binding generation from
an event's transport `generation`, and never triggers refresh itself. Therefore GUI,
future Steel `lab/*`, and any later Workbench adapter share the same semantic guard;
the rebuild coordinator remains the sole bounded refresh owner.

Prepared detail-dependent intents retain the exact version of their renderer-neutral
`AuthorityGuard`. `ObservationOnly` is sufficient for ordinary projections;
`ResourceDetail` captures the query-owned transport generation for reconnect drafts,
and `ControllerDetail` captures the query-owned revision for PID drafts.
`PreparedOperatorIntent::is_current()` requires this detail authority to remain Fresh
and at the captured version, in addition to same boot, Fresh entity, and unchanged
ordinary guarded value. Thus a later refresh at another controller revision or
transport generation cannot revive the old draft even when another guarded field,
such as `binding_generation`, happens to remain unchanged. `confirm()` returns
`DraftStale`, sends nothing, does not rebase the draft, and does not start a query.
An identical-version refresh is not claimed to invalidate a draft. The operator must
cancel/review and prepare a new draft whenever the authority version changes.

Recorder Start accepts a nonempty label bounded to the accepted 128 bytes and
requires an authoritative idle Recorder projection. Stop copies the complete
authoritative `active_run` object; it never derives a local run counter. Submitted,
Accepted, Completed, and current Recorder state are rendered separately. Workbench
exit still only shuts down the client worker and never synthesizes recording stop.

## Mutation lifecycle and reconciliation

The one GUI workflow is explicitly:

```text
Idle
  -> AwaitingConfirmation
  -> Submitted
  -> Accepted
  -> Completed | Failed

or

Submitted/Accepted -> Ambiguous (reconciliation required)
```

Terminal workflows must be acknowledged before another GUI workflow starts. A
terminal completion may precede its projection event, so the UI says that displayed
Runtime state is still awaiting an authoritative observation. Failure retains the
bounded public envelope. Ambiguous state displays the retained identity when known;
it never invents or edits a retry payload and never auto-retries on reconnect.

## Rebuild and subscription extension

The accepted M14.4 serialized rebuild now performs, when the operations are
advertised:

1. frozen discovery pages;
2. frozen current-measurement pages;
3. up to 64 Reference queries;
4. up to 64 controller queries;
5. up to 64 resource queries;
6. one Recorder status query;
7. frozen configuration-property pages (Runtime maximum 256 records, 64/page);
8. a second discovery cursor fence;
9. one aggregate subscription and catch-up to that fence.

Controller/resource/Recorder/configuration domains are optional. Their detail query
is emitted only when the exact query operation is advertised by the current hello;
configuration paging is likewise attempted only when `configuration_page` exists.
An unsupported optional domain remains unavailable/stale without sending an
unsupported request or making signal and Reference observation unusable. Global
Fresh is declared only after the explicit snapshot/subscription cursor barrier and
any configuration refresh required by catch-up has settled.

There remains exactly one Runtime subscription with six kinds:

```text
signal, reference, controller, resource, recorder, configuration
```

Signal, Reference, controller, resource, and Recorder events normalize centrally in
`WorkbenchModel`. A configuration event invalidates fresh property observations and
requests the bounded property refresh described above. Event gap, overflow,
disconnect, boot change, and live observation epochs retain the accepted M14.4
behavior.

## Local bounds

M14.5 adds no queue or network capacity. Existing bounds remain commands 32,
in-flight exchanges/recovery records 8, ordered updates 64, one subscription, and
4,096 live plot points per signal. Operator action display history is now explicitly
bounded to `MAX_OPERATOR_ACTIONS = 64`. Additional form text bounds are:

```text
numeric edit text:       64 bytes
generic property text:  512 bytes
recording label:         128 bytes
```

## Deterministic tests

Focused tests prove:

- Stale/Rebuilding/missing target and absent hello operation disable preparation;
- recovery failure or unresolved mutation blocks new sequencing;
- Reference revision capture, boot-bound stale-draft rejection, visible disabled
  stale confirmation, and no optimistic projection;
- exact controller stable identity and state gating across Runtime boots;
- same-revision controller events preserve full PID detail, while changed-revision
  events stale it and coalesce one full-detail refresh;
- controller events use the real eight-field `controller_json()` lifecycle schema,
  including Running -> Paused -> Failed consistency;
- a PID editor can never combine revision 8 with stale revision-7 fields;
- finite/range-ordered PID validation;
- v1 property mutation is restricted to writable integer/text records; number,
  boolean, enum, deployment-only, and read-only records cannot encode a mutation;
- property `mutation_class` and neutral operation-specific warning semantics;
- same-generation resource events preserve binding/capability detail, changed
  transport generation disables reconnect and coalesces one full query, and the
  refreshed binding generation is used exactly;
- renderer-neutral reconnect preparation rejects stale full detail before submission;
  its draft captures transport generation 9, remains stale after a generation-10
  refresh with binding generation still 4, and only a new draft emits exact binding 4;
- the GUI Reconnect predicate is false during that same stale-detail interval;
- resource partial events without an advertised `resource` query issue no refresh;
- exact Recorder run capture;
- Submitted, Accepted, and Completed remain distinct;
- local Busy creates no submission claim;
- public revision conflict produces no retry and no fabricated observation;
- transport ambiguity preserves the exact prepared payload;
- finite optional-domain rebuild ordering and one six-kind subscription;
- configuration events coalesce across an in-flight refresh, including subscription
  catch-up, and create no second subscription;
- optional resource/Recorder/configuration/controller detail requests honor the
  current hello operation set;
- Recorder/configuration events normalize centrally;
- all accepted M14.4 overflow, reconnect, rebuild, and live-epoch tests remain.
- 256 ordinary query replies update their projections without creating actions;
- Reference bootstrap and untracked local rejection create no action history, while
  preserving their projection and visible client error respectively;
- only `track_operator_intent` creates an action, tracked Accepted/Completed/Failed
  transitions still work, and ordinary `Result` never alters a tracked action;
- more than 64 terminal workflows retire the oldest terminal entries while retaining
  the newest, and 64 nonterminal entries reject a new intent before wire submission;
- 32 repeated hello/rebuild/resnapshot cycles leave action count at zero.

## Real Runtime acceptance

The opt-in test
`real_runtime_operator_layer_confirms_completes_refreshes_and_never_retries_conflict`
passed against a real `lab-runtime --profile virtual-demo` process. It proved:

- full Workbench rebuild reached Fresh;
- a fresh Reference draft entered confirmation;
- the typed operator layer submitted one retune;
- Runtime Accepted and Completed were separately consumed;
- only the authoritative Reference event changed the displayed target/revision;
- a deliberately stale second draft reached Runtime and received
  `revision_conflict`;
- no automatic retry occurred and the cached observation was not rewritten;
- disconnect left Runtime alive.

The existing M14.2 process acceptance remains unchanged and opt-in.

## Native Glow GUI smoke

`apps/lab-workbench/run-gui-smoke.ps1` still runs a real native Glow Workbench and
now drives the same `OperatorWorkflow`. The latest observed result was:

```json
{"status":"pass","renderer":"glow","runtime_pid":12684,"runtime_port":55697,"native_window_observed":true,"minimize_restore_observed":true,"maximum_working_set_bytes":88838144,"idle_cpu_milliseconds_over_2s":0.0,"idle_working_set_start_bytes":87986176,"idle_working_set_end_bytes":87990272,"initial_fresh":true,"stale_after_disconnect":true,"fresh_after_reattach":true,"live_signal_points":2,"reference_visible":true,"confirmation_observed":true,"mutation_accepted_observed":true,"mutation_completed_observed":true,"authoritative_refresh_observed":true,"stale_controls_disabled":true,"controls_reenabled_after_fresh":true,"runtime_alive_after_clean_close":true,"runtime_alive_after_forced_termination":true}
```

The smoke checks those observed operator lifecycle fields rather than merely printing
them. The former constant `operator_controls_rendered` claim was removed. It also
retains native window, minimize/restore, bounded idle resource observation, clean
close, forced termination, and independent Runtime-lifetime evidence.

## External-review findings and remediation

External review accepted the typed operator boundary, confirmation model, lifecycle,
readiness gate, serialized workflow, Reference vertical slice, and one-subscription
architecture, then identified six focused authority/projection defects. The final
implementation corrects them without changing Runtime/Application semantics:

1. narrow controller events no longer replace full controller detail;
2. every prepared intent is bound to the Runtime boot and visibly becomes stale
   before submission;
3. configuration refreshes remember and coalesce events that arrive in flight, and
   catch-up cannot publish stale properties as fresh;
4. `property_configure` candidates match the existing v1 integer/text wire surface;
5. confirmation warnings distinguish authoritative property mutation classes from
   neutral operation-specific notices;
6. all optional projection queries are gated by the authoritative hello operation
   list.

The remediation added deterministic regressions for every item, including the exact
PID-A/revision-7 versus PID-B/revision-8 hazard. No dependency, Cargo file, Runtime
source, Application operation/DTO, capacity, subscription, persistence format, or
accepted M14.4 behavior changed.

External re-review accepted those items and the controller full-detail authority
concept, then found two remaining schema/partial-projection defects. The second
remediation replaces the synthetic controller event aliases with the exact public
schema and introduces symmetric full-versus-partial resource authority. Focused
regressions cover same- and changed-generation merges, refresh coalescing, reconnect
disablement, refreshed binding generation, and the unsupported-query case. Recorder
handling was deliberately not broadened.

The final re-review then required explicit proof that the typed operator boundary,
not only the GUI widget, rejects stale resource detail. The final regression drives
the shared `OperatorWorkflow`: a generation-10 partial event over generation-9 full
detail makes `begin(ReconnectResource)` return `TargetNotFresh` with an empty mutation
submitter; after a generation-10 full query with binding generation 5, confirmation
submits exactly `expected_binding_generation:"5"`. A separate GUI predicate test
retains the disabled-during-refresh evidence.

The next final finding was confirm-time detail authority. The regression opens a
valid generation-9/binding-4 confirmation before the generation-10 event, proves the
entity stays Fresh and binding 4 stays preserved while resource detail becomes Stale,
observes `PreparedOperatorIntent::is_current() == false`, and verifies
`confirm() -> DraftStale` with an empty submitter. PID confirmation uses the symmetric
controller-detail authority guard.

Final re-review then identified that freshness alone did not bind a draft to the
exact query-owned authority version. The acceptance-critical regression deliberately
refreshes full resource detail at transport generation 10 while leaving binding
generation unchanged at 4. The old generation-9 draft remains stale and two confirm
attempts send nothing; a newly reviewed draft captures transport generation 10 and
submits exactly `expected_binding_generation:"4"`. The PID regression explicitly
proves capture of controller detail revision 7, invalidation by revision 8, and capture
of revision 8 only by a newly prepared draft. No refresh is initiated by preparation
or confirmation.

The final bounded-bookkeeping review found that ordinary query, Reference bootstrap,
and local rejection updates polluted an unbounded operator action map. The model now
maintains a matching bounded insertion order and at most 64 action entries.
`track_operator_intent` is the only creation path. On insertion at capacity it retires
the oldest `Completed` or `Failed` entry; it never retires `PendingAdmission`,
`Accepted`, or `Unknown`. If every slot is nonterminal, the typed workflow preflight
returns `ActionCapacityFull` before mutation submission and the model insertion path
also fails visibly without losing an active entry. Mutation replies and local
rejections update only an already tracked command; ordinary `Result` replies and
Reference bootstrap remain purely observational.

The accompanying boundedness sweep found no other monotonic M14.5 bookkeeping
collection. Recovery records remain capped at 8; ordered worker updates and per-frame
drain remain 64; the aggregate subscription has cardinality one; controller/resource
detail-refresh maps and rebuild identity queues have explicit 64-identity caps;
configuration refresh uses two finite flags; optional operations and observational
entity/detail maps are bounded by the authoritative hello/registry and Runtime domain
cardinalities and replace values by stable identity; live samples replace oldest at
4,096 points per server-bounded signal; and `OperatorWorkflow` has one current
workflow. Presentation-owned collections retain their accepted M14.3 validation
bounds.

## Dependencies and scope

M14.5 adds no third-party dependency and does not enable wgpu, eframe persistence,
RON, async, Steel, or a second transport. `Cargo.toml` and `Cargo.lock` are unchanged
from accepted M14.4.

M14.6 remains responsible for consolidated reconnect/recovery/fault acceptance and
is not authorized. Steel remains blocked and unauthorized.

## Verification

The completed tree passed:

```text
cargo fmt --all -- --check
cargo test --workspace --locked
cargo test --workspace --release --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test -p lab-workbench --locked                 (three consecutive runs)
cargo build -p lab-runtime -p lab-workbench --locked
cargo test -p lab-workbench --locked runtime_acceptance:: -- --ignored --nocapture
apps/lab-workbench/run-gui-smoke.ps1
git diff --check
```

Each focused Workbench run reported 89 passed and the two process acceptances
ignored by the ordinary suite. The explicit process command then ran both ignored
acceptances successfully. The two pre-existing opt-in Runtime soak/rotation tests
remain ignored and unchanged.

During the full workspace gate, the queue-saturation regression exposed a test-peer
race: the peer could panic on the expected TCP reset after the worker deliberately
closed an overloaded connection. The test peer now accepts EOF/reset while emitting
surplus events. No production behavior changed; the focused pressure test passed ten
consecutive times before the complete verification above.

```text
M14.4: ACCEPTED

M14.5 operator controls + properties/config:
READY FOR FINAL EXTERNAL RE-REVIEW

M14.6: NOT AUTHORIZED

STATUS: M14_5_BOUNDED_ACTIONS_READY_FOR_EXTERNAL_REVIEW
```
