#!/opt/codex/runtimes/codex-primary-runtime/dependencies/python/bin/python3

import os, pathlib, signal, sys, time, urllib.request
pathlib.Path(os.environ['CONTROL_PID']).write_text(str(os.getpid()))
assert '--timeout=10000' in sys.argv
assert '--no-sandbox' not in sys.argv
mode = os.environ['CONTROL_MODE']
body = urllib.request.urlopen(sys.argv[-1], timeout=2).read().decode()
if mode == 'missing-content':
    print('<title>Rustwright Linux sandbox preflight</title>', flush=True)
else:
    print(body, flush=True)
if mode == 'nonzero':
    sys.exit(7)
if mode == 'outer-timeout-exit-zero':
    signal.signal(signal.SIGTERM, lambda *_: sys.exit(0))
    time.sleep(60)
