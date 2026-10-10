# Independent Firefox browser-memory evidence

This is one Firefox 157.0.1 Linux localhost churn experiment using a raw Python/WebSocket driver, with no Rustwright helper or preload scripts. All 100 warmup plus 3,000 measured cycles succeeded. Every one of the 18 recorded samples restored the original page/user-context lists and had zero pending commands; the final result also had zero pending commands. The workload alternates closing the page before removing its user context and removing the user context directly.

Six normal `SIGRTMIN` reports are preserved. The harness never sends `SIGRTMIN+1` and requests no forced memory minimization or GC. `nsMemoryInfoDumper.cpp` and its provenance record explain the signal distinction; that upstream-main source is not proof of the exact installed Firefox source. The actual run records Firefox binary identity, installed signal handling, and successful report captures. The tail remains instrumented: count queries and reports run at nominal idle times 0, 30, and 90 seconds.

| Main-process quantity (MiB) | After 100 warmup | End of 3,000 measured | 90-second tail |
| --- | ---: | ---: | ---: |
| `/proc` PSS | 351.68848 | 419.92871 | 388.89355 |
| Reporter `resident` | 384.91016 | 453.59766 | 422.69531 |
| Allocator `heap-allocated` | 287.30455 | 282.40331 | 262.84633 |
| Reported explicit heap (kind=1) | 228.48409 | 253.26090 | 238.97343 |
| Heap minus reported explicit heap | 58.82046 | 29.14241 | 23.87290 |
| Explicit `js-non-window` category | 108.17660 | 92.15747 | 69.82654 |
| Explicit `window-objects` category | 15.02559 | 51.95078 | 48.73644 |
| Explicit `network` category | 0.71483 | 21.68246 | 21.67537 |
| Explicit `gfx` category | 100.72740 | 102.33769 | 101.33504 |

Allocator totals decline while PSS and some categories remain above warmup. The 90-second PSS remains 37.20508 MiB above warmup; the allocator total is 24.45822 MiB below warmup. Explicit category totals can include both heap and non-heap reporters and must not be treated as disjoint subdivisions of `heap-allocated`. All reported processes have `ghost-windows=0` in all six captures.

These measurements describe residency, allocator amounts, and Firefox reporter categories. They do not identify unreachable objects, prove leak freedom, establish a long-run plateau, explain all retained categories, or measure Rust driver heap. Sequential `/proc` PSS sampling and later report collection are not atomic; RSS also counts shared mappings in multiple processes. A zero ghost-window reporter is a useful observation, not a general leak detector. Automatic browser reclamation remains allowed, and report collection itself instruments the workload.

`run/` preserves the original JSONL, summary, manifest, stderr and all six gzip reports byte for byte. The historical run manifest records HEAD `3e52b79707905683da65e2bfbe0ad4b6beb93e50` with dirty source. Three archived Python source hashes match later commit `908323d38ba036960c41347ecbe2a5dfa8c24271`, as checked in `source-vs-908323d.json`; this does not mean the run happened on that commit. `memory-python-tests.log` records 13 successful tests for the memory-report helper. No browser executable or Python dependency binaries are copied.

Recompute all six raw-report amounts, verify their SHA256 records, and check the 18 samples against the preserved summary with:

```sh
python3 docs/ci/results/reliability-95/browser-memory/analyze.py
```

The output must reproduce `analysis.json`. `ROOTED_SHA256SUMS` covers all actual archive files except itself, with paths relative to the repository root. Original manifests and source-hash records remain unchanged.
