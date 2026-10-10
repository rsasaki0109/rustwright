"""Synthetic controls for comparison validation, without native browsers."""
import copy
import json
from pathlib import Path
import tempfile
import unittest

import aggregate as subject


SUMMARY_FIELDS = ("attempts", "successes", "success_rate", "success_median_ms",
                  "success_p95_ms", "all_attempts_p95_ms", "failures")


def refresh(report):
    computed = subject.summaries(report["runs"])
    report["summary"] = {engine: {case: {key: values[key] for key in SUMMARY_FIELDS}
                                   for case, values in cases.items()} for engine, cases in computed.items()}
    report["success_requirement_met"] = all(row["ok"] for run in report["runs"]
                                            if run["engine"] == "rustwright"
                                            for row in run["records"] if not row["warmup"])
    for phase, run in zip(report["phases"][6:], report["runs"]):
        phase["process"]["memory_samples"] = copy.deepcopy(run["memory_samples"])


def report(host):
    result = {
        "schema_version": 2, "status": "complete", "complete": True,
        "run_id": f"00000000-0000-4000-8000-{host:012d}",
        "created_at_utc": f"2026-10-10T00:00:0{host}+00:00",
        "ci": {"run_id": "987654", "run_attempt": "1", "job": "comparison", "matrix_host": f"host-{host}"},
        "host_name": f"runner-{host}", "platform": "Linux-controlled-test", "commit": "a" * 40,
        "git_status": [], "source_sha256": {name: "1" * 64 for name in subject.expected_source_paths()},
        "rust_binary_sha256": str(host) * 64, "chrome_binary_sha256": "c" * 64,
        "chrome_version": "155.0.8059.39", "chrome_version_output": "Google Chrome for Testing 155.0.8059.39",
        "chrome_executable": f"/host-{host}/chrome", "playwright_core": "1.63.0",
        "rustc": "rustc 1.99.0", "node": "v24.19.0", "samples_per_engine_and_case": 100,
        "pairs": 2, "cases": list(subject.CASES), "required_success_engines": ["rustwright"],
        "scenario_timeout_ms": 2000, "initial_navigation_timeout_ms": 10000,
        "fixture_delay_ms": 120, "frame_swap_delay_ms": 80, "memory_scope": "Linux synthetic samples",
        "runs": [], "phases": [], "fixture_cleanup": {"socket_closed": True, "thread_finished": True, "errors": []},
    }
    setup = ("chrome-version", "commit", "git_status", "rustc", "node", "source-files")
    for name in setup:
        result["phases"].append({"name": "setup-" + name, "status": "passed", "command": ["controlled", name]})
    for ordinal, (pair, engine) in enumerate(subject.ORDER):
        memory = [{"monotonic_seconds": 10.0 + ordinal, "driver_pid": 123 + ordinal,
                   "driver_rss_kib": 1024, "driver_hwm_kib": 2048, "driver_pss_kib": 512,
                   "descendant_processes": 2, "descendant_rss_kib": 4096, "descendant_pss_kib": 2048}]
        run = {"engine": engine, "pair": pair, "browser": "Chrome/155.0.8059.39" if engine == "rustwright" else "155.0.8059.39",
               "complete": True, "records": [{"case": case, "index": index, "warmup": index == 0,
                                              "ok": True, "error": None,
                                              "elapsed_ms": float(index + host + (100 if engine == "playwright-core" else 50))}
                                             for case in subject.CASES for index in range(51)],
               "memory_samples": memory,
               "launch_policy": {"headless": True, "sandbox": True,
                                 "viewport": {"width": 1280, "height": 720, "device_scale_factor": 1, "mobile": False},
                                 "context_policy": "reused main browser context; fresh browser/context for disconnect",
                                 "driver_defaults_differ": True,
                                 "transport": "localhost CDP port" if engine == "rustwright" else "CDP pipe",
                                 "browser_launch_args": ["chrome", "--headless=new", "--disable-gpu",
                                                         "--remote-debugging-port=0" if engine == "rustwright" else "--remote-debugging-pipe"],
                                 "viewport_checks": 816}}
        result["runs"].append(run)
        result["phases"].append({"name": f"pair-{pair}-{engine}", "status": "complete", "engine": engine,
                                 "pair": pair, "valid_partial_records": 816, "command": ["controlled", engine]})
    for index, phase in enumerate(result["phases"]):
        phase["process"] = {"command": phase["command"], "status": "passed", "exit_code": 0,
                            "pid": 200 + index if index < 6 else 123 + index - 6,
                            "exit_code_before_cleanup": 0, "observer_errors": [], "memory_samples": [],
                            "stdout_sha256": "1" * 64, "stderr_sha256": "2" * 64,
                            "cleanup": {"direct_child_reaped": True, "group_absent": True, "errors": []}}
    refresh(result)
    return result


def inputs(reports):
    return [{"path": f"host-{index + 1}/report.json", "sha256": subject.sha256(json.dumps(r, sort_keys=True).encode()),
             "bytes": len(json.dumps(r).encode()), "report": r, "raw_artifacts_verified": True}
            for index, r in enumerate(reports)]


def failure(row, message="controlled operation failure", elapsed=.1):
    row.update(ok=False, error=message, elapsed_ms=elapsed)


def write_report(root, value):
    path = root / "report.json"
    root.mkdir(parents=True)
    for phase in value["phases"]:
        directory = root / "report.json.artifacts" / phase["name"]
        directory.mkdir(parents=True)
        if "engine" in phase:
            run = next(r for r in value["runs"] if r["engine"] == phase["engine"] and r["pair"] == phase["pair"])
            stdout = json.dumps({key: run[key] for key in ("engine", "browser", "records", "launch_policy")}).encode()
        else:
            name = phase["name"].removeprefix("setup-")
            if name == "chrome-version":
                stdout = value["chrome_version_output"].encode() + b"\n"
            elif name == "source-files":
                stdout = ("\0".join(sorted(value["source_sha256"])) + "\0").encode()
            elif name == "git_status":
                stdout = "\n".join(value["git_status"]).encode()
            else:
                stdout = value[name].encode() + b"\n"
        for name, data in (("stdout", stdout), ("stderr", b"")):
            (directory / (name + ".log")).write_bytes(data)
            phase["process"][name + "_sha256"] = subject.sha256(data)
        (directory / "process.json").write_text(json.dumps(phase["process"]))
    path.write_text(json.dumps(value))
    return path


class AggregateTests(unittest.TestCase):
    def setUp(self):
        self.reports = [report(host) for host in (1, 2, 3)]

    def result(self):
        return subject.aggregate(inputs(self.reports))

    def reject(self):
        result = self.result()
        self.assertEqual(result["status"], "invalid", result.get("errors"))
        self.assertTrue(result["errors"])
        return result

    def test_complete_three_jobs_recompute_counts_and_keep_individual_records(self):
        result = self.result()
        self.assertEqual(result["status"], "passed", result["errors"])
        self.assertEqual(len(result["hosts"]), 3)
        pooled = result["pooled_raw_descriptive"]["rustwright"]["delayed_click"]
        self.assertEqual((pooled["attempts"], pooled["successes"], pooled["warmup_attempts"]), (300, 300, 6))
        self.assertEqual(result["inputs"][0]["report"]["runs"][0]["records"], self.reports[0]["runs"][0]["records"])
        self.assertIn("not independent population estimates", subject.markdown(result))

    def test_reference_failures_remain_data_and_have_no_latency_ratio(self):
        for value in self.reports:
            for run in value["runs"]:
                if run["engine"] == "playwright-core":
                    for row in run["records"]:
                        if row["case"] == "mock_return":
                            failure(row)
            refresh(value)
        result = self.result()
        self.assertEqual(result["status"], "passed", result["errors"])
        self.assertEqual(result["measured_failures"]["playwright-core"], 300)
        case = result["pooled_raw_descriptive"]["playwright-core"]["mock_return"]
        self.assertIsNone(case["success_p95_ms"])
        self.assertEqual(case["warmup_failures"], ["controlled operation failure"] * 6)
        line = next(line for line in subject.performance_table(result["hosts"][0]["summary"]) if line.startswith("| mock_return"))
        self.assertTrue(line.endswith("| — |"))

    def test_rust_failure_is_complete_failed_evaluation_with_raw_data_retained(self):
        failure(self.reports[0]["runs"][0]["records"][1])
        refresh(self.reports[0])
        result = self.result()
        self.assertEqual(result["status"], "failed", result["errors"])
        self.assertEqual(result["measured_failures"]["rustwright"], 1)
        self.assertEqual(result["inputs"][0]["report"]["status"], "complete")
        self.assertEqual(result["hosts"][0]["summary"]["rustwright"]["delayed_click"]["successes"], 99)

    def test_fast_reference_failure_suppresses_ratio_even_with_other_successes(self):
        failure(self.reports[0]["runs"][1]["records"][1], elapsed=.001)
        refresh(self.reports[0])
        result = self.result()
        summary = result["hosts"][0]["summary"]
        self.assertGreater(summary["playwright-core"]["delayed_click"]["success_median_ms"], 100)
        line = next(line for line in subject.performance_table(summary) if line.startswith("| delayed_click"))
        self.assertTrue(line.endswith("| — |"))

    def test_warmup_failure_is_separate_from_measured_requirement(self):
        failure(self.reports[0]["runs"][0]["records"][0], elapsed=100000)
        refresh(self.reports[0])
        result = self.result()
        self.assertEqual(result["status"], "passed", result["errors"])
        summary = result["hosts"][0]["summary"]["rustwright"]["delayed_click"]
        self.assertEqual(summary["warmup_successes"], 1)
        self.assertLess(summary["success_p95_ms"], 200)

    def test_duplicate_report_bytes_rejected(self):
        self.reports[1] = copy.deepcopy(self.reports[0])
        self.assertIn("duplicate report bytes", " ".join(self.reject()["errors"]))

    def test_duplicate_uuid_or_matrix_host_rejected(self):
        for key in ("uuid", "matrix"):
            with self.subTest(key=key):
                self.setUp()
                if key == "uuid":
                    self.reports[1]["run_id"] = self.reports[0]["run_id"]
                else:
                    self.reports[1]["ci"]["matrix_host"] = self.reports[0]["ci"]["matrix_host"]
                self.reject()

    def test_mixed_source_browser_playwright_toolchain_or_actions_run_rejected(self):
        for key in ("commit", "source", "chrome_version", "chrome_binary_sha256", "playwright_core", "rustc", "node", "ci"):
            with self.subTest(key=key):
                self.setUp()
                value = self.reports[1]
                if key == "source":
                    value["source_sha256"]["examples/reliability.rs"] = "2" * 64
                elif key == "chrome_version":
                    value[key] = "155.0.8059.40"
                    for run in value["runs"]:
                        run["browser"] = value[key]
                elif key == "chrome_binary_sha256":
                    value[key] = "b" * 64
                elif key == "commit":
                    value[key] = "b" * 40
                elif key == "ci":
                    value[key]["run_id"] = "987655"
                else:
                    value[key] += ".different" if key != "playwright_core" else "-different"
                self.reject()

    def test_host_paths_os_ids_and_result_hashes_are_not_source_comparison_inputs(self):
        self.reports[1]["platform"] = "Linux-other-kernel"
        self.reports[1]["source_sha256"]["docs/a.md"] = "f" * 64
        self.reports[2]["source_sha256"]["target/comparison/runtime.json"] = "d" * 64
        self.assertEqual(self.result()["status"], "passed")

    def test_missing_source_inventory_and_dirty_measurement_source_rejected(self):
        for mode in ("missing", "dirty"):
            with self.subTest(mode=mode):
                self.setUp()
                if mode == "missing":
                    del self.reports[0]["source_sha256"]["examples/reliability.rs"]
                else:
                    self.reports[0]["git_status"] = [" M examples/reliability.rs"]
                self.reject()

    def test_missing_failed_setup_or_engine_phase_rejected(self):
        for mode in ("status", "phase-missing", "setup-failed", "engine-failed", "raw-missing"):
            with self.subTest(mode=mode):
                self.setUp()
                value = self.reports[0]
                if mode == "status":
                    value.update(status="failed", complete=False)
                elif mode == "phase-missing":
                    value["phases"].pop()
                elif mode == "raw-missing":
                    value["runs"].pop()
                else:
                    value["phases"][0 if mode == "setup-failed" else 6]["status"] = "failed"
                self.reject()

    def test_duplicate_missing_boolean_or_reordered_sample_and_warmup_rejected(self):
        for mode in ("duplicate", "missing", "boolean-index", "boolean-latency", "warmup", "order", "case"):
            with self.subTest(mode=mode):
                self.setUp()
                rows = self.reports[0]["runs"][0]["records"]
                if mode == "duplicate":
                    rows[1] = copy.deepcopy(rows[0])
                elif mode == "missing":
                    rows.pop()
                elif mode == "boolean-index":
                    rows[1]["index"] = True
                elif mode == "boolean-latency":
                    rows[1]["elapsed_ms"] = True
                elif mode == "warmup":
                    rows[1]["warmup"] = True
                elif mode == "order":
                    rows[1], rows[2] = rows[2], rows[1]
                else:
                    rows[1]["case"] = "unexpected"
                self.reject()

    def test_incorrect_alternation_or_selected_case_order_rejected(self):
        self.reports[0]["runs"][0], self.reports[0]["runs"][1] = self.reports[0]["runs"][1], self.reports[0]["runs"][0]
        self.reject()
        self.setUp()
        self.reports[0]["cases"].reverse()
        self.reject()

    def test_summary_or_success_requirement_cannot_hide_raw_failures(self):
        self.reports[0]["summary"]["rustwright"]["delayed_click"]["successes"] = 99
        self.reject()
        self.setUp()
        self.reports[0]["success_requirement_met"] = False
        self.reject()

    def test_nonfinite_latency_and_memory_rejected(self):
        for number in (float("nan"), float("inf"), -float("inf"), -1):
            with self.subTest(number=number):
                self.setUp()
                self.reports[0]["runs"][0]["records"][1]["elapsed_ms"] = number
                self.reject()
        self.setUp()
        self.reports[0]["runs"][0]["memory_samples"][0]["driver_rss_kib"] = float("inf")
        self.reject()

    def test_missing_pss_remains_null_with_missing_sample_count(self):
        self.reports[0]["runs"][0]["memory_samples"][0]["driver_pss_kib"] = None
        refresh(self.reports[0])
        result = self.result()
        self.assertEqual(result["status"], "passed", result["errors"])
        memory = result["hosts"][0]["memory"][0]
        self.assertIsNone(memory["max_sampled_kib"]["driver_pss_kib"])
        self.assertEqual(memory["missing_samples"]["driver_pss_kib"], 1)

    def test_missing_memory_observer_error_and_unconfirmed_cleanup_rejected(self):
        for mode in ("missing", "observer", "group", "reap", "exit", "fixture"):
            with self.subTest(mode=mode):
                self.setUp()
                value = self.reports[0]
                process = value["phases"][6]["process"]
                if mode == "missing":
                    value["runs"][0]["memory_samples"] = []
                elif mode == "observer":
                    process["observer_errors"] = ["read failed"]
                elif mode == "group":
                    process["cleanup"]["group_absent"] = False
                elif mode == "reap":
                    process["cleanup"]["direct_child_reaped"] = False
                elif mode == "exit":
                    process["exit_code_before_cleanup"] = None
                else:
                    value["fixture_cleanup"]["socket_closed"] = False
                self.reject()

    def test_launch_policy_and_viewport_census_required(self):
        for mode in ("checks", "dimensions", "sandbox", "bypass", "gpu", "transport"):
            with self.subTest(mode=mode):
                self.setUp()
                policy = self.reports[0]["runs"][0]["launch_policy"]
                if mode == "checks":
                    policy["viewport_checks"] = 815
                elif mode == "dimensions":
                    policy["viewport"]["width"] = 1920
                elif mode == "sandbox":
                    policy["sandbox"] = False
                elif mode == "bypass":
                    policy["browser_launch_args"].append("--no-sandbox")
                elif mode == "gpu":
                    policy["browser_launch_args"].remove("--disable-gpu")
                else:
                    policy["transport"] = "unknown"
                self.reject()

    def test_exactly_three_jobs_required(self):
        for count in (0, 1, 2, 4):
            with self.subTest(count=count):
                values = self.reports[:count] if count < 4 else self.reports + [report(4)]
                self.assertEqual(subject.aggregate(inputs(values))["status"], "invalid")

    def test_nearest_rank_and_null_successes(self):
        self.assertEqual(subject.percentile95(list(range(1, 21))), 19)
        self.assertIsNone(subject.percentile95([]))
        self.assertEqual(subject.median([1e308, 1e308]), 1e308)
        self.assertFalse(subject.finite_number(10 ** 400))

    def test_json_duplicate_keys_nonfinite_and_overflow_rejected(self):
        for text in ('{"status":"complete","status":"failed"}', '{"elapsed":NaN}', '{"elapsed":Infinity}', '{"elapsed":1e999}'):
            with self.subTest(text=text), self.assertRaises(ValueError):
                subject.strict_json(text)

    def test_cli_verifies_sidecars_and_outputs_new_json_and_markdown(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            paths = [write_report(root / f"host-{index}", value) for index, value in enumerate(self.reports)]
            out, md = root / "aggregate.json", root / "aggregate.md"
            args = ["--reports", *map(str, paths), "--output", str(out), "--markdown", str(md)]
            self.assertEqual(subject.main(args), 0)
            result = json.loads(out.read_text())
            self.assertTrue(all(item["raw_artifacts_verified"] for item in result["inputs"]))
            self.assertEqual(sum(len(item["raw_artifacts"]) for item in result["inputs"]), 90)
            before = out.read_bytes()
            with self.assertRaises(SystemExit):
                subject.main(args)
            self.assertEqual(out.read_bytes(), before)

    def test_cli_rust_failure_preserves_failed_result_and_returns_nonzero(self):
        failure(self.reports[0]["runs"][0]["records"][1])
        refresh(self.reports[0])
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            paths = [write_report(root / f"host-{index}", value) for index, value in enumerate(self.reports)]
            out = root / "aggregate.json"
            self.assertEqual(subject.main(["--reports", *map(str, paths), "--output", str(out), "--markdown", str(root / "result.md")]), 1)
            result = json.loads(out.read_text())
            self.assertEqual(result["status"], "failed")
            self.assertEqual(result["measured_failures"]["rustwright"], 1)

    def test_cli_tampered_raw_stdout_is_invalid_and_retains_original_report(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            paths = [write_report(root / f"host-{index}", value) for index, value in enumerate(self.reports)]
            raw = root / "host-0/report.json.artifacts/pair-1-rustwright/stdout.log"
            raw.write_bytes(b"tampered")
            out = root / "aggregate.json"
            self.assertEqual(subject.main(["--reports", *map(str, paths), "--output", str(out), "--markdown", str(root / "result.md")]), 1)
            result = json.loads(out.read_text())
            self.assertEqual(result["status"], "invalid")
            self.assertIn("hash mismatch", result["inputs"][0]["load_error"])
            self.assertEqual(len(result["inputs"][0]["report"]["runs"]), 4)

    def test_cli_missing_reports_still_produces_invalid_artifacts(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            out = root / "aggregate.json"
            args = ["--reports", *[str(root / f"missing-{index}.json") for index in range(3)],
                    "--output", str(out), "--markdown", str(root / "result.md")]
            self.assertEqual(subject.main(args), 1)
            result = json.loads(out.read_text())
            self.assertEqual(result["status"], "invalid")
            self.assertEqual(len(result["inputs"]), 3)

    def test_cli_consistent_summary_edit_cannot_replace_raw_harness_records(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = write_report(Path(tmp) / "host", self.reports[0])
            failure(self.reports[0]["runs"][0]["records"][1])
            refresh(self.reports[0])
            path.write_text(json.dumps(self.reports[0]))
            loaded = subject.load_input(path)
            self.assertIn("raw harness stdout differs: records", loaded["load_error"])

    def test_memory_pid_must_match_owned_process(self):
        self.reports[0]["runs"][0]["memory_samples"][0]["driver_pid"] += 100
        refresh(self.reports[0])
        self.assertIn("measured driver", " ".join(self.reject()["errors"]))


if __name__ == "__main__":
    unittest.main()
