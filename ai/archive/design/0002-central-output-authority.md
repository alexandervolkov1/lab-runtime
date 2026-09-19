# ADR-0002: Единственный Rust output authority и проверяемый dispatch

Status: Proposed — проверяется будущим POC; не является заявлением о сертифицированной или уже реализованной safety.

Date: 2026-09-14.

## Context

Native, Lua, внешние controllers и manual clients могут конкурировать за один physical output. Ошибка script, разрыв связи, queued write или timeout могут оставить прибор в нежелательном состоянии. Проверка только числового `power` не защищает от альтернативной записи mode/reset/register. Подтверждение доставки команды также не равнозначно наблюдению физического состояния.

## Decision

Все воздействия проходят один Rust OutputArbiter. Producers передают OutputProposal; arbiter владеет OutputChannel, единственным owner, lease/epoch, limits, freshness/interlocks и safe policy. Доверенный Rust dispatcher исполняет только конкретную разрешённую operation/value и повторно проверяет epoch/deadline перед отправкой. Физический порт и transaction scheduling принадлежат Rust Runtime.

Разрешение охватывает все side-effect operations, включая configuration/mode/enable/reset и альтернативные register mappings. Клиент и Lua не получают raw write API. Hardware operation catalogs, native adapters и safety profiles входят в доверенную configuration/реализацию; произвольные bytes не позволяют автоматически доказать отсутствие скрытого side effect.

Revoke немедленно запрещает новые обычные воздействия, инвалидирует queued permits и запускает safe procedure. Уже начатая transaction может физически завершиться; её нужно урегулировать до safe confirmation. Safe procedure физического выхода выполняется Rust без необходимости успешного Lua/controller callback. Unknown delivery/readback остаётся unknown/fault.

State machine: Unverified, SafePending, Disarmed, ArmedManual, ArmedAuto, FaultLatched. Authority state отделён от evidence (unknown/acknowledged/readback verified) и его свежести. Start/resume/reconnect/reload не восстанавливают старые leases. Mode transfer проходит через safe transition; automatic rearm после fault/restart не предлагается.

## Consequences

Manual и automatic control имеют одинаковый final validation path. Runtime способен отозвать external/script authority и выполнить safe action независимо от живости этих producers. История и GUI показывают requested/authorized/sent/acknowledged/observed отдельно.

Потребуются per-output lifecycle, generation checks и тесты race между revoke и dispatch, включая ambiguous writes. Произвольные Lua serializers для actuator writes и maintenance raw access исключаются; неизвестный сложный протокол может потребовать Rust adapter. Trusted definitions должны правильно описывать hardware; sandbox не компенсирует неверный mapping.

Priority не прерывает in-flight serial transaction. Ordinary OS process не гарантирует hard-real-time или safe output при потере питания/порта. Для конкретного hardware заранее определяются safe action, evidence policy, timing budgets и независимая аппаратная защита. Multi-output software group не обещает атомарность нескольких hardware writes.

## Rejected alternatives

- Прямые writes из controller/GUI/script: несколько независимых owners и обход policy.
- Safety только в каждом driver: правила lease/interlock расходятся, новые adapters должны повторять всю систему.
- Проверять proposal только при постановке в очередь: отзыв lease не блокирует уже ожидающую устаревшую write.
- Считать timeout неисполненной командой и повторять: физический эффект может уже произойти.
- Считать последнее отправленное значение фактическим output: скрывает потерю подтверждения, отказ исполнительного устройства и поздние writes.
- Сохранять последнюю мощность навсегда при disconnect: переносит сбой producer на физический процесс без ограниченного срока полномочий.

Связанный historical rationale: [RUNTIME_AND_SAFETY_MODEL.md](RUNTIME_AND_SAFETY_MODEL.md).
