#!/usr/bin/env python3
"""Independent Python/WebSocket Firefox attribution probe; Linux, HTTP only.

No Rustwright transport, launcher, session, page, or injected helper is used.
One invocation owns one fresh browser/profile. Counts are protocol resources,
not a census of every browser allocation. Samples do not force garbage collection.
"""
import argparse
import asyncio
from collections import Counter
from datetime import datetime, timezone
import hashlib
import gzip
import json
import math
import os
from pathlib import Path
import platform
import re
import signal
import subprocess
import sys
import tempfile
import time
from urllib.parse import urlsplit, urlunsplit

import websockets
from websockets.asyncio.client import connect

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'bench/reliability'))
from proc_memory import fields  # noqa: E402


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def session_endpoint(advertised):
    parsed = urlsplit(advertised)
    return urlunsplit((parsed.scheme, parsed.netloc, '/session', '', ''))


def process_state(path):
    try:
        return next(line.split(':', 1)[1].split()[0]
                    for line in path.read_text().splitlines() if line.startswith('State:'))
    except (OSError, StopIteration, IndexError):
        return None


def memory(browser_pid, driver_pid=None, proc=Path('/proc')):
    """Use existing field parsing; exclude only positively identified zombies.

    A zombie has released its address space. A missing live-process measurement
    is different and keeps the corresponding aggregate unavailable.
    """
    driver_pid = os.getpid() if driver_pid is None else driver_pid
    processes = {int(p.name): {**fields(p / 'status'), 'state': process_state(p / 'status')}
                 for p in proc.iterdir() if p.name.isdecimal()}
    driver = processes.get(driver_pid, {})
    browser = processes.get(browser_pid, {})
    if 'VmRSS' not in driver or 'VmRSS' not in browser:
        raise RuntimeError('driver or owned browser root disappeared during sampling')
    pids = [browser_pid]
    for parent in pids:
        pids.extend(pid for pid, info in processes.items() if info.get('PPid') == parent)
    by_process = [{'pid': pid, 'root': pid == browser_pid,
                   'state': processes[pid]['state'],
                   'excluded_zombie': processes[pid]['state'] == 'Z',
                   'rss_kib': processes.get(pid, {}).get('VmRSS'),
                   'pss_kib': fields(proc / str(pid) / 'smaps_rollup').get('Pss')}
                  for pid in pids]
    live = [record for record in by_process if not record['excluded_zombie']]
    def total(key):
        values = [record[key] for record in live]
        return sum(values) if all(value is not None for value in values) else None
    return {'driver_pid': driver_pid, 'browser_pid': browser_pid,
            'driver_rss_kib': driver['VmRSS'],
            'driver_pss_kib': fields(proc / str(driver_pid) / 'smaps_rollup').get('Pss'),
            'browser_root_rss_kib': by_process[0]['rss_kib'],
            'browser_root_pss_kib': by_process[0]['pss_kib'],
            'browser_tree_rss_kib': total('rss_kib'),
            'browser_tree_pss_kib': total('pss_kib'),
            'browser_processes': len(live), 'browser_observed_processes': len(by_process),
            'excluded_zombies': [{'pid': record['pid'], 'state': record['state']}
                                 for record in by_process if record['excluded_zombie']],
            'browser_by_process': by_process,
            'sampling': 'sequential /proc reads; RSS double-counts shared mappings; '
                        'only State:Z excluded; missing live RSS/PSS makes aggregate null'}


def reporter_totals(data):
    """Summarize reporter leaves, not reachability or proof of a leak.

    Firefox's byte reporters use kind 0 for nonheap, 1 for heap and 2 for
    independent metrics. Only explicit leaves are added; independent allocator
    totals must not be added to them again.
    """
    totals = {}
    for record in data['reports']:
        process = totals.setdefault(record['process'], {
            'heap_allocated_bytes': None, 'reported_explicit_heap_bytes': 0,
            'explicit_categories_bytes': {}, 'resident_bytes': None})
        path, amount = record['path'], record['amount']
        if record['units'] != 0:
            continue
        if path == 'heap-allocated':
            process['heap_allocated_bytes'] = amount
        elif path == 'resident':
            process['resident_bytes'] = amount
        if path.startswith('explicit/') and record['kind'] in (0, 1):
            category = path.split('/')[1]
            categories = process['explicit_categories_bytes']
            categories[category] = categories.get(category, 0) + amount
            if record['kind'] == 1:
                process['reported_explicit_heap_bytes'] += amount
    for process in totals.values():
        allocated = process['heap_allocated_bytes']
        process['unclassified_heap_bytes'] = (
            None if allocated is None else allocated - process['reported_explicit_heap_bytes'])
    return totals


async def memory_report(browser_pid, directory, deadline):
    """Ask only this owned Firefox root for its normal Linux memory reporters.

    SIGRTMIN+1 would minimize memory and change the workload; it is never used.
    Check the installed handler first so an unsupported build fails safely.
    Firefox atomically renames a completed unified report; incomplete files are
    not accepted. A missing report is a measurement failure, never zero bytes.
    """
    started = time.monotonic()
    status = Path(f'/proc/{browser_pid}/status').read_text()
    caught = int(next(line.split()[1] for line in status.splitlines()
                      if line.startswith('SigCgt:')), 16)
    if not caught & (1 << (signal.SIGRTMIN - 1)):
        raise RuntimeError('owned Firefox has no installed memory-report signal handler')
    prior = set(directory.glob('unified-memory-report-*.json.gz'))
    os.kill(browser_pid, signal.SIGRTMIN)
    async with asyncio.timeout(deadline):
        while True:
            reports = set(directory.glob(f'unified-memory-report-*-{browser_pid}.json.gz')) - prior
            if reports:
                break
            await asyncio.sleep(.05)
    if len(reports) != 1:
        raise RuntimeError(f'expected one completed owned memory report, got {len(reports)}')
    report = reports.pop()
    data = json.loads(gzip.decompress(report.read_bytes()))
    return {'file': report.name, 'sha256': digest(report), 'size_bytes': report.stat().st_size,
            'signal': 'SIGRTMIN', 'minimize_memory': False,
            'capture_elapsed_ms': round((time.monotonic() - started) * 1000),
            'reporter_version': data['version'], 'report_count': len(data['reports']),
            'processes': reporter_totals(data),
            'scope': 'normal Firefox memory reporters; category/allocator amounts, '
                     'not unreachable-object or full allocation-stack attribution'}


class Protocol:
    """One in-flight command, strict error propagation, bounded send and receive."""
    def __init__(self, websocket, deadline=15):
        self.websocket = websocket
        self.deadline = deadline
        self.next_id = 0
        self.pending = set()
        self.commands = Counter()
        self.events = Counter()

    async def command(self, method, params):
        self.next_id += 1
        identity = self.next_id
        if self.pending:
            raise RuntimeError('probe commands must be sequential')
        self.pending.add(identity)
        self.commands[method] += 1
        try:
            async with asyncio.timeout(self.deadline):
                await self.websocket.send(json.dumps({'id': identity, 'method': method, 'params': params}))
                while True:
                    reply = json.loads(await self.websocket.recv())
                    if 'id' not in reply:
                        if reply.get('type') != 'event':
                            raise RuntimeError(f'invalid protocol event: {reply}')
                        self.events[reply.get('method', '<missing>')] += 1
                        continue
                    if reply['id'] != identity:
                        raise RuntimeError(f'unexpected response ID: {reply}')
                    if reply.get('type') != 'success' or 'result' not in reply:
                        raise RuntimeError(f'{method}: {reply}')
                    return reply['result']
        finally:
            self.pending.remove(identity)

    async def counts(self):
        tree = await self.command('browsingContext.getTree', {})
        users = await self.command('browser.getUserContexts', {})
        def walk(nodes):
            for node in nodes:
                yield node['context']
                yield from walk(node.get('children') or [])
        return {'pages': sorted(walk(tree['contexts'])),
                'user_contexts': sorted(user['userContext'] for user in users['userContexts'])}

    async def title(self, context):
        value = await self.command('script.evaluate', {
            'expression': 'document.title', 'target': {'context': context},
            'awaitPromise': True, 'resultOwnership': 'none'})
        if value.get('type') != 'success' or value.get('result') != {'type': 'string', 'value': 'endurance'}:
            raise RuntimeError(f'unexpected title evaluation: {value}')


async def churn(protocol, url, index):
    """Match examples/endurance.rs raw_cycle's helper-free command sequence."""
    user = (await protocol.command('browser.createUserContext', {}))['userContext']
    try:
        page = (await protocol.command('browsingContext.create', {'type': 'tab', 'userContext': user}))['context']
        await protocol.command('browsingContext.navigate', {'context': page, 'url': url, 'wait': 'complete'})
        await protocol.title(page)
        tree = await protocol.command('browsingContext.getTree', {})
        pages = [node for node in tree['contexts'] if node.get('userContext') == user]
        if len(pages) != 1 or pages[0]['context'] != page:
            raise RuntimeError('direct discovery did not find exactly the created tab')
        await protocol.title(page)
        if index % 2 == 0:
            await protocol.command('browsingContext.close', {'context': page})
    finally:
        await protocol.command('browser.removeUserContext', {'userContext': user})


async def http_fixture(reader, writer):
    try:
        request = await asyncio.wait_for(reader.readuntil(b'\r\n\r\n'), 5)
        if len(request) > 16384:
            raise RuntimeError('fixture request too large')
        # Exact examples/endurance.rs HTML fixture. This workload does not click.
        body = (b'<!doctype html><title>endurance</title><input id=q>'
                b"<button id=go onclick='window.clicked=event.isTrusted'>Go</button>")
        writer.write(b'HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n'
                     b'Cache-Control: no-store\r\nConnection: close\r\nContent-Length: '
                     + str(len(body)).encode() + b'\r\n\r\n' + body)
        await writer.drain()
    except (asyncio.TimeoutError, asyncio.IncompleteReadError, asyncio.LimitOverrunError, ConnectionError):
        pass
    finally:
        writer.close()
        await writer.wait_closed()


async def probe(args, output, emit):
    launcher = Path(os.environ['RUSTWRIGHT_FIREFOX']).resolve()
    stderr_path = output / 'firefox.stderr'
    profile = tempfile.TemporaryDirectory(prefix='rustwright-independent-bidi-')
    process = None
    pump = None
    server = None
    protocol = None
    http_tasks = set()
    result = {'kind': 'result', 'success': False}
    cleanup = {'kind': 'cleanup', 'profile': profile.name, 'browser_pid': None}
    started = time.monotonic()
    try:
        def accepted(reader, writer):
            task = asyncio.create_task(http_fixture(reader, writer))
            http_tasks.add(task)
            task.add_done_callback(http_tasks.discard)
        server = await asyncio.start_server(accepted, '127.0.0.1', 0, limit=16384)
        url = f'http://127.0.0.1:{server.sockets[0].getsockname()[1]}/page'
        command = [str(launcher), '--headless', '--no-remote', '--profile', profile.name,
                   '--remote-debugging-port=0', 'about:blank']
        browser_env = dict(os.environ)
        reports_dir = output / 'memory-reports'
        if args.memory_reports:
            reports_dir.mkdir()
            browser_env['TMPDIR'] = str(reports_dir)
        process = await asyncio.create_subprocess_exec(*command, env=browser_env, stdout=asyncio.subprocess.DEVNULL,
                                                       stderr=asyncio.subprocess.PIPE, start_new_session=True)
        cleanup['browser_pid'] = process.pid
        endpoint = asyncio.get_running_loop().create_future()
        async def drain_stderr():
            with stderr_path.open('wb') as log:
                while line := await process.stderr.readline():
                    log.write(line)
                    log.flush()
                    if b'WebDriver BiDi listening on' in line:
                        found = re.search(rb'ws://(?:127\.0\.0\.1|localhost|\[::1\]):\d+(?:/[^\s]*)?', line)
                        if found and not endpoint.done():
                            # Firefox advertises its base address, the BiDi session lives at /session.
                            endpoint.set_result(session_endpoint(found.group().decode()))
                if not endpoint.done():
                    endpoint.set_exception(RuntimeError('Firefox exited before advertising BiDi endpoint'))
        pump = asyncio.create_task(drain_stderr())
        address = await asyncio.wait_for(endpoint, args.deadline)
        async with connect(address, proxy=None, open_timeout=args.deadline, close_timeout=2,
                           max_size=16 * 1024 * 1024) as websocket:
            protocol = Protocol(websocket, args.deadline)
            session = await protocol.command('session.new', {'capabilities': {}})
            browser_binary = Path(f'/proc/{process.pid}/exe').resolve(strict=True)
            emit({'kind': 'metadata', 'workload': args.workload, 'session': session,
                  'browser_pid': process.pid, 'driver_pid': os.getpid(), 'endpoint': address,
                  'browser_command': command, 'browser_binary': str(browser_binary),
                  'browser_binary_sha256': digest(browser_binary), 'launcher_sha256': digest(launcher),
                  'fixture_url': url, 'helpers': 'no preload scripts or Rustwright helpers installed',
                  'cycles': args.cycles, 'warmup': args.warmup, 'checkpoint': args.checkpoint})
            retained_user = None
            retained_page = None
            if args.workload == 'evaluate':
                retained_user = (await protocol.command('browser.createUserContext', {}))['userContext']
                retained_page = (await protocol.command('browsingContext.create', {
                    'type': 'tab', 'userContext': retained_user}))['context']
                await protocol.command('browsingContext.navigate', {
                    'context': retained_page, 'url': url, 'wait': 'complete'})
                await protocol.title(retained_page)
            baseline = await protocol.counts()
            async def checkpoint(completed, idle_seconds=None):
                counts = await protocol.counts()
                pending = len(protocol.pending)
                sample = {'kind': 'sample', 'completed': completed,
                      'measured_completed': max(0, completed - args.warmup),
                      'elapsed_ms': round((time.monotonic() - started) * 1000),
                      'idle_seconds': idle_seconds, 'counts': counts, 'baseline': baseline,
                      'pending_commands': pending, 'commands': dict(protocol.commands),
                      'memory': memory(process.pid)}
                if args.memory_reports and (completed in (0, args.warmup, args.warmup + args.cycles)
                                            or idle_seconds is not None):
                    sample['memory_report'] = await memory_report(process.pid, reports_dir, args.deadline)
                emit(sample)
                if counts != baseline or pending:
                    raise RuntimeError(f'residual resources: counts={counts}, baseline={baseline}, pending={pending}')
            total = args.warmup + args.cycles
            for index in range(total + 1):
                if index:
                    async with asyncio.timeout(args.deadline):
                        if args.workload == 'churn':
                            await churn(protocol, url, index)
                        elif args.workload == 'context-only':
                            user = (await protocol.command('browser.createUserContext', {}))['userContext']
                            await protocol.command('browser.removeUserContext', {'userContext': user})
                        else:
                            await protocol.title(retained_page)
                if index in (0, args.warmup, total) or index % args.checkpoint == 0:
                    await asyncio.sleep(.1)
                    await checkpoint(index)
            idle_started = time.monotonic()
            for seconds in args.idle_tail:
                await asyncio.sleep(max(0, idle_started + seconds - time.monotonic()))
                await checkpoint(total, seconds)
            if retained_user is not None:
                await protocol.command('browser.removeUserContext', {'userContext': retained_user})
            await protocol.command('session.end', {})
            result.update(success=True, completed=total, commands=dict(protocol.commands),
                          events=dict(protocol.events), pending_commands=len(protocol.pending))
    except BaseException as error:
        result['error'] = f'{type(error).__name__}: {error}'
        if isinstance(error, (KeyboardInterrupt, asyncio.CancelledError)):
            result['interrupted'] = True
    finally:
        if server is not None:
            server.close()
            await server.wait_closed()
        for task in http_tasks:
            task.cancel()
        await asyncio.gather(*http_tasks, return_exceptions=True)
        if process is not None:
            if process.returncode is None:
                try:
                    os.killpg(process.pid, signal.SIGTERM)
                except ProcessLookupError:
                    pass
                try:
                    await asyncio.wait_for(process.wait(), 5)
                except asyncio.TimeoutError:
                    os.killpg(process.pid, signal.SIGKILL)
                    await process.wait()
            cleanup.update(browser_exit_code=process.returncode,
                           browser_root_exited=not Path(f'/proc/{process.pid}').exists())
        if pump is not None:
            try:
                await asyncio.wait_for(pump, 2)
            except asyncio.TimeoutError:
                pump.cancel()
                await asyncio.gather(pump, return_exceptions=True)
        profile.cleanup()
        cleanup['profile_removed'] = not Path(profile.name).exists()
        cleanup['all_historical_descendants_exited'] = 'not established'
        emit(cleanup)
        result['elapsed_ms'] = round((time.monotonic() - started) * 1000)
        if process is not None and not cleanup['browser_root_exited']:
            result.update(success=False, cleanup_error='owned browser root did not exit')
        emit(result)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--workload', choices=['churn', 'context-only', 'evaluate'], required=True)
    parser.add_argument('--cycles', type=int, default=1000)
    parser.add_argument('--warmup', type=int, default=100)
    parser.add_argument('--checkpoint', type=int, default=100)
    parser.add_argument('--idle-tail', default='0,10,30')
    parser.add_argument('--deadline', type=float, default=15)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--memory-reports', action='store_true',
                        help='Preserve normal Firefox Linux SIGRTMIN memory reports; no forced GC')
    args = parser.parse_args()
    try:
        args.idle_tail = [float(value) for value in args.idle_tail.split(',')]
    except ValueError:
        parser.error('idle-tail must be comma-separated seconds')
    if (args.cycles < 1 or args.warmup < 0 or args.checkpoint < 1
            or not math.isfinite(args.deadline) or args.deadline <= 0
            or any(not math.isfinite(t) or t < 0 for t in args.idle_tail)
            or args.idle_tail != sorted(set(args.idle_tail))):
        parser.error('invalid cycles, warmup, checkpoint, deadline or idle-tail')
    if platform.system() != 'Linux' or not os.environ.get('RUSTWRIGHT_FIREFOX'):
        parser.error('Linux /proc and RUSTWRIGHT_FIREFOX are required')
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    files = [Path(__file__).resolve(), ROOT / 'bench/reliability/proc_memory.py']
    dependency_root = Path(websockets.__file__).resolve().parent
    manifest = {'started_utc': datetime.now(timezone.utc).isoformat(), 'command': sys.argv,
                'python': sys.version, 'python_binary': sys.executable,
                'python_binary_sha256': digest(Path(sys.executable).resolve()),
                'websockets': websockets.__version__, 'platform': platform.platform(),
                'websockets_source_sha256': {str(p.relative_to(dependency_root)): digest(p)
                                            for p in sorted(dependency_root.rglob('*.py'))},
                'source_sha256': {str(p.relative_to(ROOT)): digest(p) for p in files},
                'git_head': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
                'git_status': subprocess.check_output(['git', 'status', '--porcelain'], cwd=ROOT, text=True).splitlines()}
    (output / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
    samples = []
    with (output / 'firefox.jsonl').open('w') as raw:
        def emit(record):
            raw.write(json.dumps(record) + '\n')
            raw.flush()
            if record['kind'] == 'sample':
                samples.append(record)
            print(json.dumps(record), flush=True)
        result = asyncio.run(probe(args, output, emit))
    summary = {'workload': args.workload, 'success': result['success'],
               'result': result, 'samples': samples}
    (output / 'summary.json').write_text(json.dumps(summary, indent=2) + '\n')
    return int(not result['success'])


if __name__ == '__main__':
    sys.exit(main())
