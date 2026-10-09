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
import statistics
import subprocess
import threading

from proc_memory import sample as sample_memory

ROOT = Path(__file__).resolve().parents[2]
SITE = Path(__file__).resolve().parent / "site"
CASES = ("delayed_click", "delayed_frame", "cross_site_frame", "cross_site_navigation", "disabled_click", "covered_click", "moving_click",
         "clipped_click", "rotated_clipped_click",
         "http_disconnect_recovery", "browser_disconnect", "mock_fetch", "mock_navigation",
         "mock_frame", "mock_return", "mock_clear")


class Fixture(http.server.BaseHTTPRequestHandler):
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


def validate(result, engine, samples, cases=CASES):
    if result["engine"] != engine or not isinstance(result["browser"], str):
        raise ValueError("unexpected engine or browser version")
    records = result["records"]
    if len(records) != len(cases) * (samples + 1):
        raise ValueError("missing or extra scenario records")
    for case in cases:
        rows = [r for r in records if r["case"] == case]
        if sorted(r["index"] for r in rows) != list(range(samples + 1)):
            raise ValueError(f"missing, duplicate or unexpected samples: {case}")
        for row in rows:
            if type(row["ok"]) is not bool or type(row["warmup"]) is not bool:
                raise ValueError("invalid outcome flags")
            if row["warmup"] != (row["index"] == 0):
                raise ValueError("incorrect warmup label")
            if not isinstance(row["elapsed_ms"], (float, int)) or not math.isfinite(row["elapsed_ms"]) or row["elapsed_ms"] < 0:
                raise ValueError("invalid elapsed time")
            if (row["error"] is None) != row["ok"] or (not row["ok"] and not isinstance(row["error"], str)):
                raise ValueError("error does not match outcome")


def run_measured(command, env, timeout):
    memory = []
    stop = threading.Event()
    with subprocess.Popen(command, env=env, cwd=ROOT, stdout=subprocess.PIPE,
                          stderr=subprocess.PIPE, text=True) as process:
        def monitor():
            if platform.system() != 'Linux':
                return
            while not stop.is_set():
                observation = sample_memory(process.pid)
                if observation is None:
                    break
                memory.append(observation)
                if stop.wait(1):
                    break
        observer = threading.Thread(target=monitor)
        observer.start()
        try:
            stdout, stderr = process.communicate(timeout=timeout)
            if process.returncode:
                raise subprocess.CalledProcessError(process.returncode, command, stdout, stderr)
            return subprocess.CompletedProcess(command, process.returncode, stdout, stderr), memory
        except subprocess.TimeoutExpired:
            process.kill()
            process.communicate()
            raise
        finally:
            stop.set()
            observer.join()


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
    if args.output.exists():
        parser.error("output must be new; previous measurements are never overwritten")
    chrome = args.chrome.resolve(strict=True)
    commands = {
        "rustwright": [str(args.rust_binary.resolve(strict=True))],
        "playwright-core": ["node", str(ROOT / "bench/playwright/reliability.mjs")],
    }
    listed = subprocess.check_output(["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard"], cwd=ROOT).split(b'\0')
    files = [os.fsdecode(path) for path in sorted(set(listed)) if path and (ROOT / os.fsdecode(path)).is_file()]
    report = {
        "schema_version": 1,
        "created_at_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
        "git_status": subprocess.check_output(["git", "status", "--porcelain"], cwd=ROOT, text=True).splitlines(),
        "rustc": subprocess.check_output(["rustc", "--version"], text=True).strip(),
        "platform": platform.platform(),
        "source_sha256": {path: hashlib.sha256((ROOT / path).read_bytes()).hexdigest() for path in files},
        "rust_binary_sha256": hashlib.sha256(args.rust_binary.read_bytes()).hexdigest(),
        "chrome_executable": str(chrome),
        "node": subprocess.check_output(["node", "--version"], text=True).strip(),
        "playwright_core": json.loads((ROOT / "bench/playwright/node_modules/playwright-core/package.json").read_text())["version"],
        "samples_per_engine_and_case": args.samples,
        "pairs": args.pairs,
        "cases": args.cases,
        "required_success_engines": list(commands) if args.require_success else args.require_engine_success,
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
        ],
        "runs": [],
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with http.server.ThreadingHTTPServer(("127.0.0.1", 0), Fixture) as server:
        threading.Thread(target=server.serve_forever, daemon=True).start()
        env = dict(os.environ, RUSTWRIGHT_RELIABILITY_URL=f"http://127.0.0.1:{server.server_port}",
                   RUSTWRIGHT_BENCH_CHROME=str(chrome), RUSTWRIGHT_RELIABILITY_SAMPLES=str(args.samples // args.pairs),
                   RUSTWRIGHT_RELIABILITY_CASES=",".join(args.cases))
        try:
            for pair in range(args.pairs):
                order = list(commands) if pair % 2 == 0 else list(reversed(commands))
                for engine in order:
                    try:
                        completed, memory = run_measured(commands[engine], env,
                                                        timeout=60 + (args.samples // args.pairs + 1) * len(args.cases) * 4)
                    except subprocess.CalledProcessError as error:
                        raise RuntimeError(f"{engine} setup/harness failed: {error.stderr[-8000:]}") from error
                    result = json.loads(completed.stdout)
                    validate(result, engine, args.samples // args.pairs, args.cases)
                    result["pair"] = pair + 1
                    result["memory_samples"] = memory
                    report["runs"].append(result)
                    args.output.write_text(json.dumps(report, indent=2) + "\n")
                    console_summary = {}
                    for case, summary in summarize(result["records"], args.cases).items():
                        console_summary[case] = {key: value for key, value in summary.items() if key != "failures"}
                        console_summary[case]["failure_count"] = len(summary["failures"])
                        console_summary[case]["first_failure"] = summary["failures"][0].splitlines()[0] if summary["failures"] else None
                    print(json.dumps({"pair": pair + 1, "engine": engine, "summary": console_summary}), flush=True)
        finally:
            server.shutdown()
    versions = {run["browser"].removeprefix("Chrome/") for run in report["runs"]}
    if len(versions) != 1:
        raise ValueError(f"different browser versions: {versions}")
    report["summary"] = {
        engine: summarize([r for run in report["runs"] if run["engine"] == engine for r in run["records"]], args.cases)
        for engine in commands
    }
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(f"Report: {args.output}", flush=True)
    required = set(report["required_success_engines"])
    if any(not row["ok"] for run in report["runs"] if run["engine"] in required for row in run["records"] if not row["warmup"]):
        raise SystemExit("measured operation failures; see report")


if __name__ == "__main__":
    main()
