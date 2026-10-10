# Rustwright v0.1.0 distribution and verification

These immutable distribution files belong to Git tag `v0.1.0`, release commit
`6ca2d4228d24616364cbcf413f227c2c2c8f11fb`. The files are added to the repository
after that tested source commit; this evidence addition changes no runtime source.

[Main release CI](https://github.com/rsasaki0109/rustwright/actions/runs/38008971390)
passed all seven jobs: 480 workspace passes, zero failures/filtered tests and
one existing ignored macro doctest, plus native/platform/version/package scopes.
All seven actual checkout logs and six official artifact ZIP digests were checked.

The normal GitHub asset-upload endpoint returned HTTP 401 with the environment's
available authentication. The release therefore links directly to these repository
downloads; automatic GitHub source ZIP/tarball downloads remain available.
The eight `.crate` files are verified Cargo source archives, not crates.io uploads.

| Download | Purpose |
| --- | --- |
| [RELEASE_VERIFICATION.json](RELEASE_VERIFICATION.json) | Exact source, CI, counts and package provenance |
| [SHA256SUMS](SHA256SUMS) | Checksums for all ten distribution/verification payloads |
| [rustwright-0.1.0.crate](rustwright-0.1.0.crate) | Verified Cargo package archive |
| [rustwright-bidi-0.1.0.crate](rustwright-bidi-0.1.0.crate) | Verified Cargo package archive |
| [rustwright-browser-0.1.0.crate](rustwright-browser-0.1.0.crate) | Verified Cargo package archive |
| [rustwright-cdp-0.1.0.crate](rustwright-cdp-0.1.0.crate) | Verified Cargo package archive |
| [rustwright-common-0.1.0.crate](rustwright-common-0.1.0.crate) | Verified Cargo package archive |
| [rustwright-core-0.1.0.crate](rustwright-core-0.1.0.crate) | Verified Cargo package archive |
| [rustwright-test-0.1.0.crate](rustwright-test-0.1.0.crate) | Verified Cargo package archive |
| [rustwright-test-macros-0.1.0.crate](rustwright-test-macros-0.1.0.crate) | Verified Cargo package archive |
| [rustwright-v0.1.0-ci-evidence.zip](rustwright-v0.1.0-ci-evidence.zip) | All original CI artifacts/logs and independent audits |

Download all payloads and `SHA256SUMS` into one directory, then verify:

```sh
sha256sum --check SHA256SUMS
```

The manifest covers every payload except itself and this explanatory README.
The evidence ZIP preserves raw original logs and their whitespace. The package
archives record the clean release commit in `.cargo_vcs_info.json` and include
the version-pinned package README, both license texts and required source assets.

The prior local fixture comparison remains scoped to sixteen Chromium cases;
general SOTA, complete-browser memory advantage and universal leak-freedom are
not established by this release. See the main comparison and compatibility records.
