#!/usr/bin/env python3
"""Derive exact live allocation amounts from preserved heaptrack v3 streams.

The interpreted format records allocation-info (`a SIZE TRACE`), allocation
(`+ INFO`), and deallocation (`- INFO`) entries. Numeric fields are hexadecimal.
This parser checks its totals against the original analyzer and native result;
it does not infer reachability or convert profiler time into workload cycles.
"""
import hashlib
import json
from pathlib import Path
import re
import subprocess


ROOT = Path(__file__).resolve().parents[1]


def analyze(name):
    folder = ROOT / "valid" / name
    trace_file = folder / "heaptrack.zst"
    process = subprocess.Popen(
        ["zstd", "-dc", str(trace_file)], stdout=subprocess.PIPE, text=True
    )
    infos, strings, ips, traces = [], [None], [None], [None]
    live = {}
    current = peak = count = 0
    for line in process.stdout:
        tag = line[:2]
        if tag == "v ":
            assert line.split()[2] == "3", "Expected interpreted format v3"
        elif tag == "a ":
            size, trace = line[2:].split()
            infos.append((int(size, 16), int(trace, 16)))
        elif tag == "+ ":
            index = int(line[2:], 16)
            current += infos[index][0]
            peak = max(peak, current)
            count += 1
            live[index] = live.get(index, 0) + 1
        elif tag == "- ":
            index = int(line[2:], 16)
            current -= infos[index][0]
            live[index] -= 1
            assert live[index] >= 0 and current >= 0
        elif tag == "s ":
            _, value = line[2:].rstrip("\n").split(" ", 1)
            strings.append(value)
        elif tag == "i ":
            ips.append([int(value, 16) for value in line[2:].split()])
        elif tag == "t ":
            traces.append([int(value, 16) for value in line[2:].split()])
    assert process.wait() == 0
    analyzer = (folder / "heaptrack-print.log").read_text()
    validation = json.loads((folder / "profile-validation.json").read_text())
    expected_count = int(re.search(r"calls to allocation functions: (\d+)", analyzer)[1])
    assert count == expected_count == validation["calls_to_allocation_functions"] > 0
    assert f"peak heap memory consumption: {peak / 1000:.2f}K" in analyzer
    assert f"total memory leaked: {current / 1000:.2f}K" in analyzer
    assert hashlib.sha256(trace_file.read_bytes()).hexdigest() == validation["trace_sha256"]
    outstanding = []
    for index, number in live.items():
        if not number:
            continue
        size, trace = infos[index]
        frames = []
        while trace:
            ip_index, trace = traces[trace]
            ip = ips[ip_index]
            frames.append({
                "ip": hex(ip[0]),
                "module": strings[ip[1]],
                "symbol": strings[ip[2]] if len(ip) >= 3 and ip[2] else hex(ip[0]),
            })
        symbols = subprocess.check_output(
            ["c++filt", "-s", "rust"],
            input="\n".join(frame["symbol"] for frame in frames) + "\n",
            text=True,
        ).splitlines()
        assert len(symbols) == len(frames)
        for frame, symbol in zip(frames, symbols):
            frame["demangled_symbol"] = symbol
        outstanding.append({
            "allocation_info_index": index,
            "allocation_size_bytes": size,
            "outstanding_allocations": number,
            "outstanding_bytes": size * number,
            "stack": frames,
        })
    assert sum(item["outstanding_bytes"] for item in outstanding) == current
    summary = json.loads((folder / "summary.json").read_text())
    manifest = json.loads((folder / "manifest.json").read_text())
    metadata = json.loads((folder / "native.jsonl").read_text().splitlines()[0])
    samples = summary["samples"]
    assert summary["success"] and summary["exit_code"] == 0
    assert samples[-1]["completed"] == manifest["requested_measured"] + manifest["requested_warmup"]
    assert all(sample["counts"] == samples[0]["counts"] for sample in samples)
    assert all(sample["pending_commands"] == 0 for sample in samples)
    massif = (folder / "massif.txt").read_text()
    sampled_heap = [int(value) for value in re.findall(r"^mem_heap_B=(.+)$", massif, re.M)]
    assert sampled_heap[-1] == current and max(sampled_heap) <= peak
    return {
        "directory": "valid/" + name,
        "measured_cycles": manifest["requested_measured"],
        "warmup_cycles": manifest["requested_warmup"],
        "run_git_head": manifest["git_head"],
        "driver_binary_sha256": manifest["binary_sha256"],
        "firefox_version": metadata["version"],
        "trace_bytes": trace_file.stat().st_size,
        "trace_sha256": validation["trace_sha256"],
        "allocation_calls": count,
        "exact_instantaneous_peak_bytes": peak,
        "sampled_massif_peak_bytes": max(sampled_heap),
        "massif_sample_count": len(sampled_heap),
        "end_outstanding_bytes": current,
        "native_count_baseline": samples[0]["counts"],
        "all_sample_counts_restored": True,
        "all_sample_pending_commands_zero": True,
        "outstanding_allocations": outstanding,
    }


if __name__ == "__main__":
    results = [analyze("driver-heap-100-fixed"), analyze("driver-heap-1000-fixed")]
    assert results[0]["driver_binary_sha256"] == results[1]["driver_binary_sha256"]
    assert all(result["end_outstanding_bytes"] == 26020 for result in results)
    print(json.dumps({"runs": results}, indent=2))
