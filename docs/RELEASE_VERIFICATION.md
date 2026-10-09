# Local release verification — 2026-10-09

This is the historical first local release-preparation record. Later source-specific
distribution and native three-OS CI results are recorded in
[CI_VERIFICATION.md](CI_VERIFICATION.md) and the
[reliability checkpoint](RELIABILITY_95_CHECKPOINT.md). The original results below
retain their recorded scope and source identity.

The eight distributable crates now contain their package README and both license
texts. Real Cargo archives build successfully and work in an external consumer
on stable Rust and Rust 1.85.0. Chrome and Firefox execute the packaged facade,
test runner and README entry points against local HTTP fixtures.

At this recorded stage, registry publication, current crate-name ownership,
remote CI and other operating systems were unverified. Registry publication and
name ownership remain separate from later CI evidence. See
[PUBLISHING.md](PUBLISHING.md) before planning an actual release.

## Executed checks

| Check | Result | Evidence |
| --- | --- | --- |
| Original archives | Cargo verification passed; 32 required README/license asset checks failed across eight crates | [before audit](release/results/local-package/before-audit/archive-audit.json), [package log](release/results/local-package/before-package.log) |
| Final archives | 8/8 pass Cargo build verification and archive audits; source inputs match the current checkout | [report](release/results/local-package/report.json), [source comparison](release/results/local-package/archive-source-match.json) |
| Third-party archives | All 57 archive hashes match the original workspace lockfile | [checksums](release/results/local-package/registry-checksums.json) |
| Version-only external consumer | All targets compile and the browser-free facade runs on stable and Rust 1.85.0; all eight Rustwright checksums match the actual archives | [stable compilation](release/results/local-package/consumer-stable-compile.log), [MSRV compilation](release/results/local-package/consumer-msrv-compile.log), [consumer lockfile](release/results/local-package/consumer/Cargo.lock) |
| Packaged native consumer | Chrome 2/2 and Firefox 2/2 pass, including real macro callbacks | [Chrome](release/results/local-package/consumer-chrome-native.log), [Firefox](release/results/local-package/consumer-firefox-native.log) |
| README Rust fences | All nine fences compile on both toolchains; the standalone entry points and generic helper are extracted verbatim | [extraction hashes](release/results/local-package/consumer/readme-snippets.json) |
| README native entry points | Chrome and Firefox 2/2 pass against the local fixture; Firefox reads its heading and produces a fresh PNG | [runtime record](release/results/local-package/readme-runtime.json) |
| README installation dependencies | Both TOML fences, the first Rust entry point and runner example compile in a separate adjacent-checkout path consumer on stable and Rust 1.85.0 | [path project](release/results/local-package/path-consumer/Cargo.toml), [stable](release/results/local-package/path-consumer-offline-final.log), [MSRV](release/results/local-package/path-consumer-msrv.log) |
| Example programs | `offline_smoke`, `quickstart` and `bidi_firefox` pass native headless execution | [statuses](release/results/local-package/examples/after-status.json), [screenshots](release/results/local-package/examples/screenshots.json) |
| Example CLI | 10/10 expected outcomes: three successful help commands and seven rejected invalid invocations | [statuses](release/results/local-package/examples/after-status.json) |
| Workspace checks | Clippy with warnings denied, Rust 1.85 all-target check, formatting and whitespace pass | [final logs](release/results/local-package/final/) |

The synthetic local registry contains the actual eight `.crate` files and only
the 57 verified locked dependency archives. The consumer has an isolated Cargo
home, exact version dependencies, and no path dependencies or `[patch]` entries.
Cargo's metadata retains the crates.io source identity under source replacement;
the manifest locations and lockfile checksums identify the local archived inputs.
The report records this distinction. Archive copies and SHA-256 records are
retained, so the evidence does not depend on the build cache remaining available.

Native consumer checks use explicitly selected installed browsers, headless mode,
fresh disposable profiles, no shard filtering and zero retries. They check
Unicode input, exactly one trusted click, screenshot dimensions, request mocking
and clearing, zero pending protocol commands, and procedural-macro callbacks.
The generic helper executed here is the helper extracted from the README.
Illustrative API fragments and the README runner example compile; that runner's
external-site test was not executed.

## Fixed example and installation failures

The original `offline_smoke` failed on Chrome with
`Navigation("net::ERR_BLOCKED_BY_ADMINISTRATOR")` while loading `file://`.
Its original source, binary hash, status and log are preserved in
[the example evidence](release/results/local-package/examples/offline-before.log).
It now serves its embedded fixture over ephemeral loopback HTTP and checks
Unicode input, a trusted click, PNG output and zero JavaScript errors. This needs
the local network stack but no external website. The original binary itself was
not retained.

`quickstart` and `bidi_firefox` now accept a URL, `--headless`, `--profile` and
`--screenshot`; their headed defaults remain available. The README supplies the
correct path dependency for a project adjacent to this checkout, documents the
runner dependency, and no longer assumes `example.com` contains a search form.

The first offline path-consumer check lacked cached `parking_lot`, enabled by the
README's Tokio `full` features. A normal online Cargo check fetched the missing
dependencies and succeeded; subsequent locked offline stable and MSRV checks
also succeeded. [The initial log](release/results/local-package/path-consumer.log)
and [online log](release/results/local-package/path-consumer-online.log) preserve
this cache limitation. The workspace lockfile did not change during this work;
the path consumer has its own lockfile and dependency graph.

## Reproduction and provenance

Activate the repository's Rust environment, or provide equivalent toolchains and
browser paths, then run:

```sh
python3 scripts/release_check.py --run-browser-tests
python3 scripts/release/readme_runtime.py
```

Packaging used Cargo/Rust 1.99.0; MSRV checks used Rust 1.85.0. Native checks used
Chrome 151.0.7922.173 and Firefox 157.0.1 on Linux. Browser launch wrappers adapt
to the cloud sandbox; the browser engines were not patched. Registry-name API
requests returned HTTP 403, while Cargo dependency fetching succeeded.

The [frozen evidence](release/results/local-package/) contains actual before and
after archives, consumer sources and lockfiles, normalized manifests, logs,
screenshots, executable hashes and a source snapshot. `SHA256SUMS` covers the
retained files; `source.sha256` identifies the source snapshot inputs. Absolute
paths in raw logs and Cargo metadata are historical execution locations; rerun
the harness to generate a usable registry configuration in another workspace.
The packaging agent's earlier source hashes are retained separately as a stage
record; the final harness gained fresh-profile handling and native execution.

No version bump, name change, registry upload or Git publication occurred. This
run did not repeat the earlier 158 library tests or 160 native HTTP tests. It
does not establish visible-window behavior, external-site compatibility,
Windows/macOS results, remote CI success, or general memory-leak freedom. Those
remain separate reliability checks in the
[roadmap](RELIABILITY_ROADMAP.md).
