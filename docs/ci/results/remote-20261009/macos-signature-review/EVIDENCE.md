# Introduced macOS strict signature-check failure

Run37951867466 macOS setup fails at our added codesign --verify --deep --strict gate, exit1: code has no resources but signature indicates they must be present. This occurs before any required native scenario executes and must not be reported as a Rustwright native driver-test failure.

The independently TLS-inspected official CfT155 mac-arm64 ZIP contains673 central-directory entries and no _CodeSignature or CodeResources entry anywhere. The exact layout digest/URL/selected entry observations are retained. This is compatible with the observed inability to verify a complete application resource seal. The review does not claim all Mach-O executables are unsigned: embedded executable signatures are a separate question and were not inspected here.

Official CfT README describes Mac archive installation using @puppeteer/browsers/curl/wget and a distinct browser-download quarantine issue. It does not impose strict resource-seal verification or explain why this archive lacks seal files. The reviewed Chrome signing source discusses signing complete application/framework parts, but does not explain this public ZIP's omission. Its packaging rationale remains unverified. No quarantine changes are proposed.

Root's correction should record codesign exit/output as metadata while retaining the untouched official archive/app bytes and browser sandbox. The original archive absence cannot be repaired by asserting a stricter local gate; no re-signing, signature removal, code modification or disabled sandbox is proposed. Corrected app-layout native tests remain necessary to verify Rustwright operation.

This was a read-only review; production/test/workflow sources were not edited. Exact remote log digest, excerpt, source URLs/hashes and archive metadata are recorded separately from prior immutable review evidence. checksum-manifest.sha256 freezes these files.
