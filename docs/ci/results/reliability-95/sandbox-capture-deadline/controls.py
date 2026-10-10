"""Non-browser capture/exit/outer-timeout controls for the scoped CI helper."""
from pathlib import Path
import importlib.util
import json
import os
import sys

ROOT = Path(__file__).resolve().parents[3]
OUTPUT = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('sandbox', ROOT / 'scripts/ci/chrome_linux_sandbox.py')
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
engine = OUTPUT / 'controlled-process.py'
engine.write_text('#!' + sys.executable + '\n' + r'''
import os, pathlib, signal, sys, time, urllib.request
pathlib.Path(os.environ['CONTROL_PID']).write_text(str(os.getpid()))
assert '--timeout=10000' in sys.argv
assert '--no-sandbox' not in sys.argv
mode = os.environ['CONTROL_MODE']
body = urllib.request.urlopen(sys.argv[-1], timeout=2).read().decode()
if mode == 'missing-content':
    print('<title>Rustwright Linux sandbox preflight</title>', flush=True)
else:
    print(body, flush=True)
if mode == 'nonzero':
    sys.exit(7)
if mode == 'outer-timeout-exit-zero':
    signal.signal(signal.SIGTERM, lambda *_: sys.exit(0))
    time.sleep(60)
''')
engine.chmod(0o700)
records = []
for mode, passed in [('complete-dom', True), ('missing-content', False), ('nonzero', False), ('outer-timeout-exit-zero', False)]:
    directory = OUTPUT / 'controls' / mode
    directory.mkdir(parents=True, exist_ok=False)
    env = dict(os.environ, CONTROL_MODE=mode, CONTROL_PID=str(directory / 'child.pid'))
    # Only this controlled process uses the short outer deadline. Production
    # keeps its 25s outer deadline around the supported 10s capture budget.
    module.PROBE_TIMEOUT_SECONDS = 0.25 if mode == 'outer-timeout-exit-zero' else 25
    record = module.probe(engine, directory, mode, env)
    assert record['passed'] is passed
    assert '--timeout=10000' in record['command']
    pid = int((directory / 'child.pid').read_text())
    assert not Path(f'/proc/{pid}').exists()
    try:
        os.waitpid(pid, os.WNOHANG)
    except ChildProcessError:
        already_reaped = True
    else:
        already_reaped = False
    assert already_reaped
    record.update(owned_synthetic_pid=pid, process_absent=True, direct_child_already_reaped=True)
    if mode == 'outer-timeout-exit-zero':
        assert record['timed_out'] and record['exit_code'] == 0 and record['document_rendered']
        assert not record['passed']
    (directory / 'record.json').write_text(json.dumps(record, indent=2) + '\n')
    records.append(record)
(OUTPUT / 'controls-report.json').write_text(json.dumps({'status': 'passed', 'scope': 'Four owned synthetic Python processes; real fixture HTTP responses; no native browser/build execution or claim', 'records': records}, indent=2) + '\n')
print('Four capture/exit/timeout controls passed; timeout+exit0+full fixture DOM still fails')
