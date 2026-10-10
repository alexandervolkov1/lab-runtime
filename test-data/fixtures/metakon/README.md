# Metakon regression fixtures

These four TOML deployments and three JSON definitions are deterministic test
inputs, not ready-to-run hardware examples. The COM ports and device addresses
describe historical setups. Do not launch them against physical equipment without
reviewing the deployment and output safety settings.

- `configuration_validation` loads all four deployments with the production parser.
- `configured_physical` checks whole-degree temperature scaling on a scripted transport.
- `configured_physical_output` checks output/readback/safe-zero and that reconnect
  cannot rearm a controller after ambiguous output, using a fake device.

Keep each TOML beside `definitions/`: definition paths resolve relative to the TOML.
The recording paths name generated output databases; no historical SQLite files
are required. User examples are tested separately under [examples/](../../../examples/).
