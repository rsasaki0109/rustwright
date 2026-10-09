#!/usr/bin/env python3
"""Run both reliability harnesses against the same local HTTP fixture."""

import argparse
import datetime
import hashlib
import http.server
import json
import math
import os
import platform
from pathlib import Path
import re
import signal
import statistics
import subprocess
import tempfile
import threading
import time
import uuid

from proc_memory import sample as sample_memory

ROOT = Path(__file__).resolve().parents[2]
SITE = Path(__file__).resolve().parent / "site"
CASES = ("delayed_click", "delayed_frame", "cross_site_frame", "cross_site_navigation", "disabled_click", "covered_click", "moving_click",
         "clipped_click", "rotated_clipped_click",
         "http_disconnect_recovery", "browser_disconnect", "mock_fetch", "mock_navigation",
         "mock_frame", "mock_return", "mock_clear")


def file_sha256(path):
    digest = hashlib.sha256()
    with Path(path).open('rb') as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b''):
            digest.update(chunk)
    return digest.hexdigest()


class Fixture(http.server.BaseHTTPRequestHandler):
    def setup(self):
        super().setup()
        self.connection.settimeout(5)

    def do_GET(self):
        path = self.path.split("?", 1)[0]
        if path == "/disconnect":
            # A real HTTP connection closed without a response, not a mocked
            # browser error or an HTTP 500 that navigation could accept.
            self.close_connection = True
            return
        if path not in ("/index.html", "/frame.html", "/swap-frame.html", "/network-frame.html", "/api/mock", "/actionability.js"):
            self.send_response(404)
            self.send_header("Content-Length", "0")
            self.end_headers()
            return
        body = b"network" if path == "/api/mock" else (SITE / path[1:]).read_bytes()
        self.send_response(200)
        self.send_header("Content-Type", "text/javascript; charset=utf-8" if path.endswith(".js") else "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        try:
            self.wfile.write(body)
        except (BrokenPipeError, ConnectionResetError):
            # Closing a page after a failed operation can cancel an in-flight
            # resource request. Harness postconditions still decide success.
            pass

    def log_message(self, *_args):
        pass


def percentile95(values):
    """Nearest-rank p95; no interpolation or invented tail samples."""
    return sorted(values)[math.ceil(0.95 * len(values)) - 1] if values else None


def summarize(records, cases=CASES):
    summaries = {}
    for case in cases:
        measured = [r for r in records if r["case"] == case and not r["warmup"]]
        successful = [r["elapsed_ms"] for r in measured if r["ok"]]
        summaries[case] = {
            "attempts": len(measured),
            "successes": len(successful),
            "success_rate": len(successful) / len(measured) if measured else None,
            "success_median_ms": statistics.median(successful) if successful else None,
            "success_p95_ms": percentile95(successful),
            "all_attempts_p95_ms": percentile95([r["elapsed_ms"] for r in measured]),
            "failures": [r["error"] for r in measured if not r["ok"]],
        }
    return summaries


def validate_partial(result, engine, samples, cases=CASES):
    """Accept only parseable, consistent rows; never invent missing attempts."""
    if result["engine"] != engine or not isinstance(result["browser"], str) or not result["browser"]:
        raise ValueError("unexpected engine or browser version")
    records = result["records"]
    if not isinstance(records, list):
        raise ValueError("records must be an array")
    seen = set()
    for row in records:
        if row["case"] not in cases or type(row["index"]) is not int or not 0 <= row["index"] <= samples:
            raise ValueError("unexpected case or sample index")
        key = (row["case"], row["index"])
        if key in seen:
            raise ValueError("duplicate sample")
        seen.add(key)
        if type(row["ok"]) is not bool or type(row["warmup"]) is not bool:
            raise ValueError("invalid outcome flags")
        if row["warmup"] != (row["index"] == 0):
            raise ValueError("incorrect warmup label")
        if type(row["elapsed_ms"]) not in (float, int) or not math.isfinite(row["elapsed_ms"]) or row["elapsed_ms"] < 0:
            raise ValueError("invalid elapsed time")
        if (row["error"] is None) != row["ok"] or (not row["ok"] and not isinstance(row["error"], str)):
            raise ValueError("error does not match outcome")


def validate(result, engine, samples, cases=CASES):
    validate_partial(result, engine, samples, cases)
    records = result["records"]
    if len(records) != len(cases) * (samples + 1):
        raise ValueError("missing or extra scenario records")
    for case in cases:
        rows = [r for r in records if r["case"] == case]
        if sorted(r["index"] for r in rows) != list(range(samples + 1)):
            raise ValueError(f"missing, duplicate or unexpected samples: {case}")


def stop_process(process, grace=2):
    """Terminate this command's owned session/group and finitely reap its child."""
    record = {"pid": process.pid, "errors": [], "direct_child_reaped": False}
    if os.name == 'posix':
        for sig in (signal.SIGTERM, signal.SIGKILL):
            try:
                os.killpg(process.pid, sig)
                record.setdefault('signals_sent', []).append(sig.name)
            except ProcessLookupError:
                break
            except OSError as error:
                record['errors'].append(str(error))
                break
            deadline = time.monotonic() + grace
            while time.monotonic() < deadline:
                try:
                    os.killpg(process.pid, 0)
                except ProcessLookupError:
                    break
                except OSError as error:
                    record['errors'].append(str(error))
                    break
                # Reap the direct child as soon as it exits; a zombie would
                # otherwise keep a completed group visible until the deadline.
                process.poll()
                time.sleep(0.02)
            try:
                os.killpg(process.pid, 0)
            except ProcessLookupError:
                break
            except OSError:
                break
        try:
            os.killpg(process.pid, 0)
            record['group_absent'] = False
        except ProcessLookupError:
            record['group_absent'] = True
        except OSError as error:
            record['errors'].append(str(error))
            record['group_absent'] = None
        if record['group_absent'] is False:
            record['errors'].append('owned process group still exists after TERM/KILL; descendant zombie reaping is outside direct-child ownership')
    else:
        # Windows has no killpg. A new process group plus taskkill's subtree
        # termination is the supported fallback, with its result retained.
        try:
            result = subprocess.run(['taskkill', '/PID', str(process.pid), '/T', '/F'],
                                    capture_output=True, text=True, timeout=grace)
            record['taskkill'] = {'exit_code': result.returncode, 'stdout': result.stdout, 'stderr': result.stderr}
            if result.returncode and process.poll() is None:
                record['errors'].append('taskkill could not terminate the owned command')
        except (OSError, subprocess.SubprocessError) as error:
            record['errors'].append(str(error))
    try:
        process.wait(timeout=grace)
        record['direct_child_reaped'] = True
    except (OSError, subprocess.SubprocessError) as error:
        record['errors'].append(str(error))
    return record


def run_measured(command, env, timeout, artifacts=None, observe_memory=True):
    """Capture raw bytes throughout execution, including every failed setup."""
    temporary = tempfile.TemporaryDirectory(prefix='rustwright-measure-') if artifacts is None else None
    directory = Path(temporary.name) if temporary else Path(artifacts)
    if temporary is None:
        directory.mkdir(parents=True, exist_ok=False)
    memory = []
    stop = threading.Event()
    process = observer = None
    error = None
    monitor_errors = []
    metadata = {'command': command, 'started_at_utc': datetime.datetime.now(datetime.timezone.utc).isoformat(),
                'timeout_seconds': timeout, 'stdout': str(directory / 'stdout.log'), 'stderr': str(directory / 'stderr.log'),
                'status': 'running', 'memory_samples': memory, 'observer_errors': monitor_errors}
    started = time.monotonic()
    with (directory / 'stdout.log').open('xb') as out, (directory / 'stderr.log').open('xb') as err:
        try:
            options = {'start_new_session': True} if os.name == 'posix' else {'creationflags': subprocess.CREATE_NEW_PROCESS_GROUP}
            process = subprocess.Popen(command, env=env, cwd=ROOT, stdout=out, stderr=err, **options)
            metadata['pid'] = process.pid

            def monitor():
                try:
                    if platform.system() != 'Linux' or not observe_memory:
                        return
                    while not stop.is_set():
                        observation = sample_memory(process.pid)
                        if observation is None:
                            break
                        memory.append(observation)
                        if stop.wait(1):
                            break
                except Exception as caught:
                    monitor_errors.append(f'{type(caught).__name__}: {caught}')

            observer = threading.Thread(target=monitor, daemon=True)
            observer.start()
            status = process.wait(timeout=timeout)
            if status:
                error = subprocess.CalledProcessError(status, command)
        except BaseException as caught:
            error = caught
        finally:
            if process is not None:
                metadata['exit_code'] = process.poll()
                metadata['cleanup'] = stop_process(process)
                metadata['exit_code_before_cleanup'] = metadata['exit_code']
                metadata['exit_code'] = process.returncode
            stop.set()
            if observer is not None and observer.ident is not None:
                observer.join(timeout=2)
                if observer.is_alive():
                    monitor_errors.append('memory observer did not finish within two seconds')
            metadata['duration_seconds'] = time.monotonic() - started
            metadata['finished_at_utc'] = datetime.datetime.now(datetime.timezone.utc).isoformat()
    stdout = (directory / 'stdout.log').read_text(errors='replace')
    stderr = (directory / 'stderr.log').read_text(errors='replace')
    if error is None and (monitor_errors or metadata.get('cleanup', {}).get('errors')):
        error = RuntimeError('memory observer or owned command cleanup failed; see process metadata')
    metadata['status'] = 'failed' if error is not None else 'passed'
    metadata['stdout_sha256'] = file_sha256(directory / 'stdout.log')
    metadata['stderr_sha256'] = file_sha256(directory / 'stderr.log')
    if error is not None:
        metadata['error'] = f'{type(error).__name__}: {error}'
        metadata['timed_out'] = isinstance(error, subprocess.TimeoutExpired)
        error.measurement = metadata
        error.stdout = stdout
        error.stderr = stderr
        if isinstance(error, (subprocess.CalledProcessError, subprocess.TimeoutExpired)):
            error.output = stdout
    (directory / 'process.json').write_text(json.dumps(metadata, indent=2) + '\n')
    if temporary:
        temporary.cleanup()
    if error is not None:
        raise error
    completed = subprocess.CompletedProcess(command, metadata['exit_code'], stdout, stderr)
    completed.measurement = metadata
    return completed, memory


class OwnedFixture:
    """Own the listening socket and serving thread even when thread start fails."""

    def __init__(self):
        self.server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Fixture)
        self.server.daemon_threads = True
        self.server.block_on_close = False
        self.server.timeout = 0.1
        self.stop = threading.Event()
        self.thread = None
        self.errors = []
        self.cleanup = None

    def __enter__(self):
        def serve():
            try:
                while not self.stop.is_set():
                    self.server.handle_request()
            except Exception as error:
                if not self.stop.is_set():
                    self.errors.append(f'{type(error).__name__}: {error}')
        try:
            self.thread = threading.Thread(target=serve, daemon=True)
            self.thread.start()
        except BaseException:
            self.close()
            raise
        return self.server

    def close(self):
        self.stop.set()
        try:
            if self.thread is not None and self.thread.ident is not None:
                self.thread.join(timeout=2)
        except RuntimeError as error:
            self.errors.append(f'fixture thread join failed: {error}')
        try:
            self.server.server_close()
        except OSError as error:
            self.errors.append(f'fixture socket close failed: {error}')
        self.cleanup = {'socket_closed': self.server.fileno() == -1,
                        'thread_finished': self.thread is None or not self.thread.is_alive(),
                        'errors': self.errors}
        if not self.cleanup['thread_finished']:
            self.errors.append('fixture serving thread did not finish within two seconds')

    def __exit__(self, kind, error, traceback):
        self.close()
        if error is not None:
            error.fixture_cleanup = self.cleanup
        elif self.errors:
            raise RuntimeError(f'fixture server failed: {self.errors}')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--chrome", required=True, type=Path)
    parser.add_argument("--samples", type=int, default=20, help="measured samples per engine and scenario")
    parser.add_argument("--pairs", type=int, default=2, help="alternate engine order across pairs")
    parser.add_argument("--cases", nargs="+", choices=CASES, default=list(CASES), help="scenarios to measure")
    parser.add_argument("--output", type=Path, default=ROOT / "target/reliability.json")
    parser.add_argument("--rust-binary", type=Path, default=ROOT / "target/release/examples/reliability")
    parser.add_argument("--require-success", action="store_true", help="exit nonzero if any measured operation fails")
    parser.add_argument("--require-engine-success", action="append", choices=("rustwright", "playwright-core"), default=[],
                        help="require this engine's measured operations to succeed; keep reference failures visible")
    args = parser.parse_args()
    if args.samples <= 0 or args.pairs <= 0 or args.samples % args.pairs:
        parser.error("samples must be positive and divisible by positive pairs")
    if len(args.cases) != len(set(args.cases)):
        parser.error("cases must be unique")
    if args.output.exists() or args.output.is_symlink():
        parser.error("output must be new; previous measurements are never overwritten")
    args.output = args.output.resolve()
    sidecars = args.output.with_name(args.output.name + '.artifacts')
    if sidecars.exists() or sidecars.is_symlink():
        parser.error("output sidecar directory must be new; previous evidence is never overwritten")
    args.output.parent.mkdir(parents=True, exist_ok=True)
    # Exclusively reserve both paths before any setup operation can fail.
    with args.output.open('x'):
        pass
    report = {
        "schema_version": 2,
        "run_id": str(uuid.uuid4()),
        "status": "running",
        "complete": False,
        "created_at_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "ci": {"run_id": os.environ.get('GITHUB_RUN_ID'), "run_attempt": os.environ.get('GITHUB_RUN_ATTEMPT'),
               "job": os.environ.get('GITHUB_JOB'), "matrix_host": os.environ.get('RUSTWRIGHT_BENCH_HOST_ID')},
        "platform": platform.platform(),
        "host_name": platform.node(),
        "artifacts": str(sidecars),
        "samples_per_engine_and_case": args.samples,
        "pairs": args.pairs,
        "cases": args.cases,
        "required_success_engines": ['rustwright', 'playwright-core'] if args.require_success else args.require_engine_success,
        "scenario_timeout_ms": 2000,
        "initial_navigation_timeout_ms": 10000,
        "fixture_delay_ms": 120,
        "frame_swap_delay_ms": 80,
        "memory_scope": "Linux /proc sampled once per second: driver separately from all descendants (browser processes and any harness helpers); RSS double-counts shared mappings; PSS is null if any process is unreadable. Sampling is not an atomic snapshot or an exact peak.",
        "limitations": [
            "Local deterministic Chromium scenarios do not establish broad site or cross-browser compatibility.",
            "Default launch flags and page/context creation differ between engines.",
            "Rustwright searches bounded quad points; Playwright retains default click selection without position or force overrides.",
            "Cross-site frame scenarios use 127.0.0.1/localhost and verify an iframe CDP target as a postcondition; site isolation remains enabled.",
            "Mock scenarios verify status/body for native fetch and XHR; mock_return verifies an OOPIF during untimed setup and times its return to the parent site.",
            "Route registration is timed in mock_fetch; mock_navigation/frame/return time with routes already installed; mock_clear times clearing and navigation.",
            "Timing excludes launch/navigation setup, includes postcondition checks and intentional delays.",
            "Browser disconnect uses graceful browser.close(), not a crash or forced TCP reset.",
            "p95 uses nearest rank from successful measured attempts; failures and all-attempt p95 are separate.",
            "Small sample sizes do not establish production tail latency or statistically significant superiority.",
            "The same external memory observer runs for both engines and can perturb timing; memory spans launch, setup, operations and shutdown, including temporary browsers in browser_disconnect.",
            "Owned command-group cleanup and direct-child reaping do not guarantee termination of descendants that create another session/group; OS kill/reap refusal is recorded as failure.",
        ],
        "runs": [],
        "phases": [],
    }
    fixture = None

    def save():
        args.output.write_text(json.dumps(report, indent=2) + '\n')

    def command(label, arguments, env=None, timeout=30, engine=None, pair=None, memory=False):
        phase = {'name': label, 'command': arguments, 'status': 'running', 'artifacts': str(sidecars / label)}
        if engine is not None:
            phase.update(engine=engine, pair=pair)
        report['phases'].append(phase)
        save()
        try:
            completed, observations = run_measured(arguments, env or dict(os.environ), timeout,
                                                   artifacts=sidecars / label, observe_memory=memory)
            phase.update(status='passed', process=completed.measurement)
            save()
            return completed, observations, phase
        except BaseException as error:
            phase.update(status='failed', error=f'{type(error).__name__}: {error}')
            if hasattr(error, 'measurement'):
                phase['process'] = error.measurement
            # Only a whole parseable, internally consistent partial JSON report
            # contributes rows. Malformed/truncated output remains raw evidence.
            if engine is not None and hasattr(error, 'stdout'):
                try:
                    partial = json.loads(error.stdout)
                    validate_partial(partial, engine, args.samples // args.pairs, args.cases)
                    partial.update(pair=pair, complete=False, memory_samples=phase.get('process', {}).get('memory_samples', []))
                    report['runs'].append(partial)
                    phase['valid_partial_records'] = len(partial['records'])
                except (ValueError, KeyError, TypeError):
                    phase['valid_partial_records'] = 0
            save()
            raise

    try:
        save()
        sidecars.mkdir()
        chrome = args.chrome.resolve(strict=True)
        rust_binary = args.rust_binary.resolve(strict=True)
        commands = {'rustwright': [str(rust_binary)], 'playwright-core': ['node', str(ROOT / 'bench/playwright/reliability.mjs')]}
        report['chrome_executable'] = str(chrome)
        report['chrome_binary_sha256'] = file_sha256(chrome)
        report['chrome_identity_scope'] = 'Selected executable bytes and --version; protocol versions checked for every phase'
        report['rust_binary_sha256'] = file_sha256(rust_binary)
        version, _, _ = command('setup-chrome-version', [str(chrome), '--version'])
        report['chrome_version_output'] = version.stdout.strip()
        match = re.search(r'(?<![\d.])(\d+\.\d+\.\d+\.\d+)(?![\d.])', version.stdout)
        if match is None:
            raise ValueError('selected browser did not report an exact four-component Chrome version')
        report['chrome_version'] = match[1]
        for key, arguments in [('commit', ['git', 'rev-parse', 'HEAD']), ('git_status', ['git', 'status', '--porcelain']),
                               ('rustc', ['rustc', '--version']), ('node', ['node', '--version'])]:
            result, _, _ = command('setup-' + key, arguments)
            report[key] = result.stdout.splitlines() if key == 'git_status' else result.stdout.strip()
        listed, _, _ = command('setup-source-files', ['git', 'ls-files', '-z', '--cached', '--others', '--exclude-standard'])
        files = [path for path in sorted(set(listed.stdout.split('\0'))) if path and (ROOT / path).is_file()]
        report['source_sha256'] = {path: file_sha256(ROOT / path) for path in files}
        report['playwright_core'] = json.loads((ROOT / 'bench/playwright/node_modules/playwright-core/package.json').read_text())['version']
        save()
        fixture = OwnedFixture()
        with fixture as server:
            env = dict(os.environ, RUSTWRIGHT_RELIABILITY_URL=f'http://127.0.0.1:{server.server_port}',
                       RUSTWRIGHT_BENCH_CHROME=str(chrome), RUSTWRIGHT_RELIABILITY_SAMPLES=str(args.samples // args.pairs),
                       RUSTWRIGHT_RELIABILITY_CASES=','.join(args.cases))
            report['fixture_url'] = env['RUSTWRIGHT_RELIABILITY_URL']
            for pair in range(args.pairs):
                order = list(commands) if pair % 2 == 0 else list(reversed(commands))
                for engine in order:
                    completed, memory, phase = command(f'pair-{pair + 1}-{engine}', commands[engine], env,
                        timeout=60 + (args.samples // args.pairs + 1) * len(args.cases) * 4,
                        engine=engine, pair=pair + 1, memory=True)
                    try:
                        result = json.loads(completed.stdout)
                        validate_partial(result, engine, args.samples // args.pairs, args.cases)
                        result.update(pair=pair + 1, complete=False, memory_samples=memory)
                        report['runs'].append(result)
                        phase['valid_partial_records'] = len(result['records'])
                        validate(result, engine, args.samples // args.pairs, args.cases)
                        if result['browser'].removeprefix('HeadlessChrome/').removeprefix('Chrome/') != report['chrome_version']:
                            raise ValueError('protocol browser version differs from the selected executable --version')
                        if not isinstance(result.get('launch_policy'), dict) or not result['launch_policy']:
                            raise ValueError('harness omitted self-reported launch_policy metadata')
                        result['complete'] = True
                        phase['status'] = 'complete'
                    except (ValueError, KeyError, TypeError) as error:
                        phase.update(status='failed', error=f'{type(error).__name__}: {error}')
                        save()
                        raise
                    save()
                    console_summary = {}
                    for case, summary in summarize(result["records"], args.cases).items():
                        console_summary[case] = {key: value for key, value in summary.items() if key != "failures"}
                        console_summary[case]["failure_count"] = len(summary["failures"])
                        console_summary[case]["first_failure"] = summary["failures"][0].splitlines()[0] if summary["failures"] else None
                    print(json.dumps({"pair": pair + 1, "engine": engine, "summary": console_summary}), flush=True)
        report['status'] = 'complete'
        report['complete'] = True
        required = set(report['required_success_engines'])
        report['success_requirement_met'] = not any(not row['ok'] for run in report['runs'] if run['engine'] in required
                                                   for row in run['records'] if not row['warmup'])
        save()
        if not report['success_requirement_met']:
            raise SystemExit('measured operation failures; see complete report')
    except BaseException as error:
        if not report['complete']:
            report.update(status='failed', complete=False, error=f'{type(error).__name__}: {error}')
        raise
    finally:
        if fixture is not None:
            report['fixture_cleanup'] = fixture.cleanup
        report['summary'] = {engine: summarize([row for run in report['runs'] if run['engine'] == engine for row in run['records']], args.cases)
                             for engine in ('rustwright', 'playwright-core')}
        report['finished_at_utc'] = datetime.datetime.now(datetime.timezone.utc).isoformat()
        save()
        print(f'Report: {args.output}', flush=True)


if __name__ == "__main__":
    main()
