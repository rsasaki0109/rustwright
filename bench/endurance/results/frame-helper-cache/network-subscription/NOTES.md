# Acknowledged network setup

Changed `session.rs` network subscription setup, `browser.rs` monitoring setup,
`network.rs` receiver acquisition, and new `network_subscription_tests.rs`.
Routing and remote intercept ownership are unchanged.

Optimistic atomic ready flags were replaced with shared async-mutex-protected
booleans. Detached workers serialize handshake/setup and publish readiness only
after successful acknowledgment; monitoring also installs its one pump before
publishing readiness. Caller cancellation leaves the worker running. Failed
setup retains false readiness so a later or already queued caller may retry;
protocol failures retain their typed error. Repeated successful calls reuse the
subscription/pump. Receiver acquisition precedes spawning the monitoring pump,
so readiness cannot precede registration of its local event receiver.

Each worker has an overall **five-second local deadline**, including waiting for
the setup lock. This is not remote subscription registry reclamation: a lost
acknowledgment may conceal a successful browser-side registration, and a retry
may create another subscription. No complete remote subscription/intercept census
or guarantee of delivery during setup is claimed.

The same six tests ran in an isolated before workspace and in the live corrected
workspace: **5 failed / 1 passed before; 6 passed after**. Tests cover concurrent
acknowledgment waiting, cancelled subscription waiters, rejection followed by
queued retry, cancelled monitoring setup followed by 20 concurrent starts reusing
the same pump task ID, rejection without a failed pump, and bounded silent-peer
waiting with local pending-request cleanup. Event delivery is also checked.
The silent-peer test advances the virtual Tokio clock after command receipt.

Current full BiDi library tests: **46 passed**. Scoped all-target Clippy output is
in `clippy.log`. The pump task-ID accessor is compiled only for unit tests; it
verifies pump identity rather than introducing a production diagnostic API.

Before builds used `baseline-target`, separate from the live target directory.
Original production copies have `*-before.rs` names; actual baseline files add
only the new test module declaration and test-only task-ID accessor. Executed
before/after copies, identical tests, source hashes, scoped patches and logs are
preserved. No production file was restored to old code during verification.

Parent-owned native HTTP tests and integrated/MSRV verification are separate.
Known remote intercept lifetime/removal issues and unbounded opt-in full-history
request retention remain outside this correction. This work does not explain
the earlier raw/basic Firefox browser PSS increases.
