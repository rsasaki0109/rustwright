#!/usr/bin/env python3
"""Preserve native endurance output and source/binary identifiers; Linux only."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import subprocess
import sys
from datetime import datetime, timezone

ROOT = Path(__file__).resolve().parents[2]


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--engine', choices=['chrome', 'firefox', 'both'], default='both')
    parser.add_argument('--cycles', type=int, default=1000)
    parser.add_argument('--warmup', type=int, default=100)
    parser.add_argument('--workload', choices=['full', 'basic', 'raw', 'raw-helper', 'default'], default='full')
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--binary', type=Path, default=ROOT / 'target/release/examples/endurance')
    args = parser.parse_args()
    if args.cycles < 1 or args.warmup < 0:
        parser.error('cycles must be positive and warmup nonnegative')
    if args.workload.startswith('raw') and args.engine != 'firefox':
        parser.error('direct BiDi workloads require --engine firefox')
    if args.workload == 'default' and args.engine != 'chrome':
        parser.error('default-context workload requires --engine chrome')
    if platform.system() != 'Linux':
        parser.error('this harness requires Linux /proc')
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    binary = args.binary.resolve()
    files = subprocess.check_output(['git', 'ls-files', '-z', '--cached', '--others', '--exclude-standard'], cwd=ROOT).split(b'\0')
    sources = {os.fsdecode(p): digest(ROOT / os.fsdecode(p)) for p in sorted(set(files)) if p and (ROOT / os.fsdecode(p)).is_file()}
    metadata = {
        'started_utc': datetime.now(timezone.utc).isoformat(),
        'command': sys.argv,
        'git_head': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
        'git_status': subprocess.check_output(['git', 'status', '--porcelain'], cwd=ROOT, text=True).splitlines(),
        'source_sha256': sources,
        'binary_sha256': digest(binary),
        'rustc': subprocess.check_output(['rustc', '--version'], text=True).strip(),
        'platform': platform.platform(),
        'profile': 'caller-supplied binary; standard command builds release',
        'cycles': args.cycles,
        'warmup': args.warmup,
        'workload': args.workload,
    }
    (output / 'manifest.json').write_text(json.dumps(metadata, indent=2) + '\n')
    engines = ['chrome', 'firefox'] if args.engine == 'both' else [args.engine]
    summaries = []
    failed = False
    for engine in engines:
        command = [str(binary), engine, str(args.cycles), str(args.warmup), args.workload]
        print(f'Running {engine}: {args.warmup} warmup + {args.cycles} measured cycles', flush=True)
        records = []
        with (output / f'{engine}.jsonl').open('w') as raw, (output / f'{engine}.stderr').open('w') as errors:
            process = subprocess.Popen(command, cwd=ROOT, stdout=subprocess.PIPE, stderr=errors, text=True)
            try:
                for line in process.stdout:
                    raw.write(line)
                    raw.flush()
                    record = json.loads(line)
                    records.append(record)
                    if record.get('kind') in ('sample', 'result'):
                        print(engine, line.strip(), flush=True)
                code = process.wait()
            except BaseException:
                process.terminate()
                try:
                    process.wait(timeout=20)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()
                raise
            finally:
                process.stdout.close()
        samples = [r for r in records if r.get('kind') == 'sample']
        measured = [r for r in samples if r['completed'] >= args.warmup]
        result = records[-1] if records else {}
        success = code == 0 and result.get('kind') == 'result' and result.get('success') is True and bool(samples) and samples[-1]['completed'] == args.warmup + args.cycles
        summary = {'engine': engine, 'success': success, 'exit_code': code, 'result': result, 'samples': samples}
        if measured:
            start, end = measured[0], measured[-1]
            summary['post_warmup_rss_delta_kib'] = {key: end['memory'][key] - start['memory'][key] for key in ('driver_rss_kib', 'browser_tree_rss_kib')}
            summary['tail_rss_range_kib'] = {key: [min(s['memory'][key] for s in measured[-5:]), max(s['memory'][key] for s in measured[-5:])] for key in ('driver_rss_kib', 'browser_tree_rss_kib')}
        summaries.append(summary)
        failed |= not success
    (output / 'summary.json').write_text(json.dumps(summaries, indent=2) + '\n')
    return int(failed)


if __name__ == '__main__':
    sys.exit(main())
