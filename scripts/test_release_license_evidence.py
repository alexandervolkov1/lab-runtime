"""License packaging regressions, without builds or network access."""
import importlib.util
import json
from pathlib import Path
import shutil
import sys
import tarfile
import tempfile
import unittest
from unittest.mock import patch

sys.dont_write_bytecode = True
import release_license_evidence as licenses

ROOT = Path(__file__).resolve().parent.parent


class EvidenceTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix='lab-license-test-')
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        shutil.copytree(ROOT / 'third-party-licenses/release', self.root / 'third-party-licenses/release')
        shutil.copy2(ROOT / 'Cargo.lock', self.root / 'Cargo.lock')

    def test_actual_toolchain(self):
        self.assertTrue(licenses.evidence(ROOT, 'linux'))

    def test_source_content_and_version(self):
        import tomllib
        base = self.root / 'third-party-licenses/release/serialport-4.10.1'
        with tarfile.open(base / 'serialport-4.10.1.crate') as tar:
            rows = json.loads((base / 'SOURCE-CONTENTS.json').read_text())
            self.assertEqual([r['path'] for r in rows], tar.getnames())
            for row in rows:
                self.assertEqual(licenses.hashlib.sha256(tar.extractfile(row['path']).read()).hexdigest(), row['sha256'])
            package = tomllib.loads(tar.extractfile('serialport-4.10.1/Cargo.toml').read().decode())['package']
            self.assertEqual((package['name'], package['version'], package['license']), ('serialport','4.10.1','MPL-2.0'))

    def test_every_supplement_rejects_corruption(self):
        for target in ('windows','linux'):
            for row in licenses.evidence(self.root, target, check_toolchain=False):
                with self.subTest(target=target, path=row['path']):
                    path = self.root / 'third-party-licenses/release' / row['path']
                    original = path.read_bytes()
                    try:
                        path.write_bytes(b'corrupt')
                        with self.assertRaises(RuntimeError):
                            licenses.evidence(self.root, target, check_toolchain=False)
                    finally:
                        path.write_bytes(original)

    def test_missing_manifest_row(self):
        path = self.root / 'third-party-licenses/release/evidence.json'
        manifest = json.loads(path.read_text())
        manifest['files'] = [r for r in manifest['files'] if r['path'] != 'rust-1.95.0/compiler-builtins/LICENSE.txt']
        path.write_text(json.dumps(manifest))
        with self.assertRaises(RuntimeError):
            licenses.evidence(self.root, 'linux', check_toolchain=False)

    def test_changed_lock_version_checksum(self):
        path = self.root / 'Cargo.lock'
        path.write_text(path.read_text().replace(licenses.CRATES['serialport'][1], '0'*64))
        with self.assertRaises(RuntimeError):
            licenses.evidence(self.root, 'linux', check_toolchain=False)

    def test_unknown_toolchain(self):
        path = self.root / 'third-party-licenses/release/evidence.json'
        text = path.read_text()
        path.write_text(text.replace('"rust_release": "1.95.0"','"rust_release": "1.96.0"'))
        with self.assertRaises(RuntimeError):
            licenses.evidence(self.root, 'linux', check_toolchain=False)

    def test_archive_extract_inventory_and_tamper(self):
        spec = importlib.util.spec_from_file_location('packager', ROOT / 'scripts/package-linux-runtime.py')
        packager = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(packager)
        rows = licenses.evidence(self.root, 'linux', check_toolchain=False)
        manifest = self.root / 'LICENSE-EVIDENCE.json'
        manifest.write_text(json.dumps(licenses.package_manifest(rows, 'linux')))
        files = [(licenses.destination(r['path']), self.root / 'third-party-licenses/release' / r['path'], 0o644) for r in rows]
        files.append(('LICENSE-EVIDENCE.json', manifest, 0o644))
        archive = self.root / 'licenses.tar.gz'
        packager.archive(archive, files, 0)
        package = self.root / 'extracted'
        with tarfile.open(archive) as tar:
            self.assertEqual(sorted(tar.getnames()), sorted(name for name, _, _ in files))
            tar.extractall(package, filter='data')
        licenses.check_extracted(package, rows, 'linux')
        (package / 'licenses/serialport-4.10.1/serialport-4.10.1.crate').unlink()
        with self.assertRaises(FileNotFoundError):
            licenses.check_extracted(package, rows, 'linux')

    def test_packager_main_inventory_and_provenance(self):
        # Stub external build/ELF queries, but execute real packaging, archive,
        # manifest and extracted-evidence code. No alternate Cargo target.
        spec = importlib.util.spec_from_file_location('packager', ROOT / 'scripts/package-linux-runtime.py')
        packager = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(packager)
        (self.root / 'Cargo.toml').write_text('[workspace.package]\nversion="0.1.0"\n')
        shutil.copy2(ROOT / 'LICENSE', self.root / 'LICENSE')
        binary = self.root / 'lab-runtime'
        binary.write_bytes(b'\x7fELF\x02\x01'+bytes(12)+b'\x3e\x00'+bytes(100))
        rows = licenses.evidence(self.root, 'linux', check_toolchain=False)
        def command(*args):
            if args == ('rustc','-vV'):
                return 'rustc 1.95.0\nhost: x86_64-unknown-linux-gnu'
            if args[:2] == ('git','status'):
                return ''
            if args[:2] == ('cargo','build'):
                return json.dumps({'reason':'compiler-artifact','target':{'name':'lab-runtime'},'executable':str(binary)})
            if args[:2] == ('cargo','metadata'):
                return '{}'
            if args[:2] == ('cargo','--version'):
                return 'cargo 1.95.0'
            if args[0] == 'ldd':
                return 'libc.so.6 => /usr/lib/libc.so.6'
            if args[0] == 'readelf':
                return 'GLIBC_2.34'
            if args[:2] == ('git','show'):
                return '1700000000'
            if args[:2] == ('git','rev-parse'):
                return '1'*40
            raise AssertionError(args)
        with patch.object(packager,'ROOT',self.root), patch.object(packager,'command',side_effect=command), \
             patch.object(packager,'evidence',return_value=rows), \
             patch.object(packager,'license_inputs',return_value=([('LICENSE',self.root/'LICENSE',0o644)],[])), \
             patch.object(packager.subprocess,'check_output',return_value=b''), patch.object(sys,'argv',['packager']):
            packager.main()
        prefix = 'lab-runtime-0.1.0-linux-x86_64'
        with tarfile.open(self.root / 'dist' / (prefix+'.tar.gz')) as tar:
            self.assertEqual(tar.getnames(),[prefix+'/NOTICE.txt',prefix+'/lab-runtime'])
        provenance=json.loads((self.root/'dist'/(prefix+'.build.json')).read_text())
        self.assertEqual((provenance['package'],provenance['commit']),(prefix,'1'*40))
        self.assertEqual(len(provenance['artifacts']),2)


if __name__ == '__main__':
    unittest.main(verbosity=2)
