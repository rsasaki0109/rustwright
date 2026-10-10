#!/usr/bin/env python3
"""Run preserved external consumer against actual installed browser execs."""
import hashlib
import json
from pathlib import Path
import subprocess
import time

ROOT = Path(__file__).resolve().parent
EXECUTABLE = ROOT / "build/debug/rustwright-native-startup-observer"
identities = json.loads((ROOT / "browser-identities.json").read_text())
commands = []
results = []
for backend in ("chrome", "firefox"):
    folder = ROOT / (backend + "-run")
    command = [str(EXECUTABLE), str(folder), backend, "10",
               str(ROOT / "launchers" / backend), identities[backend]["actual_executable"]]
    commands.append(command)
    (ROOT / "run-commands.json").write_text(json.dumps(commands, indent=2) + "\n")
    with (ROOT / (backend + ".stdout.jsonl")).open("w") as stdout, (ROOT / (backend + ".stderr.log")).open("w") as stderr:
        process = subprocess.Popen(command, stdout=stdout, stderr=stderr)
        # Per-cycle consumer deadlines are ten seconds to observe startup and
        # six seconds to await cancellation, with library-owned cleanup. This
        # outer deadline is only a failsafe, not a success condition.
        try:
            code = process.wait(timeout=200)
        except subprocess.TimeoutExpired:
            process.kill()
            code = process.wait()
            raise RuntimeError("outer failsafe fired; native evidence invalid")
    (ROOT / (backend + ".exit")).write_text(str(code) + "\n")
    summary = json.loads((folder / "summary.json").read_text())
    results.append({"backend": backend, "exit_code": code,
                    "completed": summary["completed"], "success": summary["success"]})
    print(json.dumps(results[-1]), flush=True)
    if code != 0:
        break
(ROOT / "summary.json").write_text(json.dumps({"runs": results,
    "success": len(results) == 2 and all(run["success"] for run in results),
    "scope": "direct actual browser child cancellation before API readiness handoff and local profile ownership; no descendant or latency claim"}, indent=2) + "\n")
