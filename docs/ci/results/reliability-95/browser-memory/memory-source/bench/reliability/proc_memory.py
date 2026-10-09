"""Linux /proc snapshots, preserving missing PSS instead of inventing zeros."""
from pathlib import Path
import time


def fields(path):
    try:
        return {line.split(':', 1)[0]: int(line.split(':', 1)[1].split()[0])
                for line in path.read_text().splitlines()
                if line.startswith(('PPid:', 'VmRSS:', 'VmHWM:', 'Pss:'))}
    except (OSError, ValueError, IndexError):
        return {}


def sample(driver_pid, proc=Path('/proc')):
    processes = {int(path.name): fields(path / 'status')
                 for path in proc.iterdir() if path.name.isdecimal()}
    driver = processes.get(driver_pid, {})
    if 'VmRSS' not in driver:
        return None
    descendants = []
    parents = [driver_pid]
    while parents:
        parent = parents.pop()
        children = [pid for pid, info in processes.items() if info.get('PPid') == parent]
        descendants.extend(children)
        parents.extend(children)
    pss = [fields(proc / str(pid) / 'smaps_rollup').get('Pss') for pid in descendants]
    rss = [processes[pid].get('VmRSS') for pid in descendants]
    return {
        'monotonic_seconds': time.monotonic(), 'driver_pid': driver_pid,
        'driver_rss_kib': driver['VmRSS'], 'driver_hwm_kib': driver.get('VmHWM'),
        'driver_pss_kib': fields(proc / str(driver_pid) / 'smaps_rollup').get('Pss'),
        'descendant_processes': len(descendants),
        'descendant_rss_kib': sum(rss) if all(x is not None for x in rss) else None,
        'descendant_pss_kib': sum(pss) if all(x is not None for x in pss) else None,
    }
