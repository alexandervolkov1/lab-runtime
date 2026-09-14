# v1 → lab-runtime: основной план переноса

Статус: analysis / рекомендации для будущей implementation. Никакой реализации этим документом не создаётся. [Baseline](V1_FEATURE_INVENTORY.md#baseline-и-границы-достоверности), [классификация компонентов](V1_COMPONENT_MAP.md), [тестовые активы](REUSE_PLAN.md#тесты-как-отдельный-актив).

Документ идёт сверху вниз по target subsystem, а не воспроизводит дерево v1. Domain policies относятся к будущему `lab-core`; physical I/O, storage и local API — к host adapters `lab-runtime`; bounded Lua — к `lab-lua`. Это роли из текущей архитектуры, не предложение создавать дополнительные micro-crates. [Architecture feedback](ARCHITECTURE_FEEDBACK.md) и [POC refinement](POC_REFINEMENT.md) не изменяют исходные ADR/POC молча.

## 1. Autonomous Runtime / experiment lifecycle

Relevant v1: [ApplicationRuntime](../../../com_port_reader/src/application_runtime.rs), handlers, [CoreHost](../../../com_port_reader/src/core_runtime/host.rs), [RuntimeSession](../../../com_port_reader/src/core_runtime/session.rs). Classification: REWRITE ownership/application services; EXTRACT lifecycle knowledge (C42–C44).

В v1 уже решены существенные задачи: один owner thread для graph/controllers; worker-owned transport; отдельный Lua worker; output service отдельно от math; safe pause всех controllers до остановки транспорта; stop как barrier для queued start; stale retired runtime не пишет safe поверх replacement. Текущий CoreHost поддерживает pumping без GUI frames; standalone JSON listener можно остановить, сохранив acquisition/recording. Это реальные доноры требований, а не отсутствующая headless functionality.

Но future Runtime не является rename ApplicationRuntime. GUI-host snapshot содержит SeriesStore и LuaHandle; runtime конфигурация включает presentation; mandatory application Lua и scenarios управляют экспериментом; termination зависит от blocking recv/join. Кроме того, `dispatch_command` добавляет safe-pause barrier перед emulator stop только для origin `ExternalCore`; GUI/Lua сохраняют прежний путь. Это конкретный semantic split: общий transport/host ещё не означает единый command contract. Target должен владеть экспериментом независимо от всех clients, разделять lifecycle experiment/instrument/controller/recorder, иметь общие safety semantics, admission/quiesce/drain/deadline policy и не восстанавливать output authority автоматически после restart.

Reload сохранить как validate candidate → revoke/safe old outputs → stop/release old transports → activate candidate → publish new generation. Failed validation не трогает активный эксперимент; failed safe оставляет recovery path и запрещает replacement; failed activation после release не означает rollback hardware effects/auto-resume old controller. Для общего lifecycle нужны явные bounded outcomes, а не ожидание Drop как safety mechanism. Tests: T13; staged generation/epoch — [POC](POC_REFINEMENT.md).

## 2. Transport executor

Relevant v1: SerialConnection/config registry, worker runtime/router, SerialCommandSource, protocol transport loops (C01–C08, C11). Classification: ADAPT low-level port adapter; EXTRACT scheduling/validation; REWRITE executor/application queues.

Сохранить:

- один владелец connection и последовательность целого request/response, а не отдельных reads/writes;
- explicit serial settings, open/close/errors, distinct connections и per-series intervals;
- no fallback after owning-source error; start rollback/reverse stop; stop completion только после cleanup;
- cadence/deadline bookkeeping и isolated failure counter, отдельно от output safety.

Изменить:

- executor получает заранее разрешённую typed transaction с total deadline, response expectation, bounded retry/idempotency/recovery policy;
- protocol codec знает bytes, Instrument adapter — смысл operations, scheduler — polling; executor не знает SeriesStore/GUI/Metakon enums;
- один физический bus остаётся одним executor даже при нескольких Instruments. V1 profile уже запрещает case-insensitive повтор COM names; target дополнительно требует canonical binding/rebind semantics, а не второй mutex на тот же port;
- bounded polling/management/output queues, справедливость с ограниченной задержкой safety action. V1 due-poll-first может задержать command; текущая cadence `previous_deadline + interval` с skip не должна превратиться ни в busy catch-up, ни в бесконечный drift;
- перед фактическим send проверяется текущий output permit/epoch/expiry/interlocks. Проверка только в OutputService при enqueue недостаточна;
- timeout запроса не доказывает absence of physical effect; late reply/partial frame/ambiguous write требуют recovery, не blind repeat.

Что останется от AcquisitionSource: знание поддерживаемой операции и изоляция ошибок. Сам trait с `sample(SeriesMetadata)` и family-specific writes переписывается. CombinedSource как общая domain abstraction не нужен: explicit Instrument binding выбирает один route. Его error/rollback tests нужны (T02/T04). SerialCommandSource распадается на transport adapter, Metakon Instrument driver и polling task; sensor sentinel не живёт в generic acquisition.

## 3. Protocol codecs и native Metakon Instrument

Relevant v1: [metakon protocol](../../../com_port_reader/src/protocol/metakon.rs), [Metakon5x3](../../../com_port_reader/src/instrument/metakon_5x3.rs), [serial source](../../../com_port_reader/src/acquisition/serial_command_source.rs). Classification: EXTRACT codecs/register knowledge; REWRITE retries/transport glue; ADAPT descriptor projection (C10–C12).

Metakon — собственный binary protocol: READ `0x00`, WRITE `0x01`, custom one-byte CRC initialized `0xFF`, little-endian types, address/channel/register/op/type/length validation, frame до 38 bytes. Это не Modbus RTU и его CRC нельзя использовать как Modbus CRC. Тип канала `0x03` подтверждает family/channel compatibility, не уникальный serial identity прибора.

| Слой target | Donor knowledge | Что не переносить |
| --- | --- | --- |
| Serial transport | SerialConnection config/exchange mechanics | Protocol branches, implicit retries, raw user authority |
| Metakon codec | CRC vectors, encode/decode, response matching/type tags, ASCII bounds | Concrete SerialConnection dependency |
| Metakon Instrument | Register keys/access/ranges/scaling, channel verification, sensor fault | Hardcoded family enum во всех consumers, fault as generic acquisition string |
| Actuator operation | Validated representable value → typed write; expected ACK/readback | Writing any numeric register as interchangeable actuator, retries of ambiguous write |
| Evidence | ACK и subsequent register read — разные observations | Readback register value как доказательство applied heating power |

### Register/data-driven mapping

| V1 metadata/behavior | Target descriptor/adapter rule |
| --- | --- |
| `channel_type`, address `0x00`, read-only | Identification operation, expected type 3; не Signal/Actuator по умолчанию |
| `measurement`, `0x01`, raw -999…9999, sentinel -32768 | Measurement parameter + poll operation, engineering scale/unit; sentinel → explicit sensor-fault quality, не f64 sample |
| `setpoint`, `0x02`, и `proportional_band`, `0x03` | Typed configuration parameters, scale/range/granularity; P-band raw минимум 1; не автоматически actuator capability |
| `integral_time`, `0x04` | Raw seconds 1…30000 → engineering minutes через 1/60; independent of user scale. Numeric register не кодирует front-panel OFF |
| `derivative_time`, `0x05` | Seconds, raw 0…255, unscaled |
| `output_power`, `0x06` | Explicit actuator parameter %, -100…100; разрешённый operating/safe range определяется конкретной installation, не demo zero assumption |
| `pwm_positive/pwm_negative`, `0x07/0x08` | Read-only Boolean states; diagnostics не command authority |
| `upper_setpoint/lower_setpoint`, `0x09/0x0C`; hysteresis `0x0A/0x0D` | Threshold/hysteresis config: scaled bounds, hysteresis raw 0…255 |
| `upper_output/lower_output`, `0x0B/0x0E` | Boolean writable parameters. Нужна явная semantic classification; bool не автоматически excluded из future actuator model |

Native driver — разумный первый donor, потому что behavior выходит за пределы простой register table: channel verification, sentinel, write ACK+readback и representability. Data-driven definition может описать addresses/types/access/scale/units/limits/operation templates/fault codes; сложные проверки остаются native adapter или bounded callback с тем же контрактом. Полностью исполняемый generic driver из metadata в v1 отсутствует.

До использования таблиц нужны provenance/version и cross-field validation. Rust renderer/driver исполняет только named validated operations; definitions/native adapters — доверенная часть, byte table сама не доказывает отсутствие побочных эффектов. Golden vectors T03 сохранить почти без изменений; separate tests должны покрыть timeout after send, stale response и отсутствие неразрешённых repeat writes.

## 4. Common Instrument model и extension levels

Relevant v1: [instrument](../../../com_port_reader/src/instrument.rs), [virtual descriptors](../../../com_port_reader/src/instrument/virtual_instrument.rs), [API projection](../../../com_port_reader/src/core_api/instruments.rs), [catalog](../../../com_port_reader/src/core_runtime/instruments.rs). Classification: ADAPT typed knowledge, REWRITE public family-independent contracts (C09, C45).

V1 уже показывает, какие данные нужны generic клиенту: identity, parameter key/name/type/access/range/unit, supports-series и metadata enumeration. Но current physical descriptors static; virtual IDs позиционные; v1 JSON считает writable non-Boolean parameter подходящим output target. Из этой эвристики следовало бы, что `kp` или `noise_amplitude` — actuator; это неприемлемый target contract.

Target metadata должна явно различать measurement/configuration/actuator/action и side effects, содержать typed validation и units, operation capabilities, safe-profile binding и конфигурационную revision. Identity/generation не заменяется именем или COM address. Catalog refresh должен отличать descriptor sameness от нового model instance и исчезновения прибора: текущий virtual refresh сохраняет unchanged entries и сам не удаляет исчезнувшие как unavailable. Это уточнение runtime lifecycle, а не аргумент за глобальную hot-plugin framework.

Native, data-driven, bounded Lua и virtual implementation предоставляют один Instrument contract. Raw byte/text escape hatch через Lua/GUI/Babashka не сохраняется. Чтение с физическим I/O — named refresh/read operation с budget; Query возвращает snapshot без скрытого ожидания hardware.

## 5. Virtual Instrument / emulator

Relevant v1: frame/message codecs, client/server, memory adapters, LuaVirtualInstrumentModel, DeviceEmulatorHandle/Service, furnace/sine models (C13–C21). Classification: EXTRACT generic engine/codecs, ADAPT models/lifecycle, REWRITE runtime bindings.

Существующие части решают три разных задачи:

| Назначение | Перенос |
| --- | --- |
| Generic model engine: catalog, typed read/write, state, elapsed time, validation | First-class Virtual Instrument через тот же runtime registry, polling, signals и output authority; native model возможна без wire server |
| Wire compatibility: VI frames, CRC16, client/server, memory byte stream, optional serial linked ports | Optional adapter и valuable protocol harness; не mandatory round-trip для native in-process virtual |
| Lua model host: own VM, `instruments/read/write`, conversion | Bounded component host, stable manifest binding, budgets и allowlisted environment; не application `app` VM |

Сохранить init-before-publish, stop/restart/endpoint replacement, distinction device error vs broken session. MemoryClientTransport отслеживает недочитанный ответ и не отдаёт late response следующему exchange; без protocol request IDs это критическое знание. `clear_input` физического COM само по себе не исключает поздние bytes.

Bounded byte queue, transaction deadline и generation required для нового lifecycle. V1 memory queue unbounded; custom emulator transport должен сам возвращаться из read; stop flag наблюдается между callbacks/I/O, не прерывает зависший native call. Lua 500 ms hook не превращает это в process isolation.

Сохранить deterministic noise/equations как fixtures. Thermal plant (lagged delivered power + heat capacity/loss) и Furnace controller model — разные объекты; их параметры можно намеренно mismatch. Fake clock должен поставлять elapsed явно. Для большого dt не выполнять неограниченное число model substeps в control lane. T04/T05/T08; serial integration не требуется для minimal virtual milestone.

## 6. Signals, Series, Filter/Transform DAG

Relevant v1: SeriesStore/Sample, filters/graph/service, controller diagnostics (C22–C25). Classification: REWRITE storage/ownership; EXTRACT algorithms; ADAPT graph behavior.

| V1 понятие | Target |
| --- | --- |
| `SeriesSource::SerialCommand/Instrument` | Polling Source, создающий typed Signal samples |
| `Filtered {input, definition}` | Filter node DAG; Transform role может содержать другие преобразования, не обязан наследовать enum filters |
| `ControllerDiagnostic` series | Diagnostic Signal с controller identity/config revision; requested output не actuator evidence |
| `Sample {timestamp:f64,value:f64}` | Value + unit + quality + monotonic/wall timestamps + identity generation/sequence |
| SeriesStore metadata и ID allocator | Signal registry; rename не меняет stable ID |
| Unbounded history Vec | Bounded latest/window buffers для live use; durable history через recorder query |
| Color/visible/pane | Client presentation metadata; visibility не выключает poll/record |

V1 missing value обычно означает отсутствие sample/error log; target нужен explicit invalid/stale/gap quality. Три failed polls — scheduling health, но consumer input может стать unsafe раньше по freshness TTL. После read/write/retry нельзя автоматически давать controller authority до fresh validated sample.

| Filter | State/config/timing v1 | Сохранить и уточнить |
| --- | --- | --- |
| EMA | last filtered output/time, positive time constant; alpha = `-expm1(-dt/tau)`; first sample=input | Numerically stable alpha, uneven dt; retune может сохранять current value/clock. Explicit reconfiguration policy/revision и warm-up quality |
| Moving average | bounded VecDeque + sum, sample-count window 1…100000; partial window mean | Sliding window/math; configured workload bound для target, overflow checks результата; time window не подменять sample count |
| Median | bounded window + scratch sort; odd configured size; partial even count → mean middle pair | Spike behavior/order; bounded CPU budget/window policy, warm-up status |

Все три проверяют finite/increasing inputs; computed filter output не имеет столь же полного finite-before-commit guard, как PID. Extreme finite values/overflow — missing test, не заявленный найденный production incident. Graph tests содержат cycle/duplicate/removal/downstream reset знания. Branch failure сейчас aborts returned batch после возможных earlier mutations; target must publish scoped quality/errors и определить commit semantics, не объявлять транзакционным весь произвольный DAG.

Новых Lua Filter/Controller algorithms в v1 нет: `app.filter`, `plant:pid/on_off/furnace` создают native enum variants. Callback contract с input/state/output/budgets/failure/reset нужно спроектировать по target, а не «выделить из v1». Lua callback не исполняет physical write. Tests T06/T11; controller timing quality не делегируется UI.

## 7. Independent Reference

Relevant v1: FixedReference/RampReference/ReferenceRuntime и ControlLoop (C29, C31–C32). Classification: EXTRACT algorithms/config validation; REWRITE clock/ownership.

Fixed/Ramp value-at-elapsed почти переносимы напрямую: positive finite rate, вверх/вниз, clamp в target, finite span. V1 ReferenceRuntime хранит elapsed и previous input timestamp: первый sample не сдвигает ramp, subsequent samples двигают её; pause/resynchronize сохраняет elapsed и убирает previous timestamp. Target Reference является самостоятельным компонентом со своим monotonic progress, не частью PID и не вычислением от системных часов measurement.

Сохранить atomic reconfiguration и continuity: изменение ramp target/rate без explicit start rebases от текущего reference value; explicit start/replacement/reset перезапускает progress. Controller setpoint при active Reference read-only. Перенести tests как contracts с injected clock и independent reference binding.

Default target resume по текущей safety architecture требует deliberate restart/reinitialization, не implicit retention v1. Возможность preserve progress/integral допустима только как явно выбранная и проверенная policy после fresh input/safe transfer, не обязательная legacy default. Program и Script Reference отсутствуют в v1 и откладываются за пределы Fixed/Ramp POC. Shared reference может обслуживать consumers, но coupling controller pause → shared Reference pause требует explicit owner policy.

## 8. Native controllers

Relevant v1: [PID](../../../com_port_reader/src/process_control/pid.rs), [OnOff](../../../com_port_reader/src/process_control/on_off.rs), [Furnace](../../../com_port_reader/src/process_control/furnace.rs), ControlLoop/Registry (C26–C30). Они не зависят непосредственно от GUI/Lua runtime в math, но совмещают algorithm с InstrumentValue/ParameterDescriptor dispatch. Это EXTRACT, не copy whole modules.

| Аспект | PID | OnOff | Furnace |
| --- | --- | --- | --- |
| Algorithm state | Integral, previous timestamp/measurement | Active bool | Integral, filtered measurement rate, previous timestamp/measurement |
| Configuration | SP, nonnegative Kp/Ki/Kd, finite min < max | SP, hysteresis ≥0, finite off/on; off/on не обязаны быть 0/100 | SP, nonnegative Kp/Ki; ambient ≥-273.15 °C, max_power >0, lag/loss ≥0, min < max |
| Inputs | One scalar measurement/time; SP or Reference via wrapper | One scalar measurement; timestamp validated finite, не dt | One scalar temperature/time; Reference задаёт SP через wrapper |
| Output | Clamped scalar proposal; not write authority | Off/on scalar proposal | Heater percentage scalar proposal, clamped |
| Diagnostics | P/I/D/output/unconstrained/SP; internal saturated flag | Output/SP; internal active flag | FF/P/I/output/unconstrained/predicted measurement/rate/SP; internal saturated flag |
| Timing | Strict increasing sample timestamp; first sample P only; D=-Kd·dMeasurement/dt | No dt/increasing-time check в math | Strict increasing dt; filtered rate alpha от lag/3, predictor uses lag |
| Error atomicity | Validate candidate output before state commit | Validate config/finite inputs before switch | Invalid sample preserves integral/rate/previous state |
| Reset | Integral+previous cleared; reset_integral clears only I | Active=false | Integral+rate+previous cleared; reset_integral only I |
| V1 resume | resync previous=None, integral retained | resync noop, active retained | resync previous=None/rate=0, integral retained |
| Target pause/resume | Runtime revokes output, default deliberate reset/rearm; retention separate policy | Same authority lifecycle; math off-state не hardware safe proof | Same authority lifecycle; rate must reinitialize before resumed prediction |
| Dependencies to remove | Descriptor/InstrumentValue dispatch from math; registry/target wrapper | Same | Same + do not bind model config to Lua/UI defaults |

| Granule | PID | OnOff | Furnace | Migration contract |
| --- | --- | --- | --- | --- |
| Algorithm | EXTRACT | EXTRACT | EXTRACT | Preserve numerical kernels and validation tests; pure operations may be REUSE |
| Runtime wrapper/registry | REWRITE | REWRITE | REWRITE | Own scheduling/state lifecycle, freshness/quality, revision, diagnostic publication; output proposal only |
| Configuration API | ADAPT | ADAPT | ADAPT | Common descriptors + atomic cross-field validation, explicit state preservation/reset policy |
| Lua exposure | REWRITE | REWRITE | REWRITE | Thin common commands externally; embedded Lua component contract is separate, not native constructor API |

PID и Furnace используют conditional integration относительно собственных output limits; actual applied value/arbiter restriction не поступает в них. При external limiting или manual override integrator может не соответствовать applied output. Сначала target lifecycle подавляет такие updates/reinitializes при rearm; tracking/bumpless transfer с явной feedback policy — отдельное улучшение, не обещание автоматического сохранения физической траектории. V1 anti-windup tests остаются math assets, но не доказательством arbiter-aware anti-windup.

### Furnace: ответ на открытый архитектурный вопрос

Furnace — один native controller по внешнему контракту и композиционный алгоритм внутри:

`thermal-loss model → feed-forward(SP)` плюс `filtered dT/dt → predicted T = measured T + heater_lag × rate → PI(SP - predicted T) → sum/limits`.

Radiative term использует Kelvin и нормировку на `(1273.15^4 - 293.15^4)`; feed-forward переводит ожидаемые linear+radiative losses в % от max_power. Это не PID с переименованными коэффициентами, не simulator самой печи и не orchestration workflow. Reference не входит в Furnace math.

Рекомендация: native `Furnace` component с приватными model/feed-forward/predictor/PI pieces и отдельной Reference binding. Публичный subgraph для каждой части сейчас не нужен. Диагностики нужно сохранить отдельно, а не ужимать в общие PID P/I/D nullable columns. [Comparison tests](../../../com_port_reader/src/process_control/furnace_comparison_tests.rs) ценны как deterministic metrics/mismatch/saturation/manual-transition fixture, но [documented baseline](../../../com_port_reader/docs/furnace-controller-comparison.md) прямо не демонстрирует превосходство текущего Furnace tuning над PID. Demo и comparison имеют разные heat capacity; numerical thresholds переносятся только вместе с точной model/config/dt.

## 9. OutputArbiter / trusted dispatch

Relevant v1: OutputControl/service/OutputTarget/conversion, controller command handler, ProcessControlDispatcher (C33–C35). Classification: REWRITE authority state machine/dispatch; EXTRACT exclusion, conversion, completion, rollback tests.

V1 target хранит connection+parameter, optional safe scalar и controller instance. `AutomaticPending` означает requested takeover до matching successful write completion; `Automatic` при новой регистрации может существовать ещё до первой записи. `Manual` означает exclusion of controller proposals, но не наличие безопасного physical value. `last_applied` — acknowledged completion/value, не универсальное applied-effect evidence.

### Semantic mapping, не переименование modes

| V1 event / invariant | V1 behavior | Представление в target / сохраняемое знание | Новые guarantees / сознательно изменённое поведение |
| --- | --- | --- | --- |
| Instrument/target discovered | Нет отдельного output verification lifecycle | `Unverified`; explicit binding, safe profile и capability validation | Presence/address ≠ safe/authorized; нет автоматического arm |
| Safe verification | Safe request optional, отдельного SafePending нет | `SafePending` → `Disarmed` только при требуемом для profile evidence | Failure/unknown outcome → `FaultLatched`/unverified physical state, не optimistic Manual |
| Controller registration/start | Register сразу `Automatic`, loop Running | Registration без authority; safe verification/disarmed precedes explicit `ArmedAuto` lease | Первая команда не разрешена только фактом создания controller |
| Exclusive owner | Один controller на ConnectedParameterAddress | One current owner/lease на canonical ActuatorHandle | Identity generation+epoch; aliases не создают вторую authority |
| First automatic write | Pending завершён только matching successful completion; initial Automatic особый случай | `ArmedAuto` означает authority, не proof of effect; pending command/evidence отдельны от mode | Permit для конкретного proposal; нельзя требовать ACK до выдачи permit и создавать цикл |
| Manual override | После успешного enqueue → `Manual`; math продолжает updates, proposals rejected | Revoke auto epoch → safe transfer → explicit `ArmedManual` lease → validated manual command | Safety-first transfer по target, queued old auto не проходит final send check; nonzero manual не safe state |
| Failed manual enqueue | Mode остаётся Automatic | Preserve rejected-command observability/atomic local validation | Не обязаны восстанавливать auto после уже выполненного revoke/safe action; никаких optimistic rollback hardware effects |
| Pause | Safe request, pause math даже при ошибке; wait write response | Revoke authority → `SafePending`; successful safe → `Disarmed`, failure → fault; computation pause отдельно | Paused всегда без output authority; old queued proposals fenced |
| Resume | `Manual` → `AutomaticPending`, resume math; rollback transition при failure | Validate fresh input/config/Reference, explicit reset policy и new lease → `ArmedAuto`; first result tracked отдельно | Old epoch не resurrect; safe-first default, no automatic retention/rearm |
| Installation rollback | Register output, install loop, unregister if install fails | Candidate validation/commit or rollback before authority activation | No physical write during incomplete installation |
| Explicit safe output | Safe value должен быть задан, иначе error | Trusted SafeProfile operation path, независимо от Lua VM/client | Zero не universal safe; priority/deadline/evidence и hardware fallback assumptions explicit |
| Failed automatic write | Failure recorded/logged, mode не обязательно изменяется | Policy revokes/fault-latches при failed/ambiguous safety-relevant output | Desired/sent/ACK/readback/unknown различены; нет silent continue/retry |
| Late/out-of-order completion | Instance+transition/completion checks не дают старому ACK восстановить mode/value | Preserve tests with generation/epoch/request correlation | Final permit check важнее post-hoc ACK rejection; sent in-flight write может ещё иметь эффект |
| Controller removal | Safe pause must succeed, then release/remove; failure retains recovery state | Revoke → safe evidence → detach; keep fault/recovery binding при failure | Нельзя забыть actuator, пока safe outcome unknown; release lease ≠ physical safe |
| Shutdown/reload | Attempt all safe outputs before transport teardown; retired flag | Stop admission/revoke → safe attempts → bounded drain/evidence → transport stop → recorder finish | Shutdown report честно сообщает unknown/failed safe; crashed process не может software-only гарантировать actuator safety |

Lease/TTL/freshness target отсутствуют в v1. Permit expiry не позже lease, proposal или input freshness; Rust dispatcher непосредственно перед physical send проверяет актуальные epoch/interlocks. Сброс lease не отменяет bytes, уже отправленные на устройство. До release/rebind нужны in-flight accounting, deadline и recovery; safe command может требовать независимого hardware interlock, если канал потерян.

В v1 unregistered manual writes и raw text проходят worker без controller ownership. В target все output-affecting operations, включая data-driven/Lua/external/manual/config side effects, подчинены одной Rust authority. При этом read/config operation classification должно быть явно задано trusted metadata; arbiter не способен вывести смысл произвольных bytes. Тесты T09 — основной behavioral donor, а TTL/stale-dispatch/fault-latch/in-flight/recorder-required tests добавляются.

## 10. Recorder / history

Relevant v1: ProcessRecord/ProcessRecorder/SQLite writer, session (C36–C38). Classification: ADAPT recording semantics/storage/schema; REWRITE service wrapper/buffering/failure policy.

SQLite разумно оставить кандидатом на первый storage adapter: writer already single-owner, measurement batches transactional, WAL/NORMAL, independent recorder port. Это не требование domain и не обещание сохранения schema. Не нужен новый storage engine ради greenfield, но нужен новый contract.

| V1 | Сохранить | Изменить |
| --- | --- | --- |
| Tables session/configurations/logs/actions/measurements/control_outputs/external_events; timeline view | Configuration provenance, action start/completion, raw/derived samples, user annotations, stable identity snapshots | Schema version, experiment/run vs runtime session, generic diagnostics, quality/unit/monotonic time/generation/revision, gaps и evidence |
| Session на весь process, same DB при reload/acquisition cycles | Resource lifetime отделён от acquisition start/stop | Run/config activation имеют собственную identity; restart не continuation authority |
| `actual_output: Option<f64>` | Отличать requested output от successful result | ACK/readback/physical evidence typed отдельно; null не смешивает rejection/failure/unknown |
| Unbounded writer/timeline + bounded API observer | Producers не выполняют disk I/O; completion registered before publish, first error retained | Bounded byte/count queues, explicit overflow/gap policy, storage health signal; observer loss не durable loss |
| First writer failure disables recording, control continues | Honest failure and finalization report | Recorder-required physical control по target → revoke/safe/fault; observation-only может degraded по explicit policy |
| finish после producer retirement, Drop best effort | Ordered flush/finalization, session ended marker | Bounded shutdown outcome, durability level defined; WAL/NORMAL не power-loss guarantee |

Не пытаться выдавать v1 event tap за event-sourced domain store. Events для clients и durable observations — разные consumers с разными loss policies. Import/read-only historical v1 SQLite — DEFER; автоматическое преобразование старых DB не входит в текущий этап и первый POC. T10/T13; реальные disk-failure/recovery/slow-sink tests только в будущей реализации.

## 11. Lua capability → future owner

Relevant v1: application Lua runtime/API, profile loader, scenario engine, Lua virtual model (C18, C39–C43, C48). Classification: REWRITE application API; EXTRACT validation/fixtures; ADAPT bounded model intent. Один общий C/Q/E domain contract, не три параллельных application API.

| Current capability | Future owner / решение |
| --- | --- |
| Persistent console/global Lua, Run script | Babashka/external REPL для experiment management; arbitrary embedded application REPL не переносится |
| `app.start/stop/clear/retry/retry_all` | External Commands → Rust application lifecycle; acquisition stop не должен неявно оставлять unsafe active controller |
| `app.start_emu/stop_emu`, profile emulator path | Rust runtime config + Instrument lifecycle Commands; Lua component не владеет чужим lifecycle |
| `app.log`, external process annotations | Structured external event/annotation command + recorder; bounded component diagnostics для Lua host |
| `app.add_serial/send_serial` | Named validated Instrument operation/configuration; unrestricted raw serial capability removed |
| `app.metakon/virtual_instrument`, parameters/read/write | Catalog Queries + typed refresh/config/output Commands; writes → authority по semantic capability; userdata/name-specific API removed |
| `plant:pid/on_off/furnace` | Native controller create/configure Commands через общие descriptors; не Lua algorithm callbacks |
| controller parameters/diagnostics/read/configure/set_input/state | Common Commands/Queries; dynamic read-only SP при Reference; runtime validates all cross-field rules |
| pause/resume/reset_integral/reset/remove | Rust controller/output lifecycle Commands с explicit outcomes; не VM-owned safety |
| Fixed/Ramp set/config/read | Independent Reference Commands/Queries; progress/reset semantics в Rust |
| `app.filter/set_filter`, diagnostic series add | Processing graph Commands + introspection; native filter constructors migration, bounded Lua filter — новая extension capability |
| Virtual model `instruments/read/write` | Bounded Lua component host с typed request/state/results и budgets; safe physical operations не callback-owned |
| Будущие Lua Filter/Controller/Reference callbacks | Bounded Lua component host; входы/outputs ограничены component contract, output только proposal; в v1 отсутствуют |
| `app.scenario`, after/at/when/race/stage, on_stop/on_error/complete/stop/cancel | Babashka/external orchestration, deferred rich workflows; Runtime оставляет локальные native loops/safety независимо от client |
| threshold/hold/rate/stale predicates из scenario | Recipe/test knowledge для external events; необходимые safety interlocks реализуются отдельно native, не на внешнем callback |
| `app.register_script/unregister_script`, panel callbacks, set_control/enabled | Future GUI/presentation integration; UI guards не output permission |
| color/visible/pane и profile fps/plot_window | GUI client state, не domain acquisition/control config |
| Profile setup/scripts (actions) | External orchestration; декларативные experiment/instrument configs — Rust validation/activation |
| Validation/helper conversions | Shared domain validators + thin adapters; не оставлять invariant только в Lua parser |

Application Lua и model Lua уже разные VM, но current hook не memory/capability/native-call isolation. Сохранить busy-queue/error/RAII-hook tests как частичные checks; заново определить standard-library allowlist, no raw COM/OS/FFI, instruction/time/memory budgets, callback state/reset and failure quality. Native host callback должен быть bounded сам; safe path работает при недоступной VM.

## 12. External Commands/Queries/Events и Babashka

Relevant v1: CoreHost/CoreHandle/CoreCommand, operation store, NDJSON codec/server/session, catalog DTOs и [integration tests](../../../com_port_reader/tests/core_api_operations.rs) (C44–C45). Classification: EXTRACT protocol/lifecycle tests; REWRITE target DTO/application mapping; ADAPT narrow transport utility where useful.

Сохранить separation accepted/applied/failed/outcome_unknown; register completion before dispatch; instance identity; malformed client isolation; bounded admission/events; disconnect не отменяет уже отправленную operation; listener lifecycle отдельно от runtime. V1 duplicate operation ID возвращает duplicate status, не выполняет другой payload под тем же ID; retention конечная (4096 completed, 128 nonterminal), после eviction/restart нет durable exactly-once. Нужны explicit scope/retention/conflict semantics target, а не магическое dedup обещание.

Не переносить автоматически NDJSON field names/operation names, monotonic request-id rule, numeric ID formatting и descriptive authorization categories. Loopback restriction не authentication; real security/permission model при расширении access требует отдельного решения. Future Query — snapshot; v1 `instrument/read` отнесён к Monitor, но выполняет I/O. Physical refresh — отдельная Command/operation. Events subscription после barrier — не durable replay: при отставании за retained history `events_after` возвращает None и session закрывается, без явного gap event. Cursor/gap/snapshot recovery надо определить в реальном slice, не раньше. Recording API содержит status/annotations, но не start/stop recording; это отдельная target lifecycle capability, не готовая v1 parity.

Проверки Rust TCP clients и passive profile подтверждают возможность управления, но в tracked donor нет полноценного Babashka client implementation/test. Рекомендуется ранний маленький реальный Babashka slice после bounded Lua и до production recorder: discover/describe → read snapshot/subscribe → configure Fixed/Ramp/native PID → observe command result → disconnect/reconnect/query state. Для manual control lease expiry проверяется отдельно: автономность native loop не даёт вечной authority отключённому external owner. Полная v1 ScenarioService parity не нужна для этого теста.

## 13. Future GUI requirements, без переноса GUI в POC

Relevant v1: app/components/help/panels/plot/settings и demo scripts (C46–C48). Classification: DEFER future GUI; EXTRACT downsampling/UX requirements; DISCARD in-process runtime glue.

| View / поведение | Domain API requirements | Что остаётся client-specific |
| --- | --- | --- |
| Series list/status/latest, retry/rename/delete | Stable IDs, source/dependency/quality/health/unit, remove impact preview, outcomes | Selection, colors, pane mapping, visibility |
| Generic instrument/controller forms | Descriptor types/ranges/units/read-only/cross-field error, revision, safe/authority status, typed operations | Widgets/layout; numeric range сам не заменяет semantic safety |
| Output/manual/automatic controls | Current owner/mode, permits/lease status, pending vs completed result, safe/fault recovery evidence | Disabled buttons/pending edit UX, confirmations; не authority |
| Plot/follow/history | Bounded time-range history/query or decimated display data, live subscription/gaps, raw data independent of visibility | Min/max envelope, axes/zoom/panes/reusable buffers |
| Logs/scenario/progress | Structured errors, command lifecycle, correlations, external annotations | Search, filtering, workflow presentation |
| Profile settings | Validate candidate/activation status/revision and safe replacement result | File picker/open editor/remembered path; runtime не открывает editor |
| Help | Descriptor docs + API version/capability info | Bilingual search/static teaching material; Lua-specific help signatures rewritten |
| Furnace custom panels | Generic config/diagnostics плюс explicit model-vs-controller units и commands | Presets, learning scenarios, grouped plots; не всё выводится из scalar descriptors |

Min/max downsampling можно выделить, заменив egui `PlotPoint` на нейтральную пару/индекс. Он не должен попадать на controller input или подменять stored measurements. Tests endpoint/extrema/ordering/point budgets — future GUI asset. Old GUI closes вместе со своим host process; target remote GUI должен терять только connection и client view state, не Runtime. T15/T14.

## 14. Итоговые правила будущего переноса

Порядок: domain contracts/tests → small native virtual → minimal authority → transport/codecs/native/data-driven slice → signal/reference/native PID → bounded Lua → real external slice → recorder/fault verification. Детальные gates — [POC_REFINEMENT](POC_REFINEMENT.md).

Не переносить старый class graph, direct raw write escape hatches, unlimited in-memory samples/queues, Lua-as-application-owner, GUI-shared mutable Runtime и auto-arm defaults. Сохранять mathematical behavior, protocol knowledge, independent worker/recording patterns и tested lifecycle intent. Любой compatibility exception должен быть явно назван, покрыт тестом и не обходить Rust OutputArbiter.
