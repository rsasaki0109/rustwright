# Strict legacy browser test policy

Scope: test-only browser selection and startup policy in six legacy integration targets.
No production browser discovery behavior, test manifests, workflows, or immutable historical evidence was changed by this task.

`RUSTWRIGHT_CHROME` / `RUSTWRIGHT_FIREFOX` presence makes that backend mandatory, including an empty value. Configured paths must identify a file; invalid overrides cannot quietly fall back to another installed browser. `RUSTWRIGHT_REQUIRE_BROWSERS=1` makes both backends mandatory. Optional discovery and legacy Firefox launch skips remain when neither condition applies.

Before: actual legacy `bidi::firefox_bidi_network_monitoring` and `unified::same_code_on_firefox` selected the executable exit42 stub, skipped the startup failure, exited0, and reported one passing test. `before/` retains sources, executable hashes, logs and case status.

After: all 16 real legacy negative cases exited101 with exactly one failed test: seven missing override cases, seven existing-file executable exit42 stub cases, and two empty overrides. These cover all six modified targets and both unified backends. `negative-cases.json` retains commands, explicit bindings and logs.

Eight deterministic helper subprocess cases compiled with Rust1.85 verify optional versus required discovery and launch errors for both backend keys. These synthetic error checks are separate from native browser tests; `policy-probe-cases.json` records their scope.

Four required native cases pass without skipping: legacy Firefox HTTP network monitoring; Chrome HTTP API mocking; Chrome HTTP cookie, navigation and network diagnostics; Chrome launch diagnostics. The latter does not navigate; the other three use local HTTP. Legacy file/data-URL scenarios were not run positively because this environment blocks those URL types. `positive-cases.json` retains exact commands and logs.

Scoped stable Clippy with denied warnings, Rust1.85 checks of all six modified integration targets, rustfmt checks and whitespace checks pass. `after-source/` and `after-source-sha256.json` freeze the final seven edited files; `after-executables.json` binds the native runs to binaries.

Native evidence is Linux-only. Windows/macOS execution requires the remote CI matrix. No registry publication, push or SOTA claim is included.
