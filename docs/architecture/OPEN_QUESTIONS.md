# Открытые вопросы и самопроверка архитектуры

Статус: target baseline после foundation reconciliation 2026-09-14. Исходный архитектурный review завершён; implementation разрешена только для Milestone 1.

## 1. Что уже решено в предложении

Runtime владеет долгоживущим состоянием, Rust — физическим I/O и output authority. GUI/Babashka являются clients; Lua — ограниченный optional component host. Native/data-driven/Lua implementations используют общую Instrument/Signal/Controller модель. Reference отделена от PID. Активное управление не восстанавливается автоматически после Runtime crash. В v1 не нужны Rust dylib plugins, большой plugin framework, arbitrary graph cycles или полный durable workflow engine.

Эти решения не оставлены «на потом», но имеют статус предложенных до review пользователя. Открытые вопросы ниже касаются ещё неизвестных требований или выбора реализации, который безопасно отложить. Для них указана граница, до которой решение обязательно.

## 2. Вопросы, требующие конкретизации

| ID / вопрос | Почему важен | Варианты | Когда решить / текущая позиция |
| --- | --- | --- | --- |
| Q01. Первые реальные приборы и workload | Задаёт protocol scope, число каналов, частоты и смысл safety | Один heater/датчик; несколько приборов общей шины; иной лабораторный процесс | До выбора physical POC. Сейчас небольшой virtual thermal process, без предположений о моделях hardware |
| Q02. Safe action и допустимое evidence по каждому actuator | Ноль/выключение не универсально безопасны; write ACK не подтверждает физический эффект | Off/zero; закрытие; bounded охлаждение/последовательность; readback либо явно допустимый ACK с независимой защитой | До arming любого физического выхода. Без утверждённого profile — запрет arming |
| Q03. Аппаратная защита при потере Runtime/порта | Программа не исполнит safe command при crash/power/COM failure | Device watchdog, независимый interlock/контактор, безопасное аппаратное состояние, ограничение класса допустимых опытов | До physical-control slice; software arbiter её не заменяет |
| Q04. Timing и freshness budgets | Определяют пригодность общей шины и script path; приоритет не прерывает in-flight transaction | Разные periods/TTL на контуры; выделенный ресурс; более простой контроллер/аппаратное управление | POC budgets — перед timing tests, реальные — до hardware admission. Нет заранее выбранных чисел |
| Q05. Furnace semantics — закрыт migration analysis | Один scalar input/output, feed-forward + predictor + PI | Один native Controller с private composition, Reference отдельно; public internal subgraph не нужен | Решено; алгоритм и runtime перенос позже, не M1 |
| Q06. Lua engine/binding и гарантии budgets | От этого зависит прерываемость CPU/memory/host operations | Ограниченный embedded VM host; ещё более узкий callback набор; process host при неудовлетворительном isolation | До/на этапе 5 POC. FFI/raw OS недоступны; неограниченный embedded callback не допускается |
| Q07. Достаточность bounded transaction profiles | Нужно поддержать две нестандартные команды, не открывая raw-write обход | Fixed text templates + Rust response checks; небольшой набор binary primitives; новый native protocol adapter | Выбрать минимальный профиль перед этапами 3/5 POC. Произвольный Lua serializer actuator writes не принят |
| Q08. Definition format, versioning и local deployment trust | Ошибка registers/side effects/scaling влияет на hardware, introspection требует совместимости | Один schema-validated JSON/TOML/другой формат; пакеты с manifest; local allowlist/installation permissions | Формат — до data-driven POC; permissions/процедура допуска — до physical deployment. Definitions не изменяются callback на лету |
| Q09. Multi-output и межконтурные interlocks | Несколько writes нельзя считать атомарными; v1 Furnace не требует multi-output | Независимые каналы; fail-together group; аппаратная атомарность/защита | До первого реального связанного multi-output controller |
| Q10. Присутствие supervisor и длительность внешней процедуры | Автономный native loop не продолжает неотправленные шаги Babashka recipe | Продолжать текущий native plan; explicit supervisor lease; checkpoint/recovery во внешнем сценарии | Policy выбирается при configuration эксперимента; общий default — независимый Runtime. Durable workflow engine только при новом требовании |
| Q11. Recorder format/durability | Нужна честная persisted граница; v1 SQLite writer — кандидат, не frozen schema | Bounded writer, explicit gaps, session/run/config provenance и typed evidence | До stage 7 уточнённого POC и physical acceptance |
| Q12. Retention/rotation/export | Многодневная работа не должна молча терять данные | Explicit quota/stop/archive policy | К recorder/stress stages 7–8; 24/72h stage9, не gate M1 |
| Q13. IPC transport и compatibility window | Нужны локальные reconnect/multi-client и будущий remote adapter | TCP + NDJSON; другой framed protocol; OS-local IPC | До Babashka slice. Зафиксированы domain semantics, bounds и loopback/local-only, не полный JSON schema |
| Q14. Local authentication / permissions | Loopback сам по себе не различает локальных клиентов; read и output acquire имеют разные права | OS-local permissions; local token/session capabilities; сочетание | До первого IPC listener. Internet authentication/TLS — до remote exposure, не текущая реализация |
| Q15. Restart recovery сверх configuration/history | Иногда нужно продолжить опыт после сбоя, но слепой restore output опасен | Ручной restart с прогревом; явные checkpoints алгоритмов; согласованный recovery protocol | Отложить до реального требования. V1: Unverified, новые epochs, никакого automatic rearm |
| Q16. Clock/device timestamp и sample alignment | Разные приборы могут иметь несинхронные часы, влияющие на derived/control values | Host receive time; device time с provenance; позже clock synchronization/interpolation | Базовый timestamp contract в этапе 1; реальные tolerance перед multi-device control. История не обещает точный глобальный физический порядок |
| Q17. Hot reload с сохранением algorithm state | Удобно для REPL, но старое состояние может быть несовместимо с новой версией | Stop/reset/re-warm; explicit versioned migration; shadow run и ручное переключение | Можно отложить. Default — новая generation, reset state, safe transition и явный rearm |
| Q18. Compiled extensions | Потребность может возникнуть для SDK или дорогого алгоритма, но сейчас не доказана | Native rebuild; isolated process; WASM; C ABI только при отдельном обосновании | После конкретного use case/измерений. Rust dylib ABI в v1 не нужен |
| Q19. Первый GUI и специализированные views | Определяет UX, но не должна определять Core ownership | egui; другой local/remote GUI; generic descriptor forms + optional custom views | После domain/IPC POC. Runtime остаётся отдельным процессом |

## 3. Финальная самопроверка из ARCHITECTURE_PLAN.md

Ответы относятся к предлагаемой модели. Эмпирические подтверждения должны появиться только после выполнения POC.

| Вопрос | Ответ и существенные условия | Где описано / как проверить |
| --- | --- | --- |
| Можно ли добавить Modbus-прибор без изменения Rust? | Да, если уже поддержаны нужные операции/типы протокола: новое definition и instance binding. Новый низкоуровневый protocol primitive может потребовать Rust | [Extensions, раздел 3](EXTENSION_MODEL.md#3-четыре-реализации-instrument); POC 3 |
| Можно ли добавить прибор с двумя нестандартными командами без полноценного Rust driver? | Да, Lua adapter + проверенный bounded text/transaction profile. Для actuator writes bytes/value формирует Rust; произвольный протокол вне профиля требует native adapter | [Extensions](EXTENSION_MODEL.md); POC 5 |
| Можно ли написать новый фильтр без recompilation? | Да, Lua filter с типами, bounded state/window и budgets, заменяемый через новую generation | [Signal model](HIGH_LEVEL_ARCHITECTURE.md); POC 5 |
| Можно ли написать experimental controller без recompilation? | Да, Lua либо external producer с тем же lifecycle/proposals/leases. Это не право менять safety rules | [Extensions, controllers](EXTENSION_MODEL.md); POC 5 и Babashka slice |
| Останется ли native PID работать при падении Babashka? | Да, если PID и его пригодные inputs/reference принадлежат Runtime и нет явно требуемого supervisor interlock. External producer или недоставленные future reference updates не превращаются в автономные | [Leases и disconnect](RUNTIME_AND_SAFETY_MODEL.md); POC 7 |
| Может ли Lua extension обойти OutputArbiter? | Через предоставленные capabilities — нет: нет raw I/O/FFI, операции связаны с validated catalog, dispatcher повторно проверяет permit. Доверенные definitions/host и независимость protective inputs обязательны | [Lua instruments](EXTENSION_MODEL.md), [Output authority](RUNTIME_AND_SAFETY_MODEL.md); POC 2/5 |
| Может ли GUI закрыться без уничтожения Runtime state? | Да, отдельный процесс; manual authority GUI прекращается по lease, а native loop/recorder живут дальше | [Ownership](HIGH_LEVEL_ARCHITECTURE.md); POC 7 |
| Может ли новый GUI быть написан без изменения Core? | Да, на domain API и descriptors для объявленных capabilities; новый предметный feature может потребовать расширения модели, смена UI сама по себе — нет | [Commands/Queries/Events](HIGH_LEVEL_ARCHITECTURE.md); generic harness + следующий IPC slice |
| Можно ли в будущем подключиться удалённо? | Да, adapter boundary, versioning, identity, reconnect и leases это допускают. До network exposure нужен отдельный security/deployment этап | [External API](HIGH_LEVEL_ARCHITECTURE.md); Q13/Q14 |
| Можно ли добавить compiled plugin позже без поломки domain model? | Да, через instrument/processing/controller runner и versioned data contracts. Новые transport/host capabilities могут потребовать adapter work; ABI заранее не обещается | [Extension comparison](EXTENSION_MODEL.md), [ADR-0003](../adr/0003-bounded-extension-model.md); Q18 |
| Не созданы ли два конкурирующих API для Lua и Babashka? | Нет: единая management semantics Commands/Queries/Events; Lua имеет узкий component context, Babashka — client API. Bindings не реализуют собственные safety/validation rules | [Lua/Babashka roles](EXTENSION_MODEL.md); POC 1/5 и Babashka slice |
| Не стала ли архитектура framework сложнее задачи? | Предложены один Runtime, три начальные единицы сборки, DAG без произвольных циклов, один arbiter и ограниченные extension levels. Нет generic plugin manager, durable workflow engine и event-sourced Core | [Структура проекта](HIGH_LEVEL_ARCHITECTURE.md), [POC](POC_PLAN.md); пересматривать лишние abstractions на каждом POC этапе |

Ограничения ответов намеренны: неизвестный физический протокол, ложный sensor value и потеря питания не становятся решёнными задачами только благодаря универсальной abstraction. При этом все требуемые способы расширения имеют конкретный путь в модели.

## 4. Покрытие ARCHITECTURE_PLAN.md

| Раздел плана | Результат |
| --- | --- |
| Перед началом | Три root-документа прочитаны полностью после обновления; git status/log просмотрены; v1 не исследовался |
| 1. Domain model | HIGH_LEVEL_ARCHITECTURE §2: назначение, владелец, соседи, domain/detail для всех перечисленных сущностей |
| 2. Rust Core | HIGH_LEVEL_ARCHITECTURE §4: Core/library и deployed Runtime, adapters и dependency diagrams |
| 3. Instrument | HIGH_LEVEL_ARCHITECTURE §3 + EXTENSION_MODEL §3 |
| 4. Transport / Protocol | HIGH_LEVEL_ARCHITECTURE §5 + EXTENSION_MODEL §3 + RUNTIME_AND_SAFETY_MODEL §1 |
| 5. Signal processing | HIGH_LEVEL_ARCHITECTURE §6: DAG, quality/time, windows, history, diagnostics |
| 6. Reference | HIGH_LEVEL_ARCHITECTURE §6: Fixed/Ramp/Program/Script и независимый lifecycle |
| 7. Controller | HIGH_LEVEL_ARCHITECTURE §7 + RUNTIME_AND_SAFETY_MODEL §5 |
| 8. Output safety | RUNTIME_AND_SAFETY_MODEL §2–4: arbiter, state machine, leases, evidence |
| 9. Extension architecture | EXTENSION_MODEL §1–4, §7: сравнительная таблица всех критериев и решение о compiled plugins |
| 10. Lua и Babashka | EXTENSION_MODEL §5–6: ограничения, общие и раздельные capabilities |
| 11. State ownership | HIGH_LEVEL_ARCHITECTURE §2, §9 + RUNTIME_AND_SAFETY_MODEL §1 |
| 12. Execution | RUNTIME_AND_SAFETY_MODEL §1, §7–8: isolation, queues, deadlines, longevity |
| 13. Commands / Queries / Events | HIGH_LEVEL_ARCHITECTURE §8: единая semantics, outcomes, revision, reconnect |
| 14. GUI | HIGH_LEVEL_ARCHITECTURE §7–8: descriptor client, subscriptions, commands |
| 15. External API | HIGH_LEVEL_ARCHITECTURE §8: local-only/versioned, remote evolution без полного wire protocol |
| 16. Recording | HIGH_LEVEL_ARCHITECTURE §9 + RUNTIME_AND_SAFETY_MODEL §6 |
| 17. Configuration | HIGH_LEVEL_ARCHITECTURE §9: hardware/definition/safety/procedure separation |
| 18. Failures | RUNTIME_AND_SAFETY_MODEL §7: все перечисленные отказы, recovery owner и safe behavior |
| 19. Project structure | HIGH_LEVEL_ARCHITECTURE §10, после domain model; только предложение |
| 20. POC | POC_PLAN: девять уточнённых stages, реальный небольшой Babashka stage6; architecture acceptance 30–60min, 24/72h candidate stage9 |
| 21. Без production code | Только Markdown; никакие crates/bindings/server/GUI не создаются |
| 22. Deliverables | Пять требуемых файлов в docs/architecture |
| 23. ADR | Три определяющих решения в docs/adr, статус Proposed, Context/Decision/Consequences/Rejected alternatives |
| 24. Самопроверка | Таблица в §3 этого документа |
| 25. Git | Проверки diff/status выполняются при завершении; изменения пользователя в root-документах сохраняются; без автоматического commit |

## 5. Следующая точка решения

Foundation reconciliation внесло согласованные AF01–AF08: Furnace закрыт; roles/side effects, Query purity, configuration policies, identity staging, recorder metadata и POC sequencing уточнены. См. [Milestone 1 design](../implementation/MILESTONE_1_DESIGN.md). Hardware budgets/profiles, IPC/storage engines и GUI choice не решаются этим milestone. Историческая coverage таблица выше описывает исходный архитектурный этап; актуальные ограничения implementation задаёт AGENTS.md.
