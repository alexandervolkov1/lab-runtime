# M13 Steel dependency safety resolution

## Status and decision

Audit date: 2026-09-28.

This is a read-only dependency investigation against repository coordination commit
`115adaaab63ea375d0a2541f47d363265c0f683e`. It changes no Cargo manifest, lockfile,
Rust source, test, Application contract or Runtime behavior.

No currently reviewable upstream Steel release or commit removes the dependency
safety blocker:

- released `steel-core 0.8.3` still requires `im-rc 15.1.0`, which reaches
  `sized-chunks 0.6.5` and RUSTSEC-2026-0255;
- current Steel `master` at
  `9774aa06f74329364b84b0a5d104d75b483a571a` has the same mandatory dependency and
  the same advisory path under the required minimal `std` feature set;
- Steel's optional `steel-imbl 7.1.0` path does not remove the mandatory old graph
  and independently reaches RUSTSEC-2026-0292 and RUSTSEC-2025-0167.

M13.2 must remain blocked. The smallest responsible direction is to wait for an
upstream Steel revision and release that actually migrates the core collection
implementation to a reviewable graph, then repeat this audit. Pinning current
upstream `master` does not help, and a lab-runtime-owned fork is not recommended.

## 1. Audit method and environment

The audit used fresh repositories and Cargo probes outside the lab-runtime
workspace under:

```text
C:\Users\user\AppData\Local\Temp\m13-steel-resolution-fac54614a84840f092aaeeda3cc28ad8
```

Platform:

```text
Microsoft Windows 10 Pro 10.0.19045 build 19045
AMD64
rustc 1.95.0 (59807616e 2026-04-14)
cargo 1.95.0 (f2d3ce0bd 2026-03-21)
```

Each investigated graph used an isolated executable probe. Commands included:

```text
cargo metadata --format-version 1
cargo tree
cargo tree -e features
cargo check
```

The advisory pass submitted every resolved non-probe package name/version from Cargo
metadata to the OSV crates.io batch API, including the git-sourced Steel package
names, and inspected all returned RustSec records. The probe Cargo locks and build
products remain outside the repository.

Primary upstream/advisory sources:

- <https://github.com/mattwparas/steel/tree/9774aa06f74329364b84b0a5d104d75b483a571a>
- <https://github.com/mattwparas/steel/blob/9774aa06f74329364b84b0a5d104d75b483a571a/crates/steel-core/Cargo.toml>
- <https://github.com/mattwparas/steel/blob/9774aa06f74329364b84b0a5d104d75b483a571a/Cargo.lock>
- <https://github.com/mattwparas/steel-imbl/tree/32c4170d6694efda08185b2849122360808ec066>
- <https://crates.io/crates/steel-core/0.8.3>
- <https://crates.io/crates/steel-imbl/7.1.0>
- <https://rustsec.org/advisories/RUSTSEC-2026-0255.html>
- <https://rustsec.org/advisories/RUSTSEC-2026-0250.html>
- <https://rustsec.org/advisories/RUSTSEC-2026-0251.html>
- <https://rustsec.org/advisories/RUSTSEC-2026-0247.html>
- <https://rustsec.org/advisories/RUSTSEC-2025-0141.html>
- <https://rustsec.org/advisories/RUSTSEC-2026-0292.html>
- <https://rustsec.org/advisories/RUSTSEC-2025-0167.html>
- <https://rustsec.org/advisories/RUSTSEC-2023-0126.html>
- <https://osv.dev/>

## 2. Current upstream Steel master

At audit time, the exact upstream HEAD was:

```text
repository: https://github.com/mattwparas/steel
branch:     master
commit:     9774aa06f74329364b84b0a5d104d75b483a571a
date:       2026-09-27T22:44:02-07:00
subject:    LSP tests (#705)
```

The repository is active and not archived. The latest crates.io release remains
`steel-core 0.8.3`, published 2026-08-20 from release commit
`11a57adcf42c22733463c59b1f44ec49eb7043bc`. Active project maintenance does not,
however, remove the current graph defect.

### Manifest and source result

`crates/steel-core/Cargo.toml` at the pinned master commit contains:

```toml
im-rc = { version = "15.1.0", features = ["serde"] }
steel-imbl = { version = "7.1", optional = true, features = ["serde"] }

[features]
default = ["std", "modules"]
std = ["dep:chrono"]
sync = ["dep:im"]
imbl = ["dep:steel-imbl"]
```

`im-rc` is unconditional, not optional. The default non-`sync` value aliases in
`crates/steel-core/src/values/mod.rs` use `im_rc::{Vector,HashMap,HashSet}`. The
`steel-imbl` aliases are selected only by the combined `sync` plus `imbl` feature
configuration. Enabling that alternative does not remove the mandatory `im-rc`
package from the graph.

The source workspace lock contains both the old collection chain and optional
`steel-imbl`, but the isolated downstream probe is the authoritative M13 candidate
graph. It used exactly:

```toml
steel-core = {
    git = "https://github.com/mattwparas/steel.git",
    rev = "9774aa06f74329364b84b0a5d104d75b483a571a",
    default-features = false,
    features = ["std"]
}
```

### Exact minimal dependency path

Fresh Cargo metadata resolved 158 package identities including the probe and the
git-sourced Steel packages. The required unsafe path is:

```text
steel-core 0.8.3
  -> im-rc 15.1.0
      -> sized-chunks 0.6.5
          -> bitmaps 2.1.0
```

`bincode 1.3.3` is also a direct Steel dependency. `cargo tree -e features` proved
that this result used only `steel-core/std`; default features, `sync`, `imbl`, JIT,
dylibs, FFI, git, HTTP/ureq and experimental features were not enabled.

The exact pinned git graph passed `cargo check` on the Windows toolchain above. A
successful build is compatibility evidence only; it does not disposition the
advisories.

### Fresh advisory result

The minimal current-master graph returned exactly these five RustSec findings:

| Package | Advisory | Classification | Disposition |
| --- | --- | --- | --- |
| `sized-chunks 0.6.5` | RUSTSEC-2026-0255 | Safe-Rust panic-safety use-after-free/double-free; no fixed version | Blocking |
| `im-rc 15.1.0` | RUSTSEC-2026-0250 | Unmaintained | Confirms abandoned direct dependency |
| `sized-chunks 0.6.5` | RUSTSEC-2026-0251 | Unmaintained | Confirms no upstream fix line |
| `bitmaps 2.1.0` | RUSTSEC-2026-0247 | Unmaintained | Maintenance debt |
| `bincode 1.3.3` | RUSTSEC-2025-0141 | Unmaintained | Maintenance debt |

The explicitly required advisory IDs therefore have the same disposition as the
released 0.8.3 graph: all five are present. Current Steel master is not a safe pin.

## 3. `mattwparas/steel-imbl`

### Provenance and exact revision

Steel does not reference a `steel-imbl` git commit. Its manifest requests the
crates.io range `7.1`, and its workspace lock selects `steel-imbl 7.1.0` from the
registry. The published crate's `.cargo_vcs_info.json` identifies this exact source
commit:

```text
repository:  https://github.com/mattwparas/steel-imbl
branch:      main
commit:      32c4170d6694efda08185b2849122360808ec066
date:        2026-01-25T19:48:40-08:00
crate:       steel-imbl 7.1.0
published:   2026-01-26
```

That commit remains the current repository HEAD. The repository is owned by the
Steel maintainer, is not archived, and has no tags or open issues/PRs. It has had no
source update since January 2026. Its README says it is a fork of `imbl`, itself a
fork of `im`, specifically for use within Steel.

The manifest declares `MPL-2.0+`; the repository includes the Mozilla Public License
2.0 text. It declares Rust 1.84 and edition 2018.

### Exact graph

Both the crates.io `7.1.0` probe and the exact git-commit probe resolved and built
the same graph on Windows:

```text
steel-imbl 7.1.0
  -> bitmaps 3.2.1
  -> imbl-sized-chunks 0.1.3
       -> bitmaps 3.2.1
  -> rand_core 0.9.5
  -> rand_xoshiro 0.7.0
  -> serde_core 1.0.229
  -> wide 0.7.33
```

The graph does not contain packages named `im-rc` or `sized-chunks`. Therefore the
package-specific RUSTSEC-2026-0255, RUSTSEC-2026-0250 and RUSTSEC-2026-0251 records
are not reachable from standalone `steel-imbl`.

That absence does not make this graph safe. Fresh advisory review found:

| Package | Advisory | Classification | Disposition |
| --- | --- | --- | --- |
| `imbl-sized-chunks 0.1.3` | RUSTSEC-2026-0292 | Safe-Rust panic-safety double-free/use-after-free | Blocking; fixed only in `>=0.2.0` |
| `bitmaps 3.2.1` | RUSTSEC-2025-0167 | Safe API can create invalid `bool`, immediate undefined behavior | Blocking; no fixed version |
| `bitmaps 3.2.1` | RUSTSEC-2026-0247 | Unmaintained | Confirms no maintained fix line |

`imbl-sized-chunks 0.2.0` was released on 2026-09-09 and fixes
RUSTSEC-2026-0292, but `steel-imbl` pins the incompatible semver range `0.1.3` and
cannot select 0.2.0 without a manifest/source update. `bitmaps 3.2.1` has no fixed
release and its repository is archived.

### Unsafe-code surface

Lexical source inspection found 26 `unsafe` matches across five `steel-imbl` source
files:

```text
src/shared_ptr.rs
src/nodes/hamt.rs
src/nodes/btree.rs
src/vector/focus.rs
src/vector/mod.rs
```

The resolved `imbl-sized-chunks 0.1.3` source has a much larger unsafe
implementation surface (158 lexical matches in the local registry source). Counts
are inventory, not proof that each use is defective. The two RustSec memory-safety
findings above are the authoritative blockers.

### Can Steel use it as a safe replacement now?

No. An audit-only Steel master probe with `std`, `sync` and `imbl` enabled proved
that the resulting graph contains both collection families:

```text
steel-core
  -> im-rc -> sized-chunks                 # still mandatory
  -> im -> sized-chunks                    # added by sync
  -> steel-imbl -> imbl-sized-chunks       # added by imbl
```

That graph built, but its advisory set expanded to include:

```text
RUSTSEC-2026-0255
RUSTSEC-2026-0292
RUSTSEC-2025-0167
RUSTSEC-2023-0126   (safe-Rust aliasing UB in im)
```

plus the related unmaintained-package notices. Enabling `sync`/`imbl` is therefore
not a mitigation and is outside the accepted minimal M13 feature target.

## 4. Direction comparison

### Direction A — wait for a new crates.io Steel release

This remains the preferred eventual direction because it preserves registry-based
reproducibility and upstream maintenance ownership. It is not a safe candidate
today:

- latest release `0.8.3` has the original five findings;
- current master still has the same mandatory chain;
- no merged upstream dependency migration exists to release;
- no open Steel or steel-imbl issue/PR found in the audit advertises a pending
  migration for these advisories.

Re-evaluate only after upstream removes mandatory `im-rc`/`sized-chunks` and the
replacement collection graph itself passes a fresh advisory and unsafe-code review.
An assumed future release is not an implementation candidate.

### Direction B — pin an exact upstream Steel git commit

Rejected for the current HEAD. Exact commit
`9774aa06f74329364b84b0a5d104d75b483a571a`:

- is upstream-owned and active;
- builds on supported Windows;
- retains Steel's MIT OR Apache-2.0 license;
- but resolves the same blocking `im-rc -> sized-chunks` path as 0.8.3.

A git pin would add reproducibility/update-policy costs without improving safety.
There is no alternate upstream branch exposed by the repositories that contains a
reviewable migration.

### Direction C — project-owned patch or fork

Evaluated and rejected before patch creation. This would not be a one-line Cargo
override. A credible fork would need, at minimum:

1. make `im-rc` optional or remove it from `steel-core`;
2. migrate every non-`sync` value/conversion/primitive use to a vetted collection;
3. avoid the `im` path because it has RUSTSEC-2023-0126;
4. if based on `steel-imbl`, upgrade/test `imbl-sized-chunks >=0.2.0` and replace or
   repair the unfixed `bitmaps >=3.2.0` dependency;
5. add panic/drop/Miri-style regression coverage for the affected collection paths;
6. carry MPL-covered collection modifications, provenance and license inventory;
7. continuously rebase, re-audit and retire the fork when upstream resolves it.

Because current Steel and current steel-imbl both need substantive upstream source
changes, lab-runtime would become the owner of a language VM collection-safety fork.
That burden is disproportionate to M13 and contrary to the goal of finding an
upstream-owned candidate. No patch/fork was created.

## 5. Required advisory disposition summary

| Advisory | Released 0.8.3 minimal `std` | Master `9774aa...` minimal `std` | Standalone steel-imbl 7.1.0 | Disposition |
| --- | --- | --- | --- | --- |
| RUSTSEC-2026-0255 | Present | Present | Package absent | Steel blocker remains |
| RUSTSEC-2026-0250 | Present | Present | Package absent | Steel uses unmaintained `im-rc` |
| RUSTSEC-2026-0251 | Present | Present | Package absent | Steel uses unmaintained `sized-chunks` |
| RUSTSEC-2026-0247 | Present via bitmaps 2.1.0 | Present via bitmaps 2.1.0 | Present via bitmaps 3.2.1 | Present in both directions |
| RUSTSEC-2025-0141 | Present | Present | Package absent | Direct Steel maintenance debt |

Additional blockers exposed by investigating `steel-imbl`:

| Advisory | Package | Result |
| --- | --- | --- |
| RUSTSEC-2026-0292 | `imbl-sized-chunks 0.1.3` | Present; memory safety; fixed in 0.2.0 but current steel-imbl cannot select it |
| RUSTSEC-2025-0167 | `bitmaps 3.2.1` | Present; memory safety; no fixed version |

Unmaintained INFO findings are recorded separately from memory-unsoundness. They do
not independently carry the same severity, but here they confirm that the affected
collection lines have no maintained fix path.

## 6. Feature decision

The accepted eventual host target remains:

```toml
default-features = false
features = ["std"]
```

Current master still compiles with that feature set, so no feature expansion is
justified. The `sync`/`imbl` combination was enabled only in an isolated audit probe
to test the proposed replacement and made the graph worse. M13 must not enable
dylibs, JIT, FFI/plugin machinery, git, HTTP/ureq, unsafe-internals or experimental
features.

## 7. Re-entry criteria

Before M13.2 can be reconsidered, a new read-only resolution should prove all of:

1. an exact upstream-owned Steel release or commit no longer resolves `im-rc`,
   `sized-chunks`, or affected `bitmaps` versions in the minimal `std` graph;
2. any `steel-imbl`/replacement graph no longer resolves
   `imbl-sized-chunks <0.2.0` or `bitmaps >=3.2.0` without a disposition accepted by
   external review;
3. fresh `cargo metadata`, normal tree and feature tree document the exact graph;
4. locked Windows `cargo check` passes on the supported toolchain;
5. full OSV/RustSec review records both vulnerabilities and unmaintained notices;
6. licenses, unsafe surface, source provenance and update ownership are accepted;
7. no lab-runtime-owned patch/fork is introduced without a separately authorized
   fork design and maintenance commitment.

Until then, the accepted external Steel architecture remains valid but cannot be
implemented with the currently available dependency candidates.

## 8. Repository scope verification

The investigation did not modify:

```text
Cargo.toml
Cargo.lock
Rust production source
tests
Application operations or DTOs
SessionStore or deduplication
Runtime, Recorder, SQLite or OutputAuthority
TCP or WebSocket transports
managed components
presentation semantics
```

Temporary clones, Cargo probes, generated locks and build outputs stayed outside the
repository. M13.2 and M14 remain unauthorized.

```text
B. NO ACCEPTABLE CANDIDATE

M13.2 remains blocked
```

```text
STATUS: M13_DEPENDENCY_RESOLUTION_READY_FOR_EXTERNAL_REVIEW
```

## External review acceptance

External review accepted this dependency-safety resolution at commit
`7a150cd8e15d990e00ad62b5c9d9b66c401b7086`. M13.2 remains blocked and
unauthorized. M14.1 Workbench architecture audit is authorized; Steel remains a
deferred optional Workbench scripting candidate.
