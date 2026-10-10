Frozen local packaging evidence, 2026-10-09.

See ../../../RELEASE_VERIFICATION.md for results, reproduction and limits.

Logs preserve original execution paths. consumer/.cargo/config.toml is a historical registry configuration, not a portable registry: rerun scripts/release_check.py in the source snapshot with cached dependencies. The 57 dependency archive checksums are retained; the dependency archives themselves are not duplicated here. The adjacent-checkout path consumer needs a sibling rustwright checkout when rerun.

SHA256SUMS covers retained files. source.sha256 identifies source.tar.gz inputs. executables.sha256 records built binaries; binaries themselves are not retained. stages/ contains the earlier agent stage rather than final native execution. Native PNG files are selected from the final logs rather than stale output directories.
