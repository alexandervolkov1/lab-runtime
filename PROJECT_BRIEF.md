# lab-runtime — Project Brief

## Статус

Это новый проект.

Он не является рефакторингом существующего `com_port_reader` и на первом архитектурном этапе не должен наследовать его структуру автоматически.

Существующий проект является:

* работающей версией v1;
* источником проверенных алгоритмов;
* источником драйверов;
* источником реальных требований;
* будущим донором отдельных реализаций.

Архитектура lab-runtime должна сначала быть спроектирована независимо.

После утверждения архитектуры будет отдельный этап сопоставления новой системы с кодом v1 и переноса подходящих компонентов.

---

# Главная цель

Создать универсальный runtime для лабораторной автоматизации.

Система должна сочетать:

* быстрое и надёжное ядро на Rust;
* поддержку физических приборов;
* динамически подключаемые приборы;
* обработку сигналов;
* управление процессами;
* безопасную работу с исполнительными устройствами;
* встроенные скриптовые расширения;
* внешнюю интерактивную автоматизацию;
* GUI;
* регистрацию эксперимента;
* возможность удалённого подключения в будущем.

Главное требование к архитектуре:

> Расширение системы должно требовать изменения Rust Core только тогда, когда для этого действительно нужна новая низкоуровневая или safety-critical возможность.

---

# 1. Приборы

Система должна работать с различными физическими приборами.

Основной транспорт на первом этапе:

* serial / COM / RS-485.

В будущем архитектура не должна мешать добавить:

* TCP;
* другие транспорты;
* виртуальные устройства;
* replay/mock устройства.

Физический прибор может быть реализован несколькими способами.

## Native Rust instrument

Полноценный драйвер на Rust.

Используется для:

* нового физического протокола;
* сложного бинарного протокола;
* критичных по надёжности устройств;
* производительных реализаций.

## Data-driven instrument

Если Rust уже знает протокол, новый прибор должен по возможности добавляться декларативно.

Например для Modbus RTU определение может содержать:

* slave address;
* параметры;
* регистры;
* типы данных;
* scaling;
* units;
* read/write permissions;
* limits.

Добавление такого прибора не должно требовать перекомпиляции Rust Core.

## Script-backed instrument

Если нужен только небольшой набор операций нестандартного прибора, должно быть возможно написать небольшой script adapter.

Пример:

* запросить температуру;
* установить мощность;
* распарсить текстовый ответ.

После регистрации такой script-backed instrument должен выглядеть для остальной системы как обычный Instrument.

---

# 2. Разделение transport / protocol / instrument

Архитектура должна явно рассмотреть разделение:

```text
Transport
    ↓
Protocol
    ↓
Instrument
```

Transport отвечает за передачу данных.

Protocol отвечает за framing и семантику протокола.

Instrument отвечает за предметные понятия:

```text
temperature
pressure
setpoint
power
flow
```

Необходимо оценить, насколько строго это разделение должно присутствовать в runtime API.

---

# 3. Единая модель Instrument

Независимо от реализации:

```text
Rust driver
data-driven definition
Lua driver
virtual instrument
future external plugin
```

прибор должен представляться ядру через единую domain model.

Instrument должен уметь публиковать descriptor.

Descriptor потенциально содержит:

* instrument type;
* identity;
* display name;
* parameters;
* parameter types;
* units;
* readable/writable flags;
* limits;
* polling capability;
* control-output capability;
* other capabilities.

GUI и внешние клиенты должны преимущественно использовать descriptors, а не hardcoded knowledge конкретных приборов.

---

# 4. Signal processing

Система должна позволять строить поток обработки сигналов.

Необходимо рассмотреть общую модель вроде:

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

Также требуется понятие Reference.

Например:

```text
FixedReference
RampReference
ProgramReference
ScriptReference
```

Постепенное изменение setpoint должно рассматриваться отдельно от самого PID-контроллера.

---

# 5. Фильтры

Нужны как минимум два класса фильтров.

## Native filters

Реализованы на Rust.

Для:

* часто используемых алгоритмов;
* производительности;
* deterministic execution.

## Script filters

Пользователь должен иметь возможность добавить экспериментальный или простой фильтр без перекомпиляции Rust Core.

Снаружи native и script filter желательно должны выглядеть одинаково.

---

# 6. Управление

Нужны native Rust controllers:

* PID;
* On/Off;
* существующий Furnace controller;
* будущие deterministic/safety-critical controllers.

При этом архитектура должна позволять добавлять экспериментальные control strategies без перекомпиляции ядра.

Нужно рассмотреть:

* embedded script controller;
* external controller;
* future compiled plugin.

Все controller implementations должны работать через единую safety boundary.

Controller не должен напрямую писать в физический прибор.

Предпочтительная концепция:

```text
Controller
    ↓
OutputProposal
    ↓
OutputArbiter / Safety
    ↓
Instrument
```

---

# 7. Output safety

Rust должен оставаться окончательным владельцем физического выхода.

Нужно предусмотреть:

* ownership;
* manual/automatic state;
* safe output;
* output limits;
* interlocks;
* failed writes;
* controller pause/resume;
* controller removal;
* client disconnect;
* script failure;
* watchdog/lease для external control.

Script или внешний клиент могут предложить output value, но не должны обходить safety layer.

---

# 8. Lua

Lua рассматривается как embedded extension language.

Её потенциальные области:

* lightweight instrument drivers;
* protocol adapters;
* virtual instruments;
* emulators;
* simple filters;
* local transforms;
* возможно lightweight local control nodes.

Преимущества:

* встроенность;
* небольшой runtime;
* отсутствие отдельного процесса;
* удобные callbacks;
* простое распространение программы.

Lua не должна автоматически становиться владельцем всего high-level orchestration.

---

# 9. Babashka / Clojure

Babashka рассматривается как external orchestration environment.

Её потенциальные области:

* experiment scenarios;
* recipes;
* high-level procedures;
* supervisory logic;
* live development;
* REPL/nREPL;
* dynamic configuration;
* composition of instruments/signals/controllers;
* experimental external control.

Babashka подключается к Rust Runtime через стабильный внешний API.

Rust Runtime должен продолжать безопасную работу при закрытии или перезапуске Babashka.

---

# 10. Два скриптовых языка

Необходимо критически оценить сложность использования одновременно Lua и Babashka.

Предпочтительная концепция:

```text
Lua
    embedded extensions close to runtime

Babashka
    external interactive orchestration
```

Необходимо избегать создания двух полностью дублирующихся scripting APIs.

Архитектура должна явно определить:

* где заканчивается Lua;
* где начинается Babashka;
* какие возможности намеренно доступны обоим;
* какие принадлежат только одному уровню.

Если два языка дают больше сложности, чем пользы, архитектор должен прямо это сказать и предложить альтернативу.

---

# 11. External API

Rust Runtime должен в будущем предоставлять внешний API.

Первая версия:

```text
localhost only
```

Предпочтительно простой versioned protocol.

Возможный кандидат:

```text
TCP + NDJSON
```

но это не зафиксированное решение.

Архитектура должна предусматривать в будущем:

* remote clients;
* reconnect;
* multiple clients;
* GUI as client;
* Babashka as client;
* monitoring client;
* authentication/authorization extension;
* SSH/VPN remote operation.

Client lifetime не должен определять experiment lifetime.

---

# 12. GUI

GUI не должен быть ядром системы.

Желаемая граница:

```text
GUI
  ↓
Domain Commands / Queries / Events
  ↓
Runtime
```

GUI не должен владеть:

* polling;
* protocol logic;
* controllers;
* safety;
* recorder;
* instrument semantics.

Первым GUI может снова стать egui, но Runtime не должен зависеть от egui.

---

# 13. Recorder

Durable recording остаётся ответственностью Rust Runtime.

Нужно записывать как минимум:

* measurements;
* important runtime events;
* user actions;
* controller/output actions;
* errors;
* configuration/session metadata.

Внешние scripts должны иметь возможность добавлять структурированные experiment events.

---

# 14. Extension levels

Архитектура должна оценить следующий многоуровневый extension model:

| Extension level            | Typical purpose                                       | Core rebuild |
| -------------------------- | ----------------------------------------------------- | ------------ |
| Rust built-in              | hardware, protocols, safety, deterministic algorithms | yes          |
| data-driven                | known protocols / simple device definitions           | no           |
| embedded Lua               | lightweight runtime extensions                        | no           |
| Babashka                   | orchestration and interactive development             | no           |
| future WASM/process plugin | compiled third-party extensions                       | no           |

WASM/plugin system не нужно автоматически включать в первую реализацию.

Нужно лишь определить, требуется ли оставить для него разумную extension boundary.

---

# 15. Native plugins

Не следует автоматически использовать Rust dynamic libraries как plugin ABI.

Необходимо сравнить:

* Rust dylib plugins;
* stable C ABI;
* WASM;
* isolated plugin processes;
* embedded Lua.

Критерии:

* safety;
* ABI stability;
* crash isolation;
* complexity;
* latency;
* ease of development.

---

# 16. Runtime properties

Система рассчитана на лабораторные процессы продолжительностью до многих часов или дней.

Runtime должен иметь ясную модель:

* state ownership;
* scheduling;
* command handling;
* event handling;
* thread/task ownership;
* shutdown;
* error propagation;
* reconnect;
* persistence.

Slow UI/script/network clients не должны блокировать acquisition или native controllers.

---

# 17. Главные архитектурные сущности

На первом этапе необходимо особенно хорошо определить:

```text
Instrument
InstrumentDescriptor

Transport
Protocol

Signal
SignalNode

Source
Transform
Filter

Reference

Controller
OutputProposal
OutputArbiter

Runtime Command
Runtime Event

Recorder

Extension
```

Не все эти сущности обязаны стать Rust traits.

Архитектор должен выбирать representation исходя из простоты и реальных требований.

---

# 18. Что НЕ требуется на первом этапе

Не нужно:

* писать GUI;
* переносить код v1;
* реализовывать COM;
* реализовывать PID;
* внедрять Lua;
* внедрять Babashka;
* проектировать полный wire protocol;
* создавать десятки crates;
* создавать generic plugin framework;
* достигать feature parity с v1.

Первый этап должен доказать архитектуру на уровне domain boundaries и runtime responsibilities.

---

# 19. Отношение к v1

`com_port_reader` v1 является отдельной стабильной системой.

После утверждения архитектуры lab-runtime будет выполнен отдельный анализ:

```text
lab-runtime subsystem
    ↕
existing v1 implementation
```

После этого подходящие части кода будут:

* перенесены;
* адаптированы;
* переписаны;
* либо оставлены только в v1.

Новая архитектура не должна автоматически повторять структуру v1.
