# Three CI reliability observations

Status: **invalid**.

Status describes validation and the required Rustwright measured-success policy, not an assertion that the reference had no failures or that Rustwright was faster.

Successful latency excludes warmups and failed attempts. Ratios appear only for cases/scopes with every measured attempt successful in both engines; they do not establish significant or general SOTA superiority.

Validation errors:

- target/comparison/input/comparison-host-1/report.json: incomplete/failed setup report
- target/comparison/input/comparison-host-2/report.json: incomplete/failed setup report
- target/comparison/input/comparison-host-3/report.json: incomplete/failed setup report

## Separate sampled memory

Each field has its own maximum in MiB; maxima need not be simultaneous. Missing readings remain unavailable, with counts below.

| Host | Pair | Engine | Samples | Driver RSS | Driver PSS | Descendant RSS | Descendant PSS | Missing RSS/PSS counts (driver; descendants) |
| --- | ---: | --- | ---: | ---: | ---: | ---: | ---: | --- |

## Limits and provenance

- Three distinct CI matrix IDs/UUIDs record three job observations; they do not prove a statistically independent population or significant SOTA superiority.
- Pooled counts and nearest-rank p95 are raw descriptive summaries, not independent estimates, confidence intervals or cross-host latency inference.
- Successful median/p95 excludes failed attempts and warmups; reference-to-Rustwright p95 ratios appear only when both engines succeed on every measured attempt in that case/scope.
- All failure records and elapsed times are retained; all-attempt p95 is separate and is never used in a successful-latency advantage ratio.
- Linux memory is sampled driver-versus-descendants, not an atomic snapshot, exact peak, browser-only footprint or leak test; RSS double-counts shared mappings and missing PSS stays null.
- Memory spans launch, setup, measured operations and shutdown, including temporary browsers/helpers; monitoring can perturb timing.
- Source equality covers the declared measurement/build inputs, not whole-checkout identity; report hashes identify inputs but are not independent attestation of their contents.
- Launch flags, debugging port versus pipe, initial pages and driver implementation remain engine-specific; these local fixture observations do not establish broad sites, Firefox, headed or cross-platform superiority.

Input file hashes, raw host reports (including failed samples), source hashes, launch policies and aggregate-tool identities remain in the JSON artifact.
