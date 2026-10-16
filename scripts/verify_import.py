"""Verify the extraction against the explicit import manifest, without the donor."""
import hashlib
import json
from pathlib import Path
import subprocess
import tomllib

ROOT = Path(__file__).resolve().parent.parent


def git(*args):
    return subprocess.check_output(['git', *args], cwd=ROOT)


def main():
    manifest = json.loads((ROOT / 'IMPORT-MANIFEST.json').read_text())
    initial = git('rev-list', '--max-parents=0', 'HEAD').decode().strip()
    unchanged = []
    adapted = []
    for row in manifest['files']:
        name = row['path']
        original = git('show', f'{initial}:{name}')
        assert hashlib.sha256(original).hexdigest() == row['sha256'], name
        current = ROOT / name
        # Git text normalization is explicit; license/source archives stay binary.
        data = current.read_bytes() if current.exists() else None
        same = data == original
        if not same and data is not None and not name.startswith('third-party-licenses/'):
            same = data.replace(b'\r\n', b'\n') == original.replace(b'\r\n', b'\n')
        (unchanged if same else adapted).append(name)
        if name.startswith(('apps/lab-runtime/', 'crates/lab-core/', 'examples/', 'test-data/')) or name.endswith('manual-output.clj'):
            assert same, f'Imported implementation/fixture/client changed: {name}'
    before = tomllib.loads(git('show', f'{initial}:Cargo.lock').decode())['package']
    after = tomllib.loads((ROOT / 'Cargo.lock').read_text())['package']
    identity = lambda p: (p['name'], p['version'], p.get('source'), p.get('checksum'))
    assert set(map(identity, after)) <= set(map(identity, before)), 'Dependency added/upgraded'
    workspace = tomllib.loads((ROOT / 'Cargo.toml').read_text())['workspace']
    assert set(workspace['members']) == {'apps/lab-runtime', 'crates/lab-core'}
    print(json.dumps({'source_commit': manifest['source_commit'], 'import_commit': initial,
                     'imported_files': len(manifest['files']), 'unchanged_files': len(unchanged),
                     'adapted_or_relocated': adapted, 'lock_packages_before': len(before),
                     'lock_packages_after': len(after), 'new_dependency_identities': [],
                     'implementation_and_fixtures': 'unchanged', 'status': 'PASS'}, indent=2))


if __name__ == '__main__':
    main()
