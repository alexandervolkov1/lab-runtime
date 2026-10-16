# lab-runtime

Программа для лабораторных экспериментов без графического интерфейса. Runtime
получает измерения от приборов, управляет мощностью вручную или через PID-регулятор
и сохраняет историю в SQLite. Клиент подключается по одному
[API](docs/api/README.md) через TCP или WebSocket; язык клиента не ограничен.
Отключение клиента не завершает эксперимент.

Для знакомства есть встроенная модель печи: температура, мощность нагревателя,
уставка и PID. Для оборудования поддерживаются последовательные приборы Metakon
и описания SimpleDevice.

## Установка и первый запуск

Скачайте готовый комплект со страницы
[v0.1.0-preview.8](https://github.com/alexandervolkov1/lab-runtime/releases/tag/v0.1.0-preview.8).

| Платформа | Файлы |
|---|---|
| Windows x64 | `lab-runtime-v0.1.0-preview.8-windows-x86_64.zip` |
| Linux x86_64 | `lab-runtime-v0.1.0-preview.8-linux-x86_64.tar.gz` и `lab-runtime-v0.1.0-preview.8-linux-x86_64.licenses.tar.gz` |

Сохраните соответствующие `.sha256` и `.build.json`. Архивы **Source code**
содержат исходники, а не готовое приложение.

Распакуйте комплект в новую папку и откройте терминал в каталоге с приложением.
На Windows нужен Microsoft Visual C++ Redistributable x64. На Linux сохраните
рядом с приложением содержимое соответствующего архива лицензий; совместимость
glibc указана в файле `.build.json` комплекта.

PowerShell:

```powershell
.\lab-runtime.exe --serve --profile virtual-demo --port 7420
```

Linux:

```sh
./lab-runtime --serve --profile virtual-demo --port 7420
```

Строка `"state":"ready"` означает, что API слушает `127.0.0.1:7420`.
Приложение продолжает работать до Ctrl+C или команды остановки. Этот профиль
не открывает физические приборы. Без аргументов программа выполняет короткую
демонстрацию и выходит.

[Руководство пользователя](docs/README.md) показывает подключение, новые измерения
по подписке, ручную мощность, PID, запись и чтение истории, настройку прибора
и удалённый доступ через tuna. Там же — необязательная проверка одним внешним
Babashka-клиентом. Для обычного запуска Babashka не нужен.

## Сборка из исходников

Нужны Rust 1.95.0 и C toolchain: MSVC на Windows, GCC/Clang на Linux. SQLite
включён в сборку. Workspace содержит `apps/lab-runtime` и `crates/lab-core`.

```sh
cargo build --release --locked -p lab-runtime
cargo test --workspace --locked
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
```

Приложение появится в `target/release`. Упаковка описана
[отдельно](https://github.com/alexandervolkov1/lab-runtime/blob/main/docs/packaging.md).
Модель печи и программные тесты не подтверждают безопасность физического оборудования.
