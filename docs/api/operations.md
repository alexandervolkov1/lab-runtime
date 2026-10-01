# Operations and capabilities

Applicable to Application protocol v1 and the current repository/v0.1 product.

The source authority is the single registry in
`apps/lab-runtime/src/protocol.rs`. It contains exactly 43 unique operations: 22
queries and 21 mutations.

## Availability

Every operation has one source-defined availability category:

| Category | Advertisement rule |
|---|---|
| `Always` | present in every active composition |
| `Recorder` | durable Recorder/history service configured |
| `Configuration` | declarative deployment lifecycle configured |
| `SimpleDeviceProvisioning` | an eligible configured resource and declarative provisioning path are available |
| `ResourceReconnect` | at least one configured physical resource supports explicit reconnect |
| `EmulatorPublication` | at least one explicit virtual signal accepts external publication |
| `VirtualModelLifecycle` | at least one Runtime-owned virtual model supports restart |

The hello `operations` list is authoritative. `recording_status` is `Always` and
reports an unconfigured state when Recorder is absent; Recorder mutations/history
operations are advertised only with `Recorder`.

Arguments below are the strict top-level field allowlist. Some fields are conditional
on a mode/kind; operation-specific validation still applies.

## Complete operation registry

| Name | Kind | Arguments | Important result/outcome | Availability | Purpose |
|---|---|---|---|---|---|
| `hello` | Query | `scope` | boot/protocol/API identity, scope, next sequence, operations, capabilities, limits, event cursors | Always | establish or reattach a session |
| `discover` | Query | none | first frozen discovery page | Always | enumerate instruments, signals, components, controllers, References, outputs, resources, configuration |
| `discovery_page` | Query | `projection`, `index` | next page of same frozen discovery view | Always | continue discovery projection |
| `describe` | Query | `instrument` | stable instrument and parameter descriptor | Always | inspect one instrument |
| `resource` | Query | `resource` | state, availability, generations, bindings, revision, failure | ResourceReconnect | inspect reconnectable resource |
| `configuration_status` | Query | none | active revision/source, staged candidate, overlays | Configuration | inspect deployment lifecycle |
| `configuration_properties` | Query | none | first frozen property page | Configuration | enumerate typed configuration properties |
| `configuration_page` | Query | `projection`, `index` | next page of same frozen property view | Configuration | continue property projection |
| `latest` | Query | `signal` | current observation or explicit `not_observed` | Always | read one current signal |
| `measurements_current` | Query | none | first frozen current-measurement page | Always | snapshot all current signals |
| `measurements_page` | Query | `projection`, `index` | next page of same frozen current view | Always | continue current projection |
| `measurement_window` | Query | `signal`, `max_records` | oldest-first bounded Runtime recent history | Always | inspect in-memory signal history |
| `controller` | Query | `controller` | lifecycle, revision, bindings, policy, last tick/output | Always | inspect one controller |
| `reference` | Query | `reference` | fixed/ramp state, revision, value/target/rate/unit | Always | inspect one Reference |
| `component` | Query | `component` | implementation, binding, generation, state, diagnostics | Always | inspect one managed component |
| `output` | Query | `actuator` | read-only output authority/evidence projection | Always | inspect output state without write authority |
| `operation_status` | Query | `request_id` | accepted/completed/failed, or outcome_unknown when no record is retained | Always | inspect retained state for current scope; unknown makes no high-water claim |
| `subscribe` | Query | `after`, `filter` | aggregate subscription token and accepted cursor | Always | start connection-local event delivery |
| `unsubscribe` | Query | `subscription` | `removed` boolean | Always | remove connection-local subscription |
| `reference_configure` | Mutation | `reference`, `expected_revision`, `kind`, `value`, `target`, `rate` | committed complete Reference projection | Always | replace fixed/ramp policy |
| `reference_retune` | Mutation | `reference`, `expected_revision`, `target`, `rate` | committed ramp projection | Always | retune an existing ramp |
| `controller_configure_pid` | Mutation | `controller`, `expected_revision`, `pid` | updated controller projection | Always | replace PID fields in safe lifecycle |
| `controller_configure` | Mutation | `controller`, `expected_revision`, `pid`, `ema`, `max_input_age_ns`, `max_tick_gap_ns`, `lease_lifetime_ns`, `proposal_ttl_ns` | full updated controller projection | Always | replace complete mutable controller policy |
| `controller_start` | Mutation | `controller` | controller lifecycle projection | Always | start Ready controller |
| `controller_pause` | Mutation | `controller` | safe-transition projection | Always | pause active controller |
| `controller_resume` | Mutation | `controller` | Warming projection with fresh authority | Always | resume safely Paused controller |
| `controller_reset_failed` | Mutation | `controller` | Paused projection | Always | acknowledge safely failed controller |
| `runtime_shutdown` | Mutation | none | truthful finite cleanup/safety/Recorder result | Always | explicitly request Runtime process shutdown |
| `stage_configuration` | Mutation | none | candidate ID, base revision, expiry, classified effects | Configuration | validate and retain one deployment candidate |
| `stage_simple_device_candidate` | Mutation | `expected_revision`, `candidate` | candidate ID, definition identity, bounded expiry and classified effects | SimpleDeviceProvisioning | validate and retain one process-local declarative SimpleDevice candidate |
| `apply_configuration` | Mutation | `candidate_id`, `expected_revision` | new committed configuration revision | Configuration | apply retained candidate under fences |
| `reload_configuration` | Mutation | none | new revision | Configuration | read, validate, stage, and apply source atomically |
| `property_configure` | Mutation | `target`, `property`, `value`, `expected_revision` | target/property/new revision | Configuration | apply supported scalar property |
| `emulator_publish` | Mutation | `signal`, `state`, `value`, `expected_generation` | virtual signal/generation/state/time | EmulatorPublication | publish good/unavailable virtual observation |
| `virtual_models_restart` | Mutation | none | model count and new generation | VirtualModelLifecycle | restart Runtime-owned virtual models |
| `reconnect_resource` | Mutation | `resource`, `expected_binding_generation` | resource and new binding generation | ResourceReconnect | explicitly replace resource session; never rearm control |
| `recording_status` | Query | none | Recorder/archive/run/admission/coverage/failure projection | Always | inspect Recorder or unconfigured state |
| `recording_start` | Mutation | `label` | database/run/interval identity and provenance flags | Recorder | durably start a run |
| `recording_stop` | Mutation | `run_id` | drained/sealed/committed flags and run identity | Recorder | stop and seal exact active run |
| `experiment_annotate` | Mutation | `name`, `data` | record sequence and pending durability | Recorder | admit bounded informational fact |
| `history_read` | Mutation | `mode`, `database_id`, `boot_id`, `run_id`, `signal`, `from_ns`, `to_ns`, `max_records`, `cursor` | terminal retained history page token | Recorder | execute bounded asynchronous archive selection in the connection-local job/page slot |
| `history_page` | Query | `page_token` | retained durable page | Recorder | read the connection's completed retained history page |
| `history_release` | Query | `page_token` | `released:true` | Recorder | release retained page |

There is no raw output/register write operation. Virtual publication cannot target a
physical signal or fabricate resource readiness, transport completion, ACK, readback,
or output-safety evidence.

## Capabilities

Capabilities are structured versioned feature/discovery labels, not authorization
credentials. Each is returned as `{"name":...,"version":1,"stability":"stable"}`
when its representative operation is available. Use hello's operation list for actual
operation availability.

| Capability | Representative operation |
|---|---|
| `operation_lifecycle` | `operation_status` |
| `structured_discovery` | `discover` |
| `live_subscriptions` | `subscribe` |
| `current_measurements` | `measurements_current` |
| `recent_measurement_history` | `measurement_window` |
| `instrument_queries` | `describe` |
| `reference_read_write` | `reference_configure` |
| `controller_status` | `controller` |
| `controller_configuration` | `controller_configure` |
| `controller_lifecycle` | `controller_start` |
| `managed_components` | `component` |
| `output_status` | `output` |
| `runtime_shutdown` | `runtime_shutdown` |
| `recording_status` | `recording_status` |
| `recording_control` | `recording_start` |
| `measurement_history` | `history_read` |
| `resource_status` | `resource` |
| `configuration_read` | `configuration_status` |
| `configuration_properties` | `configuration_properties` |
| `configuration_write` | `property_configure` |
| `deployment_configuration` | `stage_configuration` |
| `simple_device_provisioning` | `stage_simple_device_candidate` |
| `resource_reconnect` | `reconnect_resource` |
| `virtual_instruments` | `discover` |
| `emulator_publication` | `emulator_publish` |
| `virtual_model_lifecycle` | `virtual_models_restart` |

For request shapes, see [Protocol and sessions](protocol-and-sessions.md). For mutation
lifecycle and paging, see
[Events, mutations, and recovery](events-mutations-and-recovery.md).
