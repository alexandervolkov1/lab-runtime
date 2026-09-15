# Milestone 6 implementation report

## Result and scope

M6 implements the fixed autonomous headless `virtual-demo` host, version-one
loopback NDJSON API, semantic event/snapshot recovery, and a thin Babashka client.
The real Babashka A/B process acceptance passed. M1's finite default executable,
the corrected M4 W1–W9/R1–R10 lifecycle, and M5 L1–L22 Lua boundary remain
accepted. `lab-core` still has no external dependencies. This is a virtual/software
implementation, not physical output or long-soak acceptance.

The implementation followed [the fixed M6 design](MILESTONE_6_DESIGN.md) in
tests-first slices: Core Reference/PID revisions; one-owner scheduler and safe
shutdown; bounded wire, sessions and nonblocking reactor; semantic events and
frozen replay barrier; real two-worker Lua integration; and actual-process bb A/B.
Each slice was committed separately. The reviewed incoming baseline was 123
tests; the final workspace lists 189 named Rust tests.

## H1–H28 acceptance map

Every row names passing test functions. Test files are relative to the repository
root; `server.rs` tests run through the `lab-runtime` library test target.

| ID | Passing acceptance evidence |
| --- | --- |
| H1 | Existing M1–M5 suites all pass; `cargo run -p lab-runtime` remains the finite five-measurement demo; std-only `lab-core/Cargo.toml`, fmt and warning-free clippy checks pass. |
| H2 | `host_scheduler::host_refreshes_measurement_reference_and_native_controller_without_clients`; `host_scheduler::actual_owner_thread_progresses_without_any_tcp_client_and_shuts_down_safely`. |
| H3 | `host_scheduler::scheduler_skips_missed_deadlines_and_fails_an_expired_native_gap_before_pid_replay`; Core M4 actual-dt tests remain passing. |
| H4 | `host_scheduler::safety_service_runs_once_per_due_slot_before_native_work`; `lua_network_isolation::two_confirmed_lua_worker_barriers_leave_real_socket_reads_and_native_renewal_alive` also observes independent M3 recovery under stalled workers. |
| H5 | `host_scheduler::repeated_or_unavailable_input_cannot_advance_pid_or_renew_lease`; `babashka_reconnect::actual_babashka_a_killed_after_running_and_b_reconciles_after_three_finite_leases`. |
| H6 | Core `reference_operations::live_retune_commits_at_the_old_ramp_value_without_a_jump` and `invalid_or_conflicting_retune_leaves_reference_state_and_revision_unchanged`; the real bb A task computes the old slope at retune time and checks continuity while Running. Existing M4 paused-Reference tests pass. |
| H7 | Core `controller_configuration::ready_configuration_resets_memory_and_keeps_bindings_without_starting_output`, `invalid_revisions_limits_and_active_state_reject_without_config_or_memory_change`, and `safe_paused_configuration_is_atomic_and_resets_the_earlier_pid_diagnostics`. |
| H8 | `api_owner_queries::descriptor_and_controller_configuration_report_exact_roles_units_and_limits`, `output_query_reports_rust_safe_readback_and_cannot_forge_authority`; `operation_outcomes::a_pure_query_does_not_refresh_or_advance_reference_without_the_scheduler`; `retune_outcome_reports_the_registered_reference_identity_instead_of_a_profile_constant`. |
| H9 | `api_protocol::fragmented_safe_frame_and_crlf_decode_to_the_same_hello_request`, `duplicate_keys_at_any_level_depth_and_value_budgets_are_rejected`, `oversize_utf8_schema_version_and_unadvertised_output_commands_fail_closed`; `client_isolation::fragmented_and_coalesced_frames_preserve_connection_order_and_hello_gate`, `partial_frame_trickler_closes_at_absolute_first_byte_deadline_while_healthy_peer_progresses`; `server::bounded_peer_tests::one_sweep_partial_write_resumes_at_the_exact_byte_offset`, `fixed_first_byte_and_unsent_write_deadlines_close_tricklers`, `a_complete_frame_at_the_exact_input_cap_is_dispatched_before_further_read`. |
| H10 | `client_isolation::incompatible_version_receives_bounded_error_then_only_that_socket_closes`, `fragmented_and_coalesced_frames_preserve_connection_order_and_hello_gate`; `api_protocol::every_version_one_operation_has_an_explicit_known_name_without_raw_core_commands`, `oversize_utf8_schema_version_and_unadvertised_output_commands_fail_closed`; `api_owner_queries::output_query_reports_rust_safe_readback_and_cannot_forge_authority`. |
| H11 | `operation_outcomes::start_is_accepted_then_completed_at_warming_without_claiming_running_or_physical_evidence`, `invalid_domain_config_fails_after_acceptance_and_its_revision_does_not_change`, `shutdown_is_accepted_first_and_terminal_only_after_safe_evidence_is_recorded`. |
| H12 | `request_deduplication::reconnecting_identical_request_returns_retained_terminal_outcome_without_execution`; changed target or expected revision conflicts in the same test. |
| H13 | `request_deduplication::evicted_and_ttl_expired_ids_return_unknown_and_can_never_mutate_again`, `pending_work_never_evicted_and_sequence_gaps_capacity_or_live_attachment_fail`. |
| H14 | `request_deduplication::pending_work_never_evicted_and_sequence_gaps_capacity_or_live_attachment_fail`, `scope_expiry_and_restart_identity_do_not_recreate_unknown_mutations`; `operation_outcomes::two_live_scopes_racing_on_one_revision_have_exactly_one_committed_retune`; `client_isolation::duplicate_msg_id_while_first_exchange_is_pending_is_rejected_without_second_dispatch`, `completed_exchange_releases_msg_id_for_later_use_on_the_same_socket`. |
| H15 | `runtime_events::pure_queries_emit_no_events_but_retune_publishes_one_ordered_causal_change`, `scheduled_observation_and_reference_progress_publish_at_monotonic_cursor`, `failed_latest_attempt_publishes_even_at_the_same_owner_publication_instant`; Core `signal::managed_provenance_tests::equal_time_failure_can_replace_good_but_cannot_restore_good`. |
| H16 | `subscription_recovery::retune_between_snapshot_and_subscribe_is_replayed_and_pages_remain_frozen`, `expired_snapshot_and_previous_boot_cursor_or_scope_require_explicit_resync`. |
| H17 | `subscription_recovery::ring_overrun_rejects_old_cursor_with_gap_and_filtered_scan_reports_progress`, `eviction_during_an_installed_replay_reports_gap_and_abandons_that_subscription`, `typed_target_filter_delivers_only_matching_events_and_advances_over_others`; `server::bounded_peer_tests::replay_gap_is_offered_then_affected_socket_detaches_without_affecting_owner`. |
| H18 | `server::bounded_peer_tests::nonreading_event_flood_detaches_and_stale_slot_frames_cannot_reach_reused_capacity`; `client_isolation::partial_frame_trickler_closes_at_absolute_first_byte_deadline_while_healthy_peer_progresses`, `client_disconnect_does_not_stop_native_owner_or_other_client_queries`; H20's real socket/native test covers worker pressure. |
| H19 | `server::bounded_peer_tests::full_owner_mailbox_cannot_drop_detach_or_reuse_its_connection_generation`, `nonreading_event_flood_detaches_and_stale_slot_frames_cannot_reach_reused_capacity`; `request_deduplication::pending_work_never_evicted_and_sequence_gaps_capacity_or_live_attachment_fail`; H16/H17 snapshot and ring capacity tests. |
| H20 | `lua_network_isolation::two_confirmed_lua_worker_barriers_leave_real_socket_reads_and_native_renewal_alive` blocks both fixed real Lua slots and observes independent native renewal, M3 timeout/recovery, and real TCP reads before release. |
| H21 | `lua_network_isolation::managed_dependent_controller_fails_safe_while_independent_native_controller_continues`; no replacement worker or forged managed freshness. |
| H22 | `runtime_startup::trusted_virtual_profile_is_safe_and_ready_without_auto_start`, `service_cli_rejects_unsafe_bind_or_unknown_profile_before_runtime_activation`, `startup_binds_ephemeral_loopback_only_after_safe_ready_profile_and_has_new_boot_id`, `occupied_loopback_bind_unwinds_real_lua_startup_without_publishing_readiness`; `lua_network_isolation::standard_profile_prepares_two_real_lua_components_before_readiness_and_schedules_steps`. |
| H23 | `runtime_shutdown::running_controller_stops_production_then_rust_confirms_safe_before_completion`, `warming_controller_has_no_lease_and_cannot_activate_after_shutdown_barrier`; `operation_outcomes::accepted_shutdown_barrier_rejects_other_clients_queued_start_before_any_output_lease`, `stopping_owner_refuses_new_hello_scope_and_reports_service_state_honestly`, `shutdown_is_accepted_first_and_terminal_only_after_safe_evidence_is_recorded`. |
| H24 | `runtime_shutdown::failed_m3_recovery_and_two_stalled_workers_expire_grace_with_unconfirmed_safe_evidence`, `service_shutdown_grace_stays_nonblocking_and_reports_stalled_cleanup`, `fatal_owner_clock_rejection_reaches_bounded_failed_shutdown_instead_of_abrupt_return`, `fatal_owner_reactor_path_offers_terminal_window_then_returns_nonzero`. |
| H25 | `subscription_recovery::expired_snapshot_and_previous_boot_cursor_or_scope_require_explicit_resync`; `runtime_startup::startup_binds_ephemeral_loopback_only_after_safe_ready_profile_and_has_new_boot_id`; `request_deduplication::scope_expiry_and_restart_identity_do_not_recreate_unknown_mutations`. |
| H26 | `babashka_reconnect::actual_babashka_a_killed_after_running_and_b_reconciles_after_three_finite_leases` runs the real binary and actual `bb` A/B processes; details below. |
| H27 | Actual Babashka `test-client` checks `malformed_utf8_event_is_rejected_without_replacement_decoding`, `replay_gap_requires_resync_instead_of_a_generic_stream_error`, `failed_terminal_outcome_advances_sequence_but_never_claims_completion`, `overflow_after_acceptance_exposes_original_uncertain_request_id`, `unknown_outcome_remains_epistemic_and_does_not_issue_a_replacement_command`, `changed_boot_in_event_requires_instance_reconciliation`, `frozen_snapshot_pages_cannot_accumulate_above_the_client_budget`, `reconnect_discards_already_applied_sequence_before_new_replay_event`; H26 exercises the same client against the real server. |
| H28 | Final debug and release `cargo test --workspace`, fmt check, warning-free workspace clippy, finite M1 run, `bb --version`, Babashka client task, standalone real-process acceptance and `git diff --check` all pass; scope/gate audit below. |

## Actual process and measured recovery

On this Windows workstation, `rustc` and `cargo` report 1.95.0 and `bb` reports
`babashka v1.13.220`. The standalone `babashka_reconnect` run exited in 7.29 s.
The harness launched `target/debug/lab-runtime.exe --serve --profile virtual-demo
--port 0`, consumed its bounded readiness JSON, and launched real `bb -m
lab.acceptance` A and B processes. A discovered descriptors by role/unit,
configured Ramp/PID with revisions, observed Warming then Running, checked the
continuous live Ramp retune and bounded PID output, took snapshot/pages and
subscribed. Rust killed A without Pause after its checkpoint. B resumed the same
boot/scope, reconciled the retained retune outcome, replayed from A's applied
cursor, observed Running and plant movement, paused to Rust-confirmed virtual
safe readback with no lease, observed Reference progress while Paused, and received
the safe/clean shutdown terminal before both processes exited successfully.

One passing run printed: A temperature 20.0749 °C, B temperature 30.6670 °C,
6,138 ms with no ordinary client, and a 6,305 ms advance in finite lease expiry.
The test also asserts the authority instance, owner and epoch remain identical;
the lease never becomes an unbounded client heartbeat. Its whole-test watchdog is
60 s, readiness deadline 3 s, process checkpoint deadlines 25 s, and actual
client-free interval must exceed 6.1 s (>3 full 2-s leases). Missing/unusable `bb`
fails the test; no skipped or Rust-fake path exists.

## Fixed bounds and evidence behavior

The service admits at most 8 live loopback peers, 16,384-byte UTF-8 NDJSON frames,
depth 16 and 1,024 total values/members. It holds at most 8 incoming requests,
8 replies and 16 event offers per peer; global owner/reactor mailboxes hold 64.
One owner turn handles at most 4 external requests with rotation. First-byte
handshake, incomplete frame and unsent-write deadlines are each absolute 2 s.
The reactor is nonblocking with 8-KiB per-peer read/write sweeps and a 5-ms wait.
It retains a bounded pending-detach queue under mailbox pressure and uses
monotonic connection generations so a stale frame cannot reach a reused slot.

The event ring retains 1,024 records of at most 4 KiB. A connection has at most
one 256-KiB frozen snapshot for 5 s, 8-KiB whole-record pages and one
subscription; replay scans at most 32 records and offers at most 4 matching
events per turn. Scope state has 16 scopes, 64 accepted nonterminal operations,
256 terminal results, 10-minute terminal TTL and 30-minute detached-scope TTL.
The Babashka client caps each frame at 16,384 bytes, its event buffer at 16,
frozen snapshot accumulation at 256 KiB, and an individual frame read at 2 s.

Safety precedes client and Lua work on a 10-ms lane; plant, Ramp and native PID
use 100-ms opportunities, Lua Source 200 ms and Transform fresh-upstream
admission. Missed deadlines coalesce from their prior phase. Native renewal is
private, finite and evidence-backed by successful trusted delivery; A's death
does not change the Running owner. The shutdown stop barrier blocks new hello,
Start and Lua admission. Ctrl-C only publishes an atomic stop flag for that owner.
Virtual safe evidence or a 2-s failed grace precedes
terminal recording and network closure; stalled worker cleanup gets a separate
200-ms bound. Fault-injected failed M3 recovery and two unfinished worker slots
produce an explicit unconfirmed-safe, nonzero result. Fatal owner faults likewise
retain virtual safe facts and return nonzero after the terminal network window.

## Tests-first observations and remaining limits

Meaningful red checks preceded their fixes. Examples captured during this run:
the nested-singleton JSON budget test returned `invalid_args` instead of
`json_values`; the version-mismatch socket closed without a bounded error; a
pipelined duplicate `msg_id` produced a second result; a full owner mailbox lost
Detach; a fatal owner clock error returned `InvalidTime` before grace; and the
registered Reference 2 retune reported identity 1. The real bb harness initially
failed without an actual client, then failed on B event-buffer pressure before
the bounded replay/reconciliation flow passed. Babashka client tests failed on
invalid UTF-8 replacement decoding, gap classification, accepted-command
uncertainty, overlarge snapshot accumulation and replay duplicates before their
fixes. All named checks pass in both final Rust profiles or the actual bb task.

The API is local coordination, not authentication or physical authority. Output
evidence is virtual Rust simulated readback; ACK is never promoted to readback.
The M3 failed-recovery case uses a trusted fake byte adapter, not a dangerous
physical actuator test. Lua deadlines fence acceptance and worker slots but do
not kill a stalled foreign instruction immediately. No Recorder/SQLite, GUI,
real serial/COM adapter, physical production deployment, multi-day soak or M7
design/implementation was added. The separate v1 donor remains read-only and is
not a dependency.

The user explicitly crossed ASTRA_HIGH -> SOL_HIGH for M6. There is no unresolved
architectural contradiction. This report is the M6 implementation checkpoint for
external review; it does not claim external approval. The pre-existing user-staged
`ai/project_snapshot.txt` remains untouched and outside every scoped M6 commit.
