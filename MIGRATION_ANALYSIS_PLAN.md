Ты продолжаешь работу над greenfield-проектом `lab-runtime`.

Первый высокоуровневый архитектурный этап завершён.

Архитектура `lab-runtime` теперь является исходной точкой для дальнейшей работы.

На этом этапе тебе впервые разрешается изучить существующий рабочий проект:

```text
D:\rust\com_port_reader
```

Его роль:

```text
стабильная v1
источник проверенного поведения
источник рабочих алгоритмов
источник реальных требований
донор отдельных реализаций
```

Но:

> `com_port_reader` НЕ является архитектурным шаблоном для `lab-runtime`.

Твоя задача сейчас не переносить код.

Твоя задача:

> сопоставить утверждённую архитектуру `lab-runtime` с реальной реализацией `com_port_reader`, определить ценность каждого существующего компонента и подготовить точный план будущего переноса.

Production-код `lab-runtime` пока не писать.

---

# 0. Сначала прочитай архитектуру `lab-runtime`

Работай из:

```text
D:\rust\lab-runtime
```

Полностью прочитай:

```text
AGENTS.md
PROJECT_BRIEF.md
ARCHITECTURE_PLAN.md

docs/architecture/HIGH_LEVEL_ARCHITECTURE.md
docs/architecture/EXTENSION_MODEL.md
docs/architecture/RUNTIME_AND_SAFETY_MODEL.md
docs/architecture/POC_PLAN.md
docs/architecture/OPEN_QUESTIONS.md

docs/adr/
```

Проверь:

```powershell
git status
git log --oneline -15
```

Не начинай читать v1, пока не восстановишь в контексте целевую архитектуру.

---

# 1. Небольшая правка naming

Проект называется:

```text
lab-runtime
```

а не:

```text
lab-runtime-v2
Lab Runtime v2
```

Старый проект можно называть:

```text
com_port_reader v1
```

если это удобно для сравнения поколений.

Перед основным анализом найди оставшиеся упоминания старого названия в документации `lab-runtime`.

Например:

```powershell
rg "Lab Runtime v2|lab-runtime-v2|design v2|архитектура v2" .
```

Исправь только очевидные naming leftovers.

Не меняй смысл архитектурных документов.

Если эти изменения образуют отдельный логический шаг, сделай отдельный commit:

```text
docs: normalize lab-runtime project naming
```

---

# 2. Затем изучи `com_port_reader`

Старый репозиторий нельзя изменять.

Используй его только как read-only source.

Начни с:

```powershell
git -C D:\rust\com_port_reader status
git -C D:\rust\com_port_reader log --oneline -30
git -C D:\rust\com_port_reader tag
```

Получить список файлов:

```powershell
rg --files D:\rust\com_port_reader
```

Изучи:

```text
Cargo.toml
src/
tests, если выделены отдельно
profiles/
lua scripts
emulator scripts
documentation
examples
```

Если baseline текущей v1 не зафиксирован в документации, запиши:

```text
commit
release tag
Cargo version
```

которые были проанализированы.

Если рабочее дерево v1 грязное, ничего там не исправляй и явно укажи это в отчёте.

---

# 3. Не оценивай старый код только по именам файлов

Не делай вывод:

```text
старый SeriesStore
    =
новый Series
```

лишь из-за похожего имени.

Для каждого значимого компонента выясни:

```text
какую задачу он реально решает

какое состояние хранит

кто владеет этим состоянием

какие ошибки обрабатывает

с какими подсистемами связан

какие concurrency assumptions содержит

какие инварианты защищают тесты

какое user-visible поведение от него зависит
```

Нас интересует семантика, а не файловая структура.

---

# 4. Построй полный функциональный inventory v1

Сначала отдельно от архитектурного mapping составь список реально работающих возможностей `com_port_reader`.

Не основывайся только на README.

Проверь код, tests, Lua API и профили.

Как минимум рассмотри:

## Acquisition

```text
workers
polling
poll intervals
scheduling
retry
failure suspension
series history
worker events
shutdown
```

## Physical I/O

```text
SerialConnection
serial configuration
port lifecycle
raw serial commands
RS-485 / relevant protocol assumptions
```

## Instruments

```text
Metakon
virtual instruments
instrument descriptors/parameters
reads
writes
fault handling
```

## Emulator

```text
memory transport
serial emulator mode
virtual instrument protocol
Lua model
start/stop/restart
```

## Signal processing

```text
all current filters
filter configuration
derived series
failure behavior
```

## Process control

```text
PID
On/Off
Furnace
Reference model if present
diagnostics
controller registry
control loops
```

## Output control

```text
Manual
AutomaticPending
Automatic

ownership
safe output
pause/resume
rollback
tracked writes
manual overrides
```

## Recording

```text
SQLite
session lifecycle
measurements
actions
logs
failure policy
```

## Lua

```text
embedded runtime
public app API
instrument userdata
series
filters
controllers
emulator
UI/control panels
profiles/setup
```

## GUI

```text
plots
series controls
instrument controls
controller controls
process recording
help
configuration/profile UI
```

Не ограничивай inventory этим списком.

---

# 5. Создай feature-parity map

Создай:

```text
docs/migration/V1_FEATURE_INVENTORY.md
```

Для каждой пользовательской возможности укажи:

```text
feature
current implementation
dependencies
important behavior
important tests
target lab-runtime subsystem
migration priority
```

Пример структуры:

```markdown
| v1 feature | Current implementation | Required behavior | lab-runtime target | Priority |
|---|---|---|---|---|
| Periodic instrument polling | ... | ... | Runtime acquisition scheduler | High |
```

Цель документа:

> не потерять работающую функциональность при создании новой архитектуры.

Это НЕ означает, что каждая внутренняя абстракция v1 должна сохраниться.

---

# 6. Центральный component map

Создай:

```text
docs/migration/V1_COMPONENT_MAP.md
```

Сопоставь компоненты `com_port_reader` с новой architecture.

Для каждого значимого компонента используй одну из категорий:

```text
REUSE
EXTRACT
ADAPT
REWRITE
DISCARD
DEFER
```

Значения:

## REUSE

Алгоритм или модуль уже соответствует новой границе и может быть перенесён почти без изменений.

## EXTRACT

Код хороший, но сейчас находится внутри неправильного владельца/модуля.

Нужно отделить его от существующего окружения.

## ADAPT

Основная реализация ценна, но API/state ownership/lifecycle нужно изменить под `lab-runtime`.

## REWRITE

Поведение важно сохранить, но текущая реализация слишком связана со старой архитектурой.

## DISCARD

Исторический glue или abstraction, который новой системе не нужен.

## DEFER

Полезно, но не относится к первому POC или ближайшему vertical slice.

---

# 7. Обязательные компоненты для анализа

Особенно внимательно разбери:

```text
SerialConnection

SerialConfigStore

AcquisitionSource

CombinedSource

SerialCommandSource

worker runtime

poll scheduling

SeriesStore

instrument requests / values

Metakon protocol/driver

virtual instrument protocol

VirtualInstrumentClient

VirtualInstrumentServer

memory transport

DeviceEmulator

DeviceEmulatorService

signal processing service

all filters

PID controller

OnOff controller

Furnace controller

ControlLoop

controller registry

Reference implementation

OutputControl

OutputTarget

process recorder

SQLite schema/storage

Lua runtime

Lua API

profile loader

application runtime

GUI application/state

plot preparation/downsampling
```

Если названия в текущем коде отличаются, найди соответствующие реальные компоненты.

---

# 8. Для каждого компонента зафиксируй реальную ценность

Не пиши только:

```text
PID -> REUSE
```

Нужно указать:

```text
что конкретно можно перенести
что нельзя переносить
какие зависимости убрать
какие assumptions изменить
какие tests являются ценными
```

Пример conceptual:

```text
PID algorithm:
classification: EXTRACT

Reuse:
- derivative-on-measurement algorithm
- anti-windup
- diagnostics
- dt handling

Do not reuse:
- current registry ownership
- Lua-specific parameter dispatch
- direct assumptions about SeriesStore

Target:
lab-core control native PID
```

---

# 9. Tests являются отдельным активом

Отдельно анализируй тесты v1.

Очень часто тест содержит более ценное знание о системе, чем сам старый interface.

Для каждого subsystem отметь:

```text
tests reusable almost unchanged

tests expressing important behavior but requiring new harness

tests tied only to old architecture

missing tests discovered during analysis
```

Особенно ценны tests, защищающие:

```text
polling drift

retry/suspension

controller math

anti-windup

pause/resume

safe output

rollback

instrument fault mapping

Lua validation

emulator framing

recording failure
```

---

# 10. Transport / Protocol / Instrument mapping

Проверь, насколько текущий `com_port_reader` уже разделяет:

```text
Transport
Protocol
Instrument
```

Выясни, где сейчас смешаны:

```text
serial ownership

protocol framing

device semantics

instrument parameters

polling

retry

application routing
```

Предложи минимальный путь извлечения полезного кода.

Не подгоняй старый код искусственно под новые названия.

---

# 11. Data-driven instruments

Проверь текущие instrument/parameter descriptors и Metakon implementation.

Ответь:

```text
какие metadata уже существуют?

можно ли из них получить будущий InstrumentDescriptor?

что сейчас hardcoded?

что уже data-driven?

какие части Metakon являются generic protocol knowledge?

какие являются device-specific semantics?
```

Особенно оцени возможность будущей схемы:

```text
known protocol primitive
        +
data definition
        =
new instrument without Core rebuild
```

Не реализуй её.

---

# 12. Lua instrument extension

Сравни текущий Lua virtual-instrument/emulator механизм с новой моделью bounded Lua host.

Нужно определить:

```text
что можно сохранить

какой Lua model format полезен

какие callbacks можно переиспользовать

что слишком тесно связано с current app API

есть ли raw/direct access, который новой архитектуре нельзя сохранять
```

Особенно сравни с будущим правилом:

```text
Lua does not own COM

Lua uses bounded named operations

physical writes cannot bypass OutputArbiter
```

---

# 13. Lua filters/controllers

Проверь, есть ли в v1 script-backed:

```text
filters
controllers
references
```

или достаточная инфраструктура для них.

Если нет, не считай это недостатком v1.

Определи только:

```text
какие существующие Lua host mechanisms можно использовать

что придётся спроектировать заново
```

---

# 14. Babashka

`com_port_reader` не требуется иметь Babashka support.

На этом этапе Babashka-код не писать.

Но после изучения реальной v1 оцени:

```text
какая domain information необходима внешнему клиенту

какие текущие operations уже естественно становятся Command/Query

какие события нужны

какие current app operations слишком UI-specific
```

Особенно проверь, насколько легко внешний клиент сможет:

```text
discover instruments

describe parameters

add a series

configure a filter

create native PID

configure Reference

inspect diagnostics

start recording

observe experiment state
```

---

# 15. Signal/Series mapping

Сравни current `SeriesStore`, sampling/history и filter system с новой моделью:

```text
Signal
current sample
bounded Series window
durable history
SignalNode
```

Определи, где v1 смешивает эти роли.

Не переносить автоматически unlimited-history design в runtime memory model.

Выясни:

```text
что требуется фильтрам

что требуется GUI

что требуется recorder

что требуется controller
```

---

# 16. Reference model

Разбери текущую reference implementation.

Особенно выясни:

```text
fixed reference

ramp

pause/resume

time semantics

controller coupling
```

Новая архитектура предполагает Reference как самостоятельный Runtime component.

Определи, насколько current implementation можно:

```text
REUSE
EXTRACT
ADAPT
REWRITE
```

---

# 17. Native controllers

Для:

```text
PID
OnOff
Furnace
```

составь отдельную таблицу.

Для каждого:

```text
algorithm state

configuration

parameter knowledge

inputs

outputs

diagnostics

timing assumptions

reset behavior

pause/resume behavior

dependencies on registry/store/Lua
```

Нужно отдельно классифицировать:

```text
algorithm

runtime wrapper

configuration API

Lua exposure
```

Один controller может получить разные категории для разных частей.

---

# 18. Furnace controller

Это отдельный важный пункт.

В архитектурном review `Furnace` сознательно оставлен открытым вопросом до анализа v1.

Теперь ответь:

```text
что Furnace реально делает

является ли он самостоятельным controller

или композицией:
    model
    feed-forward
    predictor
    PI/PID
    Reference

какое внутреннее состояние ему нужно

какие diagnostics нужны

что требуется для переноса
```

Не меняй архитектуру только ради совпадения с current Furnace code.

Если более чистая композиция новой системы сохраняет поведение, укажи это как вариант.

---

# 19. OutputControl vs новый OutputArbiter

Это один из самых важных разделов.

Разбери current:

```text
Manual
AutomaticPending
Automatic
```

и весь lifecycle:

```text
ownership

controller start

first automatic write

pause

resume

manual write

rollback

safe output

write failure

controller removal

shutdown
```

Сравни с target:

```text
Unverified
SafePending
Disarmed
ArmedManual
ArmedAuto
FaultLatched
```

Не делай вывод, что новая state machine должна быть переписана под старую.

Вместо этого создай semantic mapping:

```text
какой старый invariant должен сохраниться

какое новое состояние его представляет

какие новые guarantees отсутствовали в v1

какое old behavior больше не нужно
```

Особенно найди reusable tests для:

```text
manual override

automatic pending

safe output

pause/resume

rollback

failed writes
```

---

# 20. Safety implementation staging

Целевая safety architecture остаётся действующей.

Но первый POC не обязан реализовывать все её понятия одновременно.

Предложи минимальный progression.

Например:

```text
POC foundation:
    owner
    generation
    Disarmed / Armed / Fault
    safe action

later:
    lease
    richer evidence
    ambiguous write reconciliation
    detailed recovery
```

Не отменяй архитектурные semantics.

Раздели:

```text
target invariant
```

и:

```text
earliest implementation milestone
```

чтобы POC не превратился в industrial safety framework до доказательства основных extension boundaries.

---

# 21. Recorder

Разбери текущий SQLite recorder.

Определи:

```text
schema

session model

measurement representation

actions/events

logs

failure handling

threading/buffering

shutdown/flush

current coupling to UI/runtime
```

Классифицируй отдельно:

```text
domain recording semantics

storage implementation

schema

runtime service wrapper
```

Не предполагай автоматически сохранение SQLite.

Но если текущая реализация хорошо подходит новой architecture, укажи это.

---

# 22. Emulator и virtual instrument

Разбери текущую систему максимально внимательно.

Особенно:

```text
VirtualInstrumentServer

VirtualInstrumentClient

frame protocol

Lua device model

memory transport

serial integration mode

DeviceEmulator lifecycle
```

Новая architecture требует virtual instrument как полноценной Instrument implementation.

Определи:

```text
какие части являются полезным generic emulator engine

какие являются transport compatibility layer

какие являются Lua model host

что стоит перенести
```

---

# 23. ApplicationRuntime / worker architecture

Необходимо понять, какие проблемы current runtime уже решил хорошо.

Изучи:

```text
thread ownership

worker command channels

event channels

shutdown

sample ownership

poll scheduling

profile reload

controller execution

UI repaint/event handling
```

Но не пытайся сохранить current `ApplicationRuntime` как будущий Runtime.

Ищи:

```text
algorithms

failure semantics

tested lifecycle patterns

useful synchronization decisions
```

а не class structure.

---

# 24. GUI

GUI не переносится в первый POC.

Но current GUI является источником требований.

Определи:

```text
какую domain information он реально использует

какие views generic

какие views instrument-specific

какие controls можно построить из descriptors

какие требуют custom presentation
```

Отдельно оцени:

```text
plotting/downsampling
help system
control panels
profile UI
```

Классифицируй их как:

```text
future GUI reusable code

future GUI requirement

obsolete v1-specific glue
```

---

# 25. Lua API

Не переносить current Lua API один-в-один.

Составь mapping:

```text
current Lua capability
        ↓
future owner
```

Возможные owners:

```text
bounded Lua component host

Babashka/external Commands/Queries/Events API

GUI

Rust Runtime configuration

removed/obsolete
```

Например:

```text
embedded filter callback
    -> Lua host

experiment scenario
    -> Babashka

controller lifecycle command
    -> common external domain API

direct output write
    -> OutputArbiter command
```

Создай явную таблицу.

---

# 26. Создай `V1_TO_LAB_RUNTIME_MAP.md`

Файл:

```text
docs/migration/V1_TO_LAB_RUNTIME_MAP.md
```

Он должен показывать новую систему сверху вниз:

```text
lab-runtime component
    ↓
relevant v1 implementation
    ↓
migration classification
```

Например:

```markdown
## Transport executor

Relevant v1:
- ...
- ...

Classification:
ADAPT

Reuse:
- ...

Rewrite:
- ...

Tests:
- ...
```

Это основной документ будущего переноса.

---

# 27. Создай `REUSE_PLAN.md`

Файл:

```text
docs/migration/REUSE_PLAN.md
```

Раздели всё ценное из v1 на группы.

## Direct algorithm reuse

Например, если анализ подтвердит:

```text
PID mathematics
OnOff mathematics
protocol CRC
downsampling
```

## Extract after removing dependencies

## Adapt to new domain contracts

## Rewrite behavior from tests/specification

## Do not migrate

## Defer until GUI/full product stage

Для каждого пункта объясни почему.

---

# 28. Создай `MIGRATION_RISKS.md`

Файл:

```text
docs/migration/MIGRATION_RISKS.md
```

Особенно ищи:

```text
behavior hidden only in GUI

behavior hidden only in Lua bindings

implicit state ownership

unsafe coupling between writes and instrument transport

tests that depend on old architecture

features not represented in new descriptors

v1 behavior contradicting target architecture

high-value code with heavy UI dependencies

protocol code with hidden serial assumptions
```

Каждый риск:

```text
risk

impact

evidence from v1

recommended handling
```

---

# 29. Уточни POC после анализа v1

Создай:

```text
docs/migration/POC_REFINEMENT.md
```

Не переписывай оригинальный `POC_PLAN.md` молча.

Сначала предложи изменения отдельно.

Нужно определить:

```text
какой код реально стоит переносить уже в POC

какие current tests можно использовать

какие POC stages нужно изменить
```

---

# 30. Изменение порядка POC: рассмотри ранний Babashka slice

Текущий architecture plan откладывает настоящий Babashka/IPC slice после первого полного POC.

Нужно критически пересмотреть это после анализа v1.

Предпочтительный порядок для обсуждения:

```text
1. Domain + virtual instrument

2. Minimal OutputArbiter

3. Transport/protocol/data-driven instrument

4. Signal + Reference + native PID

5. Bounded Lua extensions

6. SMALL Babashka / external API vertical slice

7. Recorder

8. Runtime failure/load tests

9. Longer soak/release hardening
```

Причина:

Babashka/REPL является одной из главных продуктовых целей `lab-runtime`.

Нужно сравнительно рано проверить, удобна ли domain boundary для интерактивной Clojure работы.

Не утверждай этот порядок автоматически.

Сравни его с реальным v1 и новой architecture и дай рекомендацию.

---

# 31. Soak tests

Не требуй 24/72-часовые тесты для самого раннего архитектурного POC.

Раздели:

## Architecture POC

```text
fake-clock stress
fault injection
30–60 minute real-time integration run
memory/task/queue leak inspection
```

## Runtime candidate / pre-release

```text
24 h
72 h
```

Если после анализа v1 есть причины сохранить иной порядок, обоснуй.

---

# 32. Какие архитектурные понятия реализовывать когда

Создай небольшую таблицу:

```text
concept
why it exists
first milestone where required
```

Для:

```text
ID
generation
revision
epoch
sequence
cursor
request ID
correlation ID
lease
TTL
evidence
snapshot
deduplication
```

Правило:

> наличие понятия в target architecture не означает, что оно должно быть реализовано в первом commit.

Особенно избегай premature implementation:

```text
cursor before real subscriptions

request deduplication before IPC

full evidence model before physical output tests
```

---

# 33. Не менять architecture без явного обоснования

Если реальная v1 показывает проблему в утверждённой architecture:

не подгоняй её молча.

Запиши:

```text
Architecture issue

Evidence from v1

Current proposed model

Why it is insufficient

Recommended change
```

в:

```text
docs/migration/ARCHITECTURE_FEEDBACK.md
```

Если серьёзных замечаний нет, всё равно создай короткий файл и напиши, что mapping не выявил необходимости менять high-level architecture.

Не редактируй ADR молча.

---

# 34. Не писать production code

На этом этапе запрещено создавать:

```text
Cargo.toml

Rust workspace

src/

Rust implementation

Lua implementation

Babashka implementation

IPC server

GUI
```

Разрешены только:

```text
documentation
migration analysis
architecture feedback
```

В `com_port_reader` не вносить вообще никаких изменений.

---

# 35. Git

Изменения делаются только в:

```text
D:\rust\lab-runtime
```

Рекомендуемая структура после анализа:

```text
docs/
├── architecture/
├── adr/
└── migration/
    ├── V1_FEATURE_INVENTORY.md
    ├── V1_COMPONENT_MAP.md
    ├── V1_TO_LAB_RUNTIME_MAP.md
    ├── REUSE_PLAN.md
    ├── MIGRATION_RISKS.md
    ├── POC_REFINEMENT.md
    └── ARCHITECTURE_FEEDBACK.md
```

Проверь:

```powershell
git diff --check
git status
```

Если разрешено делать commits, предпочти несколько логических commits.

Например:

```text
docs: normalize lab-runtime project naming

docs: inventory com_port_reader v1 functionality

docs: map v1 components to lab-runtime

docs: refine migration and poc strategy
```

Не складывай весь анализ в один гигантский commit, если он естественно делится.

---

# 36. Финальная проверка mapping

До завершения убедись, что можешь конкретно ответить:

```text
Какие части SerialConnection можно использовать?

Что станет transport executor?

Что останется от AcquisitionSource?

Нужен ли CombinedSource в новой architecture?

Что переносится из worker scheduling?

Как переносится Metakon?

Что можно использовать для data-driven instruments?

Что переносится из virtual instrument/emulator?

Какие filters можно перенести?

Какая часть PID переиспользуется?

Какая часть OnOff переиспользуется?

Furnace является controller или композиционным компонентом?

Как current Reference отображается на новую Reference model?

Что сохраняется из OutputControl?

Какие old OutputControl semantics должен поддержать новый OutputArbiter?

Можно ли переиспользовать SQLite recorder?

Что останется от current Lua runtime?

Какая часть Lua API исчезает или переходит в Babashka?

Что GUI требует от будущего domain API?

Какие v1 tests обязательно перенести?

Какие крупные части v1 вообще не нужны новому Runtime?
```

Если на значимый вопрос нет ответа, анализ ещё не закончен.

---

# 37. Итоговый отчёт пользователю

В конце дай отчёт на русском.

Структура:

## 1. Общий вывод

Насколько хорошо `com_port_reader` ложится на новую architecture.

## 2. Самые ценные части v1

5–15 компонентов/алгоритмов, которые действительно стоит сохранить.

## 3. Что придётся переписать

Крупные области и причины.

## 4. Что вообще не стоит переносить

Historical glue / obsolete structure.

## 5. Architecture feedback

Нужно ли менять утверждённую architecture после знакомства с реальным кодом.

## 6. POC refinement

Как должен измениться первоначальный POC.

## 7. Первый будущий implementation milestone

Только описание.

Production-код пока не создавать.

## 8. Открытые вопросы

Только вопросы, которые действительно нельзя решить из architecture + v1.

---

# Definition of done

Этап завершён, когда:

```text
полностью изучена релевантная v1

создан feature inventory

каждый крупный v1 component классифицирован

определён target subsystem каждого ценного компонента

определено, какие tests являются knowledge assets

Metakon mapping понятен

serial/acquisition mapping понятен

signal/filter mapping понятен

PID/OnOff/Furnace mapping понятен

Reference mapping понятен

OutputControl → OutputArbiter mapping понятен

Lua mapping понятен

emulator mapping понятен

recorder mapping понятен

GUI requirements извлечены

предложен уточнённый POC

архитектурные замечания записаны отдельно

production code не написан

старый repository не изменён
```

После этого остановись.

Не создавай Cargo workspace и не начинай перенос кода без отдельной команды пользователя.
