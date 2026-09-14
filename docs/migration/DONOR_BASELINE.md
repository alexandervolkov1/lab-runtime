# Donor test baseline — Foundation

Дата проверки: 2026-09-14. Donor: `D:\rust\com_port_reader`, read-only source, не dependency lab-runtime.

| Поле | Фактическое значение |
| --- | --- |
| Branch | `feature/rust-core-api` |
| Commit | `50d3d1e3de84c650e1aa0ffbf1625044f794d315` |
| Describe | `v0.1.0-34-g50d3d1e` |
| Cargo package version | `0.1.0`; HEAD не равен tagged release |
| Working tree before | Clean, `git status --porcelain=v1 --untracked-files=all` без вывода |
| Environment | Windows, PowerShell; rustc `1.95.0 (59807616e 2026-04-14)`, cargo `1.95.0 (f2d3ce0bd 2026-03-21)` |
| Command | `cargo test` из donor root, без фильтров и изменения исходников |
| Result | Exit 1; library: 786 tests, **784 passed, 1 failed, 1 ignored**, 0 measured, 0 filtered out; 2.69 s test runtime |
| Working tree after | Clean; tracked-content manifest совпадает с начальным |

## Точный failure

Failing test: `lua_api::documentation_tests::documentation_relative_links_and_anchors_resolve`.

```text
thread 'lua_api::documentation_tests::documentation_relative_links_and_anchors_resolve' (14860) panicked at src\lua_api\documentation_tests.rs:28:13:
D:\rust\com_port_reader\docs\BABASHKA_HANDOFF.md: broken link ../BABASHKA_MIGRATION_PLAN.md

test result: FAILED. 784 passed; 1 failed; 1 ignored; 0 measured; 0 filtered out; finished in 2.69s
error: test failed, to rerun pass `--lib`
```

Cargo остановился на library test target. Integration/binary/doc-test targets после этого не считаются выполненными этим запуском. Failed test не отключался, donor не исправлялся, повторный запуск с исключением failure не выдавался за green baseline.

## Влияние на migration assets

Failure относится к целостности ссылки документации, а не к обнаруженному провалу numeric/validation/identity алгоритмов. T11 documentation-completeness asset требует восстановить контекст отсутствующего документа при будущем переносе; ссылка не является specification нового API. T14 external integration assets остаются статически изученными, без полного fresh-run подтверждения здесь. Milestone 1 использует только заново написанные tests stable rename, typed validation, atomic rejection; production bodies v1 не копируются. Поэтому данный failure не блокирует domain foundation и не освобождает её от собственной verification.

## Неизменность donor и ограничения

До и после теста manifest SHA-256 всех 181 tracked files: `5AD5F84B77825E558C667E1C201376FAFCE556658969B0F9114E5ED986728ACA`. Метод тот же, что в [inventory](V1_FEATURE_INVENTORY.md): UTF-8 `path SHA256`, LF, порядок `git ls-files`. HEAD и clean status не изменились. Cargo/test-generated ignored artifacts не являются изменением donor source; никаких ручных записей/исправлений в donor не делалось.

Это test-suite baseline, **не hardware-tested certification**. Физический стенд отдельно не запускался; ignored test и не запущенные targets не засчитываются.
