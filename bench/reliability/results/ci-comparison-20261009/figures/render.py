"""Render descriptive figures from a validated aggregate, without new inference."""
import argparse
import hashlib
import json
from pathlib import Path

import matplotlib
matplotlib.use('Agg')
import matplotlib.pyplot as plt

p = argparse.ArgumentParser()
p.add_argument('aggregate', type=Path)
p.add_argument('output', type=Path)
args = p.parse_args()
raw = args.aggregate.read_bytes()
data = json.loads(raw)
assert data['status'] == 'passed' and len(data['hosts']) == 3
pooled = data['pooled_raw_descriptive']
cases = list(pooled['rustwright'])
assert len(cases) == 16
assert all(pooled[engine][case]['attempts'] == 300 for engine in pooled for case in cases)
args.output.mkdir(parents=True, exist_ok=False)
plt.rcParams.update({'font.size': 10, 'svg.fonttype': 'none'})

fig, ax = plt.subplots(figsize=(12, 8.8))
for offset, engine, label, color in [(-.18, 'rustwright', 'Rustwright', '#006c93'), (.18, 'playwright-core', 'Playwright Core 1.64.0', '#ce761c')]:
    counts = [pooled[engine][case]['successes'] for case in cases]
    ax.barh([i + offset for i in range(16)], counts, height=.32, color=color, label=label)
    for i, count in enumerate(counts):
        ax.text(max(count, 0) + 3, i + offset, f'{count}/300', va='center', fontsize=8)
ax.set_yticks(range(16), cases)
ax.invert_yaxis()
ax.set_xlim(0, 350)
ax.set_xlabel('Successful measured attempts (300 per engine and case)')
ax.set_title('Sixteen local Chromium fixture cases\nRaw counts pooled across three CI job environments; warmups excluded')
ax.legend(loc='upper center', bbox_to_anchor=(.5, -.08), ncol=3)
ax.grid(axis='x', alpha=.2)
ax.set_axisbelow(True)
fig.tight_layout()
fig.savefig(args.output / 'success-counts.svg', metadata={'Date': None})
fig.savefig(args.output / 'success-counts.png', dpi=150)
plt.close(fig)

fig, ax = plt.subplots(figsize=(12, 8.8))
omitted = []
for host_index, host in enumerate(sorted(data['hosts'], key=lambda h: h['matrix_host'])):
    xs, ys = [], []
    for i, case in enumerate(cases):
        left, right = (host['summary'][engine][case] for engine in ('rustwright', 'playwright-core'))
        assert left['attempts'] == right['attempts'] == 100
        if all(row['successes'] == row['attempts'] for row in (left, right)) and left['success_p95_ms'] > 0 and right['success_p95_ms'] > 0:
            xs.append(right['success_p95_ms'] / left['success_p95_ms'])
            ys.append(i + (host_index - 1) * .17)
        else:
            omitted.append({'host': host['matrix_host'], 'case': case})
    ax.scatter(xs, ys, s=36, marker=['o', 's', '^'][host_index], label=host['matrix_host'])
ax.axvline(1, color='#555', linestyle='--', linewidth=1)
ax.set_xscale('log', base=2)
ax.set_yticks(range(16), cases)
ax.invert_yaxis()
ax.set_xlabel('Playwright / Rustwright successful p95 latency (log scale; 1 = equal)')
ax.set_title('Descriptive p95 ratios within each CI job\nShown only when both engines succeed on all 100 measured attempts')
ax.legend(loc='upper center', bbox_to_anchor=(.5, -.08), ncol=3)
ax.grid(axis='x', alpha=.2)
ax.set_axisbelow(True)
fig.tight_layout()
fig.savefig(args.output / 'p95-ratios.svg', metadata={'Date': None})
fig.savefig(args.output / 'p95-ratios.png', dpi=150)
plt.close(fig)
(args.output / 'figure-metadata.json').write_text(json.dumps({
    'aggregate_sha256': hashlib.sha256(raw).hexdigest(),
    'matplotlib_version': matplotlib.__version__,
    'p95_ratios_omitted': omitted,
    'limits': 'Descriptive observations, no confidence intervals, population inference or general SOTA claim. Warmups excluded. A ratio is omitted for any measured failure in either engine in that host/case.',
}, indent=2) + '\n')
