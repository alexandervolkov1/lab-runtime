# POC refinement после migration analysis

Статус: предложенное уточнение, не автоматически утверждённая замена [POC_PLAN](../architecture/POC_PLAN.md). Production-код, workspace, Lua/Babashka modules и IPC server в текущей задаче не создаются. Donor baseline/tests — [inventory](V1_FEATURE_INVENTORY.md) и [REUSE_PLAN](REUSE_PLAN.md).

## Рекомендация и её обоснование

Рекомендуется принять девять небольших stages ниже. Это близко к предпочтительному порядку MIGRATION_ANALYSIS_PLAN, но основано на фактическом donor:

- CoreHost/listener independence и operation completion уже представлены в v1; перенос их tests снижает цену раннего external slice. Реального Babashka acceptance нет, поэтому его нельзя заменить ещё одним Rust fake client.
- Native math/codecs — независимые assets; их можно проверять раньше больших runtime services.
- Known physical protocol donor — Metakon, не Modbus. Narrow tested Metakon slice полезнее для migration feedback, чем называть его Modbus reuse. Новый Modbus driver нужен только по реальному bench/product requirement.
- Bounded Lua stage до BB проверяет, что shared model не превращается в Lua-centric application API. После него BB slice проверяет второй extension boundary до больших recorder/workflow/GUI investments.
- Recorder остаётся обязательным до признания physical recorder-required control работоспособным. Stage6 проводится на virtual instrument, а не на hazardous физическом эксперименте без журнала.

### Сравнение вариантов

| Вариант | Выгода | Недостаток | Вывод |
| --- | --- | --- | --- |
| Исходный: real BB только после full POC | Меньше ранней IPC работы | Поздно обнаруживает неудобный для REPL domain contract; fake clients не закрывают product requirement | Не рекомендуется сохранять без конкретного ограничения команды/среды |
| BB сразу после domain/arbiter | Очень ранний user feedback | Ещё нет native signal/reference/PID и bounded Lua boundary, легко спроектировать API под temporary internals | Слишком рано для показательного vertical slice |
| BB после native pipeline и bounded Lua, до recorder | Проверяет coherent domain и interactive workflow, scope ограничен | Нужны минимальные protocol/session/outcome/gap semantics | Рекомендуемый stage6; только virtual/safe fixture, без rich workflow engine |
| Recorder до BB | Ранее проверяет обязательный storage health | Отодвигает одну из главных целей продукта | Допустим при physical bench-first priority; такого требования сейчас нет |

## Изменения относительно исходного POC

| Исходный раздел | Предложенное изменение | Основание |
| --- | --- | --- |
| Domain + virtual | Оставить первым; native virtual, без mandatory VI wire/Lua/GUI | V1 model engine можно отделить от transport compatibility |
| Minimal arbiter | Оставить вторым, ограничить понятия actual tested invariant | Old modes insufficient, но full storage/evidence/API сейчас не нужны |
| Native Modbus + data-driven | Выделить protocol-independent executor и small named-operation definition; Metakon native codec/driver как preferred donor fixture | CRC/frames Metakon проверяемы; Modbus donor отсутствует |
| Native signal/Reference/PID | Оставить; independent monotonic Reference, explicit warm-up/rearm policy | Math сохраняется, lifecycle меняется |
| Lua | Оставить bounded component host, не whole `app` API | V1 native constructors не Lua filter/controller implementation |
| Recorder + simulated clients/faults | Вставить real BB slice stage6; recorder stage7; simulated/fault clients stage8 сохраняются | Реальная interactive boundary + robustness проверяются разными тестами |
| 24/72h soak | Architecture POC: fake-clock/fault и 30–60min real-time; 24/72h перенести stage9 | Deterministic failures дают ранний architecture feedback; длительный runtime confidence позже |

## Stages и acceptance gates

### 1. Domain + native virtual Instrument

Цель: один small end-to-end data path без hardware, Lua, GUI, IPC и SQLite: validated config → instrument identity/descriptors → native virtual read → Signal/latest snapshot. Virtual actuator descriptor допустим, но command execution, способное выступать как output path, не активируется до arbiter stage2; configuration-only model edits валидируются отдельно.

Donor: C09/C15/C21 descriptor/model knowledge, pure fake values/seeded sine ideas; T04/T05/T06 validation/identity tests. Не копировать VI frame/server/thread, SeriesStore или Lua profiles.

Gate: typed access/range/unit checks; stable ID при rename; invalid config не меняет instance; deterministic injected time; bounded latest/window memory; no unbounded global history, no presentation dependency. Local Query snapshot достаточен. Error и unavailable measurement различимы; детали multi-node DAG ещё не нужны.

### 2. Minimal OutputArbiter + simulated dispatcher

Цель: virtual actuator управляется только через Rust authority. Минимальная проверяемая state machine использует target states Unverified/SafePending/Disarmed/ArmedManual/ArmedAuto/FaultLatched, но storage/wire representation не проектируется заранее целиком.

Donor: T09 manual exclusion/stale controller/ACK order/safe/rollback fixtures, C35 conversion. Новая implementation, не rename v1 modes.

Gate: explicit safe profile/arm, one owner, lease/TTL и epoch revocation, fresh proposal checks; delayed queued proposal после revoke не отправляется; safe failure не выдаётся за Disarmed; first ACK/evidence отдельно от authority state. Injected clock/dispatcher делает safe/failed/unknown outcomes воспроизводимыми. Safe path не зависит от Lua/client. Minimal evidence — desired/sent/success/failure/unknown simulated result, не полная persisted taxonomy.

### 3. Transport / protocol / data-driven Instrument slice

Цель: bounded single-owner serial executor с typed named operations и deadline/retry policy; один narrow native Metakon driver плюс small declarative definition fixture, проходящая те же validators/authority. Не строить универсальный language/driver compiler.

Donor: C01/C03/C08/C10/C12, pure Metakon vectors T03; virtual frame/memory tests T04 для optional byte-stream harness. Metakon channel/measurement/output subset достаточно; full register coverage позже. Если реального Metakon bench нет, native codec и fault-injected transport проверяются без COM; hardware acceptance честно остаётся незакрытым.

Gate: transaction не interleaves на bus; different connections изолированы; total deadline, no unsafe write retries, late/partial/invalid response recovery, rebind generation, final permit check непосредственно перед send. Добавить minimal physical evidence distinction sent/ACK/readback/unknown, не утверждать observed register = physical effect. Полезны sensor sentinel и nonrepresentable write tests. Normal experiment со включённым физическим actuator до recorder-stage не считается supported; bench test возможен только с отдельным безопасным hardware plan/limits/interlock.

Modbus: если выбран первым bench, отдельные codec/driver/tests должны быть предусмотрены явно; Metakon CRC не переиспользуется. Это меняет donor choice, не transport/domain boundary.

### 4. Signal + Reference + native PID

Цель: source → native EMA → independent Fixed/Ramp Reference + native PID → output proposals/arbiter → virtual plant; diagnostics как Signals. Controllers — не произвольные DAG filters.

Donor: C23/C26/C31, T06/T07/T08. OnOff/Furnace math fixtures можно исследовать/извлекать отдельно, но их full runtime parity не gate этого stage.

Gate: variable monotonic dt, no derivative kick, local anti-windup, invalid state update atomic, scoped quality/freshness/warm-up, bounded window, config revision and explicit reset/retention policy. Reference прогресс не зависит от wall-clock jumps/GUI polling; ramp target/rate continuity retained. Output never bypasses arbiter. Repeated-tick drift/overrun/skip tests нужны сверх v1 interval tests.

### 5. Bounded Lua extensions

Цель: один Lua virtual/model component и один bounded processing callback, использующие target typed contracts; native pipeline остаётся работоспособным при failure VM. Минимальный callback может быть filter; Lua controller proof ограничить scalar proposal через тот же arbiter, если это требуется для проверки contract, не целой библиотекой algorithms.

Donor: C18 strict dynamic schema/state tests, T05/T11; v1 `app.filter/plant:pid` — не callback implementations и не переносятся как proof.

Gate: instruction/time/memory/state-size limits, library/capability allowlist; no raw COM/OS/FFI; host operations имеют собственные budgets; callback failure выдаёт quality/fault по policy; reset/replace releases state; safe path работает без VM. No global experiment-manager `app`, no GUI panel host/full Lua REPL. Embedded VM не выдаётся за crash isolation.

### 6. Small real Babashka / external API vertical slice

Цель: настоящий отдельный Babashka process/REPL управляет одним running Runtime на virtual fixture через небольшой local API. Конкретный wire transport — adapter choice; NDJSON utility/tests donor полезны, но весь v1 operation catalog не нужен.

Минимальные operations: hello/capabilities, describe/catalog, snapshot/latest/status, subscribe/unsubscribe, configure existing Reference/controller, deliberate lifecycle/safe command, operation status. Small recipe: получить fixture handles → subscribe → изменить ramp target/PID setting → дождаться Applied → посмотреть diagnostics → безопасно pause. Configuration/fixture setup может быть заранее задана Runtime, не обязательно dynamic create всего graph.

Donor: C44/C45, T14; [passive profile](../../../com_port_reader/profiles/core_furnace.lua) как requirement «client starts deliberately», не target config syntax. T12 stage/completion idea использовать без переноса ScenarioService.

Gate: REPL calls ergonomic and typed; invalid config одинаково отвергается internal/external path; accepted ≠ applied; correlation/finite in-memory dedup/outcome_unknown, bounded sessions/events. Потеря BB process не останавливает locally viable native controller; external/manual authority expires по lease/TTL. Reconnect получает current snapshot; dropped events явно signal gap и требуют recovery; duplicate command не выполняет иной payload под тем же retained ID. Нет exactly-once guarantee после eviction/restart.

Использовать virtual output и in-memory diagnostic/test sink; full durable recording не имитировать. Physical recorder-required operation запрещена, пока stage7 не закрыт. Если BB отсутствует в implementation environment, stage не считать завершённым по Rust TCP tests: потребуется установить/предоставить среду по отдельному workflow, а не silently replace test.

### 7. Recorder

Цель: bounded recorder port/service + SQLite adapter candidate, typed records, session/run/config provenance и required-recording safety behavior.

Donor: C36/C38 writer/transaction patterns и T10; C37 service wrapper переписывается. Old database format и import не scope.

Gate: raw/derived/diagnostic/control/evidence/annotations связаны identity/time/revision; byte/count budgets, explicit loss/overflow semantics; observer loss ≠ durable sink failure; injected disk error/slow writer не блокирует control calculations и вызывает required safe/fault. Normal shutdown retires producers → drains/finishes within defined outcome budget; finalization failure отражена в report. WAL/durability limitations documented. После этого можно планировать отдельную физическую acceptance с installation-specific safe profile.

### 8. Runtime failure/load tests и architecture POC acceptance

Цель: проверить boundary/lifecycle/queue assumptions под fault и умеренной реальной нагрузкой, не максимальный throughput.

Donor: T02/T09/T10/T13/T14 и thermal/virtual fixtures; дополнения T16.

Gate:

- fake-clock stress с большим числом ticks, lease expirations, monotonic progress, drift/overrun/restart;
- fault injection: missing/stale sample, bad frame/late ACK, disconnect, full queue, recorder fail, Lua timeout/memory exhaustion, obsolete generation/revision и in-flight write during revoke;
- ordered shutdown/reload, admission closed, safe failures truthful, no stale retired writes, bounded recovery;
- минимум один 30–60 minute real-time integration run: native virtual loop + bounded Lua component + actual BB reconnect + recorder, без GUI;
- inspect memory/task/thread/queue/window counters до/после run и repeated start/stop; нет монотонного накопления resources после прогрева, gaps/errors наблюдаемы;
- API work и slow subscribers не ухудшают documented control/deadline budget вне разрешённых границ.

Конкретные queue sizes/deadline tolerances выбираются до запуска для заявленного workload и записываются в acceptance report. Не подменять boundedness отсутствием OOM за один короткий запуск. Fake time не заменяет real serial/disk scheduling, а real 30–60min не доказывает 72h reliability.

### 9. Runtime candidate / pre-release hardening

После architecture POC: 24h, затем 72h runs на выбранной candidate configuration, long-lived recording/storage growth, repeated client reconnects, fault recovery и process/resource accounting. Physical runs только после отдельной bench safety acceptance. Не блокировать первые architectural commits требованием 24/72h.

Если длительные tests обнаруживают leaks/deadline drift/DB issues, возвращаться к соответствующему subsystem gate. Stage9 не доказывается ускоренными virtual timestamps. GUI/rich recipes/full devices могут расширять отдельные product milestones, но не заменяют reliability gate.

## Когда нужны архитектурные понятия

First milestone означает первую реальную потребность, а не обязательство строить полный generic type/system. Если implementation вводит async replacement раньше, matching generation/fence вводится вместе с ним.

| Concept | Зачем | First milestone / минимальный scope |
| --- | --- | --- |
| ID | Stable instance identity, rename без потери binding | 1: Instrument/Parameter/Signal IDs; не string name и не global distributed ID scheme |
| generation | Отсечь результаты/handles старой instance после recreate/rebind | 2: async owner/dispatcher fixture replacement; обязательно 3 для transport/model restart |
| revision | Отличить config/descriptor changes от instance lifetime | 3: validated operation/definition binding; 4: atomic controller/filter/reference reconfiguration |
| epoch | Немедленно отозвать authority queued proposals | 2: arbiter authority counter + dispatch recheck |
| sequence | Order/duplicate/gap knowledge in samples/events | 1: минимальный sample order при polling; 6: separate event/subscription sequence, не смешивать namespaces |
| cursor | Bounded subscription/history continuation/recovery | 6: только после реальных subscriptions; opaque position/scoped gap handling, не durable replay engine |
| request ID | Сопоставить wire request/reply | 6: per-session client requests; раньше достаточно internal operation handle |
| correlation ID | Связать proposal/dispatch/result и later audit | 2: minimal local operation correlation для delayed completion; 3: transport evidence; 6/7: external/audit linkage |
| lease | Explicit owner authority with lifetime | 2: local simulated manual/auto owner; 6: external client ownership, без отдельного lease framework заранее |
| TTL | Запрет stale proposal/owner/input и finite lifetime permit | 2: fake-clock proposal/lease deadline; 4: real signal freshness; permit ≤ все applicable expiries |
| evidence | Не путать authority, desired, sent и observed result | 2: minimal simulated result; 3: sent/ACK/readback/unknown physical distinction; 7: persisted provenance; full generic taxonomy не day1 |
| snapshot | Coherent bounded view без shared mutable GUI/runtime state | 1: local latest/descriptor view; 6: versioned external state recovery |
| deduplication | Не повторять accepted operation при network retry | 6: bounded in-memory retained requests/outcomes and conflict semantics; не before IPC, не persistent exactly-once |

## Что реально переносить уже в POC

Pure descriptor/value validation cases, Metakon CRC/codec vectors и narrow register/scaling rules; virtual server/model contract knowledge; EMA/PID/Fixed/Ramp math; output exclusion/late-completion/safe/rollback tests; shutdown/reload and host/listener tests; SQLite single-owner writer patterns позже stage7. Runtime wrappers, public enums/DTO и application Lua API не копировать.

Не включать в early POC: complete Metakon/Modbus suite, all controllers/filters, generalized workflow engine, GUI/control panels, old SQLite import, full DSL, native ABI plugins, complete evidence storage from day1. Это scope control, не исключение из будущей product parity.

## Первый будущий implementation milestone

Milestone 1: минимальный native virtual Instrument с typed parameter descriptors/validation, stable identity, deterministic injected clock, bounded Signal/latest state и local command/query façade без output authority activation. Нужны tests invalid read/write/config types/ranges/access, rename identity, bounded storage, controlled error/unavailable result. No hardware, GUI, IPC, Lua, BB, SQLite и no wholesale v1 code import.

После gate milestone1 отдельно переходить к simulated OutputArbiter milestone2. Даже создание Cargo workspace — действие будущей implementation phase по отдельной команде пользователя, не часть текущего migration analysis.

## Оставшиеся внешние решения

Из code нельзя определить первый реальный bench/protocol, installation safe values/limits/verification/hardware watchdog и required sampling/deadline/durability workload. До physical acceptance эти параметры должен задать владелец оборудования. Rich GUI technology и v1 historical DB compatibility нужны только на соответствующих product stages, не блокируют virtual POC.

Рекомендация early BB/Metakon donor/short architecture soak передана на review этим документом; исходный POC_PLAN не переписан. Это завершает migration planning, а не начинает implementation.
