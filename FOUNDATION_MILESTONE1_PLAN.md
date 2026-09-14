# lab-runtime: Foundation + Milestone 1

Ты продолжаешь работу над проектом:

```text
D:\rust\lab-runtime
```

Высокоуровневая архитектура и migration analysis `com_port_reader` завершены.

Теперь разрешено начать implementation phase.

Однако задача этого этапа строго ограничена:

> Зафиксировать архитектурный baseline, проверить donor test baseline, создать минимальный Rust workspace и реализовать только Milestone 1.

Не переходить к Milestone 2.

Не реализовывать OutputArbiter, serial, Metakon runtime, Lua, Babashka, IPC, recorder или GUI.

---

# 1. Перед началом

Полностью прочитай:

```text
AGENTS.md
PROJECT_BRIEF.md
ARCHITECTURE_PLAN.md
MIGRATION_ANALYSIS_PLAN.md

docs/architecture/HIGH_LEVEL_ARCHITECTURE.md
docs/architecture/EXTENSION_MODEL.md
docs/architecture/RUNTIME_AND_SAFETY_MODEL.md
docs/architecture/POC_PLAN.md
docs/architecture/OPEN_QUESTIONS.md

docs/adr/0001-runtime-ownership-and-domain-boundary.md
docs/adr/0002-central-output-authority.md
docs/adr/0003-bounded-extension-model.md

docs/migration/V1_FEATURE_INVENTORY.md
docs/migration/V1_COMPONENT_MAP.md
docs/migration/V1_TO_LAB_RUNTIME_MAP.md
docs/migration/REUSE_PLAN.md
docs/migration/MIGRATION_RISKS.md
docs/migration/POC_REFINEMENT.md
docs/migration/ARCHITECTURE_FEEDBACK.md
```

Затем:

```powershell
cd D:\rust\lab-runtime

git status
git log --oneline -20
```

Не начинай кодирование, пока не восстановишь в контексте target architecture и migration conclusions.

---

# 2. Основной принцип этапа

`com_port_reader` является donor.

Он НЕ является dependency нового проекта.

Запрещено:

```text
path dependency на com_port_reader
copy whole modules
copy ApplicationRuntime
copy SeriesStore
copy AcquisitionSource hierarchy
copy OutputControl как новый arbiter
copy current Lua API
copy GUI ownership
```

Допустимо переносить:

```text
чистую математику
protocol vectors
validation knowledge
test cases
domain semantics
```

с явным указанием provenance.

Milestone 1 вообще не должен требовать production-кода из v1.

---

# 3. Сначала проверить donor baseline

Старый repository:

```text
D:\rust\com_port_reader
```

использовать read-only.

Сначала:

```powershell
git -C D:\rust\com_port_reader status
git -C D:\rust\com_port_reader rev-parse HEAD
git -C D:\rust\com_port_reader describe --tags --always
```

Зафиксируй фактический commit.

Затем выполни:

```powershell
cd D:\rust\com_port_reader
cargo test
```

Если test suite проходит, зафиксируй это.

Если есть failures:

* не исправляй их;
* не меняй donor repository;
* сохрани точные failing tests/errors;
* оцени, влияют ли failures на test assets migration analysis.

После проверки снова:

```powershell
git status
```

Tracked files donor repository не должны измениться.

Создай в новом проекте:

```text
docs/migration/DONOR_BASELINE.md
```

Укажи:

```text
branch
commit
tag/version
working tree before
cargo test command
result
failing tests, если есть
working tree after
```

Не называй donor hardware-tested, если hardware tests фактически не выполнялись.

---

# 4. Зафиксировать завершённый migration analysis

Проверь текущие незакоммиченные изменения `lab-runtime`.

Если migration documents и naming fixes ещё не committed, разбей их на логические commits.

Предпочтительно:

```text
docs: normalize lab-runtime project naming

docs: document com_port_reader migration analysis
```

Не смешивай их с будущим Rust implementation.

После этого:

```powershell
git status
```

перед первым implementation commit должен быть чистым.

---

# 5. Reconcile architecture feedback

Migration analysis выявил несколько полезных уточнений.

До написания Rust внеси согласованные уточнения из:

```text
docs/migration/ARCHITECTURE_FEEDBACK.md
```

в основные architecture documents.

Не переписывай архитектуру заново.

Не добавляй новый framework.

Нужно отразить как минимум следующие решения.

---

## 5.1 Furnace

Зафиксировать:

```text
Furnace = один native Controller по внешнему контракту
```

Внутри допустима private composition:

```text
thermal-loss model
    ↓
feed-forward

measurement rate
    ↓
predictor

error
    ↓
PI

sum
    ↓
OutputProposal
```

Reference остаётся отдельным Runtime component.

Не создавать публичный subgraph внутренних частей Furnace.

---

## 5.2 Semantic parameter roles

Instrument descriptor не должен выводить actuator capability только из:

```text
writable == true
numeric == true
```

Явно предусмотреть semantic role:

```text
measurement
configuration
actuator
action
diagnostic
```

или минимальное эквивалентное представление.

Side effects должны быть явной частью capability/operation metadata.

Конкретный Rust representation ещё можно уточнить в Milestone 1 design.

---

## 5.3 Query не выполняет скрытый I/O

Зафиксировать:

```text
Query
    = snapshot / metadata / current known state

Command / Operation
    = может инициировать physical/runtime action
```

Поэтому:

```text
GetLatest
Describe
GetStatus
```

являются Query.

А:

```text
RefreshMeasurement
ReadPhysicalParameterNow
```

если они инициируют I/O, являются Command/Operation.

Milestone 1 virtual implementation должна уже соблюдать эту семантику.

---

## 5.4 Reconfiguration policy

У components должна быть явная политика изменения configuration:

```text
preserve
reset
reinitialize
```

Но Milestone 1 не требует generic framework этой политики.

Нужно лишь не проектировать API так, чтобы state retention происходил случайно.

---

## 5.5 Identity

Различать:

```text
stable logical ID
instance generation
configuration revision
display name
```

Но реализовывать каждое понятие только с момента реальной необходимости.

Для Milestone 1 обязательно:

```text
stable ID
display name
```

Generation вводить только если Milestone 1 действительно имеет replace/recreate asynchronous result problem.

Revision вводить только там, где реально появляется configuration mutation/concurrency.

Не создавать distributed identity framework.

---

# 6. Обновить статус архитектурного этапа

После reconciliation обнови `AGENTS.md`.

Текущий phase больше не:

```text
high-level architecture design
```

Новая фаза:

```text
implementation: milestone 1 domain foundation
```

Сохрани все прежние design priorities и safety rules.

Добавь правило:

> Реализовывать только понятия, необходимые текущему milestone. Наличие concept в target architecture не означает необходимость реализовать его заранее.

---

# 7. Создай implementation design для Milestone 1

До Rust-кода создай:

```text
docs/implementation/MILESTONE_1_DESIGN.md
```

Если каталога нет:

```text
docs/implementation/
```

В документе зафиксируй конкретную минимальную модель.

Milestone 1:

```text
native virtual Instrument
        ↓
typed descriptors
        ↓
validation
        ↓
stable identity
        ↓
deterministic time
        ↓
Signal/latest + bounded window
        ↓
local Commands / Queries
```

---

# 8. Определи минимальные domain types

Нужно спроектировать только действительно необходимые types.

Рассмотри как минимум:

```text
InstrumentId
ParameterId
SignalId

InstrumentDescriptor
ParameterDescriptor

ParameterRole
AccessMode
Value
Unit

Sample
SampleQuality

InstrumentState

Command
CommandResult

Query
QueryResult
```

Но:

> Не превращай этот список автоматически в отдельные traits/files/types.

Если несколько понятий естественно объединяются, объедини их.

---

# 9. IDs

Для Milestone 1 нужен stable identity.

Не используй display name как identity.

Например:

```text
InstrumentId
ParameterId
SignalId
```

должны быть отдельными domain concepts.

Не требуется:

```text
UUID
distributed IDs
database IDs
network-compatible identity scheme
```

если простой runtime-local typed ID достаточно проверяет архитектуру.

Переименование display name не должно менять ID.

---

# 10. Value model

Milestone 1 должен поддержать достаточно типов, чтобы модель не была `f64 everywhere`.

Минимально рассмотри:

```text
Float
Integer
Boolean
Text
Enum
```

Не добавляй сложные structures без use case.

Любой numeric value, предназначенный для дальнейшего signal path:

```text
finite
type-valid
range-valid
```

где это требует descriptor.

Не превращай core в dynamic language runtime.

---

# 11. Units

Units должны присутствовать в descriptor/sample vocabulary.

Но Milestone 1 НЕ требует:

```text
dimensional-analysis engine
automatic unit conversion framework
SI type-level algebra
```

Нужна простая явная domain representation.

Она должна позволять:

```text
temperature -> °C
power -> %
pressure -> Pa
```

и отличать incompatible signals позднее.

---

# 12. ParameterDescriptor

Минимально descriptor должен отвечать:

```text
Как называется параметр?
Каков его stable ID?
Какого он типа?
Какая единица?
Можно ли его наблюдать?
Можно ли менять configuration?
Является ли он actuator?
Есть ли диапазон?
Какова его semantic role?
```

Пример conceptual:

```text
temperature
  type: float
  unit: °C
  role: measurement
  readable: yes
  writable: no

heater_power
  type: float
  unit: %
  role: actuator
  readable: yes
  writable: yes
  range: 0..100
```

Не использовать эвристику:

```text
numeric + writable = actuator
```

---

# 13. Native virtual Instrument

Реализуй один минимальный native virtual instrument.

Это не Lua emulator.

Это не serial protocol.

Это обычная Rust implementation domain Instrument contract.

Пример:

```text
VirtualHeater
```

с параметрами:

```text
temperature
heater_power
```

Но на Milestone 1:

```text
temperature
```

может генерировать measurement.

`heater_power` может присутствовать в descriptor как actuator capability, но физическая/output mutation через него пока НЕ активируется.

Output execution начинается только в Milestone 2 после OutputArbiter.

Если actuator descriptor усложняет Milestone 1 без пользы, допустимо иметь его metadata без executable write path.

---

# 14. Virtual model scope

Не реализовывай полноценную thermal plant модель старой печи.

Достаточно deterministic модели, например:

```text
temperature = function(time)
```

или небольшого stateful generator.

Главное проверить:

```text
Instrument
Descriptor
Sample
Signal
Clock
Command/Query boundary
```

а не термодинамику.

Сложную furnace simulation позже можно перенести как fixture.

---

# 15. Clock

Milestone 1 должен быть полностью deterministic в tests.

Не использовать:

```text
SystemTime::now()
Instant::now()
sleep()
```

в domain tests как единственный источник времени.

Выбери минимальную clock boundary.

Это может быть:

```text
явно передаваемое monotonic timestamp
```

или небольшой injected clock abstraction.

Не создавай abstraction framework ради одного вызова.

Wall-clock time в Milestone 1 необязателен, если recorder ещё отсутствует.

Основная цель:

```text
tests сами контролируют течение времени
```

---

# 16. Sample model

Sample должен содержать минимум:

```text
SignalId
value
unit/type context или гарантию через Signal descriptor
monotonic timestamp
quality
```

Рассмотри sequence только если он реально нужен уже для ordering/bounded window tests.

Не добавляй пока:

```text
network cursor
request ID
deduplication
external correlation
```

---

# 17. SampleQuality

Нужно различать как минимум:

```text
good measurement
unavailable / failed measurement
```

Не кодировать отказ как:

```text
0.0
NaN
magic sentinel
```

Точный enum выбери минимальный.

Не проектируй сейчас полноценную industrial OPC quality taxonomy.

---

# 18. Signal state

Не переносить v1 `SeriesStore`.

Для Milestone 1 достаточно:

```text
latest sample
+
bounded recent window
```

если окно действительно нужно для acceptance test.

Memory bound должен быть явным.

Например:

```text
max N samples
```

или другой простой policy.

Не допускается:

```text
Vec forever
```

Durable history появится только с recorder.

GUI history сейчас отсутствует.

---

# 19. Instrument state

Runtime/Core должен владеть:

```text
registered instruments
their descriptors
current observed parameter state
signals/latest windows
```

Virtual Instrument implementation не должна становиться владельцем experiment lifecycle.

Клиент не получает mutable references на runtime state.

---

# 20. Command / Query boundary

Milestone 1 должен иметь локальную in-process façade.

Не networking.

Не JSON.

Не serde wire contract.

Пример semantics:

```text
Query:
    DiscoverInstruments
    DescribeInstrument
    GetLatestSignal
    GetInstrumentState

Command:
    RenameInstrument
    RefreshMeasurement
```

Это conceptual examples.

Выбери минимальный набор, нужный acceptance tests.

Главные правила:

```text
Query не меняет state
Query не инициирует скрытый I/O

Command может изменить state
Command возвращает явный outcome
```

---

# 21. Не создавай external API

Milestone 1 НЕ включает:

```text
TCP
NDJSON
named pipes
WebSocket
HTTP
Babashka client
GUI client
authentication
subscriptions
cursor
network request IDs
deduplication
```

Local Commands/Queries нужны только для проверки будущей общей boundary.

---

# 22. Минимальная структура workspace

Только после `MILESTONE_1_DESIGN.md` создай Cargo workspace.

Предпочтительная минимальная структура:

```text
lab-runtime/
│
├── Cargo.toml
│
├── crates/
│   └── lab-core/
│       ├── Cargo.toml
│       └── src/
│
└── apps/
    └── lab-runtime/
        ├── Cargo.toml
        └── src/
```

Но не следуй структуре механически.

Если analysis показывает более простой layout с теми же dependency boundaries, используй его и объясни.

Обязательны только две логические единицы:

```text
lab-core
lab-runtime host executable
```

Не создавать сейчас:

```text
lab-lua
lab-gui
lab-ipc
lab-serial
lab-protocol
lab-recorder
```

Это пока modules/будущие milestones, а не повод открыть зоопарк crates.

---

# 23. Dependency direction

Должно быть:

```text
lab-runtime
    ↓
lab-core
```

`lab-core` не зависит от:

```text
egui
mlua
serialport
rusqlite
tokio
network stack
Windows APIs
```

`lab-core` желательно держать максимально лёгким.

Не добавляй dependency только потому, что она «потом наверняка понадобится».

---

# 24. Async runtime

Не добавляй Tokio или другой async runtime в Milestone 1 без конкретной необходимости.

Milestone 1 является in-process deterministic domain POC.

Синхронная реализация предпочтительна, если полностью решает задачу.

Будущий transport concurrency не является причиной внедрять executor сейчас.

---

# 25. Error model

Ошибки должны быть typed и domain-specific настолько, насколько это помогает caller.

Различать минимум:

```text
unknown instrument
unknown parameter
wrong type
read-only / operation not allowed
out of range
invalid configuration
measurement unavailable
```

Не делай один:

```text
String error
```

Но и не строй generic error taxonomy всей будущей системы.

---

# 26. Tests first

Перед основным implementation напиши acceptance tests Milestone 1.

Как минимум:

### Test 1. Instrument discovery

Зарегистрированный virtual instrument появляется в discovery.

Generic code получает descriptor без knowledge конкретного Rust type.

### Test 2. Descriptor

Descriptor возвращает:

```text
stable IDs
names
types
units
access
semantic roles
ranges
```

### Test 3. Rename

Переименование instrument:

```text
изменяет display name
не изменяет InstrumentId
не ломает parameter/signal identity
```

### Test 4. Invalid configuration

Неверный type/range/access:

```text
возвращает typed error
не изменяет прежнее valid state
```

### Test 5. Query purity

Query:

```text
не двигает virtual time
не создаёт новый measurement
не меняет state
```

### Test 6. Explicit measurement refresh/tick

Явная operation создаёт новый measurement.

Контролируемый clock задаёт timestamp.

### Test 7. Measurement failure

Failure/unavailable measurement:

```text
не превращается в 0
не превращается в старое значение с новым timestamp
имеет явный quality/error outcome
```

### Test 8. Bounded signal memory

После количества samples больше configured bound:

```text
latest корректен
window не превышает bound
старые samples удаляются ожидаемо
```

### Test 9. Stable deterministic run

Одинаковая последовательность:

```text
commands
clock advances
```

даёт одинаковый результат.

---

# 27. Дополнительные useful tests

Если они естественно следуют из модели, добавь:

```text
duplicate ID rejection
duplicate display name policy
non-finite float rejection
invalid unit/range combination
parameter lookup by stable ID
remove impact if removal already implemented
```

Но не расширяй scope ради количества тестов.

---

# 28. Что нельзя переносить из v1 в Milestone 1

Не использовать:

```text
SeriesStore
ApplicationRuntime
AcquisitionSource
CombinedSource
SerialCommandSource
OutputControl
ControllerRegistry
LuaRuntime
CoreHost JSON DTO
ProcessRecorder
egui models
```

Можно использовать v1 только как reference для:

```text
stable rename behavior
typed value validation
descriptor ideas
atomic validation expectations
```

Если конкретный test intent переносится, перепиши его под новую domain model.

---

# 29. Provenance

Для явно перенесённых algorithms/test vectors позднее потребуется source provenance.

Milestone 1 почти не должен копировать v1 production body.

Но если тест или validation rule явно основан на v1, добавь короткий комментарий или design note:

``
