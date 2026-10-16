# Origin

Source repository: https://github.com/alexandervolkov1/lab-runtime

Exact source commit: `7f91e715051f2b0d0ba1cde24a61d7135e15c0fc`.
This is the ordinary two-parent merge of [PR #53](https://github.com/alexandervolkov1/lab-runtime/pull/53),
whose verified HEAD was `3f40ba1925b13056f0f3643f3712af76aa18ce2c`.
Parents: `2426fc0823e1d6a5e28425cc8ebc2e5f079953cb` and that PR HEAD.
The merge tree equals the verified PR tree. PR #52's HEAD
`7e532202c5b2b405fc34a5132b262afe5243c557` is not an ancestor of the source;
PR #52 remains outside this import. Source branches, tags, releases and data remain
in the original repository. No old Git history is rewritten.

## Explicit import

[IMPORT-MANIFEST.json](IMPORT-MANIFEST.json) lists every one of the 262 selected
source files with its original SHA-256 and a concrete reason for inclusion.
The initial root commit is an exact selected-blob import; adaptation is in later
logical commits. `python scripts/verify_import.py` checks this independently of
the old checkout, including unchanged package sources/tests and lock provenance.

| Imported part | Necessity |
|---|---|
| `apps/lab-runtime`, `crates/lab-core` | Complete implementation, manifests, all Rust tests and test support |
| Six `examples` files | Two safe virtual deployments and SimpleDevice definitions/deployments used by Rust tests/tutorials |
| `test-data/fixtures/metakon`, reconnect JSON | Definition/configuration and reconnect regression inputs, not experimental measurements |
| Selected `docs` files | Active Runtime architecture, safety, API, configuration, driver, transport and Recorder contracts |
| Six selected `scripts` files | Packaging, document allowlist and exact-version license validation/regressions |
| Two evidence helpers, later moved to `scripts` | Repeatable bounded experiment and read-only SQLite verification; no old run data |
| `clients/babashka-smoke/manual-output.clj` | Optional external Application client, never an execution dependency |
| Selected `third-party-licenses/release` files | Serialport 4.10.1 original source/MPL and Rust 1.95.0 Windows/Linux standard-library evidence |
| Root Cargo, license and Git files | Build identity, pinned resolution, licensing and exact-byte handling |

## Standalone adaptations

- Workspace reduced from five to two packages; removed the egui build override.
  Cargo.lock reduced from 468 to 109 packages without adding/upgrading any
  name/version/source/checksum. Cargo also simplifies now-unambiguous dependency IDs.
- Production Rust, both package manifests, all Rust tests, wire protocol,
  configuration format and SQLite schema are unchanged from the source commit.
- Runtime-only README, AGENTS and operational documentation; client implementation
  instructions removed or linked to immutable source history.
- Windows packaging builds only Runtime; Linux retains its Runtime/license
  companion archives. Shared evidence checks exclude the absent Workbench `ring`
  dependency. Upstream license bytes are unchanged; authored notice/inventory is
  scoped to the two Runtime targets.
- External runner relocated and made portable; it accepts an explicit binary for
  extracted-package verification. The Clojure client itself remains byte-identical.
- New reproducible source-equivalence check, package smoke check and review report.

Excluded: Workbench, native GUI child, Windows Job crate, GUI probes and support
workspaces, memory studies, old plans and all historical experiment archives.
See [original engineering history](https://github.com/alexandervolkov1/lab-runtime/blob/7f91e715051f2b0d0ba1cde24a61d7135e15c0fc/docs/developer/project-status.md)
and [manual-output qualification](https://github.com/alexandervolkov1/lab-runtime/blob/7f91e715051f2b0d0ba1cde24a61d7135e15c0fc/docs/developer/runtime-manual-output-validation.md)
for prior evidence. Their task permissions and unfinished milestones are not
requirements of this project. New validation is recorded separately in
[extraction review](docs/extraction-review.md).
