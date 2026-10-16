# Руководство пользователя

Runtime работает отдельным процессом. Он хранит состояние приборов и эксперимента;
клиент получает данные и отправляет команды. Закрытие клиента не останавливает
PID или Recorder. Для знакомства используйте встроенную печь, которая не открывает
последовательные порты.

## Установка

Откройте [релиз v0.1.0-preview.8](https://github.com/alexandervolkov1/lab-runtime/releases/tag/v0.1.0-preview.8)
и скачайте файлы своей платформы из раздела **Assets**:

| Платформа | Файлы |
|---|---|
| Windows x64 | `lab-runtime-v0.1.0-preview.8-windows-x86_64.zip` |
| Linux x86_64 | `lab-runtime-v0.1.0-preview.8-linux-x86_64.tar.gz` и `lab-runtime-v0.1.0-preview.8-linux-x86_64.licenses.tar.gz` |

Для каждого архива сохраните соответствующий `.sha256`, а для платформы —
`.build.json` со сведениями о сборке. **Source code** для обычного запуска не нужен.
Сравните SHA-256 скачанных архивов с сопровождающими файлами:
`Get-FileHash -Algorithm SHA256 путь-к-архиву.zip` в PowerShell либо
`sha256sum -c имя-архива.tar.gz.sha256` в Linux. Распакуйте в новую папку.

В Windows установите Microsoft Visual C++ Redistributable x64 при ошибке
отсутствующего `VCRUNTIME140.dll`. На Linux нужен x86_64 с glibc не старее
версии, указанной в `.build.json`. Сохраните лицензии: после распаковки Runtime
перейдите в каталог с `lab-runtime` и распакуйте туда соответствующий companion:

```sh
tar -xzf /путь/к/lab-runtime-v0.1.0-preview.8-linux-x86_64.licenses.tar.gz
```

Имя выше — пример версии; используйте файл из того же комплекта.
Все последующие команды запуска выполняются из каталога с исполняемым файлом.
Rust, Python и Babashka для запуска готового приложения не нужны.

## Печь и база экспериментов

Если уже запущен пример из README, остановите его через Ctrl+C.
Создайте папку для данных и включите Recorder при запуске.

PowerShell:

```powershell
New-Item -ItemType Directory -Force .\state | Out-Null
$db = [IO.Path]::GetFullPath((Join-Path $PWD "state/history.sqlite"))
.\lab-runtime.exe --serve --profile virtual-demo --port 7420 --record-db $db --record-policy required
```

Linux:

```sh
mkdir -p state
./lab-runtime --serve --profile virtual-demo --port 7420 \
  --record-db "$PWD/state/history.sqlite" --record-policy required
```

Дождитесь строки `{"boot_id":"…","port":7420,"state":"ready"}`.
В этом профиле прибор `1` — печь, параметр `1` — температура, параметр `2` —
мощность в процентах. Уставка и PID имеют ID `1`. На реальном приборе ID и единицы
нужно получить через обнаружение и описание, а не переносить из этого примера.

`--record-db` требует абсолютный путь к локальному файлу. Каталог должен существовать
и быть доступен для записи. Можно выбрать свой каталог на C:, D: или другом локальном
диске; временный каталог не требуется. SQLite входит в приложение. Один процесс
Runtime должен владеть базой. Включение Recorder открывает архив, но ещё не начинает
эксперимент: для этого используется `recording_start`.

Политика `required` запрещает продолжать обычное управление при потере обязательной
записи. `best-effort` допускает работу без полной истории и явно показывает отказ
Recorder. Она не исправляет неисправный диск и не скрывает пропуски.

## Подключение и первое измерение

TCP слушает `127.0.0.1:7420`. Каждое сообщение — UTF-8 JSON с переводом строки.
Первый запрос на каждом соединении — `hello`. Ответ содержит `scope` (идентификатор
клиентской сессии), `next_seq` для следующей команды и список доступных операций.

В отдельном PowerShell можно запросить температуру без дополнительных программ:

```powershell
$tcp = [Net.Sockets.TcpClient]::new("127.0.0.1", 7420)
$stream = $tcp.GetStream()
$stream.ReadTimeout = 10000
$stream.WriteTimeout = 10000
$reader = [IO.StreamReader]::new($stream, [Text.UTF8Encoding]::new($false))
$writer = [IO.StreamWriter]::new($stream, [Text.UTF8Encoding]::new($false))
$writer.NewLine = "`n"
$writer.AutoFlush = $true
$writer.WriteLine('{"v":1,"msg_id":"hello","op":"hello","args":{"scope":null}}')
$hello = $reader.ReadLine() | ConvertFrom-Json
$hello.result
$writer.WriteLine('{"v":1,"msg_id":"temperature","op":"latest","args":{"signal":{"instrument":"1","parameter":"1"}}}')
$reader.ReadLine() | ConvertFrom-Json
$tcp.Dispose()
```

В Linux тот же запрос можно выполнить средствами Bash; `timeout` ограничивает
ожидание всего примера:

```bash
timeout 5 bash -c '
exec 3<>/dev/tcp/127.0.0.1/7420
printf "%s\n" "{\"v\":1,\"msg_id\":\"hello\",\"op\":\"hello\",\"args\":{\"scope\":null}}" >&3
IFS= read -r hello <&3
printf "%s\n" "$hello"
printf "%s\n" "{\"v\":1,\"msg_id\":\"temperature\",\"op\":\"latest\",\"args\":{\"signal\":{\"instrument\":\"1\",\"parameter\":\"1\"}}}" >&3
IFS= read -r sample <&3
printf "%s\n" "$sample"
'
```

Успешный ответ имеет `type:"result"`, соответствующий `msg_id` и объект `result`.
Для температуры ожидаются `quality:"good"`, числовое `value` и единица `degC`.
`latest` читает уже полученное измерение, а не запускает опрос прибора.

Для обнаружения отправьте `discover` с `args:{}`, затем при `complete:false`
получайте `discovery_page` с возвращёнными `projection` и `next_index`.
`describe {"instrument":"1"}` возвращает параметры, типы, единицы и разрешённые
действия. Все запросы далее используют ту же оболочку `v/msg_id/op/args`.

## Новые точки по подписке

Не нужно постоянно вызывать `latest`. Получите текущие данные через
`measurements_current` и страницы `measurements_page`. В первом ответе сохраните
`revision.boot_id` и `revision.event_seq`, затем отправьте:

```json
{"v":1,"msg_id":"subscribe","op":"subscribe","args":{"after":{"boot_id":"<revision.boot_id>","seq":"<revision.event_seq>"},"filter":{"kinds":["signal"],"targets":[]}}}
```

Текст в угловых скобках заменяется полученными значениями. После ответа с токеном
`subscription` Runtime сам присылает сообщения `type:"event"`, `kind:"signal"`.
Их `data` содержит новое измерение, а `target` — прибор и параметр. Например,
существенные поля события для печи:

```json
{"type":"event","kind":"signal","target":{"instrument":"1","parameter":"1"},"data":{"quality":"good","value":20.1,"observed_at_ns":"1200000000"}}
```

Это фрагмент события: полная оболочка также содержит версию, boot/sequence и время
публикации. `subscription_progress` сообщает продвижение курсора без подходящего
события. Ответы на запросы и события могут приходить вперемежку: различайте `type`,
а ответы сопоставляйте по `msg_id`. Для остановки подписки отправьте `unsubscribe`
с её токеном. При `event_gap` заново получите снимки и подписку, не склеивайте
неполную историю. Внешний пример ниже проверяет несколько новых точек подряд,
не вызывая `latest` или `measurement_window` во время ожидания.

## Команды, мощность и PID

Команда имеет дополнительный `request_id:{"scope":"<hello.scope>","seq":"<next_seq>"}`.
Используйте следующий номер для каждой новой принятой команды. `accepted` означает
приём; затем дождитесь `completed` или `failed`. Не считайте первый ответ завершением.

Для ручной мощности сначала запросите `output` с
`{"actuator":{"instrument":"1","parameter":"2"}}`. Дождитесь `safe_confirmed:true`
и отсутствия `owner`. Скопируйте `authority_instance` и `epoch` в
`expected_instance` и `expected_epoch`; перед каждой командой прочитайте их заново.

| Действие | Операция и дополнительные аргументы |
|---|---|
| Получить право управления на 2 секунды | `output_acquire`, `lease_lifetime_ns:"2000000000"` |
| Поставить 10%, затем 20%, затем 15% | `output_write`, числовое `value:10.0` / `20.0` / `15.0`, `proposal_ttl_ns:"200000000"` |
| Продлить право ещё на 2 секунды от текущего времени Runtime | `output_renew`, `lease_lifetime_ns:"2000000000"` |
| Отозвать право и запросить безопасное значение | `output_safe` |

Все четыре операции также получают `actuator` и два свежих поля `expected_*`.
Lease — конечный срок права управления. Его истечение отзывает право и запускает
безопасный переход; подключение клиента само по себе срок не продлевает.
Команда `output_safe` завершается после запроса перехода: подтверждение проверяйте
отдельно через `output.safe_confirmed` и отсутствие владельца.

У завершённого `output_write` проверяйте `terminal` и `outcome`.
`acknowledged` означает ACK прибора; `readback_verified` — также проверенное чтение
значения. Это не независимое доказательство физического нагрева. Встроенная печь
явно помечает доказательства как `evidence_source:"virtual_simulation"`.
[Полный контракт ручного управления](api/manual-output.md) описывает все поля.

Для PID сначала завершите ручное управление через safe. Прочитайте
`reference {"reference":"1"}` и установите постоянную температуру:

```json
{"v":1,"msg_id":"setpoint","op":"reference_configure","request_id":{"scope":"<hello.scope>","seq":"<next_seq>"},"args":{"reference":"1","expected_revision":"<reference.revision>","kind":"fixed","value":40.0}}
```

Затем `controller_start {"controller":"1"}` запускает PID. Запрос `controller`
покажет переход `warming` → `running`. `controller_pause` запрашивает безопасную
паузу; дождитесь `paused` и safe. `controller_resume` снова выдаёт конечное право
управления при допустимых условиях. Работающий PID и ручное управление одним
выходом несовместимы. Перед ручным управлением поставьте PID на паузу и дождитесь safe.
Изменять уставку можно через Reference; изменять PID-параметры — через
`controller_configure_pid` в разрешённом безопасном состоянии.

## Запись, история и остановка

После `hello` запросите `recording_status`. Для начала эксперимента отправьте
команду `recording_start {"label":"Нагрев печи"}` и сохраните возвращённый `run_id`.
Один файл содержит несколько экспериментов и запусков Runtime. Label — подпись,
а не уникальный идентификатор. Для заметки есть `experiment_annotate` с
`name` и JSON-объектом `data`; завершение этой команды означает постановку в запись,
а не COMMIT.

Перед завершением Required-записи поставьте PID на паузу или выполните manual safe
и дождитесь подтверждения. Отправьте `recording_stop {"run_id":<полученный объект>}`.
Успешный терминальный ответ должен содержать:

```json
{"completion":{"accepted_facts_drained":true,"interval_sealed":true,"transaction_committed":true}}
```

Это фрагмент `result`, а не вся оболочка. После stop база остаётся открыта,
можно начать следующий эксперимент. Измерения, состояния управления и результаты
операций записывает Runtime.

История читается через API. `history_read` — команда, поскольку планирует работу
с архивом; после её завершения запросите `history_page` и освободите страницу
через `history_release`:

| Выборка | Аргументы `history_read` |
|---|---|
| Эксперименты | `mode:"runs"`, `database_id` из статуса, `max_records:32`, `cursor:null` |
| Температура эксперимента | `mode:"measurements"`, тот же `database_id`, `boot_id` и `run_id` выбранного эксперимента, `signal:{"instrument":"1","parameter":"1"}`, `from_ns:"0"`, `to_ns` больше последней нужной отметки, `max_records:128`, `cursor:null` |

Завершение возвращает `page_token`; запрос `history_page {"page_token":"…"}`
возвращает `runs` или `rows`. Для продолжения используйте `next_cursor` с тем же
фильтром. Период измерений — монотонное время выбранного запуска, не Unix-время.
Точные поля и пределы приведены в [API](api/operations.md).

Ctrl+C или команда `runtime_shutdown {}` запрашивают штатную остановку.
На Linux SIGTERM и SIGHUP также останавливают Runtime; SIGHUP не перезагружает настройки.
При API-остановке проверяйте терминальное `recorder_flushed:true`; остановленный поток
или читаемая база сами по себе этого не доказывают.

Для резервной копии завершите эксперимент, остановите Runtime и проверьте результат.
Скопируйте базу вместе с оставшимися `-wal` и `-shm`, если они существуют.
После аварии сохраняйте все три файла до восстановления; не удаляйте WAL.
Закрытую копию можно проверить SQLite CLI в режиме чтения:

```sh
sqlite3 -readonly state/history.sqlite "PRAGMA integrity_check; SELECT state,coverage FROM runs; SELECT count(*) FROM gaps;"
```

Ожидаются `ok`, завершённые интервалы без пропусков и ноль gaps для успешного
примера. Integrity check проверяет структуру базы, а не полноту эксперимента.
[Контракт Recorder и схема](recorder-sqlite.md) описывают долговечность и восстановление.

## Внешняя проверка

Установите [Babashka](https://github.com/babashka/babashka#installation) только если
хотите запустить этот пример; проверена версия 1.13.220. В исходниках находится
один файл `clients/babashka-smoke/manual-output.clj`; он не входит в комплект
исполнения приложения. Можно взять этот файл отдельно из той же версии исходников.

Запустите свежую встроенную печь с Recorder, как показано выше. В другом терминале,
из корня исходников, команда одинакова для PowerShell и Linux:

```sh
bb clients/babashka-smoke/manual-output.clj --url tcp://127.0.0.1:7420 --shutdown
```

Пример сам обнаруживает печь, получает измерения по подписке, ставит мощность
10 → 20 → 15%, продлевает lease, проверяет safe и истечение короткого lease,
устанавливает 40 °C, запускает/приостанавливает/возобновляет PID, проверяет конфликт
ручного владельца, завершает запись и читает историю. `--shutdown` дополнительно
останавливает принадлежащий вам Runtime и проверяет flush. Без этого флага Runtime
остаётся запущен. Успех — строка JSON с `kind:"PASS"` и код выхода 0; ошибка —
`FAIL` и ненулевой код. Ожидание одной операции ограничено 10 секундами
(`--timeout-ms`, диапазон 1000–60000). Автоматических повторов команд нет.

Для WS перезапустите свежую печь, добавив после параметров Recorder:

```text
--ws-port 7421 --ws-origin http://localhost:3000
```

Затем запустите тот же клиент:

```sh
bb clients/babashka-smoke/manual-output.clj --url ws://127.0.0.1:7421/application/v1 --origin http://localhost:3000 --shutdown
```

Origin и подпротокол клиент отправляет в начальном HTTP Upgrade. Пример работает
только со встроенной виртуальной печью и не проверяет физический Arduino.

## Конфигурация и прибор

Для постоянных настроек скопируйте готовый пример и редактируйте копию:

```powershell
New-Item -ItemType Directory -Force .\my-config | Out-Null
Copy-Item .\examples\runtime.virtual.toml .\my-config\runtime.toml
.\lab-runtime.exe --serve --config .\my-config\runtime.toml
```

```sh
mkdir -p my-config
cp examples/runtime.virtual.toml my-config/runtime.toml
./lab-runtime --serve --config my-config/runtime.toml
```

Предыдущий Runtime нужно остановить. Файл сохраняйте в UTF-8 без BOM.
Не смешивайте `--config` и профильные параметры командной строки.
`recording.path = "history.sqlite"` в TOML означает файл рядом с конфигурацией,
а не рядом с приложением. JSON-описания приборов также разрешаются относительно
конфигурации; `..` и сетевые пути не допускаются.

Для реального прибора возьмите проверенное под вашу модель JSON-описание Metakon
или SimpleDevice и соответствующую конфигурацию. В исходниках есть
[Metakon 513](https://github.com/alexandervolkov1/lab-runtime/tree/main/test-data/fixtures/metakon)
и [SimpleDevice](https://github.com/alexandervolkov1/lab-runtime/tree/main/examples/simple-device).
Это примеры и регрессионные фикстуры: COM3/COM5/COM256, адрес и масштаб нельзя
считать настройками вашего прибора.

Скопируйте выбранный TOML и его каталог definitions в отдельную рабочую папку.
В `[[resources]]` задайте фактический порт, скорость, parity, data_bits, stop_bits,
flow_control и таймауты; в `[[instruments]]` — `definition`, `resource_id`,
адрес и период опроса. Сверьте операции, масштаб и единицы в JSON с документацией
прибора. Для первого чтения используйте read-only определение без controllers
и без writable/output-варианта. Например, для существующего read-only SimpleDevice
сохраните рядом `runtime.read-only.toml` и `read-only.json`, замените COM256
в TOML и запускайте:

```powershell
.\lab-runtime.exe --serve --config .\my-instrument\runtime.read-only.toml
```

```sh
./lab-runtime --serve --config my-instrument/runtime.read-only.toml
```

В Windows найдите порт в диспетчере устройств. В Linux укажите устойчивый
`/dev/serial/by-id/…` и предоставьте пользователю Runtime доступ к устройству.
Историческое имя типа ресурса `windows_com_read_only` используется на обеих
платформах. Само наличие подходящего порта не проверяет протокол прибора.

После ready выполните discover/describe, убедитесь в physical identity и сравните
измерение с показанием прибора. Этот физический прогон не выполнен; Arduino не
квалифицирован. Для подбора собственного определения используйте
[SimpleDevice](simple-device.md) и [справочник конфигурации](reference/configuration.md).

## Tuna: удалённый доступ по WSS

Этот рецепт соответствует [официальной документации tuna](https://tuna.am/docs/tunnels/http/).
Установите CLI по
[инструкции tuna](https://tuna.am/docs/guides/install/install-cli/) и авторизуйте
аккаунт командой `tuna login`. Аккаунтный токен не является ключом доступа к Runtime.

1. На машине Runtime запустите печь с loopback WS:

```powershell
$db = [IO.Path]::GetFullPath((Join-Path $PWD "state/tuna.sqlite"))
.\lab-runtime.exe --serve --profile virtual-demo --port 7420 --record-db $db --record-policy required --ws-port 7421 --ws-origin http://localhost:3000
```

```sh
./lab-runtime --serve --profile virtual-demo --port 7420 \
  --record-db "$PWD/state/tuna.sqlite" --record-policy required \
  --ws-port 7421 --ws-origin http://localhost:3000
```

2. В другом терминале задайте длинный случайный ключ доступа без записи в файл
или командную историю и откройте туннель.

PowerShell 7:

```powershell
$env:TUNA_KEY_AUTH = Read-Host "Ключ доступа к Runtime" -MaskInput
tuna http 127.0.0.1:7421 --request-header="Host:127.0.0.1:7421" --inspect=false --https-redirect
Remove-Item Env:TUNA_KEY_AUTH
```

Linux Bash:

```bash
read -rsp 'Ключ доступа к Runtime: ' TUNA_KEY_AUTH
printf '\n'
export TUNA_KEY_AUTH
tuna http 127.0.0.1:7421 --request-header="Host:127.0.0.1:7421" --inspect=false --https-redirect
unset TUNA_KEY_AUTH
```

Задайте непустой ключ перед запуском; не включайте туннель без защиты.
`127.0.0.1:7421` — локальный WS Runtime. `--request-header` заменяет HTTP Host
публичного адреса на разрешённый Runtime локальный Host; без этого Upgrade будет
отклонён. `TUNA_KEY_AUTH` включает проверку заголовка `X-Token`.
`--inspect=false` выключает инспектор запросов, `--https-redirect` перенаправляет
обычный HTTP на HTTPS. Удаление переменной выполняется после остановки tuna.

3. Tuna напечатает HTTPS-адрес вида `https://<публичный-домен>`.
Клиенту нужен `wss://<публичный-домен>/application/v1`: замените только
схему и добавьте путь. Скопируйте домен целиком из вывода tuna: его окончание
может отличаться от примеров в документации. Сразу после запуска tuna может
кратковременно отвечать «Туннель не найден» (HTTP 404); дождитесь доступности
туннеля до отправки команд Runtime.

На клиентской машине введите тот же ключ и запустите единственный внешний пример.

PowerShell 7:

```powershell
$env:LAB_RUNTIME_ACCESS_KEY = Read-Host "Ключ доступа к Runtime" -MaskInput
$endpoint = Read-Host "WSS URL, включая /application/v1"
bb clients/babashka-smoke/manual-output.clj --url $endpoint --origin http://localhost:3000 --timeout-ms 30000
Remove-Item Env:LAB_RUNTIME_ACCESS_KEY
```

Linux Bash:

```bash
read -rsp 'Ключ доступа к Runtime: ' LAB_RUNTIME_ACCESS_KEY
printf '\n'
export LAB_RUNTIME_ACCESS_KEY
read -rp 'WSS URL, включая /application/v1: ' endpoint
bb clients/babashka-smoke/manual-output.clj --url "$endpoint" --origin http://localhost:3000 --timeout-ms 30000
unset LAB_RUNTIME_ACCESS_KEY
```

Клиент автоматически передаёт подпротокол `lab-runtime.application.v1` и `X-Token`.
Проверка TLS-сертификата и имени сервера остаётся включённой. Origin
`http://localhost:3000` — согласованная метка источника подключения; она должна точно
совпадать с allowlist Runtime. Это не пароль и не авторизация: произвольный клиент
может выставить Origin сам. Здесь доступ защищают ключ туннеля и TLS. Разрешённый
клиент получает весь API, отдельной роли «только наблюдение» нет.

TCP по умолчанию также loopback, но не имеет шифрования и авторизации. Для
удалённого подключения по этому рецепту публикуйте именно WS через защищённый WSS.

## Ошибки и восстановление соединения

| Симптом | Что проверить или сделать |
|---|---|
| Нет ready, занят порт | Остановить предыдущий процесс или выбрать свободный порт; `--port 0` напечатает назначенный порт. |
| Ошибка конфигурации | UTF-8 без BOM, имена полей, пути и настройки прибора; неизвестные поля запрещены. |
| Recorder не открылся | Папка существует, локальная и доступна для записи; нет другого владельца базы. |
| `not_observed` / `unavailable` | Дождаться реального измерения или устранить причину в приборе; не подменять его числом. |
| `busy` при ручной мощности | Проверить владельца через output, состояние PID и Recorder. Не повторять команду вслепую. |
| `revision_conflict` | Заново прочитать объект и осознанно сформировать новую команду. |
| Lease истёк | Дождаться safe, заново оценить состояние; восстановление соединения не выдаёт новое право управления. |
| WS Upgrade отклонён | Путь, точный Origin, подпротокол и Host; через tuna также ключ X-Token. |
| Required Recorder failed | Управление закрывается, причина видна в recording_status и диагностике. Сохранить базу/WAL/SHM, устранить причину; не скрывать пропуск переключением политики. |

При обрыве соединения команда могла быть принята даже без полученного ответа.
Сохраните boot_id, scope, request_id, операцию и точные аргументы. На новом
соединении явно отправьте hello с прежним scope. При успешном присоединении
получите свежие снимки и новую подписку; только потом решайте, продолжать ли работу.
Состояние конкретной команды можно явно запросить через operation_status.
Не повторяйте автоматически ни команды, ни status/Exact Retry. Новый boot или
неизвестный scope требует нового сеанса и сверки состояния. `outcome_unknown`
не означает, что команда не исполнилась.

Диагностика настраивается `LAB_RUNTIME_LOG_DIRECTORY` и `LAB_RUNTIME_LOG_LEVEL`;
это журналы, не научная история. Подробности:
[восстановление](recovery-and-faults.md), [API](api/README.md),
[архитектура](architecture.md), [Recorder](recorder-sqlite.md).
