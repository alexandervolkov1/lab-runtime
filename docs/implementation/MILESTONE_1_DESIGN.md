# Milestone 1: domain foundation

Дата: 2026-09-14. Дизайн зафиксирован **до написания Rust**. Scope задают
[Foundation plan](../../FOUNDATION_MILESTONE1_PLAN.md) и [AGENTS](../../AGENTS.md).
Это минимальный in-process foundation, не полный автономный runtime.

## 1. Граница и структура

Cargo workspace содержит ровно два package:

- `crates/lab-core`: стандартная библиотека Rust, domain types/validation,
  bounded signal storage, native virtual instrument, synchronous Runtime owner.
- `apps/lab-runtime`: конечный deterministic console demonstration, зависит
  только от `lab-core`. Отображает introspection generic-образом.

Core не использует clock/OS I/O, threads, async, serde, внешние зависимости или
донор. Host не запускает daemon, listener, GUI или hardware. Нет output execution,
OutputArbiter, controllers, serial/Metakon, Lua, Babashka, recorder.
Actuator descriptor не является разрешением писать actuator.

Модули core: `model`, `signal`, `virtual_instrument`, `runtime` и re-exports в
`lib.rs`. Traits для plugins/clock/transport сейчас не нужны.

## 2. Identity и ownership

`InstrumentId(u64)` назначает вызывающая сторона при явной регистрации.
`ParameterId(u64)` локален instrument. `SignalId` — typed пара InstrumentId и
ParameterId для measurement данного экземпляра. Это local identity, не persistent
глобальный namespace. Нет удаления/replacement, поэтому reuse поколения не возникает.
Rename меняет только display name; IDs, config, latest и window сохраняются.
Duplicate instrument IDs отклоняются до mutation. Duplicate display names разрешены:
клиент обязан использовать ID, а не имя. Name: nonblank, максимум 128 UTF-8 bytes.

Runtime единолично владеет BTreeMap registry, descriptors, virtual configuration
и signal buffers. Queries возвращают owned snapshots; изменённая копия не влияет
на Runtime. Порядок discovery — InstrumentId, параметров — стабильный descriptor order.
Нет global registry, Arc/Mutex, mutable handles наружу.

Generation, epochs, leases, sequence, cursors, request IDs, event subscriptions и
optimistic revision сейчас отсутствуют: нет async completion, replacement или
concurrent read-modify-write. Configuration command задаёт одно абсолютное значение
и валидируется синхронным owner; revision не решала бы дополнительной задачи M1.

## 3. Typed descriptors и validation

`Value`: Float(f64), Integer(i64), Boolean(bool), Text(String), Enum(String).
Integer не преобразуется неявно во Float, Text — в Enum. `ValueType` описывает тип;
`ValueSpec` задаёт тип и constraint: inclusive finite float range, inclusive integer
range, bool, максимальную byte length текста, непустой bounded список enum choices.
Validate-before-commit: wrong type, nonfinite, range и invalid definition различаются.

`Unit`: Celsius, Percent, Pascal, Unitless. Это небольшой vocabulary (°C, %, Pa, 1),
не dimensional engine и не конвертер. Units относятся к descriptor и sample;
Configure не принимает единицы и не делает скрытого пересчёта. Нечисловое значение
может иметь только Unitless. Range — constraint descriptor, а не safety profile.

Parameter descriptor содержит ID, name, ValueSpec, Unit, AccessMode,
ParameterRole, WriteEffect и optional SignalId. Roles: Measurement, Configuration,
Actuator, Action, Diagnostic. Access: ReadOnly, ReadWrite, WriteOnly.
WriteEffect: None, ConfigurationOnly, OutputAffecting. Число + writable не определяет
actuator; разрешены только configuration-only writes, и только роли Configuration.
Неизвестная/неподдержанная операция отклоняется, не проходит через общий setter.

## 4. Native VirtualTemperature

Одна implementation с четырьмя параметрами, одинаковая для всех экземпляров:

| ParameterId | Name | Type/unit/range | Role/access/effect |
| --- | --- | --- | --- |
| 1 | temperature | Float °C [-100, 110] | Measurement / ReadOnly / None; Signal |
| 2 | heater_power | Float % [0, 100] | Actuator / ReadWrite / OutputAffecting; metadata only |
| 3 | base_temperature | Float °C [-100, 100] | Configuration / ReadWrite / ConfigurationOnly |
| 4 | measurement_enabled | Boolean / Unitless | Configuration / ReadWrite / ConfigurationOnly |

`heater_power` не имеет commanded/observed значения: отсутствие не заменяется нулём.
Показ writable hardware capability не означает implemented write operation M1.
Выключенный measurement_enabled — явная детерминированная fault injection.

Generator при refresh: `base_temperature + (elapsed_milliseconds % 10000) / 1000.0`.
Пилообразная прибавка [0, 10) не зависит от числа queries/ticks и не моделирует печь,
нагрев, feedback или thermodynamics. Сначала modulo целого Duration, затем f64;
даже большой Duration не даёт overflow/nonfinite. Generator всегда укладывается
в declared measurement range при валидном base_temperature.

Configuration policy — **preserve observations**, новые настройки влияют только
на следующий explicit refresh. Старый sample сохраняет своё исходное value/time;
Configure/Rename/Query не делают его новым измерением. Значения configuration и
measurement state представлены раздельно. Нет implicit reset/reinitialize graph.

## 5. Time, failure и bounded Signal

В RefreshMeasurement вызывающий код передаёт `std::time::Duration` — elapsed
monotonic time в одной runtime time domain. Wall-clock и sleep не используются.
Первый sample допускает zero; последующие timestamps строго возрастают **для
данного signal**. Equal/backward timestamp отклоняется до любых изменений. Порядок
между независимыми signals не требуется. Это проверка входа, не аппаратный clock.

Sample содержит SignalId, Unit, monotonic timestamp и явный outcome:
Good(Value) либо Unavailable(MeasurementFailure). Quality и optional value доступны
через методы; невозможно представить failed sample как Good со старым значением.
До первого refresh latest отсутствует (не Good zero и не fabricated error sample).

При measurement failure latest и window получают Unavailable с временем попытки;
Command возвращает typed MeasurementUnavailable. Это **зафиксированное наблюдение
неудачи**, единственная предусмотренная ошибка Command с обновлением observation.
Validation/access/time/config ошибки не меняют вообще никакого runtime state.
Повтор после включения measurement даёт новый Good sample на более позднем времени.

Window — VecDeque с configured capacity, oldest eviction до push; latest — back
того же buffer (нет копии, которая может разойтись). Capacity 1..=4096, максимум 64
instruments на Runtime; четыре фиксированных descriptor на instrument, один Signal.
Configured names ограничены 128 bytes, text/enum definitions ограничены constants.
Это bounds хранимых domain данных; не hard allocator/RSS guarantee. Query копирует
не более одного bounded window либо bounded registry. Клиент сам отвечает за срок
жизни и число сохранённых копий. Full history, persistence и subscriptions отсутствуют.

## 6. Локальная façade

`Runtime::command(&mut self, Command) -> Result<CommandResult, Error>`:
RegisterVirtual, RenameInstrument, ConfigureParameter, RefreshMeasurement.
Результаты различают Registered, Renamed, Configured и MeasurementRefreshed.
Все runtime mutations проходят этот путь; native implementation private.

`Runtime::query(&self, Query) -> Result<QueryResult, Error>`:
Discover, DescribeInstrument, GetInstrumentState, GetLatestSignal, GetSignalWindow.
State snapshot содержит configuration values отдельно от latest observations;
unsupported actuator state явно отсутствует. Discovery возвращает descriptors,
по которым клиент может отобразить любой instrument без ветвления по model name.
Ни один query не вызывает generator, не читает часы и не меняет registry/config/buffer.

Typed Error различает unknown instrument/parameter/signal, duplicate ID,
wrong type, nonfinite, out of range, read-only, operation not allowed,
invalid configuration, nonmonotonic time и measurement unavailable. Display/Error
реализуются без string parsing клиентом. Нет wire schema или стабильного external API.

## 7. Tests-first acceptance и проверка

До primary implementation пишутся integration tests публичной boundary:

1. Generic discovery нового instrument и initial unknown observation.
2. Полный descriptor, explicit actuator semantics и запрет output mutation.
3. Rename сохраняет instrument/parameter/signal identity, config и observations.
4. Invalid type/range/access/config отклоняется атомарно.
5. Повторные Queries не меняют state, window, timestamp или generator.
6. Explicit refresh на injected elapsed time создаёт ожидаемый sample.
7. Failure не становится zero/old fresh value; latest bad, recovery explicit.
8. Window bound/eviction/latest, включая capacity 1.
9. Одинаковая command/time sequence даёт одинаковые snapshots/outcomes.

Дополнительно: duplicate ID/name policy, nonfinite, stable parameter lookup,
unknown IDs, invalid bounds/unit/type combinations, equal/backward time,
snapshot isolation, resource limits, большие Duration и typed value variants.
Removal tests не нужны: removal не реализуется ради теста.
Записать red acceptance run, затем green `cargo test --workspace`,
`cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
`cargo run -p lab-runtime`, dependency/scope review. Hardware/soak не входят в M1.

## 8. Provenance и baseline ограничения

Весь production Rust и tests пишутся заново. Stable rename и atomic typed validation
test intents опираются на [reuse plan](../migration/REUSE_PLAN.md) и
[v1 mapping](../migration/V1_TO_LAB_RUNTIME_MAP.md), а не на копирование v1 bodies,
SeriesStore, JSON DTO или runtime. IDs/ownership/buffers здесь новая минимальная модель.
Алгоритм virtual generator — новая простая функция, не перенос Furnace.

[Donor baseline](../migration/DONOR_BASELINE.md): HEAD
50d3d1e3de84c650e1aa0ffbf1625044f794d315; 784 passed, 1 failed (documentation link),
1 ignored в library tests; последующие targets не подтверждены. Donor не изменён.

Предоставленный Foundation plan содержит 1114 строк и обрывается в §29 после
начала code fence (две обратные кавычки). Все имеющиеся требования учтены;
отсутствующее продолжение не реконструировалось. Пользователь предупреждён.
После M1 работа останавливается с отчётом; следующий milestone требует новой команды.
