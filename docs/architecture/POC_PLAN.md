# План архитектурного proof-of-concept

Статус: целевой план, согласованный с migration analysis и Foundation 2026-09-14. Сейчас разрешён только этап 1 по [FOUNDATION_MILESTONE1_PLAN.md](../../FOUNDATION_MILESTONE1_PLAN.md); остальные этапы требуют отдельной команды. Архитектура: [HIGH_LEVEL_ARCHITECTURE.md](HIGH_LEVEL_ARCHITECTURE.md), проверяемые инварианты: [RUNTIME_AND_SAFETY_MODEL.md](RUNTIME_AND_SAFETY_MODEL.md). Ниже описаны критерии, а не заявление об их прохождении.

## 1. Что должен доказать POC

Первый POC проверяет совместимость расширяемости, независимого исполнения и единого output authority. Успешная демонстрация PID без отказов этого не доказывает. Нужны наблюдаемые traces: решения arbiter, фактические отправки mock transport, timestamps/generations, queue bounds, controller ticks и durable history.

Минимальный стенд:

- Virtual thermal process: один измеряемый физический смысл — temperature, один actuator — power, управляемые часы и fault injection.
- Один native instrument adapter: по migration feedback предпочтителен минимальный Metakon codec/profile с проверенными fixtures, поверх эмулируемого serial endpoint. Modbus RTU — альтернатива при явном требовании стенда, а не предположение о протоколе Metakon. Это настоящий путь driver → protocol → scheduler, а не только генератор числа.
- Data-driven definition второго логического прибора того же профиля: проверка добавления без изменения Rust. Вспомогательная проверка того же adapter, не ещё один subsystem.
- Один Lua-backed instrument с двумя текстовыми операциями temperature.read/power.set через bounded Rust transaction profile. Эмулятор позволяет проверить sharing одного serial resource с другим instrument при совместимых bus settings.
- Один основной measurement signal с raw/native-filtered/script-filtered производными потоками, один native filter и один Lua filter, один Ramp Reference, один native PID, один OutputArbiter.
- Небольшой recorder и CLI/test harness через domain boundary. CLI может быть in-process adapter для POC; GUI и внешний сервер первому срезу не нужны.

Эмуляция — первый способ проверки отказов без реальных нагревателей. Native adapter в POC не означает готовый production driver. Физический serial smoke test допустим только в следующем отдельно разрешённом срезе с определённым hardware/safety profile. Не ставим feature parity с `com_port_reader` и не используем его код в текущем архитектурном этапе.

## 2. Как измерять результат

До timing tests выбирается небольшой явный POC profile:

| Budget | Что зафиксировать до проверки |
| --- | --- |
| Control period и maximum tick gap | Частота mock процесса и предельная допустимая пауза |
| Input freshness / input skew | Когда sample/reference непригодны для управления |
| Transaction duration / queue wait | Верхние пределы обмена и ожидания общей шины |
| Proposal TTL / lease lifetime | Максимум действия устаревшего producer при потере связи |
| Lua instruction/memory/wall-clock limits | Условия завершения или quarantine |
| Safety decision / safe completion budget | Отдельно время от обнаружения до revoke и время до подтверждения safe procedure |
| Queue/window/history limits | Проверяемые ограничения памяти и политика overflow |
| Recorder flush/loss window | Когда данные считаются durable и что допускается потерять при crash |

Числа выбираются для стенда, не выдаются за требования реальной установки. Virtual clock проверяет семантику deadlines; реальные часы проверяют isolation и измеренную задержку. Оба нужны: тест на fake clock не доказывает поведение VM/ОС под нагрузкой. Test report сохраняет профиль, среду, observed maximum/распределение задержек, размер очередей и все violations. Нельзя считать своевременность доказанной только по среднему времени.

## 3. Девять небольших этапов

### Этап 1. Domain harness и virtual instrument

Создать только два crate: lab-core и lab-runtime. Минимальные registry, descriptor/introspection, локальные синхронные Commands/Queries и native virtual temperature instrument. Power пока только actuator metadata, без write execution. Ввести стабильные локальные IDs, typed values, units, roles/access/side effects, quality, monotonic timestamps и bounded window. Events/cursors, async generations, leases и request deduplication в M1 не нужны; configuration revision — только при реальной потребности, не общий framework.

Проверить: generic client отображает descriptor без instrument-specific branching; rename сохраняет IDs; Query не запускает I/O и не продвигает virtual state; explicit refresh использует заданное monotonic time; invalid type/range/access отклоняются атомарно; failure имеет явное quality и не маскируется старым значением; window ограничен; одинаковый сценарий воспроизводим. Подробный контракт: [MILESTONE_1_DESIGN.md](../implementation/MILESTONE_1_DESIGN.md).

Критерий: trace показывает одного Runtime owner, bounded current/window state и воспроизводимые domain outcomes. Пока только simulation, без физического output.

### Этап 2. OutputArbiter до включения controllers

Реализовать минимальные Unverified/SafePending/Disarmed/ArmedManual/ArmedAuto/FaultLatched, output owner, lease/epoch и trusted simulated dispatcher. Пройти manual proposal, revoke, safe action и confirmation levels.

Проверить конфликт двух owners, outside-limits/NaN/неверные units, expiry, interlock trip, mode change, queued stale write, истечение freshness входа во время ожидания отправки, поздний in-flight result, ambiguous acknowledgement и неуспех safe action. Generic configuration/reset write с side effects тоже должен потребовать permit.

Критерий: у каждой фактической отправки есть актуальное разрешение конкретной операции/значения; после revoke не отправляются новые старые requests; уже начатая write явно урегулируется до safe confirmation. Неопределённость остаётся fault/unknown, а не success.

### Этап 3. Rust-owned serial scheduling и data-driven instrument

Добавить один native instrument implementation, ограниченный known-protocol adapter и эмулируемый serial transport с configurable delays/errors. Зарегистрировать дополнительный instrument через definition, изменив registers/scaling/units без пересборки Rust.

Проверить exclusive port ownership, отсутствие interleaving на одной шине, независимость второй шины при необходимости в harness, queue timeout, CRC/framing/response mismatch, late response после timeout, bounded retry и неопределённую write. Несовместимые bus settings должны отклоняться до запуска; несовпадающая physical identity после reconnect не должна получать даже вслепую повторённую safe command.

Критерий: новый data-driven instrument доступен через тот же descriptor API; native driver не получает отдельный output путь. Safety обслуживается в пределах выбранного профиля с учётом текущей bounded transaction. Устройство, которое не укладывается в профиль, отклоняется admission/health policy.

### Этап 4. Native signal/reference/controller loop

Соединить temperature Source → один native filter → native PID → arbiter → virtual power; Reference — один Ramp. Реализовать только необходимые window/freshness и native controller lifecycle, без generic graph editor.

Проверить warming-up, stale/invalid measurements, units mismatch, большой dt, pause/resume/reset/removal, ограничение выхода и controller feedback о фактически разрешённой цели. Reference progress должен соответствовать явному lifecycle и monotonic clock, в том числе при изменении wall-clock.

Критерий: native loop регулирует эмулируемый процесс; при потере входа прекращает authority в пределах профиля; pause не удерживает старую мощность без safe transition; resume не применяет интеграл за всю паузу. Независимая цепочка не останавливается из-за постороннего client disconnect.

### Этап 5. Ограниченный Lua host

Добавить один Lua instrument и один Lua filter. Проверить bounded transaction catalog и script parser; power template и safe action исполняются Rust. Для проверки controller extension contract использовать небольшой экспериментальный Lua controller в отдельном запуске того же стенда; не строить вторую управляющую систему. Lua filter включать/выключать в цепи native PID с безопасным переходом.

Проверить script exception, бесконечный loop, memory/output quota, завал diagnostics, задержанный callback, reload и старый generation result. Попытки raw port access, подмены адреса/template/value после permit, произвольного host call и изменения operation catalog должны отклоняться. Проверить, что safe action остаётся исполнимой при недоступной Lua VM.

Критерий: новые filter/instrument/controller definitions загружаются без Rust rebuild; все воздействия проходят прежний arbiter. Infinite script не блокирует независимый native PID и watchdog. PID, намеренно использующий просроченный Lua result, переходит в явный fault вместо продолжения на старом значении. Если host calls нельзя надёжно ограничить, остановить допуск Lua к physical-control path и пересмотреть host/process boundary.

### Этап 6. Малый реальный Babashka vertical slice

После Lua, до recorder проверить настоящий local-only IPC adapter и тонкий Babashka helper: discovery, snapshot, явная операция, disconnect/reconnect, malformed/oversized messages и медленный client. Использовать virtual-only стенд без hazardous output. Все действия проходят общую domain boundary; происхождение клиента не меняет safety semantics. Это проверка внешнего контракта, не полный SDK, nREPL server или интернет-платформа. Recording-сценарий дополняется на этапе 7.

### Этап 7. Recorder и реконструкция эксперимента

Добавить минимальный append-oriented storage adapter: SQLite — стартовый кандидат, окончательное решение после проверки бюджета. Session/run metadata, generic typed diagnostics, gaps и durable watermark не должны зависеть от Furnace или nullable числовых колонок v1. Записывать raw/filtered values, applied configuration, script versions, controller transitions, actions и output outcome chain. Для управляемого стенда включить recording-required policy.

Проверить disk error/full через fault injection, writer stall/queue overflow, crash до/после durable boundary и невозможность записать собственный RecordingFailed. Провести Query истории после перезапуска без GUI/client caches.

Критерий: запись объясняет, что было запрошено/разрешено/отправлено/подтверждено; неизвестный хвост явно обозначен. Recorder failure инициирует safe transition, не блокируя safety. Metadata достаточно, чтобы найти фактически использованные scripts/definitions, даже если исходный файл позднее изменён.

### Этап 8. Client lifetime, нагрузка и завершение

Через два test client adapters и реальный Babashka slice проверить multi-client semantics, slow subscriber, command flood, disconnect/reconnect и истечение external/manual producer lease. Fake-clock/fault-injection tests дополнить коротким интегрированным real-time прогоном 30–60 минут; это POC gate, не доказательство многодневной надёжности.

Проверить reconnect snapshot/cursor/gaps, повтор request ID, истечение окна дедупликации, stale ownership, GUI-like close, graceful shutdown при in-flight write и restart после interrupted recording. Safety command capacity должна сохраняться при command flood.

Критерий: native PID/recorder продолжают при отключении supervisory client; external/manual output теряет authority по policy. Повтор неоднозначной write не вызывает слепое второе воздействие. Shutdown сохраняет итог с unknown/failure, если safe confirmation не получено, а restart начинается Unverified без восстановления leases.

### Этап 9. Pre-release soak и архитектурный review

Провести интегрированный 24-часовой simulation run с reconnect/errors/reload/recording rotation и ограниченными очередями; после устранения обнаруженных дефектов — 72-часовой прогон для требования многодневной работы. Ускоренные fake-clock tests дополняют, но не заменяют реальные длительные прогоны.

Проверить рост памяти/числа tasks/instances, очередь recorder, размер history windows, задержки tick/safety, повторные faults и отсутствие накопления quarantine workers. Сверить все инварианты предыдущих этапов по traces.

Критерий: нет необъяснённого роста ресурсов, silent sample gaps и нарушений выбранных timing/authority bounds. Прогон даёт эмпирическое подтверждение профиля, не математическую hard-real-time гарантию. Выпустить отчёт: подтверждённые гипотезы, опровергнутые гипотезы, изменения ADR, остаточные ограничения и решение о следующем slice.

## 4. Матрица риск → свидетельство

| Риск | Где проверяется | Что является свидетельством |
| --- | --- | --- |
| Script обходится с портом как собственник | 3, 5 | Transport trace без interleaving; запрет raw requests и неавторизованных side effects |
| Revoked output применяется из очереди | 2, 3, 8 | Permit epoch при фактическом send; поздний in-flight effect не скрыт |
| Hung Lua блокирует native loop | 5, 8, 9 | Реальные tick/watchdog timestamps независимой native цепи при infinite script |
| Неизвестный прибор требует изменения GUI/Core | 1, 3, 5 | Новое definition/script, тот же binary и generic introspection |
| Статус safe выдаётся после timeout | 2, 3, 8 | Fault/unknown outcome при неоднозначном send/readback; явный recovery |
| Client становится владельцем experiment | 6, 8 | Unchanged Runtime instances и продолжающаяся recording session после disconnect |
| Recording скрыто теряет данные | 7, 8, 9 | Watermark/gaps/interrupted outcome; recording-required fault policy |
| Dynamic replacement оживляет старый controller | 5, 8 | Старые generations/leases отклонены после reload/reconnect |
| Модель слишком сложна | Review каждого этапа | Небольшой набор component contracts; отсутствие generic plugin/workflow framework |

## 5. Babashka и следующий vertical slice

По [AF-08](../migration/ARCHITECTURE_FEEDBACK.md) прежняя рекомендация отложить Babashka за recorder заменена этапом 6: маленький реальный внешний клиент раньше проверяет пригодность boundary. Он не разрешён в текущем M1 и не заменяет port ownership, output fencing, Lua isolation или durability tests.

После появления recorder расширить сценарий этапа 6: discover неизвестный прибор, получить snapshot, настроить Ramp/PID, начать recording, подписаться, добавить structured event, закрыть Babashka и подключиться снова. Native PID/recording должны продолжить; отдельный external-controller test должен истечь по lease. Проверить compatibility rejection. Не нужны GUI, большой SDK, nREPL внутри Runtime и production internet security.

Только после этого выбирать первый физический прибор/стенд, завершать [открытые hardware/safety вопросы](OPEN_QUESTIONS.md) и выполнять отдельно разрешённый hardware slice. Migration analysis уже выполнен; его reuse recommendations не означают автоматического переноса Furnace или готового driver кода.
