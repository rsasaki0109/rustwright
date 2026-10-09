# Rust driver allocation evidence

These are two **instrumented Firefox 157.0.1 runs** of the Rust endurance driver: 100 and 1,000 measured cycles, each with 100 warmup cycles. They use the same driver binary and recorded Rust source commit `908323d38ba036960c41347ecbe2a5dfa8c24271`. Every Rust source hash in both manifests was compared with that commit; `source-vs-908323d.json` records all 76 matches, including the endurance source. The later Chrome context fix is outside this run identity. No later commit is substituted into these manifests.

| Measurement | 100 measured + 100 warmup | 1,000 measured + 100 warmup |
| --- | ---: | ---: |
| Recorded allocation calls | 1,616,387 | 7,357,224 |
| Exact instantaneous live-heap peak | 648,786 B | 649,883 B |
| Original analyzer display (SI) | 648.79K | 649.88K |
| Maximum recorded massif sample | 638,588 B | 644,793 B |
| Outstanding requested heap at process exit | 26,020 B | 26,020 B |
| Compressed interpreted trace size | 165,764 B | 676,564 B |
| Native workload | success | success |

The exact peak changes by 1,097 B between these two separate runs, and the exit amount is identical. This is evidence for these bounded workloads, not a universal bound or a latency comparison. Every native count checkpoint restores the original baseline (one page target, four isolated contexts, zero acknowledged helper preloads), and every recorded checkpoint has zero pending commands. Firefox alone is **not** heaptrack-instrumented: each `instrumentation.json` records process-map observations, driver/browser PIDs, the actual browser executable, and its SHA. This is Rust-driver allocation evidence; browser resident and allocator/category evidence is archived separately.

## Exact exit stacks

`analysis/recompute.py` walks allocation/deallocation events in the preserved interpreted v3 streams, checks allocation totals against the original analyzer, verifies trace hashes and native completion, and records exact outstanding allocation stacks in `analysis/recomputed.json`. The following allocations account for all 26,020 B at exit in both runs:

| Recorded allocation stack | Bytes | Outstanding allocations |
| --- | ---: | ---: |
| `tokio::signal::registry::globals_init`, constructing `Vec<SignalInfo>` | 24,064 | 65: one 2,048 B allocation and 64 of 344 B |
| `_dl_allocate_tls` → `pthread_create` → Rust/Tokio runtime worker launch | 1,408 | four of 352 B |
| `std::sys::pal::unix::stack_overflow::thread_info::set_current_info` → `std::rt::lang_start_internal` | 548 | one 544 B and one 4 B |

The full original analyzer labels its exit accounting `MEMORY LEAKS` and `total memory leaked: 26.02K`; those bytes and labels are preserved. Its thread-info group says six calls, while the raw event accounting finds two outstanding allocations in that group. This archive distinguishes all call-site calls from outstanding allocations. These are allocation stacks for runtime/loader initialization, not a reachability analysis. The two traces show the same exit accounting; they do **not** prove that all objects are reachable, that every workload is leak-free, or that Firefox's heap has this bound.

## Preserved valid and invalid records

`valid/` contains each corrected run's actual compressed trace, full analyzer and Rust-demangled output, massif samples, native JSONL, stdout/stderr, manifest, summary, instrumentation assertions, analysis command, and nonzero-allocation validation. The analyzer disables built-in and embedded suppressions, as recorded by its exact command. `sources/` preserves original and corrected observers, the driver-only browser wrapper, and the endurance source. `environment/` preserves locally unpacked package/dependency records, explicitly used non-secret environment settings, and executable identities. The driver/browser/tool ELF files, package archives, and caches are excluded; their identity records remain.

`invalid-instrumentation/` preserves both initial 100/1,000-cycle attempts. Their native workloads succeeded, but `heaptrack_interpret` could not load `libdw.so.1`, leaving 13-byte empty compressed traces. The initial 1,000-cycle observer also raised `StopIteration` when finding its trace. These attempts have **no valid allocation capture**, and none of their analyzer output is used for the peak/end findings or plot. `VALIDITY.json` makes that distinction explicit.

## Reproduction and plot

Use the recorded source commit and binary/build settings from the run manifests, activate `/workspace/.rustwright-env/activate.sh`, restore the unpacked heaptrack dependencies, and run the corrected observer with a **new** output directory. The observer records the exact workload command, requires native success, asserts driver-only instrumentation, checks positive allocation counts, and retains the full analyzer command. Its fixed absolute local paths are preserved, not silently rewritten for the archive.

To independently recompute the frozen allocation evidence without launching a browser:

```sh
python3 docs/ci/results/reliability-95/driver-heap/analysis/recompute.py
```

`analysis/live-heap.svg` is a standalone scientific plot of the original massif samples against seconds since profiler start. It includes startup, all 100 warmup cycles, measured work, sampling overhead, and shutdown. A post-step line represents only recorded samples; their maxima miss the exact instantaneous event-stream peaks, shown separately as dashed references. No workload cycles are interpolated onto the time axis. `analysis/plot.py` and `plot-metadata.json` preserve the plotting source and tool version.

`copied-originals.json` maps each byte-identical copied file to its original local path and SHA. `SHA256SUMS` covers every actual archive file except itself. Existing resource-audit and browser-memory archives were left unchanged.
