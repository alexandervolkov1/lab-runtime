# Current work — M9A neutral managed components and native Rust execution

```text
M8: ACCEPTED
M9A: AUTHORIZED
Current phase: M9A — neutral managed components and native Rust execution
Current model: SOL_HIGH
M9B: NOT AUTHORIZED
```

This is the only current detailed implementation authorization. The documentation
transition that created this authorization must stop before implementation; begin
the work below only in a subsequent task.

## Objective

Remove Lua-specific leakage from the shared managed-component boundary and add a
first-class native Rust executor path through the retained language-neutral
contract. Prove that path with at least one small native model, filter or transform.

Preserve the established concepts and semantics:

```text
Invocation
ComponentResult
ComponentCompletion
ComponentExecutor
PlainData
generation/revision/failure fencing
```

## Authorized scope

1. Introduce neutral common definition, capability, lifecycle and provenance
   vocabulary for new work. Audit at least `ComponentDefinition.source`, public
   `lua_source`/`lua_transform` capabilities, `reload_managed_scripts`, generic
   helpers such as `stage_standard_lua`, and
   `ManagedLuaSource`/`managed_lua_source`.
2. Add registration and execution for native Rust managed components through the
   same validation, unit, bounded-state, bounded-execution, failure and
   replacement/reload rules used by the common contract.
3. Implement and test at least one small deterministic native Rust model, filter or
   transform through that path.
4. Keep M5 Lua frozen except for changes strictly required to preserve compatibility
   with the neutral boundary, plus bug/regression, safety/security or documentation
   corrections.
5. Update current architecture and user/developer documentation to describe the
   resulting neutral boundary and native path without presenting Lua as the v0.1
   application workflow.

## Required invariants

- The Rust Runtime remains the sole authoritative mutable experiment owner.
- Managed components remain Runtime-invoked computations; they do not receive raw
  transport access, OutputAuthority, general Runtime mutability, or authority to
  fabricate physical observation, ACK, readback, safe or transport evidence.
- Generation and revision fencing reject late completions after replacement/reload.
- Registration, queues, execution, component state and retained diagnostics remain
  explicitly bounded with documented overflow, shutdown and failure behavior.
- Validation and unit checks occur before results can affect committed Runtime
  state.
- Replacement/reload does not imply controller rearm or output authorization.
- Required Recorder and provenance behavior remain truthful and fail closed.
- `lab-core` remains independent of OS, GUI, storage and scripting runtimes.
- Historical SQLite provenance values and all M8 evidence archives remain
  unchanged; do not rename old evidence merely to neutralize vocabulary.

## Tests-first acceptance

Add deterministic tests before or with each behavior change that prove at least:

1. neutral definitions and capabilities do not require Lua-specific common fields;
2. the native executor follows the existing invocation/result/completion contract;
3. the example native component validates configuration and units and has bounded,
   deterministic state and execution;
4. late completions from a replaced generation cannot commit;
5. native failure, timeout/limit and invalid-result behavior is scoped and truthful;
6. replacement/reload preserves generation, revision, dependent warmup and recorder
   provenance semantics;
7. frozen M5 Lua behavior and sandbox restrictions remain covered;
8. existing controller, output-safety, Recorder, deployment and Application API
   regressions remain green.

Prefer injected deterministic executors/counters over wall-clock timing as an
oracle. Do not weaken an invariant or enlarge a production bound merely to satisfy a
test.

## Verification

Run focused tests first, then the repository's established full software gates,
including at minimum:

```powershell
cargo test --workspace
cargo test --workspace --release
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
git diff --check
```

Also run the established warning-denied rustdoc, Babashka acceptance, client and
finite demo/smoke gates documented by the repository. No COM5 or physical hardware
run is part of M9A.

## Boundaries and non-goals

- Do not reopen or modify any M8 hardware evidence archive. The successful archive
  remains `examples/metakon-513-com5-transient-open-retry-history.sqlite`, SHA-256
  `63ecb8575be5ac82ef968b929e8b3098b051a0f30a55c190b7fdc1dcaf64dc80`.
- Do not perform the deferred `.gitattributes` release-hardening correction as part
  of managed-component behavior work.
- Do not add a persistent Lua workspace, Lua application/scenario API, Lua GUI API,
  Lua REPL/editor or broader M5 feature scope.
- Do not implement M9B virtual/emulator, presentation/control/properties or broader
  Application API work. M9B remains NOT AUTHORIZED until M9A receives explicit
  external acceptance.
- Do not begin the M10 GUI or M11 release-hardening milestones.
- Do not implement Steel or remove `lab-lua` in M9A.

## Commit and review discipline

Use tests-first logical commits. Keep neutral contract/native execution changes
separate from final coordination/report updates, and do not mix unrelated cleanup.
At completion, stop for explicit external M9A review; do not cross into M9B
automatically.
