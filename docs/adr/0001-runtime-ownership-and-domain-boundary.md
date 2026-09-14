# ADR-0001: Runtime владеет экспериментом и предоставляет одну domain boundary

Status: Proposed — архитектурное предложение, ожидающее review, не свидетельство реализации или пользовательского утверждения.

Date: 2026-09-14.

## Context

Лабораторный эксперимент продолжается часы или дни. GUI и внешние REPL/сценарии могут закрываться, зависать и перезапускаться. Brief требует descriptors, несколько способов реализации instruments, независимые controllers/recorder и будущих remote clients. Если критическое состояние живёт в GUI или Babashka, время жизни клиента определяет время жизни управления. Отдельные business APIs для каждого frontend приводят к разным правилам валидации и lifecycle.

## Decision

Предлагается самостоятельный headless Rust Runtime process. Он владеет experiment configuration/topology, instrument registry/polling, signal windows, controller/reference lifecycle, output authority и recording sessions. Рабочие исполнители хранят только делегированное закрытое состояние под Runtime lifecycle; клиенты взаимодействуют через IDs, versions и snapshots.

Commands / Queries / Events образуют одну языконезависимую domain boundary. Команды проверяют identity/capabilities, revisions и preconditions; accepted, completed, physical effect и durable recording различаются. Clients используют descriptors/introspection, bounded subscriptions, snapshot/cursor и явный reconnect/resync. GUI и Babashka не получают mutable runtime objects.

Core — библиотека domain/application правил. Rust host объединяет Core с physical I/O, protocol/driver, storage, local API и optional Lua adapters. Compile-time dependencies направлены от adapters к Core contracts; Core не зависит от GUI, VM, wire encoding или OS serial API. Virtual clocks и I/O заменяются через узкие границы для проверки runtime semantics.

Отключение клиента не удаляет созданный им native controller. External/manual output leases прекращаются отдельно по safety policy. Будущие шаги внешней recipe не принадлежат Runtime, пока явно не переданы ему; это решение не вводит durable workflow engine.

## Consequences

GUI может быть заменён без изменения domain model; Babashka и test harness проверяют те же правила. Native управление и запись продолжаются независимо от клиента при пригодных локальных inputs и отсутствии явно требуемого supervisor dependency.

Понадобятся самостоятельный lifecycle процесса, command outcomes, bounded queues, revisions и reconnect semantics. Это небольшая необходимая цена независимых клиентов; full event sourcing, распределённые транзакции и generic message broker не нужны. Runtime crash остаётся общим отказом процесса; persistent configuration/history не означает автоматическое восстановление активного управления.

## Rejected alternatives

- GUI как host всей системы: закрытие окна связывает experiment lifetime с интерфейсом, усложняет headless и remote usage.
- Babashka как владелец native loop/recorder: REPL restart начинает определять безопасность и непрерывность опыта.
- Отдельные object APIs для Lua, GUI и Babashka: дублируют lifecycle/validation и создают несогласованные права.
- Микросервисы для instruments/controllers/recorder: добавляют распределённые отказы и deployment без требования масштабирования.
- Traits/crates для каждой domain сущности: большинство descriptor/message/value concepts достаточно представить данными и modules.

Подробности: [HIGH_LEVEL_ARCHITECTURE.md](../architecture/HIGH_LEVEL_ARCHITECTURE.md), [execution и disconnect](../architecture/RUNTIME_AND_SAFETY_MODEL.md).
