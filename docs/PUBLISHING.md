# Publishing

How to release the Rustwright crates, and the current name situation.

## crates.io name availability

As of this writing, two intended names are **already taken on crates.io**:

| Crate | Status |
|---|---|
| `rustwright` | taken (0.1.1, by an unrelated project) |
| `rustwright-core` | taken (0.1.1, by an unrelated project) |
| `rustwright-cdp`, `rustwright-browser`, `rustwright-common`, `rustwright-bidi`, `rustwright-test`, `rustwright-test-macros` | available |
| `rustwright-rs`, `rustwright-automation`, `rustwright-engine` | available |

So the facade cannot be published as `rustwright`, and `rustwright-core` cannot
keep its name either.

### Options

1. **Stay git-only (current).** Consumers depend on the repository by path/git.
   Nothing to publish; no naming constraints.
2. **Rename the two taken packages, keeping import paths.** Cargo allows a
   package name to differ from its library name:

   ```toml
   [package]
   name = "rustwright-rs"        # unique on crates.io

   [lib]
   name = "rustwright"           # `use rustwright::prelude::*;` still works
   ```

   Apply the same trick to `rustwright-core` (for example package
   `rustwright-engine`, lib `rustwright_core`) and update internal path
   dependencies. This preserves the public import paths while satisfying
   crates.io.

## Publish order

Path dependencies must be published first. Order:

1. `rustwright-common`
2. `rustwright-cdp`
3. `rustwright-browser` (depends on `rustwright-cdp`)
4. `rustwright-core` (depends on `rustwright-browser`, `rustwright-cdp`, `rustwright-common`)
5. `rustwright-bidi` (depends on `rustwright-browser`, `rustwright-common`)
6. `rustwright-test-macros`
7. `rustwright` (facade)
8. `rustwright-test` (depends on `rustwright` and `rustwright-test-macros`)

## Local validation

`cargo package` still resolves the dependency graph, so a crate whose
dependencies are not yet on crates.io cannot be fully packaged locally (a
`[patch.crates-io]` mapping does not help here, because `cargo package`
re-resolves against the registry). In practice:

- `rustwright-common`, `rustwright-cdp` and `rustwright-test-macros` have only
  external dependencies and validate with `cargo package --no-verify`:

  ```sh
  cargo package -p rustwright-common --no-verify
  cargo package -p rustwright-cdp --no-verify
  cargo package -p rustwright-test-macros --no-verify
  ```

- The remaining crates (`rustwright-browser`, `rustwright-core`,
  `rustwright-bidi`, `rustwright`, `rustwright-test`) can only be validated once
  their Rustwright dependencies are published. Use `cargo publish --dry-run` for
  each, in the order above, after the previous ones land.

## Actual publish

Requires a crates.io API token (`cargo login`) and, for the README badges and
docs.rs, a **public** repository. Then publish in the order above:

```sh
cargo publish -p rustwright-common
# ... wait for the index to update, then the next crate ...
```

`cargo publish --dry-run` cannot fully verify unpublished path dependencies, so
publish them one at a time in order.

## Checklist

- [ ] Decide on the names in the table above.
- [ ] `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`.
- [ ] Update `CHANGELOG.md` and bump versions in the workspace manifest.
- [ ] `cargo package` validation passes for every crate.
- [ ] `cargo login` with a crates.io token.
- [ ] Publish the crates in order.
- [ ] Tag the release (`git tag vX.Y.Z && git push --tags`).
