"""Read-only verification of a closed manual-output experiment archive."""
import hashlib
import json
import pathlib
import sqlite3
import sys

path = pathlib.Path(sys.argv[1]).resolve()
with sqlite3.connect(path.as_uri() + '?mode=ro', uri=True) as db:
    integrity = db.execute('PRAGMA integrity_check').fetchall()
    operations = db.execute('SELECT command, phase, count(*) FROM operation_events GROUP BY command, phase ORDER BY command, phase').fetchall()
    outputs = db.execute('SELECT stage, value, evidence_source, count(*) FROM output_events GROUP BY stage, value, evidence_source ORDER BY stage, value').fetchall()
    measurements = db.execute('SELECT quality, count(*), min(float_value), max(float_value) FROM measurements GROUP BY quality').fetchall()
    runs = db.execute('SELECT state, coverage FROM runs').fetchall()
    boots = db.execute('SELECT state FROM runtime_boots').fetchall()
    gaps = db.execute('SELECT count(*) FROM gaps').fetchone()[0]
    assert integrity == [('ok',)], integrity
    counts = {(command, phase): count for command, phase, count in operations}
    assert counts[('output_write', 'accepted')] == 3
    assert counts[('output_write', 'completed')] == 3
    assert not any(phase == 'failed' for _, phase, _ in operations)
    delivered = {value for stage, value, source, _ in outputs if stage == 'readback_verified' and source == 'virtual_simulation'}
    assert delivered == {10.0, 20.0, 15.0}, delivered
    for power in delivered:
        for stage in ('requested', 'send_started', 'readback_verified'):
            assert sum(count for actual_stage, value, _, count in outputs if actual_stage == stage and value == power) == 1
    assert gaps == 0, gaps
    assert runs == [('sealed', 'complete')], runs
    assert boots == [('sealed',)], boots
    assert any(quality == 'good' and count > 0 and high > low for quality, count, low, high in measurements)
    print(json.dumps({'archive': path.name, 'sha256': hashlib.sha256(path.read_bytes()).hexdigest(),
                     'integrity': integrity, 'operations': operations, 'outputs': outputs,
                     'measurements': measurements, 'runs': runs, 'boots': boots,
                     'gaps': gaps, 'status': 'PASS'}, indent=2))
