#!/usr/bin/env python3
"""Plot only recorded massif samples against profiler time, with exact peaks."""
import json
from pathlib import Path
import re

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt


ROOT = Path(__file__).resolve().parents[1]
results = json.loads((ROOT / "analysis/recomputed.json").read_text())["runs"]
plt.rcParams.update({"font.family": "DejaVu Sans", "svg.fonttype": "none"})
figure, axes = plt.subplots(1, 2, figsize=(10.5, 4.8), sharey=True)
for axis, result in zip(axes, results):
    text = (ROOT / result["directory"] / "massif.txt").read_text()
    times = [float(value) for value in re.findall(r"^time=(.+)$", text, re.M)]
    amounts = [int(value) / 1000 for value in re.findall(r"^mem_heap_B=(.+)$", text, re.M)]
    assert len(times) == len(amounts) == result["massif_sample_count"]
    axis.step(times, amounts, where="post", color="#246c9b", linewidth=0.8,
              label="Recorded massif samples")
    axis.axhline(result["exact_instantaneous_peak_bytes"] / 1000,
                 color="#a34d39", linestyle="--", linewidth=0.9,
                 label="Exact event-trace peak")
    axis.scatter([times[-1]], [amounts[-1]], color="#246c9b", s=18, zorder=3)
    axis.annotate("End: 26,020 B", (times[-1], amounts[-1]),
                  xytext=(-82, 13), textcoords="offset points", fontsize=9)
    axis.set_title(f"{result['measured_cycles']:,} measured + 100 warmup cycles", fontsize=11)
    axis.set_xlabel("Seconds since profiler start")
    axis.set_xlim(0, times[-1] * 1.04)
    axis.set_ylim(0, 700)
    axis.grid(axis="y", alpha=0.2)
    axis.spines[["right", "top"]].set_visible(False)
axes[0].set_ylabel("Observed live requested heap (kB; 1 kB = 1,000 B)")
axes[0].legend(loc="center left", fontsize=8, frameon=False)
figure.suptitle("Rust driver heap with Firefox 157.0.1", fontsize=14)
figure.text(0.5, 0.018,
    "Separate instrumented runs; includes startup, warmup, sampling, and shutdown. "
    "No cycle interpolation or latency comparison.",
    ha="center", fontsize=8)
figure.tight_layout(rect=(0, 0.07, 1, 0.92))
figure.savefig(ROOT / "analysis/live-heap.svg", metadata={"Date": None})
print(json.dumps({"matplotlib_version": matplotlib.__version__,
                  "source": "original massif.txt time/mem_heap_B snapshots",
                  "curve": "post-step representation of recorded samples; not an event-complete curve",
                  "exact_peak_source": "allocation and deallocation event stream, separate dashed reference"}, indent=2))
