# Firefox operation ablation — 2026-10-09

All **6,000 measured cycles** (plus 600 warmup cycles) completed successfully on
Firefox 157.0.1. Three matched workloads ran twice, in forward/reverse order,
with 1,000 measured cycles and 100 warmup cycles per fresh browser/session/profile.
Page targets and isolated contexts returned to their startup counts at all
72 checkpoints; pending commands and the internal-helper ledger were zero.
The raw-helper workload additionally awaited successful removal of each
caller-managed preload; those preloads are excluded from the internal ledger.

## Observations

MiB = KiB / 1024. Values run from the end of 100 warmup cycles to the end of
1,000 measured cycles. RSS double-counts shared mappings. PSS apportions them;
neither metric identifies unreachable objects or isolates allocator/browser caches.

| Order | Workload | Driver RSS warmup → end | Browser tree RSS delta | Browser tree PSS warmup → end (delta) | Browser root PSS delta |
| --- | --- | --- | --- | --- | --- |
| 1.1 | Direct BiDi (`raw`) | 4.32 → 4.34 | +101.79 | 433.55 → 524.01 (+90.47) | +79.17 |
| 1.2 | Direct BiDi + helper (`raw-helper`) | 4.38 → 4.44 | +22.39 | 503.67 → 524.05 (+20.37) | +24.46 |
| 1.3 | Rustwright page/context APIs (`basic`) | 4.61 → 4.79 | +12.41 | 502.60 → 511.95 (+9.35) | +49.48 |
| 2.1 | Rustwright page/context APIs (`basic`) | 4.47 → 4.63 | +26.45 | 487.44 → 510.97 (+23.53) | +28.03 |
| 2.2 | Direct BiDi + helper (`raw-helper`) | 4.36 → 4.47 | +23.24 | 514.94 → 535.47 (+20.53) | +23.53 |
| 2.3 | Direct BiDi (`raw`) | 4.33 → 4.39 | +19.09 | 495.69 → 513.22 (+17.53) | +22.95 |

Browser PSS grows even when page/context wrappers and helper injection are
absent. Automatic helper ownership is therefore not necessary for this observed
growth. Driver RSS grows by only 0.03–0.17 MiB over these measured intervals.
Browser root PSS rises in all six runs, while descendants/process reuse can
offset that growth in the tree total. Per-process samples preserve that distinction.

The first raw pass starts at a substantially lower post-warmup PSS than later
passes. Its endpoint increase is much larger than the reverse-order raw pass,
although their final PSS values are closer. These two observations do not prove
that helper injection lowers memory or that the browser reaches a steady bound.
Browser startup activity, caches, garbage collection, allocator retention,
process reuse and unmeasured protocol resources remain possible explanations.
No percentage leak reduction, attribution to a Firefox defect, or absence of
every Rustwright leak is established.

## Method and evidence

```sh
cargo build --offline --locked --release --example endurance
python3 bench/endurance/ablation.py --cycles 1000 --warmup 100 \
  --output target/endurance/ablation-1000
```

See [workload definitions](README.md#firefox-operation-ablation). All workloads
use the same Rust binary, BiDi transport/session, native Firefox launcher, HTTP
fixture and count/sampling cadence. Direct commands bypass `BidiPage` and
`BidiContext`, but still use Rustwright's connection and the typed evaluation
decoder. This is not an independent transport implementation. Ablation omits
input, absent waits, cancellation, routing and frame swaps; the previously
[recorded full endurance runs](RESULTS.md) cover those separately.

No browser URL policy was changed. Every run uses a disposable profile and the
existing cloud Firefox launcher. Heavy compilation and other browser suites did
not run concurrently. Two replicates expose order variation, not statistical
significance; cycle duration/pacing differs between workloads. Observations do
not force garbage collection or measure the browser heap's reachable objects.

[Preserved results](results/ablation-1000/summary.json) contain six JSONL streams,
stderr/logs and per-run manifests with source/binary hashes, versions, platform,
Git revision/status and timestamps. All 72 PSS totals were readable. Samples are
sequential rather than an atomic process-tree snapshot; per-process reads can
differ slightly from the aggregate reads.

[Source checks](results/ablation-1000/source-check.json) verify the same Rust
binary and measurement source hashes across all six runs and that their browser
root/driver PIDs exited afterward. This does not establish exit of every formerly
attached descendant. `source.tar.gz` and `source-sha256.json` preserve the first
run's code/fixture/config snapshot. Unrelated comparison tooling changed while
the ablation ran; each manifest records that worktree, while all Rust sources,
Cargo files, injected helper and endurance runners used by the measurement match
the archived snapshot. The recorded Git commit alone does not identify the
uncommitted implementation.

Next attribution steps are longer/repeated runs, an independent WebSocket
baseline and workloads isolating context churn from script evaluation. Attached
sessions/intercepts, creation/shutdown cancellation and heap-retention diagnosis
remain separate gaps; zero sampled counters do not close them.
