# M14.6B4 consolidated recovery/fault acceptance

Status: ready for final external re-review.

`STATUS: M14_6B4_PROCESS_CONTINUITY_READY_FOR_EXTERNAL_REVIEW`

Accepted B3 implementation: `0319b1d1a7917903c1cf0aa6e4852c149543f7fd`.

## Scope and non-goals

B4 turns the frozen M14.6A matrix into executable Workbench evidence. Production
changes are limited to three direct restorations of already accepted behavior:

- a zero-byte ordinary mutation retires its durable Pending precursor instead of
  becoming unresolved; a retirement failure preserves evidence and blocks mutation;
- a resnapshot invalidates the old catch-up cursor, so an `event_gap` cannot make a
  replacement subscription Fresh from the prior barrier;
- ordered-update overflow waits until its one deferred fail-closed notification is
  published before reattaching, preventing the authoritative replacement Hello from
  being lost behind the occupied deferred slot.

The journal save/retire fault selector is compiled only under `cfg(test)`. Process
helpers and self-spawn modes are test-only. No dependency, Cargo manifest, Runtime,
Application operation/DTO, deduplication/session rule, output/Recorder authority,
journal schema, PresentationDocument format, or public error taxonomy changed. B4
adds no Discard/Forget and no automatic mutation, status, or Exact Retry replay.

## Frozen 30-row executable ledger

Every row below names executable evidence; none relies on architecture alone.

| Row | Frozen fault | Exact automated evidence | Layer | Authoritative oracle and exercised bound | Result |
|---:|---|---|---|---|---|
| 1 | Runtime unavailable at initial Connect | `runtime_acceptance::b4_scenario_a_runtime_absent_then_manual_connect_reaches_fresh` | real process | Initial `scope:null` reaches TransportFailure/Disconnected with no loop; later manual Connect completes the full projection/fence/subscription Fresh barrier. | PASS |
| 2 | Unexpected disconnect while idle | `client::worker::tests::unexpected_eof_automatically_reattaches_the_runtime_advertised_scope`; `client::worker::tests::automatic_faults_and_hello_failures_never_restart_the_absolute_deadline` | scripted peer/unit | Exactly one retained-scope automatic episode; immutable 3 s deadline; live peer accepts the retained-scope Hello. | PASS |
| 3 | Disconnect during ordinary query | `client::worker::tests::transport_loss_clears_queries_status_and_retry_without_replay`; `gui::rebuild::tests::loss_at_three_rebuild_phases_restarts_one_complete_freshness_barrier` | unit | Lost query leaves no pending/outgoing replay; only the bounded post-Hello rebuild restores Fresh. | PASS |
| 4 | Mutation failure before wire | `client::worker::tests::mutation_wire_boundary_distinguishes_unsent_ambiguous_and_accepted`; `client::worker::tests::journal_failure_rejects_mutation_before_wire_emission` | unit/scripted peer | Worker byte marker remains false; zero peer bytes; Pending is durably retired, or retained only when retirement durability fails. | PASS |
| 5 | Mutation transmitted, admission unknown | `client::worker::tests::mutation_wire_boundary_distinguishes_unsent_ambiguous_and_accepted`; `client::worker::tests::transport_loss_clears_queries_status_and_retry_without_replay` | unit | First-byte marker converts exact Pending to Ambiguous; pending wire work is cleared and no automatic retry/status request appears. | PASS |
| 6 | Mutation Accepted then loss | `client::worker::tests::mutation_sequence_advances_only_after_accepted_and_disconnect_is_ambiguous`; `runtime_acceptance::real_runtime_exact_retry_retained_outcome_never_reexecutes` | scripted peer/real process | Accepted identity remains authoritative; retained Runtime dedup returns the original outcome and Recorder run id without second execution. | PASS |
| 7 | Ordered-update overflow | `client::worker::tests::ordered_update_overflow_runs_one_reattach_and_full_freshness_barrier`; `client::worker::tests::update_queue_pressure_closes_connection_and_publishes_resnapshot_state` | scripted peer | UPDATE_QUEUE=64 produces one deferred connection-lost resnapshot, one retained-scope episode, one replacement Hello, and one complete Fresh barrier. | PASS |
| 8 | `event_gap` | `client::worker::tests::public_error_and_event_gap_remain_structured_client_updates`; `gui::rebuild::tests::event_gap_rebuilds_same_connection_to_fresh_with_one_new_aggregate_subscription` | scripted peer/unit | `connection_lost=false`; connection stays Ready; old cursor is invalidated and exactly one new aggregate subscription catches up to Fresh. | PASS |
| 9 | Runtime boot change | `runtime_acceptance::b4_scenario_d_runtime_restart_quarantines_old_boot_and_builds_new_epoch`; `model::tests::boot_change_never_leaves_old_observations_fresh` | real process/unit | Boot A observations/draft stale; exact old evidence quarantined; boot B starts a clean barrier and default reference proves zero old mutation send. | PASS |
| 10 | Retained-scope reattach success | `runtime_acceptance::b4_scenario_b_explicit_disconnect_requires_manual_fresh_reconnect`; `client::worker::tests::successful_reattach_ends_episode_and_a_later_fault_gets_one_new_window` | real process/unit | Same scope/boot/next-seq Hello ends episode at Ready, while only the full rebuild changes Rebuilding to Fresh. | PASS |
| 11 | `scope_unknown` | `client::worker::tests::scope_in_use_reuses_the_current_episode_and_invalidation_ends_it`; `client::worker::tests::invalid_hello_quarantines_exact_records_without_changing_journal_bytes` | unit | Episode/retry cleared, exact bytes unchanged, active evidence moves to bounded quarantine, Disconnected requires manual Connect. | PASS |
| 12 | `instance_changed` | `client::worker::tests::scripted_instance_change_preserves_journal_and_emits_no_followup_request`; `client::worker::tests::scope_in_use_reuses_the_current_episode_and_invalidation_ends_it` | scripted peer/unit | Same fail-closed quarantine, zero follow-up status/retry/mutation, no second episode. | PASS |
| 13 | `scope_in_use` | `client::worker::tests::reattach_retries_scope_in_use_inside_one_absolute_deadline`; `client::worker::tests::reattach_connect_timeout_is_capped_by_one_absolute_deadline` | scripted peer/unit | Every retry reuses one 3 s Instant, spacing is at least 10 ms, and connect timeout is `min(2 s, remaining)`. | PASS |
| 14 | Journal missing | `runtime_acceptance::b4_scenario_a_runtime_absent_then_manual_connect_reaches_fresh`; `client::worker::tests::missing_journal_is_lazy_until_mutation_requires_durable_recovery` | real process/scripted peer | Missing path stays absent and warning-free through Hello/full Fresh/query; first durable mutation creates the exact journal before wire. | PASS |
| 15 | Journal corrupt | `runtime_acceptance::b4_scenario_j_corrupt_journal_allows_fresh_observation_but_blocks_mutation`; `recovery::tests::corrupt_truncated_wrong_root_and_versions_are_rejected` | real process/unit | Real bounded parser reports a visible problem; bytes remain identical; observations become Fresh while mutation/status/retry authority stays blocked. | PASS |
| 16 | Journal read failure | `client::worker::tests::journal_read_failure_is_fail_closed_without_fabricated_empty_state` | unit/real filesystem | Reading a directory through the production bounded read path deterministically returns I/O failure; source remains a directory and no default journal is fabricated. | PASS |
| 17 | Journal write/durability failure | `client::worker::tests::journal_failure_rejects_mutation_before_wire_emission`; `client::worker::tests::injected_post_admission_and_retirement_failures_preserve_exact_evidence` | scripted peer/test seam | Pre-wire save failure emits zero bytes; post-admission save retains Accepted in memory/prior Pending on disk; retirement failure leaves terminal evidence byte-identical and blocks sequencing. | PASS |
| 18 | Clean Workbench close | `client::worker::tests::worker_shutdown_is_finite_and_sends_no_runtime_shutdown_operation`; `run-gui-smoke.ps1` | scripted peer/native GUI | Shutdown completes inside 3 s and emits no Runtime/controller/Recorder operation; real Runtime remains alive after native close. | PASS |
| 19 | Forced Workbench termination | `runtime_acceptance::b4_scenario_e_process_crash_preserves_exact_journal_without_auto_send`; `run-gui-smoke.ps1` | self-spawn/native GUI | Actual OS child kill is bounded; Runtime survives; same workspace is reacquired; journal/reference persist; controller identity/state/revision and Recorder state/run identity match authoritative pre-kill observations. | PASS |
| 20 | Restart with retained recovery | `runtime_acceptance::b4_scenario_e_process_crash_preserves_exact_journal_without_auto_send`; `client::worker::tests::loaded_journal_only_requests_reconciliation_and_never_auto_sends` | self-spawn/scripted peer | Exact op/args/request identity survives process death; restart opens no socket before manual Connect and emits no status/retry/mutation after Hello. | PASS |
| 21 | Runtime restarts while Workbench survives | `runtime_acceptance::b4_scenario_d_runtime_restart_quarantines_old_boot_and_builds_new_epoch` | real process | Runtime A is killed and B starts on the same port; old episode terminates/quarantines; draft and Exact Retry are invalid; B reaches Fresh with zero old mutation effect. | PASS |
| 22 | Loss during projection rebuild | `gui::rebuild::tests::loss_at_three_rebuild_phases_restarts_one_complete_freshness_barrier`; `model::tests::disconnect_retains_cached_plot_but_gap_and_boot_change_start_new_live_epochs` | unit | Loss after discovery, after detail issue, and after subscription-before-catch-up never yields Fresh; each new Hello restarts a finite barrier and live epochs do not bridge. | PASS |
| 23 | Status → Completed | `client::worker::tests::operation_status_outcomes_preserve_authority_and_never_fabricate_observations`; `runtime_acceptance::real_runtime_reference_reconnect_reconcile_and_replay` | unit/real process | One status request changes only exact recovery to Completed; presentation values change only through Runtime query/event evidence. | PASS |
| 24 | Status → Failed | `client::worker::tests::operation_status_outcomes_preserve_authority_and_never_fabricate_observations`; `model::recovery_status::tests::status_reply_never_mutates_admission_without_recovery_state` | unit | Failed becomes authoritative only after worker RecoveryProjection; observational/physical projection remains unchanged. | PASS |
| 25 | Status → `outcome_unknown` | `client::worker::tests::operation_status_outcomes_preserve_authority_and_never_fabricate_observations`; `model::recovery_status::tests::outcome_unknown_is_visible_and_admission_is_unchanged` | unit | Ambiguous evidence remains unresolved; no Completed/Failed downgrade or mutation send. | PASS |
| 26 | Manual Exact Retry | `model::exact_retry::tests::prepare_captures_exact_record_and_confirm_submits_only_identity`; `client::worker::tests::retry_uses_only_worker_owned_exact_payload_and_rejects_untracked_identity`; both `real_runtime_exact_retry_*_never_reexecutes` tests | unit/scripted peer/real process | Confirmation sends zero; Confirm sends one identity; worker supplies immutable op/args/request_id; retained and evicted Runtime outcomes never execute twice. | PASS |
| 27 | Retry after boot/scope invalidation | `client::worker::tests::retry_worker_rejects_terminal_quarantine_and_journal_failure_before_wire`; `model::exact_retry::tests::any_prepared_record_or_session_change_is_stale_and_sends_nothing` | unit | Quarantine/session mismatch refuses locally with zero outgoing bytes and unchanged bounded evidence. | PASS |
| 28 | Capacity pressure | `client::contract_tests::command_mailbox_rejects_the_thirty_third_item_without_blocking`; `client::worker::tests::disconnect_fence_bypasses_turn_limit_for_automatic_and_manual_episodes`; `client::worker::tests::ninth_in_flight_request_is_rejected_before_socket_write`; `client::worker::tests::full_recovery_capacity_rejects_before_wire_and_reopens_after_reconciliation`; `model::tests::quarantine_is_bounded_separate_evidence_and_never_reconciliation_authority`; `client::worker::tests::bootstrap_event_pressure_emits_one_resnapshot_at_sixty_four`; `gui::app::tests::gui_drain_is_bounded_to_the_worker_queue_capacity`; `gui::rebuild::tests::operator_rebuild_queries_bounded_optional_domains_before_one_aggregate_subscription`; `gui::rebuild::tests::discovery_identity_pressure_fails_at_sixty_four_without_followup_growth`; `model::tests::operator_action_history_is_absolutely_bounded_and_never_evicts_active_work`; `model::tests::live_display_window_evicts_only_oldest_display_points`; `recovery::tests::journal_enforces_eight_records_and_sixty_four_kibibytes`; `client::worker::tests::reattach_connect_timeout_is_capped_by_one_absolute_deadline`; `client::worker::tests::typed_faults_start_one_episode_with_exact_deadline_and_retry_spacing`; `client::worker::tests::worker_shutdown_is_finite_and_sends_no_runtime_shutdown_operation` | unit | Command 32, commands/turn 8, in-flight 8, active recovery 8, quarantine 8, ordered updates 64, GUI drain 64, aggregate subscription 1, bootstrap 64, operator actions 64, display points 4096, rebuild identities 64, journal 64 KiB, connect 2 s, episode 3 s, spacing >=10 ms, request 5 s, and shutdown 3 s all exercise their reject, resnapshot, fail-closed, eviction, or finite-time rule rather than only constants. | PASS |
| 29 | Malformed/oversized protocol | `client::framing::tests::lf_crlf_partial_and_exact_body_bound_are_compatible`; `client::framing::tests::oversized_body_and_incomplete_deadline_fail_locally`; `client::framing::tests::malformed_utf8_json_and_oversized_stream_are_rejected`; `client::framing::tests::partial_write_identity_is_retained_until_all_bytes_are_accepted`; `client::worker::tests::semantic_protocol_faults_never_reach_projections_and_start_one_episode`; `client::worker::tests::malformed_server_frame_and_partial_timeout_are_connection_local` | unit/scripted peer | Exact 16,383-body/16,384-frame acceptance, extra byte/UTF-8/JSON/envelope/msg-id/cursor/partial deadline rejection; blocked-write timer retains byte identity; eligible faults own one episode and never reach projections. | PASS |
| 30 | Explicit Disconnect vs unexpected loss | `client::worker::tests::explicit_disconnect_from_ready_opens_no_socket_beyond_the_fault_window`; `client::worker::tests::unexpected_eof_automatically_reattaches_the_runtime_advertised_scope`; `client::worker::tests::disconnect_under_update_pressure_finishes_without_reconnect_or_authority_loss`; `client::worker::tests::transport_loss_clears_queries_status_and_retry_without_replay` | scripted peer/unit | Explicit Disconnect is coalesced/out-of-band with zero AR; unexpected loss owns one bounded AR; both paths emit zero automatic mutation/status/Exact Retry. | PASS |

## A-J scenario ledger

| Scenario | Exact automated oracle | Result |
|---|---|---|
| A | `runtime_acceptance::b4_scenario_a_runtime_absent_then_manual_connect_reaches_fresh` | PASS — failed initial attempt terminates; later manual Connect reaches Fresh. |
| B | `runtime_acceptance::b4_scenario_b_explicit_disconnect_requires_manual_fresh_reconnect` | PASS — no automatic activity after explicit Disconnect; Runtime stays alive; manual retained-scope Connect reaches Fresh. |
| C | `apps/lab-workbench/run-gui-smoke.ps1` | PASS — actual GUI process kill, Runtime survival, workspace reacquisition, usable journal, preserved Reference authority, controller `1`/`ready`/revision `1` before and after, Recorder `idle` with null `active_run`/`run_id` before and after, and post-kill full Fresh rebuild. |
| D | `runtime_acceptance::b4_scenario_d_runtime_restart_quarantines_old_boot_and_builds_new_epoch` | PASS — Runtime A/B process restart on one endpoint, quarantine, invalid draft/retry, clean boot-B epoch, zero old mutation effect. |
| E | `runtime_acceptance::b4_scenario_e_process_crash_preserves_exact_journal_without_auto_send` | PASS — self-spawned Workbench test process is killed after mutation bytes; exact journal survives; restart is silent until manual Hello/reconciliation. |
| F | `runtime_acceptance::real_runtime_reference_reconnect_reconcile_and_replay`; `model::recovery_status::tests::exact_identity_is_sent_once_and_duplicate_is_rejected` | PASS — one manual Check Status produces one request and terminal authoritative recovery without mutation replay. |
| G | `runtime_acceptance::real_runtime_exact_retry_retained_outcome_never_reexecutes`; `runtime_acceptance::real_runtime_exact_retry_outcome_unknown_never_reexecutes` | PASS — exact retained identity/payload is used; retained dedup and evicted-outcome paths execute no duplicate. |
| H | `gui::rebuild::tests::event_gap_rebuilds_same_connection_to_fresh_with_one_new_aggregate_subscription`; `client::worker::tests::public_error_and_event_gap_remain_structured_client_updates` | PASS — same connection, one replacement aggregate subscription, new catch-up, Fresh. |
| I | `client::worker::tests::ordered_update_overflow_runs_one_reattach_and_full_freshness_barrier` | PASS — model becomes non-Ready, one deferred resnapshot/AR, retained scope, complete replacement barrier. |
| J | `runtime_acceptance::b4_scenario_j_corrupt_journal_allows_fresh_observation_but_blocks_mutation` | PASS — corrupt bytes unchanged, warning visible, real Runtime observations Fresh, mutation/status/retry authority blocked. |

## Process and fault-injection methodology

Real `lab-runtime` processes cover A, B, D, J and the four previously accepted
Runtime tests: eight real-Runtime process acceptances in total. Runtime readiness is
a bounded JSON stdout predicate. Runtime restart
D reuses the exact endpoint; child exit is polled with `try_wait` under a five-second
deadline. Every RAII cleanup first kills and then polls only to that deadline.

Scenario E uses the current Rust test executable as a test-only Workbench helper.
The parent owns the listening peer, observes the exact mutation frame as its readiness
predicate, forcibly terminates the child, waits with a finite deadline, parses the
real journal, then starts a new client against a quiet scripted peer. There is no
production CLI mode or extra executable.

Scripted peers are used where a real Runtime cannot externally expose the required
byte/socket boundary: EOF before reply, partial/invalid frames, update pressure,
scope errors, and precise no-follow-up-wire assertions. They parse and emit the real
bounded NDJSON/Application envelopes and drive the real client/model/rebuild owners.

The journal test seam is a `cfg(test)` one-shot enum with only `SaveNext` and
`RetireNext`. It changes no production filesystem path. Read failure, corrupt input,
oversize input, missing input, and pre-wire write failure still use the real bounded
filesystem/parser paths. Every fault test asserts both memory and disk state plus
whether mutation sequencing is blocked.

## Mutation, status, retry, Freshness, and pressure evidence

The consolidated byte-boundary test freezes all three mutation cases: zero bytes
retires Pending, the first possible byte produces exact Ambiguous evidence, and an
Accepted reply remains Accepted across loss. No socket condition is terminal
operation evidence.

Check Status tests cover one request, Accepted nonterminal, Completed, Failed,
`outcome_unknown`, and transport interruption without resend. Exact Retry tests cover
confirmation-without-send, one identity on Confirm, immutable worker-owned payload,
retained Runtime dedup, evicted `outcome_unknown`, and boot/scope/quarantine refusal.

Fresh requires discovery/current/detail/fence/one aggregate subscription/catch-up.
The three-phase loss regression and event-gap regression prove that no partial or old
cursor barrier can satisfy it. The overflow regression proves that the single
deferred notification is delivered before reattach can publish the replacement
Hello.

Capacity evidence covers command mailbox 32, commands/turn 8, in-flight 8, active
recovery 8, quarantine 8, update queue and GUI drain 64, aggregate subscription 1,
bootstrap events 64, operator actions 64, live points 4096, rebuild identities 64,
journal 64 KiB, JSON body 16,383, frame 16,384, connect timeout 2 s, absolute
reattach 3 s, retry spacing >=10 ms, request timeout 5 s, and worker shutdown 3 s.
The named tests exercise each overflow/failure behavior; no diagnostic or reconnect
history was added.

## Native GUI evidence

`run-gui-smoke.ps1 -Profile debug` uses native Glow and real processes. It proves a
native window, full Fresh rebuild, confirmation, Accepted, Completed, authoritative
refresh, explicit Disconnect stale controls, manual Fresh reattach, clean close,
actual OS kill, Runtime survival, workspace mutex release, recovery-journal reuse,
authoritative reference revision continuity, and a third post-kill full rebuild.
The killed Workbench records authoritative controller identity, lifecycle state, and
revision plus Recorder state, `active_run`, and `run_id`. The replacement Workbench
records the same fields after Fresh, and the harness compares each field before
reporting `controller_continuity_proven` or `recorder_continuity_proven`. The
`virtual-demo` uses an existing best-effort Recorder database in this smoke and the
Recorder is `idle`; therefore the accepted Recorder oracle is the honest weaker
invariant: authoritative `idle` before and after, with null `active_run` and
`run_id`. The observed controller is identity `1`, lifecycle state `ready`, revision
`1` before and after. Neither continuity result is inferred from Fresh or Runtime
liveness.

## Reliability notes

No sleep is a correctness barrier. Socket/process loops use state predicates and
absolute deadlines; short yields avoid busy-spinning. During B4 construction, the
self-spawn peer initially inherited nonblocking mode from its listener and produced
Windows `WouldBlock`; the accepted socket is now explicitly returned to blocking mode
with a finite read timeout. The first overflow fixture also omitted the required
`reference` operation and encoded subscription progress in the wrong envelope shape;
both deterministic fixture errors were corrected before final repetition. No test
was rerun merely to conceal an intermittent failure.

## Verification record

Final commands and counts:

```text
cargo fmt --all -- --check: PASS
cargo test -p lab-workbench --locked, three consecutive runs:
    160 passed; 9 ignored; 0 failed (each run)
cargo test --workspace --locked:
    680 passed; 11 ignored; 0 failed
cargo test --workspace --release --locked:
    680 passed; 11 ignored; 0 failed
cargo clippy --workspace --all-targets --locked -- -D warnings: PASS
real Runtime process acceptances A, B, D, J plus four existing: 8 passed; 0 failed
self-spawn Workbench plus scripted peer acceptance E: 1 passed; 0 failed
A-J acceptance group: PASS
native Glow GUI smoke: PASS
focused race/fault repetitions: 10/10 for each recorded target
git diff --check: PASS
```

The nine ignored Workbench tests comprise eight real-Runtime process acceptances
(A, B, D, J and the four existing tests) and one self-spawn Workbench plus scripted
peer acceptance (E). They remain ignored so an ordinary
unit run does not assume a prebuilt `lab-runtime` sibling or spawn/kill external
processes; the verification gate executes every one explicitly. The two existing
non-Workbench ignored diagnostic-rotation and preview-soak tests are unchanged.

## End state

M14.6B3 is accepted. M14.6B4 is ready for final external re-review. M14 consolidated
acceptance is not yet granted. M13.2 Steel remains blocked/not authorized.
