"""Package only the native Windows Runtime, with exact license evidence."""
import argparse
import importlib.util
import json
from pathlib import Path
import platform
import re
import tempfile
import time
import zipfile

from package_user_documentation import user_files, validate_user_files
from release_license_evidence import check_extracted, destination, evidence, package_manifest

ROOT = Path(__file__).resolve().parent.parent
TARGET = 'x86_64-pc-windows-msvc'
spec = importlib.util.spec_from_file_location('shared_packager', ROOT/'scripts/package-linux-runtime.py')
shared = importlib.util.module_from_spec(spec)
spec.loader.exec_module(shared)
shared.TARGET = TARGET
command, sha256 = shared.command, shared.sha256


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--preview-version', required=True)
    args = parser.parse_args()
    if platform.system() != 'Windows' or platform.machine().lower() not in ('amd64', 'x86_64'):
        parser.error('Run on native x86_64 Windows with MSVC Rust')
    if not re.fullmatch(r'v0\.1\.0-preview\.[1-9][0-9]*', args.preview_version):
        parser.error('Preview identity must match Cargo package version 0.1.0')
    if command('git', 'status', '--porcelain'):
        parser.error('Commit the review inputs before packaging')
    rows = evidence(ROOT, 'windows')
    docs = user_files(ROOT)
    name = f'lab-runtime-{args.preview_version}-windows-x86_64'
    dist = ROOT/'dist'
    dist.mkdir(exist_ok=True)
    archive = dist/(name+'.zip')
    provenance_path = dist/(name+'.build.json')
    checksum = dist/(name+'.zip.sha256')
    if any(p.exists() for p in (archive, provenance_path, checksum)):
        parser.error('Outputs exist; preserve previous evidence before rebuilding')
    build = command('cargo', 'build', '-p', 'lab-runtime', '--release', '--target', TARGET,
                    '--locked', '--message-format=json')
    binaries = [Path(row['executable']) for line in build.splitlines()
                if (row := json.loads(line)).get('reason') == 'compiler-artifact'
                and row.get('executable') and row['target']['name'] == 'lab-runtime']
    if len(binaries) != 1 or binaries[0].read_bytes()[:2] != b'MZ':
        raise RuntimeError('Expected exactly one Windows Runtime executable')
    binary = binaries[0]
    metadata = json.loads(command('cargo', 'metadata', '--format-version', '1', '--locked',
                                  '--filter-platform', TARGET))
    license_files, dependencies = shared.license_inputs(metadata)
    files = {path: source for path, source, _ in license_files}
    for row in rows:
        path = destination(row['path'])
        if path in files and sha256(files[path]) != row['sha256']:
            raise RuntimeError(f'License evidence conflict: {path}')
        files[path] = ROOT/'third-party-licenses/release'/row['path']
    files.update({path: ROOT/path for path in docs})
    files['lab-runtime.exe'] = binary
    identity = {'package': name, 'target': TARGET, 'commit': command('git', 'rev-parse', 'HEAD'),
                'preview_version': args.preview_version, 'cargo_package_version': '0.1.0',
                'dirty_status': '', 'rustc': command('rustc', '-vV'),
                'cargo': command('cargo', '--version'), 'build_host': platform.platform(),
                'cargo_lock_sha256': sha256(ROOT/'Cargo.lock'), 'binary_sha256': sha256(binary)}
    timestamp = int(command('git', 'show', '-s', '--format=%ct', 'HEAD'))
    with tempfile.TemporaryDirectory(prefix='.runtime-package-', dir=dist) as tmp:
        stage = Path(tmp)
        for path, data in {
            'THIRD-PARTY-NOTICES.json': {'target': TARGET, 'dependencies': dependencies,
                                       'standard_library_evidence': 'LICENSE-EVIDENCE.json'},
            'LICENSE-EVIDENCE.json': package_manifest(rows, 'windows'),
            'BUILD.json': identity,
        }.items():
            source = stage/path
            source.write_text(json.dumps(data, indent=2)+'\n', encoding='utf-8')
            files[path] = source
        contents = stage/'PACKAGE-CONTENTS.txt'
        contents.write_text('\n'.join(sorted([*files, contents.name]))+'\n')
        files[contents.name] = contents
        with zipfile.ZipFile(archive, 'x', compression=zipfile.ZIP_DEFLATED) as zipped:
            for path, source in sorted(files.items()):
                info = zipfile.ZipInfo(name+'/'+path, time.gmtime(timestamp)[:6])
                info.compress_type = zipfile.ZIP_DEFLATED
                zipped.writestr(info, source.read_bytes())
        extracted = stage/'audit'
        with zipfile.ZipFile(archive) as zipped:
            if sorted(zipped.namelist()) != sorted(name+'/'+path for path in files):
                raise RuntimeError('Archive inventory mismatch')
            zipped.extractall(extracted)
        package = extracted/name
        for path, source in files.items():
            if sha256(package/path) != sha256(source):
                raise RuntimeError(f'Extracted bytes changed: {path}')
        check_extracted(package, rows, 'windows')
        validate_user_files(package, docs)
    identity['artifacts'] = {archive.name: sha256(archive)}
    provenance_path.write_text(json.dumps(identity, indent=2)+'\n', encoding='utf-8')
    checksum.write_text(f'{sha256(archive)}  {archive.name}\n', encoding='ascii')
    print(archive)
    print(provenance_path)


if __name__ == '__main__':
    main()
