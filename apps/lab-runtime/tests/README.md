# Integration-test map

These tests exercise public/process boundaries; focused implementation unit tests
remain beside their modules. The grouping below is navigational, not a hierarchy of
importance.

- Application/protocol: `api_*`, `protocol_foundation`, `client_isolation`,
  `request_deduplication`, `operation_outcomes`, `subscription_recovery` and
  `m9b8_fault_acceptance`.
- Runtime orchestration: `host_scheduler`, `runtime_startup`, `runtime_events`,
  `runtime_lifecycle_operations` and `runtime_shutdown`.
- Control and virtual sources: `control_api`, `emulator_api`, `measurement_api` and
  `milestone9a_native`.
- Configuration/resources/transport: `configuration_*`, `configured_physical*`,
  `resource_configuration_api`, `definitions` and `com_recorder_shutdown`.
- Recorder/SQLite: every `recorder_*` binary covers a distinct durability,
  provenance, history, failure, isolation, time, reopen or shutdown invariant.

High-value historical oracles are intentionally retained:

- `m9b8_fault_acceptance` freezes malformed-client isolation and bounded failures;
- `client_isolation` additionally freezes the real TCP malformed-frame matrix,
  eight-client admission/churn, slow-writer isolation and duplicate exchange IDs;
- `com_recorder_shutdown` proves `TransportShutdown::Pending` receives later owner
  turns rather than becoming a false terminal result;
- Core `milestone9d_physical_output` protects authority recheck, ACK/readback
  separation and the no-retry rule after an ambiguous started write;
- Recorder `writer_barrier_tests` protects the mutex/Condvar predicate against the
  former lost-wake race;
- `recorder_history_api` protects cursor/job cleanup across disconnect and reused
  client capacity.

Fault barriers and fake transports are test seams only. They do not grant Runtime,
transport-write or SQLite authority to Application clients or managed components.
