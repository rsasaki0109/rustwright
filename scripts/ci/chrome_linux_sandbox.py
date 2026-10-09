#!/usr/bin/env python3
"""Diagnose Linux Chrome launch and configure its bundled sandbox when required."""
from __future__ import annotations

import argparse
import base64
import hashlib
import hmac
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import platform
import re
import secrets
import signal
import shutil
import stat
import socket
import struct
import subprocess
import sys
import tempfile
import threading
import time

PROBE_TIMEOUT_SECONDS = 25
MAX_MESSAGE_BYTES = 1024 * 1024
MAX_EVENTS = 4096
MAX_EVENT_BYTES = 2 * 1024 * 1024
MAX_LOG_BYTES = 4 * 1024 * 1024
FIXTURE_TITLE = '<title>Rustwright Linux sandbox preflight</title>'
FIXTURE_CONTENT = '<h1>ready</h1>'


def digest(path: Path) -> str:
    result = hashlib.sha256()
    with path.open('rb') as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b''):
            result.update(chunk)
    return result.hexdigest()


def file_info(path: Path) -> dict:
    info = path.stat()
    return {'path': str(path), 'uid': info.st_uid, 'gid': info.st_gid,
            'mode': oct(stat.S_IMODE(info.st_mode)), 'bytes': info.st_size,
            'sha256': digest(path)}


class FixtureHandler(BaseHTTPRequestHandler):
    def do_GET(self) -> None:
        body = ('<!doctype html>' + FIXTURE_TITLE + FIXTURE_CONTENT).encode('utf-8')
        self.send_response(200)
        self.send_header('Content-Type', 'text/html; charset=utf-8')
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)
        self.server.fixture_records.append({'path': self.path, 'status': 200, 'bytes_written': len(body)})

    def log_message(self, *args) -> None:
        pass


def remaining(deadline: float) -> float:
    duration = deadline - time.monotonic()
    if duration <= 0:
        raise TimeoutError('Shared Chrome CDP preflight deadline expired')
    return duration


class CdpSocket:
    """Bounded localhost RFC6455 text transport, without third-party dependencies."""

    def __init__(self, port: int, path: str, deadline: float, log):
        self.deadline, self.log = deadline, log
        self.buffer = b''
        self.events = []
        self.event_bytes = 0
        self.logged_bytes = 0
        self.next_id = 0
        self.sock = socket.create_connection(('127.0.0.1', port), timeout=remaining(deadline))
        try:
            key = base64.b64encode(secrets.token_bytes(16)).decode()
            request = (f'GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n'
                       f'Upgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\n'
                       'Sec-WebSocket-Version: 13\r\n\r\n')
            self.sock.sendall(request.encode('ascii'))
            while b'\r\n\r\n' not in self.buffer:
                self.sock.settimeout(remaining(self.deadline))
                data = self.sock.recv(4096)
                if not data:
                    raise RuntimeError('CDP socket closed during WebSocket handshake')
                self.buffer += data
                if len(self.buffer) > 16384:
                    raise RuntimeError('WebSocket handshake exceeded bounded header size')
            header, self.buffer = self.buffer.split(b'\r\n\r\n', 1)
            lines = header.decode('ascii').split('\r\n')
            headers = {}
            for line in lines[1:]:
                name, value = line.split(':', 1)
                name = name.lower()
                if name in headers:
                    raise RuntimeError('Duplicate WebSocket handshake header')
                headers[name] = value.strip()
            expected = base64.b64encode(hashlib.sha1((key + '258EAFA5-E914-47DA-95CA-C5AB0DC85B11').encode()).digest()).decode()
            if (not re.fullmatch(r'HTTP/1\.1 101(?: .*)?', lines[0])
                    or headers.get('upgrade', '').lower() != 'websocket'
                    or 'upgrade' not in [token.strip() for token in headers.get('connection', '').lower().split(',')]
                    or not hmac.compare_digest(headers.get('sec-websocket-accept', ''), expected)):
                raise RuntimeError('Invalid WebSocket upgrade/accept response')
        except BaseException:
            self.sock.close()
            raise

    def close(self) -> None:
        self.sock.close()

    def log_message(self, direction: str, value: dict) -> None:
        line = json.dumps({'direction': direction, 'message': value}) + '\n'
        self.logged_bytes += len(line.encode('utf-8'))
        if self.logged_bytes > MAX_LOG_BYTES:
            raise RuntimeError('CDP raw protocol log exceeded bound')
        self.log.write(line)
        self.log.flush()

    def read(self, size: int) -> bytes:
        while len(self.buffer) < size:
            self.sock.settimeout(remaining(self.deadline))
            data = self.sock.recv(min(65536, size - len(self.buffer)))
            if not data:
                raise RuntimeError('CDP WebSocket closed before requested response')
            self.buffer += data
        result, self.buffer = self.buffer[:size], self.buffer[size:]
        return result

    def send(self, opcode: int, data: bytes) -> None:
        if len(data) > MAX_MESSAGE_BYTES:
            raise RuntimeError('Outgoing CDP message exceeded bound')
        length = len(data)
        header = bytes([0x80 | opcode])
        header += bytes([0x80 | length]) if length < 126 else bytes([0x80 | 126]) + struct.pack('!H', length) if length < 65536 else bytes([0x80 | 127]) + struct.pack('!Q', length)
        mask = secrets.token_bytes(4)
        masked = bytes(value ^ mask[index % 4] for index, value in enumerate(data))
        self.sock.settimeout(remaining(self.deadline))
        self.sock.sendall(header + mask + masked)

    def receive(self) -> dict:
        message = bytearray()
        fragmented = False
        while True:
            first, second = self.read(2)
            final, opcode = bool(first & 0x80), first & 0x0f
            if first & 0x70 or second & 0x80:
                raise RuntimeError('Unsupported reserved bits or masked server frame')
            length = second & 0x7f
            if length == 126:
                length = struct.unpack('!H', self.read(2))[0]
                if length < 126:
                    raise RuntimeError('Noncanonical WebSocket frame length')
            elif length == 127:
                length = struct.unpack('!Q', self.read(8))[0]
                if length < 65536:
                    raise RuntimeError('Noncanonical WebSocket frame length')
            if length > MAX_MESSAGE_BYTES or len(message) + length > MAX_MESSAGE_BYTES:
                raise RuntimeError('Incoming CDP message exceeded bound')
            if opcode >= 8 and (not final or length > 125):
                raise RuntimeError('Invalid WebSocket control frame')
            payload = self.read(length)
            if opcode == 8:
                raise RuntimeError('CDP WebSocket sent close before requested response')
            if opcode == 9:
                self.send(10, payload)
                continue
            if opcode == 10:
                continue
            if opcode not in (0, 1) or (opcode == 0) != fragmented:
                raise RuntimeError('Invalid CDP text/continuation frame sequence')
            message.extend(payload)
            if final:
                value = json.loads(message.decode('utf-8'))
                if not isinstance(value, dict):
                    raise RuntimeError('CDP message is not an object')
                self.log_message('received', value)
                return value
            fragmented = True

    def event(self, message: dict) -> None:
        if not isinstance(message.get('method'), str) or not isinstance(message.get('params', {}), dict):
            raise RuntimeError('Invalid unsolicited CDP event')
        self.event_bytes += len(json.dumps(message).encode('utf-8'))
        if len(self.events) >= MAX_EVENTS or self.event_bytes > MAX_EVENT_BYTES:
            raise RuntimeError('CDP event buffer exceeded bound')
        self.events.append(message)

    def command(self, method: str, params=None, session=None) -> dict:
        self.next_id += 1
        message = {'id': self.next_id, 'method': method, 'params': params or {}}
        if session is not None:
            message['sessionId'] = session
        self.log_message('sent', message)
        self.send(1, json.dumps(message).encode())
        while True:
            response = self.receive()
            if 'id' not in response:
                self.event(response)
                continue
            if type(response['id']) is not int or response['id'] != message['id'] or response.get('sessionId') != session:
                raise RuntimeError(f'Unexpected CDP response id/session for {method}: {response}')
            if 'error' in response or not isinstance(response.get('result'), dict):
                raise RuntimeError(f'CDP {method} failed: {response}')
            return response['result']

    def wait_event(self, method: str, session: str, predicate) -> dict:
        while True:
            for index, event in enumerate(self.events):
                if event.get('sessionId') == session and event['method'] == method and predicate(event.get('params', {})):
                    self.event_bytes -= len(json.dumps(event).encode('utf-8'))
                    return self.events.pop(index)['params']
            response = self.receive()
            if 'id' in response:
                raise RuntimeError('Unsolicited CDP command response while awaiting event')
            self.event(response)


def terminate_owned(process: subprocess.Popen) -> dict:
    result = {'termination_requested': False, 'errors': []}
    if process.poll() is None:
        result['termination_requested'] = True
        for sig in (signal.SIGTERM, signal.SIGKILL):
            try:
                os.killpg(process.pid, sig)
            except ProcessLookupError:
                pass
            except OSError as error:
                result['errors'].append(str(error))
            try:
                process.wait(timeout=2)
                break
            except (OSError, subprocess.TimeoutExpired) as error:
                if sig == signal.SIGKILL:
                    result['errors'].append(str(error))
    result.update(direct_child_reaped=process.poll() is not None, exit_code=process.returncode)
    return result


def probe(chrome: Path, output: Path, phase: str, env: dict[str, str]) -> dict:
    deadline = time.monotonic() + PROBE_TIMEOUT_SECONDS
    stdout, stderr = output / f'{phase}-stdout.log', output / f'{phase}-stderr.log'
    protocol = output / f'{phase}-cdp.jsonl'
    record = {'phase': phase, 'method': 'CDP HTTP renderer preflight', 'outer_timeout_seconds': PROBE_TIMEOUT_SECONDS,
              'stdout': str(stdout), 'stderr': str(stderr), 'protocol': str(protocol), 'timed_out': False,
              'document_rendered': False, 'exit_code': None, 'passed': False}
    server = thread = profile = process = cdp = failure = None
    try:
        # A retained profile must never upload its private cache through artifacts.
        profile = Path(tempfile.mkdtemp(prefix=f'rustwright-{phase}-', dir=env.get('RUNNER_TEMP') or None))
        if profile.resolve().is_relative_to(output.resolve()):
            raise ValueError('Owned profile must remain outside uploaded artifact output')
        record['profile_path'] = str(profile)
        server = ThreadingHTTPServer(('127.0.0.1', 0), FixtureHandler)
        server.daemon_threads = True
        server.fixture_records = []
        thread = threading.Thread(target=lambda: server.serve_forever(poll_interval=0.05), daemon=True)
        thread.start()
        fixture_path = '/preflight-' + secrets.token_hex(8)
        url = f'http://127.0.0.1:{server.server_port}{fixture_path}'
        command = [str(chrome), '--no-first-run', '--no-default-browser-check', f'--user-data-dir={profile}',
                   '--headless=new', '--disable-gpu', '--hide-scrollbars', '--mute-audio',
                   '--remote-debugging-address=127.0.0.1', '--remote-debugging-port=0', 'about:blank']
        record.update(command=command, fixture_url=url, fixture_path=fixture_path)
        with stdout.open('wb') as out, stderr.open('wb') as err, protocol.open('w') as log:
            process = subprocess.Popen(command, env=env, stdin=subprocess.DEVNULL, stdout=out, stderr=err, start_new_session=True)
            port_file = profile / 'DevToolsActivePort'
            while True:
                remaining(deadline)
                if process.poll() is not None:
                    raise RuntimeError(f'Chrome exited before its debugging endpoint: {process.returncode}')
                if port_file.is_file():
                    lines = port_file.read_text().splitlines()
                    if len(lines) >= 2:
                        port = int(lines[0])
                        if not 0 < port < 65536 or not re.fullmatch(r'/devtools/browser/[A-Za-z0-9-]+', lines[1]):
                            raise RuntimeError('Invalid local DevToolsActivePort endpoint')
                        break
                time.sleep(min(0.02, remaining(deadline)))
            cdp = CdpSocket(port, lines[1], deadline, log)
            target = cdp.command('Target.createTarget', {'url': 'about:blank'})['targetId']
            session = cdp.command('Target.attachToTarget', {'targetId': target, 'flatten': True})['sessionId']
            if not isinstance(target, str) or not target or not isinstance(session, str) or not session:
                raise RuntimeError('CDP did not create/attach a valid target/session')
            cdp.command('Page.enable', session=session)
            cdp.command('Page.setLifecycleEventsEnabled', {'enabled': True}, session)
            cdp.command('Network.enable', session=session)
            navigation = cdp.command('Page.navigate', {'url': url}, session)
            frame, loader = navigation.get('frameId'), navigation.get('loaderId')
            if navigation.get('errorText') or navigation.get('isDownload') or not isinstance(frame, str) or not frame or not isinstance(loader, str) or not loader:
                raise RuntimeError(f'HTTP fixture navigation failed: {navigation}')
            record['navigation'] = navigation
            lifecycle = cdp.wait_event('Page.lifecycleEvent', session, lambda event: event.get('frameId') == frame and event.get('loaderId') == loader and event.get('name') == 'DOMContentLoaded')
            response = cdp.wait_event('Network.responseReceived', session, lambda event: event.get('frameId') == frame and event.get('loaderId') == loader and event.get('type') == 'Document' and event.get('response', {}).get('url') == url)
            if response['response'].get('status') != 200 or response['response'].get('mimeType') != 'text/html':
                raise RuntimeError(f'Fixture document did not return expected HTTP200 HTML: {response}')
            if not isinstance(response.get('requestId'), str) or not response['requestId']:
                raise RuntimeError('Fixture document response omitted its request identifier')
            finished = cdp.wait_event('Network.loadingFinished', session, lambda event: event.get('requestId') == response.get('requestId'))
            evaluation = cdp.command('Runtime.evaluate', {'expression': '({url:location.href,title:document.title,body:document.body.innerHTML,readyState:document.readyState})', 'returnByValue': True}, session)
            value = evaluation.get('result', {}).get('value')
            if (evaluation.get('exceptionDetails') or not isinstance(value, dict) or value.get('url') != url
                    or value.get('title') != 'Rustwright Linux sandbox preflight' or value.get('body') != FIXTURE_CONTENT
                    or value.get('readyState') not in ('interactive', 'complete')):
                raise RuntimeError(f'Renderer fixture DOM did not match: {evaluation}')
            record.update(document_rendered=True, document=value, lifecycle=lifecycle, document_response=response, loading_finished=finished)
            cdp.command('Browser.close')
            record['browser_close_acknowledged'] = True
            process.wait(timeout=remaining(deadline))
            if process.returncode != 0:
                raise RuntimeError(f'Chrome exited abnormally after Browser.close: {process.returncode}')
    except BaseException as error:
        failure = error
        record.update(error=f'{type(error).__name__}: {error}', timed_out=isinstance(error, (TimeoutError, subprocess.TimeoutExpired)))
    finally:
        if cdp is not None:
            cdp.close()
        if process is not None:
            record['cleanup'] = terminate_owned(process)
            record['exit_code'] = process.returncode
        if server is not None:
            if thread is not None and thread.ident is not None:
                server.shutdown()
                thread.join(timeout=2)
            server.server_close()
            record['fixture_requests'] = server.fixture_records
        confirmed_exit = process is None or process.returncode is not None
        if profile is not None and confirmed_exit:
            try:
                shutil.rmtree(profile)
                record['profile_removed'] = True
            except OSError as error:
                record['profile_cleanup_error'] = str(error)
        elif profile is not None:
            record['profile_retained_after_unconfirmed_exit'] = str(profile)
        cleanup = record.get('cleanup', {})
        record['passed'] = (failure is None and record['document_rendered'] and record['exit_code'] == 0
                            and record.get('browser_close_acknowledged') is True and record.get('profile_removed') is True
                            and not cleanup.get('termination_requested') and not cleanup.get('errors')
                            and any(item['path'] == record.get('fixture_path') and item['status'] == 200 and item['bytes_written'] > 0 for item in record.get('fixture_requests', [])))
        (output / f'{phase}-probe.json').write_text(json.dumps(record, indent=2) + '\n')
        print(f'Chrome {phase}: {json.dumps(record)}', flush=True)
        print(stderr.read_text(errors='replace')[:12000] if stderr.is_file() else '', flush=True)
    if failure is not None and not isinstance(failure, Exception):
        raise failure
    return record


def install_helper(chrome: Path, version: str, output: Path) -> Path:
    if (os.environ.get('GITHUB_ACTIONS') != 'true' or os.getuid() == 0
            or not os.environ.get('GITHUB_ENV')):
        raise RuntimeError('Sandbox installation requires the non-root GitHub Linux runner')
    # Chrome155 consults CHROME_DEVEL_SANDBOX only for user-owned builds.
    if chrome.stat().st_uid != os.getuid():
        raise RuntimeError('Chrome executable is not runner-owned; retain diagnostics for explicit setup review')
    source = chrome.parent / 'chrome_sandbox'
    if not source.is_file() or source.is_symlink():
        raise RuntimeError(f'The selected Chrome bundled sandbox helper is missing: {source}')
    with source.open('rb') as stream:
        if stream.read(4) != b'\x7fELF':
            raise RuntimeError('The bundled sandbox helper is not an ELF executable')
    destination = Path('/usr/local/lib/rustwright-ci') / version / 'chrome-sandbox'
    commands = [
        ['sudo', '-n', 'install', '-d', '-o', 'root', '-g', 'root', '-m', '0755', str(destination.parent)],
        ['sudo', '-n', 'install', '-o', 'root', '-g', 'root', '-m', '4755', str(source), str(destination)],
    ]
    with (output / 'helper-install.log').open('w') as log:
        for command in commands:
            print(json.dumps(command), file=log, flush=True)
            subprocess.run(command, check=True, stdout=log, stderr=subprocess.STDOUT, timeout=15)
    installed = destination.stat()
    if installed.st_uid != 0 or installed.st_gid != 0 or stat.S_IMODE(installed.st_mode) != 0o4755:
        raise RuntimeError('Installed sandbox helper must be root:root mode4755')
    if digest(source) != digest(destination):
        raise RuntimeError('Installed helper differs from the selected Chrome bundled helper')
    return destination


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, default=Path('target/ci/linux-sandbox'))
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    report = {'platform': platform.platform(), 'status': 'running', 'probes': [],
              'sandbox_disabled': False, 'global_policy_modified': False}
    path = output / 'report.json'

    def save() -> None:
        path.write_text(json.dumps(report, indent=2) + '\n')

    save()
    try:
        if sys.platform != 'linux':
            raise RuntimeError('This setup helper is Linux-only')
        configured = os.environ.get('RUSTWRIGHT_CHROME', '')
        if not configured:
            raise RuntimeError('RUSTWRIGHT_CHROME must select the action-installed Chrome executable')
        chrome = Path(configured).resolve(strict=True)
        report['uid'] = os.getuid()
        report['chrome'] = file_info(chrome)
        with chrome.open('rb') as stream:
            if stream.read(4) != b'\x7fELF':
                raise RuntimeError('RUSTWRIGHT_CHROME must select native action-installed Chrome, not a wrapper')
        version_text = subprocess.check_output([str(chrome), '--version'], text=True, timeout=10).strip()
        match = re.search(r'\b(\d+\.\d+\.\d+\.\d+)\b', version_text)
        if not match:
            raise RuntimeError(f'Unable to determine installed Chrome version: {version_text}')
        version = match.group(1)
        report['version'] = version_text
        policies = {}
        for name in ['/proc/sys/kernel/apparmor_restrict_unprivileged_userns',
                     '/proc/sys/kernel/unprivileged_userns_clone', '/sys/module/apparmor/parameters/enabled']:
            policy = Path(name)
            policies[name] = policy.read_text().strip() if policy.is_file() else None
        report['host_policy_observation'] = policies
        bundled = chrome.parent / 'chrome_sandbox'
        report['bundled_helper'] = file_info(bundled) if bundled.is_file() else None
        env = dict(os.environ)
        report['initial_CHROME_DEVEL_SANDBOX'] = env.get('CHROME_DEVEL_SANDBOX')
        save()
        before = probe(chrome, output, 'before', env)
        report['probes'].append(before)
        save()
        if not before['passed']:
            cleanup = before.get('cleanup', {})
            if (not cleanup.get('direct_child_reaped') or cleanup.get('errors')
                    or before.get('profile_retained_after_unconfirmed_exit') or before.get('profile_cleanup_error')):
                raise RuntimeError('Failed Chrome preflight did not confirm owned-child/profile cleanup; refusing a second launch')
            stderr = (output / 'before-stderr.log').read_text(errors='replace')
            diagnosed = ('No usable sandbox!' in stderr or
                         'The SUID sandbox helper binary was found, but is not configured correctly' in stderr)
            if not diagnosed:
                raise RuntimeError('Chrome preflight failed without a recognized sandbox setup error; see preserved stderr')
            report['diagnosis'] = 'Chrome explicitly reported no usable or misconfigured sandbox; host policy is observed separately'
            destination = install_helper(chrome, version, output)
            report['installed_helper'] = file_info(destination)
            env['CHROME_DEVEL_SANDBOX'] = str(destination)
            save()
            after = probe(chrome, output, 'after-configuration', env)
            report['probes'].append(after)
            save()
            if not after['passed']:
                raise RuntimeError('Configured sandbox preflight failed; see preserved stderr')
            with open(os.environ['GITHUB_ENV'], 'a') as stream:
                stream.write(f'CHROME_DEVEL_SANDBOX={destination}\n')
        report['status'] = 'passed'
        save()
        return 0
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        report['status'] = 'failed'
        report['error'] = str(error)
        save()
        print(str(error), file=sys.stderr)
        return 1


if __name__ == '__main__':
    raise SystemExit(main())
