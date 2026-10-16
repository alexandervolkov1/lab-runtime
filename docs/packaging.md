# Runtime packaging

Prepare candidate archives from a clean committed checkout with Rust 1.95.0,
Python 3.11+ and a native C toolchain. Linux also needs binutils. These commands
build one application and do not publish a release, tag or install a service.
Choose an unused candidate label matching Cargo version 0.1.0:

```powershell
python scripts/test_release_license_evidence.py
python scripts/package-windows-runtime.py --preview-version v0.1.0-preview.7
```

```sh
python3 scripts/test_release_license_evidence.py
python3 scripts/package-linux-runtime.py --preview-version v0.1.0-preview.7
```

Outputs appear in `dist`; existing artifacts are never overwritten. Windows
produces one ZIP with the executable, documentation, two virtual configurations,
notices and licenses/source. Linux produces a Runtime tarball and a required
matching license companion. SHA-256 files and build provenance identify source,
lockfile, compiler and executable. Windows requires the Microsoft VC++ x64
runtime; Linux's build record states the measured glibc requirement.

Each packager validates the extracted bytes, offline documentation links and
license evidence. To check execution, extract into a fresh directory and follow
the [user guide](README.md). Run the optional single external client separately
against that executable for TCP and WS. Check Recorder completion through the
API and inspect a closed SQLite copy; report each operating system independently.
Real WSS/tuna and physical hardware require separate available access.

## License scope

The explicit `scripts/user-package-files.json` allowlist controls documentation
and examples. Client scripts and development tools are not execution dependencies
or application-package contents.

Cargo inventories include only Runtime's target-specific normal dependency graph.
The lockfile pins versions/checksums. Missing or ambiguous license evidence fails
packaging. Supplemental evidence retains unmodified serialport 4.10.1 source,
MPL text and member hashes, plus Rust 1.95.0 standard-library texts and rlib hashes.
Upstream evidence bytes must remain unchanged. Review new dependencies/toolchains
before distribution; preserve all accompanying notices and Linux license archives.
