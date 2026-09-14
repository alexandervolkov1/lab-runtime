# ADR-0003: Rust, определения и ограниченный Lua; orchestration через client API

Status: Proposed — выбор embedded host должен пройти POC; compiled plugin SDK не утверждается и не реализуется.

Date: 2026-09-14.

## Context

Brief требует добавлять известные приборы, небольшие adapters, filters и экспериментальное управление без постоянных изменений Rust Core. Низкоуровневые протоколы и safety-critical возможности остаются Rust. Lua удобна для embedded callbacks, Babashka — для external REPL/recipes, но два полноразмерных API и native plugin ABI создадут несоразмерную стоимость поддержки.

## Decision

Первая модель расширения включает статически подключённые Rust modules, schema-validated data definitions для известных протоколов и optional bounded Lua component host. Native/data/Lua/virtual instruments используют одну Instrument/Descriptor/Operation модель. Filters и Reference используют SignalNode contracts; controllers возвращают только OutputProposal.

Lua host расположен вокруг Core и имеет ограниченный component context: inputs/windows/configuration, typed results, scoped queries/events и разрешённые именованные instrument operations. Нет raw COM/OS access, FFI, снятия budgets и владения experiment lifecycle. Физические requests ограничены проверенными Rust-executable profiles; safe output path не зависит от Lua VM. Reload создаёт новую generation через quiescent/safe boundary; private state по умолчанию сбрасывается.

Babashka — необязательный внешний клиент общего C/Q/E API. Foundation reconciliation по migration AF08 уточняет порядок, не boundary: после bounded Lua выполняется небольшой реальный Babashka slice на virtual fixture, затем recorder/fault verification. Никакого BB/IPC в Milestone 1; прежнее откладывание после полного POC заменено явным staged plan.

Compiled plugins без rebuild в v1 не нужны. Для будущих process/WASM adapters сохраняются typed component contracts, lifecycle, deadlines, generation и central authority. Stable C ABI или другой SDK принимается только после конкретного требования; Rust dylib не предлагается как внешний стабильный ABI.

## Consequences

Известный прибор добавляется через данные, простой parser/filter/controller — через Lua, recipe — в Babashka без Core change. Runtime может поставляться без Lua/Babashka и не становится зависимым от одного scripting языка.

Цена Lua — бюджеты, worker isolation, проверка host calls и ограниченный reload. Embedded VM не даёт process crash isolation. Если POC не подтвердит ограничение нужного execution path, область Lua сужается либо выбирается process host; небезопасный fallback не включается молча. Hardware definitions и trusted host требуют проверки; script sandbox не доказывает правильность данных прибора.

Не всякий новый протокол можно выразить данными или Lua без Rust: это сознательная граница низкоуровневой/safety возможности. Новый compiled extension впоследствии потребует host adapter и compatibility work, но не должен менять Instrument/Controller domain semantics.

## Rejected alternatives

- Поддержать Rust DLL, C ABI, WASM и process plugins сразу: нет подтверждённых задач, оправдывающих несколько loaders/SDK и lifecycle моделей.
- Привязать Core к Lua VM: ухудшает headless testing и будущую замену host.
- Перенести всё в embedded Lua orchestration: увеличивает API и связывает lifecycle опыта с VM.
- Использовать только external Babashka для всех adapters/filters: добавляет process/IPC lifecycle к маленьким локальным компонентам и не закрывает требование embedded extension.
- Обязательная установка обоих языков: не нужна пользователям только native/data-driven Runtime.
- Разрешить Lua произвольный transport transaction под названием extension: возвращает обход output authority и совместного port scheduling.

Полная сравнительная таблица: [EXTENSION_MODEL.md](../architecture/EXTENSION_MODEL.md). Проверка и дальнейшая эволюция: [POC_PLAN.md](../architecture/POC_PLAN.md), [OPEN_QUESTIONS.md](../architecture/OPEN_QUESTIONS.md).
