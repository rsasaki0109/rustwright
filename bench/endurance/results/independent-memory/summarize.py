import hashlib
import json
import os
from pathlib import Path
import shutil
import tarfile

ROOT = Path('/workspace/rustwright')
RUN = ROOT / 'target/independent-memory'
OUT = ROOT / 'bench/endurance/results/independent-memory'
OUT.mkdir(parents=True, exist_ok=True)

def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def root_pss(row):
    memory = row['memory']
    return memory.get('browser_root_pss_kib', next(
        (p['pss_kib'] for p in memory['browser_by_process'] if p['root']), None))

def delta(start, end, key):
    a, b = start['memory'][key], end['memory'][key]
    return b - a if a is not None and b is not None else None

rows = json.loads((RUN / 'order.json').read_text())
assert len(rows) == 8
expected_binary = json.loads((RUN / 'rust-binary-sha256.json').read_text())['sha256']
assert digest(RUN / 'endurance-frozen') == expected_binary
expected_sources = json.loads((RUN / 'rust-source-sha256.json').read_text())
with tarfile.open(RUN / 'rust-source.tar.gz') as archive:
    actual = {m.name: hashlib.sha256(archive.extractfile(m).read()).hexdigest()
              for m in archive.getmembers() if m.isfile()}
assert actual == expected_sources
independent_sources = json.loads((RUN / 'independent-source-sha256.json').read_text())
assert all(digest(ROOT / name) == value for name, value in independent_sources.items())
summary = []
metadata = []
traces = []
for row in rows:
    folder = RUN / row['directory']
    records = [json.loads(line) for line in (folder / 'firefox.jsonl').read_text().splitlines()]
    manifest = json.loads((folder / 'manifest.json').read_text())
    meta = next(r for r in records if r['kind'] == 'metadata')
    outcome = next(r for r in records if r['kind'] == 'result')
    assert outcome['success']
    assert not Path(f'/proc/{meta["browser_pid"]}').exists()
    assert not Path(f'/proc/{meta["driver_pid"]}').exists()
    if row['driver'] == 'python':
        assert all(independent_sources[name] == value for name, value in manifest['source_sha256'].items())
        assert not set(outcome['commands']) & {'script.addPreloadScript', 'network.addIntercept'}
    else:
        assert manifest['binary_sha256'] == expected_binary
    samples = [r for r in records if r['kind'] == 'sample']
    assert all(r['pending_commands'] == 0 for r in samples)
    if row['driver'] == 'python':
        assert all(r['counts'] == r['baseline'] for r in samples)
    else:
        assert all(r['counts'] == samples[0]['counts'] for r in samples)
    start = next(r for r in samples if r['completed'] == 100 and r.get('idle_seconds') is None)
    end = next(r for r in samples if r['completed'] == 1100 and r.get('idle_seconds') is None)
    idle = next((r for r in samples if r.get('idle_seconds') == 30), None)
    summary.append({**row, 'success': True, 'measured_cycles': 1000, 'warmup_cycles': 100,
                    'samples': len(samples), 'root_and_driver_exited': True,
                    'post_warmup_delta_kib': {key: delta(start, end, key) for key in
                        ('driver_rss_kib', 'driver_pss_kib', 'browser_tree_rss_kib', 'browser_tree_pss_kib')},
                    'browser_root_pss_delta_kib': root_pss(end) - root_pss(start),
                    'idle_30s_root_pss_delta_kib': root_pss(idle) - root_pss(end) if idle else None,
                    'idle_30s_tree_pss_delta_kib': delta(end, idle, 'browser_tree_pss_kib') if idle else None,
                    'measurement_duration_ms': end['elapsed_ms'] - start['elapsed_ms'],
                    'tree_pss_unavailable_samples': sum(s['memory']['browser_tree_pss_kib'] is None for s in samples)})
    metadata.append({'directory': row['directory'], 'metadata': meta})
    traces.append((row, start, end, [s for s in samples if s['completed'] >= 100]))
    shutil.copytree(folder, OUT / row['directory'], dirs_exist_ok=True)
    shutil.copyfile(RUN / (row['directory'] + '.log'), OUT / (row['directory'] + '.log'))

for name in ['build.log', 'matrix.log', 'order.json', 'rust-source.tar.gz', 'rust-source-sha256.json',
             'rust-binary-sha256.json', 'independent-source-sha256.json', 'smoke-churn.log']:
    shutil.copyfile(RUN / name, OUT / name)
shutil.copytree(RUN / 'independent-source', OUT / 'independent-source', dirs_exist_ok=True)
shutil.copytree(RUN / 'smoke-churn', OUT / 'smoke-churn', dirs_exist_ok=True)
shutil.copyfile('/tmp/run_independent_memory.py', OUT / 'run-order.py')
shutil.copyfile(__file__, OUT / 'summarize.py')
(OUT / 'summary.json').write_text(json.dumps({'runs': summary, 'measured_cycles': 8000,
    'warmup_cycles': 800, 'samples': sum(s['samples'] for s in summary),
    'scope': 'Independent transport necessity and sampled resource restoration; differing pace/time, two replicates, no heap reachability or exactly-once lost-ack claims'}, indent=2) + '\n')
(OUT / 'metadata.json').write_text(json.dumps(metadata, indent=2) + '\n')
(OUT / 'source-check.json').write_text(json.dumps({'frozen_rust_binary_unchanged': True,
    'frozen_rust_source_archive_hashes_match': True, 'rust_source_file_count': len(expected_sources),
    'independent_sources_unchanged_across_runs': True, 'root_and_driver_exited_all_eight': True,
    'historical_descendant_exit': 'not established'}, indent=2) + '\n')

os.environ['MPLCONFIGDIR'] = '/tmp/rustwright-memory-mpl'
import matplotlib
matplotlib.use('Agg')
import matplotlib.pyplot as plt
fig, axes = plt.subplots(1, 2, figsize=(11, 4), sharey=True, layout='constrained')
for row, start, end, samples in traces:
    index = 0 if row['workload'] in ('raw', 'churn') else 1
    ax = axes[index]
    label = f"{row['order']}: {row['driver']} {row['workload']}"
    measured = [s for s in samples if s.get('idle_seconds') is None]
    ax.plot([(s['elapsed_ms'] - start['elapsed_ms']) / 1000 for s in measured],
            [(root_pss(s) - root_pss(start)) / 1024 for s in measured], label=label)
    tail = [end] + [s for s in samples if s.get('idle_seconds') is not None]
    if len(tail) > 1:
        color = ax.lines[-1].get_color()
        ax.plot([(s['elapsed_ms'] - start['elapsed_ms']) / 1000 for s in tail],
                [(root_pss(s) - root_pss(start)) / 1024 for s in tail], '--s', ms=3, color=color)
    ax.scatter([(end['elapsed_ms'] - start['elapsed_ms']) / 1000],
               [(root_pss(end) - root_pss(start)) / 1024], s=22, color=ax.lines[-1].get_color())
for ax, title in zip(axes, ['Matched context/tab churn', 'Faster context-only/evaluation controls']):
    ax.set_title(title, fontsize=11)
    ax.axhline(0, color='#999999', lw=.7)
    ax.set_xlabel('Seconds since each warmup endpoint')
    ax.grid(alpha=.2)
    ax.legend(fontsize=8)
axes[0].set_ylabel('Browser root PSS change (MiB)')
fig.suptitle('Firefox 157.0.1: sampled memory, without forced garbage collection', fontsize=12)
fig.savefig(OUT / 'root-pss.png', dpi=160)
plt.close(fig)
print(json.dumps({'runs': summary, 'total_measured': 8000}, ensure_ascii=False))
