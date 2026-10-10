# Three CI reliability observations

Status: **passed**.

Status describes validation and the required Rustwright measured-success policy, not an assertion that the reference had no failures or that Rustwright was faster.

Successful latency excludes warmups and failed attempts. Ratios appear only for cases/scopes with every measured attempt successful in both engines; they do not establish significant or general SOTA superiority.

## Host host-1

Excluded warmup successes: Rustwright 32/32; Playwright 28/32.

| Scenario | Rustwright success | Playwright success | Rust median/p95 ms | Playwright median/p95 ms | Reference/Rust successful p95 |
| --- | ---: | ---: | ---: | ---: | ---: |
| delayed_click | 100/100 | 100/100 | 167.87/176.61 | 217.67/228.20 | 1.29 |
| delayed_frame | 100/100 | 100/100 | 205.33/208.34 | 284.11/295.48 | 1.42 |
| cross_site_frame | 100/100 | 100/100 | 231.98/243.49 | 287.34/302.09 | 1.24 |
| cross_site_navigation | 100/100 | 100/100 | 292.68/304.36 | 365.52/389.11 | 1.28 |
| disabled_click | 100/100 | 100/100 | 173.85/195.72 | 234.09/241.07 | 1.23 |
| covered_click | 100/100 | 100/100 | 161.46/174.91 | 231.55/238.77 | 1.37 |
| moving_click | 100/100 | 100/100 | 197.32/206.32 | 255.85/266.09 | 1.29 |
| clipped_click | 100/100 | 100/100 | 80.84/89.06 | 91.54/100.66 | 1.13 |
| rotated_clipped_click | 100/100 | 0/100 | 77.28/83.66 | —/— | — |
| http_disconnect_recovery | 100/100 | 100/100 | 23.32/30.49 | 38.91/56.36 | 1.85 |
| browser_disconnect | 100/100 | 100/100 | 44.28/52.38 | 70.99/79.06 | 1.51 |
| mock_fetch | 100/100 | 100/100 | 11.94/14.81 | 17.39/27.52 | 1.86 |
| mock_navigation | 100/100 | 100/100 | 33.28/39.26 | 42.05/57.02 | 1.45 |
| mock_frame | 100/100 | 100/100 | 65.04/74.58 | 138.54/151.78 | 2.04 |
| mock_return | 100/100 | 0/100 | 35.70/41.28 | —/— | — |
| mock_clear | 100/100 | 100/100 | 21.49/30.25 | 27.98/37.31 | 1.23 |

## Host host-2

Excluded warmup successes: Rustwright 32/32; Playwright 28/32.

| Scenario | Rustwright success | Playwright success | Rust median/p95 ms | Playwright median/p95 ms | Reference/Rust successful p95 |
| --- | ---: | ---: | ---: | ---: | ---: |
| delayed_click | 100/100 | 100/100 | 166.96/176.94 | 218.40/228.53 | 1.29 |
| delayed_frame | 100/100 | 100/100 | 203.50/207.77 | 284.06/298.09 | 1.43 |
| cross_site_frame | 100/100 | 100/100 | 232.69/244.18 | 288.64/304.79 | 1.25 |
| cross_site_navigation | 100/100 | 100/100 | 298.20/311.44 | 365.18/390.24 | 1.25 |
| disabled_click | 100/100 | 100/100 | 173.06/196.19 | 233.49/242.27 | 1.23 |
| covered_click | 100/100 | 100/100 | 163.03/175.38 | 229.01/239.76 | 1.37 |
| moving_click | 100/100 | 100/100 | 196.72/205.95 | 259.98/266.16 | 1.29 |
| clipped_click | 100/100 | 100/100 | 80.63/89.62 | 90.18/100.35 | 1.12 |
| rotated_clipped_click | 100/100 | 0/100 | 78.02/84.46 | —/— | — |
| http_disconnect_recovery | 100/100 | 100/100 | 27.37/33.37 | 45.69/60.04 | 1.80 |
| browser_disconnect | 100/100 | 100/100 | 44.89/51.97 | 71.75/83.82 | 1.61 |
| mock_fetch | 100/100 | 100/100 | 11.88/14.12 | 19.53/25.54 | 1.81 |
| mock_navigation | 100/100 | 100/100 | 33.25/38.75 | 42.57/54.57 | 1.41 |
| mock_frame | 100/100 | 100/100 | 65.14/74.68 | 136.64/154.52 | 2.07 |
| mock_return | 100/100 | 0/100 | 36.60/43.32 | —/— | — |
| mock_clear | 100/100 | 100/100 | 21.57/29.45 | 31.61/39.34 | 1.34 |

## Host host-3

Excluded warmup successes: Rustwright 32/32; Playwright 28/32.

| Scenario | Rustwright success | Playwright success | Rust median/p95 ms | Playwright median/p95 ms | Reference/Rust successful p95 |
| --- | ---: | ---: | ---: | ---: | ---: |
| delayed_click | 100/100 | 100/100 | 168.37/175.15 | 207.45/218.35 | 1.25 |
| delayed_frame | 100/100 | 100/100 | 197.01/204.34 | 258.15/267.23 | 1.31 |
| cross_site_frame | 100/100 | 100/100 | 216.48/229.35 | 261.74/272.28 | 1.19 |
| cross_site_navigation | 100/100 | 100/100 | 279.85/285.45 | 290.28/308.91 | 1.08 |
| disabled_click | 100/100 | 100/100 | 166.61/173.15 | 227.96/238.55 | 1.38 |
| covered_click | 100/100 | 100/100 | 146.88/167.30 | 228.87/238.26 | 1.42 |
| moving_click | 100/100 | 100/100 | 197.13/203.31 | 248.57/258.27 | 1.27 |
| clipped_click | 100/100 | 100/100 | 80.72/86.99 | 81.41/91.46 | 1.05 |
| rotated_clipped_click | 100/100 | 0/100 | 78.25/83.48 | —/— | — |
| http_disconnect_recovery | 100/100 | 100/100 | 17.14/21.90 | 28.23/39.64 | 1.81 |
| browser_disconnect | 100/100 | 100/100 | 43.73/52.88 | 60.88/69.29 | 1.31 |
| mock_fetch | 100/100 | 100/100 | 6.77/8.64 | 10.52/17.22 | 1.99 |
| mock_navigation | 100/100 | 100/100 | 19.61/22.37 | 27.48/38.88 | 1.74 |
| mock_frame | 100/100 | 100/100 | 50.94/57.27 | 93.86/110.21 | 1.92 |
| mock_return | 100/100 | 0/100 | 21.98/27.60 | —/— | — |
| mock_clear | 100/100 | 100/100 | 16.50/20.44 | 21.32/27.21 | 1.33 |

## Pooled raw descriptive results

These pool the recorded attempts; they are not independent population estimates.

| Scenario | Rustwright success | Playwright success | Rust median/p95 ms | Playwright median/p95 ms | Reference/Rust successful p95 |
| --- | ---: | ---: | ---: | ---: | ---: |
| delayed_click | 300/300 | 300/300 | 167.55/176.40 | 215.63/227.53 | 1.29 |
| delayed_frame | 300/300 | 300/300 | 201.57/207.95 | 280.47/296.00 | 1.42 |
| cross_site_frame | 300/300 | 300/300 | 228.80/241.38 | 283.86/301.28 | 1.25 |
| cross_site_navigation | 300/300 | 300/300 | 288.12/306.16 | 348.51/388.01 | 1.27 |
| disabled_click | 300/300 | 300/300 | 171.07/195.72 | 232.84/241.25 | 1.23 |
| covered_click | 300/300 | 300/300 | 154.42/174.84 | 229.56/239.44 | 1.37 |
| moving_click | 300/300 | 300/300 | 196.87/205.95 | 255.01/265.27 | 1.29 |
| clipped_click | 300/300 | 300/300 | 80.78/89.13 | 87.26/99.21 | 1.11 |
| rotated_clipped_click | 300/300 | 0/300 | 77.78/84.29 | —/— | — |
| http_disconnect_recovery | 300/300 | 300/300 | 23.18/31.43 | 37.49/57.45 | 1.83 |
| browser_disconnect | 300/300 | 300/300 | 44.27/52.78 | 68.95/80.73 | 1.53 |
| mock_fetch | 300/300 | 300/300 | 10.57/14.01 | 15.90/25.54 | 1.82 |
| mock_navigation | 300/300 | 300/300 | 31.83/38.09 | 36.92/54.06 | 1.42 |
| mock_frame | 300/300 | 300/300 | 61.82/73.46 | 132.14/150.92 | 2.05 |
| mock_return | 300/300 | 0/300 | 34.19/41.49 | —/— | — |
| mock_clear | 300/300 | 300/300 | 20.59/29.08 | 26.96/37.36 | 1.28 |

## Separate sampled memory

Each field has its own maximum in MiB; maxima need not be simultaneous. Missing readings remain unavailable, with counts below.

| Host | Pair | Engine | Samples | Driver RSS | Driver PSS | Descendant RSS | Descendant PSS | Missing RSS/PSS counts (driver; descendants) |
| --- | ---: | --- | ---: | ---: | ---: | ---: | ---: | --- |
| host-1 | 1 | rustwright | 193 | 7.68 | 5.50 | 3799.66 | 0.00 | 0/0; 84/192 |
| host-1 | 1 | playwright-core | 352 | 239.91 | 236.23 | 1735.17 | 0.00 | 0/0; 26/351 |
| host-1 | 2 | playwright-core | 352 | 259.44 | 255.80 | 1744.55 | 0.00 | 0/0; 31/351 |
| host-1 | 2 | rustwright | 193 | 7.78 | 5.52 | 3854.04 | 0.00 | 0/0; 67/192 |
| host-2 | 1 | rustwright | 194 | 7.52 | 5.32 | 3817.83 | 0.00 | 0/0; 68/193 |
| host-2 | 1 | playwright-core | 354 | 262.65 | 258.95 | 1756.52 | 0.00 | 0/0; 27/353 |
| host-2 | 2 | playwright-core | 352 | 264.33 | 260.63 | 1749.87 | 0.00 | 0/0; 39/351 |
| host-2 | 2 | rustwright | 192 | 7.75 | 5.56 | 3453.14 | 0.00 | 0/0; 80/191 |
| host-3 | 1 | rustwright | 163 | 7.83 | 5.44 | 3849.92 | 0.00 | 0/0; 34/162 |
| host-3 | 1 | playwright-core | 310 | 261.22 | 257.46 | 1751.14 | 0.00 | 0/0; 24/309 |
| host-3 | 2 | playwright-core | 311 | 254.36 | 250.69 | 1753.79 | 0.00 | 0/0; 27/310 |
| host-3 | 2 | rustwright | 164 | 7.91 | 5.61 | 3856.22 | 0.00 | 0/0; 46/163 |

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
