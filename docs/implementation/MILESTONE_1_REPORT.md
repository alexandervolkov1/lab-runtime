# Foundation + Milestone 1 — итоговый отчёт

Дата: 2026-09-14. Статус: **выполнены все доступные требования Foundation и M1**.
Работа остановлена на domain foundation; переход к M2 не выполнялся.
Implementation checkpoint: `44aa678`; этот отчёт и ссылки на него — последующий
documentation-only commit.

## 1. Исходные условия и Foundation

Полностью прочитаны AGENTS, PROJECT_BRIEF, ARCHITECTURE_PLAN, MIGRATION_ANALYSIS_PLAN,
все пять architecture documents, три ADR, семь существовавших migration documents
и FOUNDATION_MILESTONE1_PLAN. Последний содержит 1114 строк и **обрывается в §29**
после начала code fence (две обратные кавычки). Исходный файл сохранён как получен;
невидимое продолжение не домысливалось. Ограничение заранее сообщено пользователю.

До implementation существовали незакоммиченные заполненные документы и migration
analysis. Они зафиксированы отдельными documentation commits. Локальный агрегат
`all.txt` не изменён и не включён в Git; добавлен в .gitignore вместе с /target/.
После baseline commits и перед созданием Rust workspace worktree был clean.

Основные architecture documents, а не только migration appendix, согласованы с
AF01–AF08: Furnace — один native controller с private composition, Reference отдельно;
пять parameter roles и explicit side effects; pure Queries и explicit refresh;
preserve/reset/reinitialize policies; раздельные ID/name/generation/revision;
catalog lifecycle; generic recorder diagnostics/evidence и SQLite как кандидат;
малый реальный Babashka slice на этапе 6 до recorder, короткий POC gate и отдельный
24/72-hour pre-release soak. Это целевые решения, **не реализация будущих этапов**.

AGENTS переведён в `implementation: milestone 1 domain foundation`.
[M1 design](MILESTONE_1_DESIGN.md) создан и закоммичен до Rust.

## 2. Реализованная модель

| Часть | Реализация |
| --- | --- |
| Workspace | Только crates/lab-core и apps/lab-runtime; host → core, внешних dependencies нет |
| Identity | Typed InstrumentId/ParameterId; SignalId — их пара; rename сохраняет identity/state; duplicate IDs запрещены, names допустимы |
| Descriptors | Stable ID/name, ValueSpec/type/range, Unit, access, пять roles, WriteEffect, optional signal |
| Values | Float, Integer, Boolean, Text, Enum; strict types, finite numeric values, inclusive ranges, bounded text/enum definitions |
| Units | °C, %, Pa, Unitless; nonnumeric physical units отклоняются; без conversion engine |
| Virtual instrument | temperature, heater_power metadata, base_temperature config, measurement_enabled fault injection |
| Generator | Простая deterministic функция elapsed time; не thermal plant и не Furnace |
| Ownership | Один synchronous Runtime, private registry/implementation/buffers, owned query snapshots |
| Commands | RegisterVirtual, RenameInstrument, ConfigureParameter, RefreshMeasurement; typed results/errors |
| Queries | Discover, DescribeInstrument, GetInstrumentState, GetLatestSignal, GetSignalWindow; без I/O/clock/mutation |
| Reconfiguration | Validate-before-commit, preserve observations, настройки влияют на следующий explicit refresh |
| Samples | SignalId, Unit, monotonic Duration, Good(value) либо Unavailable(reason); до refresh latest отсутствует |
| Failure | Failed refresh записывает Unavailable с временем попытки и возвращает typed MeasurementUnavailable; не zero/NaN/old-fresh |
| Memory | 1..=4096 samples на instrument, oldest eviction перед push, максимум 64 instruments; latest = back того же window |
| Time | Injected elapsed Duration; zero допустим первым, затем строгое увеличение per signal; query не продвигает simulation |
| Host | Конечная descriptor-driven console demo, пять explicit samples, window из последних трёх |

Output-affecting writes **не исполняются**. heater_power заявляет actuator semantics,
но ConfigureParameter возвращает OperationNotAllowed; ни commanded, ни fabricated
observed power нет. AccessMode сам по себе не даёт output permission.

Нет generations, revisions, leases, events/cursors, deduplication или async:
в синхронном single-owner M1 нет replacement/in-flight completion и optimistic
concurrency. Это обоснованный scope cut, не попытка объявить будущие механизмы ненужными.

## 3. Выполнение разделов плана

| Разделы | Результат / свидетельство |
| --- | --- |
| 1–4 | Полное чтение; исходный Git status/log; donor baseline; раздельные doc commits и clean baseline |
| 5–6 | Reconciliation в основных architecture docs/ADR/POC и phase/scope в AGENTS |
| 7 | Design-before-code: commit 1f4ec2e раньше test scaffold 09a9855 |
| 8–12 | model.rs: minimal types, typed IDs, values, units, descriptors, validation |
| 13–15 | virtual_instrument.rs: native generator без hardware, injected Duration |
| 16–18 | signal.rs: explicit quality/time/value и bounded recent window, без SeriesStore |
| 19–21 | runtime.rs: ownership, synchronous in-process Commands/Queries, без external DTO/API |
| 22–24 | Два package, std-only core, host → core, без async/dependency stack |
| 25 | Typed errors; validation failures atomic, measurement failure отдельно документирован |
| 26 | Все девять обязательных acceptance scenarios проходят |
| 27 | Дополнительные применимые edge cases проходят; removal не добавлен ради тестов |
| 28–29 (доступная часть) | Ни одного donor production body/dependency; provenance test intents в design и tests |

## 4. Tests-first и acceptance

Первый `cargo test --workspace` до implementation: exit 1, отсутствующие domain
types/API (181 compiler errors). Acceptance suite закоммичен в `09a9855` в
намеренно red состоянии. Затем перед value implementation отдельно добавлены
value tests: red из-за отсутствующего API, после реализации — 5 passed.
Промежуточный `cde6c6b` закрывает value layer, но ещё не весь runtime suite;
`2b96995` переводит весь тогдашний workspace suite в green.

Перед host body добавлен executable smoke test: red на отсутствующем demo output,
затем green после реализации. Два дополнительных boundary regression tests добавлены
при review уже реализованного core, без заявления, что они предшествовали всему коду.

| Обязательный scenario | Test в crates/lab-core/tests/milestone1.rs |
| --- | --- |
| 1. Generic discovery | discovery_is_generic_and_initial_observation_is_unknown |
| 2. Descriptor completeness | descriptor_exposes_type_units_access_roles_and_side_effects |
| 3. Stable rename | rename_preserves_identity_configuration_and_samples |
| 4. Atomic invalid configuration | invalid_configuration_is_atomic_for_type_range_and_access |
| 5. Query purity | queries_are_pure_owned_snapshots_and_do_not_advance_generator |
| 6. Explicit refresh/time | explicit_refresh_uses_supplied_time_and_configuration_preserves_old_observation |
| 7. Measurement failure | measurement_failure_is_new_unavailable_observation_not_stale_success |
| 8. Bounded memory | window_evicts_oldest_and_latest_matches_back_at_all_supported_small_bounds |
| 9. Deterministic run | identical_commands_and_clock_sequence_have_identical_results |

Ещё семь core acceptance tests проверяют duplicate ID/name policy, unknown IDs,
invalid/equal/backward time, registration limits, Duration::MAX, initial failure и
maximum-capacity eviction. Пять value tests покрывают варианты/types/constraints,
units/descriptor consistency и typed error display. Один host test запускает demo
дважды и проверяет идентичность output и bounded window.

## 5. Фактическая verification

Среда: Windows/PowerShell, rustc 1.95.0 (59807616e 2026-04-14),
cargo 1.95.0 (f2d3ce0bd 2026-03-21).

| Проверка | Результат |
| --- | --- |
| cargo test --workspace | Exit 0: **22 passed**, 0 failed, 0 ignored (16 core acceptance + 5 values + 1 host) |
| cargo test --workspace --locked --offline --release | Exit 0: те же **22 passed**, 0 failed, 0 ignored |
| cargo fmt --all -- --check | Exit 0 |
| cargo clippy --workspace --all-targets --locked --offline -- -D warnings | Exit 0, warnings отсутствуют |
| cargo run -p lab-runtime | Exit 0: 20..24 °C на t=0..4s, window=3, oldest=2s/latest=4s |
| cargo tree --workspace --edges all / cargo metadata --no-deps --locked --offline | Только два package, lab-core без dependencies, host зависит только от lab-core |
| git diff --check cd20dc5..HEAD | Exit 0, whitespace errors нет |
| Relative Markdown file links | Все проверенные links в docs и README существуют; anchors не проверялись |
| Scope review / Rust source scan | Нет runtime OS/clock/thread/network/VM/storage/output APIs или forbidden dependencies |

Unit-test/doc-test targets с нулём tests не включены в число 22.
Нет sleeps, real-time waits и случайности в measurement tests.
Default и release проходят независимо; это не hardware/real-time/soak certification.

## 6. Donor baseline и сохранность

Полный фактологический baseline: [DONOR_BASELINE.md](../migration/DONOR_BASELINE.md).

- Branch feature/rust-core-api, HEAD 50d3d1e3de84c650e1aa0ffbf1625044f794d315,
  describe v0.1.0-34-g50d3d1e; это не tagged-release HEAD.
- Запущен исходный `cargo test`: library **784 passed, 1 failed, 1 ignored**.
- Failure: lua_api::documentation_tests::documentation_relative_links_and_anchors_resolve,
  broken ../BABASHKA_MIGRATION_PLAN.md из docs/BABASHKA_HANDOFF.md.
- Cargo остановился на library failure; последующие targets не засчитаны.
  Donor не исправлялся, failed test не исключался.
- Повторная финальная read-only проверка: HEAD и clean status сохранены;
  SHA-256 manifest всех 181 tracked files до/после совпал:
  5AD5F84B77825E558C667E1C201376FAFCE556658969B0F9114E5ED986728ACA.

Игнорируемые Cargo build artifacts могли создаваться исходным test run; source,
tracked documents и manifests старого репозитория не менялись. Новые tests опираются
только на documented rename/validation intent, production bodies написаны заново.

## 7. Логические commits

| Commit | Содержание |
| --- | --- |
| 4f8fd54 | Filled project brief/AGENTS и naming baseline |
| 7117d79 | Snapshot исходной high-level architecture и ADR |
| 83ead05 | Snapshot migration analysis и его плана |
| 45eb5f1 | Foundation scope, donor baseline, ignore локального aggregate/build output |
| 1b5afb4 | Основная architecture reconciliation и M1 phase |
| 1f4ec2e | Minimal M1 design до code |
| 09a9855 | Два package и acceptance tests, намеренно red |
| cde6c6b | Typed identity/descriptors/validation и value tests |
| 2b96995 | Runtime owner, virtual measurements и bounded signal state; green suite |
| 906941f | Boundary regressions для unavailable/max window |
| 44aa678 | Descriptor-driven host demo, executable test и README |
| Финальный documentation-only commit | Этот отчёт, completion links и stop note |

Первоначальные snapshots сохраняют ранее подготовленные пользовательские документы,
а не выдают их за заново выполненный анализ этого implementation этапа.

## 8. Ограничения и остановка

M1 — library foundation и finite demo, **не production deployment целого Runtime**.
Нет serial/Metakon, OutputArbiter, output execution, controllers, Lua, Babashka/IPC,
recorder, GUI, persistence, removal/replacement, subscriptions или multi-client service.
Нет physical safety profile, hardware tests, timing guarantees и 24/72-hour soak.
Memory bounds описывают retained domain data, не hard RSS/allocator limits; клиент
может хранить сколько угодно owned query snapshots вне Runtime.

Неполный donor baseline и обрыв исходного Foundation plan явно сохранены как
ограничения, не скрыты за green статусом M1. Все доступные требования M1 выполнены.
Продолжение, включая OutputArbiter/Milestone 2, требует новой команды пользователя.
