#!/usr/bin/env python3
"""Diagnose Linux Chrome launch and configure its bundled sandbox when required."""
from __future__ import annotations

import argparse
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import platform
import re
import signal
import shutil
import stat
import subprocess
import sys
import tempfile
import threading

CAPTURE_TIMEOUT_MS = 10000
PROBE_TIMEOUT_SECONDS = 25
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

    def log_message(self, *args) -> None:
        pass


def probe(chrome: Path, output: Path, phase: str, env: dict[str, str]) -> dict:
    server = ThreadingHTTPServer(('127.0.0.1', 0), FixtureHandler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    profile = Path(tempfile.mkdtemp(prefix=f'{phase}-profile-', dir=output))
    command = [str(chrome), '--no-first-run', '--no-default-browser-check',
               f'--user-data-dir={profile}', '--headless=new', '--disable-gpu',
               '--hide-scrollbars', '--mute-audio', '--dump-dom',
               # Supported by this Chrome version's headless command handler.
               # Bound DOM capture independently of waiting for a load event;
               # still require normal exit and the actual fixture DOM below.
               f'--timeout={CAPTURE_TIMEOUT_MS}',
               f'http://127.0.0.1:{server.server_port}/']
    stdout = output / f'{phase}-stdout.log'
    stderr = output / f'{phase}-stderr.log'
    timed_out = False
    try:
        with stdout.open('wb') as out, stderr.open('wb') as err:
            process = subprocess.Popen(command, env=env, stdin=subprocess.DEVNULL,
                                       stdout=out, stderr=err, start_new_session=True)
            try:
                status = process.wait(timeout=PROBE_TIMEOUT_SECONDS)
            except subprocess.TimeoutExpired:
                timed_out = True
                os.killpg(process.pid, signal.SIGTERM)
                try:
                    status = process.wait(timeout=2)
                except subprocess.TimeoutExpired:
                    os.killpg(process.pid, signal.SIGKILL)
                    status = process.wait(timeout=2)
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=2)
        # Retain command/stdout/stderr, not browser cache/profile contents.
        shutil.rmtree(profile, ignore_errors=True)
    text = stdout.read_text(errors='replace')
    error = stderr.read_text(errors='replace')
    record = {'phase': phase, 'command': command, 'exit_code': status,
              'timed_out': timed_out, 'stdout': str(stdout), 'stderr': str(stderr),
              'capture_timeout_ms': CAPTURE_TIMEOUT_MS,
              'outer_timeout_seconds': PROBE_TIMEOUT_SECONDS,
              'capture_scope': 'fixture DOM/title observed; internal capture can stop loading and does not prove full load-event completion',
              'document_rendered': FIXTURE_TITLE in text and FIXTURE_CONTENT in text}
    record['passed'] = status == 0 and not timed_out and record['document_rendered']
    # Preserve complete raw stderr in the artifact and a bounded console excerpt.
    print(f'Chrome {phase}: {json.dumps(record)}', flush=True)
    print(error[:12000], flush=True)
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
