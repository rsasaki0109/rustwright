import hashlib
import json
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path('/workspace/rustwright')
OUT = ROOT / 'target/independent-memory'
PY = ROOT / 'bench/endurance/independent_bidi.py'
FROZEN = OUT / 'endurance-frozen'
expected = json.loads((OUT / 'rust-binary-sha256.json').read_text())['sha256']

def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

files = ['bench/endurance/independent_bidi.py', 'bench/endurance/test_independent_bidi.py', 'bench/reliability/proc_memory.py']
frozen_sources = {name: digest(ROOT / name) for name in files}
(OUT / 'independent-source-sha256.json').write_text(json.dumps(frozen_sources, indent=2) + '\n')
for name in files:
    destination = OUT / 'independent-source' / name
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.write_bytes((ROOT / name).read_bytes())
rows = [{'order': 1, 'driver': 'rust', 'workload': 'raw', 'directory': 'order-1-rust-raw'}]
sequence = [(2, 'context-only'), (3, 'evaluate'), (4, 'churn'),
            (5, 'churn'), (6, 'evaluate'), (7, 'context-only'), (8, 'rust-raw')]
for order, workload in sequence:
    assert all(digest(ROOT / name) == value for name, value in frozen_sources.items())
    assert digest(FROZEN) == expected
    directory = f'order-{order}-' + ('rust-raw' if workload == 'rust-raw' else f'python-{workload}')
    folder = OUT / directory
    if workload == 'rust-raw':
        command = [sys.executable, str(ROOT / 'bench/endurance/run.py'), '--engine', 'firefox', '--workload', 'raw',
                   '--cycles', '1000', '--warmup', '100', '--binary', str(FROZEN), '--output', str(folder)]
    else:
        command = [sys.executable, str(PY), '--workload', workload, '--cycles', '1000', '--warmup', '100',
                   '--checkpoint', '100', '--idle-tail', '0,10,30', '--output', str(folder)]
    print(json.dumps({'started_order': order, 'workload': workload, 'directory': directory}), flush=True)
    started = time.monotonic()
    with (OUT / (directory + '.log')).open('w') as log:
        subprocess.run(command, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT, check=True, timeout=1800)
    summary = json.loads((folder / 'summary.json').read_text())
    if isinstance(summary, list):
        summary = summary[0]
    assert summary['success'], summary.get('result')
    records = [json.loads(line) for line in (folder / 'firefox.jsonl').read_text().splitlines()]
    metadata = next(record for record in records if record['kind'] == 'metadata')
    assert not Path(f'/proc/{metadata["browser_pid"]}').exists()
    assert not Path(f'/proc/{metadata["driver_pid"]}').exists()
    rows.append({'order': order, 'driver': 'rust' if workload == 'rust-raw' else 'python',
                 'workload': 'raw' if workload == 'rust-raw' else workload,
                 'directory': directory, 'elapsed_seconds': round(time.monotonic() - started, 3),
                 'root_and_driver_exited': True})
    (OUT / 'order.json').write_text(json.dumps(rows, indent=2) + '\n')
    print(json.dumps({'completed_order': order, 'success': True, 'elapsed_seconds': rows[-1]['elapsed_seconds']}), flush=True)
print('All eight sequential runs completed.', flush=True)
