"""Owned-process and truthful failed-report controls; no browsers or Rust builds."""
import contextlib
import io
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import threading
import unittest
from unittest import mock

import run


@unittest.skipUnless(os.name == 'posix', 'POSIX session ownership controls')
class ProcessOwnershipTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.directory = Path(self.temporary.name) / 'phase'

    def tearDown(self):
        self.temporary.cleanup()

    def measure(self, code, timeout=5):
        return run.run_measured([sys.executable, '-c', code], dict(os.environ), timeout,
                                artifacts=self.directory, observe_memory=False)

    def assert_reaped(self, metadata):
        pid = metadata['pid']
        self.assertTrue(metadata['cleanup']['direct_child_reaped'])
        self.assertTrue(metadata['cleanup']['group_absent'])
        self.assertEqual(metadata['cleanup']['errors'], [])
        with self.assertRaises(ChildProcessError):
            os.waitpid(pid, os.WNOHANG)
        self.assertFalse(Path(f'/proc/{pid}').exists())

    def test_success_preserves_raw_stdout_stderr_and_owned_reaping(self):
        result, memory = self.measure("import sys; print('output'); print('diagnostic', file=sys.stderr)")
        self.assertEqual(result.stdout, 'output\n')
        self.assertEqual(result.stderr, 'diagnostic\n')
        self.assertEqual(memory, [])
        self.assertEqual((self.directory / 'stdout.log').read_text(), result.stdout)
        self.assertEqual((self.directory / 'stderr.log').read_text(), result.stderr)
        self.assert_reaped(result.measurement)

    def test_setup_failure_keeps_original_exit_output_and_metadata(self):
        with self.assertRaises(subprocess.CalledProcessError) as caught:
            self.measure("import sys; print('partial'); print('setup failed', file=sys.stderr); sys.exit(7)")
        self.assertEqual(caught.exception.returncode, 7)
        self.assertEqual(caught.exception.stdout, 'partial\n')
        self.assertEqual(caught.exception.stderr, 'setup failed\n')
        metadata = json.loads((self.directory / 'process.json').read_text())
        self.assertEqual(metadata['exit_code'], 7)
        self.assertEqual(metadata['status'], 'failed')
        self.assert_reaped(metadata)

    def test_timeout_exit_zero_still_fails_and_reaps_real_sleep_child(self):
        code = "import signal,sys,time; signal.signal(signal.SIGTERM, lambda *_:sys.exit(0)); print('partial',flush=True); time.sleep(60)"
        with self.assertRaises(subprocess.TimeoutExpired) as caught:
            self.measure(code, timeout=0.3)
        metadata = caught.exception.measurement
        self.assertTrue(metadata['timed_out'])
        self.assertEqual(metadata['exit_code'], 0)
        self.assertEqual(metadata['status'], 'failed')
        self.assertEqual(caught.exception.stdout, 'partial\n')
        self.assert_reaped(metadata)

    def test_sigterm_refusal_escalates_and_reaps(self):
        code = "import signal,time; signal.signal(signal.SIGTERM,signal.SIG_IGN); print('ready',flush=True); time.sleep(60)"
        with self.assertRaises(subprocess.TimeoutExpired) as caught:
            self.measure(code, timeout=0.3)
        self.assertIn('SIGKILL', caught.exception.measurement['cleanup']['signals_sent'])
        self.assert_reaped(caught.exception.measurement)

    def test_observer_start_exception_reaps_child(self):
        with mock.patch.object(threading.Thread, 'start', side_effect=RuntimeError('controlled start failure')):
            with self.assertRaisesRegex(RuntimeError, 'controlled start failure') as caught:
                self.measure('import time; time.sleep(60)')
        self.assert_reaped(caught.exception.measurement)
        self.assertEqual(caught.exception.measurement['status'], 'failed')
        self.assertTrue((self.directory / 'stdout.log').is_file())
        self.assertTrue((self.directory / 'stderr.log').is_file())

    def test_memory_observer_failure_cannot_be_a_successful_measurement(self):
        with mock.patch.object(run, 'sample_memory', side_effect=RuntimeError('controlled memory failure')):
            with self.assertRaisesRegex(RuntimeError, 'memory observer') as caught:
                run.run_measured([sys.executable, '-c', 'import time; time.sleep(0.1)'], dict(os.environ), 5,
                                 artifacts=self.directory, observe_memory=True)
        self.assert_reaped(caught.exception.measurement)
        self.assertEqual(caught.exception.measurement['observer_errors'], ['RuntimeError: controlled memory failure'])

    def test_keyboard_interrupt_reaps_child_and_propagates_original_cancel(self):
        original = subprocess.Popen.wait

        def wait(process, timeout=None):
            if timeout == 17:
                raise KeyboardInterrupt('controlled cancellation')
            return original(process, timeout=timeout)

        with mock.patch.object(subprocess.Popen, 'wait', wait):
            with self.assertRaisesRegex(KeyboardInterrupt, 'controlled cancellation') as caught:
                self.measure('import time; time.sleep(60)', timeout=17)
        self.assert_reaped(caught.exception.measurement)
        self.assertEqual(caught.exception.measurement['status'], 'failed')

    def test_group_termination_reaches_owned_descendant_without_adopting_it(self):
        child_pid = Path(self.temporary.name) / 'descendant.pid'
        code = f"""
import pathlib,signal,subprocess,sys,time
child = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(60)'])
pathlib.Path({str(child_pid)!r}).write_text(str(child.pid))
def stop(*_):
    child.wait(timeout=2)
    sys.exit(0)
signal.signal(signal.SIGTERM, stop)
print('ready', flush=True)
time.sleep(60)
"""
        with self.assertRaises(subprocess.TimeoutExpired) as caught:
            self.measure(code, timeout=0.4)
        self.assert_reaped(caught.exception.measurement)
        self.assertFalse(Path(f'/proc/{int(child_pid.read_text())}').exists())


class FixtureOwnershipTests(unittest.TestCase):
    def test_thread_start_failure_closes_bound_listener(self):
        fixture = run.OwnedFixture()
        with mock.patch.object(threading.Thread, 'start', side_effect=RuntimeError('server start failure')):
            with self.assertRaisesRegex(RuntimeError, 'server start failure'):
                fixture.__enter__()
        self.assertEqual(fixture.cleanup, {'socket_closed': True, 'thread_finished': True, 'errors': []})

    def test_cancelled_scope_closes_listener_and_serving_thread(self):
        fixture = run.OwnedFixture()
        address = fixture.server.server_address
        with self.assertRaisesRegex(KeyboardInterrupt, 'fixture cancellation'):
            with fixture:
                raise KeyboardInterrupt('fixture cancellation')
        self.assertTrue(fixture.cleanup['socket_closed'])
        self.assertTrue(fixture.cleanup['thread_finished'])
        self.assertFalse(fixture.thread.is_alive())
        with socket.socket() as connection:
            self.assertNotEqual(connection.connect_ex(address), 0)


class MainReportTests(unittest.TestCase):
    """Mock browser/harness identities; exercise only report and phase policy."""

    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        self.output = self.root / 'output/report.json'
        self.chrome = self.root / 'chrome'
        self.rust = self.root / 'rust'
        self.chrome.write_bytes(b'controlled executable identity')
        self.rust.write_bytes(b'controlled Rust identity')
        package = self.root / 'bench/playwright/node_modules/playwright-core/package.json'
        package.parent.mkdir(parents=True)
        package.write_text('{"version":"controlled-reference"}')
        self.argv = ['run.py', '--chrome', str(self.chrome), '--rust-binary', str(self.rust), '--samples', '2',
                     '--pairs', '2', '--cases', 'delayed_click', '--output', str(self.output),
                     '--require-engine-success', 'rustwright']

    def tearDown(self):
        self.temporary.cleanup()

    def payload(self, engine, failed=False, missing=False):
        rows = [{'case': 'delayed_click', 'index': index, 'warmup': index == 0,
                 'ok': not (failed and index == 1), 'elapsed_ms': 120,
                 'error': 'controlled failed operation' if failed and index == 1 else None} for index in (0, 1)]
        if missing:
            rows.pop()
        return {'engine': engine, 'browser': 'Chrome/155.0.8059.39', 'records': rows,
                'launch_policy': {'headless': True, 'sandbox': True, 'scope': 'mock test only'}}

    def execute(self, outcome='complete'):
        def measured(command, env, timeout, artifacts=None, observe_memory=True):
            artifacts = Path(artifacts)
            artifacts.mkdir(parents=True)
            name = artifacts.name
            if name == 'setup-chrome-version':
                stdout = 'Google Chrome for Testing 155.0.8059.39\n'
            elif name == 'setup-source-files':
                stdout = ''
            elif name.startswith('setup-'):
                stdout = 'controlled setup metadata\n'
            else:
                engine = 'playwright-core' if 'playwright-core' in name else 'rustwright'
                payload = self.payload(engine, failed=outcome == 'measured-failure' and engine == 'rustwright',
                                       missing=outcome == 'incomplete')
                if outcome == 'wrong-version':
                    payload['browser'] = 'Chrome/151.0.7922.138'
                if outcome == 'missing-policy':
                    payload.pop('launch_policy')
                stdout = '{truncated' if outcome == 'malformed' else json.dumps(payload)
            (artifacts / 'stdout.log').write_text(stdout)
            (artifacts / 'stderr.log').write_text('controlled diagnostic\n')
            metadata = {'exit_code': 0, 'stdout': str(artifacts / 'stdout.log'),
                        'stderr': str(artifacts / 'stderr.log'), 'memory_samples': [], 'status': 'passed'}
            if not name.startswith('setup-') and outcome == 'setup-failure':
                error = subprocess.CalledProcessError(9, command, output=stdout, stderr='controlled diagnostic\n')
                metadata.update(exit_code=9, status='failed')
                error.measurement = metadata
                raise error
            if not name.startswith('setup-') and outcome == 'cancelled':
                error = KeyboardInterrupt('controlled phase cancellation')
                error.stdout = stdout
                error.measurement = metadata
                raise error
            (artifacts / 'process.json').write_text(json.dumps(metadata))
            completed = subprocess.CompletedProcess(command, 0, stdout, 'controlled diagnostic\n')
            completed.measurement = metadata
            return completed, []

        with mock.patch.object(run, 'ROOT', self.root), mock.patch.object(run, 'run_measured', side_effect=measured), \
                mock.patch.object(sys, 'argv', self.argv), contextlib.redirect_stdout(io.StringIO()):
            run.main()

    def report(self):
        return json.loads(self.output.read_text())

    def test_complete_alternates_order_and_preserves_launch_metadata(self):
        self.execute()
        report = self.report()
        self.assertEqual(report['status'], 'complete')
        self.assertTrue(report['complete'])
        self.assertTrue(report['success_requirement_met'])
        self.assertEqual([(r['pair'], r['engine']) for r in report['runs']],
                         [(1, 'rustwright'), (1, 'playwright-core'), (2, 'playwright-core'), (2, 'rustwright')])
        self.assertTrue(all(r['complete'] and r['launch_policy']['sandbox'] for r in report['runs']))
        self.assertEqual(report['summary']['rustwright']['delayed_click']['attempts'], 2)

    def test_measured_failures_keep_complete_report_but_fail_required_exit(self):
        with self.assertRaisesRegex(SystemExit, 'measured operation failures'):
            self.execute('measured-failure')
        report = self.report()
        self.assertEqual(report['status'], 'complete')
        self.assertTrue(report['complete'])
        self.assertFalse(report['success_requirement_met'])
        self.assertEqual(report['summary']['rustwright']['delayed_click']['successes'], 0)
        self.assertEqual(report['summary']['rustwright']['delayed_click']['attempts'], 2)

    def test_setup_failure_retains_valid_rows_as_incomplete_and_raw_diagnostics(self):
        with self.assertRaises(subprocess.CalledProcessError):
            self.execute('setup-failure')
        report = self.report()
        self.assertEqual(report['status'], 'failed')
        self.assertFalse(report['complete'])
        self.assertEqual(len(report['runs']), 1)
        self.assertFalse(report['runs'][0]['complete'])
        self.assertEqual(report['phases'][-1]['valid_partial_records'], 2)
        self.assertTrue(Path(report['phases'][-1]['process']['stderr']).is_file())

    def test_missing_rows_are_retained_truthfully_without_complete_claim(self):
        with self.assertRaisesRegex(ValueError, 'missing or extra'):
            self.execute('incomplete')
        report = self.report()
        self.assertEqual(report['status'], 'failed')
        self.assertFalse(report['complete'])
        self.assertEqual(len(report['runs'][0]['records']), 1)
        self.assertEqual(report['summary']['rustwright']['delayed_click']['attempts'], 0)

    def test_truncated_json_has_raw_evidence_but_no_invented_rows(self):
        with self.assertRaises(json.JSONDecodeError):
            self.execute('malformed')
        report = self.report()
        self.assertEqual(report['status'], 'failed')
        self.assertEqual(report['runs'], [])
        self.assertEqual(Path(report['phases'][-1]['process']['stdout']).read_text(), '{truncated')

    def test_cancelled_phase_retains_partial_report_and_original_cancellation(self):
        with self.assertRaisesRegex(KeyboardInterrupt, 'controlled phase cancellation'):
            self.execute('cancelled')
        report = self.report()
        self.assertEqual(report['status'], 'failed')
        self.assertFalse(report['complete'])
        self.assertFalse(report['runs'][0]['complete'])
        self.assertTrue(report['fixture_cleanup']['socket_closed'])
        self.assertTrue(report['fixture_cleanup']['thread_finished'])

    def test_protocol_identity_mismatch_retains_rows_without_complete_claim(self):
        with self.assertRaisesRegex(ValueError, 'protocol browser version differs'):
            self.execute('wrong-version')
        report = self.report()
        self.assertEqual(report['status'], 'failed')
        self.assertFalse(report['complete'])
        self.assertFalse(report['runs'][0]['complete'])
        self.assertEqual(report['runs'][0]['browser'], 'Chrome/151.0.7922.138')

    def test_missing_harness_launch_policy_cannot_complete(self):
        with self.assertRaisesRegex(ValueError, 'launch_policy'):
            self.execute('missing-policy')
        self.assertFalse(self.report()['complete'])
        self.assertEqual(self.report()['phases'][-1]['status'], 'failed')

    def test_existing_output_or_sidecars_are_never_overwritten(self):
        self.output.parent.mkdir()
        self.output.write_text('original evidence')
        with self.assertRaises(SystemExit):
            self.execute()
        self.assertEqual(self.output.read_text(), 'original evidence')
        self.output.unlink()
        sidecars = self.output.with_name(self.output.name + '.artifacts')
        sidecars.mkdir()
        (sidecars / 'sentinel').write_text('original sidecar')
        with self.assertRaises(SystemExit):
            self.execute()
        self.assertEqual((sidecars / 'sentinel').read_text(), 'original sidecar')
        self.assertFalse(self.output.exists())


if __name__ == '__main__':
    unittest.main()
