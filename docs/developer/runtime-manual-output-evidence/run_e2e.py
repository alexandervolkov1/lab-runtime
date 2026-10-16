"""Bounded Windows fixture: one real virtual Runtime and one external Babashka client."""
import json, pathlib, subprocess, sys, time, hashlib, platform

root = pathlib.Path(__file__).resolve().parents[3]
profile = sys.argv[1] if len(sys.argv) > 1 else 'debug'
if profile not in ('debug', 'release'):
    raise SystemExit('profile must be debug or release')
folder = root / 'target/runtime-manual-api' / (profile + '-e2e-' + time.strftime('%Y%m%d-%H%M%S'))
folder.mkdir(parents=True)
exe = root / 'target' / profile / 'lab-runtime.exe'
command = [str(exe), '--serve', '--profile', 'virtual-demo', '--port', '0', '--record-db', str(folder/'history.sqlite'), '--record-policy', 'required']
metadata = {'command': command, 'source_head': subprocess.check_output(['git','rev-parse','HEAD'],cwd=root,text=True).strip(),
            'binary_sha256': hashlib.sha256(exe.read_bytes()).hexdigest(), 'working_diff_sha256': hashlib.sha256(subprocess.check_output(['git','diff','HEAD'],cwd=root)).hexdigest()}
build_paths = ['Cargo.toml', 'Cargo.lock', 'apps/lab-runtime/Cargo.toml', 'crates/lab-core/Cargo.toml']
for directory in ['apps/lab-runtime/src', 'crates/lab-core/src']:
    build_paths.extend(path.relative_to(root).as_posix() for path in (root/directory).rglob('*.rs'))
for optional in ['rust-toolchain.toml', '.cargo/config.toml']:
    if (root/optional).exists(): build_paths.append(optional)
inputs = {}
for name in sorted(build_paths):
    content = (root/name).read_bytes().replace(b'\r\n', b'\n')
    committed = subprocess.check_output(['git', 'show', 'HEAD:'+name], cwd=root).replace(b'\r\n', b'\n')
    if content != committed: raise RuntimeError('uncommitted build input: '+name)
    inputs[name] = hashlib.sha256(content).hexdigest()
metadata['canonical_build_inputs'] = inputs
metadata['platform'] = platform.platform()
metadata['architecture'] = platform.machine()
metadata['rustc'] = subprocess.check_output(['rustc','-Vv'],cwd=root,text=True)
metadata['babashka'] = subprocess.check_output(['bb','--version'],cwd=root,text=True).strip()
with (folder/'runtime.stdout').open('w') as stdout, (folder/'runtime.stderr').open('w') as stderr:
    runtime = subprocess.Popen(command, cwd=root, stdout=stdout, stderr=stderr, creationflags=subprocess.CREATE_NO_WINDOW)
    try:
        deadline=time.monotonic()+20
        ready=None
        while time.monotonic()<deadline:
            for line in (folder/'runtime.stdout').read_text(errors='replace').splitlines():
                try: value=json.loads(line)
                except json.JSONDecodeError: continue
                if value.get('state')=='ready': ready=value; break
            if ready: break
            if runtime.poll() is not None: raise RuntimeError('Runtime exited before readiness')
            time.sleep(.05)
        if not ready: raise RuntimeError('Runtime readiness timeout')
        metadata['ready']=ready
        with (folder/'client.stdout').open('w') as out, (folder/'client.stderr').open('w') as err:
            client=subprocess.run(['bb','clients/babashka-smoke/manual-output.clj',str(ready['port']),'--shutdown-fixture'],cwd=root,stdout=out,stderr=err,timeout=60,creationflags=subprocess.CREATE_NO_WINDOW)
        metadata['client_exit']=client.returncode
        if client.returncode: raise RuntimeError('Babashka client failed')
        metadata['runtime_exit']=runtime.wait(timeout=15)
        if metadata['runtime_exit']: raise RuntimeError('Runtime cleanup failed')
    finally:
        if runtime.poll() is None:
            runtime.terminate();runtime.wait(timeout=10);metadata['forced_cleanup']=True
        (folder/'run.json').write_text(json.dumps(metadata,indent=2)+'\n')
        print(folder)
