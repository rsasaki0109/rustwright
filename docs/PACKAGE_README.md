# Rustwright

Rustwright provides browser automation for Rust, with Chrome over CDP and Firefox over WebDriver BiDi. It uses browser protocol input for clicks and typing, and exposes shared page and locator APIs.

The crate family includes `rustwright` (the public facade), `rustwright-core` (Chrome automation), `rustwright-bidi` (Firefox automation), `rustwright-browser` (browser launching), `rustwright-cdp` (CDP transport), `rustwright-common` (shared types and page helpers), `rustwright-test` (browser test runner), and `rustwright-test-macros` (its procedural macro).

Rust 1.85 or later is required. A supported browser must be installed to run browser automation. Use the v0.1.0 GitHub source release:

```toml
[dependencies]
rustwright = { git = "https://github.com/rsasaki0109/rustwright", tag = "v0.1.0" }
```

See the [release notes](https://github.com/rsasaki0109/rustwright/releases/tag/v0.1.0), [project README](https://github.com/rsasaki0109/rustwright/blob/v0.1.0/README.md) for usage, the [compatibility matrix](https://github.com/rsasaki0109/rustwright/blob/v0.1.0/docs/COMPATIBILITY_MATRIX.md) for verified behavior and limits, and the [reliability roadmap](https://github.com/rsasaki0109/rustwright/blob/v0.1.0/docs/RELIABILITY_ROADMAP.md) for remaining work. This is a GitHub source release; it does not establish crates.io name ownership or registry publication.

Licensed under either the [MIT license](https://github.com/rsasaki0109/rustwright/blob/main/LICENSE-MIT) or [Apache License 2.0](https://github.com/rsasaki0109/rustwright/blob/main/LICENSE-APACHE), at your option. Both license texts are included in each crate archive.
