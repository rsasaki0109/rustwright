# Firefox network-idle verification

The implemented API and its scope are documented in
[the implementation record](../../../FIREFOX_NETWORK_IDLE_IMPLEMENTATION.md).
Tested production, test and workflow source is
`e3bc08e7ce0a117d173bdcde2924165806e8e854`.

- [Backend record](backend/): controlled protocol tests, original missing-API
  compilation baseline, an intermediate stale-document regression before/after,
  exact relevant source and build checks. Historical binaries are identified;
  compiled artifacts and caches are excluded.
- [Local record](local/): exact-source 193 library and 193 HTTP passes
  (79 portable plus 114 Linux additions), actual eight-archive verification,
  four native consumer cases and two README entry points. Local launchers use
  container sandbox adjustments; engine versions are Chrome 151/Firefox 157.
- [Remote record](remote/): all six successful jobs of
  [run 37974358667](https://github.com/rsasaki0109/rustwright/actions/runs/37974358667),
  79 portable cases on each native OS with Chrome 155/Firefox 157,
  114 additional Ubuntu HTTP cases, 453 workspace passes and one existing
  intentionally ignored macro doctest. Package consumers and README programs
  execute on Ubuntu. Official artifact digests and exact Git source are checked.

The 26 parity cases and 22 new backend cases are included in those totals.
Repeated local/remote jobs overlap; do not sum them as distinct coverage.
The missing-method baseline is a compilation result, not a native browser failure.
The intermediate failing regression concerns the developing implementation.

Each child record is frozen independently. Verify its root-relative manifest
from the repository root, for example:

```sh
sha256sum -c docs/ci/results/firefox-network-idle/local/SHA256SUMS
sha256sum -c docs/ci/results/firefox-network-idle/backend/SHA256SUMS
sha256sum -c docs/ci/results/firefox-network-idle/remote/SHA256SUMS
```

Later documentation-only commits do not replace the tested source identity.
These records do not establish memory attribution, real-site/headed behavior,
broader version compatibility or general SOTA performance.
