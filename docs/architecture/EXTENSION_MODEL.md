# Extension model

Статус: предложение. Основание и общая модель — [HIGH_LEVEL_ARCHITECTURE.md](HIGH_LEVEL_ARCHITECTURE.md). Никакие расширения на этом этапе не реализуются.

## 1. Минимальный набор

Предлагаются три способа реализации локальных компонентов: статически подключённый Rust, проверенные data-driven definitions и опциональный embedded Lua host. Внешняя orchestration использует один versioned domain API; Babashka — первый целевой scripting client этого API, а не обязательная зависимость Runtime. Compiled plugins без пересборки в первой версии не нужны.

Внутри Rust native extension — обычный модуль, зарегистрированный host при сборке. Не нужен универсальный discovery loader или обещание стабильного Rust ABI. Динамическое добавление поддерживается на уровне определений и Lua-компонентов, а не произвольных библиотек.

## 2. Сравнение вариантов

Оценки качественные, для небольшой лабораторной установки; это не результаты benchmarks. «Safety» означает возможность сохранить центральную авторизацию воздействий и ограничить отказ, а не аппаратную гарантию безопасности. Любой механизм требует проверки корректности конкретного алгоритма/определения.

| Уровень | Скорость разработки | Runtime overhead | Failure isolation / safety | Deployment | ABI / compatibility | Hot reload | Сложность и решение |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Rust built-in | Хорошая для сложного протокола; медленнее интерактивных правок | Минимальная дополнительная прослойка | В одном процессе; доверенный код может повредить Runtime. Подходит для обязательного trusted path | Пересборка/обновление host | Одна согласованная сборка; публичный binary ABI не нужен | Нет; штатный restart в безопасном состоянии | Низкая исходная сложность; основной уровень |
| Data-driven definition | Наиболее быстрый путь для известного протокола | Разбор/валидация при загрузке и небольшой mapping при работе | Нет произвольного кода, но неверный адрес/scaling способен вызвать опасное воздействие; trusted definitions + Runtime checks | Версионированный файл/пакет определения | Schema version, явная совместимость | На quiescent boundary, активные outputs сначала disarm | Низкая/средняя; включить |
| Embedded Lua | Быстрые простые algorithms/adapters | VM, преобразование данных, очереди worker | Ошибки/лимиты изолируются host; crash VM/native host code остаётся процессным отказом. Нет raw I/O/FFI | Runtime с Lua + script bundle | Версия host contract и script manifest | Новый instance/generation после остановки старого; без автоматического переноса private state | Средняя; включить ограниченно, проверить в POC |
| External Babashka | Быстро для recipes/REPL | IPC, сериализация, внешний scheduler/GC | Падение процесса отделено от Runtime; expired lease блокирует его воздействия | Отдельная установка и client helper | Versioned domain protocol | Перезапуск/reload клиента; leases не наследуются автоматически | Низкая добавка к общему API; необязательный клиент |
| WASM | Нужен SDK и сборка guest | VM/host calls и marshaling; измерять по задаче | Возможны ограничения памяти/инструкций; ошибка host всё ещё существенна | Модуль + engine | Версионированный host interface, не произвольный Rust ABI | Замена instance с новым generation | Средняя/высокая; отложить до конкретной потребности |
| Isolated plugin process | Больше инфраструктуры, язык можно выбирать | IPC/копирование/планирование ОС | Crash isolation процесса; ресурсы/права надо ограничить отдельно. Выход только через Runtime | Executable, protocol compatibility, supervisor | Versioned messages | Перезапуск и handshake; контролируемый state restore | Средняя/высокая; вероятный путь для сложного стороннего кода |
| Native DLL с C ABI | Медленнее из-за SDK, memory ownership и packaging | Небольшая call overhead | В одном адресном пространстве; ошибка/unsafe/FFI может обойти любые логические границы | Platform-specific binaries | Требуется собственный versioned C contract, ownership и error rules | Опасно при живых callbacks/handles; по умолчанию restart | Высокая; не включать без доказанной необходимости |
| Rust dylib/plugin | Удобен только внутри согласованной сборки | Близка к native | Нет crash isolation или защиты от обхода safety | Жёсткая связка toolchain, зависимостей и приложения | Не принимается как стабильный внешний ABI; нужна согласованная пересборка | Не проектируется для активного процесса | Высокая цена поддержки без нужной выгоды; отвергнут для v1 |

WASM и process plugins оставляют возможность будущих compiled extensions. Сохранить для них нужно domain types, component lifecycle, capability/operation boundary, deadlines и generation checks. Не нужно заранее реализовывать общий plugin manager.

## 3. Четыре реализации Instrument

| Реализация | Что меняется при добавлении прибора | Что остаётся общим |
| --- | --- | --- |
| Native Rust | Driver/protocol module и регистрация в host, пересборка | Descriptor, scheduling, operation/result model, lifecycle, safety |
| Data-driven | Проверенное определение на поддержанном protocol profile + instance binding | Rust codec/executor, descriptors/introspection, safety |
| Lua-backed | Script bundle, descriptor/operation definitions, host capabilities | Registry, Rust port ownership, budgets, typed results, safety |
| Virtual/mock/replay | Модель в Rust/Lua или источник записанных samples | Instrument/Signal identity, timestamps/quality, lifecycle и диагностика |

Virtual actuator также проходит arbiter, чтобы проверить общую модель. Его подтверждение имеет simulation provenance и не является подтверждением реального hardware. Hardware mode и simulation/replay mode должны явно различаться при configuration и arming.

### Native instruments

Native driver нужен для нового низкоуровневого протокола, сложного stateful обмена, непредставимого декларативно framing или требуемой надёжности проверенного native пути. Он преобразует domain operations в protocol transactions, не владеет experiment lifecycle и не принимает решения manual/auto.

Driver — доверенная часть поставки. Статически связанный код технически может вызвать ОС; гарантия общего output boundary обеспечивается архитектурной инкапсуляцией, review и проверками dispatch path, а не sandbox для собственного Rust. Controller modules не получают транспортных зависимостей. Не загружаем сторонние native DLL как равноценные безопасные extensions.

### Data-driven instruments

Definition известного протокола задаёт protocol profile, parameter mapping, register addresses/function kinds, data types, byte/word order, scaling, units, доступы, response/exception semantics и operation side effects. Instance configuration задаёт COM resource и slave address. Decoder/encoder и transaction scheduling принадлежат Rust.

Добавление Modbus-прибора без изменения Rust возможно, когда имеющийся Modbus adapter поддерживает необходимые function codes, типы, framing и модель обмена. Неизвестный function code или нестандартная handshake sequence не маскируются произвольным фрагментом кода в YAML/JSON. Новый Rust primitive добавляется, только если ограниченный Lua/data путь действительно недостаточен.

При загрузке проверяются schema/version, типы, scaling и переполнение, адресные диапазоны, permissions, пересечения с actuator registers, конфликтующие instance bindings и согласованность capabilities. Правила кросс-регистровых aliases должны исключать альтернативную запись в тот же физический выход в обход arbiter. Сведения о скрытых side effects конкретного прибора остаются ответственностью проверенного device definition.

Declaration языка конфигурации не должна постепенно превратиться в собственный язык программирования. Ветвящаяся обработка данных — Lua/native; аппаратные процедуры с критичным timing — native adapter.

### Lua instruments и протокольные adapters

Lua instrument предоставляет тот же descriptor и возвращает такие же typed results, как native instrument. Для примера «temperature + power» достаточно небольшого определения и script parser/adapter при наличии поддержанных transaction primitives.

Физический extension bundle содержит проверенный каталог именованных операций. Для каждой операции зафиксированы instrument/resource binding, address, kind (read/configuration/output/action), типизированные arguments, ограничения, deadline, предельная длина ответа, framing/response checks, идемпотентность и способ проверки результата. Каталог после validation неизменяем в течение активной generation.

Conceptual example, не binding или wire schema:

```text
temperature.read -> bounded text request template; response -> Lua numeric parser
power.set(value)  -> typed output operation; Rust renders the approved template
                    only for the value authorized by OutputArbiter
```

Для простых нестандартных текстовых команд Rust поддерживает ограниченный declarative request/response profile: фиксированные bytes/templates, допустимые подстановки, delimiter/length и проверки ответа. Lua может разобрать ответ или выбрать объявленную операцию; она не может подменить адрес, raw bytes, kind или разрешённое output value. Произвольный script serializer физической записи без проверяемого соответствия разрешённому значению не предлагается. Если протокол требует этого, нужен новый trusted Rust adapter; виртуальные приборы такого ограничения на синтез данных не имеют.

Read operation без side effects может выполняться по acquisition schedule. Любая физическая операция, способная изменить выход, включая mode/configuration/reset, требует разрешения safety. Не существует «maintenance raw write» для GUI/Lua/Babashka. Во время active control запрещены configuration operations, способные изменить смысл управления; их выполнение после disarm тоже проверяется и журналируется.

Этот механизм не доказывает автоматически истинность описания прибора: если автор definition ошибочно назвал команду нагрева чтением, по произвольным байтам это не определить. Hardware definitions, output bindings и safe-action profiles являются доверенной configuration установки, проверяемой до допуска к управлению. Обычный script callback не может менять этот каталог или свою capability. Lua VM ограничивает script code, а не исправляет неверно описанное hardware.

Lua не удерживает COM между callbacks и не блокирует executor ожиданием произвольного кода. Invocation именованной операции создаёт запрос; Runtime делает bounded transaction, callback получает результат после завершения. Для последовательности без interleaving потребуется заранее описанная bounded transaction group с общим deadline; если простой последовательный обмен достаточен, не добавляем эту возможность в v1. Автоматическое владение шиной на время Lua function запрещено.

Safe action физического выхода должна выполняться без работающей Lua VM: native driver либо заранее проверенный Rust-executable profile, включая достаточную проверку результата. Если безопасное воздействие или его обязательное подтверждение требует произвольного Lua callback, такой instrument допускается для чтения/эмуляции, но не для активного физического управления в v1. Внешняя аппаратная защита нужна там, где потеря порта/процесса лишает Runtime возможности достичь safe state.

## 4. Processing, Reference и controllers

Native и script processing используют одинаковые входные snapshots, timestamps/quality, configuration, diagnostics и output validation. Script получает ограниченное окно, а не всю историю сессии и не mutable ссылки на Runtime. Host проверяет тип, units, размер результата, конечность чисел и generation. Private state filter принадлежит его instance, а Runtime определяет creation, reset, failure и removal.

Fixed/Ramp/Program Reference — native определения; Script Reference — ограниченное вычисление desired value. Для обоих одно понятие времени и качества. Script Reference не может продлевать lease или отменять interlock.

Native PID, On/Off и будущий Furnace работают через OutputProposal. Lua controller получает измерения/reference/dt и возвращает proposals/diagnostics; он не получает другой output API. Это экспериментальный controller: admissibility зависит от измеренного бюджета исполнения и назначения установки. Сам protective interlock и финальная output policy всегда остаются Rust. Lua-derived measurements не считаются независимой защитой от ошибки Lua; критичный interlock должен иметь пригодный независимый источник.

External controller представлен в Runtime как зарегистрированный producer с lifecycle, declared inputs/output bindings, deadlines и lease. Алгоритм исполняется в Babashka/другом процессе, но Runtime владеет полномочиями и последним принятым результатом. Клиент отправляет proposals со ссылкой на входные sequences; host проверяет их пригодность. Heartbeat не заменяет свежие proposals и свежие измерения. Это supervisory/experimental control, не обещание детерминированного сетевого loop.

Будущий compiled plugin должен использовать эти же contracts. Если появится требование передать физический порт внешнему процессу, это будет отдельный пересмотр архитектуры port ownership, а не незаметное расширение plugin API.

## 5. Lua execution, lifecycle и reload

Lua host опционален и зависит от Core contracts; Core не зависит от Lua. Он владеет VM/workers, а Runtime — регистрацией, configuration и admission. Скрипт проходит registered → validated → ready → running; ошибки или превышение budget переводят instance в failed/quarantined. Replacement создаёт новый generation; поздние результаты старого экземпляра отклоняются.

Контекст Lua ограничен memory quota, instruction/work budget, wall-clock deadline, числом/размером outputs и объёмом host requests. Прямой filesystem/network/process access, dynamic native modules, FFI и средства снятия лимитов не предоставляются. Пакеты загружаются host из разрешённого deployment path. Все предоставленные host calls должны быть ограниченными либо асинхронными — одного instruction hook недостаточно против зависшего native call.

Timeout снимает полномочия и зависимость control path от результата. Worker isolation не гарантирует безопасного принудительного завершения зависшего native кода в том же процессе. Нельзя бесконечно создавать replacement workers взамен зависших; вводится bounded quarantine, новые запуски отклоняются, fault видим Runtime. Если POC не подтвердит возможность ограничить предоставленный Lua execution path, физическое управление через него не допускается; следующий вариант — process isolation или сужение Lua возможностей.

Reload не выполняется внутри активного callback. Сначала validate новой версии, затем остановка затронутого instance, отзыв leases/ожидание safe transition для управления, инвалидирование старой generation и установка новой. Filter window и controller state по умолчанию сбрасываются и прогреваются заново; автоматическая миграция произвольного Lua state не предлагается. Не относящиеся к заменяемому component цепочки продолжают работу. Перезапуск controller после reload требует явного arming. Версия, manifest и фактический script source фиксируются recording metadata.

## 6. Нужны ли одновременно Lua и Babashka

Для требований brief оба уровня полезны, но обязательная одновременная установка двух языков не нужна. Lua закрывает локальный быстрый extension без отдельного процесса. Babashka закрывает внешний REPL и recipes без встраивания orchestration VM в Core. Runtime способен работать без обоих; первый POC проверяет Lua, следующий небольшой slice — Babashka через общий external API.

| Возможность | Lua component context | Babashka / external client |
| --- | --- | --- |
| Typed values, units, quality, diagnostics | Да, общий domain vocabulary | Да, тот же vocabulary |
| Describe / scoped Query | Разрешённое подмножество, snapshots без reentrant mutation | По client permissions через domain API |
| Structured experiment events | Да, через host и общий event validation | Да, через Command; origin добавляет Runtime |
| Предложить выход | Результат зарегистрированного controller либо scoped domain command; общий arbiter | Зарегистрированный external/manual producer с lease; общий arbiter |
| Обращаться к физическому прибору | Только именованные операции конкретного зарегистрированного adapter; output через arbiter | Только instrument-level domain commands; raw transport не доступен |
| Локальные filter/reference/instrument callbacks | Да, основной смысл Lua | Нет, внешний клиент не callback внутри Runtime |
| Управлять всем experiment graph, configuration, recording, deployment | Нет в стандартном component context | Да, в пределах permissions и lifecycle checks |
| REPL, recipes, внешние файлы/интеграции | Не входит в embedded host API | Да, ответственность клиентского процесса |
| Safety policy implementation / port ownership | Нет | Нет |

Не создаём по большому объектному SDK для каждого языка. Есть один набор domain messages/semantics и маленький component host contract. Lua-host обращения к management boundary используют существующие handlers с ограниченными capabilities; bindings преобразуют значения, не повторяют бизнес-валидацию. Babashka helper предоставляет discover/query/command/subscribe и удобства REPL поверх тех же понятий. Callback не может синхронно реентерабельно изменить graph посреди controller tick; разрешённые обращения поступают в обычную очередь commands.

Если практика покажет, что локальные extensions не нужны, Lua host можно не поставлять. Если нужны только локальные adapters и CLI, Babashka остаётся внешним выбором пользователя. Не заменяем два чётких уровня одним огромным embedded orchestration API.

## 7. Условия расширения модели позже

Process plugin рассматривается при сложном стороннем SDK, необходимости crash isolation или языке, плохо подходящем embedded host. WASM рассматривается при переносимых вычислительных модулях с ограниченными host calls. C ABI — только если измеренная потребность в in-process compiled вызовах оправдает отсутствие isolation и стоимость совместимости. Rust dylib не становится публичным ABI по умолчанию.

Новые adapters подключают component contracts и сохраняют output authority в Runtime. Версионируются данные/контракты и generation, а не сериализуются Rust object layouts. Детали SDK, discovery, signatures, automatic state migration и hot replacement активного control loop откладываются до конкретного use case. См. [ADR-0003](../adr/0003-bounded-extension-model.md) и [OPEN_QUESTIONS.md](OPEN_QUESTIONS.md).
