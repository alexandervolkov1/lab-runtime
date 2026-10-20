# Единый API типизированных параметров: аудит и предложение

Статус: **предложение для ревью; реализация не выполнена**.

Аудит выполнен по текущему `lab-runtime` на коммите
[`efaefd15d018128c047634c34102ec5730a683c4`](https://github.com/alexandervolkov1/lab-runtime/commit/efaefd15d018128c047634c34102ec5730a683c4).
Перед работой проверены `D:\rust_projects\lab-runtime`, remote
`https://github.com/alexandervolkov1/lab-runtime.git`, чистое рабочее дерево и
совпадение `main` с GitHub. Документ подготовлен в отдельной ветке
`docs/unified-parameter-api-proposal`; `main` и production-код не изменялись.
Другие репозитории не использовались как источник реализации.

## 1. Рекомендация и границы

Добавить небольшой адаптер в Application API:

1. `parameter_catalog` и `parameter_catalog_page` — метаданные параметров одного
   владельца, используя существующий механизм ограниченных frozen projections.
2. `parameter_get` — текущее типизированное значение, его источник и версия.
3. `parameter_set` — одна типизированная запись с обязательной проверкой версии,
   **только для явно перечисленных существующих путей конфигурации**.

Это четыре новые операции protocol v1, а не новое ядро, транспорт или система
управления. Клиент узнаёт владельцев через прежний `discover`, затем получает
каталог и работает с адресами и типами, не зная реализации печи, двигателя или
клапана. Сервер знает, какой существующий механизм обслуживает каждый адрес.

В первую реализацию записи включить Fixed Reference, retune полей Ramp,
конфигурацию native PID и уже поддержанные live deployment properties.
Физические выходы доступны для чтения доказанного состояния, но **не записываются
через `parameter_set`**. Для них сохраняются `output_acquire`, `output_renew`,
`output_write`, `output_safe` со всеми прежними полномочиями и ограничениями.
Lifecycle, смена вида Reference, запуск/пауза/reset PID, Recorder, reconnect,
публикация эмулятора и restart моделей остаются самостоятельными действиями.

Контракт содержит пять типов: `float`, `integer`, `boolean`, `text`, `enum`.
Это не обещание writable-параметра каждого типа в любой композиции. В нынешней
реализации нет подходящего публичного live setter для boolean/enum, который
можно было бы честно включить в этот адаптер без расширения доменной семантики.
Они поддерживаются в каталоге и чтении; запись получает явный отказ.

Предложение не меняет старые имена операций, аргументы, DTO, числа в JSON,
SessionStore, существующие события, OutputAuthority, Recorder или схему SQLite.
Новые типизированные DTO существуют только у новых операций.

## 2. Что установлено по коду

### 2.1. Карта проверенных исходников

Ссылки ниже относительны к этому репозиторию; базовый SHA указан выше.

| Источник | Проверенные обязанности |
|---|---|
| [`model.rs`](../../crates/lab-core/src/model.rs) | `Value`, `ValueSpec`, `ParameterDescriptor`, `AccessMode`, `ParameterRole`, `WriteEffect`, валидация |
| [`virtual_instrument.rs`](../../crates/lab-core/src/virtual_instrument.rs), [`plant.rs`](../../crates/lab-core/src/plant.rs), [`instrument.rs`](../../crates/lab-core/src/instrument.rs), [`simple_device.rs`](../../crates/lab-core/src/simple_device.rs) | Реальные параметры, configured state, доступность чтения и записи |
| [`runtime.rs`](../../crates/lab-core/src/runtime.rs), [`runtime/dispatch.rs`](../../crates/lab-core/src/runtime/dispatch.rs) | `GetInstrumentState`, `ConfigureParameter`, Reference commands и фактическая маршрутизация |
| [`runtime/controllers.rs`](../../crates/lab-core/src/runtime/controllers.rs), [`reference.rs`](../../crates/lab-core/src/reference.rs), [`control.rs`](../../crates/lab-core/src/control.rs) | Revision CAS, ограничения состояния, полная конфигурация и сброс алгоритмов |
| [`application/discovery.rs`](../../apps/lab-runtime/src/application/discovery.rs), [`measurements.rs`](../../apps/lab-runtime/src/measurements.rs), [`application/projections.rs`](../../apps/lab-runtime/src/application/projections.rs) | `discover`, `describe`, `latest`, фактические публичные проекции |
| [`configuration_api.rs`](../../apps/lab-runtime/src/configuration_api.rs), [`configuration.rs`](../../apps/lab-runtime/src/configuration.rs) | Property metadata, `property_configure`, `with_property_override`, разрешённые свойства |
| [`service/configuration.rs`](../../apps/lab-runtime/src/service/configuration.rs), [`deployment.rs`](../../apps/lab-runtime/src/deployment.rs) | Staging/apply, revision, safe/Recorder barriers, process-local overlays |
| [`application/references.rs`](../../apps/lab-runtime/src/application/references.rs), [`application/controllers.rs`](../../apps/lab-runtime/src/application/controllers.rs), [`application/outputs.rs`](../../apps/lab-runtime/src/application/outputs.rs) | Декодирование и исполнение существующих команд |
| [`output/authority.rs`](../../crates/lab-core/src/output/authority.rs), [`manual-output.md`](../api/manual-output.md) | Lease, scope, epoch, ограничение renew, фактические подтверждения выхода |
| [`protocol.rs`](../../apps/lab-runtime/src/protocol.rs), [`wire.rs`](../../apps/lab-runtime/src/wire.rs), [`sessions.rs`](../../apps/lab-runtime/src/sessions.rs) | Реестр, codec, нормализация, admission, дедупликация и bounded outcomes |
| [`application.rs`](../../apps/lab-runtime/src/application.rs), [`application/operations.rs`](../../apps/lab-runtime/src/application/operations.rs), [`application/delivery.rs`](../../apps/lab-runtime/src/application/delivery.rs), [`events.rs`](../../apps/lab-runtime/src/events.rs) | Маршрутизация, снимки, подписки, порядок событий и терминальных результатов |
| [`host/lifecycle.rs`](../../apps/lab-runtime/src/host/lifecycle.rs), [`host/recording.rs`](../../apps/lab-runtime/src/host/recording.rs), [`recorder/worker/reference_completion.rs`](../../apps/lab-runtime/src/recorder/worker/reference_completion.rs), [`recorder/sqlite/write.rs`](../../apps/lab-runtime/src/recorder/sqlite/write.rs) | Запись до эффекта Reference, резервирование accepted/completion, COMMIT evidence |
| [`recorder.rs`](../../apps/lab-runtime/src/recorder.rs), [`recorder_api.rs`](../../apps/lab-runtime/src/recorder_api.rs) | Валидация configuration lifecycle records, Recorder API и отдельные гарантии durability |

Также прочитаны `AGENTS.md`, `README.md`, `docs/README.md`, `ORIGIN.md` и
действующие документы `docs/api/`. Тесты из раздела 9 изучены как регрессионные
свидетельства в исходниках; их выполнение в рамках этого аудита не заявляется.

### 2.2. Дескриптор не является разрешением

`ParameterDescriptor` содержит стабильный локальный `id`, имя, `value_spec`,
единицу, `access`, `role`, `write_effect` и необязательный `signal`.

| Измерение метаданных | Реальные варианты | Значение для нового API |
|---|---|---|
| `AccessMode` | ReadOnly, ReadWrite, WriteOnly | Объявленная возможность прибора; не текущее разрешение и не наличие Application handler |
| `ParameterRole` | Measurement, Configuration, Actuator, Action, Diagnostic | Выбор семантического источника; Action не превращается в значение для присваивания |
| `WriteEffect` | None, ConfigurationOnly, OutputAffecting | OutputAffecting запрещён в generic property path; числовой тип ничего не меняет |

Валидация уже требует: ReadOnly имеет `None`; writable-параметр — ненулевой
write effect; writable Actuator — `OutputAffecting`; `ConfigurationOnly` допустим
только для Configuration. Нечисловой параметр должен быть безразмерным.
Signal допустим для readable Measurement/Diagnostic и специальной формы
ReadWrite Actuator с OutputAffecting. Наличие signal у actuator также не даёт
права управления.

В `discover` признак `capabilities.properties` вычисляется из роли Configuration.
Он не доказывает наличия публичного setter. Это особенно заметно у native
VirtualInstrument с `base_temperature` и `measurement_enabled`.

### 2.3. Таблица скалярных типов

| Тип | Core / ограничения | Публичное чтение сейчас | Реальная запись сейчас / пробел |
|---|---|---|---|
| float | `Value::Float(f64)`, только finite, включительные min/max | `latest` — JSON number; Reference/PID/output — числа; property metadata использует имя `number` | Reference/PID и отдельный manual output; `property_configure` дробные числа не принимает |
| integer | `Value::Integer(i64)`, точный тип, включительные min/max | `latest` — десятичная строка; deployment properties — JSON integer; durations/revisions — строки | `poll_period_ms` через JSON integer; durations/EMA через полный `controller_configure`; единого codec нет |
| boolean | Точный bool, без преобразования 0/1 | Serializer signal поддерживает bool; configured `measurement_enabled` доступен Core; deployment projection показывает bool | Core virtual configure существует; Application live property setter для bool отсутствует |
| text | UTF-8, `max_bytes`; предел Core 1024 байта | Serializer signal и property projection; `display_name` | Только поддержанные имена через `property_configure`; имя непустое после trim, до 128 байт |
| enum | Непустые уникальные choices, до 64; каждое до 1024 байт | Core/serializer умеют enum; диагностические состояния также публикуются строковыми перечислениями | Универсального enum setter нет; lifecycle state нельзя присваивать строкой |

Core умеет пять типов, но это не означает пять типов физических драйверов.
Действующие Metakon definition DTO и SimpleDevice definition DTO допускают
`float`/`integer`; SimpleDevice actuator использует float engineering domain.
Native managed measurement также строится как float.

Публичный codec ограничивает каждую строку 512 байт, глубину JSON 16,
количество values/members 1024, JSON body 16 383 байт. Поэтому максимально
допустимый Core text/enum не всегда представим на wire. Нельзя молча обрезать
значение или choices. Это существующая граница, а не основание увеличивать
общие лимиты в данном расширении.

### 2.4. Инвентаризация семантических классов

Таблица покрывает роли дескрипторов, deployment properties, native Reference/PID
и выходы. Числовые ID ниже — примеры конкретной native модели, не универсальные
значения для клиентов.

| Класс / параметры | Чтение сейчас | Изменение и ограничения сейчас | Решение для единого API |
|---|---|---|---|
| Measurement/Diagnostic с signal | `describe` + `latest`, current/window, события signal; явно unavailable/not_observed | Обычно ReadOnly; `latest` не выполняет I/O | Единое чтение; запись запрещена |
| Actuator: например `heater_power` | `describe`; `output` — requested/sent/ACK/readback; signal может отсутствовать | Только существующий manual/automatic OutputAuthority path | Читать readback отдельно от намерения; generic set запрещён |
| Метаданные actuator без executable adapter | Дескриптор есть; фактического значения может не быть, manual_supported=false | Наличие ReadWrite не создаёт реализацию | Нет фиктивного значения и setter |
| Configuration descriptor native VirtualInstrument: `3/base_temperature`, `4/measurement_enabled` | Core `GetInstrumentState.configured`; публичного эквивалента сейчас нет; deployment metadata может показывать одноимённые значения | Core `ConfigureParameter` валидирует ConfigurationOnly, но без expected_revision; Application не выставляет этот путь | `parameter_get` может открыть committed Core snapshot; set отложен |
| Configuration/Diagnostic без signal и без configured storage | Только metadata | Объявление роли не реализует backend | Каталог с явным отсутствием read/set support |
| Action / WriteOnly | Нет обещания readable state | Семантика операции должна быть определена отдельно | Нет generic присваивания; использовать конкретное действие |
| Instrument property `display_name` у VirtualMeasurement/ThermalPlant | `configuration_properties`, `discover` | `property_configure`: text, live_safe, глобальная deployment revision; не для Metakon/SimpleDevice | Читать/писать через тот же staged path |
| Instrument property `poll_period_ms` | Те же projections | Integer 1..60000; SimpleDevice 10..60000; ordinary_live, revision и lifecycle barriers | Читать/писать через прежний path |
| VirtualMeasurement properties `base_temperature`, `measurement_enabled` | `number`/boolean в deployment projection | deployment_only/reinitialize; не допускаются `with_property_override` | Читать; не превращать в live writable |
| VirtualMeasurement `external_publication` | Boolean property | ReadOnly; наличие режима не означает запись произвольного сигнала | Только чтение; `emulator_publish` остаётся отдельной операцией |
| ThermalPlant `ambient_temperature`, `initial_temperature`, `gain_per_percent`, `time_constant_ms` | Deployment projection | deployment_only/reinitialize | Только чтение; автоматического restart нет |
| Resource `port`, `baud_rate` | Property projection + `resource` | port read_only; baud_rate deployment_only; rebind_required | Только чтение; не обходить reconnect/configuration lifecycle |
| Managed component `period_ms` и scalar config | Property projection: integer/number/boolean/text, constraints из component metadata | Все deployment_only; mutation_class обычно reinitialize; handler записи отсутствует | Читать; generic запись не добавлять |
| Managed component arrays (`number_array`) | Property projection | Вне пяти скалярных типов; live setter отсутствует | Явно отметить как неподдержанный scalar; старый API сохраняется |
| Fixed Reference value | `reference`, discovery state, reference events | `reference_configure`, expected_revision; finite, unit сохраняется; **запрещено при связанном PID Warming/Running** | `fixed.value` можно писать только с теми же ограничениями |
| Ramp current/target/rate | `reference`, reference events | Current — вычисляемое значение; `reference_retune` меняет target+rate непрерывно, CAS; rate > 0; допускается live retune | Current только читать; target/rate — адаптер retune с сохранением другого поля |
| Reference kind и unit | `reference` | Смена fixed/ramp через полную `reference_configure`; не эквивалентна присваиванию current | Kind enum только читать; смена режима остаётся специальной операцией |
| PID `kp`, `ki`, `kd`, `output_min`, `output_max` | Полный `controller.config.pid` и controller events | `controller_configure_pid`, config revision; только Ready/Paused, без lease/конкурирующего warming; finite, min < max, пределы OutputAuthority; **сбрасывает память алгоритмов** | Scalar set через один полный validated PID candidate |
| PID EMA/time policies | `controller.config`: EMA time constant/warmup, input age, tick gap, lease lifetime, proposal TTL | `controller_configure`, тот же CAS и состояние; полная валидация графа/единиц/output policy; bindings фиксированы | Scalar set через полный native candidate; никаких самостоятельных изменений lease |
| PID state/active/paused/latest_output | `controller`, события | State меняется только lifecycle; latest_output — вычисленный output PID, не физическое измерение | Read-only проекции enum/bool/float; start/pause/resume/reset отдельно |
| Recorder state/policy/run/history | `recording_status`, Recorder events и собственные операции | Отдельная lifecycle/durability семантика | Оставить существующий API; не включать arbitrary Recorder fields в первый каталог |

`property_configure` сейчас принимает только `PropertyMutationValue::Integer(i64)`
из JSON number и `Text(String)` из JSON string. Строка `"100"` — text, не integer.
`with_property_override` допускает только target `instrument` и два свойства:
`display_name`, `poll_period_ms`. Декодируемый target `component`/`resource`
не означает реализованный setter. Максимум process-local overlays — 32.
У compiled profile без deployment lifecycle этот механизм недоступен.

Одноимённый descriptor parameter и deployment property — **разные адреса**.
Нельзя сопоставлять их по display name и использовать Core ConfigureParameter
для обхода deployment-only политики, provenance и Recorder activation.

### 2.5. События, версии, сессии, результаты

На базовом SHA реестр содержит 47 операций: 22 query и 25 mutation.
`hello` согласует protocol `lab-runtime.application` v1, API `0.1-pre`,
boot, scope, next_seq, доступные операции и лимиты. TCP и WebSocket используют
один Application/codec/SessionStore.

Разные виды версии уже имеют разные значения:

- `{boot_id,event_seq}` — позиция потока фактов, не CAS параметра;
- Reference revision — конфигурация, не каждый шаг Ramp;
- controller revision — общая native configuration, не каждый PID tick;
- deployment revision — общая конфигурация, не revision одного property;
- signal generation — происхождение данных, не версия значения;
- output instance/epoch/lease — полномочия, не generic revision.

Mutation декодируется в нормализованный typed payload **до admission**.
Identity `{scope,seq}` и этот payload определяют дедупликацию.
Повтор одинакового retained запроса не исполняется снова; другой payload/operation
под тем же ID конфликтует. Старый seq без retained outcome не исполняется вновь.
New seq строго последовательный. Потеря соединения не отменяет admitted work.

`accepted` не означает эффект, terminal `completed` означает результат конкретной
доменной операции, а не универсальный COMMIT или подтверждение физического эффекта.
У property activation возможен отказ после owner commit при проблеме Recorder;
`failed` нельзя объявлять гарантией отката всех состояний.

`operation_status: outcome_unknown` означает отсутствие retained evidence, а не
доказательство исполнения/неисполнения. После disconnect, timeout, смены boot
нельзя автоматически повторять мутацию, менять её scope или выводить исход
старой команды из нового фактического значения.

`EventLog.observe` сравнивает полные committed projections после доменной единицы.
Факты состояния предшествуют `operation_terminal`. Reference/controller events
несут соответствующие проекции; configuration event сообщает lifecycle revision,
но **не содержит все значения properties**. Для них событие означает перечитать
интересующий параметр. Нет существующего универсального parameter event.
Произвольные Core configured values без signal также не имеют своего event kind.

## 3. Предлагаемый контракт новых операций

Все имена и JSON в разделах 3–6 — проект, не описание уже работающего API.

### 3.1. Версионирование и адрес

Сохранить envelope `v:1`; добавить capability
`{"name":"parameter_api","version":1,"stability":"experimental"}` и четыре имени
в `hello.operations`. Старый сервер не рекламирует capability; клиент использует
прежние операции. Новая capability не изменяет версии старых DTO.

Адрес:

```json
{
  "owner": {"kind": "instrument", "id": "1"},
  "namespace": "descriptor",
  "key": "1"
}
```

| owner.kind | namespace | key |
|---|---|---|
| instrument | descriptor | Каноническая десятичная строка ParameterId |
| instrument / component / resource | property | Существующий стабильный property ID, например `poll_period_ms` |
| reference | field | `value`, `kind`, `fixed.value`, `ramp.target`, `ramp.rate` |
| controller | field | `pid.kp`, `pid.ki`, `pid.kd`, `pid.output_min`, `pid.output_max`, `ema.time_constant_ns`, `ema.warmup_samples`, `max_input_age_ns`, `max_tick_gap_ns`, `lease_lifetime_ns`, `proposal_ttl_ns`, `state`, `active`, `paused`, `latest_output` |

Это конечные серверные таблицы полей, не JSONPath, reflection или произвольный
доступ к Rust-структурам. Никаких адресов регистров, выражений и путей в файлах.
`owner.id` и descriptor key — прежние стабильные ID; имена не используются
для разрешения адреса. Адрес действителен в текущем boot; после пересоздания
источника необходимо обновить metadata/generation. Namespace исключает коллизии.
У Reference `value` всегда read-only; `fixed.value` существует только у Fixed,
`ramp.*` — только у Ramp. Смена kind не меняет смысл прежнего writable адреса.

### 3.2. Значение и ограничения

```json
[
  {"type": "float", "value": 42.5},
  {"type": "integer", "value": "100"},
  {"type": "boolean", "value": true},
  {"type": "text", "value": "Камера 1"},
  {"type": "enum", "value": "paused"}
]
```

Это примеры codec, а не обещание записи `"paused"` или writable boolean.
Float — finite JSON number; integer — каноническая signed decimal string
без `+`, ведущих нулей и `-0`; bool строго bool; text/enum различаются тегом.
Не конвертировать строки в float, 0/1 в bool или text в enum автоматически.
Null — отсутствие значения с отдельным status, не шестой тип и не команда reset.

Integer в новой оболочке должен точно представлять и Core i64, и существующие
unsigned durations/counters. Предлагается bounded parsing в `i128` с последующей
проверенной конверсией в тип источника; это не изменение `lab_core::Value`.
Диапазон конкретного поля задаёт `value_spec`. В частности, writable ns-поля
сохраняют диапазон прежнего `u64` decoder; большое read-only `Duration::as_nanos`
не округляется до f64. Все текущие Rust Duration nanoseconds помещаются в i128.
Расширение wire integer до i128 не расширяет разрешённый диапазон Core i64.

Для descriptor сохраняются его min/max, bytes/choices. Для native полей без
объявленных bounds `value_spec` допускает null min/max; дополнительно описываются
действующие проверки finite, positive и зависимости полей. Например, Reference
не получает выдуманный диапазон температур печи, а PID gains не получают
выдуманное требование неотрицательности.

`number` в property metadata отображается в `float` только в новом DTO.
Для metadata/current несоответствий, например `time_constant_ms` объявлен как
number, но хранится Unsigned, использовать точный storage type и представить
integer с исходной единицей; отметить source metadata type. Не менять старый DTO.
Неизвестная единица — null, без вывода единиц gains из имени поля.

### 3.3. Каталог

Запрос одного владельца:

```json
{"v":1,"msg_id":"catalog-1","op":"parameter_catalog","args":{"owner":{"kind":"reference","id":"1"}}}
```

Пример полной страницы для Fixed Reference:

```json
{
  "v": 1,
  "msg_id": "catalog-1",
  "type": "result",
  "result": {
    "projection": "<projection>",
    "revision": {"boot_id": "<boot>", "event_seq": "120"},
    "records": [
      {
        "address": {"owner":{"kind":"reference","id":"1"},"namespace":"field","key":"fixed.value"},
        "label": "Fixed value",
        "value_spec": {"type":"float","min":null,"max":null,"finite":true},
        "unit": {"id":"degC","symbol":"°C"},
        "declared": null,
        "read": {"supported":true,"source":"reference_configuration"},
        "write": {
          "supported": true,
          "via": "parameter_set",
          "revision_domain": "reference",
          "effects": ["changes_reference"],
          "conditions": ["no_linked_controller_warming_or_running"]
        },
        "observe": {"mode":"invalidate","kind":"reference","target":{"id":"1"}}
      },
      {
        "address": {"owner":{"kind":"reference","id":"1"},"namespace":"field","key":"value"},
        "label": "Current reference",
        "value_spec": {"type":"float","min":null,"max":null,"finite":true},
        "unit": {"id":"degC","symbol":"°C"},
        "declared": null,
        "read": {"supported":true,"source":"reference_value"},
        "write": {"supported":false,"reason":"derived_value"},
        "observe": {"mode":"invalidate","kind":"reference","target":{"id":"1"}}
      },
      {
        "address": {"owner":{"kind":"reference","id":"1"},"namespace":"field","key":"kind"},
        "label": "Reference kind",
        "value_spec": {"type":"enum","choices":["fixed","ramp"]},
        "unit": null,
        "declared": null,
        "read": {"supported":true,"source":"reference_configuration"},
        "write": {"supported":false,"reason":"explicit_action_required"},
        "observe": {"mode":"invalidate","kind":"reference","target":{"id":"1"}}
      }
    ],
    "next_index": null,
    "complete": true
  }
}
```

Для descriptor `declared` содержит прежние `access`, `role`, `write_effect`,
`signal`. Для property — исходные `access`, `mutation_class`, `value_type`.
Они не подменяются новым `write.supported`.

`write.supported:true` означает наличие адаптера, **не разрешение сейчас**.
Conditions — подсказки для UI, не второй policy engine. Runtime повторно
проверяет реальное состояние и все ограничения при исполнении.
Для actuator: `supported:false`, `reason:"output_authority_required"`,
`via:"output_operations"`; если executable adapter отсутствует — отдельная
причина `no_execution_adapter`. Никакого автоматически получаемого lease.

Включать неподдержанные scalar properties в каталог с `value_spec:null`,
`read.supported:false`, `write.supported:false`, `reason:"non_scalar"` и исходным
type. Так `number_array` не исчезает молча и не выдаётся за text.

Продолжение:

```json
{"v":1,"msg_id":"catalog-2","op":"parameter_catalog_page","args":{"projection":"<projection>","index":"64"}}
```

Использовать один существующий shared frozen slot на соединение, TTL 5 секунд,
страницы до 64 records и 8 KiB records, плюс общий wire validator. Предлагаемый
предел одного owner catalog — 256 records. Детерминированная сортировка по адресу.
Начало каталога заменяет предыдущий frozen slot так же, как нынешние start
queries; это должно быть явно документировано. Каталоги разных владельцев и
`discover`/`configuration_properties` читать последовательно до `complete`.
Переполнение/непредставимый record — явный отказ, не усечение.

Для этого нужны четыре операции, а не три: повторное использование существующего
pagination устраняет новый cursor engine и сохраняет достаточность для metadata.
Не добавлять глобальный неограниченный список всех параметров.

### 3.4. Чтение

```json
{"v":1,"msg_id":"get-1","op":"parameter_get","args":{"address":{"owner":{"kind":"reference","id":"1"},"namespace":"field","key":"fixed.value"}}}
```

```json
{
  "v":1,
  "msg_id":"get-1",
  "type":"result",
  "result":{
    "address":{"owner":{"kind":"reference","id":"1"},"namespace":"field","key":"fixed.value"},
    "snapshot":{"boot_id":"<boot>","event_seq":"120"},
    "version":{"boot_id":"<boot>","domain":"reference","revision":"7"},
    "current":{
      "status":"available",
      "value":{"type":"float","value":40.0},
      "observed_at_ns":null,
      "source_at_ns":null,
      "quality":null,
      "failure":null
    },
    "source":{"kind":"reference_configuration","generation":null}
  }
}
```

Query читает committed state на одном ходе owner; не вызывает refresh, retune,
poll прибора или вычисление Reference. Снимок и event cursor фиксируются вместе.
Configuration value не получает выдуманный timestamp измерения: время последней
оценки Reference относится к `value`, не обязательно к commit `fixed.value`.

`current.status`: `available`, `not_observed` или `unavailable`; в двух последних
случаях typed value null. Signal сохраняет quality/failure, observed/source times
и generation. Output readback сохраняет свой `at` как observed time; не подставлять
туда время запроса. Неподдержанное чтение отличается от отсутствующего наблюдения:
оно возвращает `parameter_not_readable`, а не available/null.

`version` — CAS token только для mutable configuration domain:
`reference`, `controller` или `deployment`. Для read-only измерения, output
evidence и Core configured параметра без подходящей версии — null.
Snapshot cursor и generation никогда не принимаются вместо этой revision.

Actuator descriptor с поддержанным чтением возвращает только `output.readback`,
с `source.kind:"output_readback"` и bounded evidence context:
authority_instance, epoch, outcome, readback_failure, pending/in_flight.
WriteOnly descriptor не приобретает readable значение через эту проекцию;
существующий query `output` сохраняет отдельную семантику состояния authority.
Если readback отсутствует, requested/ACK/`manual_attempt.readback` его не заменяют.
Последний readback может быть старым; его время и текущий outcome сохраняются.
Это подтверждённое чтение регистра/модели, не независимая непрерывная мощность.
Наличие отдельного signal остаётся в descriptor; signal доступен прежним `latest`.
Не интерполировать пропущенные output observations.

### 3.5. Запись и результат

```json
{
  "v":1,
  "msg_id":"set-1",
  "op":"parameter_set",
  "request_id":{"scope":"<scope>","seq":"1"},
  "args":{
    "address":{"owner":{"kind":"reference","id":"1"},"namespace":"field","key":"fixed.value"},
    "expected":{"boot_id":"<boot>","domain":"reference","revision":"7"},
    "value":{"type":"float","value":42.0}
  }
}
```

```json
{"v":1,"msg_id":"set-1","type":"operation","request_id":{"scope":"<scope>","seq":"1"},"state":"accepted"}
```

```json
{
  "v":1,
  "msg_id":"set-1",
  "type":"operation",
  "request_id":{"scope":"<scope>","seq":"1"},
  "state":"completed",
  "result":{
    "address":{"owner":{"kind":"reference","id":"1"},"namespace":"field","key":"fixed.value"},
    "effective_operation":"reference_configure",
    "committed":{
      "version":{"boot_id":"<boot>","domain":"reference","revision":"8"},
      "value":{"type":"float","value":42.0}
    }
  }
}
```

Пример предполагает допустимое состояние PID и Recorder. Сохранить прежние
terminal errors при нарушении состояния/CAS. `committed` фиксируется Runtime
после успешного доменного результата, до другой клиентской мутации; не является
эхом входных данных. Последующий `parameter_get` может уже показывать иную
версию, если второй клиент успел изменить значение. Он не переписывает результат
первого запроса.

Ещё два примера requests; scope/seq/revision нужно брать из реальной сессии:

```json
{"v":1,"msg_id":"period","op":"parameter_set","request_id":{"scope":"<scope>","seq":"2"},"args":{"address":{"owner":{"kind":"instrument","id":"1"},"namespace":"property","key":"poll_period_ms"},"expected":{"boot_id":"<boot>","domain":"deployment","revision":"3"},"value":{"type":"integer","value":"100"}}}
```

```json
{"v":1,"msg_id":"kp","op":"parameter_set","request_id":{"scope":"<scope>","seq":"3"},"args":{"address":{"owner":{"kind":"controller","id":"1"},"namespace":"field","key":"pid.kp"},"expected":{"boot_id":"<boot>","domain":"controller","revision":"2"},"value":{"type":"float","value":1.5}}}
```

Последний запрос означает полную проверку нового PID candidate и сброс памяти
алгоритмов, как `controller_configure_pid`. Он не разрешает tuning работающего PID.

Для нового DTO строгие allowlists, duplicate-key rejection, bounds и
конечность чисел действуют до admission. Domain lookup, CAS, type/spec и state
проверяются в обычном пути исполнения после дедупликации; semantic failure
принятого запроса терминален. Read-side ошибки не расходуют seq.
Неподдержанная запись получает новый bounded code `parameter_not_settable`,
category `unsupported_operation`, retryable=false. Для неподдержанного чтения
аналогично `parameter_not_readable`. Неизвестный адрес — `unknown_parameter`
либо существующий unknown-owner code; неверная оболочка/type shape — `invalid_args`;
несовпадение source type/bounds — прежняя доменная ошибка.
У структурно корректного set сначала проверить существование адреса и наличие
set route, затем соответствие expected domain/boot/revision. Версия другого
домена никогда не делает неподдержанный адрес writable.
Не публиковать новые названия вместо прежних ошибок старых операций.

Нет `force`, wildcard target, batch set, `skip_revision`, клиентского timestamp,
автоподбора revision, automatic retry или отдельного transaction ID.
No-op запись сохраняет семантику исходной операции, включая её revision advance
и reset, если они выполняются сейчас.

## 4. Отображение на существующие механизмы

| Новый адрес / операция | Авторитетный read path | Set route и CAS | Событие / особенность |
|---|---|---|---|
| instrument/descriptor с Measurement/Diagnostic signal | Describe + GetLatestSignal, serializer measurements | Нет | signal; generation и failure сохраняются |
| instrument/descriptor Configuration | GetInstrumentState.configured | Нет в первой реализации | snapshot-only; универсального configured event/revision сейчас нет |
| instrument/descriptor Actuator | Query::Output.readback | Нет; только отдельный output API | output; authority/lease не переносятся |
| instrument/property supported live | FrozenDeployment effective property metadata | `ServiceHost::configure_property`, общая deployment revision | configuration invalidates значение; старые barriers/provenance |
| прочие scalar properties component/resource/instrument | Существующие property metadata/current | Нет при deployment_only/read_only/no handler | configuration invalidates; не предполагается live setter |
| reference/field fixed.value | Query::Reference | `ConfigureReferenceFixed` → ReconfigureReference; reference revision | reference, затем operation; запрет Warming/Running сохраняется |
| reference/field ramp.target или ramp.rate | Query::Reference | RetuneRampReference, reference revision | Непрерывность прежней траектории, rate > 0 |
| reference/field value/kind | Query::Reference | Нет | reference; progress не меняет configuration revision |
| controller/field pid.* | Query::ControllerConfig | ConfigureControllerPid, общая controller revision | controller; полный PID reset |
| controller/field EMA/time policy | Query::ControllerConfig | ReconfigureController, общая controller revision | controller; bindings сохраняются, full validation/reset |
| controller/field state/active/paused/latest_output | Query::Controller | Нет | controller; действия остаются отдельными |

Для Ramp/PID scalar setter строит **один** полный candidate непосредственно на
owner: прочитать текущее состояние, проверить expected revision/kind, заменить
одно поле, передать прежнему semantic handler. Никакого network get-modify-set,
промежуточного commit, скрытого pause/start или отдельной mutation identity.
Новая команда проходит те же Core проверки; клиентская availability лишь подсказка.

Если изменение пары полей по одному нарушает инвариант (например, перенос обоих
PID limits), использовать существующую полную операцию. Не добавлять транзакционный
язык или делать незаконное промежуточное состояние допустимым.

### 4.1. Почему не выставлять Core ConfigureParameter напрямую

В текущем dispatcher этот command обслуживает native VirtualInstrument и только
два configuration значения. Он не имеет ожидаемой revision и общего механизма
configured events; не является общим property backend для физических драйверов.
Deployment API те же настройки намеренно маркирует deployment_only/reinitialize.

Прямой вызов дал бы иной lifecycle и мог рассогласовать effective deployment,
Core state и Recorder provenance. Поэтому здесь предлагается только чтение
реального configured snapshot. Для будущей записи нужны отдельное решение о
доменной семантике, версия, coherent event и включение в прежний lifecycle.
Это следующий самостоятельный проект, не скрытая часть API-адаптера.

### 4.2. OutputAuthority и независимые сессии

A и B получают разные scopes. Lease A не становится lease B оттого, что оба
читают один parameter address. `parameter_set` не принимает owner ID, lease token,
expected_epoch, expected_instance, raw bytes или evidence. Даже ReadWrite actuator
не может попасть в property/Reference/PID adapter.

Manual output остаётся прежним конечным процессом: obtain lease, explicit write,
renew только допустимого idle lease, safe и последующее подтверждение.
Проверяются profile, exact physical encoding, Recorder gate, срок, epoch,
binding/mapping generation и последняя граница перед возможными байтами.
После possible-send timeout исход ambiguous; автоматического повторения нет.
`output_safe.completed` по-прежнему означает запрос перехода, не safe_confirmed.
Запуск PID конфликтует с manual authority и не вызывается setter автоматически.

Обёртка `parameter_set` с вложенным manual authorization технически возможна,
но повторит arguments и asynchronous receipt lifecycle `output_write`.
Для первого расширения она не даёт полезного упрощения и создаёт риск обхода
guards; рекомендуется её не добавлять.

## 5. Admission, Recorder и совместимость событий

### 5.1. Дедупликация должна зависеть только от запроса

Нужен отдельный нормализованный `Mutation::SetParameter` с адресом, typed value
и expected token. Не преобразовывать его до admission в существующий полный PID
или Ramp payload, дополненный **текущими** значениями: повтор того же wire request
после первого commit иначе получит другой payload и ложный request_conflict.

Retained identity включает исходную операцию. `parameter_set` и
`reference_configure` под одним ID конфликтуют, даже если доменный эффект совпадает.
Canonicalization scalar формы детерминирована; она не зависит от mutable state.
Exact duplicate возвращает прежний результат до повторной проверки новой revision.

Разрешение адреса к конечному adapter и создание полного candidate происходят
в owner execution path. Для записи используется один existing admission, один
request_id и один terminal result, без рекурсивного вызова `Application.handle`.

### 5.2. Особый риск Reference audit

В текущем коде недостаточно вызвать Reference dispatcher:

- Application gate вызывает `reserve_reference_operation` только для
  `reference_retune`/`reference_configure`;
- до effect вызывается `prepare_reference_completion`;
- Host `record_operation`, worker reservation и SQLite
  `append_reference_completion_assigned` также распознают эти имена;
- completion объединяет Reference fact и terminal audit в прежний защищённый
  credit/COMMIT путь. Замена имени на `parameter_set` может его разрушить.

**Минимальная рекомендация:** сохранить существующее semantic operation name
для Recorder и lifecycle, оставив wire/dedup identity `parameter_set`.
В новых operation audit data добавить `source_operation:"parameter_set"`,
исходный address, typed value, expected и `effective_operation`.
Таким образом в архиве явно виден запрос нового API, а command остаётся
`reference_configure`/`reference_retune`/`controller_configure_pid`/
`controller_configure`/`property_configure` по фактическому adapter.
Старые audit payloads и имена старых вызовов не меняются.

Статический route для `fixed.value` и `ramp.*` позволяет выбрать правильный
Reference gate без чтения текущего значения и без изменения dedup payload.
Для неподдержанного адреса route не должен выдумывать Reference effect.
Recorded intent новых wrapper-вызовов описывает scalar изменение; полный
authoritative результат и факты отражают реально применённую конфигурацию.
Это deliberate новая форма audit data только новых запросов, не изменение
SQLite schema или исторических записей.

Для property route вызвать именно `configure_property`, сохранив его lifecycle
operation_kind `property_configure`. Валидатор `ConfigurationLifecycleRecord`
допускает это имя и ровно один revision advance. Не обходить его записью
непосредственно в effective DTO. Дополнительный configuration event должен
создаваться в том же месте относительно operation terminal.

Если реализация не может доказать сохранение этих guarantees небольшим адаптером,
первый code PR ограничить каталогом/get и оставить записи в прежних операциях.
Не расширять worker/SQL allowlists «для прохождения тестов» и не обещать
универсальную запись до доказательства эквивалентности.

### 5.3. Наблюдение без нового event framework

Не добавлять parameter event kind, второе sequence space или журнал.
В первой версии `observe.mode:"invalidate"` сообщает существующие kind/target.
Событие помечает адрес dirty; клиент объединяет повторные invalidations и делает
один `parameter_get`, чтобы получить новый унифицированный DTO.
Для интенсивных измерений клиент продолжает использовать существующий signal
event payload и measurements API; не нужен запрос на каждую точку.

| Адрес | Существующий trigger |
|---|---|
| signal-backed descriptor | signal с прежним compound target |
| output readback | output с прежним actuator target |
| Reference / PID fields | reference / controller с `{id}` |
| Deployment property | configuration с `{id:"runtime"}`; перечитать нужные адреса/каталог |
| Core configured без соответствующего event | `observe.mode:"snapshot_only"`; отсутствие уведомления указано явно |

В каталоге не обещать полные уведомления для snapshot-only. Не открывать запись
таких параметров, пока не определён согласованный существующий event path.
Смена Reference kind/configuration может изменить набор/тип адресов — перечитать
каталог, а не продолжать использовать старую writable карточку.

Bootstrap для клиента:

1. Hello, последовательный полный `discover` и нужные каталоги.
2. Получить нужные `parameter_get`; у каждого сохранить собственный
   `snapshot={boot_id,event_seq}`. Это не общий атомарный снимок всех owners.
3. Подписаться после минимального сохранённого cursor, включая cursor discovery,
   с aggregate filter, покрывающим нужные state kinds и configuration/host changes.
   Буфер replay ограничен; при gap начать rebuild.
4. Replay events с seq не новее snapshot данного адреса не откатывают его.
   Более новые invalidations требуют перечитывания; более новые signal snapshots
   можно применить по прежнему measurement contract.
5. Для проверки catch-up получить свежий fence query/cursor и дождаться
   продвижения подписки до него через events или subscription_progress, затем
   обработать накопленные invalidations. `accepted_cursor` не является барьером
   актуальности. Если во время перечитывания пришло ещё более новое событие,
   адрес остаётся dirty.

При event_gap, instance_changed, истечении frozen projection или локальном
переполнении нельзя объявлять частичное состояние актуальным. Boot/generation
не склеивать; изменения наблюдения не восстанавливают исход неопределённой команды.
Существующий ring остаётся 1024 события, один subscription/connection, фильтр
до 8 kinds и 16 targets; пустой список — wildcard.

## 6. Точный минимальный объём будущих изменений

В этом PR создаётся **только данный документ**. Нижеследующее — план code PR.

| Файл / область | Изменение | Чего не требуется |
|---|---|---|
| `apps/lab-runtime/src/protocol.rs` | Четыре OperationSpec, capability, strict argument/nested schemas, два новых bounded public error code; дополнить registry tests | Изменять 47 существующих specs, категории или protocol v1 |
| Новый `apps/lab-runtime/src/application/parameters.rs` | Конечный ParameterAddress, новый scalar codec, catalog/get projections, таблица поддержанных routes и создание candidates | Plugin registry, reflection, новый generic Core trait |
| `application.rs` и `application/delivery.rs` | Query dispatch; новая family существующего frozen slot; включение нового payload в прежний mutation/Recorder/configuration-event путь | Новые workers, очереди, subscription или storage |
| `sessions.rs` | Новый typed Mutation variant; deterministic equality исходного запроса | Менять scope/seq, capacity, TTL, старые enum варианты/их semantics |
| `application/operations.rs` | Decode/dispatch нового варианта; semantic Recorder intent с исходным source_operation, terminal projection | Второй admission или вложенная публичная mutation |
| `application/references.rs`, `application/controllers.rs` | При необходимости ограниченный reuse существующих dispatch helpers, когда candidate уже построен | Изменять domain guards, lifecycle и прежние результаты |
| `configuration_api.rs` | При необходимости выделить bounded read metadata одного owner, чтобы переиспользовать property metadata без JSON roundtrip | Расширять writable allowlist или PropertyValue |
| `docs/api/*`, новые/существующие API tests | Документировать только аддитивные ops, audit mapping и qualification | Переписывать старые examples/DTO ради единообразия |

Не нужны изменения `lab-core`, OutputAuthority, native drivers, transport
executors, Recorder worker/SQLite schema, configuration loader, Cargo dependencies.
Используются имеющиеся Rust/serde_json, checked conversions и existing owner.
Если при реализации появится необходимость изменить этот список, её нужно
обосновать отдельным ревью, прежде чем расширять scope.

Новый catalog ограничен 256 records на owner; get и terminal результата одного
параметра должны помещаться в существующие границы. Retained terminal — до
4096 bytes; event — до 4096; общий body — 16383. Непредставимые строки/metadata
не усекать. Preflight serialization гарантирует, что успешный эффект не потеряет
terminal из-за слишком большого нового DTO. Для bounded scalar setters это
можно доказать статическими максимальными размерами.

Никакой постоянной памяти API: каталоги/снимки и операции используют прежние
процессные bounded stores; SQLite остаётся архивом Recorder. Не увеличивать
очереди/лимиты для маскировки нагрузки от нового каталога.

## 7. Риски совместимости и меры проверки

| Риск | Как ограничить |
|---|---|
| ReadWrite ошибочно принимается за authority | Explicit adapter allowlist; отдельные отрицательные тесты Actuator/OutputAffecting/Action и подмены namespace |
| Новое имя обходит Reference protected audit credit | Сохранить semantic names и existing gates; fault/overload tests до и после |
| Same-ID duplicate зависит от изменившегося PID/Ramp snapshot | Дедуплицировать исходный typed scalar request, candidate строить только после admission |
| Lost update между клиентами/полями одного объекта | CAS общей domain revision; конфликт без автоматического merge/retry |
| Присваивание PID незаметно меняет поведение | Публично описать полный validation/reset; никакого автоматического pause/start |
| Fixed setpoint ошибочно считают live writable при Running | Сохранить guard `ReconfigureReference`; отдельный отрицательный тест |
| Core configured write расходится с deployment/Recorder | Не включать этот setter; не alias по имени |
| Новые generic queries запускают I/O/вычисление | Только read-side queries; сравнить отсутствие событий/ревизий/байтов |
| Несовпадение integer codec и переполнение durations | Новая tagged оболочка, decimal exactness, checked source conversion, границы i64/u64/Duration |
| Пагинация подменяет frozen snapshot или ломает старый token | Прежний shared slot/TTL; тесты всех комбинаций start/page |
| Мнимый универсальный event | Объявлять invalidate/snapshot_only честно; preserve order, gap/rebuild |
| Postcommit failure выдают за отсутствие изменения | Terminal failure + самостоятельный current query; без rollback обещаний и автоматических повторов |
| Большой enum/text/каталог ломает response/retained bounds | Проверять общий codec и bounded records; явный отказ без обрезки или увеличения лимитов |
| Старые клиенты зависят от точного hello operation count | Старые DTO/specs сохраняются; тест реестра меняется только на 4 новых ops; документировать capability negotiation |

Отдельный компромисс: архив нового `parameter_set` сохраняет доменное имя операции,
а источник запроса находится в audit data. Это позволяет не менять Recorder,
но должно быть утверждено как публичная семантика новых записей. Альтернатива —
новые Recorder operation names и изменение всех admission/worker/SQL проверок;
для минимального расширения она существенно рискованнее.

## 8. Последовательные PR

| PR | Содержание | Критерий завершения |
|---|---|---|
| 0 — этот Draft | Аудит, контракт, границы и regression plan | Ревью адресов, write coverage и audit mapping; production-код отсутствует |
| 1 — каталог/get | Новый codec, bounded catalog/pages, get, capability; все writes пока unsupported | Любой клиент читает температуру, Reference/PID, bool/text properties и output readback; старые query/event сценарии проходят |
| 2 — live properties | Только уже разрешённые `display_name`, `poll_period_ms` через `parameter_set` | CAS двух sessions, overlay limit, events, Recorder lifecycle и прежний property API доказанно сохранены |
| 3 — Reference | Fixed scalar и Ramp target/rate, прежние guarded audit names/credit | Fixed Running rejection, непрерывный live retune, Required/BestEffort overload/failure/dedup проходят |
| 4 — PID | Scalar adapters полного PID/native config, без смены bindings/lifecycle | Reset semantics, state restrictions, coupled validation и old full-config операции эквивалентны |
| 5 — qualification | Общий TCP/WS сценарий, Debug/Release matrix, документация фактических результатов | Независимые sessions, manual output и Recorder проходят в обеих сборках; gap/unknown не замаскированы |

`parameter_set` рекламировать только с появлением первого работающего setter
(PR 2). Catalog metadata отражает реально реализованный support на каждом шаге.
Capability version 1 заранее описывает возможность unsupported адресов; клиент
проверяет operation list и metadata, не номер этапа.
Bool/enum setters, Core ConfigureParameter exposure, batch updates и manual
output alias не входят в эти PR и требуют отдельного задания.

## 9. Проверки до и после реализации

### 9.1. Неизменённая регрессионная база

Перед code PR зафиксировать результаты на базовом SHA, toolchain, profile и
transport; повторить те же тесты после. Старые тесты не ослаблять ради новых ops.
В registry equality tests дополнить только ожидаемые новые записи, сохранив
проверку всех старых аргументов/availability.

| Существующие тесты | Что обязано сохраниться |
|---|---|
| `crates/lab-core/tests/values.rs` | Все пять типов, без coercion; bounds, units, role/access/effect consistency |
| `crates/lab-core/tests/reference_operations.rs`, `controller_configuration.rs` | Непрерывность Ramp, revision, atomic validation, PID reset и lifecycle ограничения |
| `apps/lab-runtime/tests/api_owner_queries.rs`, `measurement_api.rs`, `control_api.rs` | Старые projections; старый DTO `reference_retune`; no query side effects |
| `protocol_foundation.rs`, `api_protocol.rs`, `request_deduplication.rs`, `operation_outcomes.rs`, `client_isolation.rs` | Hello-first, strict codec, exact request identity, next_seq, outcome retention/unknown, независимость clients |
| `runtime_events.rs`, `subscription_recovery.rs` | State-before-terminal, pure queries без событий, replay/gap, snapshot rebuild |
| `resource_configuration_api.rs`, `recorder_property_lifecycle.rs` | Revision-fenced properties, generation сохранён, durable lifecycle в Idle/Recording, отсутствие replay |
| `manual_output_api.rs`, `configured_physical_output.rs`, Core `milestone9d_physical_output.rs` | Существующие manual/automatic authority, точное кодирование, ACK/readback/ambiguity |
| `transport_parity.rs`, `websocket_transport.rs`, `tcp_lan.rs` | Общие semantics TCP/WS, миграция retained scope, session isolation, bounds/slow peers |
| `recorder_admission.rs`, `recorder_operations.rs`, `recorder_required.rs`, `recorder_evidence.rs`, `recorder_receipts.rs`, `recorder_backpressure.rs` | Protected Reference credit, Required gates, COMMIT evidence, provenance и отсутствие ложной durability |
| Внутренние `host/reference_completion_tests.rs`, `host/reconnect_recorder_ordering_tests.rs`, `service/property_lifecycle_tests.rs`, `recorder/property_lifecycle_tests.rs` | Late receipt ordering, SQL commit failure, postcommit failure, правильное lifecycle operation_kind |
| Полный workspace suite, включая Recorder reopen/shutdown/history | Старые SQLite archives, stop/flush и перезапуск не изменились |

Пути в строках без полного prefix относятся к `apps/lab-runtime/tests`, кроме
явно обозначенных внутренних файлов в `apps/lab-runtime/src`.

### 9.2. Новые проверки контракта

1. **Codec и каталог.** Все пять typed variants round-trip; integer i64 min/max,
   значения больше 2^53, u64 max и Duration read bounds без округления; неверный
   тег, дробный integer, bool 0/1, enum вне choices, nonfinite/overflow, unknown/
   duplicate keys, UTF-8 bytes и общий JSON budget отвергаются. Production enum
   проверять на read-only `kind`/`state`; generic enum codec — на тестовом DTO,
   не выдавая его за существующий driver setter.
2. **Адреса.** Одинаковый display name/ID в разных owners/namespaces не смешивается.
   Неизвестный key отвергается; переименование не меняет identity. Configuration
   descriptor и одноимённый deployment property не alias. Action/WriteOnly,
   output-affecting и неподдержанный scalar не становятся writable.
3. **Snapshots.** Get не меняет state/events/transport. Каталог frozen при изменении
   между страницами; TTL и slot replacement работают с тремя старыми families.
   Oversized record явно отклоняется. Неobserved/unavailable не заменяются нулём.
4. **Две сессии.** A/B читают revision R; один новый set проходит, второй получает
   conflict без эффекта. Повтор A с тем же ID возвращает исходный terminal после
   изменения B; смена только op на старую эквивалентную operation конфликтует.
   Разные PID fields используют общую revision. Capacity rejection не тратит seq.
5. **Reference.** Fixed set при Warming/Running не проходит; нет hidden pause.
   Ramp target/rate сохраняют continuity и второе поле, positive rate/CAS.
   Kind меняется только прежней полной операцией; старый адрес не меняет новый kind.
6. **PID.** Ready/Paused success и точный reset; Created/Warming/Running/Failed
   следуют прежним guards; lease/warming conflict и profile bounds не обходятся.
   Неверная пара limits не оставляет частичного commit; bindings не меняются.
7. **Properties.** Имя/period работают только на поддержанных targets, integer
   boundary и 32 overlays сохраняются. Bool/enum/deployment_only/rebind properties
   rejected. Compiled profile не получает fictitious deployment backend.
8. **Manual output.** A владеет выходом, B наблюдает его через get; B не может
   написать его generic setter, renew/write/safe за A или начать PID. Проверить
   также попытки A обойти expired lease через parameter_set. Подсчитать bytes/
   proposals: отвергнутый generic set не делает физической попытки.
   Прежний output_write успешен только по прежнему пути; timeout после send
   остаётся ambiguous, safe readback не подменяет receipt старой записи.
9. **События.** Старый set виден новому наблюдателю и новый set — старому.
   Нет дополнительного parameter event и дублированного terminal. Reference/
   controller facts предшествуют terminal; property configuration invalidation
   приводит к get. Update между snapshot и subscribe, filtered progress, ring
   overflow, reconnect и новый boot проверяются без пропусков/ложной Fresh.
10. **Recorder и результат.** Повторить Reference credit tests через оба API:
    обычная очередь заполнена, две sessions, Required failure до admission,
    поздний receipt, SQL failure, BestEffort explicit gap. Проверить protected
    group, semantic command и source_operation, один effect/accepted/terminal.
    Property activation по-прежнему сохраняет lifecycle kind/revision и post-COMMIT
    границы. Terminal success не заменяет durability status; postcommit failure
    не объявляется rollback. Reopen старого архива и новые записи без schema change.
11. **Потеря ответа.** Разрыв до получения accepted и после него; readback текущего
    значения не подтверждает старую команду. Retained operation_status/Exact Retry
    только явные; TTL eviction, scope_unknown, instance_changed и high-water
    исключают исполнение старого ID. Новый endpoint не запускает retry.
12. **Нагрузка.** Slow TCP/WS peer и повторные bounded catalog/get не блокируют
    acquisition, leases, safety и Recorder. Не появляются неограниченные очереди.
    Размер completed payload гарантирован до эффекта; saturating/wrapping
    revision запрещены.

### 9.3. Матрица и изоляция артефактов

Обязательные сочетания — **Debug/Release × TCP/WebSocket**. В каждом — два
независимых clients (отдельно проверить смешанную пару TCP+WS), legacy/new API
в обе стороны, Recorder unconfigured/Idle/Recording/Required failure/BestEffort
failure и manual output с virtual и byte-transport fixtures.
Virtual/fixture результат не является квалификацией реального оборудования.

Перед будущим запуском создать подпапки только внутри sandbox:

```powershell
$auditSandbox = 'D:\lab-runtime-parameter-sandbox'
New-Item -ItemType Directory -Force -Path "$auditSandbox\tmp", "$auditSandbox\target" | Out-Null
$env:TEMP = "$auditSandbox\tmp"
$env:TMP = "$auditSandbox\tmp"
$env:CARGO_TARGET_DIR = "$auditSandbox\target"
cargo fmt --all -- --check
cargo test --workspace --locked
cargo test --workspace --locked --release
cargo clippy --workspace --all-targets --locked -- -D warnings
```

Полная последовательность выше — план будущих запусков, не выполненная здесь
регрессионная проверка. Перед ними проверить
fixture paths и inherited env дочерних процессов: одной установки target dir
недостаточно для SQLite/log/temp. Baseline checkout при необходимости — отдельный
worktree **этого же репозитория** внутри sandbox, без переключения/изменения main.
Все базы/WAL/SHM, stdout/stderr и отчёты тестов остаются в sandbox. Не использовать
реальные научные данные и не удалять сторонние файлы. Toolchain/dependencies
не обновлять в рамках проверки.

## 10. Решения для ревью и результат этого этапа

Предлагается утвердить:

1. Единый address с owner/namespace/key; четыре additive операции с reuse
   существующей пагинации и nullable CAS token только подходящих domains.
2. Пять типов в новом codec без переписывания старых DTO; поддержка чтения
   не равна поддержке записи.
3. Scalar adapters Reference/PID/live properties, сохраняющие complete candidate,
   revision, state guards, reset, Recorder gates и события.
4. Отказ от generic actuator writes, Core ConfigureParameter exposure и новых
   bool/enum setters до отдельного доменного решения.
5. Сохранение semantic operation names в Recorder с явным source_operation
   нового wrapper-запроса.

Открытые вопросы ограничены этими решениями: приемлема ли audit mapping;
достаточно ли read-only boolean/enum в первой итерации; нужен ли позднее live
configured setter и какие lifecycle/revision/events он должен иметь.
Отдельного решения о безопасности или владении выходами не предлагается.

Этот этап содержит технический аудит и проект контракта. Production-код,
Cargo.toml, Cargo.lock, тестовые исходники и основная ветка не изменены.
Сборка, Debug/Release regression matrix и физические проверки не выполнялись.
Для этого документа проверены JSON-примеры, относительные ссылки, Git diff
и `cargo fmt --all -- --check`; formatting check прошёл без изменений исходников.
Следующий шаг — ревью Draft PR; реализация и merge требуют следующего задания.
