"""Fail-closed, offline validation of pinned binary-distribution license evidence."""
import hashlib
import json
from pathlib import Path, PurePosixPath
import subprocess
import tomllib

RUST_RELEASE = '1.95.0'
RUST_COMMIT = '59807616e1fa2540724bfbac14d7976d7e4a3860'
RING_FILES = ('LICENSE', 'LICENSE-BoringSSL', 'LICENSE-other-bits',
              'src/polyfill/once_cell/LICENSE-APACHE', 'src/polyfill/once_cell/LICENSE-MIT',
              'third_party/fiat/LICENSE', 'third_party/fiat/AUTHORS')
CRATES = {'serialport': ('4.10.1', '6ba5f8f29aa20853c4e3e85a33ec580eb66be1f057142e77a333834a318bacf2'),
          'ring': ('0.17.14', 'a4689e6c2294d81e88dc6261c768b63bc4fcdb852be6d1352498b114f61383b7')}


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def sha256(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def destination(relative):
    path = PurePosixPath(relative)
    require(not path.is_absolute() and '..' not in path.parts and '\\' not in relative,
            f'Unsafe evidence path: {relative}')
    return relative if relative == 'NOTICE.txt' else 'licenses/' + relative


def evidence(root, target, check_toolchain=True):
    """Validate source inputs before packaging; no network or source mutation."""
    root = Path(root)
    base = root / 'third-party-licenses/release'
    manifest = json.loads((base / 'evidence.json').read_text())
    require(manifest['schema'] == 1 and manifest['rust_release'] == RUST_RELEASE
            and manifest['rust_commit'] == RUST_COMMIT, 'Unreviewed license evidence version')
    require(target in ('windows', 'linux'), 'Unsupported evidence target')
    lock = tomllib.loads((root / 'Cargo.lock').read_text())
    for name, (version, checksum) in CRATES.items():
        matches = [p for p in lock['package'] if p['name'] == name]
        require(len(matches) == 1 and matches[0]['version'] == version
                and matches[0]['checksum'] == checksum, f'Unreviewed locked dependency: {name}')
    rows = [r for r in manifest['files'] if target in r['targets']]
    paths = {r['path'] for r in rows}
    require(len(paths) == len(rows), 'Duplicate evidence paths')
    required = {'NOTICE.txt', 'serialport-4.10.1/serialport-4.10.1.crate',
                'serialport-4.10.1/MPL-2.0.txt', 'serialport-4.10.1/SOURCE-CONTENTS.json',
                'serialport-4.10.1/LICENSE.txt', 'rust-1.95.0/COPYRIGHT-library.html',
                'rust-1.95.0/NOTICE.txt', 'rust-1.95.0/toolchain.json',
                'rust-1.95.0/compiler-builtins/LICENSE.txt',
                'rust-1.95.0/compiler-builtins/libm/LICENSE.txt',
                'rust-1.95.0/STD-CARGO-LOCK.toml', 'rust-1.95.0/std-package.toml'}
    required.update(f'rust-1.95.0/licenses/{name}.txt' for name in
                    ('MIT', 'Apache-2.0', 'Unicode-3.0', 'BSD-2-Clause'))
    if target == 'windows':
        required.update('ring-0.17.14/' + name for name in RING_FILES)
    else:
        required.update('rust-1.95.0/dependencies/' + path for path in
                        ('addr2line-0.25.1/LICENSE-APACHE', 'adler2-2.0.1/LICENSE-APACHE',
                         'memchr-2.7.6/LICENSE-MIT', 'miniz_oxide-0.8.9/LICENSE-APACHE.md',
                         'object-0.37.3/LICENSE-APACHE'))
    require(required <= paths, f'Missing required license evidence: {sorted(required - paths)}')
    for row in rows:
        destination(row['path'])
        require(sha256(base / row['path']) == row['sha256'], f"Evidence hash mismatch: {row['path']}")
    require(sha256(base / 'serialport-4.10.1/serialport-4.10.1.crate') == CRATES['serialport'][1],
            'MPL source archive differs from Cargo.lock')
    if check_toolchain:
        check_rust(base, target)
    return rows


def check_rust(base, target):
    output = subprocess.check_output(['rustc', '-vV'], text=True)
    require(f'release: {RUST_RELEASE}\n' in output and f'commit-hash: {RUST_COMMIT}\n' in output,
            'Toolchain changed: standard-library license review required')
    pinned = json.loads((base / 'rust-1.95.0/toolchain.json').read_text())
    scope = pinned['targets'][target]
    require(f"host: {scope['triple']}\n" in output, 'Unexpected Rust host')
    sysroot = Path(subprocess.check_output(['rustc', '--print', 'sysroot'], text=True).strip())
    require(sha256(sysroot / 'share/doc/rust/COPYRIGHT-library.html') == pinned['copyright_library_sha256'],
            'Installed Rust library notice differs from reviewed 1.95.0 evidence')
    for library in scope['rlibs']:
        path = sysroot / 'lib/rustlib' / scope['triple'] / 'lib' / library['file']
        require(sha256(path) == library['sha256'], f"Unreviewed sysroot library: {library['file']}")


def package_manifest(rows, target):
    return {'schema': 1, 'target': target, 'rust_release': RUST_RELEASE, 'rust_commit': RUST_COMMIT,
            'scope': 'Supplemental application and standard-library evidence; see NOTICE.txt',
            'files': [{**row, 'path': destination(row['path'])} for row in rows]}


def check_extracted(root, rows, target):
    """Check bytes after unpacking, not only the pre-archive stage."""
    root = Path(root)
    actual = json.loads((root / 'LICENSE-EVIDENCE.json').read_text())
    require(actual == package_manifest(rows, target), 'Extracted evidence inventory differs')
    for row in rows:
        require(sha256(root / destination(row['path'])) == row['sha256'],
                f"Extracted license/source missing or changed: {row['path']}")
