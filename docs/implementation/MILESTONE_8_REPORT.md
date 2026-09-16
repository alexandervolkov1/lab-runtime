# M8 implementation report

Status: implementation in progress under SOL_HIGH, 2026-09-16.

Design authority: [MILESTONE_8_DESIGN.md](MILESTONE_8_DESIGN.md). M7 was
externally accepted at `f3ff456`; the design-only checkpoint was committed as
`443fa4e`. This report records actual implementation evidence. Planned tests
are not reported as passing evidence.

## Acceptance implementation map

| IDs | Tests-first implementation slice | Evidence status |
| --- | --- | --- |
| C1-C4 | Strict bounded TOML loader, immutable artifact bundle, schema/cross-reference/safety validation and zero-side-effect rejection | Pending |
| C5-C8 | Staged diff, atomic owner apply, safe barrier, Required durability fence and no rearm | Pending |
| C9-C11 | Separate managed-source reload and virtual-model restart with generation fencing | Pending |
| C12-C15 | Bounded read-only Windows COM worker, M3 adapter semantics, disconnect/reconnect/rebind fencing | Pending |
| C16-C17 | Actual Windows COM + Metakon read-only acquisition and durable SQLite inspection | Hardware pending |
| C18 | Public API/Babashka process independence under configured acquisition | Pending |
| C19 | Finite COM + Recorder shutdown in software fault cases and actual hardware | Software pending; hardware pending |
| C20 | Fixed Recorder budgets and complete M1-M7/M8 regression gate | Pending |

## Actual sequence

1. Recovered clean accepted implementation HEAD `f3ff456`; `53880ca` is an
   ancestor. The only incoming changes were the expected uncommitted Astra design
   and coordination documents. `git diff --check` passed.
2. Read all required coordination, project, release-plan and M8-design files.
   The donor path `D:\rust\com_port_reader` is unavailable on this computer;
   no donor file was modified. Committed the documentation-only design checkpoint
   as `443fa4e`.

Red/green test names, commands, defects and resolved dependency versions will be
added after each logical slice.

## Current limitations

No M8 software or physical hardware acceptance has passed yet. In particular,
fake transports will not be counted as C16/C17 or the hardware portion of C19.
M8 makes no physical-output, power-loss, remote-security, GUI or long-soak claim.
