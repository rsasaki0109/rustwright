# Frozen independent Firefox network probe

This is a new immutable evidence set for the read-only investigation in
[Firefox network-idle design](../../../FIREFOX_NETWORK_IDLE.md). It is not an
implementation or a cross-platform verification result.

The 23 original artifact files and their original `sha256.json` were copied
byte-for-byte from `target/firefox-network-probe` after checking every hash.
`SHA256SUMS` verifies all 24 copied files. `archive-files.sha256` additionally
covers these archive instructions and the permanent design document.

| File | Purpose |
| --- | --- |
| `run1/probe.py` | Exact first-run source, before the extended cases were added. |
| `run2/probe.py`, `run3/probe.py` | Exact sources of the extended run and first-ever late-subscription run. |
| `run*/events.jsonl` | Timestamped commands, replies, raw events, fixture headers/body ends, metadata and cleanup. |
| `run*/firefox.stderr` | Native browser stderr. |
| `run1/run.log`, `run2.log`, `run3.log` | Original command output; empty because successful probes log events to JSONL. |
| `summary.json` | Validated observations, timings, Firefox version and ELF hash. |
| `summarize.py` | Exact observation-assertion and original source-snapshot script. |
| `inspected-source/` | Exact production sources inspected during the investigation; not an implementation change. |
| `DESIGN.md` | Original exploratory design note. |

Firefox reported 157.0.1. Its actual `/proc/<pid>/exe` resolved to
`/workspace/.rustwright-env/firefox/firefox-bin`, SHA-256
`b3089612093d427f3912dead43f377e65c709e8da4496c2912a84b447a368779`.
Every run records its launch command and disposable profile path. Python was
3.12.14, websockets 16.0. Browser binaries and external Python dependencies are
not bundled. The version/hash and local commands identify the tested environment;
a different browser build may produce different observations.

## Verify without rerunning

From the repository root:

```sh
(cd docs/ci/results/firefox-network-probe && sha256sum -c SHA256SUMS)
sha256sum -c docs/ci/results/firefox-network-probe/archive-files.sha256
```

The raw native probes completed successfully. The subsequent assertion script
validated 5 / 9 / 5 observations across three sessions, with repetition between
sessions. These counts are not unique tests and do not exercise a production
network-idle API.

## Reproduce in a new output directory

**Do not run the scripts inside this frozen directory.** Their default output
is beside the script and the original summarizer writes snapshots and hashes.
The following copies them into a fresh writable target directory. Provide an
installed Firefox executable and Python with websockets 16.0 first; the original
cloud environment used `/workspace/.rustwright-env/activate.sh` for its browser
path and environment configuration.

```sh
export RUSTWRIGHT_FIREFOX=/absolute/path/to/firefox
archive_dir=docs/ci/results/firefox-network-probe
replay_dir="$(mktemp -d target/firefox-network-replay.XXXXXX)"
mkdir -p "$replay_dir/run1"
cp "$archive_dir/run1/probe.py" "$replay_dir/run1/probe.py"
cp "$archive_dir/probe.py" "$replay_dir/probe.py"
cp "$archive_dir/fresh-subscription.py" "$replay_dir/fresh-subscription.py"
cp "$archive_dir/summarize.py" "$replay_dir/summarize.py"
python3 "$replay_dir/run1/probe.py" > "$replay_dir/run1/run.log" 2>&1
PROBE_OUTPUT="$replay_dir/run2" python3 "$replay_dir/probe.py" > "$replay_dir/run2.log" 2>&1
PROBE_OUTPUT="$replay_dir/run3" python3 "$replay_dir/fresh-subscription.py" > "$replay_dir/run3.log" 2>&1
python3 "$replay_dir/summarize.py"
```

Run these commands from the repository root so the original summarizer's
production-source snapshot paths resolve correctly. All HTTP fixtures use
loopback addresses and dynamically allocated ports. Each launch uses a fresh
profile and cleans up its browser process and fixture tasks. The raw capture
contains process/profile IDs and loopback ports, so a rerun is not expected to
produce identical event-file hashes. Preserved hashes establish the provenance
of the original observations.
