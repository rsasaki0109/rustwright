# Frame helper cache correction

Owned changes: `crates/rustwright-bidi/src/browser.rs` (only helper-context cache
and document-check paths) and new `frame_helper_cache_tests.rs`.

Removed `PageHelper.contexts: HashSet<String>`. Every locator helper check now
evaluates the compact Boolean `!!window.__rustwright` in the current document;
the shared helper is injected only if absent. Live documents reuse their existing
helper without reinjection. Preload registration ownership is unchanged. This
adds one small evaluate roundtrip per helper check and a second evaluate when
injection is necessary; earlier performance comparisons predate this change.

An independent baseline workspace was extracted from the parent's frozen source
archive and given the new tests plus one before-only assertion against the old
private context set. It was compiled into **baseline-target**, never the current
workspace's target directory. No production source was restored to old code.

- Same four behavioral tests: **3 fail / 1 pass before; all 4 pass after**.
- Baseline-only retention assertion: after 1,000 different frames are injected
  then destroyed, the remote has one live document but the old set retains
  **1,001 context IDs**. The bounded-history assertion fails.
- Current full BiDi library tests: **40 passed**.
- Current BiDi all-target Clippy with `-D warnings`: passed.

Final behavioral tests cover live-document reuse, a missing helper after same-ID
document replacement, a destroyed frame's typed protocol error, and injection
acknowledgment arriving after document destruction/replacement. The baseline-only
retention test is deliberately not present after removal of the field; no fake
counter or tautological after-test was introduced.

The mock controls document replacement explicitly; it does not establish that
native Firefox always fails to apply preloads to navigation. A real iframe churn
test and final integrated validation are the parent's responsibility. Probe and
locator evaluation remain separate commands, so a document may change between
them; this correction eliminates historical cache retention, not every possible
navigation race. It does not measure browser heap reachability or explain prior
raw/basic Firefox browser PSS increases.

The evidence directory contains before/after logs, exact test/source snapshots,
source SHA256 metadata, and a scoped `browser.patch`. The old-production snapshot
is `browser-before.rs`; the actual baseline source adds the test module declaration.
Do not include large `baseline-target` build artifacts in published evidence.
