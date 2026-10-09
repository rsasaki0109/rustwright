"""No browser needed: strict protocol failure, deadlines, and resource census."""
import asyncio
import json
from pathlib import Path
import sys
import subprocess
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
from independent_bidi import Protocol, memory, reporter_totals, session_endpoint


class ReporterTests(unittest.TestCase):
    def test_allocator_aliases_are_not_double_counted_with_explicit_leaves(self):
        def record(path, amount, kind=2, units=0, process='root'):
            return dict(path=path, amount=amount, kind=kind, units=units, process=process)
        data = {'reports': [record('heap-allocated', 100),
                            record('heap/committed/allocated', 100),
                            record('explicit/js/a', 30, kind=1),
                            record('explicit/js/b', 20, kind=0),
                            record('explicit/images/x', 10, kind=1),
                            record('explicit/count', 999, kind=1, units=1),
                            record('resident', 200),
                            record('heap-allocated', 25, process='child')]}
        result = reporter_totals(data)
        self.assertEqual(result['root']['explicit_categories_bytes'], {'js': 50, 'images': 10})
        self.assertEqual(result['root']['reported_explicit_heap_bytes'], 40)
        self.assertEqual(result['root']['unclassified_heap_bytes'], 60)
        self.assertEqual(result['root']['resident_bytes'], 200)
        self.assertEqual(result['child']['unclassified_heap_bytes'], 25)

    def test_missing_or_inconsistent_allocator_measurement_is_not_clamped(self):
        reports = [dict(process='missing', path='explicit/js/a', amount=10, kind=1, units=0),
                   dict(process='inconsistent', path='heap-allocated', amount=5, kind=2, units=0),
                   dict(process='inconsistent', path='explicit/js/a', amount=10, kind=1, units=0)]
        result = reporter_totals({'reports': reports})
        self.assertIsNone(result['missing']['unclassified_heap_bytes'])
        self.assertEqual(result['inconsistent']['unclassified_heap_bytes'], -5)


class Socket:
    def __init__(self, replies):
        self.replies = iter(replies)
        self.sent = []

    async def send(self, request):
        self.sent.append(json.loads(request))

    async def recv(self):
        reply = next(self.replies)
        if reply == 'wait':
            await asyncio.Event().wait()
        return json.dumps(reply)


class ProtocolTests(unittest.IsolatedAsyncioTestCase):
    async def test_events_do_not_complete_commands(self):
        socket = Socket([{'type': 'event', 'method': 'log.entryAdded', 'params': {}},
                         {'type': 'success', 'id': 1, 'result': {'ok': True}}])
        protocol = Protocol(socket)
        self.assertEqual(await protocol.command('probe', {}), {'ok': True})
        self.assertEqual(protocol.events['log.entryAdded'], 1)
        self.assertEqual(protocol.pending, set())

    async def test_protocol_error_is_failure_and_clears_local_waiter(self):
        protocol = Protocol(Socket([{'type': 'error', 'id': 1, 'error': 'no such frame', 'message': 'gone'}]))
        with self.assertRaisesRegex(RuntimeError, 'no such frame'):
            await protocol.command('browsingContext.close', {'context': 'gone'})
        self.assertFalse(protocol.pending)

    async def test_wrong_response_id_is_failure(self):
        protocol = Protocol(Socket([{'type': 'success', 'id': 99, 'result': {}}]))
        with self.assertRaisesRegex(RuntimeError, 'unexpected response ID'):
            await protocol.command('probe', {})
        self.assertFalse(protocol.pending)

    async def test_deadline_removes_local_waiter(self):
        protocol = Protocol(Socket(['wait']), deadline=.01)
        with self.assertRaises(TimeoutError):
            await protocol.command('probe', {})
        self.assertFalse(protocol.pending)

    async def test_counts_include_nested_frames_and_exact_ids(self):
        protocol = Protocol(Socket([
            {'type': 'success', 'id': 1, 'result': {'contexts': [
                {'context': 'z', 'children': [{'context': 'nested', 'children': None}]},
                {'context': 'a', 'children': []}]}},
            {'type': 'success', 'id': 2, 'result': {'userContexts': [
                {'userContext': 'default'}, {'userContext': 'isolated'}]}}]))
        self.assertEqual(await protocol.counts(), {
            'pages': ['a', 'nested', 'z'], 'user_contexts': ['default', 'isolated']})

    async def test_script_exception_does_not_count_as_successful_title(self):
        protocol = Protocol(Socket([{'type': 'success', 'id': 1, 'result': {
            'type': 'exception', 'exceptionDetails': {'text': 'bad'}}}]))
        with self.assertRaisesRegex(RuntimeError, 'unexpected title evaluation'):
            await protocol.title('page')

    def test_advertised_endpoint_always_uses_session_path(self):
        for endpoint in ('ws://127.0.0.1:123', 'ws://127.0.0.1:123/',
                         'ws://127.0.0.1:123/session'):
            self.assertEqual(session_endpoint(endpoint), 'ws://127.0.0.1:123/session')


class MemoryTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.proc = Path(self.directory.name)
        self.process(10, 1, 'S', 20, 15)
        self.process(20, 10, 'S', 100, 80)

    def process(self, pid, parent, state, rss, pss):
        folder = self.proc / str(pid)
        folder.mkdir()
        lines = [f'PPid:\t{parent}']
        if state is not None:
            lines.append(f'State:\t{state} (test)')
        if rss is not None:
            lines.append(f'VmRSS:\t{rss} kB')
        (folder / 'status').write_text('\n'.join(lines) + '\n')
        if pss is not None:
            (folder / 'smaps_rollup').write_text(f'Pss:\t{pss} kB\n')

    def test_only_confirmed_zombies_are_excluded_from_memory_totals(self):
        self.process(30, 20, 'Z', None, None)
        self.process(40, 20, 'S', 50, 30)
        record = memory(20, driver_pid=10, proc=self.proc)
        self.assertEqual(record['browser_tree_rss_kib'], 150)
        self.assertEqual(record['browser_tree_pss_kib'], 110)
        self.assertEqual(record['excluded_zombies'], [{'pid': 30, 'state': 'Z'}])
        self.assertEqual(record['browser_processes'], 2)
        self.assertEqual(record['browser_observed_processes'], 3)
        self.assertEqual(record['browser_root_pss_kib'], 80)

    def test_missing_live_pss_makes_total_null(self):
        self.process(30, 20, 'S', 50, None)
        record = memory(20, driver_pid=10, proc=self.proc)
        self.assertEqual(record['browser_tree_rss_kib'], 150)
        self.assertIsNone(record['browser_tree_pss_kib'])
        self.assertEqual(record['excluded_zombies'], [])

    def test_unknown_state_and_missing_rss_are_not_assumed_zombies(self):
        self.process(30, 20, None, None, None)
        record = memory(20, driver_pid=10, proc=self.proc)
        self.assertIsNone(record['browser_tree_rss_kib'])
        self.assertIsNone(record['browser_tree_pss_kib'])
        self.assertEqual(record['excluded_zombies'], [])

    def test_nonfinite_deadlines_and_idle_tails_are_rejected_before_launch(self):
        for flag, value in (('--deadline', 'nan'), ('--deadline', 'inf'),
                            ('--idle-tail', '0,nan'), ('--idle-tail', '0,inf')):
            completed = subprocess.run([sys.executable, str(Path(__file__).with_name('independent_bidi.py')),
                                        '--workload', 'churn', '--output', str(self.proc / 'unused'), flag, value],
                                       capture_output=True, text=True, timeout=5)
            self.assertEqual(completed.returncode, 2, completed.stderr)
            self.assertIn('invalid cycles', completed.stderr)
            self.assertFalse((self.proc / 'unused').exists())


if __name__ == '__main__':
    unittest.main()
