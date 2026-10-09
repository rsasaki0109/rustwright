#!/usr/bin/env python3
"""Run matched Firefox workloads sequentially in forward/reverse order."""
import argparse
import json
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--cycles', type=int, default=1000)
    parser.add_argument('--warmup', type=int, default=100)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    if args.cycles < 1 or args.warmup < 0:
        parser.error('invalid cycles or warmup')
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    summaries = []
    for pair, order in enumerate((('raw', 'raw-helper', 'basic'), ('basic', 'raw-helper', 'raw')), 1):
        for workload in order:
            folder = output / f'pair-{pair}-{workload}'
            print(f'Pair {pair}: {workload}', flush=True)
            command = [sys.executable, str(ROOT / 'bench/endurance/run.py'), '--engine', 'firefox',
                       '--workload', workload, '--cycles', str(args.cycles), '--warmup', str(args.warmup),
                       '--output', str(folder)]
            with (output / f'pair-{pair}-{workload}.log').open('w') as log:
                subprocess.run(command, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT, check=True,
                               timeout=60 + 15 * (args.cycles + args.warmup))
            summary = json.loads((folder / 'summary.json').read_text())[0]
            samples = [s for s in summary['samples'] if s['completed'] >= args.warmup]
            delta = {key: samples[-1]['memory'][key] - samples[0]['memory'][key]
                     if all(s['memory'][key] is not None for s in (samples[0], samples[-1])) else None
                     for key in ('driver_rss_kib', 'browser_tree_rss_kib', 'driver_pss_kib', 'browser_tree_pss_kib')}
            row = {'pair': pair, 'workload': workload, 'success': summary['success'],
                   'directory': folder.name, 'post_warmup_delta_kib': delta}
            summaries.append(row)
            (output / 'summary.json').write_text(json.dumps(summaries, indent=2) + '\n')
            print(json.dumps(row), flush=True)


if __name__ == '__main__':
    main()
