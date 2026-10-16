#!/usr/bin/env python3
"""Build the headless GNU/Linux x86_64 Runtime and a minimal release archive.

Run on Linux with Python 3.11+, Rust, a C toolchain and binutils. License texts
travel in a separate companion archive, keeping the executable archive minimal.
No Workbench build, installation, publication or service registration occurs.
"""

import argparse
import gzip
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import subprocess
import sys
import tarfile
import tempfile
import tomllib

sys.dont_write_bytecode = True
from release_license_evidence import check_extracted, destination as license_destination, evidence, package_manifest
from package_user_documentation import user_files, validate_user_files

ROOT = Path(__file__).resolve().parent.parent
TARGET = "x86_64-unknown-linux-gnu"


def command(*args):
    return subprocess.check_output(args, cwd=ROOT, text=True).strip()


def sha256(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def archive(destination, files, timestamp):
    """Write only the explicit allowlist with portable, deterministic metadata."""
    with destination.open("xb") as raw:
        with gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=0) as zipped:
            with tarfile.open(fileobj=zipped, mode="w", format=tarfile.PAX_FORMAT) as tar:
                for name, source, mode in sorted(files):
                    info = tar.gettarinfo(str(source), arcname=name)
                    info.uid = info.gid = 0
                    info.uname = info.gname = ""
                    info.mode = mode
                    info.mtime = timestamp
                    with source.open("rb") as stream:
                        tar.addfile(info, stream)


def license_inputs(metadata):
    """Inventory the Runtime's target-specific normal dependency graph only."""
    tree = command(
        "cargo", "tree", "-p", "lab-runtime", "--target", TARGET,
        "--edges", "normal", "--prefix", "none", "--format", "{p}", "--locked",
    )
    identities = set()
    for line in tree.splitlines():
        match = re.match(r"^([A-Za-z0-9_-]+) v([^ ]+)(?: |$)", line)
        if not match:
            raise RuntimeError(f"unrecognized dependency: {line}")
        identities.add(match.groups())
    files = [("LICENSE", ROOT / "LICENSE", 0o644)]
    inventory = []
    for name, version in sorted(identities):
        matches = [p for p in metadata["packages"]
                   if (p["name"], p["version"]) == (name, version)]
        if len(matches) != 1:
            raise RuntimeError(f"ambiguous Cargo package: {name} {version}")
        package = matches[0]
        if package["source"] is None:
            continue
        directory = Path(package["manifest_path"]).parent
        texts = {p for p in directory.iterdir() if p.is_file() and re.match(
            r"^(LICENSE|LICENCE|COPYING|NOTICE|UNLICENSE)([.-]|$)", p.name, re.I)}
        if package.get("license_file"):
            texts.add(directory / package["license_file"])
        if not texts or not package.get("license"):
            raise RuntimeError(f"license evidence missing: {name} {version}")
        evidence = []
        for source in sorted(texts):
            relative = source.relative_to(directory).as_posix()
            destination = f"licenses/{name}-{version}/{relative}"
            files.append((destination, source, 0o644))
            evidence.append({"path": destination, "sha256": sha256(source)})
        inventory.append({"name": name, "version": version, "source": package["source"],
                          "license": package["license"], "evidence": evidence})
    return files, inventory


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--allow-dirty", action="store_true",
                        help="record an uncommitted review build explicitly")
    parser.add_argument("--preview-version", required=True,
                        help="explicit release identity, for example v0.1.0-preview.5")
    args = parser.parse_args()
    if platform.system() != "Linux" or platform.machine() != "x86_64":
        parser.error("run this packager on Linux x86_64 (native, VM or WSL2)")
    toolchain = command("rustc", "-vV")
    if f"host: {TARGET}" not in toolchain:
        parser.error(f"use a native {TARGET} Rust toolchain")
    status = command("git", "status", "--porcelain")
    if status and not args.allow_dirty:
        parser.error("worktree is dirty; commit first or use --allow-dirty for review")
    changed_paths = subprocess.check_output(
        ["git", "ls-files", "-z", "--modified", "--others", "--exclude-standard"], cwd=ROOT,
    ).decode("utf-8").split("\0")
    source_changes = [
        {"path": path, "sha256": sha256(ROOT / path) if (ROOT / path).is_file() else None}
        for path in sorted(set(changed_paths) - {""})
    ]
    version = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["package"]["version"]
    if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+(?:-[A-Za-z0-9.-]+)?", version):
        parser.error("unsafe package version")
    if not re.fullmatch(r'v[0-9]+\.[0-9]+\.[0-9]+-preview\.[1-9][0-9]*', args.preview_version) or \
            args.preview_version[1:].split('-')[0] != version:
        parser.error('preview version must match the Cargo package base version')
    name = f"lab-runtime-{args.preview_version}-linux-x86_64"
    documentation = user_files(ROOT)
    dist = ROOT / "dist"
    dist.mkdir(exist_ok=True)
    artifacts = [dist / f"{name}.tar.gz", dist / f"{name}.licenses.tar.gz",
                 dist / f"{name}.build.json"]
    outputs = artifacts + [p.with_name(p.name + ".sha256") for p in artifacts[:2]]
    if any(p.exists() for p in outputs):
        parser.error("release outputs already exist; move them aside before rebuilding")
    release_licenses = evidence(ROOT, "linux")
    build = command(
        "cargo", "build", "-p", "lab-runtime", "--release", "--target", TARGET,
        "--locked", "--message-format=json",
    )
    executables = [Path(row["executable"]) for line in build.splitlines()
                   if (row := json.loads(line)).get("reason") == "compiler-artifact"
                   and row.get("executable") and row["target"]["name"] == "lab-runtime"]
    if len(executables) != 1:
        raise RuntimeError("Cargo did not report exactly one Runtime executable")
    binary = executables[0]
    with binary.open("rb") as stream:
        header = stream.read(20)
    if header[:6] != b"\x7fELF\x02\x01" or header[18:20] != b"\x3e\x00":
        raise RuntimeError("expected a little-endian ELF64 x86_64 executable")
    linked = command("ldd", str(binary))
    if "not found" in linked:
        raise RuntimeError(f"unresolved shared libraries:\n{linked}")
    symbols = command("readelf", "--version-info", str(binary))
    glibc = sorted(set(re.findall(r"GLIBC_([0-9.]+)", symbols)),
                   key=lambda v: tuple(int(part) for part in v.split(".")))
    metadata = json.loads(command(
        "cargo", "metadata", "--format-version", "1", "--locked", "--filter-platform", TARGET,
    ))
    files, dependencies = license_inputs(metadata)
    by_name = {name: (name, path, mode) for name, path, mode in files}
    for row in release_licenses:
        evidence_name = license_destination(row["path"])
        path = ROOT / "third-party-licenses/release" / row["path"]
        if evidence_name in by_name and sha256(by_name[evidence_name][1]) != row["sha256"]:
            raise RuntimeError(f"supplemental license evidence conflicts: {evidence_name}")
        by_name[evidence_name] = (evidence_name, path, 0o644)
    files = list(by_name.values())
    timestamp = int(command("git", "show", "-s", "--format=%ct", "HEAD"))
    # Stage on the destination filesystem: WSL /tmp and a mounted Windows dist/
    # can be different devices, so a cross-device rename would fail.
    with tempfile.TemporaryDirectory(prefix=".lab-runtime-package-", dir=dist) as temporary:
        staging = Path(temporary)
        notices = staging / "THIRD-PARTY-NOTICES.json"
        notices.write_text(json.dumps({"target": TARGET, "dependencies": dependencies,
                                      "standard_library_evidence": "LICENSE-EVIDENCE.json",
                                      "source_availability": "NOTICE.txt"},
                                     indent=2) + "\n", encoding="utf-8")
        files.append((notices.name, notices, 0o644))
        evidence_path = staging / "LICENSE-EVIDENCE.json"
        evidence_path.write_text(json.dumps(package_manifest(release_licenses, "linux"), indent=2) + "\n")
        files.append((evidence_path.name, evidence_path, 0o644))
        contents = staging / "PACKAGE-CONTENTS.txt"
        contents.write_text("\n".join(sorted([name for name, _, _ in files] + [contents.name])) + "\n")
        files.append((contents.name, contents, 0o644))
        executable_archive = staging / artifacts[0].name
        licenses_archive = staging / artifacts[1].name
        runtime_files = [(f"{name}/lab-runtime", binary, 0o755),
                         (f"{name}/NOTICE.txt", ROOT / "third-party-licenses/release/NOTICE.txt", 0o644)]
        runtime_files.extend((f'{name}/{path}', ROOT / path, 0o644) for path in documentation)
        runtime_contents = staging / 'RUNTIME-PACKAGE-CONTENTS.txt'
        runtime_contents.write_text('\n'.join(sorted(
            [path for path, _, _ in runtime_files] + [f'{name}/{runtime_contents.name}'])) + '\n')
        runtime_files.append((f'{name}/{runtime_contents.name}', runtime_contents, 0o644))
        archive(executable_archive, runtime_files, timestamp)
        runtime_extract = staging / 'runtime-audit'
        with tarfile.open(executable_archive, 'r:gz') as tar:
            if sorted(tar.getnames()) != sorted(path for path, _, _ in runtime_files) or \
                    not all(m.isfile() for m in tar.getmembers()):
                raise RuntimeError('Runtime archive inventory mismatch')
            tar.extractall(runtime_extract, filter='data')
        for path, source, _ in runtime_files:
            if sha256(runtime_extract / path) != sha256(source):
                raise RuntimeError(f'Extracted Runtime package file changed: {path}')
        validate_user_files(runtime_extract / name, documentation)
        archive(licenses_archive, files, timestamp)
        # Validate the actual archive bytes and offline source after extraction.
        extracted = staging / "license-audit"
        with tarfile.open(licenses_archive, "r:gz") as tar:
            expected = sorted(name for name, _, _ in files)
            if sorted(tar.getnames()) != expected or not all(m.isfile() for m in tar.getmembers()):
                raise RuntimeError("license archive inventory mismatch")
            tar.extractall(extracted, filter="data")
        if (extracted / contents.name).read_text().splitlines() != expected:
            raise RuntimeError("extracted license package manifest mismatch")
        check_extracted(extracted, release_licenses, "linux")
        for file_name, path, _ in files:
            if sha256(extracted / file_name) != sha256(path):
                raise RuntimeError(f"extracted license file changed: {file_name}")
        provenance = {
            "package": name, "target": TARGET, "commit": command("git", "rev-parse", "HEAD"),
            "preview_version": args.preview_version, "cargo_package_version": version,
            "dirty_status": status, "source_changes": source_changes,
            "build_host": {"system": platform.platform(), "machine": platform.machine()},
            "rustc": toolchain, "cargo": command("cargo", "--version"),
            "cargo_lock_sha256": sha256(ROOT / "Cargo.lock"),
            "binary_sha256": sha256(binary), "ldd": linked,
            "required_glibc_symbol_version": glibc[-1] if glibc else None,
            "artifacts": {p.name: sha256(p) for p in [executable_archive, licenses_archive]},
        }
        for source, destination in zip([executable_archive, licenses_archive], artifacts):
            os.replace(source, destination)
        artifacts[2].write_text(json.dumps(provenance, indent=2) + "\n", encoding="utf-8")
    for artifact in artifacts[:2]:
        artifact.with_name(artifact.name + ".sha256").write_text(
            f"{sha256(artifact)}  {artifact.name}\n", encoding="ascii")
    print("\n".join(str(p) for p in outputs))


if __name__ == "__main__":
    main()
