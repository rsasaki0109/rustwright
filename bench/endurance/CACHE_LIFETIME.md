# Closed Chrome page retention — 2026-10-09

The comparison exposed retained closed pages in a long-lived Chrome default
context. Its first complete Rustwright pass succeeded in all 800 measured
operations (16 cases × 50), but sampled driver RSS rose gradually to **36.21 MiB**.
The comparison was interrupted before a complete reference pass or second pair.
This is a diagnostic baseline, not a completed 100-sample comparison.

## Defect and correction

The context's registry held a strong `Page` clone after explicit page closure,
external root-session detachment or browser disconnection. Closed entries were
removed only on `pages()`/`refresh_pages()` or context closure. Repeated default
context page creation/closure without discovery retained event pumps, logs and
state. Previous isolated-context endurance drained the registry every cycle,
concealing this path.

Three protocol regressions failed before correction. Explicit closure retained
two entries when only one live page should remain; external detachment and
browser disconnection failed to empty the registry within their one-second
guards. The explicit-close test repeats 20 times while retaining a closed page
clone and another live page, without calling a list/discovery method.

Page state now holds a weak reference to its owning registry. Closure removes
the entry immediately without an ownership cycle, and detached root-session
event pumps exit. Registration publishes the weak owner and checks closure
under the registry lock, preventing stale insertion during initialization.
User-retained handles still own their state and diagnostics. All three tests
now pass, and another live page stays tracked.

## Native verification

```sh
cargo build --offline --locked --release --example endurance
python3 bench/endurance/run.py --engine chrome --workload default \
  --cycles 1000 --warmup 100 --output target/endurance/default-context
```

One fresh browser/default context completed **1,000 measured cycles** plus 100
warmup cycles with zero failures. Half the pages closed via `Page::close`, half
via a separate CDP connection, while retaining page clones. Each page navigates
to local HTTP and verifies a filled Unicode value. This workload never calls
`pages()` or `refresh_pages()`.

All 12 checkpoints returned to one startup page, zero isolated contexts and
zero pending commands. From the end of warmup to the end (MiB = KiB / 1024):

| Linux memory | Warmup → end, MiB | Delta, MiB |
| --- | --- | --- |
| Driver RSS | 5.63 → 5.78 | +0.15 |
| Driver PSS | 4.84 → 4.99 | +0.15 |
| Browser tree RSS | 590.54 → 618.27 | +27.73 |
| Browser tree PSS | 337.02 → 363.90 | +26.87 |

The native run and comparison baseline use different workloads; these endpoints
are not a controlled percentage reduction estimate. Registry tests establish
the specific closed-handle correction. RSS/PSS do not prove absence of other
leaks. Browser memory growth, attached sessions/intercepts and cancellation of
creation/shutdown remain separate gaps.

Native HTTP suites passed **125/125**: Chrome/Firefox shared matrix 24, Chrome
contexts 10, frames 67, navigation 14 and network-idle 10. Workspace unit tests
passed 68/68. Locked Rust 1.85.0 all-target checks, stable Clippy with `-D warnings`,
formatting, whitespace checks and seven Python reporting/memory tests passed.
Windows/macOS and remote CI were not executed.

## Evidence

- [Native raw samples/summary](results/default-context/summary.json), manifest,
  stderr, exact code/config snapshot and
  [source checks](results/default-context/source-check.json). Source/binary hashes
  matched; driver/browser root PIDs exited. Every former descendant PID was not
  tracked through shutdown.
- [Pre-fix partial report](../reliability/results/cache-before/partial-report.json)
  retains 800 successful measured operations, 16 excluded warmups, memory samples
  and binary/source identifiers. Its planned count was 100 per case, but only one
  50-sample Rust pass was recorded. The interrupted reference pass has no complete
  records and is not counted as passed or failed. The source archive identifies
  the uncommitted implementation more precisely than the Git commit alone.
- `../reliability/results/cache-before/registry-regressions.patch` adds the three
  ownership tests to the archived pre-fix context source. Apply in an extracted
  scratch copy, then run
  `cargo test --locked -p rustwright-core --lib releases_registry` to replay them.

The [full alternating 100-sample comparison](../reliability/COMPARISON_100.md)
was rerun with the correction: Rustwright completed 1,600/1,600, with maximum
sampled driver RSS of 6.19 and 6.16 MiB in its two passes. The interrupted
baseline is not evidence for the final code's performance.
