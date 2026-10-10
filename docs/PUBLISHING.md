# Packaging and publishing

Version 0.1.0 is distributed as a GitHub source release and the `v0.1.0` Git tag.
Use `rustwright = { git = "https://github.com/rsasaki0109/rustwright", tag = "v0.1.0" }`
or an adjacent-checkout path dependency. Downloadable `.crate` assets are verified
archives, not registry uploads. See the [release notes](../CHANGELOG.md#010--2026-10-10).

The current distribution uses this repository through path/git dependencies.
Local package verification does not upload a crate, reserve a name or establish
crates.io ownership. No package names or versions were changed by these checks.

## Registry names

Earlier publishing notes recorded `rustwright` and `rustwright-core` at version
0.1.1 on crates.io as unrelated projects, while the other intended names were
available. A read-only refresh on 2026-10-09 returned HTTP 403 in this environment;
current ownership and name availability remain unverified. Recheck the registry
before making a publishing decision. Do not assume this repository's crate can
be installed with `rustwright = "0.1.0"` from crates.io.

Path/git dependencies keep the current package and Rust import names. If a
registry release is desired, resolve the name conflicts first. Cargo permits a
package name such as `rustwright-rs` with `[lib] name = "rustwright"`, but a rename
also requires updating internal dependencies and validating the new packages.
This is a separate release decision.

## Reproducible local checks

With cached locked dependencies, Python 3.11+, a recent stable Cargo supporting
multi-package packaging, and Rust 1.85.0 installed, run:

```sh
python3 scripts/release_check.py
```

The verifier selects the eight distributable crates explicitly, runs
`cargo package --allow-dirty --offline --locked` with verification enabled,
audits each real `.crate` archive, and constructs a local registry from those
archives plus cached third-party archives. Third-party archive hashes must match
the original workspace lockfile before they enter that registry. Root
`Cargo.lock` must remain unchanged.

The external consumer uses exact version dependencies, with no path dependencies
or `[patch]` overrides. Metadata and lockfile checks verify that all eight
Rustwright packages were resolved from the actual archive checksums. The consumer
compiles on stable and Rust 1.85.0, checks the packaged injected helper, compiles
the test macro, and compiles every Rust fence extracted from the project README.
Illustrative fragments receive only imports and function context; standalone
entry points and the generic helper are copied verbatim.

The recorded packaging tool is Cargo 1.99.0. Rust 1.85.0 is verified for the
workspace and extracted consumer; this does not claim that Cargo 1.85 itself can
run the newer multi-package packaging command. Older per-crate packaging may
still fail when dependencies are unpublished. The previous `--no-verify` recipe
has been replaced by verified staging of the complete local crate graph.

For native consumer checks, pin both installed browsers and run:

```sh
RUSTWRIGHT_CHROME=/path/to/chrome \
RUSTWRIGHT_FIREFOX=/path/to/firefox \
python3 scripts/release_check.py --run-browser-tests
```

This runs a local HTTP fixture separately on Chrome and Firefox, with disposable
profiles, headless mode, zero retries and no shard filtering. It checks Unicode
input, a trusted click, screenshot content, request mocking/clearing and the
procedural-macro runner. Required browser absence or startup failure fails the
check. Run `python3 scripts/release/readme_runtime.py` afterward to execute the
verbatim main README entry points against a local fixture; external
website and visible-window examples require separate validation.

Outputs, normalized manifests, consumer metadata, lockfiles, logs, archive
checksums and generated README snippets go under `target/release-check/` by
default. The [release verification record](RELEASE_VERIFICATION.md) states the
executed checks and their limits.

Each distributable archive includes a compact `PACKAGE_README.md` with absolute
documentation links, both license texts, and its required source assets. License
copies are audited against the root originals. The helper JavaScript in
`rustwright-common` is also checked against its source. Update the license copies
when the root license texts change.

## An actual registry release

After resolving name ownership and choosing a reviewed release version, publish
dependencies before their dependents:

1. `rustwright-common`, `rustwright-cdp`, `rustwright-test-macros`.
2. `rustwright-browser`.
3. `rustwright-core`, `rustwright-bidi`.
4. `rustwright`.
5. `rustwright-test`.

Update this order if packages are renamed. Real publication additionally requires
registry authentication, indexing of published dependencies, CI results and a
release/tag decision. Offline local staging establishes none of those. Use
registry dry runs once prerequisites exist, then publish the reviewed crates in
dependency order and tag the chosen release. Do not use local verification
success as evidence that these steps have happened.
