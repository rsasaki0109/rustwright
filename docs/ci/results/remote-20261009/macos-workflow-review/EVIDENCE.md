# macOS setup workflow review

Read-only review of the root's intact-bundle setup step. The embedded Python parses successfully; exact selected version and RUNNER_ARCH mapping create the published CfT URLs; bundle/executable paths match the independently inspected authoritative mac-arm64155 archive. System ditto preserves original app layout and links; codesign verification precedes output. curl is bounded120 seconds and the step3 minutes. SHA256/plist/framework-link observations are retained. No engine or sandbox settings change.

The Windows setup block exactly matches current HEAD and its successful native run. The portable environment selects the new Mac executable when its step runs, otherwise the original action executable. No blocking issue was found. Actual corrected Mac native execution is still pending.
