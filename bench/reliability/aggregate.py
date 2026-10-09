#!/usr/bin/env python3
"""Validate three fresh CI reliability reports and emit descriptive comparisons.

No browser is launched. Raw records, failed attempts and per-host identities
remain in the JSON output. Three runner observations are not a population-level
independence or statistical-superiority claim.
"""
from __future__ import annotations

import argparse
import datetime
import hashlib
import json
import math
from pathlib import Path, PurePosixPath
import re
import statistics
import sys
import uuid

ROOT = Path(__file__).resolve().parents[2]
CASES = ("delayed_click", "delayed_frame", "cross_site_frame", "cross_site_navigation",
         "disabled_click", "covered_click", "moving_click", "clipped_click",
         "rotated_clipped_click", "http_disconnect_recovery", "browser_disconnect",
         "mock_fetch", "mock_navigation", "mock_frame", "mock_return", "mock_clear")
ENGINES = ("rustwright", "playwright-core")
ORDER = ((1, "rustwright"), (1, "playwright-core"),
         (2, "playwright-core"), (2, "rustwright"))
MEMORY_FIELDS = ("driver_rss_kib", "driver_pss_kib",
                 "descendant_rss_kib", "descendant_pss_kib")
FIXED_SOURCES = {
    "Cargo.toml", "Cargo.lock", "examples/Cargo.toml", "examples/reliability.rs",
    "bench/playwright/package.json", "bench/playwright/package-lock.json",
    "bench/playwright/reliability.mjs", "bench/playwright/launch_metadata.mjs", "bench/reliability/run.py",
    "bench/reliability/proc_memory.py", "scripts/ci/chrome_linux_sandbox.py",
}
LIMITATIONS = [
    "Three distinct CI matrix IDs/UUIDs record three job observations; they do not prove a statistically independent population or significant SOTA superiority.",
    "Pooled counts and nearest-rank p95 are raw descriptive summaries, not independent estimates, confidence intervals or cross-host latency inference.",
    "Successful median/p95 excludes failed attempts and warmups; reference-to-Rustwright p95 ratios appear only when both engines succeed on every measured attempt in that case/scope.",
    "All failure records and elapsed times are retained; all-attempt p95 is separate and is never used in a successful-latency advantage ratio.",
    "Linux memory is sampled driver-versus-descendants, not an atomic snapshot, exact peak, browser-only footprint or leak test; RSS double-counts shared mappings and missing PSS stays null.",
    "Memory spans launch, setup, measured operations and shutdown, including temporary browsers/helpers; monitoring can perturb timing.",
    "Source equality covers the declared measurement/build inputs, not whole-checkout identity; report hashes identify inputs but are not independent attestation of their contents.",
    "Launch flags, debugging port versus pipe, initial pages and driver implementation remain engine-specific; these local fixture observations do not establish broad sites, Firefox, headed or cross-platform superiority.",
]


def require(condition, message):
    if not condition:
        raise ValueError(message)


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def digest_valid(value):
    return isinstance(value, str) and re.fullmatch(r"[0-9a-f]{64}", value) is not None


def selected_source(path):
    """Explicit input rule: omit runtime results, docs and host filesystem IDs."""
    parts = PurePosixPath(path).parts
    return (path in FIXED_SOURCES
            or (len(parts) >= 3 and parts[0] == "crates"
                and (parts[-1] == "Cargo.toml" or ("src" in parts and path.endswith((".rs", ".js")))))
            or path.startswith("bench/reliability/site/")
            or path == ".github/workflows/comparison.yml")


def expected_source_paths():
    """Require the source inventory present beside this checkout's aggregator.

    Hash equality is between reports; this does not relabel a report as having
    run on the aggregator's current checkout. Inventory changes require using
    the matching evaluation tool/source rather than silently omitting inputs.
    """
    paths = set(FIXED_SOURCES)
    for base in (ROOT / "crates", ROOT / "bench/reliability/site"):
        for path in base.rglob("*"):
            if path.is_file() and selected_source(path.relative_to(ROOT).as_posix()):
                paths.add(path.relative_to(ROOT).as_posix())
    workflow = ROOT / ".github/workflows/comparison.yml"
    if workflow.is_file():
        paths.add(workflow.relative_to(ROOT).as_posix())
    return paths


def reject_nonfinite(value, path="report"):
    if isinstance(value, float):
        require(math.isfinite(value), f"{path}: nonfinite number")
    elif isinstance(value, dict):
        for key, item in value.items():
            reject_nonfinite(item, f"{path}.{key}")
    elif isinstance(value, list):
        for index, item in enumerate(value):
            reject_nonfinite(item, f"{path}[{index}]")


def finite_number(value):
    try:
        return type(value) in (int, float) and math.isfinite(value) and value >= 0
    except OverflowError:
        return False


def percentile95(values):
    return sorted(values)[math.ceil(.95 * len(values)) - 1] if values else None


def median(values):
    """Avoid overflowing a+b for valid finite nonnegative observations."""
    if not values:
        return None
    try:
        ordinary = statistics.median(values)
    except OverflowError:
        ordinary = math.inf
    if math.isfinite(ordinary):
        return ordinary
    ordered = sorted(values)
    midpoint = len(ordered) // 2
    if len(ordered) % 2:
        return ordered[midpoint]
    lower, upper = ordered[midpoint - 1:midpoint + 1]
    return lower + (upper - lower) / 2


def case_summary(rows):
    measured = [row for row in rows if not row["warmup"]]
    successful = [row["elapsed_ms"] for row in measured if row["ok"]]
    warmups = [row for row in rows if row["warmup"]]
    return {
        "attempts": len(measured), "successes": len(successful),
        "success_rate": len(successful) / len(measured) if measured else None,
        "success_median_ms": median(successful),
        "success_p95_ms": percentile95(successful),
        "all_attempts_p95_ms": percentile95([row["elapsed_ms"] for row in measured]),
        "failures": [row["error"] for row in measured if not row["ok"]],
        "warmup_attempts": len(warmups),
        "warmup_successes": sum(row["ok"] for row in warmups),
        "warmup_failures": [row["error"] for row in warmups if not row["ok"]],
    }


def summaries(runs):
    return {engine: {case: case_summary([row for run in runs if run["engine"] == engine
                                       for row in run["records"] if row["case"] == case])
                     for case in CASES} for engine in ENGINES}


def validate_rows(run, name):
    rows = run.get("records")
    require(isinstance(rows, list) and len(rows) == 16 * 51, f"{name}: missing/extra raw samples")
    seen = set()
    for row in rows:
        require(isinstance(row, dict), f"{name}: sample must be an object")
        case, index = row.get("case"), row.get("index")
        require(case in CASES and type(index) is int and 0 <= index <= 50,
                f"{name}: unknown case or invalid sample index")
        require((case, index) not in seen, f"{name}: duplicate case/index")
        seen.add((case, index))
        require(type(row.get("ok")) is bool and type(row.get("warmup")) is bool,
                f"{name}: nonboolean outcome/warmup")
        require(row["warmup"] == (index == 0), f"{name}: incorrect warmup label")
        require(finite_number(row.get("elapsed_ms")), f"{name}: invalid elapsed time")
        error = row.get("error")
        require((row["ok"] and error is None)
                or (not row["ok"] and isinstance(error, str) and bool(error.strip())),
                f"{name}: inconsistent failure/error")
    expected = {(case, index) for case in CASES for index in range(51)}
    require(seen == expected, f"{name}: missing phase samples")
    require([(row["case"], row["index"]) for row in rows]
            == [(case, index) for case in CASES for index in range(51)],
            f"{name}: raw case/sample order changed")


def validate_memory(run, name):
    samples = run.get("memory_samples")
    require(isinstance(samples, list) and bool(samples), f"{name}: missing Linux memory observations")
    previous = -1
    pid = None
    for row in samples:
        require(isinstance(row, dict), f"{name}: invalid memory sample")
        timestamp = row.get("monotonic_seconds")
        require(finite_number(timestamp) and timestamp > previous, f"{name}: invalid memory timestamp/order")
        previous = timestamp
        require(type(row.get("driver_pid")) is int and row["driver_pid"] > 0,
                f"{name}: invalid driver PID")
        pid = row["driver_pid"] if pid is None else pid
        require(row["driver_pid"] == pid, f"{name}: changed driver PID within phase")
        require(type(row.get("descendant_processes")) is int and row["descendant_processes"] >= 0,
                f"{name}: invalid descendant count")
        for field in (*MEMORY_FIELDS, "driver_hwm_kib"):
            require(field in row, f"{name}: missing memory field {field}")
            value = row[field]
            require(value is None or (type(value) is int and finite_number(value)), f"{name}: invalid memory {field}")
        require(row["driver_rss_kib"] is not None, f"{name}: missing driver RSS")


def memory_summary(run):
    rows = run["memory_samples"]
    return {"pair": run["pair"], "engine": run["engine"], "samples": len(rows),
            "max_sampled_kib": {field: max((row[field] for row in rows if row[field] is not None), default=None)
                                for field in MEMORY_FIELDS},
            "missing_samples": {field: sum(row[field] is None for row in rows) for field in MEMORY_FIELDS}}


def verify_summary(report, computed):
    supplied = report.get("summary")
    require(isinstance(supplied, dict) and set(supplied) == set(ENGINES), "missing/invalid engine summary")
    for engine in ENGINES:
        require(isinstance(supplied[engine], dict) and set(supplied[engine]) == set(CASES), "missing/invalid case summary")
        for case in CASES:
            original = supplied[engine][case]
            require(isinstance(original, dict), f"{engine}/{case}: invalid summary")
            for field in ("attempts", "successes", "success_rate", "success_median_ms",
                          "success_p95_ms", "all_attempts_p95_ms", "failures"):
                require(field in original and original[field] == computed[engine][case][field],
                        f"{engine}/{case}: summary disagrees with raw records ({field})")
                require(type(original[field]) is not bool, f"{engine}/{case}: boolean summary value")


def validate_report(report):
    require(isinstance(report, dict), "report must be an object")
    reject_nonfinite(report)
    require(type(report.get("schema_version")) is int and report["schema_version"] == 2, "fresh schema_version2 required")
    require(report.get("status") == "complete" and report.get("complete") is True, "incomplete/failed setup report")
    require("error" not in report, "complete report retains an outer setup error")
    require(type(report.get("success_requirement_met")) is bool, "missing success requirement outcome")
    require(report.get("required_success_engines") == ["rustwright"], "required-success policy must preserve reference failures")
    require(type(report.get("samples_per_engine_and_case")) is int and report["samples_per_engine_and_case"] == 100,
            "exactly100 measured samples required")
    require(type(report.get("pairs")) is int and report["pairs"] == 2, "exactly2 alternating pairs required")
    require(report.get("cases") == list(CASES), "fixed16 cases in canonical order required")
    for field, expected in (("scenario_timeout_ms", 2000), ("initial_navigation_timeout_ms", 10000),
                            ("fixture_delay_ms", 120), ("frame_swap_delay_ms", 80)):
        require(type(report.get(field)) is int and report[field] == expected, f"different fixture policy: {field}")
    require(isinstance(report.get("commit"), str) and re.fullmatch(r"[0-9a-f]{40}", report["commit"]), "invalid source commit")
    require(digest_valid(report.get("rust_binary_sha256")), "missing Rust driver identity")
    require(digest_valid(report.get("chrome_binary_sha256")), "missing browser binary identity")
    require(isinstance(report.get("chrome_version"), str)
            and re.fullmatch(r"\d+\.\d+\.\d+\.\d+", report["chrome_version"]), "exact browser version required")
    for field in ("platform", "host_name", "rustc", "node", "playwright_core", "chrome_executable", "memory_scope"):
        require(isinstance(report.get(field), str) and bool(report[field].strip()), f"missing {field}")
    require(report["platform"].startswith("Linux"), "this sampled-memory comparison requires Linux reports")
    require(re.fullmatch(r"\d+\.\d+\.\d+(?:[-+][A-Za-z0-9.-]+)?", report["playwright_core"]), "invalid Playwright version")
    try:
        require(str(uuid.UUID(report.get("run_id", ""))) == report["run_id"], "noncanonical run UUID")
        created = datetime.datetime.fromisoformat(report["created_at_utc"])
        require(created.tzinfo is not None and created.utcoffset() == datetime.timedelta(0), "creation time must be UTC")
    except (KeyError, TypeError, AttributeError, ValueError) as error:
        raise ValueError(f"invalid run UUID/UTC identity: {error}") from error
    ci = report.get("ci")
    require(isinstance(ci, dict), "CI identity required")
    for field in ("run_id", "run_attempt", "job", "matrix_host"):
        require(isinstance(ci.get(field), str) and bool(ci[field].strip()), f"missing CI {field}")
    require(ci["run_id"].isdecimal() and ci["run_attempt"].isdecimal(), "invalid Actions run identity")
    require(int(ci["run_id"]) > 0 and int(ci["run_attempt"]) > 0, "invalid Actions run/attempt number")
    require(re.fullmatch(r"[A-Za-z0-9_.-]+", ci["matrix_host"]), "invalid matrix host label")
    source = report.get("source_sha256")
    require(isinstance(source, dict), "source hashes required")
    for path, digest in source.items():
        require(isinstance(path, str) and not path.startswith("/") and "\\" not in path
                and all(part not in ("", ".", "..") for part in path.split("/")), "invalid source path")
        require(digest_valid(digest), f"invalid source hash: {path}")
    selected = {path: digest for path, digest in source.items() if selected_source(path)}
    require(expected_source_paths().issubset(selected), "missing measurement/build source inputs")
    dirty = report.get("git_status")
    require(isinstance(dirty, list) and all(isinstance(line, str) for line in dirty), "invalid Git status record")
    require(not any(selected_source(line[3:]) for line in dirty), "dirty measurement source checkout")
    runs = report.get("runs")
    require(isinstance(runs, list) and len(runs) == 4, "four complete raw phases required")
    for index, (run, (pair, engine)) in enumerate(zip(runs, ORDER)):
        require(isinstance(run, dict) and type(run.get("pair")) is int
                and run["pair"] == pair and run.get("engine") == engine, "incorrect alternating engine order")
        browser = run.get("browser")
        require(browser in (report["chrome_version"], "Chrome/" + report["chrome_version"],
                            "HeadlessChrome/" + report["chrome_version"]), "phase browser version differs")
        require(run.get("complete") is True, "incomplete raw engine phase")
        validate_rows(run, f"phase{index + 1}")
        validate_memory(run, f"phase{index + 1}")
    # Schema-specific phase and launch-policy checks are intentionally strict.
    validate_phases(report)
    computed = summaries(runs)
    verify_summary(report, computed)
    success_met = all(row["ok"] for run in runs if run["engine"] == "rustwright"
                      for row in run["records"] if not row["warmup"])
    require(report["success_requirement_met"] == success_met, "required-success outcome disagrees with raw data")
    return {"summary": computed, "memory": [memory_summary(run) for run in runs],
            "source": selected, "success_requirement_met": success_met}


def validate_phases(report):
    setup = ["setup-chrome-version", "setup-commit", "setup-git_status",
             "setup-rustc", "setup-node", "setup-source-files"]
    phases = report.get("phases")
    require(isinstance(phases, list) and len(phases) == 10, "six setup and four complete measurement phases required")
    expected_names = setup + [f"pair-{pair}-{engine}" for pair, engine in ORDER]
    require(all(isinstance(phase, dict) for phase in phases)
            and [phase.get("name") for phase in phases] == expected_names, "missing/duplicate/reordered setup or measurement phase")
    for index, phase in enumerate(phases):
        harness = index >= 6
        require(phase.get("status") == ("complete" if harness else "passed"), "failed/incomplete phase status")
        require("error" not in phase, "phase retains setup/measurement error")
        command = phase.get("command")
        require(isinstance(command, list) and bool(command)
                and all(isinstance(arg, str) and bool(arg) for arg in command), "phase command identity required")
        process = phase.get("process")
        require(isinstance(process, dict) and process.get("status") == "passed", "failed/missing process outcome")
        require(type(process.get("pid")) is int and process["pid"] > 0, "owned process PID required")
        require(process.get("command") == command, "process command differs from phase")
        require(digest_valid(process.get("stdout_sha256")) and digest_valid(process.get("stderr_sha256")),
                "raw process-log identities required")
        require(type(process.get("exit_code")) is int and process["exit_code"] == 0
                and type(process.get("exit_code_before_cleanup")) is int and process["exit_code_before_cleanup"] == 0,
                "forced cleanup or failed process cannot be complete")
        require(process.get("observer_errors") == [] and "error" not in process and not process.get("timed_out"),
                "observer/process error cannot be complete")
        cleanup = process.get("cleanup")
        require(isinstance(cleanup, dict) and cleanup.get("direct_child_reaped") is True and cleanup.get("errors") == [],
                "owned direct-child cleanup not confirmed")
        require(cleanup.get("group_absent") is True, "owned Linux process group retirement unconfirmed")
        if not harness:
            continue
        run = report["runs"][index - 6]
        require(phase.get("engine") == run["engine"] and type(phase.get("pair")) is int
                and phase["pair"] == run["pair"], "phase/run engine-pair mismatch")
        require(type(phase.get("valid_partial_records")) is int and phase["valid_partial_records"] == 816,
                "phase raw-record census differs")
        require(process.get("memory_samples") == run["memory_samples"], "phase/raw memory records disagree")
        require(all(row["driver_pid"] == process["pid"] for row in run["memory_samples"]),
                "sampled memory does not belong to the measured driver")
        policy = run.get("launch_policy")
        require(isinstance(policy, dict) and policy.get("headless") is True and policy.get("sandbox") is True,
                "explicit headless/sandbox policy required")
        viewport = policy.get("viewport")
        require(isinstance(viewport, dict) and viewport.get("mobile") is False, "desktop viewport required")
        for field, expected in (("width", 1280), ("height", 720), ("device_scale_factor", 1)):
            require(type(viewport.get(field)) is int and viewport[field] == expected, "viewport policy mismatch")
        require(type(policy.get("viewport_checks")) is int and policy["viewport_checks"] == 816,
                "viewport not checked on every setup page")
        require(policy.get("context_policy") == "reused main browser context; fresh browser/context for disconnect",
                "different context reuse policy")
        require(policy.get("driver_defaults_differ") is True, "driver defaults must remain explicit")
        expected_transport = "localhost CDP port" if run["engine"] == "rustwright" else "CDP pipe"
        require(policy.get("transport") == expected_transport, "different driver transport policy")
        arguments = policy.get("browser_launch_args")
        require(isinstance(arguments, list) and bool(arguments)
                and all(isinstance(arg, str) and bool(arg) for arg in arguments), "actual browser command line required")
        forbidden = ("--no-sandbox", "--disable-setuid-sandbox", "--disable-web-security", "--ignore-certificate-errors")
        require(not any(arg.split("=", 1)[0] in forbidden for arg in arguments), "sandbox/TLS/security bypass in launch arguments")
        require(any(arg.split("=", 1)[0] == "--headless" for arg in arguments) and "--disable-gpu" in arguments,
                "shared headless/GPU launch policy not reflected in actual arguments")
        transport_flag = "--remote-debugging-port" if run["engine"] == "rustwright" else "--remote-debugging-pipe"
        require(any(arg.split("=", 1)[0] == transport_flag for arg in arguments), "actual debugging transport differs")
    fixture = report.get("fixture_cleanup")
    require(isinstance(fixture, dict) and fixture.get("socket_closed") is True
            and fixture.get("thread_finished") is True and fixture.get("errors") == [], "fixture cleanup unconfirmed")


def aggregate(inputs):
    result = {"schema_version": 1, "status": "invalid", "errors": [],
              "created_at_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
              "aggregator_sha256": sha256(Path(__file__).read_bytes()),
              "aggregation_tests_sha256": sha256(Path(__file__).with_name("test_aggregate.py").read_bytes()),
              "limitations": LIMITATIONS, "inputs": inputs, "hosts": []}
    if len(inputs) != 3:
        result["errors"].append("exactly three distinct CI reports required")
    seen_hashes, seen_runs, seen_hosts = set(), set(), set()
    reference = None
    for index, item in enumerate(inputs):
        name = item.get("path", f"input{index + 1}")
        try:
            require("load_error" not in item, item.get("load_error", "JSON load failed"))
            require(item.get("raw_artifacts_verified") is True, "raw process artifacts were not verified")
            require(item["sha256"] not in seen_hashes, "duplicate report bytes")
            seen_hashes.add(item["sha256"])
            report = item["report"]
            checked = validate_report(report)
            require(report["run_id"] not in seen_runs, "duplicate run UUID")
            seen_runs.add(report["run_id"])
            require(report["ci"]["matrix_host"] not in seen_hosts, "duplicate CI matrix host")
            seen_hosts.add(report["ci"]["matrix_host"])
            stable = {field: report[field] for field in ("commit", "chrome_version", "chrome_binary_sha256",
                                                       "playwright_core", "rustc", "node")}
            stable["source_sha256"] = checked["source"]
            stable["measurement_harness_sha256"] = {path: checked["source"][path] for path in (
                "examples/reliability.rs", "bench/playwright/reliability.mjs", "bench/playwright/launch_metadata.mjs",
                "bench/reliability/run.py", "bench/reliability/proc_memory.py")}
            stable["ci_run"] = {field: report["ci"][field] for field in ("run_id", "run_attempt", "job")}
            if reference is None:
                reference = stable
            else:
                require(stable == reference, "mixed source/browser/toolchain/CI-run identity")
            result["hosts"].append({"matrix_host": report["ci"]["matrix_host"], "run_id": report["run_id"],
                                    "input_path": name, "summary": checked["summary"], "memory": checked["memory"],
                                    "success_requirement_met": checked["success_requirement_met"]})
        except (ValueError, TypeError, KeyError) as error:
            result["errors"].append(f"{name}: {error}")
    if result["errors"]:
        return result
    result["common_identity"] = reference
    result["pooled_raw_descriptive"] = summaries([run for item in inputs for run in item["report"]["runs"]])
    result["measured_failures"] = {engine: sum(len(host["summary"][engine][case]["failures"])
                                               for host in result["hosts"] for case in CASES) for engine in ENGINES}
    result["status"] = "passed" if all(host["success_requirement_met"] for host in result["hosts"]) else "failed"
    result["evaluation_scope"] = "Validated complete fixture observations; Rustwright measured success is required. Reference failures are retained, not an invalid setup or suppressed outcome."
    return result


def format_number(value):
    return "—" if value is None else f"{value:.2f}"


def performance_table(summary):
    lines = ["| Scenario | Rustwright success | Playwright success | Rust median/p95 ms | Playwright median/p95 ms | Reference/Rust successful p95 |",
             "| --- | ---: | ---: | ---: | ---: | ---: |"]
    for case in CASES:
        left, right = (summary[engine][case] for engine in ENGINES)
        lp, rp = left["success_p95_ms"], right["success_p95_ms"]
        complete_success = all(row["attempts"] > 0 and row["successes"] == row["attempts"] for row in (left, right))
        ratio = rp / lp if complete_success and lp is not None and lp > 0 and rp is not None else None
        if ratio is not None and not math.isfinite(ratio):
            ratio = None
        lines.append(f"| {case} | {left['successes']}/{left['attempts']} | {right['successes']}/{right['attempts']} | "
                     f"{format_number(left['success_median_ms'])}/{format_number(lp)} | "
                     f"{format_number(right['success_median_ms'])}/{format_number(rp)} | {format_number(ratio)} |")
    return lines


def markdown(result):
    lines = ["# Three CI reliability observations", "", f"Status: **{result['status']}**.", "",
             "Status describes validation and the required Rustwright measured-success policy, not an assertion that the reference had no failures or that Rustwright was faster.", "",
             "Successful latency excludes warmups and failed attempts. Ratios appear only for cases/scopes with every measured attempt successful in both engines; they do not establish significant or general SOTA superiority.", ""]
    if result["errors"]:
        lines.extend(["Validation errors:", ""] + ["- " + error.replace("\n", " ").replace("|", "\\|") for error in result["errors"]] + [""])
    for host in result["hosts"]:
        warmups = {engine: (sum(row["warmup_successes"] for row in host["summary"][engine].values()),
                            sum(row["warmup_attempts"] for row in host["summary"][engine].values())) for engine in ENGINES}
        lines.extend([f"## Host {host['matrix_host']}", "",
                      f"Excluded warmup successes: Rustwright {warmups['rustwright'][0]}/{warmups['rustwright'][1]}; Playwright {warmups['playwright-core'][0]}/{warmups['playwright-core'][1]}.", "",
                      *performance_table(host["summary"]), ""])
    if "pooled_raw_descriptive" in result:
        lines.extend(["## Pooled raw descriptive results", "", "These pool the recorded attempts; they are not independent population estimates.", "",
                      *performance_table(result["pooled_raw_descriptive"]), ""])
    lines.extend(["## Separate sampled memory", "", "Each field has its own maximum in MiB; maxima need not be simultaneous. Missing readings remain unavailable, with counts below.", "",
                  "| Host | Pair | Engine | Samples | Driver RSS | Driver PSS | Descendant RSS | Descendant PSS | Missing RSS/PSS counts (driver; descendants) |",
                  "| --- | ---: | --- | ---: | ---: | ---: | ---: | ---: | --- |"])
    for host in result["hosts"]:
        for row in host["memory"]:
            values = [format_number(row["max_sampled_kib"][field] / 1024 if row["max_sampled_kib"][field] is not None else None) for field in MEMORY_FIELDS]
            missing = [str(row["missing_samples"][field]) for field in MEMORY_FIELDS]
            lines.append(f"| {host['matrix_host']} | {row['pair']} | {row['engine']} | {row['samples']} | "
                         + " | ".join(values) + f" | {missing[0]}/{missing[1]}; {missing[2]}/{missing[3]} |")
    lines.extend(["", "## Limits and provenance", "", *["- " + item for item in result["limitations"]], "",
                  "Input file hashes, raw host reports (including failed samples), source hashes, launch policies and aggregate-tool identities remain in the JSON artifact.", ""])
    return "\n".join(lines)


def strict_json(data):
    def pairs(entries):
        result = {}
        for key, value in entries:
            require(key not in result, f"duplicate JSON key: {key}")
            result[key] = value
        return result
    def constant(value):
        raise ValueError(f"nonfinite JSON constant: {value}")
    def floating(value):
        number = float(value)
        require(math.isfinite(number), "nonfinite/overflowed JSON number")
        return number
    return json.loads(data, object_pairs_hook=pairs, parse_constant=constant, parse_float=floating)


def verify_artifacts(path, report):
    """Rebase archived sidecars; never follow the original runner's absolute paths."""
    root = path.with_name(path.name + ".artifacts")
    artifacts = []
    runs = report.get("runs", [])
    for phase in report.get("phases", []):
        require(isinstance(phase, dict) and isinstance(phase.get("name"), str)
                and re.fullmatch(r"[A-Za-z0-9_-]+", phase["name"]), "invalid artifact phase name")
        process = phase.get("process", {})
        metadata_path = root / phase["name"] / "process.json"
        require(metadata_path.resolve().is_relative_to(root.resolve()), "process metadata escapes sidecar directory")
        metadata_bytes = metadata_path.read_bytes()
        require(strict_json(metadata_bytes) == process, "raw process metadata differs from report phase")
        artifacts.append({"path": str(metadata_path), "sha256": sha256(metadata_bytes), "bytes": len(metadata_bytes)})
        stdout = None
        for name in ("stdout", "stderr"):
            local = root / phase["name"] / (name + ".log")
            require(local.resolve().is_relative_to(root.resolve()), "artifact escapes downloaded sidecar directory")
            data = local.read_bytes()
            observed = sha256(data)
            require(observed == process.get(name + "_sha256"), f"raw {phase['name']}/{name} hash mismatch")
            artifacts.append({"path": str(local), "sha256": observed, "bytes": len(data)})
            if name == "stdout":
                stdout = data
        if "engine" in phase:
            raw = strict_json(stdout)
            matching = [run for run in runs if run.get("engine") == phase["engine"] and run.get("pair") == phase.get("pair")]
            require(len(matching) == 1, "raw artifact has missing/duplicate corresponding run")
            require(isinstance(raw, dict), "raw harness stdout is not an object")
            for field in ("engine", "browser", "records", "launch_policy"):
                require(field in raw and raw[field] == matching[0].get(field), f"raw harness stdout differs: {field}")
        else:
            text = stdout.decode("utf-8").strip()
            field = phase["name"].removeprefix("setup-")
            if field in ("commit", "rustc", "node"):
                require(text == report.get(field), f"raw setup {field} differs")
            elif field == "git_status":
                require(stdout.decode("utf-8").splitlines() == report.get(field), "raw Git status differs")
            elif field == "chrome-version":
                require(text == report.get("chrome_version_output"), "raw CLI version output differs")
                match = re.search(r"(?<![\d.])(\d+\.\d+\.\d+\.\d+)(?![\d.])", text)
                require(match is not None and match[1] == report.get("chrome_version"), "raw CLI browser version differs")
            elif field == "source-files":
                listed = {name for name in stdout.decode("utf-8").split("\0") if name and selected_source(name)}
                selected = {name for name in report.get("source_sha256", {}) if selected_source(name)}
                require(listed == selected, "raw Git inventory differs from measurement source map")
    return artifacts


def load_input(path):
    item = {"path": str(path)}
    try:
        data = path.read_bytes()
        item.update(sha256=sha256(data), bytes=len(data))
        report = strict_json(data)
        item["report"] = report
        # Validate structure before resolving sidecars from its phase metadata.
        validate_report(report)
        item["raw_artifacts"] = verify_artifacts(path, report)
        item["raw_artifacts_verified"] = True
    except (OSError, UnicodeError, ValueError, TypeError, KeyError) as error:
        item["load_error"] = str(error)
    return item


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--reports", nargs="+", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--markdown", required=True, type=Path)
    args = parser.parse_args(argv)
    outputs = (args.output, args.markdown)
    if args.output.resolve() == args.markdown.resolve() or any(path.exists() for path in outputs):
        parser.error("both output paths must be distinct and new; existing evidence is never overwritten")
    result = aggregate([load_input(path) for path in args.reports])
    for path in outputs:
        path.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("x") as stream:
        json.dump(result, stream, indent=2, allow_nan=False)
        stream.write("\n")
    with args.markdown.open("x") as stream:
        stream.write(markdown(result))
    print(f"Comparison {result['status']}: {args.output}")
    return 0 if result["status"] == "passed" else 1


if __name__ == "__main__":
    sys.exit(main())
