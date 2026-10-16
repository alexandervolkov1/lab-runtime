# Runtime extraction review — 2026-10-16

Repository: https://github.com/alexandervolkov1/lab-runtime-next

The Runtime extraction and local candidate packages are ready for independent
review. No repository was renamed and no release or tag was published.
The application/Cargo package remains `lab-runtime`, version `0.1.0`.

## Source and change boundary

- Source: `alexandervolkov1/lab-runtime` at
  `7f91e715051f2b0d0ba1cde24a61d7135e15c0fc`, the ordinary merge commit for PR #53.
  Its two parents are main `2426fc0823e1d6a5e28425cc8ebc2e5f079953cb` and the
  expected PR HEAD `3f40ba1925b13056f0f3643f3712af76aa18ce2c`.
  The merge tree equals that PR HEAD. PR #52 remains open and excluded.
- Exact selective import: `b025058` (262 original files). All manifest hashes
  were also compared directly with the source repository's Git blobs.
- Two-package workspace: `0348b93`. The lockfile shrank from 468 to 109 packages;
  no package name/version/source/checksum was added or upgraded. Cargo simplified
  dependency IDs that became unambiguous. The egui build override was removed.
- Documentation and packaging adaptations are later commits. The 181 files under
  `apps/lab-runtime` and `crates/lab-core`, their package manifests, all Rust tests,
  six example files, regression fixtures and `manual-output.clj` remain unchanged.
  There is no production Rust, API, configuration-format or SQLite-schema change.

[ORIGIN.md](../ORIGIN.md) explains each imported part and adaptation.
[IMPORT-MANIFEST.json](../IMPORT-MANIFEST.json) is the explicit per-file inventory.
`python scripts/verify_import.py` checks source equivalence and dependency identity
without needing the original checkout. Old GUI, Windows Job, memory research,
plans and experimental archives remain in the original repository.

## Fresh-clone verification

Both clones came from the new GitHub repository, with their own empty target
directories. Windows used `D:/rust/lab-runtime-next-verify-20261016`; Linux used
`/tmp/lab-runtime-next-verify-20261016`, not the mounted original checkout.
Babashka was absent from PATH during ordinary gates. Rust 1.95.0 was used on
both platforms; no dependency/toolchain upgrade was performed.

| Check | Windows 10 19045, x86_64 MSVC | Arch Linux, x86_64 GNU, WSL2 |
|---|---|---|
| `cargo build --workspace --locked`, `RUSTFLAGS=-D warnings` | PASS | PASS |
| `cargo test --workspace --locked` | 673 passed, 2 ignored | 675 passed, 2 ignored |
| `cargo fmt --all -- --check` | PASS | PASS |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | PASS | PASS |
| License/document packaging regressions | 10 passed | 10 passed |
| Native locked release build and package extraction audit | PASS | PASS |
| Extracted binary: discovery, measurement, recording, shutdown, SQLite | PASS, database on C: local temporary storage | PASS, Linux filesystem |
| External manual output, debug | PASS on C: local temporary storage | PASS |
| External manual output, extracted release binary | PASS on C: local temporary storage | PASS |

The two ignored tests are existing opt-in soaks. Full optimized Rust test suites
were not rerun here; the release binaries were built and exercised externally.
The full Rust gates ran at `2133fa8d213093c3c29351b9e0df6051c469640b`.
The packaged source is `6c73cf20db5db6c8bbc444c15bac97487080f0d1`.
Rust/Cargo and compile-time API-document inputs are identical between these
commits; intervening changes concern helpers and operational documentation.
Later report/example-instruction commits do not invalidate those source gates.

## External experiment and storage limitation

The unchanged Babashka 1.13.220 client ran against the real native virtual plant
through TCP/NDJSON. Each successful run discovered/described instrument 1, read
temperature, explicitly acquired manual authority, delivered **10%, 20%, 15%**,
performed three explicit lease renewals, requested safe, and sealed recording.
The extracted release runs measured 20 → 22.438929 °C on Windows and
20 → 22.575068 °C on Linux. The different temperatures reflect native scheduling;
these are explicitly virtual observations, not physical Arduino measurements.

Each successful archive has exactly three accepted/completed `output_write`
operations and one Requested, SendStarted and ReadbackVerified fact for each
selected power. SQLite integrity is `ok`, run/boot are sealed, coverage is complete
and gaps are zero. The terminal Application shutdown result explicitly reports
`safe_confirmed=true`, `recorder_flushed=true`, completed cleanup and successful
exit. Durability was not inferred merely from a readable database or process exit.

**Windows D: storage remains an observed limitation.** Four initial experiments
using the new checkout's D: `target` directory failed: one Recorder startup
deadline and three Required Recorder ingress-capacity failures. The latter runs
revoked authority and reported virtual safe readback. Failures also occurred after
compilation ended; concurrent build load is not an established root cause.
The same debug/release binaries passed with Recorder files under
`%TEMP%/lab-runtime-next-review-20261016` on C:. No capacity, deadline, safety,
Recorder policy or production code was changed to obtain those passes.
The cause of the path/environment-dependent behavior is unresolved; successful
C: runs do not qualify the D: path. All four failed run directories, transcripts,
databases and any sidecars remain preserved alongside successful evidence.

The optional runner now accepts `--output-root` to select explicit local storage.
Babashka is absent from Rust/Cargo, protocol implementation, configuration, normal
startup and application packages. Linux's optional binary was downloaded separately
at version 1.13.220 and verified against the upstream asset SHA-256
`eb3edd128276f0b6fbdefcb18dc7d42652a95ea409a81b34f08e36b8ac3cbc3c`.

## Local review artifacts

Prepared in `D:/rust/lab-runtime-next/dist/`; these are local unpublished
candidates, not downloadable GitHub releases. The `preview.6` label is only the
package identifier used for this review, not a release approval.

| Artifact | SHA-256 |
|---|---|
| `lab-runtime-v0.1.0-preview.6-windows-x86_64.zip` | `bc00f3d1379a74f661f8d395ac04c5994304a1cd46392d3b2a2aaf25c25a8015` |
| `lab-runtime-v0.1.0-preview.6-linux-x86_64.tar.gz` | `8ea74482f8de6fa44f30f8a69e30adc92f7327dcf516cd7c3d909377d26ede36` |
| `lab-runtime-v0.1.0-preview.6-linux-x86_64.licenses.tar.gz` | `a2c73ee5b0adaa11871969a17c32a55e3681d4c93edff8eb19ff537977820ee5` |
| `extraction-evidence-6c73cf2.zip` | `0f740bb0ab4e3a883a47b0497357258d54d2bac4e905e2982ac22233d002f153` |

The evidence ZIP contains 134 newly generated evidence files plus a per-entry
hash inventory: gate logs, source equivalence, transcripts, successful/failed
SQLite archives and closure evidence. It contains no historical donor run.
It was verified after archiving. `SHA256SUMS.txt`, individual archive checksums
and each platform's `.build.json` accompany the candidates.
[Machine-readable results](extraction-validation.json) retain identities, outcomes,
log hashes and build provenance for remote source review.

Windows requires the Microsoft Visual C++ 2015-2022 x64 runtime. Linux's measured
glibc symbol floor is 2.34, tested here on glibc 2.44 under WSL2; this is not a
native bare-metal or universal-distribution qualification. Linux's matching
license companion must accompany its binary. No Babashka client is in any package.

No physical Arduino, real heater effect, two-physical-host deployment, power-loss,
multi-day or universal storage qualification was performed. Repository renaming
and release publication remain the user's separate next step after review.
