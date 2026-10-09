"""CDP transport/correlation/deadline/ownership controls; no native browser."""
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[3]
OUTPUT = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('sandbox', ROOT / 'scripts/ci/chrome_linux_sandbox.py')
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
engine = OUTPUT / 'controlled_peer.py'
engine.chmod(0o700)
records = []
modes = ['normal', 'fragmented-ping', 'wrong-id', 'wrong-session', 'foreign-session-event', 'foreign-frame', 'foreign-loader', 'bad-dom', 'non200', 'bad-handshake', 'close-frame', 'oversized-frame', 'masked-server', 'bad-continuation', 'early-exit', 'missing-endpoint', 'close-hang', 'external-cancel']
with tempfile.TemporaryDirectory(prefix='rustwright-cdp-controls-') as temporary:
    for mode in modes:
        directory = OUTPUT / 'controls' / mode
        directory.mkdir(parents=True, exist_ok=False)
        env = dict(os.environ, CONTROL_MODE='normal' if mode == 'external-cancel' else mode,
                   CONTROL_PID=str(directory / 'child.pid'), RUNNER_TEMP=temporary)
        module.PROBE_TIMEOUT_SECONDS = 0.6 if mode in ('foreign-session-event', 'foreign-frame', 'foreign-loader', 'missing-endpoint', 'close-hang') else 5
        if mode == 'external-cancel':
            with patch.object(module.CdpSocket, 'command', side_effect=KeyboardInterrupt('controlled caller cancellation')):
                try:
                    module.probe(engine, directory, mode, env)
                except KeyboardInterrupt:
                    pass
                else:
                    raise AssertionError('Cancellation must propagate')
            record = json.loads((directory / f'{mode}-probe.json').read_text())
        else:
            record = module.probe(engine, directory, mode, env)
        assert record['passed'] == (mode in ('normal', 'fragmented-ping')), (mode, record)
        if mode in ('foreign-session-event', 'foreign-frame', 'foreign-loader', 'missing-endpoint', 'close-hang'):
            assert record['timed_out'] and not record['passed']
        if mode == 'close-hang':
            assert record['document_rendered'] and record['exit_code'] == 0 and record['cleanup']['termination_requested']
        if mode == 'wrong-session':
            assert 'Unexpected CDP response id/session' in record['error']
        if mode == 'wrong-id':
            assert 'Unexpected CDP response id/session' in record['error']
        pid = int((directory / 'child.pid').read_text())
        assert not Path(f'/proc/{pid}').exists(), (mode, pid)
        try:
            os.waitpid(pid, os.WNOHANG)
        except ChildProcessError:
            already_reaped = True
        else:
            already_reaped = False
        assert already_reaped
        assert record['profile_removed'] and not Path(record['profile_path']).exists()
        assert not Path(record['profile_path']).is_relative_to(directory)
        records.append({'mode': mode, 'assertions': 'passed', 'probe_passed': record['passed'], 'owned_synthetic_pid': pid, 'child_absent_and_already_reaped': True, 'profile_removed_after_child_reap': True, 'raw_record': str(directory / f'{mode}-probe.json')})
    # A simulated OS cleanup refusal retains the owned profile outside artifacts.
    directory = OUTPUT / 'controls' / 'cleanup-refusal'
    directory.mkdir(parents=True, exist_ok=False)
    env = dict(os.environ, RUNNER_TEMP=temporary)
    class Unconfirmed:
        pid = 99999999
        returncode = None
        def poll(self): return None
        def wait(self, timeout): raise subprocess.TimeoutExpired('controlled unconfirmed child', timeout)
    module.PROBE_TIMEOUT_SECONDS = 0.05
    with patch.object(module.subprocess, 'Popen', return_value=Unconfirmed()), patch.object(module.os, 'killpg', side_effect=PermissionError('controlled OS termination refusal')):
        record = module.probe(engine, directory, 'cleanup-refusal', env)
    retained = Path(record['profile_retained_after_unconfirmed_exit'])
    assert retained.exists() and not retained.is_relative_to(directory)
    assert not record['passed'] and not record['cleanup']['direct_child_reaped'] and record['cleanup']['errors']
    shutil.rmtree(retained)  # Fixture-owned cleanup: no actual process was spawned.
    records.append({'mode': 'cleanup-refusal', 'assertions': 'passed', 'scope': 'mock Popen/OS refusal; no real child', 'profile_retained_outside_artifacts': True, 'raw_record': str(directory / 'cleanup-refusal-probe.json')})
(OUTPUT / 'controls-report.json').write_text(json.dumps({'status': 'passed', 'scope': '18 owned synthetic Python peers plus one mock refusal; no browser/renderer/build execution or native repair claim', 'controls': records}, indent=2) + '\n')
print('19 CDP/deadline/cleanup controls passed; native confirmation remains pending')
