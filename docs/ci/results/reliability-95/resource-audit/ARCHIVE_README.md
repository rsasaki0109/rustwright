# Frozen resource-audit evidence

All 28 original target-directory files (27 hashed artifacts plus their original `SHA256SUMS`) are copied byte for byte, including the historical scope README/report and baseline/after logs. Original manifests remain unchanged and still verify from this directory. `ROOTED_SHA256SUMS` additionally covers every actual archive file with paths relative to the repository root. No compiled test executable is copied.

The added regressions fail three times on the original implementation and pass in the 111-test fixed BiDi suite. All seven after-source snapshots, including unchanged session.rs, match production commit 908323d38ba036960c41347ecbe2a5dfa8c24271. The recorded target report correctly says MSRV/native checks were deferred at that stage; later parent exact-commit checks are separate evidence. The control-peer run and memory experiment are different measurements.

Unknown remote subscription IDs after lost ACKs remain outside the fix; task termination does not prove all heap bytes reclaimed.
