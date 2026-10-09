#!/usr/bin/env python3
"""Fail closed when any retained native cancellation assertion is missing."""
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent
summary = json.loads((ROOT / "summary.json").read_text())
assert summary["success"] and len(summary["runs"]) == 2
checked = []
identities = json.loads((ROOT / "browser-identities.json").read_text())
for backend in ("chrome", "firefox"):
    run = json.loads((ROOT / (backend + "-run/summary.json")).read_text())
    assert (ROOT / (backend + ".exit")).read_text().strip() == "0"
    assert run["success"] and run["requested"] == run["completed"] == 10
    assert len(run["records"]) == 10
    for record in run["records"]:
        assert record["success"]
        assert record["observed_native_before_cancel"] == identities[backend]["actual_executable"]
        assert record["direct_parent"] == record["observer_pid"]
        assert record["launch_task_cancelled"] and not record["launch_handoff_reached"]
        assert record["proc_absent_after_cancel"]
        assert record["waitpid_return"] == -1 and record["waitpid_errno"] == 10
        assert record["profile_ownership_assertion"]
        assert record["fixture_cleanup"] == []
        assert record["reason"] is None and record["launch_error"] is None
        checked.append({"backend": backend, "cycle": record["cycle"], "pid": record["pid"]})
print(json.dumps({"verified_native_cancellations": len(checked), "records": checked,
                  "scope": "recorded direct-child and profile-ownership assertions only"}, indent=2))
