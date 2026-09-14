Ты работаешь над новым greenfield-проектом `lab-runtime`.

Это не рефакторинг существующего приложения.

На первом этапе твоя задача состоит исключительно в проектировании архитектуры самого высокого уровня.

## Перед началом

Прочитай полностью:

```text
AGENTS.md
PROJECT_BRIEF.md
ARCHITECTURE_PLAN.md
```

Затем:

```powershell
git status
git log --oneline -10
```

На этом этапе НЕ изучай исходный код старого `com_port_reader`, даже если он доступен.

Это намеренно.

Сначала требуется спроектировать архитектуру исходя из реальных требований продукта, не позволяя исторической структуре `com_port_reader` заранее навязать решения `lab-runtime`.

Позже будет отдельный этап сопоставления архитектуры `lab-runtime` с реализацией `com_port_reader`.

---

# Главная задача

Спроектируй высокоуровневую архитектуру универсального laboratory automation runtime.

Система должна объединить:

```text
Rust Core
native Rust extensions
data-driven instruments
embedded Lua extensions
external Babashka/Clojure orchestration
signal processing
controllers
output safety
recording
GUI clients
future remote clients
```

Но не исходи из предположения, что все перечисленные механизмы обязательно нужны.

Критически оцени требования.

Если какой-либо proposed mechanism создаёт больше сложности, чем пользы, предложи более простое решение.

---

# 1. Начни с domain model

Сначала ответь:

> Какие главные сущности реально существуют в этой системе?

Рассмотри как минимум:

```text
Runtime

Transport
Protocol

Instrument
InstrumentDescriptor
Parameter

Signal
Series

SignalNode
Source
Transform
Filter

Reference

Controller
OutputProposal
OutputArbiter

Recorder

Command
Query
Event

Extension
Client
```

Не превращай автоматически каждое существительное в Rust trait.

Для каждой сущности объясни:

```text
зачем она существует
кто владеет её состоянием
какие соседние сущности о ней знают
является ли она domain concept или implementation detail
```

Если некоторые предложенные сущности лишние, удали или объедини их.

---

# 2. Определи границу Rust Core

Нужно ясно определить:

```text
что входит в Rust Core
что находится вокруг Core
```

Особенно рассмотри:

```text
physical I/O
protocols
instrument drivers
scheduler
signals
filters
controllers
references
output safety
recorder
Lua runtime
IPC
GUI
```

Core не должен зависеть от GUI.

Критически оцени, должен ли Core напрямую зависеть от Lua runtime или Lua лучше оформить отдельным adapter/extension layer.

---

# 3. Спроектируй Instrument model

Нужно поддержать как минимум:

## Native Rust instrument

Полноценный Rust driver.

## Data-driven instrument

Прибор на уже известном протоколе, описанный данными.

Например Modbus device с набором registers.

## Embedded-script instrument

Небольшой драйвер/adapter на Lua.

Например прибор, для которого нужны только:

```text
read temperature
write power
```

## Virtual instrument

Программная модель без физического hardware.

Все варианты должны по возможности выглядеть одинаково для остальной системы.

Определи:

```text
общую instrument abstraction
descriptor model
parameter model
capability model
```

Рассмотри, как GUI и Babashka смогут introspect неизвестный заранее прибор.

---

# 4. Transport и Protocol

Критически оцени модель:

```text
Transport
    ↓
Protocol
    ↓
Instrument
```

Определи, где должны находиться:

```text
port ownership
timeouts
serialization of transactions
retry
framing
CRC
protocol parsing
instrument semantics
```

Особенно важно:

script-backed instrument не должен произвольно ломать совместное использование транспорта.

Rust Runtime должен оставаться владельцем COM/RS-485 resource и transaction scheduling.

---

# 5. Signal processing model

Спроектируй высокоуровневую модель обработки сигналов.

Рассмотри композицию:

```text
Source
  ↓
Transform
  ↓
Filter
  ↓
Controller
  ↓
Sink
```

Определи:

* является ли граф подходящей моделью;
* как представлять series/history;
* как отделить current value от recorded history;
* как добавлять filters;
* как публиковать diagnostics.

Система должна поддерживать native Rust и script-based processing.

---

# 6. Reference model

Постепенное изменение setpoint не должно обязательно быть частью PID.

Рассмотри отдельную модель Reference:

```text
Fixed
Ramp
Program
Script
```

Пример:

```text
RampReference
     ↓
     PID
     ↓
OutputProposal
```

Определи, является ли это хорошей базовой абстракцией.

---

# 7. Controller model

Спроектируй controller boundary.

Native controllers:

```text
PID
On/Off
Furnace
```

Future extensions:

```text
Lua controller
external/Babashka controller
future compiled plugin
```

Контроллер не должен писать в hardware напрямую.

Рассмотри модель:

```text
Controller
     ↓
OutputProposal
     ↓
OutputArbiter
     ↓
Instrument
```

Определи lifecycle controller:

```text
creation
configuration
running
pause
resume
reset
removal
failure
```

---

# 8. Output safety

Это одна из главных частей архитектуры.

Спроектируй единый safety/output boundary.

Он потенциально должен отвечать за:

```text
ownership
manual vs automatic control
limits
safe output
interlocks
controller leases
failed writes
client disconnect
script failure
shutdown
```

Ни Lua, ни Babashka, ни GUI не должны обходить его.

Опиши state machine на высоком уровне.

---

# 9. Extension architecture

Сравни уровни расширения:

```text
Rust built-in
data-driven definition
embedded Lua
external Babashka
WASM
isolated plugin process
native DLL/plugin
```

Для каждого оцени:

```text
скорость разработки
runtime overhead
failure isolation
safety
deployment
ABI stability
hot reload
complexity
```

Не пытайся обязательно поддержать всё.

Предложи минимальный разумный extension model для `lab-runtime`.

Отдельно ответь:

> Нужна ли вообще поддержка compiled plugins без перекомпиляции Core в первой версии?

---

# 10. Lua и Babashka

Критически оцени использование двух языков.

Исходная идея:

```text
Lua
    embedded extension language
    close to Runtime

Babashka
    external orchestration language
    REPL-driven development
```

Нужно определить:

```text
что доступно Lua
что доступно Babashka
что доступно обоим
что намеренно доступно только одному
```

Главная опасность:

создать два огромных параллельных API, которые нужно синхронно поддерживать.

Предложи архитектуру, которая этого избегает.

---

# 11. Runtime state ownership

Определи:

```text
кто владеет состоянием эксперимента
кто владеет instruments
кто владеет series
кто владеет controllers
кто владеет output state
кто владеет recording session
```

GUI, Lua callback или внешний Babashka client не должны случайно становиться владельцами критического state.

---

# 12. Execution model

Не проектируй детальный Tokio implementation, но определи высокоуровневую модель исполнения.

Рассмотри:

```text
runtime thread/task
instrument transaction scheduling
controller scheduling
signal propagation
script execution
network clients
recorder
GUI
```

Нужно гарантировать:

```text
slow client cannot block acquisition

slow script cannot silently block safety-critical controller

network disconnect cannot destroy experiment state

GUI restart does not have to stop experiment
```

Определи, где нужны isolation boundaries.

---

# 13. Commands / Queries / Events

Рассмотри единый domain boundary:

```text
Command
Query
Event
```

для взаимодействия:

```text
GUI → Runtime
Babashka → Runtime
Lua → Runtime
tests → Runtime
```

Определи, позволит ли это избежать нескольких несовместимых API.

Не проектируй полный JSON protocol.

Нужна только domain-level модель.

---

# 14. GUI

GUI должен быть adapter/client.

Определи:

```text
что GUI получает от Runtime
что GUI отправляет Runtime
как GUI получает обновления
```

GUI не должен содержать instrument-specific business logic, если descriptor позволяет этого избежать.

---

# 15. External API

Определи высокоуровневую роль external API.

Первая версия предполагается local-only.

В будущем:

```text
Babashka
remote GUI
monitoring client
test harness
```

должны иметь возможность использовать ту же domain boundary.

Архитектура должна быть:

```text
local-first
remote-ready
```

но native internet security пока реализовывать не требуется.

---

# 16. Recording

Определи, какие данные являются authoritative experiment history.

Рассмотри:

```text
measurements
configuration
commands/actions
controller state changes
output changes
errors
user/script events
```

Recorder должен быть независим от GUI и external client lifetime.

---

# 17. Configuration

Определи, какие вещи должны быть:

```text
compiled into Rust
runtime configuration
instrument descriptor
Lua extension
Babashka scenario
persistent experiment configuration
```

Не смешивай experiment procedure с hardware configuration без необходимости.

---

# 18. Failure model

Для каждого слоя рассмотрим отказ:

```text
serial transport fails

instrument goes offline

Lua extension throws

Lua extension hangs

Babashka disconnects

GUI closes

controller fails

recorder fails

IPC client is slow

Runtime shuts down
```

Определи, где должны находиться recovery policy и safe behavior.

---

# 19. Высокоуровневая структура проекта

Только после определения domain model предложи структуру Rust project/workspace.

Не начинай с crates.

Предложи минимальное количество crates/modules.

Для каждого объясни причину отдельной границы.

Избегай архитектуры из десятков микро-crates.

---

# 20. Proof-of-concept

Предложи минимальный архитектурный POC.

Он должен проверить самые рискованные предположения новой архитектуры, а не повторить `com_port_reader`.

Желаемый масштаб примерно:

```text
mock/virtual instrument

one native instrument implementation

one Lua-backed instrument

one signal

one native filter

one Lua filter

one Reference

one native PID

OutputArbiter

simple CLI/test harness
```

Babashka можно либо включить в этот POC минимально, либо отложить до следующего vertical slice.

Обоснуй решение.

---

# 21. Не писать production code

На этом этапе НЕ создавай:

```text
Cargo workspace
Rust traits
Lua bindings
IPC server
GUI
Babashka project
```

Если для объяснения интерфейса нужен псевдокод, используй короткий conceptual example.

Мы пока утверждаем архитектуру.

---

# 22. Deliverables

Создай:

```text
docs/architecture/HIGH_LEVEL_ARCHITECTURE.md
docs/architecture/EXTENSION_MODEL.md
docs/architecture/RUNTIME_AND_SAFETY_MODEL.md
docs/architecture/POC_PLAN.md
docs/architecture/OPEN_QUESTIONS.md
```

## HIGH_LEVEL_ARCHITECTURE.md

Должен содержать:

```text
цели
non-goals
domain model
component boundaries
dependency direction
основные диаграммы
state ownership
GUI/Lua/Babashka roles
```

## EXTENSION_MODEL.md

Подробно:

```text
native instruments
data-driven instruments
Lua extensions
Babashka
controllers
filters
future plugin possibilities
```

## RUNTIME_AND_SAFETY_MODEL.md

Подробно:

```text
execution ownership
OutputArbiter
safety state
failure boundaries
disconnect behavior
long-running experiment behavior
```

## POC_PLAN.md

Определи последовательность минимального proof-of-concept.

Разбей примерно на 5–10 небольших этапов.

Пока НЕ реализуй.

## OPEN_QUESTIONS.md

Запиши реально нерешённые вопросы.

Для каждого:

```text
почему вопрос важен
какие варианты есть
когда решение необходимо принять
```

Не принимай преждевременно решения, которые можно безопасно отложить.

---

# 23. ADR

Создавай ADR только для решений, которые действительно определяют архитектуру.

Не делай ADR на каждый каталог.

Для серьёзных решений используй:

```text
docs/adr/0001-....md
docs/adr/0002-....md
```

В каждом:

```text
Context
Decision
Consequences
Rejected alternatives
```

---

# 24. Финальная самопроверка

Перед завершением задай системе вопросы:

```text
Можно ли добавить Modbus-прибор без изменения Rust?

Можно ли добавить прибор с двумя нестандартными командами без написания полноценного Rust driver?

Можно ли написать новый фильтр без recompilation?

Можно ли написать experimental controller без recompilation?

Останется ли native PID работать при падении Babashka?

Может ли Lua extension обойти OutputArbiter?

Может ли GUI закрыться без уничтожения Runtime state?

Может ли новый GUI быть написан без изменения Core?

Можно ли в будущем подключиться удалённо?

Можно ли добавить compiled plugin позже, не ломая всю domain model?

Не создали ли мы два конкурирующих API для Lua и Babashka?

Не превратили ли мы архитектуру в framework сложнее самой задачи?
```

Если какой-то ответ неудовлетворителен, пересмотри архитектуру.

---

# 25. Git

На этом этапе изменяй только:

```text
docs/
```

и, если действительно нужно, архитектурные root-документы.

Не создавай production code.

После завершения выполни:

```powershell
git diff --check
git status
```

Если разрешение на commit уже дано, сделай один логический commit:

```text
docs: define high-level lab runtime architecture
```

---

# Итоговый отчёт пользователю

После записи документов дай краткий отчёт на русском:

1. Какая архитектура предлагается.
2. Какие исходные идеи подтверждены.
3. Какие исходные идеи ты отверг или изменил.
4. Нужны ли одновременно Lua и Babashka.
5. Как выглядит extension model.
6. Как обеспечивается safety.
7. Какой POC рекомендуется первым.
8. Какие вопросы пока сознательно оставлены открытыми.

Не начинай реализацию без отдельной команды пользователя.
