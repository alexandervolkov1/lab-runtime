# Runtime packaging

These commands prepare local candidate artifacts. They never tag, publish a
GitHub release, install a service or rename a repository. Candidate version labels
are packaging identities; Cargo remains `lab-runtime` version `0.1.0`.

Use a clean committed checkout, Rust 1.95.0 with the exact reviewed standard
library, Python 3.11+ and a native C toolchain. Linux additionally uses binutils.
No GUI or external verification client is built or distributed. Windows targets
the MSVC runtime and requires the Microsoft Visual C++ 2015-2022 x64 runtime
(`VCRUNTIME140.dll`) on the destination machine; it is not bundled.

```powershell
python scripts/test_release_license_evidence.py
powershell -File scripts/package-developer-preview.ps1 -PreviewVersion v0.1.0-preview.6
```

```sh
python3 scripts/test_release_license_evidence.py
python3 scripts/package-linux-runtime.py --preview-version v0.1.0-preview.6
```

Windows produces one ZIP containing Runtime, docs, two virtual configurations,
application notices and supplemental licenses/source. Linux produces the Runtime
tarball and its required matching license companion. Both have SHA-256 files and
build provenance identifying source, lockfile, compiler and binary. Existing
outputs are never silently overwritten. Each packager verifies actual extracted
bytes, offline links and license evidence; run the extracted binary separately:

```sh
python scripts/smoke_runtime.py ABSOLUTE_EXECUTABLE_PATH NEW_EVIDENCE_DIRECTORY
```

This smoke uses Python's standard library, an ephemeral loopback virtual Runtime,
Application discovery/measurement/recording, graceful shutdown and read-only
SQLite checks. For the optional complete manual-output experiment use
`scripts/run_manual_output.py release --binary ABSOLUTE_EXECUTABLE_PATH` with an
explicitly installed Babashka client. Neither checker is an execution dependency
of the packaged application.

## License scope

The explicit `scripts/user-package-files.json` allowlist controls docs/examples.
Application license inventories use only the native Runtime normal dependency
graph. Exact versions/checksums are retained from the original lockfile. Registry
license files are gathered from Cargo metadata; missing/ambiguous evidence fails
packaging. Supplemental evidence includes serialport 4.10.1's unchanged `.crate`,
MPL text and member hashes, plus Rust 1.95.0's standard-library inventory and
target rlib hashes. The old Workbench `ring` and graphics licenses are excluded
because those dependencies are absent. Original upstream notice bytes remain
unchanged. The project-authored NOTICE/evidence manifest was scoped to Runtime.

The gate rejects changed toolchains, locked serialport version/checksum, missing
required evidence and corrupted bytes. Preserve exact-version license evidence
and review any new dependency/toolchain before distributing it.
