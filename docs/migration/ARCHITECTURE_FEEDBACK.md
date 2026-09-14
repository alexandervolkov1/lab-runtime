# Architecture feedback после анализа v1

Статус: рекомендации для review, не изменение утверждённого target. Прочитаны все текущие architecture docs и ADR. Исходные [HIGH_LEVEL_ARCHITECTURE](../architecture/HIGH_LEVEL_ARCHITECTURE.md), [EXTENSION_MODEL](../architecture/EXTENSION_MODEL.md), [RUNTIME_AND_SAFETY_MODEL](../architecture/RUNTIME_AND_SAFETY_MODEL.md), [POC_PLAN](../architecture/POC_PLAN.md), [OPEN_QUESTIONS](../architecture/OPEN_QUESTIONS.md) и три ADR не редактировались.

## Общий вывод

Mapping не выявил необходимости менять high-level architecture: autonomous Rust Runtime, core без GUI/Lua/OS dependencies, central output authority, independent Reference, bounded execution, Instrument implementations разных extension levels и external orchestration остаются подходящими. V1 даёт полезные algorithms/tests и подтверждает некоторые lifecycle decisions; он не обосновывает перенос собственной class structure.

Ниже — конкретные уточнения, а не новый framework. Статус всех recommended changes — proposed. Особые implementation-detail решения вводятся только в соответствующем milestone.

## AF01. Закрыть вопрос о природе Furnace

Architecture issue: в OPEN_QUESTIONS Furnace оставлен до изучения v1.

Evidence from v1: [furnace.rs](../../../com_port_reader/src/process_control/furnace.rs) содержит thermal-loss feed-forward, filtered rate predictor, PI и limits; [ControlLoop](../../../com_port_reader/src/process_control/control_loop.rs) отдельно хранит Reference. Один input и один target. [Comparison](../../../com_port_reader/docs/furnace-controller-comparison.md) не доказывает superiority текущего tuning.

Current proposed model: native controller extension, возможно composition; Reference standalone.

Why it is insufficient: неопределённость мешает выбрать migration granule/diagnostics и может ошибочно объединить plant model, controller и recipe.

Recommended change: считать Furnace одним native controller component с private model/feed-forward/predictor/PI composition; independent Reference binding. Сохранить rate/predicted temperature/feed-forward diagnostics и exact comparison fixtures; публичный внутренний subgraph пока не вводить. Это снимает исследовательский вопрос, не требует нового ADR или crate.

## AF02. Уточнить semantic capabilities и configuration metadata

Architecture issue: generic parameter descriptors должны быть достаточны для всех clients и не давать implicit output authority.

Evidence from v1: [API instrument projection](../../../com_port_reader/src/core_api/instruments.rs) выводит output_supported из writable non-Boolean; virtual model содержит writable noise/model parameters, Metakon — configuration registers; [ControlLoop](../../../com_port_reader/src/process_control/control_loop.rs) делает setpoint read-only при Reference и atomic validates multi-field config.

Current proposed model: typed descriptors, named operations, central output arbitration.

Why it is insufficient: range/type/access сами по себе не определяют actuator side effects и relational validation; static read-only недостаточно для reference-managed config.

Recommended change: explicit measurement/config/actuator/action roles и side-effect/capability metadata, units/granularity/safe binding, dynamic parameter access with revision, atomic cross-field validation через один Rust boundary. Не нужен generic schema language с произвольной логикой: native validators допустимы. Detailed representation оставить stage 3/4.

## AF03. Явно специфицировать warm-up/reconfiguration state policy

Architecture issue: migration может потерять плавную retuning/filter startup semantics или объявить partial result полноценным свежим signal.

Evidence from v1: [filter](../../../com_port_reader/src/signal_processing/filter.rs) выдаёт partial-window mean/median сразу; EMA retune сохраняет last output/time; [graph](../../../com_port_reader/src/signal_processing/graph.rs) сбрасывает downstream для других replacements. [ControlLoop](../../../com_port_reader/src/process_control/control_loop.rs) resynchronizes input timing, сохраняя integral.

Current proposed model: quality-aware DAG, explicit lifecycle/reset and revisions.

Why it is insufficient: без выбранной policy нельзя определить, когда warm-up допустим для controller и какие state parts остаются после config commit.

Recommended change: у component config change обозначать preserve/reset/reinitialize policy; diagnostic warm-up quality отдельно от numeric output. Target default controller consumption требует подходящего quality; partial-window compatibility — явный opt-in, не silent default. Не требовать transactional rollback всего DAG: scoped node failure и predictable downstream quality достаточны для начала.

## AF04. Развести Reference progress и controller state retention

Architecture issue: default safe resume против привычного v1 continuity.

Evidence from v1: [ReferenceRuntime](../../../com_port_reader/src/process_control/reference_runtime.rs) sample-clock elapsed, target/rate rebase; [ControlLoop](../../../com_port_reader/src/process_control/control_loop.rs) pause/resume сохраняют integral/ramp. PID/Furnace не получают applied-arbiter feedback.

Current proposed model: independent monotonic Reference; safe-first manual↔auto и default reset/reinitialization.

Why it is insufficient: blanket «перенести pause/resume tests» конфликтует с target default; shared Reference нельзя неявно pause вместе с одним consumer.

Recommended change: сохранить current safe default, явно описать optional reference-progress retention separately from controller integrator retention; both require validation/freshness/new lease. Ramp target/rate continuity переносить как reference command semantics. Applied-output tracking/bumpless transfer — future capability с отдельными tests, не promise сейчас.

## AF05. Уточнить identity/revision на restart и catalog reconciliation

Architecture issue: parameter identity должна переживать rename, но не выдавать старую model instance за новую.

Evidence from v1: [catalog](../../../com_port_reader/src/core_runtime/instruments.rs) поддерживает descriptor generation, но unchanged descriptors сохраняют handle, отсутствующие entries не удаляются при refresh; virtual model IDs позиционные. [emulator service](../../../com_port_reader/src/application_runtime/device_emulator_service.rs) меняет endpoint/model при restart.

Current proposed model: runtime identities/generation/revisions и fenced stale work.

Why it is insufficient: descriptor equality и instance continuity — разные факты; без lifecycle trigger catalog может сохранять misleading availability.

Recommended change: instance generation меняется при restart/rebind независимо от equality metadata; revision — config/definition change; refresh имеет explicit unavailable/removed reconciliation. Old references/results проверяются against current binding. Вводить когда появляется restart/async completion, не полный distributed identity framework в first commit.

## AF06. Различать metadata/Query и физическую read operation

Architecture issue: будущий REPL требует предсказуемых snapshot queries и понятного async completion.

Evidence from v1: [protocol](../../../com_port_reader/src/core_api/protocol.rs) относит instrument/read к Monitor, [session](../../../com_port_reader/src/core_api/session.rs) выполняет read через worker; operation outcomes accepted/applied/unknown уже представлены, events retention bounded.

Current proposed model: единый Commands/Queries/Events API, без выбранного wire transport в core.

Why it is insufficient: если Query скрыто выполняет blocking I/O, common boundary наследует timeout/side-effect ambiguity v1 и плохо подходит interactive client.

Recommended change: Query latest/catalog/status — bounded snapshot; explicit refresh/read Command — named budgeted operation, результат через outcome/event. В раннем real Babashka slice проверить ergonomics, gap recovery, command/result correlation и bounded dedup. NDJSON может быть transport candidate, но v1 API не frozen compatibility spec.

## AF07. Уточнить recorder schema/evidence без смены high-level storage boundary

Architecture issue: nullable actual_output и process-wide DB не покрывают target traceability.

Evidence from v1: [ProcessRecord](../../../com_port_reader/src/process_recorder.rs), [SQLite schema](../../../com_port_reader/src/process_recorder/sqlite.rs): requested/actual output, nullable controller terms, session на app process, configuration source и external annotations; persistence unbounded, first failure disables sink.

Current proposed model: recorder-required control failure policy, evidence levels, decoupled history.

Why it is insufficient: буквальный schema reuse потеряет generic Furnace diagnostics, quality/time/generation и distinction ACK/readback/physical effect.

Recommended change: SQLite оставить adapter candidate, schema адаптировать с session/run/config provenance, typed evidence/diagnostics/gaps. Preserve action completion независимо от disk, но required recorder health влияет на authority. Full persistent evidence только при relevant physical/recorder stages; no event-sourcing mandate.

## AF08. POC order и реальный donor protocol

Architecture issue: оригинальный POC использует minimal Modbus RTU и откладывает настоящий Babashka после full POC/длинных soak runs.

Evidence from v1: [Metakon](../../../com_port_reader/src/protocol/metakon.rs) не Modbus; known tests пригодны только для него. [CoreHost](../../../com_port_reader/src/core_runtime/host.rs) и [TCP integration tests](../../../com_port_reader/tests/core_api_smoke.rs) уже показывают хороший external boundary donor, но real BB client acceptance отсутствует.

Current proposed model: native/data-driven transport stage, bounded Lua, recorder и fault/soak verification, simulated clients внутри POC.

Why it is insufficient: реализация ещё одного protocol before extracting known vectors увеличит scope; fake clients не проверяют основной interactive Clojure workflow. 24/72h gate слишком поздно даёт feedback о domain API.

Recommended change: nine-stage recommendation в [POC_REFINEMENT](POC_REFINEMENT.md): Metakon narrow codec/native driver + небольшая data-driven operation fixture, real BB slice stage6 до recorder7, architecture POC fake-clock/fault +30–60min, 24/72h только candidate. Если первый target bench действительно Modbus, он остаётся отдельной реализацией с собственными tests; donor не выдаётся за готовую поддержку.

## Что не меняется

Дополнительное подтверждение важности общего application boundary: [ApplicationRuntime::dispatch_command](../../../com_port_reader/src/application_runtime.rs) сейчас применяет safe barrier перед emulator stop только для ExternalCore, сохраняя другой GUI/Lua behavior. Это не требует менять target architecture, но требует explicit cross-adapter semantic parity tests: origin может влиять на authorization/audit, не отключать обязательную safety transition. Shared host/thread сам по себе не является единым domain API.

Rust central output authority, шесть target safety states, no raw-I/O scripting bypass, safe path независимо от Lua, bounded resources, core dependency direction, separate client processes и отсутствие native ABI plugin framework остаются без изменений. Старые convenience defaults не имеют приоритета над target safety.

Все предложения выше отражены в migration mapping/risks/POC. Architecture docs/ADR не переписаны и не переведены в Accepted автоматически. После review можно отдельной documentation change внести выбранные уточнения; это не выполнено молча в текущей задаче.
