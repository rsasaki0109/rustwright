"""Check reporting semantics so fast failures cannot improve success latency."""
import unittest

from run import percentile95, summarize, validate


class ReportTests(unittest.TestCase):
    def test_percentile_uses_nearest_rank(self):
        self.assertEqual(percentile95(list(range(1, 21))), 19)
        self.assertIsNone(percentile95([]))

    def test_warmups_and_failures_do_not_improve_success_latency(self):
        records = [
            {"case": "delayed_frame", "warmup": True, "ok": True, "elapsed_ms": 1, "error": None},
            {"case": "delayed_frame", "warmup": False, "ok": False, "elapsed_ms": 2, "error": "missing frame"},
            {"case": "delayed_frame", "warmup": False, "ok": True, "elapsed_ms": 200, "error": None},
        ]
        summary = summarize(records)["delayed_frame"]
        self.assertEqual(summary["attempts"], 2)
        self.assertEqual(summary["success_rate"], 0.5)
        self.assertEqual(summary["success_p95_ms"], 200)
        self.assertEqual(summary["failures"], ["missing frame"])

    def test_empty_successes_have_no_latency(self):
        summary = summarize([{"case": "delayed_frame", "warmup": False,
                              "ok": False, "elapsed_ms": 1, "error": "missing frame"}])["delayed_frame"]
        self.assertEqual(summary["success_rate"], 0)
        self.assertIsNone(summary["success_p95_ms"])

    def test_incomplete_measurement_is_rejected(self):
        with self.assertRaises(ValueError):
            validate({"engine": "rustwright", "browser": "Chrome/123", "records": []}, "rustwright", 20)

    def test_selected_cases_validate_and_keep_fast_failures_separate(self):
        records = [{"case": "mock_return", "index": 0, "warmup": True, "ok": True, "elapsed_ms": 1, "error": None},
                   {"case": "mock_return", "index": 1, "warmup": False, "ok": False, "elapsed_ms": 2, "error": "unmocked response"}]
        validate({"engine": "rustwright", "browser": "Chrome/123", "records": records}, "rustwright", 1, ("mock_return",))
        summary = summarize(records, ("mock_return",))
        self.assertEqual(set(summary), {"mock_return"})
        self.assertIsNone(summary["mock_return"]["success_p95_ms"])
        self.assertEqual(summary["mock_return"]["all_attempts_p95_ms"], 2)

    def test_unselected_cases_are_rejected(self):
        rows = [{"case": "mock_fetch", "index": index, "warmup": index == 0, "ok": True, "elapsed_ms": 1, "error": None} for index in (0, 1)]
        with self.assertRaises(ValueError):
            validate({"engine": "rustwright", "browser": "Chrome/123", "records": rows}, "rustwright", 1, ("mock_return",))


if __name__ == "__main__":
    unittest.main()
