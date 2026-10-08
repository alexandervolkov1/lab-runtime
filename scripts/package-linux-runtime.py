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
import tarfile
import tempfile
import tomllib

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
    name = f"lab-runtime-{version}-linux-x86_64"
    dist = ROOT / "dist"
    dist.mkdir(exist_ok=True)
    artifacts = [dist / f"{name}.tar.gz", dist / f"{name}.licenses.tar.gz",
                 dist / f"{name}.build.json"]
    outputs = artifacts + [p.with_name(p.name + ".sha256") for p in artifacts[:2]]
    if any(p.exists() for p in outputs):
        parser.error("release outputs already exist; move them aside before rebuilding")
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
    timestamp = int(command("git", "show", "-s", "--format=%ct", "HEAD"))
    # Stage on the destination filesystem: WSL /tmp and a mounted Windows dist/
    # can be different devices, so a cross-device rename would fail.
    with tempfile.TemporaryDirectory(prefix=".lab-runtime-package-", dir=dist) as temporary:
        staging = Path(temporary)
        notices = staging / "THIRD-PARTY-NOTICES.json"
        notices.write_text(json.dumps({"target": TARGET, "dependencies": dependencies},
                                     indent=2) + "\n", encoding="utf-8")
        files.append((notices.name, notices, 0o644))
        executable_archive = staging / artifacts[0].name
        licenses_archive = staging / artifacts[1].name
        archive(executable_archive, [(f"{name}/lab-runtime", binary, 0o755)], timestamp)
        archive(licenses_archive, files, timestamp)
        provenance = {
            "package": name, "target": TARGET, "commit": command("git", "rev-parse", "HEAD"),
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
