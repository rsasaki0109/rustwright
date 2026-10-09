#!/usr/bin/env python3
"""Recompute browser report amounts without launching a browser or forcing GC."""
import gzip
import hashlib
import json
from pathlib import Path

BASE = Path(__file__).resolve().parent
RUN = BASE / "run"
MIB = 1024 * 1024
rows = [json.loads(line) for line in (RUN / "firefox.jsonl").read_text().splitlines()]
samples = [row for row in rows if row["kind"] == "sample"]
result = next(row for row in rows if row["kind"] == "result")
summary = json.loads((RUN / "summary.json").read_text())
assert summary["samples"] == samples and summary["result"] == result
assert result["success"] and result["completed"] == 3100
assert result["pending_commands"] == 0
assert all(row["counts"] == row["baseline"] for row in samples)
assert all(row["pending_commands"] == 0 for row in samples)

observations = []
for row in samples:
    capture = row.get("memory_report")
    if capture is None:
        continue
    compressed = (RUN / "memory-reports" / capture["file"]).read_bytes()
    assert hashlib.sha256(compressed).hexdigest() == capture["sha256"]
    assert len(compressed) == capture["size_bytes"]
    assert capture["signal"] == "SIGRTMIN" and capture["minimize_memory"] is False
    report = json.loads(gzip.decompress(compressed))
    entries = report["reports"]
    roots = [entry for entry in entries if entry["process"].startswith("Main Process")]
    assert len({entry["process"] for entry in roots}) == 1
    root_name = roots[0]["process"]
    assert root_name == f'Main Process (pid {row["memory"]["browser_pid"]})'
    def amount(path):
        matches = [entry["amount"] for entry in roots if entry["path"] == path]
        assert len(matches) == 1, path
        return matches[0]
    categories = {}
    explicit_heap = 0
    for entry in roots:
        if entry["path"].startswith("explicit/") and entry["units"] == 0:
            category = entry["path"].split("/")[1]
            categories[category] = categories.get(category, 0) + entry["amount"]
            if entry["kind"] == 1:
                explicit_heap += entry["amount"]
    ghost_windows = {
        entry["process"]: entry["amount"]
        for entry in entries if entry["path"] == "ghost-windows"
    }
    assert ghost_windows and all(value == 0 for value in ghost_windows.values())
    heap_allocated = amount("heap-allocated")
    parsed_root = capture["processes"][root_name]
    assert parsed_root["heap_allocated_bytes"] == heap_allocated
    assert parsed_root["reported_explicit_heap_bytes"] == explicit_heap
    assert parsed_root["explicit_categories_bytes"] == categories
    observations.append({
        "completed": row["completed"],
        "measured_completed": row["measured_completed"],
        "idle_seconds": row["idle_seconds"],
        "elapsed_ms": row["elapsed_ms"],
        "report": capture["file"],
        "root_pss_kib": row["memory"]["browser_root_pss_kib"],
        "root_pss_mib": row["memory"]["browser_root_pss_kib"] / 1024,
        "root_heap_allocated_bytes": heap_allocated,
        "root_heap_allocated_mib": heap_allocated / MIB,
        "root_reported_resident_bytes": amount("resident"),
        "root_explicit_heap_bytes": explicit_heap,
        "root_unclassified_heap_bytes": heap_allocated - explicit_heap,
        "root_explicit_categories_bytes": categories,
        "root_explicit_categories_mib": {
            key: value / MIB for key, value in sorted(categories.items())
        },
        "ghost_windows": ghost_windows,
        "minimize_memory": False,
    })
assert len(observations) == 6
warmup = next(row for row in observations if row["completed"] == 100)
end = next(row for row in observations if row["completed"] == 3100 and row["idle_seconds"] is None)
tail = next(row for row in observations if row["idle_seconds"] == 90)
output = {
    "scope": "One independent Firefox 157.0.1 localhost churn run; no Rustwright helpers",
    "completed_warmup": 100,
    "completed_measured": 3000,
    "success": result["success"],
    "sample_count": len(samples),
    "all_sample_resource_counts_restored": True,
    "all_sample_pending_commands_zero": True,
    "final_pending_commands": result["pending_commands"],
    "signals": "Six SIGRTMIN reports; never SIGRTMIN+1; no forced minimize or GC requested",
    "measurement_limits": [
        "Allocator allocation totals do not identify unreachable objects or prove leak freedom.",
        "Explicit category sums include both heap and non-heap reporters; compare like categories only.",
        "PSS is sequential /proc accounting and report captures occur afterwards, not atomically.",
        "The idle tail still performs count queries and memory reports at 0/30/90 seconds.",
        "Root PSS and several retained categories remain above the warmup observation.",
        "Zero ghost-windows is one Firefox reporter result, not a general leak detector.",
    ],
    "warmup_to_end_to_90s": {
        "root_pss_mib": [row["root_pss_mib"] for row in [warmup, end, tail]],
        "root_heap_allocated_mib": [row["root_heap_allocated_mib"] for row in [warmup, end, tail]],
        "root_pss_end_minus_warmup_mib": end["root_pss_mib"] - warmup["root_pss_mib"],
        "root_pss_90s_minus_warmup_mib": tail["root_pss_mib"] - warmup["root_pss_mib"],
        "root_heap_90s_minus_warmup_mib": tail["root_heap_allocated_mib"] - warmup["root_heap_allocated_mib"],
    },
    "observations": observations,
}
print(json.dumps(output, indent=2))
