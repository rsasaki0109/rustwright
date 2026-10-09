# Native startup cancellation observations

Ten Chrome for Testing 155.0.8059.39 launches and ten Firefox 157.0.1 launches were cancelled before the public launch API returned. **All 20 direct-child and profile-ownership checks succeeded.** The external consumer was built and run without editing repository production source or adding a repository test target.

Each cycle has its own small PID-recording shell wrapper, which immediately `exec`s a byte-identical copy of the existing container launcher. It records its PID and exact arguments. There are no sleeps, held sockets, fake readiness messages, or browser-behavior gates in the wrapper. A separate async observer polls `/proc/PID/exe` and cancels only after seeing the actual installed Chrome or Firefox executable. This verifies native executable loading; it does not prove how far browser initialization progressed.

The consumer uses a current-thread Tokio runtime. After it observes the native executable, it checks that the launch task is unfinished and its API handoff flag is false, then aborts with no intervening await. The launch task therefore cannot advance between the pre-handoff check and cancellation. Ten seconds bound native observation; six seconds bound awaiting the cancelled task. A later independent verification rejects any missing or failed assertion.

Every retained result records:

- The actual executable path, native PID, and pre-cancellation `/proc` status proving the browser was the observer's direct child.
- Successful task cancellation with the public API handoff flag still false.
- The direct child's `/proc` directory absent afterward, and `waitpid(WNOHANG)` returning `-1/ECHILD` (Linux errno 10). The library had already reaped this direct child; the observer did not reap a leftover zombie.
- Chrome's launcher-created ephemeral profile path under the cycle-owned `TMPDIR`, absent after cancellation.
- Firefox's explicit caller-owned profile and unchanged sentinel bytes present after cancellation. That fixture-owned profile is removed only after the result is recorded.

No native cycle missed the pre-handoff window, failed an assertion, or required an observer cleanup kill/reap. `summary.json`, per-backend summaries, exact JSONL, and 20 per-cycle `result.json` files preserve the observations. The original arguments and wrapper source are retained alongside each result. Startup log files are copied where emitted; an empty log is not an initialization success claim.

## Scope and identity

These observations concern the **direct actual browser child and local profile ownership** during very early startup. They do not establish descendant/process-tree cleanup, later startup-stage cancellation behavior, startup latency, or universal leak freedom. The original controlled-peer/unit regressions cover other protocol ordering and failure conditions; these 20 native cycles are additional observations, not 20 new unique test designs.

`compiled-identity.json` records the actual external-consumer binary SHA/size and every Rust source SHA. `compiled-source/` preserves those exact source bytes and the workspace/crate manifests and lockfile; the consumer's own manifest/lockfile/source remain under `consumer/`. All recorded Rust sources were compared with the captured checkout after the isolated build, and `source-vs-908323d.json` records the relation to the Git HEAD during that build. The uncommitted Chrome context fix is represented by its actual source hashes; it is not silently attributed to `908323d` or a later commit. Firefox and process/startup sources are unchanged from `908323d`.

`browser-identities.json` records the actual native binaries, hashes, sizes, version commands/output, and container-launcher hashes. `compiled-identity.json` identifies the consumer executable, which is excluded together with Cargo build caches and dependency ELF/rlib files. `rustc-version.log` records the toolchain. `build-command.json` and `run-commands.json` preserve exact commands, and `run.py`/`verify.py` preserve observer orchestration and independent assertions.

## Reproduction and preserved setup failures

Activate `/workspace/.rustwright-env/activate.sh`, build the consumer with its standalone Cargo manifest and a separate target directory, and run `run.py` from a **new** output tree after adapting only its fixed local paths to the replicated environment. The original hard-coded repository/container paths are preserved in source and command records. The production library source must match `compiled-source/`, rather than an unspecified later checkout. `verify.py` checks recorded results and exits nonzero on any missing native, cancellation, parentage, reap, or profile assertion.

The first external-consumer build used an incorrect `rustwright::firefox` import and failed with E0432. Its source and exit-101 log remain under `attempts/compile-1/`. The consumer was corrected to the existing public `rustwright::BidiBrowser` and `rustwright::browser::Firefox` exports. A version query initially targeted a non-executable launcher source file; `attempts/setup-version-failure.json` records that error and the use of an executable owned copy with identical bytes. These setup errors occurred before native cycles and are not counted as product failures or as passed native observations.

`copied-originals.json` maps every byte-identical copy to its original local path/hash/size. `SHA256SUMS` covers all actual archive files except itself. Existing frozen evidence directories were not modified.
