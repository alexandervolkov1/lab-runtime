# Public errors and limits

Applies to Application protocol v1 and the v0.1 product.

This is the canonical public home for Application error taxonomy and bounds. For a
running Runtime, values returned in `hello.result.limits` are authoritative for that
build.

## Error envelope

A synchronous rejection has `type:"error"`, `accepted:false`, and:

| Field | Meaning |
|---|---|
| `code` | stable specific public code |
| `category` | one of 12 broad machine-readable classes |
| `message` | fixed bounded text with no untrusted/internal error echo |
| `retryable` | unchanged request might make sense later; not permission to retry a mutation blindly |
| `resync_required` | incremental state is invalid and current state must be rebuilt |
| `details` | optional trusted structured details within the advertised bound |

A failed admitted mutation uses the same fields inside `type:"operation",
state:"failed"`. In that context the request identity is terminal, even if the same
code is also possible as a synchronous pre-admission rejection.

## Categories

| Category | Meaning |
|---|---|
| `invalid_request` | envelope, IDs, arguments, sequencing, or strict schema invalid |
| `unsupported_operation` | operation/mode unavailable in this composition |
| `invalid_configuration` | invalid domain configuration or lifecycle transition |
| `revision_conflict` | caller state/archive/cursor revision conflicts; resnapshot |
| `not_found` | object or bounded retained token/result absent |
| `unavailable` | required input/service/lifecycle temporarily unavailable |
| `transport_unavailable` | required physical transport unavailable |
| `recording_unavailable` | required Recorder unavailable or failed |
| `timeout` | bounded operation deadline expired |
| `capacity_exhausted` | a fixed queue/store/service capacity is full |
| `operation_failed` | fail-closed bounded fallback for non-public internal failure |
| `protocol_error` | framing/version/encoding or incremental-state failure |

## Exact public codes

`Retry?` reproduces the source `retryable` bit; the aliased `operation_failed` row
points to its separate current-behavior table. The bit never authorizes automatic
mutation replay. “Identity” describes mutation evidence when relevant. Clients must
read `category`, `retryable`, and `resync_required` from the actual envelope rather
than infer them from `code` alone.

| Code | Category | When it occurs | Retry? / resync | Mutation identity implication |
|---|---|---|---|---|
| `invalid_args` | invalid_request | arguments fail strict operation schema | no / no | synchronous reply means not admitted |
| `invalid_id` | invalid_request | malformed/bounded ID or canonical counter | no / no | not admitted |
| `missing_request_id` | invalid_request | mutation lacks exact identity | no / no | not admitted |
| `invalid_shape` | invalid_request | envelope/root/field type invalid | no / no | not admitted |
| `unknown_field` | invalid_request | strict object contains unknown field | no / no | not admitted |
| `duplicate_key` | invalid_request | JSON repeats an object key | no / no | not admitted |
| `json_values` | invalid_request | lexical value/member bound exceeded | no / no | not admitted |
| `string_too_large` | invalid_request | string/key exceeds UTF-8 byte bound | no / no | not admitted |
| `invalid_number` | invalid_request | nonfinite or invalid numeric form | no / no | not admitted |
| `invalid_cursor` | invalid_request | cursor is malformed or in the future | no / no | no mutation admission |
| `duplicate_msg_id` | invalid_request | correlation ID already in flight on connection | no / no | mutation admission for rejected exchange did not occur |
| `already_hello` | invalid_request | hello repeated on attached connection | no / no | no admission |
| `hello_required` | invalid_request | non-hello request before successful hello | no / no | no admission |
| `scope_unknown` | invalid_request | scope absent/expired/evicted or wrong for request | no / no | old client evidence may remain unresolved; no new authority |
| `scope_in_use` | invalid_request | retained scope attached to another connection | no / no | unchanged retained evidence; no admission |
| `sequence_gap` | invalid_request | sequence is above authoritative next sequence | no / no | not admitted; do not guess missing IDs |
| `request_conflict` | invalid_request | retained ID used with different typed payload | no / no | retained original identity remains authoritative |
| `outcome_unknown` | invalid_request | mutation sequence is at/below high-water with no retained record | no / no | unresolved outcome; old identity will not execute again |
| `unsupported_operation` | unsupported_operation | operation not registered/advertised here | no / no | not admitted |
| `unsupported_history_mode` | unsupported_operation | history mode unsupported | no / no | if synchronous, not admitted |
| `invalid_configuration` | invalid_configuration | configuration/policy violates domain rules | no / no | terminal if emitted after admission; otherwise not admitted |
| `invalid_state` | invalid_configuration | lifecycle transition invalid now | no / no | terminal if operation failure |
| `output_rejected` | invalid_configuration | output authority/safety rejects request | no / no | terminal outcome is not physical-state proof |
| `domain_rejected` | invalid_configuration | domain validation rejects request | no / no | terminal if operation failure |
| `revision_conflict` | revision_conflict | expected revision differs from authority | no / yes | terminal/rejected exact identity; rebuild state |
| `history_archive_mismatch` | revision_conflict | archive selection conflicts with token/cursor | no / yes | no new physical/mutation authority |
| `history_cursor_mismatch` | revision_conflict | continuation cursor filter/scope mismatch | no / yes | history operation identity follows returned outcome |
| `unknown_controller` | not_found | controller ID absent | no / no | synchronous query none; mutation may terminal-fail |
| `unknown_reference` | not_found | Reference ID absent | no / no | same |
| `unknown_actuator` | not_found | output identity absent | no / no | same |
| `unknown_resource` | not_found | resource ID absent | no / no | same |
| `unknown_instrument` | not_found | instrument ID absent | no / no | same |
| `unknown_parameter` | not_found | parameter ID absent | no / no | same |
| `unknown_signal` | not_found | signal identity absent | no / no | same |
| `history_database_unknown` | not_found | archive database identity absent | no / no | history operation may terminal-fail |
| `history_page_expired` | not_found | retained page token absent/expired | no / no | no mutation replay authority |
| `history_cursor_expired` | not_found | retained history cursor expired | no / no | issue a new selection, not an old-ID replay |
| `snapshot_expired` | not_found | frozen projection/token expired or mismatched | no / no | restart snapshot query |
| `input_unavailable` | unavailable | required controller/input service unavailable | yes / no | retry bit is not automatic mutation permission |
| `stale_input` | unavailable | required input exceeds freshness policy | yes / no | same |
| `client_disconnected` | unavailable | admitted work cannot continue for client-scoped service | yes / no | if operation failure, identity is terminal |
| `shutdown_in_progress` | unavailable | Runtime already stopping | yes / no | rejected exchange not admitted |
| `shutdown_before_execution` | unavailable | admitted queued work fenced by shutdown | yes / no | retained failed identity is terminal |
| `transport_unavailable` | transport_unavailable | required physical resource transport absent | yes / no | never blind-retry ambiguous output |
| `recording_unavailable` | recording_unavailable | required Recorder service unavailable | yes / no | operation may terminal-fail |
| `recorder_disabled` | recording_unavailable | requested Recorder work while unavailable | yes / no | operation may terminal-fail |
| `recording_failed` | recording_unavailable | Recorder entered failure | yes / no | retained evidence governs admitted identity |
| `history_timeout` | timeout | bounded history job deadline expired | yes / no | terminal if admitted history operation |
| `timeout` | timeout | other bounded operation deadline expired | yes / no | timeout after possible send is not automatic-retry authority |
| `busy` | capacity_exhausted | bounded session/operation/resource capacity full | yes / no | pre-admission capacity leaves next sequence unchanged |
| `history_busy` | capacity_exhausted | connection already holds a pending job/retained page, or bounded history capacity is full | yes / no | an admitted `history_read` terminates failed with this code |
| `subscription_busy` | capacity_exhausted | connection already has subscription | yes / no | no mutation identity |
| `snapshot_capacity` | capacity_exhausted | one projection record cannot fit page bound | yes / no | no mutation identity |
| `history_page_oversize` | capacity_exhausted | history page cannot fit bounded result | yes / no | admitted history identity may terminal-fail |
| `history_token_exhausted` | capacity_exhausted | bounded history token/cursor store full | yes / no | admitted history identity may terminal-fail |
| `instance_changed` | protocol_error | scope/cursor belongs to another Runtime boot | no / yes | old-boot evidence remains external/quarantined, never reassigned |
| `event_gap` | protocol_error | requested event continuity fell outside replay | no / yes | rebuild observations; mutation evidence is separate |
| `frame_too_large` | protocol_error | JSON body/NDJSON frame exceeds byte bound | no / no | malformed exchange not admitted |
| `incomplete_frame` | protocol_error | NDJSON frame lacks terminal LF | no / no | not admitted |
| `invalid_utf8` | protocol_error | Application body reaching the shared codec is not UTF-8 (for example TCP NDJSON; WebSocket may reject invalid text earlier) | no / no | not admitted |
| `invalid_json` | protocol_error | malformed/trailing JSON | no / no | not admitted |
| `json_depth` | protocol_error | nesting exceeds bound | no / no | not admitted |
| `version_mismatch` | protocol_error | `v` is not supported | no / no | not admitted |
| `encode_failed` | protocol_error | bounded response cannot be encoded | no / no | rely on retained identity, not socket inference |
| `response_too_large` | protocol_error | semantic result exceeds frame limit | no / no | operation context governs retained identity |
| `event_error` | protocol_error | bounded event/cursor processing failed | no / no | rebuild observations if continuity is uncertain |
| `internal_error` | protocol_error | allowlisted internal protocol fallback | no / no | do not infer success |
| `operation_failed` | varies; see below | fail-closed public alias/fallback for unexposed internal code | varies / no | terminal if in failed operation; otherwise no admission claim |

The current source maps any non-allowlisted internal code to
`operation_failed`; raw OS, SQLite, or internal text is not exposed. Two internal
configuration conditions currently also canonicalize to that wire code without
becoming additional public codes:

| Wire code | Actual category | `retryable` | `resync_required` | Current source situation |
|---|---|---:|---:|---|
| `operation_failed` | `operation_failed` | false | false | generic non-public fallback |
| `operation_failed` | `unsupported_operation` | false | false | internal `configuration_disabled` alias |
| `operation_failed` | `capacity_exhausted` | true | false | internal `configuration_capacity` alias |

This is current source behavior, not three public code names. In particular,
`configuration_disabled` and `configuration_capacity` are not published as wire
codes. A client must inspect the envelope fields; `code:"operation_failed"` alone
does not determine category or retryability. The public table therefore lists wire
codes, not every internal source string.

The public error `code:"outcome_unknown"` is specific to mutation submission: the
submitted sequence is at or below the scope high-water mark and has no retained
record, so the old identity will not execute again. It is not the same wire shape as
the normal `operation_status` result `{"state":"outcome_unknown"}`. That query result
only says Runtime currently has no retained state evidence for the exact identity and
does not establish prior admission or compare against high-water. Neither form is a
terminal Completed/Failed outcome, physical-state proof, or blind-retry permission.

## Runtime limits advertised by hello

| Field or derived bound | Current value | Meaning |
|---|---:|---|
| `frame_bytes` | 16,384 | complete NDJSON frame including LF |
| Application JSON body | 16,383 | transport-neutral body / WebSocket text payload; TCP LF maximum |
| TCP JSON body with CRLF | 16,382 | CR and LF both consume the physical-frame limit |
| `json_depth` | 16 | nested object/array depth |
| `json_values` | 1,024 | lexical values/members per message |
| `json_string_bytes` | 512 | each UTF-8 string/key |
| `clients` | 8 | shared TCP/WebSocket client capacity |
| `owner_reactor_mailbox` | 64 | messages per owner/reactor mailbox |
| `client_pending_requests` | 8 | admitted inbound exchanges/client |
| `client_reply_queue` | 8 | reply frames/client |
| `client_event_queue` | 16 | event frames/client |
| `network_sweep_bytes` | 8,192 | network work per sweep |
| `client_deadline_ms` | 2,000 | handshake, partial input, queued output deadline |
| `scopes` | 16 | retained logical scopes |
| `pending_operations_per_scope` | 8 | accepted nonterminal operations/scope |
| `pending_operations` | 64 | accepted nonterminal operations total |
| `terminal_operations_per_scope` | 32 | retained terminal outcomes/scope |
| `terminal_operations` | 256 | retained terminal outcomes total |
| `terminal_retention_seconds` | 600 | terminal TTL |
| `detached_scope_retention_seconds` | 1,800 | detached-scope TTL |
| `terminal_result_bytes` | 4,096 | retained result/code record |
| `event_replay_records` | 1,024 | event ring records |
| `event_bytes` | 4,096 | one event |
| `discovery_page_entries` / `current_page_entries` | 64 each | frozen page records |
| `discovery_page_bytes` / `current_page_bytes` | 8,192 each | frozen page payload |
| `recent_history_records` / `durable_history_records` | 128 each | measurement records/page |
| `history_page_bytes` | 8,192 | retained history page |
| `history_jobs` / `history_cursors` | 8 each | concurrent retained history work |
| `subscriptions_per_client` | 1 | aggregate subscription |
| `subscription_kinds` / `subscription_targets` | 8 / 16 | selected filter entries |
| `reference_result_records` / `controller_result_records` | 1 / 1 | result records |
| `pid_configuration_fields` / `controller_configuration_fields` | 5 / 6 | policy groups |
| `recorder.label_bytes` / `status_records` / `event_bytes` | 128 / 1 / 4,096 | Recorder API bounds |
| `capabilities` | 32 | advertised capability capacity (26 currently registered) |
| `semantic_name_bytes` | 64 | operation/code/category/semantic name |
| `error_message_bytes` / `error_details_bytes` | 256 / 2,048 | public error text/details |
| `configuration.property_records` | 256 | property records |
| `configuration.page_records` / `page_bytes` | 64 / 8,192 | property page |
| `configuration.runtime_overrides` | 32 | committed overlays |
| `configuration.staged_candidates` | 1 | retained candidate |
| `configuration.candidate_retention_seconds` | 30 | candidate lifetime |
| `emulator.targets` / `records_per_request` | 64 / 1 | virtual publication |
| `emulator.metadata_bytes` | 0 | caller metadata unsupported |
| `emulator.pending_per_scope` | 8 | pending virtual mutations/scope |

The 16,383-byte transport-neutral body limit is `frame_bytes - 1` and applies
directly to WebSocket text. TCP requires LF, so JSON plus LF may total 16,384 bytes.
CRLF is accepted equivalently, but the optional CR is inside the physical-frame
accounting; JSON plus CRLF may also total no more than 16,384 bytes.

Other connection-local lifetimes in production source are:

| Item | Bound |
|---|---:|
| frozen discovery/current/configuration projection | 5 s |
| completed history page | 5 s |
| history continuation cursor | 30 s |
| terminal server-shutdown delivery | best effort for 200 ms |

## Native Workbench client bounds

These are client behavior, not Runtime hello fields:

| Resource | Bound |
|---|---:|
| ordinary command mailbox | 32 |
| commands serviced per worker turn | 8 |
| in-flight exchanges | 8 |
| active recovery / quarantine records | 8 each |
| ordered update queue / GUI drain per frame | 64 / 64 |
| bootstrap events / operator action history | 64 / 64 |
| aggregate subscription | 1 |
| live trace display points | 4,096 |
| rebuild identities | 64 per Reference/controller/resource class |
| recovery journal | 64 KiB |
| connect/hello/partial-frame/blocked-write attempt | 2 s |
| ordinary request/reply | 5 s |
| unexpected-fault reattach episode | one absolute 3 s window |
| reattach retry spacing | at least 10 ms |
| worker shutdown | 3 s |

Explicit Workbench Disconnect has no automatic reconnect. Unexpected continuity loss
may use the one bounded episode. Neither behavior creates automatic mutation,
`operation_status`, or exact resubmission.

Queue pressure is scoped. Slow or malformed clients may lose delivery or be detached,
but they cannot acquire experiment ownership or block required native Runtime progress
indefinitely.
