"""Bounded standard-library-only package smoke; owns only its virtual fixture."""
import argparse
import json
from pathlib import Path
import socket
import sqlite3
import subprocess
import time


def smoke(binary, output):
    binary, output = Path(binary).resolve(), Path(output).resolve()
    output.mkdir(parents=True, exist_ok=False)
    database = output / 'history.sqlite'
    command = [str(binary), '--serve', '--profile', 'virtual-demo', '--port', '0',
               '--record-db', str(database), '--record-policy', 'required']
    result = {'command': command, 'cwd': str(binary.parent)}
    flags = {'creationflags': subprocess.CREATE_NO_WINDOW} if hasattr(subprocess, 'CREATE_NO_WINDOW') else {}
    with (output/'stdout.log').open('w') as out, (output/'stderr.log').open('w') as err:
        process = subprocess.Popen(command, cwd=binary.parent, stdout=out, stderr=err, **flags)
        try:
            deadline = time.monotonic() + 20
            ready = None
            while time.monotonic() < deadline:
                for line in (output/'stdout.log').read_text(errors='replace').splitlines():
                    try:
                        row = json.loads(line)
                    except json.JSONDecodeError:
                        continue
                    if row.get('state') == 'ready':
                        ready = row
                if ready:
                    break
                assert process.poll() is None, 'Exited before ready'
                time.sleep(.05)
            assert ready, 'Readiness deadline'
            transcript = []
            with socket.create_connection(('127.0.0.1', ready['port']), timeout=10) as peer:
                peer.settimeout(15)
                with peer.makefile('rwb') as wire:
                    sequence = 0
                    scope = None

                    def call(op, args, mutation=False):
                        nonlocal sequence
                        request = {'v': 1, 'msg_id': str(len(transcript)), 'op': op, 'args': args}
                        if mutation:
                            sequence += 1
                            request['request_id'] = {'scope': scope, 'seq': str(sequence)}
                        transcript.append({'request': request})
                        wire.write(json.dumps(request).encode() + b'\n')
                        wire.flush()
                        while True:
                            reply = json.loads(wire.readline())
                            transcript.append({'response': reply})
                            assert reply['msg_id'] == request['msg_id'], reply
                            if reply.get('state') == 'accepted':
                                continue
                            assert (reply.get('state') == 'completed' if mutation else reply['type'] == 'result'), reply
                            return reply['result']

                    hello = call('hello', {'scope': None})
                    scope = hello['scope']
                    sequence = int(hello['next_seq']) - 1
                    call('discover', {})
                    call('describe', {'instrument': '1'})
                    run = call('recording_start', {'label': 'extracted Runtime smoke'}, True)
                    time.sleep(.3)
                    latest = call('latest', {'signal': {'instrument': '1', 'parameter': '1'}})
                    assert latest['quality'] == 'good', latest
                    stopped = call('recording_stop', {'run_id': run['run_id']}, True)
                    assert all(stopped['completion'][key] for key in
                               ('accepted_facts_drained', 'interval_sealed', 'transaction_committed')), stopped
                    shutdown = call('runtime_shutdown', {}, True)
                    result.update(ready=ready, latest=latest, recording=stopped, shutdown=shutdown)
            result['exit'] = process.wait(timeout=20)
            assert result['exit'] == 0
            (output/'transcript.json').write_text(json.dumps(transcript, indent=2)+'\n')
        finally:
            if process.poll() is None:
                process.terminate()
                process.wait(timeout=10)
                result['forced_cleanup'] = True
            (output/'result.json').write_text(json.dumps(result, indent=2)+'\n')
    closed = [json.loads(line) for line in (output/'stdout.log').read_text().splitlines() if line.startswith('{')]
    assert any(row.get('recorder_flushed') is True for row in closed), closed
    with sqlite3.connect(database.as_uri()+'?mode=ro', uri=True) as db:
        assert db.execute('PRAGMA integrity_check').fetchall() == [('ok',)]
        assert db.execute('SELECT state, coverage FROM runs').fetchall() == [('sealed', 'complete')]
        assert db.execute('SELECT state FROM runtime_boots').fetchall() == [('sealed',)]
        assert db.execute('SELECT count(*) FROM gaps').fetchone()[0] == 0
        assert db.execute('SELECT count(*) FROM measurements').fetchone()[0] > 0
    result.update(status='PASS', sqlite='integrity ok; sealed boot/run; complete; zero gaps', recorder_flushed=True)
    (output/'result.json').write_text(json.dumps(result, indent=2)+'\n')
    print(json.dumps(result, indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('binary')
    parser.add_argument('output', help='New retained evidence directory')
    args = parser.parse_args()
    smoke(args.binary, args.output)
